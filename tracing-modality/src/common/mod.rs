pub(crate) mod ingest;
pub(crate) mod layer;
pub(crate) mod options;

#[cfg(doc)]
use crate::Options;
use ingest::{ConnectError, TimelineId};
use once_cell::sync::OnceCell;
use std::fmt::Debug;
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct TimelineInfo {
    // TODO: Can we make this a &'static str to avoid allocations? Maybe a Cow<str>?
    pub name: String,
    pub id: TimelineId,
}

pub(crate) static TIMELINE_IDENTIFIER: OnceCell<fn() -> TimelineInfo> = OnceCell::new();

#[derive(Debug, Error)]
pub enum InitError {
    /// No auth was provided, set with
    /// [`Options::set_auth`][crate::Options::set_auth]/[`Options::with_auth`][crate::Options::with_auth]
    /// or set the `MODALITY_AUTH_TOKEN` environment variable.
    #[error("Authentication required, set init option or env var MODALITY_AUTH_TOKEN")]
    AuthRequired,

    /// Auth was provided, but was not accepted by modality.
    #[error(transparent)]
    AuthFailed(ConnectError),

    /// Modality was initialized twice
    #[error("Modality was initialized twice")]
    InitializedTwice,

    /// Errors that it is assumed there is no way to handle without human intervention, meant for
    /// consumers to just print and carry on or panic.
    #[error(transparent)]
    UnexpectedFailure(#[from] anyhow::Error),
}

/// Retrieve the current local timeline ID. Useful for for sending alongside data and a custom nonce
/// for recording timeline interactions on remote timelines.
pub fn timeline_id() -> TimelineId {
    ingest::thread_timeline().id
}
