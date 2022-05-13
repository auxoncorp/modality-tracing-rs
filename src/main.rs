use std::io::Result;
use tracing_modality_lense::TracingModalityLense;

#[tokio::main]
async fn main() -> Result<()> {
    let input = include_str!("../pile.jsonl");
    let mut lense = TracingModalityLense::connect().await.expect("connect");

    for line in input.lines() {
        let pkt: tracing_serde_wire::Packet = serde_json::from_str(line)?;
        lense.handle_packet(pkt).await.expect("handle");
    }

    Ok(())
}
