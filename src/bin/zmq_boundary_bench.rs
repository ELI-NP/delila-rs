//! ZMQ pipeline boundary cost benchmark.
//!
//! R-X3 (Phase 1 refactor sprint 2026-Q2). Records baseline numbers before
//! the structural refactor lands and again after Phase 3 so the diff makes
//! the cost of the cross-process ZMQ boundaries visible — answering the
//! long-standing question "ZMQ 境界 cost が実際どれくらいか?".
//!
//! # What it measures
//!
//! A single in-process PUB → SUB pair on `inproc://` simulates one boundary.
//! For each `--rate` × `--duration` × event-size combination we record:
//!
//! - **Throughput** end-to-end events/s and MB/s (wire bytes)
//! - **Latency** p50 / p95 / p99 of (batch produced → batch received)
//! - **Per-boundary** mean encode (rmp_serde) / send (`tmq::send`) /
//!   recv (`tmq::next`) / decode (rmp_serde) cost per batch in µs
//! - **Per-event bytes** average wire bytes / event (encode overhead)
//! - **Drops** ZMQ would normally drop on HWM but this bench enforces HWM=0
//!   via [`pub_no_hwm`]/[`sub_no_hwm`] so the drop counter is a regression
//!   guard
//!
//! Scaffold only — Phase 1 commits the binary and runs a *single boundary*
//! baseline. Phase 3 (post-refactor) re-runs the same command on gant and
//! we diff the numbers against the JSON written here. The bench can later
//! be extended to spawn the full emulator → reader → merger → recorder
//! pipeline, but that requires the production binaries; the in-process
//! single-boundary version is what gates the refactor.
//!
//! # Usage (on gant@172.18.6.114)
//!
//! ```sh
//! ssh gant@172.18.6.114 'cd /media/raid1/delila-rs && \
//!   source ~/.cargo/env && \
//!   cargo run --release --features dev-tools --bin zmq_boundary_bench -- \
//!     --rate 100000 --duration 60 --batch-size 100 --waveform-samples 0 \
//!     --output baseline_100k.json'
//! ```
//!
//! # Output
//!
//! Writes a JSON document to `--output` (default stdout). The schema is
//! stable across the sprint — `docs/plans/zmq_boundary_cost_2026-Q2.md`
//! consumes the JSON for the baseline / post-refactor / diff tables.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use clap::Parser;
use delila_rs::common::{pub_no_hwm, sub_no_hwm, EventData, EventDataBatch, Message};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tmq::{publish, subscribe, Context};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(
    name = "zmq_boundary_bench",
    about = "Measure ZMQ PUB→SUB boundary cost (R-X3, Phase 1 refactor sprint 2026-Q2)"
)]
struct Args {
    /// Target events per second (10k / 100k / 1M).
    #[arg(long, default_value_t = 100_000)]
    rate: u64,

    /// Bench duration in seconds.
    #[arg(long, default_value_t = 30)]
    duration: u64,

    /// Events per batch.
    #[arg(long, default_value_t = 100)]
    batch_size: usize,

    /// Number of waveform samples per event (0 = no waveform).
    #[arg(long, default_value_t = 0)]
    waveform_samples: usize,

    /// Output path for the JSON result document. Default: stdout.
    #[arg(long)]
    output: Option<PathBuf>,

    /// Tag the run with a free-form label (commit hash, refactor phase, etc.).
    #[arg(long, default_value = "untagged")]
    label: String,
}

/// Result schema. Keep field names stable; the diff doc parses them.
#[derive(Debug, Serialize, Deserialize)]
struct BenchReport {
    label: String,
    timestamp_unix: u64,
    git_commit: String,
    rust_version: String,
    args: ReportArgs,
    pipeline: PipelineMetrics,
    per_boundary_us: PerBoundaryMicros,
    bytes: ByteMetrics,
}

#[derive(Debug, Serialize, Deserialize)]
struct ReportArgs {
    rate: u64,
    duration_s: u64,
    batch_size: usize,
    waveform_samples: usize,
}

