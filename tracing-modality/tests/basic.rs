mod common;

use tracing_modality;

procspawn::enable_test_support!();

#[test]
fn test_basic() {
    let modality_addr = common::modality_null_mock();

    let handle = procspawn::spawn(modality_addr, |modality_addr| {
        let opts = tracing_modality::Options::new()
            .with_server_address(modality_addr)
            .with_auth("1234567890abcdef");

        tracing_modality::TracingModality::init_with_options(opts).expect("init");
    });

    let () = handle.join().unwrap();
}
