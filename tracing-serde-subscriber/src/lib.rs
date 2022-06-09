pub use tracing_serde_modality_ingest::TimelineId;
pub use tracing_serde_wire::Packet;

use std::{
    fmt::Debug,
    num::NonZeroU64,
    sync::atomic::{AtomicU64, Ordering},
    sync::RwLock,
    thread, thread_local,
    time::Instant,
};

use once_cell::sync::Lazy;
use tokio::runtime::Runtime;
use tracing_core::{
    field::Visit,
    span::{Attributes, Id, Record},
    Field, Subscriber,
};

use tracing_serde_modality_ingest::{options::GLOBAL_OPTIONS, TracingModalityLense};
use tracing_serde_structured::{AsSerde, CowString, RecordMap, SerializeValue};
use tracing_serde_wire::TracingWire;

static START: Lazy<Instant> = Lazy::new(Instant::now);
static NEXT_SPAN_ID: AtomicU64 = AtomicU64::new(1);

thread_local! {
    static HANDLER: Lazy<RwLock<TSHandler>> = Lazy::new(|| {
        let opts = GLOBAL_OPTIONS.read().unwrap().clone();

        let cur = thread::current();
        let name = cur.name().map(str::to_string).unwrap_or_else(|| format!("Thread#{:?}", cur.id()));
        let opts = opts.with_name(name);

        let rt = Runtime::new().expect("create tokio runtime");
        let lense = {
            let handle = rt.handle();
            handle.block_on(async { TracingModalityLense::connect_with_options(opts).await.expect("connect") })
        };

        RwLock::new(TSHandler {
            rt,
            lense,
        })
    });
}

pub fn timeline_id() -> TimelineId {
    HANDLER.with(|c| c.read().unwrap().lense.timeline_id())
}

fn get_next_id() -> Id {
    loop {
        // ordering of IDs doesn't matter, only uniqueness, use relaxed ordering
        let id = NEXT_SPAN_ID.fetch_add(1, Ordering::Relaxed);
        if let Some(id) = NonZeroU64::new(id) {
            return Id::from_non_zero_u64(id);
        }
    }
}

pub struct TSHandler {
    lense: TracingModalityLense,
    rt: Runtime,
}

impl TSHandler {
    fn handle_message(&mut self, message: TracingWire<'_>) {
        let packet = Packet {
            message,
            // NOTE: will give inaccurate data if the program has run for more than 584942 years.
            tick: START.elapsed().as_micros() as u64,
        };
        self.rt
            .handle()
            .block_on(async { self.lense.handle_packet(packet).await })
            .unwrap();
    }
}

pub struct TSSubscriber;

impl Subscriber for TSSubscriber {
    fn enabled(&self, _metadata: &tracing_core::Metadata<'_>) -> bool {
        // always enabled for all levels
        true
    }

    fn new_span(&self, attrs: &Attributes<'_>) -> Id {
        let id = get_next_id();

        let mut visitor = RecordMapBuilder::new();

        attrs.record(&mut visitor);

        let msg = TracingWire::NewSpan {
            id: id.as_serde(),
            attrs: attrs.as_serde(),
            values: visitor.values().into(),
        };

        HANDLER.with(move |c| c.write().unwrap().handle_message(msg));

        id
    }

    fn record(&self, span: &Id, values: &Record<'_>) {
        let msg = TracingWire::Record {
            span: span.as_serde(),
            values: values.as_serde().to_owned(),
        };

        HANDLER.with(move |c| c.write().unwrap().handle_message(msg))
    }

    fn record_follows_from(&self, span: &Id, follows: &Id) {
        let msg = TracingWire::RecordFollowsFrom {
            span: span.as_serde(),
            follows: follows.as_serde().to_owned(),
        };

        HANDLER.with(move |c| c.write().unwrap().handle_message(msg))
    }

    fn event(&self, event: &tracing_core::Event<'_>) {
        let msg = TracingWire::Event(event.as_serde().to_owned());

        HANDLER.with(move |c| c.write().unwrap().handle_message(msg))
    }

    fn enter(&self, span: &Id) {
        let msg = TracingWire::Enter(span.as_serde());

        HANDLER.with(move |c| c.write().unwrap().handle_message(msg))
    }

    fn exit(&self, span: &Id) {
        let msg = TracingWire::Exit(span.as_serde());

        HANDLER.with(move |c| c.write().unwrap().handle_message(msg))
    }
}

struct RecordMapBuilder<'a> {
    record_map: RecordMap<'a>,
}

impl<'a> RecordMapBuilder<'a> {
    fn values(self) -> RecordMap<'a> {
        self.record_map
    }
}

impl<'a> RecordMapBuilder<'a> {
    fn new() -> RecordMapBuilder<'a> {
        RecordMapBuilder {
            record_map: RecordMap::new(),
        }
    }
}

impl<'a> Visit for RecordMapBuilder<'a> {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        self.record_map.insert(
            CowString::Borrowed(field.name()),
            SerializeValue::Debug(CowString::Owned(format!("{:?}", value)).into()),
        );
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.record_map.insert(
            CowString::Borrowed(field.name()),
            SerializeValue::F64(value),
        );
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.record_map.insert(
            CowString::Borrowed(field.name()),
            SerializeValue::I64(value),
        );
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.record_map.insert(
            CowString::Borrowed(field.name()),
            SerializeValue::U64(value),
        );
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.record_map.insert(
            CowString::Borrowed(field.name()),
            SerializeValue::Bool(value),
        );
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.record_map.insert(
            CowString::Borrowed(field.name()),
            SerializeValue::Str(CowString::Borrowed(value).to_owned()),
        );
    }
}
