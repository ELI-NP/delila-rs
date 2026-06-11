//! Event Builder .delila Replayer
//!
//! `.delila` 生データから `EventDataBatch` をそのままの形で読み出し、
//! ZMQ PUB 経由で再配信する Merger インパーソネーター。
//! Online Event Builder の本番投入前の staging テスト用。
//!
//! `eb_test_sender` (ROOT → Hit → 再バッチング) と違い、
//! `EventDataBatch` を MsgPack 化してそのまま流すため、
//! `source_id` / `sequence_number` / batch 境界 / waveform / user_info が
//! 本物の Merger 出力と bit-level で一致する。
//!
//! Usage:
//!   cargo run --release --features dev-tools --bin eb_delila_replay -- \
//!       --input ./data/run0042_*.delila --publish tcp://*:5557
//!
//! Online EB 側:
//!   cargo run --release --features root --bin online_event_builder -- -f config.toml
//!
//! Online EB の `[network.event_builder].subscribe` を `tcp://localhost:5557`
//! に合わせれば、 オフラインで動いた eb_config.json / chSettings.json /
//! timeSettings.json をそのまま流用できる。

use std::collections::BTreeSet;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;
use futures::SinkExt;
use tmq::{publish, Context};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use delila_rs::common::{pub_no_hwm, Message};
use delila_rs::recorder::DataFileReader;

#[derive(Parser, Debug)]
#[command(
    name = "eb_delila_replay",
    about = "Replay .delila EventDataBatches via ZMQ PUB (Merger impersonation) for Online EB staging"
)]
struct Args {
    /// Input .delila file(s) — sent in the order given
    #[arg(short, long, required = true, num_args = 1..)]
    input: Vec<PathBuf>,

    /// ZMQ PUB bind address (must match `[network.event_builder].subscribe` in the EB TOML)
    #[arg(short, long, default_value = "tcp://*:5557")]
    publish: String,

    /// Delay between PUB sends in milliseconds (0 = no delay, burst as fast as possible)
    #[arg(long, default_value_t = 0)]
    delay_ms: u64,

    /// Warmup delay after binding the PUB socket, to let the SUB connect before sending
    #[arg(long, default_value_t = 1000)]
    warmup_ms: u64,

    /// Send EndOfStream after all input files (one per observed source_id)
    #[arg(long, default_value_t = true)]
    send_eos: bool,

    /// run_number embedded in the EOS message (the Online EB filters stale EOS by this)
    #[arg(long, default_value_t = 0)]
    run_number: u32,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("delila_rs=info".parse()?))
        .init();

    let args = Args::parse();

    let context = Context::new();
    let mut socket = publish(&context).bind(&args.publish)?;
    pub_no_hwm(&socket)?;
    info!(address = %args.publish, "ZMQ PUB bound (SNDHWM=0)");

    if args.warmup_ms > 0 {
        info!(
            ms = args.warmup_ms,
            "Warmup: waiting for subscribers to connect"
        );
        tokio::time::sleep(Duration::from_millis(args.warmup_ms)).await;
    }

    let mut total_batches: u64 = 0;
    let mut total_events: u64 = 0;
    let mut source_ids: BTreeSet<u32> = BTreeSet::new();

    for path in &args.input {
        let file = File::open(path)
            .map_err(|e| anyhow::anyhow!("Failed to open {}: {e}", path.display()))?;
        let mut reader = DataFileReader::new(BufReader::new(file))
            .map_err(|e| anyhow::anyhow!("Failed to parse {}: {e}", path.display()))?;

        let mut file_batches: u64 = 0;
        let mut file_events: u64 = 0;

        for block in reader.data_blocks() {
            match block {
                Ok(batch) => {
                    source_ids.insert(batch.source_id);
                    file_events += batch.events.len() as u64;
                    file_batches += 1;

                    let msg = Message::data(batch);
                    let bytes = msg.to_msgpack()?;
                    let zmq_msg: tmq::Multipart = vec![tmq::Message::from(bytes.as_slice())].into();
                    socket.send(zmq_msg).await?;

                    if args.delay_ms > 0 {
                        tokio::time::sleep(Duration::from_millis(args.delay_ms)).await;
                    }

                    if file_batches.is_multiple_of(1000) {
                        info!(
                            file = %path.display(),
                            batches = file_batches,
                            events = file_events,
                            "Sending progress"
                        );
                    }
                }
                Err(e) => {
                    warn!(error = %e, file = %path.display(), "Skipping corrupted block");
                }
            }
        }

        info!(
            file = %path.display(),
            batches = file_batches,
            events = file_events,
            "Done sending file"
        );
        total_batches += file_batches;
        total_events += file_events;
    }

    if args.send_eos {
        if source_ids.is_empty() {
            // No batches read — still send one EOS so the receiver can exit cleanly.
            let eos = Message::eos(0, args.run_number);
            let bytes = eos.to_msgpack()?;
            let zmq_msg: tmq::Multipart = vec![tmq::Message::from(bytes.as_slice())].into();
            socket.send(zmq_msg).await?;
            info!(
                run_number = args.run_number,
                "Sent EOS (source_id=0, no data read)"
            );
        } else {
            for source_id in &source_ids {
                let eos = Message::eos(*source_id, args.run_number);
                let bytes = eos.to_msgpack()?;
                let zmq_msg: tmq::Multipart = vec![tmq::Message::from(bytes.as_slice())].into();
                socket.send(zmq_msg).await?;
            }
            info!(
                sources = source_ids.len(),
                run_number = args.run_number,
                "Sent EOS for each observed source_id"
            );
        }
    }

    println!();
    println!("========================================");
    println!("  eb_delila_replay Complete");
    println!("========================================");
    println!("  Files:         {}", args.input.len());
    println!("  Total batches: {}", total_batches);
    println!("  Total events:  {}", total_events);
    println!("  source_ids:    {:?}", source_ids);
    println!("  Endpoint:      {}", args.publish);
    println!("========================================");

    // Let the ZMQ background thread flush before the context drops.
    tokio::time::sleep(Duration::from_millis(500)).await;
    Ok(())
}
