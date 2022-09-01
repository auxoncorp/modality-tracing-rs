//! Attribute Handlers
//!
//! Attribute handlers allow for project-specific modifications of `tracing` data
//! provided when events occur.
//!
//! This is typically needed when data needs to be modified in some way before reporting
//! to modality, or if a translation from "project specific" keys or values to "modality
//! specific" keys or values are necessary.
//!
//! Handlers are matched on a SPECIFIC key text. If a matching key text is found, the
//! provided handler will be called, will replace the key and value of the data reported
//! by `tracing`.
//!
//! ## Default Handlers
//!
//! By default, the following handlers are defined:
//!
//! | tracing key                      | modality key                           | value conversion         |
//! | :---                             | :---                                   | :---                     |
//! | "timestamp"                      | "event.timestamp"                      | see [timestamp]          |
//! | "interaction.remote_timestamp"   | "event.interaction.remote_timestamp"   | see [remote_timestamp]   |
//! | "interaction.remote_timeline_id" | "event.interaction.remote_timeline_id" | see [remote_timeline_id] |
//! | "name"                           | "event.name"                           | see [name]               |
//! | "message"                        | "event.message"                        | see [name]               |
//! | "severity"                       | "event.severity"                       | see [severity]           |
//! | "source.module"                  | "event.source.module"                  | see [source_module]      |
//! | "source.file"                    | "event.source.file"                    | see [source_file]        |
//!
//! These default handlers can be overridden using the [Options::set_attr_handlers()](crate::Options::set_attr_handlers)
//! or [Options::with_attr_handlers()](crate::Options::with_attr_handlers) functions.
//!
//! ## Fallback handler
//!
//! Additionally, there is a "fallback handler" that will take any tracing key NOT matched by the attribute
//! handlers by ensuring that the given key starts with "event.*", and the contained data is converted into
//! a format usable by modality. If this conversion is suitable for data logged by your application, then no
//! specific attribute handler needs to be provided.
//!
//! At the moment, this fallback handler cannot be configured, replaced, or disabled.
//!
//! ## Tracing Metadata
//!
//! Additionally, `tracing-modality` will provide the following event keys with data sourced from `tracing`'s
//! metadata fields, IF the values have not already been filled by any of the Default or Fallback handlers.
//!
//! | modality key             | tracing metadata (or other) source           |
//! | :---                     | :---                                         |
//! | "event.name"             | `metadata.name()`                            |
//! | "event.severity"         | `metadata.level()`                           |
//! | "event.source.module"    | `metadata.module_path()`                     |
//! | "event.source.file"      | `metadata.file()`                            |
//! | "event.source.line"      | `metadata.line()`                            |
//! | "event.internal.rs.tick" | nanosecond ticks since first tracing event   |
//! | "event.timestamp"        | nanosecond ticks since unix epoch            |
//!

use std::borrow::Cow;

pub use crate::{ingest::tracing_value_to_attr_val, layer::TracingValue};
pub use modality_ingest_client::types::AttrVal;

use modality_ingest_client::types::{Nanoseconds, TimelineId, Uuid};

