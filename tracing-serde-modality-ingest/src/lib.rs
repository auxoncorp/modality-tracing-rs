pub mod options;

use modality_ingest_protocol::{
    client::{BoundTimelineState, IngestClient},
    types::{AttrVal, BigInt, EventAttrKey, TimelineAttrKey, TimelineId},
};
use std::collections::HashMap;
use tracing_serde::{
    DebugRecord, SerializeFieldSet, SerializeId, SerializeRecordFields, SerializeValue,
};
use tracing_serde_wire::{Packet, TWOther, TracingWire};

pub use options::Options;

pub struct TracingModalityLense {
    client: IngestClient<BoundTimelineState>,
    event_keys: HashMap<String, EventAttrKey>,
    timeline_keys: HashMap<String, TimelineAttrKey>,
    spans: u64,
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

        let client = client
            .open_timeline(TimelineId::allocate())
            .await
            .expect("open new timeline");

        let mut lense = Self {
            client,
            event_keys: HashMap::new(),
            timeline_keys: HashMap::new(),
            spans: 0,
        };

        if let Some(name) = options.name {
            let timeline_key_name = lense
                .get_or_create_timeline_attr_key("name".to_string())
                .await
                .map_err(|e| format!("failed to get timeline attr key: {:?}", e))?;

            lense
                .client
                .timeline_metadata([(timeline_key_name, AttrVal::String(name))])
                .await
                .map_err(|e| format!("failed to name timeline: {}", e))?;
        }

        Ok(lense)
    }

    pub async fn handle_packet<'a>(&mut self, pkt: Packet<'_>) -> Result<(), ()> {
        match pkt.message {
            TracingWire::NewSpan(span) => {
                self.spans += 1;
                let id = self.spans;

                let mut packed_attrs = Vec::new();

                packed_attrs.push((
                    self.get_or_create_event_attr_key("event.kind".to_string())
                        .await
                        .unwrap(),
                    AttrVal::String("span-defined".to_string()),
                ));

                packed_attrs.push((
                    self.get_or_create_event_attr_key("event.span-id".to_string())
                        .await
                        .unwrap(),
                    BigInt::new_attr_val(id.try_into().expect("64 bit or smaller architechure")),
                ));

                packed_attrs.push((
                    self.get_or_create_event_attr_key("event.name".to_string())
                        .await
                        .unwrap(),
                    AttrVal::String(span.metadata.name.to_string()),
                ));

                // This duplicates the data from `source_file` and `source_line`.
                //packed_attrs.push((
                //    self.get_or_create_event_attr_key("event.metadata.target".to_string())
                //        .await
                //        .unwrap(),
                //    AttrVal::String(span.metadata.target.to_string()),
                //));

                packed_attrs.push((
                    self.get_or_create_event_attr_key("event.severity".to_string())
                        .await
                        .unwrap(),
                    AttrVal::String(format!("{:?}", span.metadata.level)),
                ));

                if let Some(ref module_path) = span.metadata.module_path {
                    packed_attrs.push((
                        self.get_or_create_event_attr_key("event.module_path".to_string())
                            .await
                            .unwrap(),
                        AttrVal::String(module_path.to_string()),
                    ));
                }

                if let Some(ref file) = span.metadata.file {
                    packed_attrs.push((
                        self.get_or_create_event_attr_key("event.source_file".to_string())
                            .await
                            .unwrap(),
                        AttrVal::String(file.to_string()),
                    ));
                }

                if let Some(ref line) = span.metadata.line {
                    packed_attrs.push((
                        self.get_or_create_event_attr_key("event.source_line".to_string())
                            .await
                            .unwrap(),
                        AttrVal::String(line.to_string()),
                    ));
                }

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

                match &span.metadata.fields {
                    SerializeFieldSet::Ser(_event) => {
                        todo!()
                    }
                    SerializeFieldSet::De(record_map) => {
                        //TODO: Indexes are a gross key. Is this the best option?
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

                match ev.fields {
                    SerializeRecordFields::Ser(_event) => {
                        todo!()
                    }
                    SerializeRecordFields::De(record_map) => {
                        for (name, value) in record_map {
                            let attrval = match value {
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
                                _ => continue,
                            };

                            if name.as_str() == "message" {
                                if let AttrVal::String(s) = attrval {
                                    message = Some(s);
                                    continue;
                                }
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

                packed_attrs.push((
                    self.get_or_create_event_attr_key("event.kind".to_string())
                        .await
                        .unwrap(),
                    AttrVal::String("event".to_string()),
                ));

                let name = message.unwrap_or_else(|| ev.metadata.name.to_string());
                packed_attrs.push((
                    self.get_or_create_event_attr_key("event.name".to_string())
                        .await
                        .unwrap(),
                    AttrVal::String(name),
                ));

                // This duplicates the data from `source_file` and `source_line`.
                //packed_attrs.push((
                //    self.get_or_create_event_attr_key("event.metadata.target".to_string())
                //        .await
                //        .unwrap(),
                //    AttrVal::String(ev.metadata.target.to_string()),
                //));

                packed_attrs.push((
                    self.get_or_create_event_attr_key("event.severity".to_string())
                        .await
                        .unwrap(),
                    AttrVal::String(format!("{:?}", ev.metadata.level)),
                ));

                if let Some(module_path) = ev.metadata.module_path {
                    packed_attrs.push((
                        self.get_or_create_event_attr_key("event.module_path".to_string())
                            .await
                            .unwrap(),
                        AttrVal::String(module_path.to_string()),
                    ));
                }

                if let Some(file) = ev.metadata.file {
                    packed_attrs.push((
                        self.get_or_create_event_attr_key("event.source_file".to_string())
                            .await
                            .unwrap(),
                        AttrVal::String(file.to_string()),
                    ));
                }

                if let Some(line) = ev.metadata.line {
                    packed_attrs.push((
                        self.get_or_create_event_attr_key("event.source_line".to_string())
                            .await
                            .unwrap(),
                        AttrVal::String(line.to_string()),
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
                            self.get_or_create_timeline_attr_key("timeline.deivce-id".to_string())
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
