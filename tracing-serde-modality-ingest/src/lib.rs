pub mod options;

use modality_ingest_protocol::{
    client::{BoundTimelineState, IngestClient},
    types::{AttrVal, BigInt, EventAttrKey, LogicalTime, TimelineAttrKey, Uuid},
};
use std::collections::HashMap;
use tracing_serde::{
    DebugRecord, SerializeFieldSet, SerializeId, SerializeRecordFields, SerializeValue,
};
use tracing_serde_wire::{Packet, TWOther, TracingWire};

pub use modality_ingest_protocol::types::TimelineId;
pub use options::Options;

pub struct TracingModalityLense {
    client: IngestClient<BoundTimelineState>,
    event_keys: HashMap<String, EventAttrKey>,
    timeline_keys: HashMap<String, TimelineAttrKey>,
    spans: u64,
    timeline_id: TimelineId,
}

impl TracingModalityLense {
    pub async fn connect() -> Result<Self, String> {
        let opt = Options::default();

        Self::connect_with_options(opt).await
    }

    pub async fn connect_with_options(options: Options) -> Result<Self, String> {
        let unauth_client = IngestClient::new(options.server_addr)
            .await
            .map_err(|e| format!("on IngestClient::new {}", e))?;

        let auth_key = options
            .auth
            .ok_or_else(|| "auth requred, specify as option or env var".to_string())?;
        let client = unauth_client
            .authenticate(auth_key.as_bytes().to_vec())
            .await
            .expect("auth");

        let timeline_id = TimelineId::allocate();

        let client = client
            .open_timeline(timeline_id)
            .await
            .expect("open new timeline");

        let mut lense = Self {
            client,
            event_keys: HashMap::new(),
            timeline_keys: HashMap::new(),
            spans: 0,
            timeline_id,
        };

        for (key, value) in options.metadata {
            let timeline_key_name = lense
                .get_or_create_timeline_attr_key(key)
                .await
                .map_err(|e| format!("failed to get timeline attr key: {:?}", e))?;

            lense
                .client
                .timeline_metadata([(timeline_key_name, value)])
                .await
                .map_err(|e| format!("failed to name timeline: {}", e))?;
        }

        Ok(lense)
    }

    pub fn timeline_id(&self) -> TimelineId {
        self.timeline_id
    }

