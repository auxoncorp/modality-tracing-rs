use tracing_core::Dispatch;
use tracing_serde_modality_ingest::TracingModalityLense;
use tracing_serde_subscriber::{Packet, TSCollector};

pub struct TracingModality {}

impl TracingModality {
    pub fn init() -> Self {
        let rt = tokio::runtime::Runtime::new().expect("create tokio runtime");
        let mut lense = {
            let handle = rt.handle();
            handle.block_on(async { TracingModalityLense::connect().await.expect("connect") })
        };

        let handler: Box<dyn FnMut(Packet<'_>) + Send> = Box::new(move |pkt| {
            // I don't understand this, it only lets me capture a reference to the runtime,
            // I would expect the runtime to get dropped and require me to capture the value of the
            // runtime, but it seems to work? Leaving for now, it'll disappear when I get a
            // non-tokio version of the SDK made anyway.
            let rt = &rt;

            let handle = rt.handle();
            handle.block_on(async {
                lense.handle_packet(pkt).await.expect("handle packet");
            })
        });

        let disp = Dispatch::new(TSCollector::new(handler));
        tracing::dispatch::set_global_default(disp).unwrap();

        Self {}
    }
}
