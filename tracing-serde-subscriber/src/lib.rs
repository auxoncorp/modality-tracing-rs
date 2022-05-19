use std::num::NonZeroU64;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use std::time::Instant;
use tracing_core::span::Id;
use tracing_core::{span::Current, Collect};
use tracing_serde::AsSerde;
use tracing_serde_wire::TracingWire;

pub use tracing_serde_wire::Packet;

pub struct TSCollector<H: FnMut(Packet<'_>) + Send + Sized> {
    handler: Mutex<Box<H>>,
    // TODO: Thread local counter?
    ctr: AtomicU64,
    start: Instant,
}

impl<H: 'static + std::ops::FnMut(tracing_serde_wire::Packet<'_>) + Send> TSCollector<H> {
    pub fn new(handler: H) -> Self {
        TSCollector {
            handler: Mutex::new(Box::new(handler)),
            ctr: Default::default(),
            start: Instant::now(),
        }
    }
    fn get_next_id(&self) -> tracing_core::span::Id {
        loop {
            let next = self.ctr.fetch_add(1, Ordering::SeqCst);
            if let Some(nzu64) = NonZeroU64::new(next as u64) {
                return Id::from_non_zero_u64(nzu64);
            }
        }
    }
}

impl<H: 'static + std::ops::FnMut(tracing_serde_wire::Packet<'_>) + Send> Collect
    for TSCollector<H>
{
    fn enabled(&self, _metadata: &tracing_core::Metadata<'_>) -> bool {
        // Note: nothing to log here, this is a `query` whether the trace is active
        // TODO: always enabled for now.
        true
    }

    fn new_span(&self, span: &tracing_core::span::Attributes<'_>) -> tracing_core::span::Id {
        (self.handler.try_lock().unwrap())(Packet {
            message: TracingWire::NewSpan(span.as_serde().to_owned()),
            // NOTE: will give inaccurate data if the program has run for more than 584942 years.
            tick: self.start.elapsed().as_micros() as u64,
        });
        self.get_next_id()
    }

    fn record(&self, span: &tracing_core::span::Id, values: &tracing_core::span::Record<'_>) {
        (self.handler.try_lock().unwrap())(Packet {
            message: TracingWire::Record {
                span: span.as_serde(),
                values: values.as_serde().to_owned(),
            },
            // NOTE: will give inaccurate data if the program has run for more than 584942 years.
            tick: self.start.elapsed().as_micros() as u64,
        });
    }

    fn record_follows_from(&self, span: &tracing_core::span::Id, follows: &tracing_core::span::Id) {
        (self.handler.try_lock().unwrap())(Packet {
            message: TracingWire::RecordFollowsFrom {
                span: span.as_serde(),
                follows: follows.as_serde().to_owned(),
            },
            // NOTE: will give inaccurate data if the program has run for more than 584942 years.
            tick: self.start.elapsed().as_micros() as u64,
        });
    }

    fn event(&self, event: &tracing_core::Event<'_>) {
        (self.handler.try_lock().unwrap())(Packet {
            message: TracingWire::Event(event.as_serde().to_owned()),
            // NOTE: will give inaccurate data if the program has run for more than 584942 years.
            tick: self.start.elapsed().as_micros() as u64,
        });
    }

    fn enter(&self, span: &tracing_core::span::Id) {
        (self.handler.try_lock().unwrap())(Packet {
            message: TracingWire::Enter(span.as_serde()),
            // NOTE: will give inaccurate data if the program has run for more than 584942 years.
            tick: self.start.elapsed().as_micros() as u64,
        });
        // self.encode(TracingWire::Enter(span.as_serde()))
    }

    fn exit(&self, span: &tracing_core::span::Id) {
        (self.handler.try_lock().unwrap())(Packet {
            message: TracingWire::Exit(span.as_serde()),
            // NOTE: will give inaccurate data if the program has run for more than 584942 years.
            tick: self.start.elapsed().as_micros() as u64,
        });
        // self.encode(TracingWire::Exit(span.as_serde()))
    }

    fn current_span(&self) -> tracing_core::span::Current {
        Current::unknown()
    }
}
