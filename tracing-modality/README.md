# tracing-modality

`tracing-modality` provides a [tracing.rs] `Subscriber` for tracing systems
written in [Rust] to [Auxon Modality](https://auxon.io).

[tracing.rs]: https://tracing.rs
[Rust]: https://www.rust-lang.org/

The quickest (and as of this version, only) way to get started is to let
[`TracingModality`] register itself as the global default tracer, done most
simply using [`TracingModality::init()`]

```rust
use tracing::debug;
use tracing_modality::TracingModality;
use tracing::info;

fn main() {
    TracingModality::init().expect("init");

    info!("my application has started");
}
```

Some basic configuration options are also available to be set at init with
[`TracingModality::init_with_options`].

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

## Metadata and Fields

Each event or span creation has the fields and message directly provided as
well as some metadata about where it was called from. This data is mapped to
Modality trace data as follows:

### Metadata

* `message` or `name` -> `event.name`
* `level` -> `event.severity`
* `module_path` -> `event.source.module`
* `file` -> `event.source.file`
* `line` -> `event.source.line`
* the kind of event -> `event.internal.rs.kind` ["span:defined", "span:enter",
  "span:exit" ]
* `id` -> `event.internal.rs.span_id`

### Fields

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
