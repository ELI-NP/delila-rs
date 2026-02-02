//! Check ChRecordLengthS parameter for DPP_OPEN firmware

use delila_rs::reader::CaenHandle;

fn main() -> anyhow::Result<()> {
    let url = "dig2://172.18.4.56";
    println!("Connecting to {}", url);

    let handle = CaenHandle::open(url)?;
    println!("Connected!");

    // Check board-level parameters related to waveform
    println!("\n--- Checking board-level waveform parameters ---");
    let board_params = [
        "/par/RecordLengthS",
        "/par/RecordLengthT",
        "/par/PreTriggerS",
        "/par/PreTriggerT",
        "/par/WaveAnalogProbe0",
        "/par/WaveAnalogProbe1",
        "/par/WaveDigitalProbe0",
        "/par/WaveDigitalProbe1",
        "/par/WaveSaving",
        "/par/WaveResolution",
    ];

    for name in &board_params {
        if let Ok(v) = handle.get_value(name) {
            println!("  {}: {}", name, v);
        }
    }

    // Check channel 0 specific parameters
    println!("\n--- Checking channel 0 parameters ---");
    let ch_params = [
        "/ch/0/par/ChRecordLengthS",
        "/ch/0/par/ChRecordLengthT",
        "/ch/0/par/ChPreTriggerS",
        "/ch/0/par/ChPreTriggerT",
        "/ch/0/par/WaveDataSource",
        "/ch/0/par/WaveResolution",
        "/ch/0/par/WaveSaving",
    ];

    for name in &ch_params {
        if let Ok(v) = handle.get_value(name) {
            println!("  {}: {}", name, v);
        }
    }

    // Try to set RecordLengthS at board level
    println!("\n--- Trying to set /par/RecordLengthS ---");
    for len in [500, 1024, 2048, 4096, 8192].iter() {
        match handle.set_value("/par/RecordLengthS", &len.to_string()) {
            Ok(()) => {
                println!("  Set to {}: OK", len);
                if let Ok(v) = handle.get_value("/par/RecordLengthS") {
                    println!("    Read back: {}", v);
                }
            }
            Err(e) => println!("  Set to {}: {}", len, e),
        }
    }

    Ok(())
}
