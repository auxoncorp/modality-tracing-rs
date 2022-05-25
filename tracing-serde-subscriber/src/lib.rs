use std::num::NonZeroU64;
use std::sync::Mutex;
use std::time::Instant;
use tracing_core::span::Id;
use tracing_core::{span::Current, Collect};
use tracing_serde::AsSerde;
use tracing_serde_wire::TracingWire;
pub use tracing_serde_wire::Packet;
use tracing_serde_modality_ingest::TracingModalityLense;

use std::thread_local;
use once_cell::sync::Lazy;
use tokio::runtime::Runtime;

thread_local! {
    static COLLECTOR: Lazy<Mutex<Collector>> = Lazy::new(|| {
        let rt = Runtime::new().expect("create tokio runtime");
        let lense = {
            let handle = rt.handle();
            handle.block_on(async { TracingModalityLense::connect().await.expect("connect") })
        };

        Mutex::new(Collector {
            rt,
            lense,
            start: Instant::now(),
            id: 1,
        })
    });
}

pub struct Collector {
    lense: TracingModalityLense,
    rt: Runtime,
    start: Instant,
    id: u64,
}

impl Collector {
    fn get_next_id(&mut self) -> Id {
        loop {
            self.id = self.id.wrapping_add(1);
            if let Some(id) = NonZeroU64::new(self.id) {
                return Id::from_non_zero_u64(id);
            }
        }
    }
}

impl Collector {
    fn enabled(&mut self, _metadata: &tracing_core::Metadata<'_>) -> bool {
        // Note: nothing to log here, this is a `query` whether the trace is active
        // TODO: always enabled for now.
        true
    }

    fn new_span(&mut self, span: &tracing_core::span::Attributes<'_>) -> tracing_core::span::Id {
        self.rt.handle().block_on(async {
            self.lense.handle_packet(Packet {
                message: TracingWire::NewSpan(span.as_serde().to_owned()),
                // NOTE: will give inaccurate data if the program has run for more than 584942 years.
                tick: self.start.elapsed().as_micros() as u64,
            }).await
        }).unwrap();
        self.get_next_id()
    }

    fn record(&mut self, span: &tracing_core::span::Id, values: &tracing_core::span::Record<'_>) {
        self.rt.handle().block_on(async {
            self.lense.handle_packet(Packet {
                message: TracingWire::Record {
                    span: span.as_serde(),
                    values: values.as_serde().to_owned(),
                },
                // NOTE: will give inaccurate data if the program has run for more than 584942 years.
                tick: self.start.elapsed().as_micros() as u64,
            }).await
        }).unwrap();
    }

    fn record_follows_from(&mut self, span: &tracing_core::span::Id, follows: &tracing_core::span::Id) {
        self.rt.handle().block_on(async {
            self.lense.handle_packet(Packet {
                message: TracingWire::RecordFollowsFrom {
                    span: span.as_serde(),
                    follows: follows.as_serde().to_owned(),
                },
                // NOTE: will give inaccurate data if the program has run for more than 584942 years.
                tick: self.start.elapsed().as_micros() as u64,
            }).await
        }).unwrap();
    }

    fn event(&mut self, event: &tracing_core::Event<'_>) {
        self.rt.handle().block_on(async {
                self.lense.handle_packet(Packet {
                message: TracingWire::Event(event.as_serde().to_owned()),
                // NOTE: will give inaccurate data if the program has run for more than 584942 years.
                tick: self.start.elapsed().as_micros() as u64,
            }).await
        }).unwrap();
    }

    fn enter(&mut self, span: &tracing_core::span::Id) {
        self.rt.handle().block_on(async {
            self.lense.handle_packet(Packet {
                message: TracingWire::Enter(span.as_serde()),
                // NOTE: will give inaccurate data if the program has run for more than 584942 years.
                tick: self.start.elapsed().as_micros() as u64,
            }).await
        }).unwrap();
    }

    fn exit(&mut self, span: &tracing_core::span::Id) {
        self.rt.handle().block_on(async {
            self.lense.handle_packet(Packet {
                message: TracingWire::Exit(span.as_serde()),
                // NOTE: will give inaccurate data if the program has run for more than 584942 years.
                tick: self.start.elapsed().as_micros() as u64,
           }).await
        }).unwrap();
    }

    fn current_span(&self) -> tracing_core::span::Current {
        Current::unknown()
    }
}

pub struct TSCollector;

impl Collect for TSCollector
{
    fn enabled(&self, metadata: &tracing_core::Metadata<'_>) -> bool {
        COLLECTOR.with(|c| c.lock().unwrap().enabled(metadata))
    }

    fn new_span(&self, span: &tracing_core::span::Attributes<'_>) -> tracing_core::span::Id {
        COLLECTOR.with(|c| c.lock().unwrap().new_span(span))
    }

    fn record(&self, span: &tracing_core::span::Id, values: &tracing_core::span::Record<'_>) {
        COLLECTOR.with(|c| c.lock().unwrap().record(span, values))
    }

    fn record_follows_from(&self, span: &tracing_core::span::Id, follows: &tracing_core::span::Id) {
        COLLECTOR.with(|c| c.lock().unwrap().record_follows_from(span, follows))
    }

    fn event(&self, event: &tracing_core::Event<'_>) {
        COLLECTOR.with(|c| c.lock().unwrap().event(event))
    }

    fn enter(&self, span: &tracing_core::span::Id) {
        COLLECTOR.with(|c| c.lock().unwrap().enter(span))
    }

    fn exit(&self, span: &tracing_core::span::Id) {
        COLLECTOR.with(|c| c.lock().unwrap().exit(span))
    }

    fn current_span(&self) -> tracing_core::span::Current {
        COLLECTOR.with(|c| c.lock().unwrap().current_span())
    }
}
