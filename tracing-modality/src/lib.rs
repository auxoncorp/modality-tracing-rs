use tracing_core::Dispatch;
use tracing_serde_modality_ingest::TracingModalityLense;
use tracing_serde_subscriber::{Packet, TSCollector};

pub use tracing_serde_modality_ingest::Options;

pub struct TracingModality {}

impl TracingModality {
    pub fn init() -> Self {
        let opt = Options::default();
        Self::init_with_options(opt)
    }

    pub fn init_with_options(opt: Options) -> Self {
        let rt = tokio::runtime::Runtime::new().expect("create tokio runtime");
        let mut lense = {
            let handle = rt.handle();
            handle.block_on(async {
                TracingModalityLense::connect_with_options(opt)
                    .await
                    .expect("connect")
            })
        };

        let handler: Box<dyn FnMut(Packet<'_>) + Send> = Box::new(move |pkt| {
            rt.handle().block_on(async {
                lense.handle_packet(pkt).await.expect("handle packet");
            })
        });

        let disp = Dispatch::new(TSCollector::new(handler));
        tracing::dispatch::set_global_default(disp).unwrap();

        Self {}
    }
}
