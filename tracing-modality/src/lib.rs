use thiserror::Error;
use tracing_core::Dispatch;
use tracing_serde_modality_ingest::options::GLOBAL_OPTIONS;
use tracing_serde_modality_ingest::ConnectError;
use tracing_serde_subscriber::TSCollector;

pub use tracing_serde_modality_ingest::{options::Options, TimelineId};

#[derive(Debug, Error)]
pub enum InitError {
    /// No auth was provided
    #[error("Authentication required, set init option or env var MODALITY_LICENSE_KEY")]
    AuthRequired,
    /// Auth was provided, but was not accepted by modality.
    #[error(transparent)]
    AuthFailed(ConnectError),
    /// Errors that it is assumed there is no way to handle without human intervention, meant for
    /// consumers to just print and carry on or panic.
    #[error(transparent)]
    UnexpectedFailure(#[from] anyhow::Error),
}

pub struct TracingModality {}

impl TracingModality {
    pub fn init() -> Result<Self, InitError> {
        let disp = Dispatch::new(TSCollector);
        tracing::dispatch::set_global_default(disp).unwrap();

        // Force a log to ensure a connection can be made, and to avoid further deferring the main thread.
        tracing::event!(tracing::Level::TRACE, "Modality connected!");

        Ok(Self {})
    }

    pub fn init_with_options(opt: Options) -> Result<Self, InitError> {
        let mut opts = GLOBAL_OPTIONS.write().unwrap();
        *opts = opt;

        Self::init()
    }
}

pub fn timeline_id() -> TimelineId {
    tracing_serde_subscriber::timeline_id()
}
