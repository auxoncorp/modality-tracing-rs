pub mod options;

use anyhow::Context;
use modality_ingest_protocol::{
    client::{BoundTimelineState, IngestClient},
    types::{AttrVal, BigInt, EventAttrKey, LogicalTime, TimelineAttrKey, Uuid},
    IngestError as SdkIngestError,
};
use once_cell::sync::Lazy;
use std::{
    borrow::Borrow,
    collections::HashMap,
    ops::{Deref, DerefMut},
    sync::RwLock,
};
use thiserror::Error;
use tracing_serde_structured::{
    DebugRecord, RecordMap, SerializeId, SerializeMetadata, SerializeRecord, SerializeRecordFields,
    SerializeValue,
};
use tracing_serde_wire::{Packet, TWOther, TracingWire};

pub use modality_ingest_protocol::types::TimelineId;
pub use options::Options;

// spans can be defined on any thread and then sent to another and entered/etc, track globally
static SPAN_NAMES: Lazy<RwLock<HashMap<u64, String>>> = Lazy::new(|| RwLock::new(HashMap::new()));

#[derive(Debug, Error)]
pub enum ConnectError {
    /// No auth was provided
    #[error("Authentication required")]
    AuthRequired,
    /// Auth was provided, but was not accepted by modality
    #[error("Authenticating with the provided auth failed")]
    AuthFailed(SdkIngestError),
    /// Errors that it is assumed there is no way to handle without human intervention, meant for
    /// consumers to just print and carry on or panic.
    #[error(transparent)]
    UnexpectedFailure(#[from] anyhow::Error),
}

#[derive(Debug, Error)]
pub enum IngestError {
    /// Errors that it is assumed there is no way to handle without human intervention, meant for
    /// consumers to just print and carry on or panic.
    #[error(transparent)]
    UnexpectedFailure(#[from] anyhow::Error),
}

pub struct TracingModalityLense {
    client: IngestClient<BoundTimelineState>,
    event_keys: HashMap<String, EventAttrKey>,
    timeline_keys: HashMap<String, TimelineAttrKey>,
    timeline_id: TimelineId,
}

impl TracingModalityLense {
    pub async fn connect() -> Result<Self, ConnectError> {
        let opt = Options::default();

        Self::connect_with_options(opt).await
    }

    pub async fn connect_with_options(options: Options) -> Result<Self, ConnectError> {
        let unauth_client = IngestClient::new(options.server_addr)
            .await
            .context("init ingest client")?;

        let auth_key = options.auth.ok_or(ConnectError::AuthRequired)?;
        let client = unauth_client
            .authenticate(auth_key.as_bytes().to_vec())
            .await
            .map_err(ConnectError::AuthFailed)?;

        let timeline_id = TimelineId::allocate();

        let client = client
            .open_timeline(timeline_id)
            .await
            .context("open new timeline")?;

        let mut lense = Self {
            client,
            event_keys: HashMap::new(),
            timeline_keys: HashMap::new(),
            timeline_id,
        };

        for (key, value) in options.metadata {
            let timeline_key_name = lense
                .get_or_create_timeline_attr_key(key)
                .await
                .context("get or define timeline attr key")?;

            lense
                .client
                .timeline_metadata([(timeline_key_name, value)])
                .await
                .context("apply timeline metadata")?;
        }

        Ok(lense)
    }

    pub fn timeline_id(&self) -> TimelineId {
        self.timeline_id
    }

