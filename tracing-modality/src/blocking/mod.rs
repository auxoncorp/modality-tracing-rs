pub mod layer;

pub use crate::{timeline_id, InitError, ModalityIngestThreadHandle, Options, TimelineId};
pub use layer::ModalityLayer;

use anyhow::Context as _;
use tracing::Dispatch;

/// A global tracer instance for [tracing.rs](https://tracing.rs/) that sends traces via a network
/// socket to [Modality](https://auxon.io/).
///
/// This is the synchronous version of `TracingModality`, it must not be initialized or `finish`ed
/// from within a tokio runtime. See [`crate::TracingModality`] for a version that can be
/// initialized inside a tokio runtime. Both versions support tracing from within and outside of a
/// tokio runtime.

pub struct TracingModality {
    ingest_handle: ModalityIngestThreadHandle,
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
    pub fn finish(self) {
        self.ingest_handle.finish();
    }
}
