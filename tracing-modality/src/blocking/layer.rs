use crate::common::options::Options;
use crate::InitError;

use crate::common::layer::{LayerHandler, LocalMetadata};
use crate::ingest;
use crate::ingest::{ModalityIngest, ModalityIngestThreadHandle, WrappedMessage};

use anyhow::Context as _;
use once_cell::sync::Lazy;
use std::{cell::Cell, thread::LocalKey, thread_local};
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
    thread_local! {
        static LOCAL_METADATA: Lazy<LocalMetadata> = Lazy::new(|| {
            LocalMetadata {
                thread_timeline: ingest::current_timeline(),
            }
        });
        static THREAD_TIMELINE_INITIALIZED: Cell<bool> = const { Cell::new(false) };
    }

    /// Initialize a new `ModalityLayer`, with default options.
    pub fn init() -> Result<(Self, ModalityIngestThreadHandle), InitError> {
        Self::init_with_options(Default::default())
    }

    /// Initialize a new `ModalityLayer`, with specified options.
    pub fn init_with_options(
        mut opts: Options,
    ) -> Result<(Self, ModalityIngestThreadHandle), InitError> {
        let run_id = Uuid::new_v4();
        opts.add_metadata("run_id", run_id.to_string());

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

    fn local_metadata(&self) -> &'static LocalKey<Lazy<LocalMetadata>> {
        &Self::LOCAL_METADATA
    }

    fn thread_timeline_initialized(&self) -> &'static LocalKey<Cell<bool>> {
        &Self::THREAD_TIMELINE_INITIALIZED
    }
}
