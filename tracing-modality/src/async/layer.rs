use crate::common::options::Options;
use crate::{InitError, TIMELINE_IDENTIFIER};

use crate::common::layer::LayerHandler;
use crate::ingest::{ModalityIngest, ModalityIngestTaskHandle, WrappedMessage};

use anyhow::Context as _;
use tokio::sync::mpsc::{self, UnboundedSender};
use tracing_core::Subscriber;
use tracing_subscriber::{layer::SubscriberExt, Registry};
use uuid::Uuid;

/// A `tracing` `Layer` that can be used to record trace events and stream them to modality in real
/// time.
///
/// Can be transformed into a `Subscriber` with [`ModalityLayer::into_subscriber()`].
pub struct ModalityLayer {
    sender: UnboundedSender<WrappedMessage>,
}

impl ModalityLayer {
    /// Initialize a new `ModalityLayer`, with default options.
    pub async fn init() -> Result<(Self, ModalityIngestTaskHandle), InitError> {
        Self::init_with_options(Default::default()).await
    }

    /// Initialize a new `ModalityLayer`, with specified options.
    pub async fn init_with_options(
        mut opts: Options,
    ) -> Result<(Self, ModalityIngestTaskHandle), InitError> {
        let run_id = Uuid::new_v4();
        opts.add_metadata("run_id", run_id.to_string());

        TIMELINE_IDENTIFIER
            .set(opts.timeline_identifier)
            .map_err(|_| InitError::InitializedTwice)?;

        let ingest = ModalityIngest::async_connect(opts)
            .await
            .context("connect to modality")?;
        let ingest_handle = ingest.spawn_task().await;
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
