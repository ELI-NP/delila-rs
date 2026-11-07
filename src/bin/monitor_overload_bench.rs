//! Monitor overload bench (TODO 84 §8).
//!
//! Drives a complete in-process `Monitor` (REP commands, HTTP API, SUB data)
//! from a paced PUB blaster and watches, from the outside, what the user saw
//! on pf-qaq: does the processed counter keep moving, do GetStatus and
//! `GET /api/status` keep answering, and does the process memory stay flat
//! when the input is faster than one thread can deserialize?
//!
//! ```sh
//! cargo run --release --features dev-tools --bin monitor_overload_bench -- \
//!     --batches-per-sec 150 --events-per-batch 33000 --duration 30
//! ```
//!
//! `--batches-per-sec 0` sends as fast as the PUB accepts. The batch is
//! serialized once; only the sequence number is patched per send, so the
//! blaster itself costs nothing measurable.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use clap::Parser;
use delila_rs::common::{Command, CommandResponse, EventData, EventDataBatch, Message, RunConfig};
use delila_rs::monitor::{Monitor, MonitorConfig};
use tokio::sync::broadcast;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(
    name = "monitor_overload_bench",
    about = "Feed a local Monitor faster than it can decode and watch for stalls"
)]
struct Args {
    /// Batches per second (0 = unpaced, as fast as possible).
    #[arg(long, default_value_t = 150)]
    batches_per_sec: u64,

    /// Events per batch (Merger at 2.8-5 MHz: ~33k).
    #[arg(long, default_value_t = 33_000)]
    events_per_batch: usize,

    /// Observation window in seconds.
    #[arg(long, default_value_t = 30)]
    duration: u64,

    /// Sampling period in milliseconds.
    #[arg(long, default_value_t = 200)]
    sample_ms: u64,

    /// Monitor HTTP port.
    #[arg(long, default_value_t = 18081)]
    http_port: u16,

    /// Monitor command (REP) port.
    #[arg(long, default_value_t = 15590)]
    command_port: u16,
}

fn batch_bytes(n: usize) -> Vec<u8> {
    let mut batch = EventDataBatch::with_capacity(0, 1, n);
    for i in 0..n {
        let mut ev = EventData::zeroed();
        ev.module = (i % 2) as u8;
        ev.channel = (i % 16) as u8;
        ev.energy = (i % 60_000) as u16;
        ev.energy_short = ev.energy / 2;
        ev.timestamp_ns = i as f64 * 100.0;
        batch.push(ev);
    }
    Message::Data(batch).to_msgpack().expect("msgpack")
}

fn send_command(ctx: &zmq::Context, addr: &str, cmd: &Command) -> anyhow::Result<CommandResponse> {
    let req = ctx.socket(zmq::REQ)?;
    req.set_rcvtimeo(10_000)?;
    req.set_linger(0)?;
    req.connect(addr)?;
    req.send(&cmd.to_json()?[..], 0)?;
    let reply = req.recv_bytes(0)?;
    Ok(CommandResponse::from_json(&reply)?)
}

