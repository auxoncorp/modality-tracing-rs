# tracing-modality

`tracing-modality` provides a [tracing.rs] `Subscriber` for tracing systems
written in [Rust] to [Auxon Modality](https://auxon.io).

[tracing.rs]: https://tracing.rs
[Rust]: https://www.rust-lang.org/

The quickest way to get started is to let [`TracingModality`] register itself
as the global default tracer, done most simply using
[`TracingModality::init()`]:

```rust,no_run
use tracing::debug;
use tracing_modality::blocking::TracingModality;
use tracing::info;

fn main() {
    // first thing in main
    let mut modality = TracingModality::init().expect("init");

    info!("my application has started");

    // last thing in main
    modality.finish()
}
```

Some basic configuration options are also available to be set at init with
[`TracingModality::init_with_options`].

As this example shows, you must [`TracingModality::finish`] at the end of your
main thread to ensure the ingest thread handing all trace events has a chance to
finish flushing all queued events before your program exits.

## Usage

After the initial setup, shown above, this library is not used directly, but is
used via macros defined in [tracing.rs]. These macros function much like the
`log` crate's logging macros `error`, `warn`, `info`, `debug` and `trace`,
except they also allow you to pass in structured values, in the form of fields,
that are recorded in the trace data as standalone values rather than being
stringified into the log message.

Additionally, `tracing` provides a similar set of macros for emitting spans:
`error_span`, `warn_span`, `info_span`, `debug_span`, and `trace_span`. Spans
are used to label regions of execution.

There are also many more ways to use `tracing`, such as the `#[instrument]`
attribute and the `Value` trait. See [`tracing`'s documentation][tracing docs]
for the full API.

[tracing docs]: https://docs.rs/tracing/latest/tracing/

## Mappings to Modality Concepts

While `tracing` and modality both deal with tracing data, there's some
difference in the exact data model each uses. This section describes how
`tracing`'s concepts are mapped into Modality's concepts.

### Modality Timelines

In Modality, a timeline is a linear sequence of events. This library represents
each OS thread as a seperate timeline.

To record interactions between threads the timeline ID of the remote thread
must be known. Each thread can access its own timeline ID with the
[`timeline_id()`] function and should send that ID along with the interaction
for the remote thread to record the interaction on its own timeline.

### `tracing` Metadata

`tracing` implicitly generates some metadata for every event and span and
`tracing-modality` will add some more based on its view of each event or span.

This is how each piece of metadata is mapped into modality:

* `message` or `name` -> `event.name`
* `level` -> `event.severity`
* `module_path` -> `event.source.module`
* `file` -> `event.source.file`
* `line` -> `event.source.line`
* the kind of event -> `event.internal.rs.kind` ["span:defined", "span:enter",
  "span:exit" ]
* `id` -> `event.internal.rs.span_id` (spans only)

### `tracing` Fields

Fields are the structured data you define when using an event or span macro.
They consist of a key that takes the form of a dot sperated string and a value
of one of `tracing`'s supported types.

All fields are mapped directly as is to `event.*`, excect fields prefixed with
`modality.` which are mapped to the datasource specific namespace
`event.internal.rs.*`. Fields manually set will overwrite any any default
values set by metadata, if present.

# License

Copyright 2022 [Auxon Corporation](https://auxon.io)

Licensed under the Apache License, Version 2.0 (the "License"); you may not use
this file except in compliance with the License.  You may obtain a copy of the
License at

<http://www.apache.org/licenses/LICENSE-2.0>

Unless required by applicable law or agreed to in writing, software distributed
under the License is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR
CONDITIONS OF ANY KIND, either express or implied.  See the License for the
specific language governing permissions and limitations under the License.