#[derive(Debug, Serialize, Deserialize)]
struct PipelineMetrics {
    events_sent: u64,
    events_received: u64,
    batches_sent: u64,
    batches_received: u64,
    drops: u64,
    throughput_eps: f64,
    throughput_mbps: f64,
    latency_p50_us: u64,
    latency_p95_us: u64,
    latency_p99_us: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct PerBoundaryMicros {
    encode_mean: f64,
    send_mean: f64,
    recv_mean: f64,
    decode_mean: f64,
}

#[derive(Debug, Serialize, Deserialize)]
struct ByteMetrics {
    total_wire_bytes: u64,
    bytes_per_event: f64,
}

fn make_event(seq: u64, waveform_samples: usize) -> EventData {
    let waveform = if waveform_samples > 0 {
        Some(delila_rs::common::Waveform {
            analog_probe1: vec![(seq & 0x3FFF) as i16; waveform_samples],
            ..Default::default()
        })
    } else {
        None
    };
    EventData {
        module: 0,
        channel: (seq % 32) as u8,
        energy: (seq & 0xFFFF) as u16,
        energy_short: ((seq >> 16) & 0xFFFF) as u16,
        timestamp_ns: seq as f64 * 8.0,
        flags: 0,
        user_info: [0; 4],
        waveform,
    }
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env().add_directive("zmq_boundary_bench=info".parse()?),
        )
        .init();

    let args = Args::parse();
    info!(?args, "starting zmq_boundary_bench");

    let context = Context::new();
    // inproc:// keeps the bench self-contained — no socket-permission issues
    // on shared hosts, no port collisions, and rules out kernel/TCP cost.
    let address = "inproc://zmq_boundary_bench";

    // Receiver task — bind first so the publisher's connect succeeds.
    let recv_ctx = context.clone();
    let recv_handle = tokio::spawn(async move {
        let socket = subscribe(&recv_ctx).bind(address)?.subscribe(b"")?;
        sub_no_hwm(&socket)?;
        let mut socket = socket;

        let mut events_received: u64 = 0;
        let mut batches_received: u64 = 0;
        let mut total_bytes: u64 = 0;
        let mut latencies_us = Vec::with_capacity(100_000);
        let mut recv_us_sum: u128 = 0;
        let mut decode_us_sum: u128 = 0;

        // Stop signal: receive an EOS message from the publisher when done.
        loop {
            let recv_start = Instant::now();
            let msg = match socket.next().await {
                Some(Ok(m)) => m,
                Some(Err(e)) => {
                    warn!(error = %e, "recv error");
                    break;
                }
                None => break,
            };
            let recv_us = recv_start.elapsed().as_micros();

            let bytes: Vec<u8> = msg
                .into_iter()
                .next()
                .map(|f| f.to_vec())
                .unwrap_or_default();
            total_bytes += bytes.len() as u64;

            let decode_start = Instant::now();
            let parsed = Message::from_msgpack(&bytes)?;
            let decode_us = decode_start.elapsed().as_micros();

            recv_us_sum += recv_us;
            decode_us_sum += decode_us;

            match parsed {
                Message::Data(batch) => {
                    let now_ns = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos() as u64;
                    if now_ns > batch.timestamp {
                        let lat_us = (now_ns - batch.timestamp) / 1000;
                        latencies_us.push(lat_us);
                    }
                    events_received += batch.events.len() as u64;
                    batches_received += 1;
                }
                Message::EndOfStream { .. } => break,
                Message::Heartbeat(_) => {}
            }
        }

        latencies_us.sort_unstable();
        Ok::<_, anyhow::Error>((
            events_received,
            batches_received,
            total_bytes,
            latencies_us,
            recv_us_sum,
            decode_us_sum,
        ))
    });

    // Tiny delay to let the SUB bind. inproc requires the bind side to win
    // the race; without this the connect fails with EAGAIN.
    tokio::time::sleep(Duration::from_millis(50)).await;

    let send_ctx = context.clone();
    let pub_socket = publish(&send_ctx).connect(address)?;
    pub_no_hwm(&pub_socket)?;
    let mut pub_socket = pub_socket;

    let total_events = args.rate * args.duration;
    let total_batches = total_events.div_ceil(args.batch_size as u64);
    let nanos_per_batch = if args.rate == 0 {
        0
    } else {
        1_000_000_000u64 * args.batch_size as u64 / args.rate
    };

