#![doc = include_str!("../README.md")]
// required for above example, showing main isn't needless, it shows the context of where this will
// (almost) always be called from
#![allow(clippy::needless_doctest_main)]

#[cfg(feature = "async")]
mod r#async;
#[cfg(feature = "blocking")]
pub mod blocking;
mod common;

#[cfg(feature = "async")]
pub use common::ingest::ModalityIngestTaskHandle;
#[cfg(feature = "blocking")]
pub use common::ingest::ModalityIngestThreadHandle;

pub use common::ingest::TimelineId;
pub use common::options::Options;
pub use common::*;

#[cfg(feature = "async")]
pub use r#async::{ModalityLayer, TracingModality};
