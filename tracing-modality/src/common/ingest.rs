pub use modality_ingest_client::types::TimelineId;

use crate::{
    layer::{RecordMap, TracingValue},
    Options, timeline_lru::TimelineLru,
};
use anyhow::Context;
use modality_ingest_client::{
    client::{BoundTimelineState, IngestClient},
    types::{AttrKey, AttrVal, BigInt, LogicalTime, Nanoseconds, Uuid},
    IngestError as SdkIngestError,
};
use std::{collections::HashMap, num::NonZeroU64, time::Duration};
use thiserror::Error;
use tokio::{
    select,
    sync::mpsc::{self, UnboundedReceiver, UnboundedSender},
    sync::oneshot,
};
use tracing_core::Metadata;

#[cfg(feature = "blocking")]
use std::thread::{self, JoinHandle};
#[cfg(feature = "blocking")]
use tokio::runtime::Runtime;
#[cfg(feature = "async")]
use tokio::task;

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

pub(crate) type SpanId = NonZeroU64;

#[derive(Debug)]
pub(crate) struct WrappedMessage {
    pub message: Message,
    pub tick: Duration,
    pub timeline_name: String,
    pub user_timeline_id: u64,
}

#[derive(Debug)]
pub(crate) enum Message {
    NewSpan {
        id: SpanId,
        metadata: &'static Metadata<'static>,
        records: RecordMap,
    },
    Record {
        span: SpanId,
        records: RecordMap,
    },
    RecordFollowsFrom {
        span: SpanId,
        follows: SpanId,
    },
    Event {
        metadata: &'static Metadata<'static>,
        records: RecordMap,
    },
    Enter {
        span: SpanId,
    },
    Exit {
        span: SpanId,
    },
    Close {
        span: SpanId,
    },
    IdChange {
        old: SpanId,
        new: SpanId,
    },
}

pub trait ModalityIngestHandle {}

#[cfg(feature = "blocking")]
/// A handle to control the spawned ingest thread.
pub struct ModalityIngestThreadHandle {
    pub(crate) ingest_sender: UnboundedSender<WrappedMessage>,
    pub(crate) finish_sender: Option<oneshot::Sender<()>>,
    pub(crate) thread: Option<JoinHandle<()>>,
}

#[cfg(feature = "blocking")]
impl ModalityIngestHandle for ModalityIngestThreadHandle {}

