use std::borrow::Cow;

use modality_ingest_client::types::{AttrVal, Nanoseconds, Uuid};

use crate::{ingest::tracing_value_to_attr_val, layer::TracingValue};

pub type AttrHandler = fn(
    tracing_key: Cow<'static, str>,
    tracing_val: TracingValue,
    run_id: &Uuid,
) -> (Cow<'static, str>, AttrVal);

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

pub fn timestamp(
    _tracing_key: Cow<'static, str>,
    tracing_val: TracingValue,
    _run_id: &Uuid,
) -> (Cow<'static, str>, AttrVal) {
    (
        Cow::Borrowed("event.timestamp"),
        timestamp_inner(tracing_val),
    )
}

pub fn remote_timestamp(
    _tracing_key: Cow<'static, str>,
    tracing_val: TracingValue,
    _run_id: &Uuid,
) -> (Cow<'static, str>, AttrVal) {
    (
        Cow::Borrowed("event.interaction.remote_timestamp"),
        timestamp_inner(tracing_val),
    )
}

pub fn remote_timeline_id(
    _tracing_key: Cow<'static, str>,
    tracing_val: TracingValue,
    run_id: &Uuid,
) -> (Cow<'static, str>, AttrVal) {
    let attrval = match tracing_val {
        TracingValue::U64(id) => {
            let remote_tid = uuid::Uuid::new_v5(run_id, &id.to_ne_bytes()).into();
            AttrVal::TimelineId(Box::new(remote_tid))
        }
        other => tracing_value_to_attr_val(other),
    };

    (
        Cow::Borrowed("event.interaction.remote_timeline_id"),
        attrval,
    )
}

pub fn name(
    _tracing_key: Cow<'static, str>,
    tracing_val: TracingValue,
    _run_id: &Uuid,
) -> (Cow<'static, str>, AttrVal) {
    let attrval = tracing_value_to_attr_val(tracing_val);
    (Cow::Borrowed("event.name"), attrval)
}

pub fn severity(
    _tracing_key: Cow<'static, str>,
    tracing_val: TracingValue,
    _run_id: &Uuid,
) -> (Cow<'static, str>, AttrVal) {
    let attrval = tracing_value_to_attr_val(tracing_val);
    (Cow::Borrowed("event.severity"), attrval)
}

pub fn source_module(
    _tracing_key: Cow<'static, str>,
    tracing_val: TracingValue,
    _run_id: &Uuid,
) -> (Cow<'static, str>, AttrVal) {
    let attrval = tracing_value_to_attr_val(tracing_val);
    (Cow::Borrowed("event.source.module"), attrval)
}

pub fn source_file(
    _tracing_key: Cow<'static, str>,
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

pub(crate) const DEFAULT_HANDLERS: &[(&str, AttrHandler)] = &[
    ("timestamp", timestamp),
    ("interaction.remote_timestamp", remote_timestamp),
    ("event.interaction.remote_timeline_id", remote_timeline_id),
    ("name", name),
    ("message", name),
    ("severity", severity),
    ("source.module", source_module),
    ("source.file", source_file),
];
