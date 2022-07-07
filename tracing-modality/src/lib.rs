#![doc = include_str!("../README.md")]
// required for above example, showing main isn't needless, it shows the context of where this will
// (almost) always be called from
#![allow(clippy::needless_doctest_main)]

mod ingest;
mod layer;
pub mod options;

pub use ingest::TimelineId;
pub use layer::ModalityLayer;
pub use options::Options;

use anyhow::Context as _;
use ingest::{ConnectError, ModalityIngestHandle};
use std::fmt::Debug;
use thiserror::Error;
use tracing_core::Dispatch;

#[derive(Debug, Error)]
pub enum InitError {
    /// No auth was provided, set with [`Options::set_auth`]/[`Options::with_auth`] or set the
    /// `MODALITY_AUTH_TOKEN` environment variable.
    #[error("Authentication required, set init option or env var MODALITY_AUTH_TOKEN")]
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
pub struct TracingModality {
    ingest_handle: ModalityIngestHandle,
}

impl TracingModality {
    /// Initialize with default options and set as the global default tracer.
    pub fn init() -> Result<Self, InitError> {
        Self::init_with_options(Default::default())
    }

    /// Initialize with the provided options and set as the global default tracer.
    pub fn init_with_options(opts: Options) -> Result<Self, InitError> {
        let mut layer =
            ModalityLayer::init_with_options(opts).context("initialize ModalityLayer")?;
        let ingest_handle = layer
            .take_handle()
            .expect("take handle on brand new layer somehow failed");

        let disp = Dispatch::new(layer.into_subscriber());
        tracing::dispatcher::set_global_default(disp).unwrap();

        Ok(Self { ingest_handle })
    }

    /// Stop accepting new trace events, flush all existing events, and stop ingest thread.
    pub fn finish(&mut self) {
        self.ingest_handle.finish();
    }
}

/// Retrieve the current local timeline ID. Useful for for sending alongside data and a custom nonce
/// for recording timeline interactions on remote timelines.
pub fn timeline_id() -> TimelineId {
    ingest::current_timeline()
}
