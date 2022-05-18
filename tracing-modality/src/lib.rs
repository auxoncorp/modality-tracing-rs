use std::thread;
use tracing_core::Dispatch;
use tracing_serde_modality_ingest::TracingModalityLense;
use tracing_serde_subscriber::TSCollector;

pub struct TracingModality {
    join_handle: thread::JoinHandle<()>,
}

impl TracingModality {
    pub fn init() -> Self {
        let disp = Dispatch::new(TSCollector::new());
        tracing::dispatch::set_global_default(disp).unwrap();

        let coll = TSCollector::take_receiver().unwrap();

        let join_handle = thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("create tokio runtime");
            let handle = rt.handle();
            handle.block_on(async {
                let mut lense = TracingModalityLense::connect().await.expect("connect");
                while let Ok(msg) = coll.recv() {
                    lense.handle_packet(msg).await.expect("handle packet");
                }
                println!("thread exiting");
            });
        });

        Self { join_handle }
    }

    pub fn exit(self) {
        self.join_handle.join().expect("join");
    }
}
