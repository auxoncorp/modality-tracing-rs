pub use modality_ingest_client::types::TimelineId;

use crate::{
    attr_handlers::{event_fallback, HandlerFunc, AttributeHandler},
    layer::{RecordMap, TracingValue},
    timeline_lru::TimelineLru,
    Options,
};
use anyhow::Context;
use modality_ingest_client::{
    client::{BoundTimelineState, IngestClient},
    types::{AttrKey, AttrVal, BigInt, LogicalTime, Nanoseconds, Uuid},
    IngestError as SdkIngestError,
};
use std::{
    borrow::Cow,
    collections::{hash_map::Entry, HashMap},
    num::NonZeroU64,
    time::Duration,
};
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
    timeline_keys: HashMap<String, AttrKey>,
    span_names: HashMap<NonZeroU64, String>,
    run_id: Uuid,
    timeline_map: TimelineLru,

    // NOTE: We use `Cow<'static, str>` to allow users to provide a String OR a `&'static str`
    // to use as the key
    report_handlers: HashMap<Cow<'static, str>, HandlerFunc>,

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

        let report_handlers = options
            .attribute_handlers
            .iter()
            .map(AttributeHandler::to_cow_key)
            .collect();

        Ok(Self {
            client,
            global_metadata: options.metadata,
            event_keys: HashMap::new(),
            timeline_keys: HashMap::new(),
            span_names: HashMap::new(),
            run_id,
            timeline_map: TimelineLru::with_capacity(options.lru_cache_size),
            report_handlers,
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
        // TODO(AJM): This check is redundant everywhere AFAICT?
        // Should we make a NewType that guarantees the string starts with
        // 'event.'?
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
        // We create a "pre packed attrs" to make it easier to quickly determine
        // if the user has provided
        let mut pre_packed_attrs: HashMap<Cow<str>, AttrVal> =
            HashMap::with_capacity(records.len());

        // First, provide all records to the user
        for (name, val) in records.drain() {
            let (key, attrval) = match self.report_handlers.get(name.as_str()) {
                Some(hdlr) => (hdlr)(&name, val, &self.run_id),
                None => event_fallback(name.into(), val),
            };
            pre_packed_attrs.insert(key, attrval);
        }

        // Then, if any of the expected keys are NOT provided by the user,
        // we attempt to fill them from other sources of data, such as tracing
        // metadata or system information.

        /// If the given `name` does not exist in `ppa`, then the function `f`
        /// will be called to insert it iff the function `f` returns a `Some`
        /// value.
        #[inline(always)]
        fn attr_adder_cond<F: FnOnce() -> Option<AttrVal>>(
            name: &'static str,
            ppa: &mut HashMap<Cow<str>, AttrVal>,
            f: F,
        ) {
            let bname = Cow::Borrowed(name);
            if let Entry::Vacant(e) = ppa.entry(bname) {
                if let Some(val) = f() {
                    e.insert(val);
                }
            }
        }
        let cb = Cow::Borrowed;
        let ppa = &mut pre_packed_attrs;

        // These cannot fail
        ppa.entry(cb("event.name"))
            .or_insert_with(|| metadata.name().into());
        ppa.entry(cb("event.severity"))
            .or_insert_with(|| format!("{}", metadata.level()).to_lowercase().into());

        // These can fail
        attr_adder_cond("event.source.module", ppa, || {
            metadata.module_path().map(Into::into)
        });
        attr_adder_cond("event.source.file", ppa, || metadata.file().map(Into::into));
        attr_adder_cond("event.source.line", ppa, || {
            metadata.line().map(|l| (l as i64).into())
        });
        attr_adder_cond("event.internal.rs.tick", ppa, || {
            let delta: u64 = tick.as_nanos().try_into().ok()?;
            Some(AttrVal::LogicalTime(LogicalTime::unary(delta)))
        });
        attr_adder_cond("event.timestamp", ppa, || {
            let delta: u64 = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .ok()?
                .as_nanos()
                .try_into()
                .ok()?;

            Some(AttrVal::Timestamp(Nanoseconds::from(delta)))
        });

        // Make sure there is enough room in `packed_attrs` for the to-be-added
        // items, so we don't need to realloc more than once
        let cur_cap = packed_attrs.capacity();
        let cur_len = packed_attrs.len();
        let ppa_len = pre_packed_attrs.len();
        if let Some(needed) = (cur_len + ppa_len).checked_sub(cur_cap) {
            packed_attrs.reserve(needed);
        }

        // Move everything from PPA to PA
        for (key, val) in pre_packed_attrs.drain() {
            let attr_key = self.get_or_create_event_attr_key(key.into()).await?;
            packed_attrs.push((attr_key, val));
        }

        Ok(())
    }
}

// `TracingValue` is `#[nonexhaustive]`, returns `None` if they add a type we don't handle and
// fail to serialize it as a stringified json value
pub fn tracing_value_to_attr_val(value: TracingValue) -> AttrVal {
    match value {
        TracingValue::String(s) => AttrVal::String(s),
        TracingValue::F64(n) => AttrVal::Float(n),
        TracingValue::I64(n) => AttrVal::Integer(n),
        TracingValue::U64(n) => BigInt::new_attr_val(n.into()),
        TracingValue::Bool(b) => AttrVal::Bool(b),
    }
}