/// Pull "Received: a, Processed: b, Dropped: c" out of the status message.
fn parse_counter(message: &str, label: &str) -> Option<u64> {
    let start = message.find(label)? + label.len();
    let digits: String = message[start..]
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

fn rss_mb() -> Option<f64> {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()?;
    let kb: f64 = String::from_utf8_lossy(&out.stdout).trim().parse().ok()?;
    Some(kb / 1024.0)
}

#[derive(Debug, serde::Deserialize)]
struct StatusJson {
    total_events: u64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("delila_rs=warn".parse()?))
        .init();
    let args = Args::parse();

    let zctx = zmq::Context::new();
    let publisher = zctx.socket(zmq::PUB)?;
    publisher.set_sndhwm(0)?;
    publisher.set_linger(0)?;
    publisher.bind("tcp://127.0.0.1:*")?;
    let sub_addr = publisher
        .get_last_endpoint()?
        .map_err(|_| anyhow::anyhow!("non-utf8 endpoint"))?;
    let cmd_addr = format!("tcp://127.0.0.1:{}", args.command_port);

    let config = MonitorConfig {
        subscribe_address: sub_addr,
        command_address: cmd_addr.clone(),
        http_port: args.http_port,
        ..MonitorConfig::default()
    };
    let mut monitor = Monitor::new(config).await?;
    let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
    let run_handle = tokio::spawn(async move { monitor.run(shutdown_rx).await });

    for cmd in [
        Command::Configure(RunConfig {
            run_number: 1,
            comment: String::new(),
            exp_name: "overload_bench".to_string(),
        }),
        Command::Arm,
        Command::Start { run_number: 1 },
    ] {
        let reply = send_command(&zctx, &cmd_addr, &cmd)?;
        anyhow::ensure!(reply.success, "{cmd} failed: {}", reply.message);
    }

    let stop = Arc::new(AtomicBool::new(false));
    let stop_for_pub = stop.clone();
    let bytes = batch_bytes(args.events_per_batch);
    let batch_len = bytes.len();
    let pace = args.batches_per_sec;
    let blaster = std::thread::spawn(move || {
        let period = 1_000_000u64
            .checked_div(pace)
            .map(Duration::from_micros)
            .unwrap_or(Duration::ZERO);
        let mut next = Instant::now();
        let mut sent = 0u64;
        while !stop_for_pub.load(Ordering::Relaxed) {
            publisher.send(&bytes[..], 0).expect("send batch");
            sent += 1;
            if period.is_zero() {
                continue;
            }
            next += period;
            let now = Instant::now();
            if next > now {
                std::thread::sleep(next - now);
            } else {
                next = now;
            }
        }
        sent
    });

    let events_per_batch = args.events_per_batch as u64;
    println!(
        "input: {} batches/s x {} events = {:.2} M events/s ({:.1} MB/s)",
        pace,
        events_per_batch,
        (pace * events_per_batch) as f64 / 1e6,
        pace as f64 * batch_len as f64 / 1e6
    );
    println!(
        "{:>6} {:>9} {:>14} {:>8} {:>10} {:>8} {:>8} {:>9}",
        "t[s]", "recv", "processed_ev", "dropped", "rate[M/s]", "http_ms", "cmd_ms", "rss_MB"
    );

    let client = reqwest::Client::new();
    let status_url = format!("http://127.0.0.1:{}/api/status", args.http_port);
    let t0 = Instant::now();
    let sample = Duration::from_millis(args.sample_ms);
    let mut last_print = Instant::now();
    let mut last_proc_print = 0u64;
    let mut prev: Option<(f64, u64, u64)> = None;
    let mut stall_start: Option<f64> = None;
    let mut longest_stall = 0.0f64;
    let mut max_http = Duration::ZERO;
    let mut max_cmd = Duration::ZERO;
    let mut max_rss = 0.0f64;
    let mut first_proc_at: Option<f64> = None;
    let mut last = (0u64, 0u64, 0u64);
    let mut http_total_events = 0u64;

    while t0.elapsed() < Duration::from_secs(args.duration) {
        tokio::time::sleep(sample).await;
        let t = t0.elapsed().as_secs_f64();

        // Command path (REP socket), from a blocking thread like the Operator.
        let ctx = zctx.clone();
        let addr = cmd_addr.clone();
        let started = Instant::now();
        let reply = tokio::time::timeout(
            Duration::from_secs(5),
            tokio::task::spawn_blocking(move || send_command(&ctx, &addr, &Command::GetStatus)),
        )
        .await;
        let cmd_latency = started.elapsed();
        max_cmd = max_cmd.max(cmd_latency);
        let (recv, proc, dropped) = match reply {
            Ok(Ok(Ok(r))) => (
                parse_counter(&r.message, "Received:").unwrap_or(0),
                r.metrics.map(|m| m.events_processed).unwrap_or(0),
                parse_counter(&r.message, "Dropped:").unwrap_or(0),
            ),
            _ => last,
        };
        last = (recv, proc, dropped);

        // HTTP path.
        let started = Instant::now();
        let http =
            tokio::time::timeout(Duration::from_secs(5), client.get(&status_url).send()).await;
        let http_latency = started.elapsed();
        max_http = max_http.max(http_latency);
        if let Ok(Ok(resp)) = http {
            if let Ok(status) = resp.json::<StatusJson>().await {
                http_total_events = status.total_events;
            }
        }

        if proc > 0 && first_proc_at.is_none() {
            first_proc_at = Some(t);
        }
        if let Some((pt, precv, pproc)) = prev {
            if recv > precv && proc == pproc {
                let start = *stall_start.get_or_insert(pt);
                longest_stall = longest_stall.max(t - start);
            } else {
                stall_start = None;
            }
        }
        prev = Some((t, recv, proc));

        let rss = rss_mb().unwrap_or(0.0);
        max_rss = max_rss.max(rss);

        if last_print.elapsed() >= Duration::from_secs(1) {
            let dt = last_print.elapsed().as_secs_f64();
            let rate = (proc - last_proc_print) as f64 / dt / 1e6;
            println!(
                "{:>6.1} {:>9} {:>14} {:>8} {:>10.2} {:>8.1} {:>8.1} {:>9.0}",
                t,
                recv,
                proc,
                dropped,
                rate,
                http_latency.as_secs_f64() * 1e3,
                cmd_latency.as_secs_f64() * 1e3,
                rss
            );
            last_print = Instant::now();
            last_proc_print = proc;
        }
    }

    stop.store(true, Ordering::Relaxed);
    let sent = blaster.join().expect("blaster thread");
    let (recv, proc, dropped) = last;
    let span = t0.elapsed().as_secs_f64() - first_proc_at.unwrap_or(0.0);
    println!();
    println!("sent batches        : {sent}");
    println!("received batches    : {recv}");
    println!("dropped batches     : {dropped}");
    println!("processed events    : {proc} (HTTP total_events at last poll: {http_total_events})");
    println!(
        "mean processed rate : {:.2} M events/s",
        proc as f64 / span.max(1e-9) / 1e6
    );
    println!("longest stall       : {longest_stall:.2} s");
    println!(
        "max GET /api/status : {:.1} ms",
        max_http.as_secs_f64() * 1e3
    );
    println!(
        "max GetStatus (REP) : {:.1} ms",
        max_cmd.as_secs_f64() * 1e3
    );
    println!("max RSS             : {max_rss:.0} MB");

    let _ = shutdown_tx.send(());
    if tokio::time::timeout(Duration::from_secs(10), run_handle)
        .await
        .is_err()
    {
        println!("Monitor::run did not return within 10 s after shutdown");
    }
    Ok(())
}
