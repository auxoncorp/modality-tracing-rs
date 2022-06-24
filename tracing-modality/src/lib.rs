#![doc = include_str!("../README.md")]
// required for above example, showing main isn't needless, it shows the context of where this will
// (almost) always be called from
#![allow(clippy::needless_doctest_main)]

use anyhow::Context;
use thiserror::Error;
use tracing_core::Dispatch;
use tracing_serde_modality_ingest::ConnectError;
use tracing_serde_subscriber::TSSubscriber;

pub use tracing_serde_modality_ingest::TimelineId;

/// A `tracing` collector `Layer`.
pub use tracing_serde_subscriber::TSLayer as ModalityLayer;

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

        TSSubscriber::connect().context("connect to modality")?;

        Ok(Self {})
    }

    /// Initialize with the provided options and set as the global default tracer.
    pub fn init_with_options(opt: Options) -> Result<Self, InitError> {
        let disp = Dispatch::new(TSSubscriber::new_with_options(opt));
        tracing::dispatcher::set_global_default(disp).unwrap();

        TSSubscriber::connect().context("connect to modality")?;

        Ok(Self {})
    }
}

/// Retrieve the current local timeline ID. Useful for for sending alongside data and a custom nonce
/// for recording timeline interactions on remote timelines.
pub fn timeline_id() -> TimelineId {
    tracing_serde_subscriber::timeline_id()
}