#[cfg(feature = "blocking")]
impl ModalityIngestThreadHandle {
    /// Stop accepting new trace events, flush all existing events, and stop ingest thread.
    ///
    /// This function must be called at the end of your main thread to give the ingest thread a
    /// chance to flush all queued trace events out to modality.
    ///
    /// # Panics
    ///
    /// This function uses [`std::thread::JoinHandle::join`] which may panic on some platforms if a
    /// thread attempts to join itself or otherwise may create a deadlock with joining threads.
    /// This case should be incredibly unlikely, if not impossible, but can not be statically
    /// guarenteed.
    pub fn finish(mut self) {
        if let Some(finish) = self.finish_sender.take() {
            let _ = finish.send(());
        }

        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(feature = "async")]
/// A handle to control the spawned ingest task.
pub struct ModalityIngestTaskHandle {
    pub(crate) ingest_sender: UnboundedSender<WrappedMessage>,
    pub(crate) finish_sender: Option<oneshot::Sender<()>>,
    pub(crate) task: Option<task::JoinHandle<()>>,
}

#[cfg(feature = "async")]
impl ModalityIngestHandle for ModalityIngestTaskHandle {}

#[cfg(feature = "async")]
impl ModalityIngestTaskHandle {
    /// Stop accepting new trace events, flush all existing events, and stop ingest thread.
    ///
    /// This function must be called at the end of your main thread to give the ingest thread a
    /// chance to flush all queued trace events out to modality.
    pub async fn finish(mut self) {
        if let Some(finish) = self.finish_sender.take() {
            let _ = finish.send(());
        }

        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

pub(crate) struct ModalityIngest {
    client: IngestClient<BoundTimelineState>,
    global_metadata: Vec<(String, AttrVal)>,
    event_keys: HashMap<String, AttrKey>,
    // TODO(AJM): Should these be per-timeline? Or are all attrs shared with all timelines?
    timeline_keys: HashMap<String, AttrKey>,
    span_names: HashMap<NonZeroU64, String>,
    run_id: Uuid,
    timeline_map: TimelineLru,

    #[cfg(feature = "blocking")]
    rt: Option<Runtime>,
}

impl ModalityIngest {
    #[cfg(feature = "blocking")]
    pub(crate) fn connect(opts: Options) -> Result<Self, ConnectError> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .expect("build intial tokio current thread runtime");

        rt.block_on(async { Self::async_connect(opts).await })
            .map(move |mut m| {
                m.rt = Some(rt);
                m
            })
    }

    pub(crate) async fn async_connect(mut options: Options) -> Result<Self, ConnectError> {
        let url = url::Url::parse(&format!("modality-ingest://{}/", options.server_addr)).unwrap();
        let unauth_client = IngestClient::connect(&url, false)
            .await
            .context("init ingest client")?;

        // TODO(AJM): Does this NEED to be in the global metadata? The old code did this.
        let run_id = Uuid::new_v4();
        options.add_metadata("run_id", run_id.to_string());

        let auth_key = options.auth.ok_or(ConnectError::AuthRequired)?;
        let client = unauth_client
            .authenticate(auth_key)
            .await
            .map_err(ConnectError::AuthFailed)?;

        // open a timeline for the current thread because we need to open something to make the
        // types work
        //
        // TODO(AJM): Grumble grumble, this is partially duplicating the work of `bind_user_timeline()`,
        // and also not actually registering the metadata for this timeline. Since this is our worker
        // thread, and we will never produce tracing information that is sent to modality, it is probably
        // fine. If we do at a later time, it will then be registered via the actual call to
        // `bind_user_timeline()`.
        let user_id = (options.timeline_identifier)().user_id;
        let timeline_id = uuid::Uuid::new_v5(&run_id, &user_id.to_ne_bytes()).into();

        let client = client
            .open_timeline(timeline_id)
            .await
            .context("open new timeline")?;

        #[allow(unreachable_code)]
        Ok(Self {
            client,
            global_metadata: options.metadata,
            event_keys: HashMap::new(),
            timeline_keys: HashMap::new(),
            span_names: HashMap::new(),
            run_id,
            timeline_map: TimelineLru::with_capacity(options.lru_cache_size),
            #[cfg(feature = "blocking")]
            rt: None,
        })
    }

    #[cfg(feature = "blocking")]
    pub(crate) fn spawn_thread(mut self) -> ModalityIngestThreadHandle {
        let (sender, recv) = mpsc::unbounded_channel();
        let (finish_sender, finish_receiver) = oneshot::channel();

        let join_handle = thread::spawn(move || {
            // ensure this thread doesn't send trace events to the global dispatcher
            let _dispatch_guard = tracing::dispatcher::set_default(&tracing::Dispatch::none());

            let rt = self.rt.take().unwrap_or_else(|| {
                tokio::runtime::Builder::new_current_thread()
                    .build()
                    .expect("build local tokio current thread runtime")
            });

            rt.block_on(self.handler_task(recv, finish_receiver))
        });

        ModalityIngestThreadHandle {
            ingest_sender: sender,
            finish_sender: Some(finish_sender),
            thread: Some(join_handle),
        }
    }

    #[cfg(feature = "async")]
    pub(crate) async fn spawn_task(self) -> ModalityIngestTaskHandle {
        let (ingest_sender, recv) = mpsc::unbounded_channel();
        let (finish_sender, finish_receiver) = oneshot::channel();

        let task = tokio::spawn(self.handler_task(recv, finish_receiver));

        ModalityIngestTaskHandle {
            ingest_sender,
            finish_sender: Some(finish_sender),
            task: Some(task),
        }
    }

    async fn handler_task(
        mut self,
        mut recv: UnboundedReceiver<WrappedMessage>,
        mut finish: oneshot::Receiver<()>,
    ) {
        loop {
            select! {
                Some(message) = recv.recv() => {
                    let _ = self.handle_packet(message).await;
                },
                _ = &mut finish => {
                    break
                }
            }
        }

        // close channel and drain existing messages
        recv.close();
        while let Some(message) = recv.recv().await {
            let _ = self.handle_packet(message).await;
        }
        let _ = self.client.flush().await;
    }

    /// Ensures the reported user timeline is the currently open client timeline.
    ///
    /// * If this is a new`*` timeline, we will first register its metadata,
    ///     and open the timeline.
    /// * If this is NOT a new timeline, but not the current timeline,
    ///     we will open it.
    /// * If this is already the current timeline, nothing will be done.
    ///
    /// `*`: In some cases, we MAY end up re-registering the metadata for a timeline,
    /// if the timeline has not been recently used.
    async fn bind_user_timeline(&mut self, name: String, user_id: u64) -> Result<(), IngestError> {
        let timeline_id = match self.timeline_map.query(user_id) {
            Ok(tid) => tid,
            Err(token) => {
                let timeline_id = uuid::Uuid::new_v5(&self.run_id, &user_id.to_ne_bytes()).into();

                // Register the metadata of the new* timeline ID
                let mut timeline_metadata = self.global_metadata.clone();
                timeline_metadata.push(("timeline.name".to_string(), name.into()));

                for (key, value) in timeline_metadata {
                    let timeline_key_name = self
                        .get_or_create_timeline_attr_key(key)
                        .await
                        .context("get or define timeline attr key")?;

                    self.client
                        .timeline_metadata([(timeline_key_name, value)])
                        .await
                        .context("apply timeline metadata")?;
                }

                // Success, now add to the map
                self.timeline_map.insert(user_id, timeline_id, token);

                // And return the timeline
                timeline_id
            }
        };

        // TODO(AJM): DO LRU EVICTION HERE!

        if self.client.bound_timeline() != timeline_id {
            self.client
                .open_timeline(timeline_id)
                .await
                .context("open new timeline")?;
        }

        Ok(())
    }

    async fn handle_packet(&mut self, message: WrappedMessage) -> Result<(), IngestError> {
        let WrappedMessage {
            message,
            tick,
            timeline_name,
            user_timeline_id,
        } = message;

        // Ensure that the user reported timeline ID is active.
        self.bind_user_timeline(timeline_name, user_timeline_id)
            .await?;

        match message {
            Message::NewSpan {
                id,
                metadata,
                mut records,
            } => {
                let name = {
                    // store name for future use
                    let name = records
                        .get("name")
                        .or_else(|| records.get("message"))
                        .map(|n| format!("{:?}", n))
                        .unwrap_or_else(|| metadata.name().to_string());

                    self.span_names.insert(id, name.clone());

                    name
                };

                let mut packed_attrs = Vec::new();

                packed_attrs.push((
                    self.get_or_create_event_attr_key("event.name".to_string())
                        .await?,
                    AttrVal::String(name),
                ));

                let kind = records
                    .remove("modality.kind")
                    .map(tracing_value_to_attr_val)
                    .unwrap_or_else(|| "span:defined".into());
                packed_attrs.push((
                    self.get_or_create_event_attr_key("event.internal.rs.kind".to_string())
                        .await?,
                    kind,
                ));

                let span_id = records
                    .remove("modality.span_id")
                    .map(tracing_value_to_attr_val)
                    .unwrap_or_else(|| BigInt::new_attr_val(u64::from(id) as i128));
                packed_attrs.push((
                    self.get_or_create_event_attr_key("event.internal.rs.span_id".to_string())
                        .await?,
                    span_id,
                ));

                self.pack_common_attrs(&mut packed_attrs, metadata, records, tick)
                    .await?;

                self.client
                    .event(tick.as_nanos(), packed_attrs)
                    .await
                    .context("send packed event")?;
            }
            Message::Record { span, records } => {
                // TODO: span events can't be added to after being sent, impl this once we can use
                // timelines to represent spans

                let _ = span;
                let _ = records;
            }
            Message::RecordFollowsFrom { span, follows } => {
                // TODO: span events can't be added to after being sent, impl this once we can use
                // timelines to represent spans

                let _ = span;
                let _ = follows;
            }
            Message::Event {
                metadata,
                mut records,
            } => {
                let mut packed_attrs = Vec::new();

                let kind = records
                    .remove("modality.kind")
                    .map(tracing_value_to_attr_val)
                    .unwrap_or_else(|| "event".into());
                packed_attrs.push((
                    self.get_or_create_event_attr_key("event.internal.rs.kind".to_string())
                        .await?,
                    kind,
                ));

                self.pack_common_attrs(&mut packed_attrs, metadata, records, tick)
                    .await?;

                self.client
                    .event(tick.as_nanos(), packed_attrs)
                    .await
                    .context("send packed event")?;
            }
            Message::Enter { span } => {
                let mut packed_attrs = Vec::new();

                {
                    // get stored span name
                    let name = self.span_names.get(&span).map(|n| format!("enter: {}", n));

                    if let Some(name) = name {
                        packed_attrs.push((
                            self.get_or_create_event_attr_key("event.name".to_string())
                                .await?,
                            AttrVal::String(name),
                        ));
                    }
                };

                packed_attrs.push((
                    self.get_or_create_event_attr_key("event.internal.rs.kind".to_string())
                        .await?,
                    AttrVal::String("span:enter".to_string()),
                ));

                packed_attrs.push((
                    self.get_or_create_event_attr_key("event.internal.rs.span_id".to_string())
                        .await?,
                    BigInt::new_attr_val(u64::from(span).into()),
                ));

                // only record tick directly during the first ~5.8 centuries this program is running
                if let Ok(tick) = TryInto::<u64>::try_into(tick.as_nanos()) {
                    packed_attrs.push((
                        self.get_or_create_event_attr_key("event.internal.rs.tick".to_string())
                            .await?,
                        AttrVal::LogicalTime(LogicalTime::unary(tick)),
                    ));
                }

                self.client
                    .event(tick.as_nanos(), packed_attrs)
                    .await
                    .context("send packed event")?;
            }
            Message::Exit { span } => {
                let mut packed_attrs = Vec::new();

                {
                    // get stored span name
                    let name = self.span_names.get(&span).map(|n| format!("exit: {}", n));

                    if let Some(name) = name {
                        packed_attrs.push((
                            self.get_or_create_event_attr_key("event.name".to_string())
                                .await?,
                            AttrVal::String(name),
                        ));
                    }
                };

                packed_attrs.push((
                    self.get_or_create_event_attr_key("event.internal.rs.kind".to_string())
                        .await?,
                    AttrVal::String("span:exit".to_string()),
                ));

                packed_attrs.push((
                    self.get_or_create_event_attr_key("event.internal.rs.span_id".to_string())
                        .await?,
                    BigInt::new_attr_val(u64::from(span).into()),
                ));

                // only record tick directly during the first ~5.8 centuries this program is running
                if let Ok(tick) = TryInto::<u64>::try_into(tick.as_nanos()) {
                    packed_attrs.push((
                        self.get_or_create_event_attr_key("event.internal.rs.tick".to_string())
                            .await?,
                        AttrVal::LogicalTime(LogicalTime::unary(tick)),
                    ));
                }

                self.client
                    .event(tick.as_nanos(), packed_attrs)
                    .await
                    .context("send packed event")?;
            }
            Message::Close { span } => {
                self.span_names.remove(&span);
            }
            Message::IdChange { old, new } => {
                let name = self.span_names.get(&old).cloned();
                if let Some(name) = name {
                    self.span_names.insert(new, name);
                }
            }
        }

        Ok(())

        // todo!("Rework processing!");
    }

    async fn get_or_create_timeline_attr_key(
        &mut self,
        key: String,
    ) -> Result<AttrKey, IngestError> {
        if let Some(id) = self.timeline_keys.get(&key) {
            return Ok(*id);
        }

        let interned_key = self
            .client
            .attr_key(key.clone())
            .await
            .context("define timeline attr key")?;

        self.timeline_keys.insert(key, interned_key);

        Ok(interned_key)
    }

    async fn get_or_create_event_attr_key(&mut self, key: String) -> Result<AttrKey, IngestError> {
        let key = if key.starts_with("event.") {
            key
        } else {
            format!("event.{key}")
        };

        if let Some(id) = self.event_keys.get(&key) {
            return Ok(*id);
        }

        let interned_key = self
            .client
            .attr_key(key.clone())
            .await
            .context("define event attr key")?;

        self.event_keys.insert(key, interned_key);

        Ok(interned_key)
    }

    async fn pack_common_attrs<'a>(
        &mut self,
        packed_attrs: &mut Vec<(AttrKey, AttrVal)>,
        metadata: &'a Metadata<'static>,
        mut records: RecordMap,
        tick: Duration,
    ) -> Result<(), IngestError> {
        let name = records
            .remove("name")
            .or_else(|| records.remove("message"))
            .map(tracing_value_to_attr_val)
            .unwrap_or_else(|| metadata.name().into());
        packed_attrs.push((
            self.get_or_create_event_attr_key("event.name".to_string())
                .await?,
            name,
        ));

        let severity = records
            .remove("severity")
            .map(tracing_value_to_attr_val)
            .unwrap_or_else(|| format!("{}", metadata.level()).to_lowercase().into());
        packed_attrs.push((
            self.get_or_create_event_attr_key("event.severity".to_string())
                .await?,
            severity,
        ));

        let module_path = records
            .remove("source.module")
            .map(tracing_value_to_attr_val)
            .or_else(|| metadata.module_path().map(|mp| mp.into()));
        if let Some(module_path) = module_path {
            packed_attrs.push((
                self.get_or_create_event_attr_key("event.source.module".to_string())
                    .await?,
                module_path,
            ));
        }

        let source_file = records
            .remove("source.file")
            .map(tracing_value_to_attr_val)
            .or_else(|| metadata.file().map(|mp| mp.into()));
        if let Some(source_file) = source_file {
            packed_attrs.push((
                self.get_or_create_event_attr_key("event.source.file".to_string())
                    .await?,
                source_file,
            ));
        }

        let source_line = records
            .remove("source.line")
            .map(tracing_value_to_attr_val)
            .or_else(|| metadata.line().map(|mp| (mp as i64).into()));
        if let Some(source_line) = source_line {
            packed_attrs.push((
                self.get_or_create_event_attr_key("event.source.line".to_string())
                    .await?,
                source_line,
            ));
        }

        // only record tick directly during the first ~5.8 centuries this program is running
        if let Ok(tick) = TryInto::<u64>::try_into(tick.as_nanos()) {
            packed_attrs.push((
                self.get_or_create_event_attr_key("event.internal.rs.tick".to_string())
                    .await?,
                AttrVal::LogicalTime(LogicalTime::unary(tick)),
            ));
        }

        // handle manually to type the AttrVal correctly
        let remote_timeline_id = records
            .remove("interaction.remote_timeline_id")
            .map(tracing_value_to_attr_val);
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
                self.get_or_create_event_attr_key("event.interaction.remote_timeline_id".into())
                    .await?,
                remote_timeline_id,
            ));
        }

        // Manually retype the remote_timestamp
        let remote_timestamp = records
            .remove("interaction.remote_timestamp")
            .map(tracing_value_to_attr_val);
        if let Some(attrval) = remote_timestamp {
            let remote_timestamp = match attrval {
                AttrVal::Integer(i) if i >= 0 => AttrVal::Timestamp(Nanoseconds::from(i as u64)),
                AttrVal::BigInt(i) if *i >= 0 && *i <= u64::MAX as i128 => {
                    AttrVal::Timestamp(Nanoseconds::from(*i as u64))
                }
                AttrVal::Timestamp(t) => AttrVal::Timestamp(t),
                x => x,
            };

            packed_attrs.push((
                self.get_or_create_event_attr_key("event.interaction.remote_timestamp".into())
                    .await?,
                remote_timestamp,
            ));
        }

        // Manually retype the local timestamp
        let local_timestamp = records.remove("timestamp").map(tracing_value_to_attr_val);
        if let Some(attrval) = local_timestamp {
            let remote_timestamp = match attrval {
                AttrVal::Integer(i) if i >= 0 => AttrVal::Timestamp(Nanoseconds::from(i as u64)),
                AttrVal::BigInt(i) if *i >= 0 && *i <= u64::MAX as i128 => {
                    AttrVal::Timestamp(Nanoseconds::from(*i as u64))
                }
                AttrVal::Timestamp(t) => AttrVal::Timestamp(t),
                x => x,
            };

            packed_attrs.push((
                self.get_or_create_event_attr_key("event.timestamp".into())
                    .await?,
                remote_timestamp,
            ));
        } else if let Ok(duration_since_epoch) =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        {
            let duration_since_epoch_in_nanos_res: Result<u64, _> =
                duration_since_epoch.as_nanos().try_into();
            if let Ok(duration_since_epoch_in_nanos) = duration_since_epoch_in_nanos_res {
                packed_attrs.push((
                    self.get_or_create_event_attr_key("event.timestamp".into())
                        .await?,
                    AttrVal::Timestamp(Nanoseconds::from(duration_since_epoch_in_nanos)),
                ));
            }
        }

        // pack any remaining records
        for (name, value) in records {
            let attrval = tracing_value_to_attr_val(value);

            let key = if name.starts_with("event.") {
                name.to_string()
            } else {
                format!("event.{}", name.as_str())
            };

            packed_attrs.push((self.get_or_create_event_attr_key(key).await?, attrval));
        }

        Ok(())
    }
}

// `TracingValue` is `#[nonexhaustive]`, returns `None` if they add a type we don't handle and
// fail to serialize it as a stringified json value
fn tracing_value_to_attr_val(value: TracingValue) -> AttrVal {
    match value {
        TracingValue::String(s) => AttrVal::String(s),
        TracingValue::F64(n) => AttrVal::Float(n),
        TracingValue::I64(n) => AttrVal::Integer(n),
        TracingValue::U64(n) => BigInt::new_attr_val(n.into()),
        TracingValue::Bool(b) => AttrVal::Bool(b),
    }
}
