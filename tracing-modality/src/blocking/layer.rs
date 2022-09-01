use crate::common::options::Options;
use crate::{InitError, TIMELINE_IDENTIFIER};

use crate::common::layer::LayerHandler;
use crate::ingest::{ModalityIngest, ModalityIngestThreadHandle, WrappedMessage};

use anyhow::Context as _;
use tokio::sync::mpsc::{self, UnboundedSender};
use tracing_core::Subscriber;
use tracing_subscriber::{layer::SubscriberExt, Registry};

/// A `tracing` `Layer` that can be used to record trace events and stream them to modality in real
/// time.
///
/// Can be transformed into a `Subscriber` with [`ModalityLayer::into_subscriber()`].
pub struct ModalityLayer {
    sender: UnboundedSender<WrappedMessage>,
}

impl ModalityLayer {
    /// Initialize a new `ModalityLayer`, with default options.
    pub fn init() -> Result<(Self, ModalityIngestThreadHandle), InitError> {
        Self::init_with_options(Default::default())
    }

    /// Initialize a new `ModalityLayer`, with specified options.
    pub fn init_with_options(
        opts: Options,
    ) -> Result<(Self, ModalityIngestThreadHandle), InitError> {
        TIMELINE_IDENTIFIER
            .set(opts.timeline_identifier)
            .map_err(|_| InitError::InitializedTwice)?;

        let ingest = ModalityIngest::connect(opts).context("connect to modality")?;
        let ingest_handle = ingest.spawn_thread();
        let sender = ingest_handle.ingest_sender.clone();

        Ok((ModalityLayer { sender }, ingest_handle))
    }

    /// Convert this `Layer` into a `Subscriber`by by layering it on a new instace of `tracing`'s
    /// `Registry`.
    pub fn into_subscriber(self) -> impl Subscriber {
        Registry::default().with(self)
    }
}

impl LayerHandler for ModalityLayer {
    fn send(&self, msg: WrappedMessage) -> Result<(), mpsc::error::SendError<WrappedMessage>> {
        self.sender.send(msg)
    }
}
