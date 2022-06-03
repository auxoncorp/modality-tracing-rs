use tracing_core::Dispatch;
use tracing_serde_modality_ingest::options::GLOBAL_OPTIONS;
use tracing_serde_subscriber::TSCollector;

pub use tracing_serde_modality_ingest::{options::Options, TimelineId};

pub struct TracingModality {}

impl TracingModality {
    pub fn init() -> Self {
        let disp = Dispatch::new(TSCollector);
        tracing::dispatch::set_global_default(disp).unwrap();

        // Force a log to ensure a connection can be made, and to avoid further deferring the main thread.
        tracing::event!(tracing::Level::TRACE, "Modality connected!");

        Self {}
    }

    pub fn init_with_options(opt: Options) -> Self {
        let mut opts = GLOBAL_OPTIONS.write().unwrap();
        *opts = opt;

        Self::init()
    }
}

pub fn timeline_id() -> TimelineId {
    tracing_serde_subscriber::timeline_id()
}
