use crate::ingest::TimelineId;

use crate::ingest::WrappedMessage;
use crate::{ingest, TIMELINE_IDENTIFIER, UserTimelineInfo};

use duplicate::duplicate_item;
use once_cell::sync::Lazy;
use std::{
    collections::{HashMap, HashSet},
    fmt::Debug,
    num::NonZeroU64,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        RwLock,
    },
    time::Instant,
};
use tokio::sync::mpsc;
use tracing_core::{
    field::Visit,
    span::{Attributes, Id, Record},
    Field, Subscriber,
};
use tracing_subscriber::{
    layer::{Context, Layer},
    registry::LookupSpan,
};

static START: Lazy<Instant> = Lazy::new(Instant::now);
static NEXT_SPAN_ID: AtomicU64 = AtomicU64::new(1);
static WARN_LATCH: AtomicBool = AtomicBool::new(false);

/// An ID for spans that we can use directly.
#[derive(Copy, Clone, Debug)]
pub(crate) struct LocalSpanId(NonZeroU64);

/// A newtype to store the span's name in itself for later use.
#[derive(Clone, Debug)]
pub(crate) struct SpanName(String);

#[cfg(feature = "async")]
impl LayerCommon for crate::r#async::ModalityLayer {}
#[cfg(feature = "blocking")]
impl LayerCommon for crate::blocking::ModalityLayer {}

static KNOWN_TIMELINES: Lazy<RwLock<HashSet<TimelineId>>> =
    Lazy::new(|| RwLock::new(HashSet::new()));

pub(crate) trait LayerHandler {
    fn send(&self, msg: WrappedMessage) -> Result<(), mpsc::error::SendError<WrappedMessage>>;
}

trait LayerCommon: LayerHandler {
    fn handle_message(&self, message: ingest::Message) {
        let info = match TIMELINE_IDENTIFIER.get() {
            Some(ident) => (*ident)(),
            None => {
                // gets a single false across all application threads, atomically replacing with true
                // only show warning on false, so we only warn once
                //
                // ordering doesn't matter, we don't care which thread prints if multiple try
                let has_warned = WARN_LATCH
                    .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
                    .is_ok();

                if !has_warned {
                    eprintln!("warning: no timeline identifier was registered. This is a bug.");
                }

                return;
            }
        };

        let UserTimelineInfo { name, user_id } = info;

        let wrapped_message = ingest::WrappedMessage {
            message,
            tick: START.elapsed(),
            timeline_name: name,
            user_timeline_id: user_id,
        };

        if let Err(_e) = self.send(wrapped_message) {
            // gets a single false across all application threads, atomically replacing with true
            // only show warning on false, so we only warn once
            //
            // ordering doesn't matter, we don't care which thread prints if multiple try
            let has_warned = WARN_LATCH
                .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok();

            if !has_warned {
                eprintln!(
                    "warning: attempted trace after tracing modality has stopped accepting \
                     messages, ensure spans from all threads have closed before calling \
                     `finish()`"
                );
            }
        }
    }

    fn get_next_span_id(&self) -> LocalSpanId {
        loop {
            // ordering of IDs doesn't matter, only uniqueness, use relaxed ordering
            let id = NEXT_SPAN_ID.fetch_add(1, Ordering::Relaxed);
            if let Some(id) = NonZeroU64::new(id) {
                return LocalSpanId(id);
            }
        }
    }
}

fn get_local_span_id<S>(span: &Id, ctx: &Context<'_, S>) -> LocalSpanId
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    // if either of these fail, it's a bug in `tracing`
    *ctx.span(span)
        .expect("get span tracing just told us about")
        .extensions()
        .get()
        .expect("get `LocalSpanId`, should always exist on spans")
}

#[cfg(feature = "blocking")]
use crate::blocking::ModalityLayer as BlockingModalityLayer;
#[cfg(feature = "async")]
use crate::r#async::ModalityLayer as AsyncModalityLayer;

#[cfg_attr(all(feature = "async", feature = "blocking"),
  duplicate_item(layer; [ AsyncModalityLayer ]; [ BlockingModalityLayer ];))]