    pub async fn handle_packet<'a>(&mut self, pkt: Packet<'_>) -> Result<(), ()> {
        match pkt.message {
            TracingWire::NewSpan(span) => {
                self.spans += 1;
                let id = self.spans;

                //  TODO: fix serde-tracing-subscriber to send field values, not just keys
                let message: Option<String> = None;
                let mut modality_fields: HashMap<String, AttrVal> = HashMap::new();
                let mut packed_attrs = Vec::new();

                match &span.metadata.fields {
                    SerializeFieldSet::Ser(_event) => {
                        todo!()
                    }
                    SerializeFieldSet::De(record_map) => {
                        // TODO: this is only recording the keys because that's all that is
                        //       currently sent, fix this
                        for (idx, value) in record_map.iter().enumerate() {
                            packed_attrs.push((
                                self.get_or_create_event_attr_key(format!("event.payload.{}", idx))
                                    .await
                                    .unwrap(),
                                AttrVal::String(value.to_string()),
                            ));
                        }
                    }
                }

                let kind = modality_fields
                    .remove("kind")
                    .unwrap_or_else(|| "span-defined".into());
                packed_attrs.push((
                    self.get_or_create_event_attr_key("event.kind".to_string())
                        .await
                        .unwrap(),
                    kind,
                ));

                let span_id = modality_fields.remove("span_id").unwrap_or_else(|| {
                    BigInt::new_attr_val(id.try_into().expect("64 bit or smaller architechure"))
                });
                packed_attrs.push((
                    self.get_or_create_event_attr_key("event.span-id".to_string())
                        .await
                        .unwrap(),
                    span_id,
                ));

                let name = modality_fields
                    .remove("name")
                    .or_else(|| message.map(|m| m.into()))
                    .unwrap_or_else(|| span.metadata.name.as_str().into());
                packed_attrs.push((
                    self.get_or_create_event_attr_key("event.name".to_string())
                        .await
                        .unwrap(),
                    name,
                ));

                // This duplicates the data from `source_file` and `source_line`.
                //packed_attrs.push((
                //    self.get_or_create_event_attr_key("event.metadata.target".to_string())
                //        .await
                //        .unwrap(),
                //    AttrVal::String(span.metadata.target.to_string()),
                //));

                let severity = modality_fields
                    .remove("severity")
                    .unwrap_or_else(|| format!("{:?}", span.metadata.level).into());
                packed_attrs.push((
                    self.get_or_create_event_attr_key("event.severity".to_string())
                        .await
                        .unwrap(),
                    severity,
                ));

                let module_path = modality_fields
                    .remove("module_path")
                    .or_else(|| span.metadata.module_path.map(|mp| mp.as_str().into()));
                if let Some(module_path) = module_path {
                    packed_attrs.push((
                        self.get_or_create_event_attr_key("event.module_path".to_string())
                            .await
                            .unwrap(),
                        module_path,
                    ));
                }

                let source_file = modality_fields
                    .remove("source_file")
                    .or_else(|| span.metadata.file.map(|mp| mp.as_str().into()));
                if let Some(source_file) = source_file {
                    packed_attrs.push((
                        self.get_or_create_event_attr_key("event.source_file".to_string())
                            .await
                            .unwrap(),
                        source_file,
                    ));
                }

                let source_line = modality_fields.remove("source_line").or_else(|| {
                    span.metadata.line.map(|mp| {
                        let mp: i64 = mp.into();
                        mp.into()
                    })
                });
                if let Some(source_line) = source_line {
                    packed_attrs.push((
                        self.get_or_create_event_attr_key("event.source_line".to_string())
                            .await
                            .unwrap(),
                        source_line,
                    ));
                }

                packed_attrs.push((
                    self.get_or_create_event_attr_key("event.logical_time".to_string())
                        .await
                        .unwrap(),
                    AttrVal::LogicalTime(LogicalTime::unary(pkt.tick)),
                ));

                // These 2 duplicate the `kind` field
                //packed_attrs.push((
                //    self.get_or_create_event_attr_key("event.metadata.is_span".to_string())
                //        .await
                //        .unwrap(),
                //    AttrVal::Bool(span.metadata.is_span),
                //));

                //packed_attrs.push((
                //    self.get_or_create_event_attr_key("event.metadata.is_event".to_string())
                //        .await
                //        .unwrap(),
                //    AttrVal::Bool(span.metadata.is_event),
                //));

                // pack any remaining fields specified to be modality keys
                for (key, val) in modality_fields {
                    if let Some(("modality.", key)) = key.split_once('.') {
                        packed_attrs.push((
                            self.get_or_create_event_attr_key(format!("event.{}", key))
                                .await
                                .unwrap(),
                            val,
                        ));
                    }
                }

                self.client
                    .event(pkt.tick.into(), packed_attrs)
                    .await
                    .unwrap();
            }
            TracingWire::Record { .. } => {
                todo!("dunno what these are")
            }
            TracingWire::RecordFollowsFrom { .. } => {
                todo!("dunno what these are")
            }
            TracingWire::Event(ev) => {
                let mut packed_attrs = Vec::new();
                let mut message = None;
                let mut modality_fields = HashMap::new();

                match ev.fields {
                    SerializeRecordFields::Ser(_event) => {
                        todo!()
                    }
                    SerializeRecordFields::De(record_map) => {
                        for (name, value) in record_map {
                            let attrval = if let Some(attrval) = tracing_value_to_attr_val(value) {
                                attrval
                            } else {
                                continue;
                            };

                            if name.as_str() == "message" {
                                if let AttrVal::String(s) = attrval {
                                    message = Some(s);
                                    continue;
                                }
                            } else if let Some(("modality", name)) = name.as_str().split_once('.') {
                                modality_fields.insert(name.to_owned(), attrval);
                                continue;
                            }

                            packed_attrs.push((
                                self.get_or_create_event_attr_key(format!(
                                    "event.payload.{}",
                                    name.as_str()
                                ))
                                .await
                                .unwrap(),
                                attrval,
                            ));
                        }
                    }
                }

                let kind = modality_fields
                    .remove("kind")
                    .unwrap_or_else(|| "event".into());
                packed_attrs.push((
                    self.get_or_create_event_attr_key("event.kind".to_string())
                        .await
                        .unwrap(),
                    kind,
                ));

                let name = modality_fields
                    .remove("name")
                    .or_else(|| message.map(|m| m.into()))
                    .unwrap_or_else(|| ev.metadata.name.as_str().into());
                packed_attrs.push((
                    self.get_or_create_event_attr_key("event.name".to_string())
                        .await
                        .unwrap(),
                    name,
                ));

                // note: overridden values will be a Integer instead of LogicalTime, is that a
                // problem?
                let logical_time = modality_fields
                    .remove("logical_time")
                    .unwrap_or_else(|| AttrVal::LogicalTime(LogicalTime::unary(pkt.tick)));
                packed_attrs.push((
                    self.get_or_create_event_attr_key("event.logical_time".to_string())
                        .await
                        .unwrap(),
                    logical_time,
                ));

                // This duplicates the data from `source_file` and `source_line`.
                //packed_attrs.push((
                //    self.get_or_create_event_attr_key("event.metadata.target".to_string())
                //        .await
                //        .unwrap(),
                //    AttrVal::String(ev.metadata.target.to_string()),
                //));

                let severity = modality_fields
                    .remove("severity")
                    .unwrap_or_else(|| format!("{:?}", ev.metadata.level).into());
                packed_attrs.push((
                    self.get_or_create_event_attr_key("event.severity".to_string())
                        .await
                        .unwrap(),
                    severity,
                ));

                let module_path = modality_fields
                    .remove("module_path")
                    .or_else(|| ev.metadata.module_path.map(|mp| mp.as_str().into()));
                if let Some(module_path) = module_path {
                    packed_attrs.push((
                        self.get_or_create_event_attr_key("event.module_path".to_string())
                            .await
                            .unwrap(),
                        module_path,
                    ));
                }

                let source_file = modality_fields
                    .remove("source_file")
                    .or_else(|| ev.metadata.file.map(|mp| mp.as_str().into()));
                if let Some(source_file) = source_file {
                    packed_attrs.push((
                        self.get_or_create_event_attr_key("event.source_file".to_string())
                            .await
                            .unwrap(),
                        source_file,
                    ));
                }

                let source_line = modality_fields.remove("source_line").or_else(|| {
                    ev.metadata.line.map(|mp| {
                        let mp: i64 = mp.into();
                        mp.into()
                    })
                });
                if let Some(source_line) = source_line {
                    packed_attrs.push((
                        self.get_or_create_event_attr_key("event.source_line".to_string())
                            .await
                            .unwrap(),
                        source_line,
                    ));
                }

                // These 2 duplicate the `kind` field
                //packed_attrs.push((
                //    self.get_or_create_event_attr_key("event.metadata.is_span".to_string())
                //        .await
                //        .unwrap(),
                //    AttrVal::Bool(ev.metadata.is_span),
                //));
                //
                //packed_attrs.push((
                //    self.get_or_create_event_attr_key("event.metadata.is_event".to_string())
                //        .await
                //        .unwrap(),
                //    AttrVal::Bool(ev.metadata.is_event),
                //));

                let remote_timeline_id = modality_fields.remove("interaction.remote_timeline_id");
                if let Some(attrval) = remote_timeline_id {
                    let remote_timeline_id = if let AttrVal::String(string) = attrval {
                        use std::str::FromStr;
                        if let Ok(uuid) = Uuid::from_str(&string) {
                            AttrVal::TimelineId(Box::new(uuid.into()))
                        } else {
                            AttrVal::String(string)
                        }
                    } else {
                        attrval
                    };

                    packed_attrs.push((
                        self.get_or_create_event_attr_key(
                            "event.interaction.remote_timeline_id".into(),
                        )
                        .await
                        .unwrap(),
                        remote_timeline_id,
                    ));
                }

                // pack any remaining fields specified to be modality keys
                for (key, val) in modality_fields {
                    packed_attrs.push((
                        self.get_or_create_event_attr_key(format!("event.{}", key))
                            .await
                            .unwrap(),
                        val,
                    ));
                }

                self.client
                    .event(pkt.tick.into(), packed_attrs)
                    .await
                    .unwrap();
            }
            TracingWire::Enter(SerializeId { id }) => {
                let mut packed_attrs = Vec::new();

                packed_attrs.push((
                    self.get_or_create_event_attr_key("event.kind".to_string())
                        .await
                        .unwrap(),
                    AttrVal::String("span-enter".to_string()),
                ));

                packed_attrs.push((
                    self.get_or_create_event_attr_key("event.span-id".to_string())
                        .await
                        .unwrap(),
                    BigInt::new_attr_val(u64::from(id).into()),
                ));

                packed_attrs.push((
                    self.get_or_create_event_attr_key("event.logical_time".to_string())
                        .await
                        .unwrap(),
                    AttrVal::LogicalTime(LogicalTime::unary(pkt.tick)),
                ));

                self.client
                    .event(pkt.tick.into(), packed_attrs)
                    .await
                    .unwrap();
            }
            TracingWire::Exit(SerializeId { id }) => {
                let mut packed_attrs = Vec::new();

                packed_attrs.push((
                    self.get_or_create_event_attr_key("event.kind".to_string())
                        .await
                        .unwrap(),
                    AttrVal::String("span-exit".to_string()),
                ));

                packed_attrs.push((
                    self.get_or_create_event_attr_key("event.span-id".to_string())
                        .await
                        .unwrap(),
                    BigInt::new_attr_val(u64::from(id).into()),
                ));

                self.client
                    .event(pkt.tick.into(), packed_attrs)
                    .await
                    .unwrap();
            }
            TracingWire::Other(two) => {
                match two {
                    TWOther::MessageDiscarded => {
                        let mut packed_attrs = Vec::new();

                        packed_attrs.push((
                            self.get_or_create_event_attr_key("event.kind".to_string())
                                .await
                                .unwrap(),
                            AttrVal::String("message-discarded".to_string()),
                        ));
                        self.client
                            .event(pkt.tick.into(), packed_attrs)
                            .await
                            .unwrap();
                    }
                    TWOther::DeviceInfo {
                        clock_id,
                        ticks_per_sec,
                        device_id,
                    } => {
                        let mut packed_attrs = Vec::new();
                        packed_attrs.push((
                            self.get_or_create_timeline_attr_key("timeline.clock-id".to_string())
                                .await
                                .unwrap(),
                            AttrVal::Integer(clock_id.into()),
                        ));
                        packed_attrs.push((
                            self.get_or_create_timeline_attr_key(
                                "timeline.ticks-per-sec".to_string(),
                            )
                            .await
                            .unwrap(),
                            AttrVal::Integer(ticks_per_sec.into()),
                        ));
                        packed_attrs.push((
                            self.get_or_create_timeline_attr_key("timeline.device-id".to_string())
                                .await
                                .unwrap(),
                            // TODO: this includes array syntax in the ID
                            AttrVal::String(format!("{:x?}", device_id)),
                        ));
                        self.client.timeline_metadata(packed_attrs).await.unwrap();
                    }
                }
            }
        }

        Ok(())
    }

    async fn get_or_create_timeline_attr_key(
        &mut self,
        key: String,
    ) -> Result<TimelineAttrKey, ()> {
        if let Some(id) = self.timeline_keys.get(&key) {
            return Ok(*id);
        }

        let interned_key = self
            .client
            .timeline_attr_key(key.clone())
            .await
            .expect("create timeline attr key");

        self.timeline_keys.insert(key, interned_key);

        Ok(interned_key)
    }

    async fn get_or_create_event_attr_key(&mut self, key: String) -> Result<EventAttrKey, ()> {
        if let Some(id) = self.event_keys.get(&key) {
            return Ok(*id);
        }

        let interned_key = self
            .client
            .event_attr_key(key.clone())
            .await
            .expect("create timeline attr key");

        self.event_keys.insert(key, interned_key);

        Ok(interned_key)
    }
}

// `SerializeValue` is `#[nonexhaustive]`, returns `None` if they add a type we don't handle
fn tracing_value_to_attr_val(value: SerializeValue) -> Option<AttrVal> {
    Some(match value {
        SerializeValue::Debug(dr) => match dr {
            // TODO: there's an opertunity here to pull out message format
            // parameters raw here instead of shipping a formatted string
            DebugRecord::Ser(s) => AttrVal::String(s.to_string()),
            DebugRecord::De(s) => AttrVal::String(s.to_string()),
        },
        SerializeValue::Str(s) => AttrVal::String(s.to_string()),
        SerializeValue::F64(n) => AttrVal::Float(n),
        SerializeValue::I64(n) => AttrVal::Integer(n),
        SerializeValue::U64(n) => BigInt::new_attr_val(n.into()),
        SerializeValue::Bool(b) => AttrVal::Bool(b),
        _ => return None,
    })
}
