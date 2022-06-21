pub use tracing_serde_modality_ingest::TimelineId;
pub use tracing_serde_wire::Packet;

use std::{fmt::Debug, sync::RwLock, thread, thread_local, time::Instant};

use once_cell::sync::Lazy;
use tokio::runtime::Runtime;
use tracing_core::{
    field::Visit,
    span::{Attributes, Id, Record},
    Field, Subscriber,
};
use tracing_subscriber::{
    layer::{Context, Layer},
    prelude::*,
    registry::Registry,
};

use tracing_serde_modality_ingest::{options::GLOBAL_OPTIONS, TracingModality};
use tracing_serde_structured::{AsSerde, CowString, RecordMap, SerializeValue};
use tracing_serde_wire::TracingWire;

static START: Lazy<Instant> = Lazy::new(Instant::now);

thread_local! {
    static HANDLER: Lazy<RwLock<TSHandler>> = Lazy::new(|| {
        let opts = GLOBAL_OPTIONS.read().unwrap().clone();

        let cur = thread::current();
        let name = cur.name().map(str::to_string).unwrap_or_else(|| format!("Thread#{:?}", cur.id()));
        let opts = opts.with_name(name);

        let rt = Runtime::new().expect("create tokio runtime");
        let tracer = {
            let handle = rt.handle();
            handle.block_on(async { TracingModality::connect_with_options(opts).await.expect("connect") })
        };

        RwLock::new(TSHandler {
            rt,
            tracer,
        })
    });
}

pub fn timeline_id() -> TimelineId {
    HANDLER.with(|c| c.read().unwrap().lense.timeline_id())
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

#[non_exhaustive] // prevents raw construction
pub struct TSSubscriber;

impl TSSubscriber {
    #[allow(clippy::new_ret_no_self)]
    // this doesn't technically build a `Self`, but that's the way people should think of it
    pub fn new() -> impl Subscriber {
        Registry::default().with(TSLayer)
    }
}

pub struct TSLayer;

impl<S: Subscriber> Layer<S> for TSLayer {
    fn enabled(&self, _metadata: &tracing_core::Metadata<'_>, _ctx: Context<'_, S>) -> bool {
        // always enabled for all levels
        true
    }

    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, _ctx: Context<'_, S>) {
        let mut visitor = RecordMapBuilder::new();

        attrs.record(&mut visitor);

        let msg = TracingWire::NewSpan {
            id: id.as_serde(),
            attrs: attrs.as_serde(),
            values: visitor.values().into(),
        };

        HANDLER.with(move |c| c.write().unwrap().handle_message(msg))
    }

    fn on_record(&self, span: &Id, values: &Record<'_>, _ctx: Context<'_, S>) {
        let msg = TracingWire::Record {
            span: span.as_serde(),
            values: values.as_serde().to_owned(),
        };

        HANDLER.with(move |c| c.write().unwrap().handle_message(msg))
    }

    fn on_follows_from(&self, span: &Id, follows: &Id, _ctx: Context<'_, S>) {
        let msg = TracingWire::RecordFollowsFrom {
            span: span.as_serde(),
            follows: follows.as_serde().to_owned(),
        };

        HANDLER.with(move |c| c.write().unwrap().handle_message(msg))
    }

    fn on_event(&self, event: &tracing_core::Event<'_>, _ctx: Context<'_, S>) {
        let msg = TracingWire::Event(event.as_serde().to_owned());

        HANDLER.with(move |c| c.write().unwrap().handle_message(msg))
    }

    fn on_enter(&self, span: &Id, _ctx: Context<'_, S>) {
        let msg = TracingWire::Enter(span.as_serde());

        HANDLER.with(move |c| c.write().unwrap().handle_message(msg))
    }

    fn on_exit(&self, span: &Id, _ctx: Context<'_, S>) {
        let msg = TracingWire::Exit(span.as_serde());

        HANDLER.with(move |c| c.write().unwrap().handle_message(msg))
    }

    fn on_id_change(&self, old: &Id, new: &Id, _ctx: Context<'_, S>) {
        let msg = TracingWire::IdClone {
            old: old.as_serde(),
            new: new.as_serde(),
        };

        HANDLER.with(move |c| c.write().unwrap().handle_message(msg))
    }

    fn on_close(&self, span: Id, _ctx: Context<'_, S>) {
        let msg = TracingWire::Close(span.as_serde());

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