    pub async fn handle_packet<'a>(&mut self, pkt: Packet<'_>) -> Result<(), IngestError> {
        match pkt.message {
            TracingWire::NewSpan { id, attrs, values } => {
                let mut records = match values {
                    SerializeRecord::Ser(_event) => {
                        unreachable!("this variant can't be sent")
                    }
                    SerializeRecord::De(record_map) => record_map,
                };

                {
                    // store name for future use
                    let name = records
                        .get(&"modality.name".into())
                        .or_else(|| records.get(&"message".into()))
                        .map(|n| format!("{:?}", n))
                        .unwrap_or_else(|| attrs.metadata.name.to_string());

                    SPAN_NAMES
                        .write()
                        .expect("span name lock poisoned, this is a bug")
                        .deref_mut()
                        .insert(id.id.get(), name);
                }

                let mut packed_attrs = Vec::new();

                let kind = records
                    .remove(&"modality.kind".into())
                    .and_then(tracing_value_to_attr_val)
                    .unwrap_or_else(|| "span:defined".into());
                packed_attrs.push((
                    self.get_or_create_event_attr_key("event.kind".to_string())
                        .await?,
                    kind,
                ));

                let span_id = records
                    .remove(&"modality.span_id".into())
                    .and_then(tracing_value_to_attr_val)
                    .unwrap_or_else(|| BigInt::new_attr_val(id.id.get() as i128));
                packed_attrs.push((
                    self.get_or_create_event_attr_key("event.span_id".to_string())
                        .await?,
                    span_id,
                ));

                self.pack_common_attrs(&mut packed_attrs, attrs.metadata, records, pkt.tick)
                    .await?;

                self.client
                    .event(pkt.tick.into(), packed_attrs)
                    .await
                    .context("send packed event")?;
            }
            TracingWire::Record { .. } => {
                // TODO: span events can't be added to after being sent, impl this once we can use
                // timelines to represent spans
            }
            TracingWire::RecordFollowsFrom { .. } => {
                // TODO: span events can't be added to after being sent, impl this once we can use
                // timelines to represent spans
            }
            TracingWire::Event(ev) => {
                let mut packed_attrs = Vec::new();

                let mut records = match ev.fields {
                    SerializeRecordFields::Ser(_event) => {
                        unreachable!("this variant can't be sent")
                    }
                    SerializeRecordFields::De(record_map) => record_map,
                };

                let kind = records
                    .remove(&"modality.kind".into())
                    .and_then(tracing_value_to_attr_val)
                    .unwrap_or_else(|| "event".into());
                packed_attrs.push((
                    self.get_or_create_event_attr_key("event.kind".to_string())
                        .await?,
                    kind,
                ));

                // handle manually to type the AttrVal correctly
                let remote_timeline_id = records
                    .remove(&"modality.interaction.remote_timeline_id".into())
                    .and_then(tracing_value_to_attr_val);
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
                        .await?,
                        remote_timeline_id,
                    ));
                }

                self.pack_common_attrs(&mut packed_attrs, ev.metadata, records, pkt.tick)
                    .await?;

