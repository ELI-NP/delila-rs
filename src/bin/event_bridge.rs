//! Event Bridge - converts MessagePack events to fixed binary format for C++ Event Builder
//!
//! Usage:
//!   cargo run --bin event_bridge
//!   cargo run --bin event_bridge -- -s tcp://localhost:5556 -p tcp://*:5600

use clap::Parser;
use delila_rs::common::{pub_no_hwm, setup_shutdown_with_message, sub_no_hwm, EventData, Message};
use futures::{SinkExt, StreamExt};
use tmq::{publish, subscribe, Context};
use tracing::{debug, info, warn};
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(
    name = "event_bridge",
    about = "MessagePack to fixed binary bridge for C++ Event Builder"
)]
struct Args {
    /// Merger PUB address to subscribe to
    #[arg(short = 's', long, default_value = "tcp://localhost:5556")]
    sub_address: String,

    /// PUB address to bind for C++ Event Builder
    #[arg(short = 'p', long, default_value = "tcp://*:5600")]
    pub_address: String,
}

// Wire format constants (see docs/event_bridge_wire_format.md)
const MSG_DATA: u8 = 0x01;
const MSG_EOS: u8 = 0x02;
const MSG_HEARTBEAT: u8 = 0x03;
const HIT_SIZE: usize = 14;

/// Encode EventData batch to fixed binary format
fn encode_data(events: &[EventData]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(5 + events.len() * HIT_SIZE);
    buf.push(MSG_DATA);
    buf.extend_from_slice(&(events.len() as u32).to_le_bytes());
    for ev in events {
        buf.push(ev.module);
        buf.push(ev.channel);
        buf.extend_from_slice(&ev.energy.to_le_bytes());
        buf.extend_from_slice(&ev.energy_short.to_le_bytes());
        buf.extend_from_slice(&ev.timestamp_ns.to_le_bytes());
    }
    buf
}

/// Encode control message (EOS/Heartbeat)
fn encode_control(msg_type: u8) -> Vec<u8> {
    let mut buf = Vec::with_capacity(5);
    buf.push(msg_type);
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("delila_rs=info".parse()?))
        .init();

    let args = Args::parse();

    // ZMQ sockets
    let context = Context::new();
    let mut sub_socket = subscribe(&context)
        .connect(&args.sub_address)?
        .subscribe(b"")?;
    // Never drop messages — buffer in memory instead (DAQ: no data loss)
    sub_no_hwm(&sub_socket)?;
    let mut pub_socket = publish(&context).bind(&args.pub_address)?;
    pub_no_hwm(&pub_socket)?;

    info!(sub = %args.sub_address, pub_ = %args.pub_address, "Event Bridge started (HWM=0)");

    println!("========================================");
    println!("       DELILA Event Bridge");
    println!("========================================");
    println!();
    println!("  Subscribe: {}", args.sub_address);
    println!("  Publish:   {}", args.pub_address);
    println!();
    println!("  Press Ctrl+C to stop.");
    println!("========================================");

    // Shutdown handler
    let (_shutdown_tx, mut shutdown_rx) =
        setup_shutdown_with_message("Received Ctrl+C, shutting down...");

    let mut batches: u64 = 0;
    let mut hits: u64 = 0;

    loop {
        tokio::select! {
            biased;

            _ = shutdown_rx.recv() => {
                break;
            }

            msg = sub_socket.next() => {
                match msg {
                    Some(Ok(multipart)) => {
                        if let Some(data) = multipart.into_iter().next() {
                            let binary = match Message::from_msgpack(&data) {
                                Ok(Message::Data(batch)) => {
                                    let n = batch.events.len();
                                    let encoded = encode_data(&batch.events);
                                    batches += 1;
                                    hits += n as u64;
                                    if batches.is_multiple_of(10000) {
                                        info!(batches, hits, "Bridge forwarding stats");
                                    }
                                    debug!(n, seq = batch.sequence_number, "Forwarded batch");
                                    encoded
                                }
                                Ok(Message::EndOfStream { source_id, .. }) => {
                                    info!(source_id, "Forwarding EndOfStream");
                                    encode_control(MSG_EOS)
                                }
                                Ok(Message::Heartbeat(hb)) => {
                                    debug!(source_id = hb.source_id, "Forwarding heartbeat");
                                    encode_control(MSG_HEARTBEAT)
                                }
                                Err(e) => {
                                    warn!(error = %e, "Failed to deserialize message");
                                    continue;
                                }
                            };

                            let zmq_msg: tmq::Multipart =
                                vec![tmq::Message::from(binary.as_slice())].into();
                            if let Err(e) = pub_socket.send(zmq_msg).await {
                                warn!(error = %e, "Failed to send message");
                            }
                        }
                    }
                    Some(Err(e)) => {
                        warn!(error = %e, "ZMQ receive error");
                    }
                    None => {
                        info!("SUB socket closed");
                        break;
                    }
                }
            }
        }
    }

    info!(batches, hits, "Event Bridge stopped");
    println!(
        "Event Bridge stopped. Forwarded {} batches, {} hits.",
        batches, hits
    );
    Ok(())
}