    info!(
        total_events,
        total_batches, nanos_per_batch, "publishing target"
    );

    let mut events_sent: u64 = 0;
    let mut encode_us_sum: u128 = 0;
    let mut send_us_sum: u128 = 0;
    let bench_start = Instant::now();
    let mut next_send = bench_start;

    for batch_idx in 0..total_batches {
        // Crude pacing — for 1 M ev/s with batch=100 the pacing window is
        // 100 µs, which tokio sleep can hit ±a few µs on Linux.
        let now = Instant::now();
        if now < next_send {
            tokio::time::sleep(next_send - now).await;
        }
        next_send += Duration::from_nanos(nanos_per_batch);

        let mut batch = EventDataBatch::with_capacity(0, batch_idx, args.batch_size);
        for j in 0..args.batch_size as u64 {
            batch.push(make_event(events_sent + j, args.waveform_samples));
        }
        // Stamp send time so the receiver can compute end-to-end latency.
        batch.timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;

        let encode_start = Instant::now();
        let bytes = Message::data(batch).to_msgpack()?;
        encode_us_sum += encode_start.elapsed().as_micros();

        let send_start = Instant::now();
        pub_socket
            .send(tmq::Multipart::from(vec![tmq::Message::from(
                bytes.as_slice(),
            )]))
            .await?;
        send_us_sum += send_start.elapsed().as_micros();

        events_sent += args.batch_size as u64;
    }

    // Publish EOS so the receiver task exits cleanly.
    let eos = Message::eos(0, 0).to_msgpack()?;
    pub_socket
        .send(tmq::Multipart::from(vec![tmq::Message::from(
            eos.as_slice(),
        )]))
        .await?;

    let elapsed = bench_start.elapsed();
    let (events_received, batches_received, total_bytes, latencies, recv_us_sum, decode_us_sum) =
        recv_handle.await??;
    let drops = events_sent.saturating_sub(events_received);

    let throughput_eps = events_received as f64 / elapsed.as_secs_f64();
    let throughput_mbps = total_bytes as f64 / elapsed.as_secs_f64() / 1_000_000.0;
    let bytes_per_event = if events_received == 0 {
        0.0
    } else {
        total_bytes as f64 / events_received as f64
    };

    let report = BenchReport {
        label: args.label.clone(),
        timestamp_unix: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        git_commit: option_env!("GIT_COMMIT").unwrap_or("unknown").to_string(),
        rust_version: rustc_version().unwrap_or_else(|| "unknown".to_string()),
        args: ReportArgs {
            rate: args.rate,
            duration_s: args.duration,
            batch_size: args.batch_size,
            waveform_samples: args.waveform_samples,
        },
        pipeline: PipelineMetrics {
            events_sent,
            events_received,
            batches_sent: total_batches,
            batches_received,
            drops,
            throughput_eps,
            throughput_mbps,
            latency_p50_us: percentile(&latencies, 0.50),
            latency_p95_us: percentile(&latencies, 0.95),
            latency_p99_us: percentile(&latencies, 0.99),
        },
        per_boundary_us: PerBoundaryMicros {
            encode_mean: encode_us_sum as f64 / total_batches.max(1) as f64,
            send_mean: send_us_sum as f64 / total_batches.max(1) as f64,
            recv_mean: recv_us_sum as f64 / batches_received.max(1) as f64,
            decode_mean: decode_us_sum as f64 / batches_received.max(1) as f64,
        },
        bytes: ByteMetrics {
            total_wire_bytes: total_bytes,
            bytes_per_event,
        },
    };

    if drops > 0 {
        warn!(
            drops,
            "events were dropped — investigate before trusting numbers"
        );
    }

    let json = serde_json::to_string_pretty(&report)?;
    if let Some(path) = args.output {
        std::fs::write(&path, &json)?;
        info!(path = %path.display(), "wrote bench report");
    } else {
        println!("{json}");
    }

    // Drop sockets first so the context teardown doesn't hang.
    drop(pub_socket);
    drop(context);
    let _ = Arc::new(()); // suppress unused-import warning if Arc is unused
    Ok(())
}

fn rustc_version() -> Option<String> {
    let out = std::process::Command::new(std::env::var("RUSTC").ok()?)
        .arg("--version")
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