/// A type alias for the signature required for attribute handlers
pub type HandlerFunc =
    fn(tracing_key: &str, tracing_val: TracingValue, run_id: &Uuid) -> (Cow<'static, str>, AttrVal);

/// Convert a run_id (UUIDv4) and user timeline id (u64) into a Modality TimelineId
///
/// This function is intended to be used within attribute handlers.
pub fn timeline_id(run_id: &Uuid, user_timeline_id: u64) -> TimelineId {
    uuid::Uuid::new_v5(run_id, &user_timeline_id.to_ne_bytes()).into()
}

/// The "fallback handler" which converts all attributes not matched to a specific
/// handler.
//
// Note, this is NOT a handler! It requires a wildcard/"starts with" match.
pub fn event_fallback(
    tracing_key: Cow<'static, str>,
    tracing_val: TracingValue,
) -> (Cow<'static, str>, AttrVal) {
    let attrval = tracing_value_to_attr_val(tracing_val);

    let key = if tracing_key.starts_with("event.") {
        tracing_key
    } else {
        format!("event.{}", tracing_key).into()
    };

    (key, attrval)
}

// HANDLERS

// NOTE: in our handlers, we generally ignore the `tracing_key`, as we do
// not change any behavior based on this information. This field is provided
// in the case it is necessary for user usage.

/// Convert a timestamp (in nanoseconds) into a Modality timestamp type
///
/// Used for converting timeline-local timestamps.
pub fn timestamp(
    _tracing_key: &str,
    tracing_val: TracingValue,
    _run_id: &Uuid,
) -> (Cow<'static, str>, AttrVal) {
    (
        Cow::Borrowed("event.timestamp"),
        timestamp_inner(tracing_val),
    )
}

/// Convert a timestamp (in nanoseconds) into a Modality timestamp type
///
/// Used for converting timeline-remote timestamps.
pub fn remote_timestamp(
    _tracing_key: &str,
    tracing_val: TracingValue,
    _run_id: &Uuid,
) -> (Cow<'static, str>, AttrVal) {
    (
        Cow::Borrowed("event.interaction.remote_timestamp"),
        timestamp_inner(tracing_val),
    )
}

/// Convert a remote timeline id (as a u64) into a Modality TimelineId
pub fn remote_timeline_id(
    _tracing_key: &str,
    tracing_val: TracingValue,
    run_id: &Uuid,
) -> (Cow<'static, str>, AttrVal) {
    let attrval = match tracing_val {
        TracingValue::U64(id) => {
            let remote_tid = timeline_id(run_id, id);
            AttrVal::TimelineId(Box::new(remote_tid))
        }
        other => tracing_value_to_attr_val(other),
    };

    (
        Cow::Borrowed("event.interaction.remote_timeline_id"),
        attrval,
    )
}

/// Convert an event name
pub fn name(
    _tracing_key: &str,
    tracing_val: TracingValue,
    _run_id: &Uuid,
) -> (Cow<'static, str>, AttrVal) {
    let attrval = tracing_value_to_attr_val(tracing_val);
    (Cow::Borrowed("event.name"), attrval)
}

/// Convert a tracing severity (or "log level") to a Modality severity level
pub fn severity(
    _tracing_key: &str,
    tracing_val: TracingValue,
    _run_id: &Uuid,
) -> (Cow<'static, str>, AttrVal) {
    let attrval = tracing_value_to_attr_val(tracing_val);
    (Cow::Borrowed("event.severity"), attrval)
}

/// Process the source module path
pub fn source_module(
    _tracing_key: &str,
    tracing_val: TracingValue,
    _run_id: &Uuid,
) -> (Cow<'static, str>, AttrVal) {
    let attrval = tracing_value_to_attr_val(tracing_val);
    (Cow::Borrowed("event.source.module"), attrval)
}

/// Process the source module file
pub fn source_file(
    _tracing_key: &str,
    tracing_val: TracingValue,
    _run_id: &Uuid,
) -> (Cow<'static, str>, AttrVal) {
    let attrval = tracing_value_to_attr_val(tracing_val);
    (Cow::Borrowed("event.source.file"), attrval)
}

fn timestamp_inner(tracing_val: TracingValue) -> AttrVal {
    let attrval = tracing_value_to_attr_val(tracing_val);

    match attrval {
        AttrVal::Integer(i) if i >= 0 => AttrVal::Timestamp(Nanoseconds::from(i as u64)),
        AttrVal::BigInt(i) if *i >= 0 && *i <= u64::MAX as i128 => {
            AttrVal::Timestamp(Nanoseconds::from(*i as u64))
        }
        AttrVal::Timestamp(t) => AttrVal::Timestamp(t),
        x => x,
    }
}

#[derive(Clone, Copy)]
pub struct AttributeHandler {
    pub(crate) key: &'static str,
    pub(crate) handler: HandlerFunc,
}

impl AttributeHandler {
    pub const fn new(key: &'static str, handler: HandlerFunc) -> Self {
        Self { key, handler }
    }

    pub(crate) const fn as_cow_key(&self) -> (Cow<'static, str>, HandlerFunc) {
        (Cow::Borrowed(self.key), self.handler)
    }

    pub fn default_handlers() -> Vec<AttributeHandler> {
        DEFAULT_HANDLERS.to_vec()
    }
}

pub(crate) const DEFAULT_HANDLERS: &[AttributeHandler] = &[
    AttributeHandler::new("timestamp", timestamp),
    AttributeHandler::new("interaction.remote_timestamp", remote_timestamp),
    AttributeHandler::new("interaction.remote_timeline_id", remote_timeline_id),
    AttributeHandler::new("name", name),
    AttributeHandler::new("message", name),
    AttributeHandler::new("severity", severity),
    AttributeHandler::new("source.module", source_module),
    AttributeHandler::new("source.file", source_file),
];
