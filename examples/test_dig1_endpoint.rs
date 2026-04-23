//! Test DIG1 endpoint configuration, data readout, and PHA1 decoding
//! Channel 4 has external pulser connected
use delila_rs::reader::caen::CaenHandle;
use delila_rs::reader::decoder::{Pha1Config, Pha1Decoder, RawData};
use std::time::Duration;

fn main() {
    let url = "dig1://caen.internal/usb?link_num=0";
    println!("Connecting to: {}", url);

    let handle = CaenHandle::open(url).expect("Failed to open device");
    println!("Connected!");

    // Get device info
    let info = handle.get_device_info().expect("Failed to get device info");
    println!(
        "Device: {} (SN: {}, FW: {})",
        info.model, info.serial_number, info.firmware_type
    );

    // Reset digitizer first
    println!("\nResetting digitizer...");
    if let Err(e) = handle.send_command("/cmd/reset") {
        println!("  Reset failed: {} (continuing anyway)", e);
    } else {
        println!("  Reset OK");
        std::thread::sleep(Duration::from_millis(500));
    }

    // Configure channel 4 (has external pulser)
    println!("\nConfiguring channel 4 (external pulser)...");
    let _ = handle.set_value("/ch/4/par/ch_enabled", "TRUE");
    let _ = handle.set_value("/ch/4/par/ch_self_trg_enable", "TRUE");
    let _ = handle.set_value("/ch/4/par/ch_threshold", "100");
    println!("  Channel 4 configured");

    // Configure endpoint (DIG1 mode - no N_EVENTS)
    println!("\nConfiguring endpoint...");
    let endpoint = handle
        .configure_endpoint(false)
        .expect("Failed to configure endpoint");
    println!("Endpoint configured!");

    // Create PHA1 decoder
    let mut decoder = Pha1Decoder::new(Pha1Config {
        time_step_ns: 2.0, // 500 MHz -> 2ns per sample
        module_id: 0,
        dump_enabled: true, // Enable debug output
    });

    // Start acquisition
    println!("\nArming/Starting acquisition...");
    handle
        .send_command("/cmd/armacquisition")
        .expect("Failed to arm");
    println!("Acquisition started!");

    std::thread::sleep(Duration::from_millis(2000));

    // Read and decode data
    println!("\nReading and decoding data...");
    let mut buffer = vec![0u8; 64 * 1024 * 1024];
    let mut total_events = 0usize;

    for i in 0..5 {
        match endpoint.read_data(1000, &mut buffer) {
            Ok(Some(raw)) => {
                println!("\n=== Read {}: {} bytes ===", i, raw.size);

                // Create RawData for decoder
                let raw_data = RawData {
                    data: raw.data[..raw.size].to_vec(),
                    size: raw.size,
                    n_events: 0, // DIG1 doesn't provide this
                };

                // Decode
                let events = decoder.decode(&raw_data);
                println!("Decoded {} events", events.len());
                total_events += events.len();

                // Show first few events
                for (j, event) in events.iter().take(3).enumerate() {
                    println!(
                        "  Event {}: ch={}, ts={:.3}ns, energy={}, extra={}",
                        j, event.channel, event.timestamp_ns, event.energy, event.energy_short
                    );
                }
                if events.len() > 3 {
                    println!("  ... and {} more events", events.len() - 3);
                }
            }
            Ok(None) => println!("Read {}: No data (timeout)", i),
            Err(e) => println!("Read {}: Error - {}", i, e),
        }
    }

    println!("\n=== Total decoded events: {} ===", total_events);

    // Stop acquisition
    println!("\nStopping acquisition...");
    handle
        .send_command("/cmd/disarmacquisition")
        .expect("Failed to stop");
    println!("Done!");
}
