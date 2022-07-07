// These are a series of tests that require a locally running modality instance.
use tokio::runtime;
use tracing::info;
use tracing_modality::ModalityLayer;

procspawn::enable_test_support!();

fn rt() -> runtime::Runtime {
    runtime::Builder::new_current_thread()
        .build()
        .expect("build tokio runtime")
}

#[test]
fn init() {
    procspawn::spawn((), |_| {
        let opts = tracing_modality::Options::new();

        tracing_modality::TracingModality::init_with_options(opts).expect("init");
    })
    .join()
    .expect("test process");
}

#[test]
fn init_l1() {
    procspawn::spawn((), |_| {
        tracing_modality::TracingModality::init().expect("init");
    })
    .join()
    .expect("test process");
}

#[test]
fn init_l2() {
    procspawn::spawn((), |_| {
        tracing_modality::TracingModality::init().expect("init");
    })
    .join()
    .expect("test process");
}

#[tokio::test]
async fn init_tokio() {
    tracing_modality::TracingModality::async_init()
        .await
        .expect("init");
}

#[test]
fn init_std_use_tokio() {
    let sub = ModalityLayer::init().expect("init").into_subscriber();
    tracing::subscriber::with_default(sub, || {
        rt().block_on(async { info!(number = 3, "hello") });
    })
}

#[tokio::test]
async fn init_tokio_use_tokio() {
    let sub = ModalityLayer::async_init()
        .await
        .expect("init")
        .into_subscriber();
    tracing::subscriber::with_default(sub, || info!(number = 3, "hello"))
}