#[cfg_attr(all(feature = "async", not(feature = "blocking")),
  duplicate_item(layer; [ AsyncModalityLayer ];))]
#[cfg_attr(all(not(feature = "async"), feature = "blocking"),
  duplicate_item(layer; [ BlockingModalityLayer ];))]
impl<S> Layer<S> for layer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn enabled(&self, _metadata: &tracing_core::Metadata<'_>, _ctx: Context<'_, S>) -> bool {
        // always enabled for all levels
        true
    }

    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let local_id = self.get_next_span_id();
        ctx.span(id).unwrap().extensions_mut().insert(local_id);

        let mut visitor = RecordMapBuilder::new();
        attrs.record(&mut visitor);
        let records = visitor.values();
        let metadata = attrs.metadata();

        let msg = ingest::Message::NewSpan {
            id: local_id.0,
            metadata,
            records,
        };

        self.handle_message(msg);
    }

    fn on_record(&self, span: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        let local_id = get_local_span_id(span, &ctx);

        let mut visitor = RecordMapBuilder::new();
        values.record(&mut visitor);

        let msg = ingest::Message::Record {
            span: local_id.0,
            records: visitor.values(),
        };

        self.handle_message(msg)
    }

    fn on_follows_from(&self, span: &Id, follows: &Id, ctx: Context<'_, S>) {
        let local_id = get_local_span_id(span, &ctx);
        let follows_local_id = get_local_span_id(follows, &ctx);

        let msg = ingest::Message::RecordFollowsFrom {
            span: local_id.0,
            follows: follows_local_id.0,
        };

        self.handle_message(msg)
    }

    fn on_event(&self, event: &tracing_core::Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = RecordMapBuilder::new();
        event.record(&mut visitor);

        let msg = ingest::Message::Event {
            metadata: event.metadata(),
            records: visitor.values(),
        };

        self.handle_message(msg)
    }

    fn on_enter(&self, span: &Id, ctx: Context<'_, S>) {
        let local_id = get_local_span_id(span, &ctx);

        let msg = ingest::Message::Enter { span: local_id.0 };

        self.handle_message(msg)
    }

    fn on_exit(&self, span: &Id, ctx: Context<'_, S>) {
        let local_id = get_local_span_id(span, &ctx);

        let msg = ingest::Message::Exit { span: local_id.0 };

        self.handle_message(msg)
    }

    fn on_id_change(&self, old: &Id, new: &Id, ctx: Context<'_, S>) {
        let old_local_id = get_local_span_id(old, &ctx);
        let new_local_id = self.get_next_span_id();
        ctx.span(new).unwrap().extensions_mut().insert(new_local_id);

        let msg = ingest::Message::IdChange {
            old: old_local_id.0,
            new: new_local_id.0,
        };

        self.handle_message(msg)
    }

    fn on_close(&self, span: Id, ctx: Context<'_, S>) {
        let local_id = get_local_span_id(&span, &ctx);

        let msg = ingest::Message::Close { span: local_id.0 };

        self.handle_message(msg)
    }
}

#[derive(Debug)]
pub(crate) enum TracingValue {
    String(String),
    F64(f64),
    I64(i64),
    U64(u64),
    Bool(bool),
}

pub(crate) type RecordMap = HashMap<String, TracingValue>;

struct RecordMapBuilder {
    record_map: RecordMap,
}

impl RecordMapBuilder {
    fn values(self) -> RecordMap {
        self.record_map
    }
}

impl RecordMapBuilder {
    fn new() -> RecordMapBuilder {
        RecordMapBuilder {
            record_map: HashMap::new(),
        }
    }
}

impl Visit for RecordMapBuilder {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        self.record_map.insert(
            field.name().to_string(),
            TracingValue::String(format!("{:?}", value)),
        );
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.record_map
            .insert(field.name().to_string(), TracingValue::F64(value));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.record_map
            .insert(field.name().to_string(), TracingValue::I64(value));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.record_map
            .insert(field.name().to_string(), TracingValue::U64(value));
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.record_map
            .insert(field.name().to_string(), TracingValue::Bool(value));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.record_map.insert(
            field.name().to_string(),
            TracingValue::String(value.to_string()),
        );
    }
}
