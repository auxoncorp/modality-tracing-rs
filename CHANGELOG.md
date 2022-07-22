Unreleased
==========

* Changed from connection per thread to a single connection and a message queue
  serviced by a dedicated ingest thread.
* Tracing from tokio is now supported.
* Replaced use of `tracing-serde*` crates with internal types.
* Initializing from tokio is now supported.

Version 0.1.0
=============

Initial Release
