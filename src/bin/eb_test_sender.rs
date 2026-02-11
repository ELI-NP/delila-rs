//! Event Builder Test Sender
//!
//! Reads hits from ROOT files, shuffles them within chunks to simulate
//! realistic time disorder, converts to EventData, and sends via ZMQ PUB.
//!
//! Usage:
//!   cargo run --features root --bin eb_test_sender -- \
//!       --input run0113_0000.root --publish tcp://*:5557

use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;
use futures::SinkExt;
use tmq::{publish, AsZmqSocket, Context};
use tracing::info;
use tracing_subscriber::EnvFilter;

use delila_rs::common::{EventData, EventDataBatch, Message};

#[cfg(feature = "root")]
use delila_rs::event_builder::read_hits_from_root;

#[derive(Parser, Debug)]
#[command(
    name = "eb_test_sender",
    about = "Send ROOT hit data via ZMQ PUB for Online Event Builder testing"
)]
struct Args {
    /// Input ROOT file(s)
    #[arg(short, long, required = true, num_args = 1..)]
    input: Vec<PathBuf>,

    /// ROOT TTree name
    #[arg(long, default_value = "ELIADE_Tree")]
    tree_name: String,

    /// ZMQ PUB bind address
    #[arg(short, long, default_value = "tcp://*:5557")]
    publish: String,

    /// Chunk size in nanoseconds for shuffling
    /// (hits within this window are shuffled randomly)
    #[arg(long, default_value_t = 30_000_000.0)]
    chunk_size_ns: f64,

    /// Batch size (events per ZMQ message)
    #[arg(long, default_value_t = 100)]
    batch_size: usize,

    /// Delay between batches in milliseconds (0 = no delay)
    #[arg(long, default_value_t = 1)]
    delay_ms: u64,

    /// Send EOS after all data
    #[arg(long, default_value_t = true)]
    send_eos: bool,
}

#[cfg(feature = "root")]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("delila_rs=info".parse()?))
        .init();

    let args = Args::parse();

    // Read all hits from ROOT files
    info!("Reading hits from {} file(s)...", args.input.len());
    let mut all_hits = Vec::new();
    for path in &args.input {
        let hits = read_hits_from_root(path, &args.tree_name)?;
        info!(file = %path.display(), hits = hits.len(), "Read hits");
        all_hits.extend(hits);
    }

    if all_hits.is_empty() {
        anyhow::bail!("No hits found in input files");
    }

    // Sort by time first (they should already be sorted, but ensure it)
    all_hits.sort_by(|a, b| {
        a.timestamp_ns
            .partial_cmp(&b.timestamp_ns)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let total_hits = all_hits.len();
    let time_range = all_hits.last().unwrap().timestamp_ns - all_hits.first().unwrap().timestamp_ns;
    info!(
        total_hits = total_hits,
        time_range_ms = time_range / 1_000_000.0,
        "Loaded all hits"
    );

    // Split into chunks and shuffle within each chunk
    info!(
        chunk_size_ns = args.chunk_size_ns,
        "Splitting into chunks and shuffling..."
    );

    use rand::seq::SliceRandom;
    let mut rng = rand::thread_rng();

    let mut chunks: Vec<Vec<delila_rs::event_builder::Hit>> = Vec::new();
    let mut chunk_start = all_hits.first().unwrap().timestamp_ns;

    let mut current_chunk = Vec::new();
    for hit in all_hits {
        if hit.timestamp_ns > chunk_start + args.chunk_size_ns && !current_chunk.is_empty() {
            current_chunk.shuffle(&mut rng);
            chunks.push(std::mem::take(&mut current_chunk));
            chunk_start = hit.timestamp_ns;
        }
        current_chunk.push(hit);
    }
    if !current_chunk.is_empty() {
        current_chunk.shuffle(&mut rng);
        chunks.push(current_chunk);
    }

    info!(chunks = chunks.len(), "Created shuffled chunks");

    // Create ZMQ PUB socket
    let context = Context::new();
    let mut socket = publish(&context).bind(&args.publish)?;
    // Never drop messages — buffer in memory instead (DAQ: no data loss)
    socket.get_socket().set_sndhwm(0)?;
    info!(address = %args.publish, "ZMQ PUB socket bound (SNDHWM=0)");

    // Give subscribers time to connect
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Send data
    let mut sent_hits = 0u64;
    let mut sent_batches = 0u64;
    let mut batch_events = Vec::new();

    for chunk in &chunks {
        for hit in chunk {
            // Convert Hit → EventData
            let event = EventData::new(
                hit.module,
                hit.channel,
                hit.energy,
                hit.energy_short,
                hit.timestamp_ns,
                0,
            );
            batch_events.push(event);

            // Send batch when full
            if batch_events.len() >= args.batch_size {
                let batch = EventDataBatch {
                    source_id: 0,
                    sequence_number: sent_batches,
                    timestamp: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos() as u64,
                    events: std::mem::take(&mut batch_events),
                };
                sent_hits += batch.events.len() as u64;

                let msg = Message::data(batch);
                let bytes = msg.to_msgpack()?;
                let zmq_msg: tmq::Multipart =
                    vec![tmq::Message::from(bytes.as_slice())].into();
                socket.send(zmq_msg).await?;

                sent_batches += 1;

                if args.delay_ms > 0 {
                    tokio::time::sleep(Duration::from_millis(args.delay_ms)).await;
                }

                if sent_batches.is_multiple_of(1000) {
                    info!(
                        batches = sent_batches,
                        hits = sent_hits,
                        "Sending progress"
                    );
                }
            }
        }
    }

    // Send remaining events
    if !batch_events.is_empty() {
        let batch = EventDataBatch {
            source_id: 0,
            sequence_number: sent_batches,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64,
            events: std::mem::take(&mut batch_events),
        };
        sent_hits += batch.events.len() as u64;

        let msg = Message::data(batch);
        let bytes = msg.to_msgpack()?;
        let zmq_msg: tmq::Multipart = vec![tmq::Message::from(bytes.as_slice())].into();
        socket.send(zmq_msg).await?;
        sent_batches += 1;
    }

    // Send EOS
    if args.send_eos {
        let eos = Message::eos(0);
        let bytes = eos.to_msgpack()?;
        let zmq_msg: tmq::Multipart = vec![tmq::Message::from(bytes.as_slice())].into();
        socket.send(zmq_msg).await?;
        info!("Sent EOS");
    }

    println!();
    println!("========================================");
    println!("  Test Sender Complete");
    println!("========================================");
    println!("  Total hits sent:    {}", sent_hits);
    println!("  Total batches:      {}", sent_batches);
    println!("  Chunk size:         {} ms", args.chunk_size_ns / 1_000_000.0);
    println!("========================================");

    // Small delay to let ZMQ flush
    tokio::time::sleep(Duration::from_millis(500)).await;

    Ok(())
}

#[cfg(not(feature = "root"))]
fn main() {
    eprintln!("Error: ROOT feature not enabled. Build with --features root");
    std::process::exit(1);
}
