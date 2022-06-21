//! `tracing-modality` provides a [tracing.rs](https://tracing.rs) `Collector` for tracing rust
//! programs to [Auxon Modality](https://auxon.io).
//!
//! The quickest (and as of this version, only) way to get started is to let [`TracingModality`]
//! register itself as the global default tracer, done most simply using [`TracingModality::init()`]
//!
//! ```rust
//!
//! use tracing::debug;
//! use tracing_modality::TracingModality;
//! use tracing::info;
//!
//! fn main() {
//!     TracingModality::init().expect("init");
//!
//!     info!("my application has started");
//! }
//! ```
//!
//! Some basic configuration options are also available to be set at init with
//! [`TracingModality::init_with_options`].

// required for above example, showing main isn't needless, it shows the context of where this will
// (almost) always be called from
#![allow(clippy::needless_doctest_main)]

use thiserror::Error;
use tracing_core::Dispatch;
use tracing_serde_modality_ingest::options::GLOBAL_OPTIONS;
use tracing_serde_modality_ingest::ConnectError;
use tracing_serde_subscriber::TSSubscriber;

pub use tracing_serde_modality_ingest::TimelineId;

/// Options used with [`TracingModality::init_with_options`].
pub use tracing_serde_modality_ingest::options::Options;

#[derive(Debug, Error)]
pub enum InitError {
    /// No auth was provided, set with [`Options::set_auth`]/[`Options::with_auth`] or set the
    /// `MODALITY_LICENSE KEY` environment variable.
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

/// A global tracer instance for [tracing.rs](https://tracing.rs/) that sends traces via a network
/// socket to [Modality](https://auxon.io/).
pub struct TracingModality {}

impl TracingModality {
    /// Initialize with default options and set as the global default tracer.
    pub fn init() -> Result<Self, InitError> {
        let disp = Dispatch::new(TSSubscriber::new());
        tracing::dispatcher::set_global_default(disp).unwrap();

        // Force a log to ensure a connection can be made, and to avoid further deferring the main thread.
        tracing::event!(tracing::Level::TRACE, "Modality connected!");

        Ok(Self {})
    }

    /// Initialize with the provided options and set as the global default tracer.
    pub fn init_with_options(opt: Options) -> Result<Self, InitError> {
        let mut opts = GLOBAL_OPTIONS.write().unwrap();
        *opts = opt;
        drop(opts);

        Self::init()
    }
}

/// Retrieve the current local timeline ID. Useful for for sending alongside data and a custom nonce
/// for recording timeline interactions on remote timelines.
pub fn timeline_id() -> TimelineId {
    tracing_serde_subscriber::timeline_id()
}
