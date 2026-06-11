//! eb_monitor_cli - SUB stub for the Event Builder PUB stream (SPEC § 9.3).
//!
//! Subscribes to the [`EbMessage`] stream emitted by `online_event_builder`
//! (default `tcp://localhost:5610`), decodes each batch and prints rolling
//! stats (event rate, batch count, multiplicity histogram). Exits cleanly
//! on `EbMessage::EndOfStream`.
//!
//! This is a placeholder for the full "EB Monitor" web UI described in the
//! Event Builder SPEC § 11.4 item M — same wire format, no histograms/REST.
//!
//! Usage:
//!
//! ```text
//! cargo run --release --features dev-tools --bin eb_monitor_cli -- \
//!     --subscribe tcp://localhost:5610
//! ```

use std::time::{Duration, Instant};

use clap::Parser;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use delila_rs::event_builder::EbMessage;

#[derive(Parser, Debug)]
#[command(
    name = "eb_monitor_cli",
    about = "Subscribe to EB BuiltEventBatch PUB stream and print rolling stats"
)]
struct Args {
    /// ZMQ endpoint to subscribe to (Online EB PUB)
    #[arg(
        short = 's',
        long = "subscribe",
        default_value = "tcp://localhost:5610"
    )]
    subscribe: String,

    /// Stats print interval in seconds
    #[arg(long = "interval-secs", default_value_t = 2)]
    interval_secs: u64,

    /// SUB receive timeout in milliseconds (controls heartbeat / stats cadence
    /// when the stream is sparse). Set to 0 to block indefinitely.
    #[arg(long = "rcvtimeo-ms", default_value_t = 500)]
    rcvtimeo_ms: i32,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("delila_rs=info".parse()?))
        .init();

    let args = Args::parse();

    let ctx = zmq::Context::new();
    let socket = ctx.socket(zmq::SUB)?;
    socket.set_rcvhwm(0)?; // never drop — buffer in memory (CLAUDE.md)
    socket.connect(&args.subscribe)?;
    socket.set_subscribe(b"")?;
    if args.rcvtimeo_ms > 0 {
        socket.set_rcvtimeo(args.rcvtimeo_ms)?;
    }

    println!("========================================");
    println!("  EB Monitor CLI (SPEC § 9.3 stub)");
    println!("========================================");
    println!("  Subscribe:  {}", args.subscribe);
    println!("  Interval:   {} s", args.interval_secs);
    println!("========================================");

    let start = Instant::now();
    let mut last_print = Instant::now();

    let mut total_events: u64 = 0;
    let mut total_batches: u64 = 0;
    let mut total_heartbeats: u64 = 0;
    // Multiplicity histogram buckets: 1, 2, 3, 4, 8, 16, 32, 64+
    // Stored as [count_eq1, count_eq2, count_eq3, count_eq4, count_5_8,
    //            count_9_16, count_17_32, count_ge33]
    let mut mult_hist = [0u64; 8];

    let mut last_events: u64 = 0;
    let mut last_rate_t = Instant::now();
    let interval = Duration::from_secs(args.interval_secs.max(1));

    loop {
        match socket.recv_bytes(0) {
            Ok(bytes) => match EbMessage::from_msgpack(&bytes) {
                Ok(EbMessage::Events(batch)) => {
                    total_events += batch.events.len() as u64;
                    total_batches += 1;
                    for ev in &batch.events {
                        let m = ev.hits.len();
                        let idx = match m {
                            1 => 0,
                            2 => 1,
                            3 => 2,
                            4 => 3,
                            5..=8 => 4,
                            9..=16 => 5,
                            17..=32 => 6,
                            _ => 7,
                        };
                        mult_hist[idx] += 1;
                    }
                }
                Ok(EbMessage::Heartbeat {
                    run_number,
                    counter,
                }) => {
                    total_heartbeats += 1;
                    info!(run_number, counter, "heartbeat");
                }
                Ok(EbMessage::EndOfStream { run_number }) => {
                    println!();
                    println!("========================================");
                    println!("  EOS received for run {run_number}");
                    println!("========================================");
                    print_summary(
                        start.elapsed(),
                        total_events,
                        total_batches,
                        total_heartbeats,
                        &mult_hist,
                    );
                    return Ok(());
                }
                Err(e) => {
                    warn!(error = %e, bytes = bytes.len(), "Failed to decode EbMessage");
                }
            },
            Err(zmq::Error::EAGAIN) => {
                // Timeout — fall through to stats print.
            }
            Err(e) => {
                warn!(error = %e, "ZMQ recv error");
                return Err(anyhow::anyhow!("ZMQ recv failed: {e}"));
            }
        }

        if last_print.elapsed() >= interval {
            let now = Instant::now();
            let dt = now.duration_since(last_rate_t).as_secs_f64().max(1e-3);
            let de = total_events - last_events;
            let rate = de as f64 / dt;
            let elapsed_s = start.elapsed().as_secs_f64();
            println!(
                "[eb_monitor] t={elapsed_s:6.1}s  rate={rate:>10.0} ev/s  \
                 batches={total_batches}  events={total_events}  \
                 hb={total_heartbeats}  hist={hist}",
                hist = format_hist(&mult_hist),
            );
            last_events = total_events;
            last_rate_t = now;
            last_print = now;
        }
    }
}

fn format_hist(hist: &[u64; 8]) -> String {
    let labels = ["m=1", "m=2", "m=3", "m=4", "5-8", "9-16", "17-32", ">=33"];
    let total: u64 = hist.iter().sum();
    if total == 0 {
        return "<no events>".to_string();
    }
    let mut parts = Vec::with_capacity(hist.len());
    for (i, &c) in hist.iter().enumerate() {
        if c > 0 {
            let pct = 100.0 * c as f64 / total as f64;
            parts.push(format!("{}:{}({:.0}%)", labels[i], c, pct));
        }
    }
    parts.join(" ")
}

fn print_summary(
    elapsed: Duration,
    total_events: u64,
    total_batches: u64,
    total_heartbeats: u64,
    hist: &[u64; 8],
) {
    println!("  Elapsed:      {:.1} s", elapsed.as_secs_f64());
    println!("  Total events: {total_events}");
    println!("  Total batches:{total_batches}");
    println!("  Heartbeats:   {total_heartbeats}");
    println!("  Histogram:    {}", format_hist(hist));
    if elapsed.as_secs_f64() > 0.0 {
        println!(
            "  Avg rate:     {:.0} ev/s",
            total_events as f64 / elapsed.as_secs_f64()
        );
    }
}
