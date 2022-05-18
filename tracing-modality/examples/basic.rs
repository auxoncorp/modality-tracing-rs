use tracing::{debug, error, event, info, span, trace, warn, Level};
use tracing_modality::TracingModality;

fn main() {
    let t = TracingModality::init();

    let span = span!(Level::TRACE, "outer_span");
    let _span = span.enter();
    do_thing::doit();

    t.exit();
}

pub mod do_thing {
    use super::*;
    pub fn doit() {
        let span = span!(Level::TRACE, "my span");
        span.in_scope(|| {
            event!(Level::INFO, "something has happened!");
        });

        event!(Level::TRACE, foo = 1, "a trace thing");

        trace!(foo = 1, "a trace thing");
        debug!(foo = 2, "a debug thing");
        info!(foo = 3, "a info thing");
        warn!(foo = 4, "a warn thing");
        error!(foo = 5, "a error thing");
    }
}
