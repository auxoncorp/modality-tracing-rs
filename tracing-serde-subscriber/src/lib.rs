pub use tracing_serde_modality_ingest::TimelineId;
pub use tracing_serde_wire::Packet;

use std::{fmt::Debug, num::NonZeroU64, sync::RwLock, thread, thread_local, time::Instant};

use once_cell::sync::Lazy;
use tokio::runtime::Runtime;
use tracing_core::{field::Visit, span::Id, Field, Subscriber};

use tracing_serde_modality_ingest::{options::GLOBAL_OPTIONS, TracingModalityLense};
use tracing_serde_structured::{AsSerde, CowString, RecordMap, SerializeValue};
use tracing_serde_wire::TracingWire;

static START: Lazy<Instant> = Lazy::new(Instant::now);

thread_local! {
    static SUBSCRIBER: Lazy<RwLock<TSSubscriber>> = Lazy::new(|| {
        let opts = GLOBAL_OPTIONS.read().unwrap().clone();

        let cur = thread::current();
        let name = cur.name().map(str::to_string).unwrap_or_else(|| format!("Thread#{:?}", cur.id()));
        let opts = opts.with_name(name);

        let rt = Runtime::new().expect("create tokio runtime");
        let lense = {
            let handle = rt.handle();
            handle.block_on(async { TracingModalityLense::connect_with_options(opts).await.expect("connect") })
        };

        RwLock::new(TSSubscriber {
            rt,
            lense,
            id: 1,
        })
    });
}

pub fn timeline_id() -> TimelineId {
    SUBSCRIBER.with(|c| c.read().unwrap().lense.timeline_id())
}

pub struct TSSubscriber {
    lense: TracingModalityLense,
    rt: Runtime,
    id: u64,
}

impl TSSubscriber {
    fn get_next_id(&mut self) -> Id {
        loop {
            self.id = self.id.wrapping_add(1);
            if let Some(id) = NonZeroU64::new(self.id) {
                return Id::from_non_zero_u64(id);
            }
        }
    }
}

impl TSSubscriber {
    fn enabled(&self, _metadata: &tracing_core::Metadata<'_>) -> bool {
        // Note: nothing to log here, this is a `query` whether the trace is active
        // TODO: always enabled for now.
        true
    }

    fn new_span(&mut self, span: &tracing_core::span::Attributes<'_>) -> tracing_core::span::Id {
        let id = self.get_next_id();

        let mut visitor = RecordMapBuilder::new();

        span.record(&mut visitor);

        self.rt
            .handle()
            .block_on(async {
                self.lense
                    .handle_packet(Packet {
                        message: TracingWire::NewSpan {
                            id: id.as_serde(),
                            attrs: span.as_serde().to_owned(),
                            values: visitor.values().into(),
                        },
                        // NOTE: will give inaccurate data if the program has run for more than 584942 years.
                        tick: START.elapsed().as_micros() as u64,
                    })
                    .await
            })
            .unwrap();

        id
    }

    fn record(&mut self, span: &tracing_core::span::Id, values: &tracing_core::span::Record<'_>) {
        self.rt
            .handle()
            .block_on(async {
                self.lense
                    .handle_packet(Packet {
                        message: TracingWire::Record {
                            span: span.as_serde(),
                            values: values.as_serde().to_owned(),
                        },
                        // NOTE: will give inaccurate data if the program has run for more than 584942 years.
                        tick: START.elapsed().as_micros() as u64,
                    })
                    .await
            })
            .unwrap();
    }

    fn record_follows_from(
        &mut self,
        span: &tracing_core::span::Id,
        follows: &tracing_core::span::Id,
    ) {
        self.rt
            .handle()
            .block_on(async {
                self.lense
                    .handle_packet(Packet {
                        message: TracingWire::RecordFollowsFrom {
                            span: span.as_serde(),
                            follows: follows.as_serde().to_owned(),
                        },
                        // NOTE: will give inaccurate data if the program has run for more than 584942 years.
                        tick: START.elapsed().as_micros() as u64,
                    })
                    .await
            })
            .unwrap();
    }

    fn event(&mut self, event: &tracing_core::Event<'_>) {
        self.rt
            .handle()
            .block_on(async {
                self.lense
                    .handle_packet(Packet {
                        message: TracingWire::Event(event.as_serde().to_owned()),
                        // NOTE: will give inaccurate data if the program has run for more than 584942 years.
                        tick: START.elapsed().as_micros() as u64,
                    })
                    .await
            })
            .unwrap();
    }

    fn enter(&mut self, span: &tracing_core::span::Id) {
        self.rt
            .handle()
            .block_on(async {
                self.lense
                    .handle_packet(Packet {
                        message: TracingWire::Enter(span.as_serde()),
                        // NOTE: will give inaccurate data if the program has run for more than 584942 years.
                        tick: START.elapsed().as_micros() as u64,
                    })
                    .await
            })
            .unwrap();
    }

    fn exit(&mut self, span: &tracing_core::span::Id) {
        self.rt
            .handle()
            .block_on(async {
                self.lense
                    .handle_packet(Packet {
                        message: TracingWire::Exit(span.as_serde()),
                        // NOTE: will give inaccurate data if the program has run for more than 584942 years.
                        tick: START.elapsed().as_micros() as u64,
                    })
                    .await
            })
            .unwrap();
    }
}

pub struct TSCollector;

impl Subscriber for TSCollector {
    fn enabled(&self, metadata: &tracing_core::Metadata<'_>) -> bool {
        SUBSCRIBER.with(|c| c.read().unwrap().enabled(metadata))
    }

    fn new_span(&self, span: &tracing_core::span::Attributes<'_>) -> tracing_core::span::Id {
        SUBSCRIBER.with(|c| c.write().unwrap().new_span(span))
    }

    fn record(&self, span: &tracing_core::span::Id, values: &tracing_core::span::Record<'_>) {
        SUBSCRIBER.with(|c| c.write().unwrap().record(span, values))
    }

    fn record_follows_from(&self, span: &tracing_core::span::Id, follows: &tracing_core::span::Id) {
        SUBSCRIBER.with(|c| c.write().unwrap().record_follows_from(span, follows))
    }

    fn event(&self, event: &tracing_core::Event<'_>) {
        SUBSCRIBER.with(|c| c.write().unwrap().event(event))
    }

    fn enter(&self, span: &tracing_core::span::Id) {
        SUBSCRIBER.with(|c| c.write().unwrap().enter(span))
    }

    fn exit(&self, span: &tracing_core::span::Id) {
        SUBSCRIBER.with(|c| c.write().unwrap().exit(span))
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
