use tracing_core::Dispatch;
use tracing_serde_subscriber::{TSCollector};

pub struct TracingModality {}

impl TracingModality {
    pub fn init() -> Self {
        let disp = Dispatch::new(TSCollector);
        tracing::dispatch::set_global_default(disp).unwrap();

        // Force a log to ensure a connection can be made, and to avoid further deferring the main thread.
        tracing::event!(tracing::Level::TRACE, "Modality connected!");

        Self {}
    }
}