                self.client
                    .event(pkt.tick.into(), packed_attrs)
                    .await
                    .context("send packed event")?;
            }
            TracingWire::Enter(SerializeId { id }) => {
                let mut packed_attrs = Vec::new();

                {
                    // get stored span name
                    let name = SPAN_NAMES
                        .read()
                        .expect("span name lock poisoned, this is a bug")
                        .deref()
                        .get(&id.get())
                        .map(|n| format!("enter: {}", n));

                    if let Some(name) = name {
                        packed_attrs.push((
                            self.get_or_create_event_attr_key("event.name".to_string())
                                .await?,
                            AttrVal::String(name),
                        ));
                    }
                };

                packed_attrs.push((
                    self.get_or_create_event_attr_key("event.kind".to_string())
                        .await?,
                    AttrVal::String("span:enter".to_string()),
                ));

                packed_attrs.push((
                    self.get_or_create_event_attr_key("event.span_id".to_string())
                        .await?,
                    BigInt::new_attr_val(u64::from(id).into()),
                ));

                packed_attrs.push((
                    self.get_or_create_event_attr_key("event.logical_time".to_string())
                        .await?,
                    AttrVal::LogicalTime(LogicalTime::unary(pkt.tick)),
                ));

                self.client
                    .event(pkt.tick.into(), packed_attrs)
                    .await
                    .context("send packed event")?;
            }
            TracingWire::Exit(SerializeId { id }) => {
                let mut packed_attrs = Vec::new();

                {
                    // get stored span name
                    let name = SPAN_NAMES
                        .read()
                        .expect("span name lock poisoned, this is a bug")
                        .deref()
                        .get(&id.get())
                        .map(|n| format!("exit: {}", n));

                    if let Some(name) = name {
                        packed_attrs.push((
                            self.get_or_create_event_attr_key("event.name".to_string())
                                .await?,
                            AttrVal::String(name),
                        ));
                    }
                };

                packed_attrs.push((
                    self.get_or_create_event_attr_key("event.kind".to_string())
                        .await?,
                    AttrVal::String("span:exit".to_string()),
                ));

                packed_attrs.push((
                    self.get_or_create_event_attr_key("event.span_id".to_string())
                        .await?,
                    BigInt::new_attr_val(u64::from(id).into()),
                ));

                packed_attrs.push((
                    self.get_or_create_event_attr_key("event.logical_time".to_string())
                        .await?,
                    AttrVal::LogicalTime(LogicalTime::unary(pkt.tick)),
                ));

                self.client
                    .event(pkt.tick.into(), packed_attrs)
                    .await
                    .context("send packed event")?;
            }
            TracingWire::Close(SerializeId { id }) => {
                SPAN_NAMES
                    .write()
                    .expect("span name lock poisoned, this is a bug")
                    .deref_mut()
                    .remove(&id.get());
            }
            TracingWire::IdClone { old, new } => {
                let mut span_names = SPAN_NAMES
                    .write()
                    .expect("span name lock poisoned, this is a bug");

                let name = span_names.deref().get(&old.id.get()).cloned();
                if let Some(name) = name {
                    span_names.deref_mut().insert(new.id.get(), name);
                }
            }
            TracingWire::Other(two) => {
                match two {
                    TWOther::MessageDiscarded => {
                        let mut packed_attrs = Vec::new();

                        packed_attrs.push((
                            self.get_or_create_event_attr_key("event.kind".to_string())
                                .await?,
                            AttrVal::String("message_discarded".to_string()),
                        ));
                        self.client
                            .event(pkt.tick.into(), packed_attrs)
                            .await
                            .context("send packed event")?;
                    }
                    TWOther::DeviceInfo {
                        clock_id,
                        ticks_per_sec,
                        device_id,
                    } => {
                        let mut packed_attrs = Vec::new();
                        packed_attrs.push((
                            self.get_or_create_timeline_attr_key("timeline.clock_id".to_string())
                                .await?,
                            AttrVal::Integer(clock_id.into()),
                        ));
                        packed_attrs.push((
                            self.get_or_create_timeline_attr_key(
                                "timeline.ticks_per_sec".to_string(),
                            )
                            .await?,
                            AttrVal::Integer(ticks_per_sec.into()),
                        ));
                        packed_attrs.push((
                            self.get_or_create_timeline_attr_key("timeline.device_id".to_string())
                                .await?,
                            // TODO: this includes array syntax in the ID
                            AttrVal::String(format!("{:x?}", device_id)),
                        ));
                        self.client
                            .timeline_metadata(packed_attrs)
                            .await
                            .context("send packed timeline metadata")?;
                    }
                }
            }
            _ => (),
        }

        Ok(())
    }

    async fn get_or_create_timeline_attr_key(
        &mut self,
        key: String,
    ) -> Result<TimelineAttrKey, IngestError> {
        if let Some(id) = self.timeline_keys.get(&key) {
            return Ok(*id);
        }

        let interned_key = self
            .client
            .timeline_attr_key(key.clone())
            .await
            .context("define timeline attr key")?;

        self.timeline_keys.insert(key, interned_key);

        Ok(interned_key)
    }

    async fn get_or_create_event_attr_key(
        &mut self,
        key: String,
    ) -> Result<EventAttrKey, IngestError> {
        if let Some(id) = self.event_keys.get(&key) {
            return Ok(*id);
        }

        let interned_key = self
            .client
            .event_attr_key(key.clone())
            .await
            .context("define event attr key")?;

        self.event_keys.insert(key, interned_key);

        Ok(interned_key)
    }

    async fn pack_common_attrs<'a>(
        &mut self,
        packed_attrs: &mut Vec<(EventAttrKey, AttrVal)>,
        metadata: SerializeMetadata<'a>,
        mut records: RecordMap<'a>,
        tick: u64,
    ) -> Result<(), IngestError> {
        let name = records
            .remove(&"modality.name".into())
            .or_else(|| records.remove(&"message".into()))
            .and_then(tracing_value_to_attr_val)
            .unwrap_or_else(|| metadata.name.as_str().into());
        packed_attrs.push((
            self.get_or_create_event_attr_key("event.name".to_string())
                .await?,
            name,
        ));

        // This duplicates the data from `source_file` and `source_line`.
        //packed_attrs.push((
        //    self.get_or_create_event_attr_key("event.metadata.target".to_string())
        //        .await?,
        //    AttrVal::String(metadata.target.to_string()),
        //));

        let severity = records
            .remove(&"modality.severity".into())
            .and_then(tracing_value_to_attr_val)
            .unwrap_or_else(|| format!("{:?}", metadata.level).into());
        packed_attrs.push((
            self.get_or_create_event_attr_key("event.severity".to_string())
                .await?,
            severity,
        ));

        let module_path = records
            .remove(&"modality.module_path".into())
            .and_then(tracing_value_to_attr_val)
            .or_else(|| metadata.module_path.map(|mp| mp.as_str().into()));
        if let Some(module_path) = module_path {
            packed_attrs.push((
                self.get_or_create_event_attr_key("event.module_path".to_string())
                    .await?,
                module_path,
            ));
        }

        let source_file = records
            .remove(&"modality.source_file".into())
            .and_then(tracing_value_to_attr_val)
            .or_else(|| metadata.file.map(|mp| mp.as_str().into()));
        if let Some(source_file) = source_file {
            packed_attrs.push((
                self.get_or_create_event_attr_key("event.source_file".to_string())
                    .await?,
                source_file,
            ));
        }

        let source_line = records
            .remove(&"modality.source_line".into())
            .and_then(tracing_value_to_attr_val)
            .or_else(|| metadata.line.map(|mp| (mp as i64).into()));
        if let Some(source_line) = source_line {
            packed_attrs.push((
                self.get_or_create_event_attr_key("event.source_line".to_string())
                    .await?,
                source_line,
            ));
        }

        packed_attrs.push((
            self.get_or_create_event_attr_key("event.logical_time".to_string())
                .await?,
            AttrVal::LogicalTime(LogicalTime::unary(tick)),
        ));

        // These 2 duplicate the `kind` field
        //packed_attrs.push((
        //    self.get_or_create_event_attr_key("event.metadata.is_span".to_string())
        //        .await?,
        //    AttrVal::Bool(metadata.is_span),
        //));

        //packed_attrs.push((
        //    self.get_or_create_event_attr_key("event.metadata.is_event".to_string())
        //        .await?,
        //    AttrVal::Bool(metadata.is_event),
        //));

        // pack any remaining records
        for (name, value) in records {
            let attrval = if let Some(attrval) = tracing_value_to_attr_val(value) {
                attrval
            } else {
                continue;
            };

            let key = if let Some(("modality", key)) = name.as_str().split_once('.') {
                format!("event.{}", key)
            } else {
                format!("event.payload.{}", name.as_str())
            };

            packed_attrs.push((self.get_or_create_event_attr_key(key).await?, attrval));
        }

        Ok(())
    }
}

// `SerializeValue` is `#[nonexhaustive]`, returns `None` if they add a type we don't handle and
// fail to serialize it as a stringified json value
fn tracing_value_to_attr_val<'a, V: Borrow<SerializeValue<'a>>>(value: V) -> Option<AttrVal> {
    Some(match value.borrow() {
        SerializeValue::Debug(dr) => match dr {
            // TODO: there's an opertunity here to pull out message format
            // parameters raw here instead of shipping a formatted string
            DebugRecord::Ser(s) => AttrVal::String(s.to_string()),
            DebugRecord::De(s) => AttrVal::String(s.to_string()),
        },
        SerializeValue::Str(s) => AttrVal::String(s.to_string()),
        SerializeValue::F64(n) => AttrVal::Float(*n),
        SerializeValue::I64(n) => AttrVal::Integer(*n),
        SerializeValue::U64(n) => BigInt::new_attr_val((*n).into()),
        SerializeValue::Bool(b) => AttrVal::Bool(*b),
        unknown_sv => {
            if let Ok(sval) = serde_json::to_string(&unknown_sv) {
                AttrVal::String(sval)
            } else {
                return None;
            }
        }
    })
}
