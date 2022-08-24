pub(crate) mod attr_handlers;
pub(crate) mod ingest;
pub(crate) mod layer;
pub(crate) mod options;
pub(crate) mod timeline_lru;

#[cfg(doc)]
use crate::Options;
use ingest::ConnectError;
use once_cell::sync::OnceCell;
use std::fmt::Debug;
use thiserror::Error;

/// A structure representing the information tracked for a given Modality Timeline
#[derive(Debug, Clone)]
pub struct UserTimelineInfo {
    pub(crate) name: String,
    pub(crate) user_id: u64,
}

impl UserTimelineInfo {
    /// Create a new TimelineInfo structure
    pub fn new(name: String, user_id: u64) -> Self {
        Self { name, user_id }
    }

    /// Retrieve the name of the timeline
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Retrieve the [TimelineId](crate::TimelineId) of the timeline
    pub fn user_id(&self) -> u64 {
        self.user_id
    }
}

// This holds the callback function used to identify the current timeline.
//
// By *default*, this uses a thread-local to store unique information per-thread,
// though this may be overriden by the user via the Options structure at initialization
// time.
pub(crate) static TIMELINE_IDENTIFIER: OnceCell<fn() -> UserTimelineInfo> = OnceCell::new();

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

/// Retrieve the current user timeline ID. Useful for for sending alongside data and a custom nonce
/// for recording timeline interactions on remote timelines.
pub fn timeline_id() -> u64 {
    let f = TIMELINE_IDENTIFIER
        .get()
        .expect("Modality should be initialized before getting current timeline id");

    (*f)().user_id
}
