//! V1743 connection test utility
//!
//! Opens a V1743 digitizer, reads board info, tests basic register access,
//! and optionally runs a software trigger test.
//!
//! Usage:
//!   cargo run --release --features x743 --bin x743_test -- [options]
//!
//! Options:
//!   --link-type <optical|usb>  Connection type (default: optical)
//!   --link-num <N>             Link/port number (default: 0)
//!   --conet-node <N>           CONET daisy chain node (default: 0)
//!   --base-address <hex>       VME base address (default: 0)
//!   --trigger-test             Run software trigger test (acquire a few events)
//!   --pulse-test               Enable test pulse generator

use clap::Parser;
use delila_rs::reader::caen_legacy::*;
use std::time::{Duration, Instant};
use tracing::{error, info, warn};

#[derive(Parser)]
#[command(name = "x743_test", about = "V1743 digitizer connection test")]
struct Args {
    /// Connection type: "optical" or "usb"
    #[arg(long, default_value = "optical")]
    link_type: String,

    /// Link/port number
    #[arg(long, default_value_t = 0)]
    link_num: u32,

    /// CONET daisy chain node
    #[arg(long, default_value_t = 0)]
    conet_node: u32,

    /// VME base address (hex, e.g., 0x32100000)
    #[arg(long, default_value = "0")]
    base_address: String,

    /// Run software trigger test
    #[arg(long)]
    trigger_test: bool,

    /// Enable test pulse generator
    #[arg(long)]
    pulse_test: bool,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();

    let link_type = match args.link_type.as_str() {
        "optical" | "opt" => ConnectionType::OpticalLink,
        "usb" => ConnectionType::USB,
        other => {
            error!("Unknown link type: {}. Use 'optical' or 'usb'", other);
            std::process::exit(1);
        }
    };

    let base_address = if args.base_address.starts_with("0x") || args.base_address.starts_with("0X")
    {
        u32::from_str_radix(&args.base_address[2..], 16)?
    } else {
        args.base_address.parse()?
    };

    // === Phase 1: Open + GetInfo ===
    info!("=== Phase 1: Open digitizer ===");
    let handle = X743Handle::open(link_type, args.link_num, args.conet_node, base_address)?;

    if let Some(info) = handle.board_info() {
        println!("\n=== V1743 Board Information ===");
        println!("  Model:          {}", info.model_name);
        println!("  Serial Number:  {}", info.serial_number);
        println!("  Channels:       {}", info.channels);
        println!("  ADC Bits:       {}", info.adc_nbits);
        println!("  ROC Firmware:   {}", info.roc_firmware);
        println!("  AMC Firmware:   {}", info.amc_firmware);
        println!("  Form Factor:    {}", info.form_factor);
        println!("  Family Code:    {}", info.family_code);
        println!(
            "  SAM Correction: {}",
            if info.sam_correction_loaded {
                "loaded"
            } else {
                "NOT loaded"
            }
        );
        println!();
    }

    // === Phase 2: Reset ===
    info!("=== Phase 2: Reset digitizer ===");
    handle.reset()?;
    println!("  Reset: OK");

    // === Phase 3: Register access test ===
    info!("=== Phase 3: Register access ===");

    // Read board fail status (0x8178)
    let board_fail = handle.read_register(0x8178)?;
    println!("  Board Fail Status (0x8178): 0x{:08X}", board_fail);
    if board_fail != 0 {
        warn!("Board fail status is non-zero!");
    }

    // Read acquisition status (0x8104)
    let acq_status = handle.read_register(0x8104)?;
    println!("  Acquisition Status (0x8104): 0x{:08X}", acq_status);

    // Read board info register (0x8140)
    let board_info_reg = handle.read_register(0x8140)?;
    println!("  Board Info (0x8140): 0x{:08X}", board_info_reg);

    // === Phase 4: Software trigger test ===
    if args.trigger_test || args.pulse_test {
        info!("=== Phase 4: Trigger test ===");

        // Configure for test
        handle.set_group_enable_mask(0xFF)?; // Enable all groups
        handle.set_record_length(256)?;
        handle.set_sam_sampling_frequency(SamFrequency::Ghz3_2)?;
        handle.set_sam_correction_level(SamCorrectionLevel::All)?;
        handle.set_max_num_events_blt(100)?;
        handle.set_acquisition_mode(AcqMode::SWControlled)?;
        handle.set_io_level(IOLevel::NIM)?;

        if args.pulse_test {
            info!("Enabling test pulse generator on ch0");
            handle.enable_sam_pulse_gen(0, 0xFFFF, SamPulseSource::Continuous)?;
            // Set self-trigger on ch0
            handle.set_channel_self_trigger(TriggerMode::AcqOnly, 0x0001)?;
        } else {
            // Software trigger mode
            handle.set_sw_trigger_mode(TriggerMode::AcqOnly)?;
            handle.set_ext_trigger_input_mode(TriggerMode::Disabled)?;
        }

        // Allocate buffers
        let mut readout_buf = handle.malloc_readout_buffer()?;
        let mut event_buf = handle.allocate_event()?;

        // Start acquisition
        handle.clear_data()?;
        handle.sw_start_acquisition()?;
        println!("  Acquisition started");

        let start = Instant::now();
        let mut total_events = 0u32;

        // Send SW triggers and read events
        for i in 0..10 {
            if !args.pulse_test {
                handle.send_sw_trigger()?;
            }
            std::thread::sleep(Duration::from_millis(100));

            let data_size = handle.read_data(&mut readout_buf)?;
            if data_size == 0 {
                continue;
            }

            let num_events = handle.get_num_events(&readout_buf, data_size)?;
            total_events += num_events;

            for evt_idx in 0..num_events {
                let (event_info, event_ptr) =
                    handle.get_event_info(&readout_buf, data_size, evt_idx)?;
                handle.decode_event(event_ptr, &mut event_buf)?;

                let event = event_buf.event();

                // Print first few events
                if total_events - num_events + evt_idx < 3 {
                    println!(
                        "  Event {}: counter={}, trigger_time_tag={}, group_mask=0x{:02X}",
                        evt_idx, event_info.EventCounter, event_info.TriggerTimeTag,
                        event_info.ChannelMask,
                    );

                    for g in 0..8usize {
                        if event.GrPresent[g] == 0 {
                            continue;
                        }
                        let group = &event.DataGroup[g];
                        println!(
                            "    Group {}: ChSize={}, TDC={}, Charge={:.2}, Peak={:.2}, Baseline={:.2}",
                            g, group.ChSize, group.TDC, group.Charge, group.Peak, group.Baseline,
                        );
                    }
                }
            }

            println!(
                "  Iteration {}: {} events (total: {})",
                i, num_events, total_events
            );
        }

        handle.sw_stop_acquisition()?;
        let elapsed = start.elapsed();

        println!("\n=== Results ===");
        println!("  Total events: {}", total_events);
        println!("  Elapsed: {:.2}s", elapsed.as_secs_f64());
        if total_events > 0 {
            println!(
                "  Rate: {:.1} events/s",
                total_events as f64 / elapsed.as_secs_f64()
            );
        }

        if args.pulse_test {
            handle.disable_sam_pulse_gen(0)?;
        }
    }

    // Handle dropped here → CloseDigitizer called automatically
    println!("\n=== Test complete ===");
    Ok(())
}
