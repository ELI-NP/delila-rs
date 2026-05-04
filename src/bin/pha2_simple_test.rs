//! Minimal PHA2 firmware sanity-check.
//!
//! Sequence is *strict*: Open → Reset → Configure → Start → Read → Stop → Close.
//! No threads, no decoder, no ZMQ — just the FELib calls and a hex dump of the
//! first few aggregates so we can hand-decode and rule out / confirm whether
//! the waveform-truncation we see in the full pipeline is real FW behaviour
//! or a delila-rs decoder false-positive.
//!
//! Build: `cargo build --release --bin pha2_simple_test`
//! Run:   `./target/release/pha2_simple_test --url dig2://172.18.4.56`

use std::time::Instant;

use clap::Parser;
use delila_rs::reader::caen::CaenHandle;

#[derive(Parser, Debug)]
#[command(about = "Minimal PHA2 acquisition test (Open→Reset→Configure→Start→Read→Stop→Close)")]
struct Args {
    /// Digitizer URL (e.g. dig2://172.18.4.56)
    #[arg(long, default_value = "dig2://172.18.4.56")]
    url: String,

    /// Per-channel record length in nanoseconds.
    #[arg(long, default_value_t = 8192)]
    record_length_ns: u32,

    /// TestPulse period in nanoseconds. 1_000_000 ≈ 1 kHz.
    #[arg(long, default_value_t = 1_000_000)]
    test_pulse_period_ns: u32,

    /// Acquire data for this many seconds.
    #[arg(long, default_value_t = 3.0)]
    duration_s: f64,

    /// `wave_trigger_source = Disabled` → list mode (no waveform).
    #[arg(long)]
    no_waveform: bool,

    /// First N aggregates get a hex dump.
    #[arg(long, default_value_t = 3)]
    dump_aggregates: usize,

    /// Words per aggregate to dump (0 = whole aggregate).
    #[arg(long, default_value_t = 280)]
    dump_words: usize,

    /// Override `wavedigitalprobe0..3` (comma-separated, exactly 4 names).
    /// Helpful for ruling out a decoder false-positive that fires when a
    /// digital-probe bit ends up at bit 63 of a sample word.
    #[arg(long, value_delimiter = ',')]
    digital_probes: Option<Vec<String>>,

    /// Set `wavedownsamplingfactor` (1, 2, 4 or 8). 1 = no downsampling.
    /// PHA2 spec is unclear on whether `wf_size` reports pre- or
    /// post-downsampling word count; this lets us probe the FW directly.
    #[arg(long, default_value_t = 1)]
    wave_downsampling: u32,
}

fn pretty_word(idx: usize, word: u64) -> String {
    let bit63 = (word >> 63) & 0x1;
    let bits62_60 = (word >> 60) & 0x7;
    let tag = if bit63 == 1 && bits62_60 == 0 {
        " ← wf_header pattern (bit63=1, bits[62:60]=0)"
    } else {
        ""
    };
    format!("[{idx:5}] 0x{word:016x}{tag}")
}

fn run() -> anyhow::Result<()> {
    let args = Args::parse();
    let target_rate_khz = 1_000_000_000.0 / args.test_pulse_period_ns as f64 / 1000.0;

    println!("=== PHA2 Simple Test ===");
    println!("URL:               {}", args.url);
    println!("record_length_ns:  {}", args.record_length_ns);
    println!("test_pulse_period: {} ns ({:.1} kHz)", args.test_pulse_period_ns, target_rate_khz);
    println!("duration:          {} s", args.duration_s);
    println!("no_waveform:       {}", args.no_waveform);
    if let Some(ref dp) = args.digital_probes {
        println!("digital_probes:    {:?}", dp);
    }
    println!();

    // [1] Open
    println!("[1] Open");
    let handle = CaenHandle::open(&args.url)?;

    // [2] Reset
    println!("[2] Reset");
    handle.send_command("/cmd/reset")?;

    // [3] Configure parameters (board first, then channel 0)
    println!("[3] Configure");
    handle.set_value("/par/globaltriggersource", "TestPulse")?;
    handle.set_value("/par/testpulseperiod", &args.test_pulse_period_ns.to_string())?;
    handle.set_value("/par/testpulsewidth", "104")?; // step=8 (CAEN snaps 100→104)
    handle.set_value("/par/testpulselowlevel", "0")?;
    handle.set_value("/par/testpulsehighlevel", "6000")?;
    handle.set_value("/par/startsource", "SWcmd")?;

    // Disable every channel except 0 to keep the dump clean.
    handle.set_value("/ch/0..31/par/chenable", "False")?;
    handle.set_value("/ch/0/par/chenable", "True")?;
    handle.set_value("/ch/0/par/pulsepolarity", "Negative")?;
    handle.set_value("/ch/0/par/dcoffset", "50.0")?;
    handle.set_value("/ch/0/par/triggerthr", "100")?;
    handle.set_value("/ch/0/par/eventtriggersource", "GlobalTriggerSource")?;
    handle.set_value("/ch/0/par/chrecordlengtht", &args.record_length_ns.to_string())?;
    handle.set_value("/ch/0/par/chpretriggert", "1024")?;

    if args.no_waveform {
        handle.set_value("/ch/0/par/wavetriggersource", "Disabled")?;
        handle.set_value("/ch/0/par/wavesaving", "OnRequest")?;
    } else {
        handle.set_value("/ch/0/par/wavetriggersource", "GlobalTriggerSource")?;
        handle.set_value("/ch/0/par/wavesaving", "Always")?;
        handle.set_value(
            "/ch/0/par/wavedownsamplingfactor",
            &args.wave_downsampling.to_string(),
        )?;
        handle.set_value("/ch/0/par/waveanalogprobe0", "ADCInput")?;
        handle.set_value("/ch/0/par/waveanalogprobe1", "EnergyFilter")?;
        let probes = args.digital_probes.clone().unwrap_or_else(|| {
            vec![
                "Trigger".into(),
                "TimeFilterArmed".into(),
                "EnergyFilterPeakReady".into(),
                "EnergyFilterPeaking".into(),
            ]
        });
        if probes.len() != 4 {
            anyhow::bail!("--digital-probes needs exactly 4 names, got {}", probes.len());
        }
        for (i, p) in probes.iter().enumerate() {
            handle.set_value(&format!("/ch/0/par/wavedigitalprobe{i}"), p)?;
        }
    }

    // [3b] Configure RAW endpoint (must be after /cmd/reset; reset wipes it)
    println!("[3b] Configure endpoint (RAW)");
    let endpoint = handle.configure_endpoint(true)?;

    // [4] Arm + Start
    println!("[4] Arm + swstartacquisition");
    handle.send_command("/cmd/armacquisition")?;
    handle.send_command("/cmd/swstartacquisition")?;

    // [5] Read aggregates
    println!("[5] Read for {} s", args.duration_s);
    let mut buffer: Vec<u8> = vec![0u8; 16 * 1024 * 1024]; // 16 MB
    let started = Instant::now();
    let mut aggregates = 0u64;
    let mut total_bytes = 0u64;
    let mut total_events = 0u64;

    while started.elapsed().as_secs_f64() < args.duration_s {
        match endpoint.read_data(100, &mut buffer)? {
            Some(raw) => {
                aggregates += 1;
                total_bytes += raw.size as u64;
                total_events += raw.n_events as u64;

                if (aggregates as usize) <= args.dump_aggregates {
                    let words = raw.size / 8;
                    let dump_n = if args.dump_words == 0 {
                        words
                    } else {
                        words.min(args.dump_words)
                    };
                    println!(
                        "\n--- Aggregate #{aggregates}  size={} bytes ({} words)  n_events_hdr={}",
                        raw.size, words, raw.n_events,
                    );
                    for i in 0..dump_n {
                        let off = i * 8;
                        let word = u64::from_be_bytes(raw.data[off..off + 8].try_into().unwrap());
                        println!("  {}", pretty_word(i, word));
                    }
                    if dump_n < words {
                        println!("  … {} more words …", words - dump_n);
                    }
                }
            }
            None => {
                // Timeout — just keep polling.
            }
        }
    }

    let elapsed = started.elapsed().as_secs_f64();

    // [6] Stop
    println!("\n[6] Stop");
    handle.send_command("/cmd/swstopacquisition")?;
    handle.send_command("/cmd/disarmacquisition")?;

    // [7] Close (both EndpointHandle and CaenHandle release on scope exit;
    //     EndpointHandle drops first per struct field order, which is what
    //     the FELib expects so the connection isn't yanked while a read is
    //     in flight).
    println!("[7] Close");
    let _ = endpoint;
    let _ = handle;

    println!(
        "\nSummary:\n  aggregates: {}\n  events_hdr: {}\n  bytes:      {} ({:.2} MB)\n  rate:       {:.1} aggregates/s, {:.1} events/s",
        aggregates,
        total_events,
        total_bytes,
        total_bytes as f64 / 1e6,
        aggregates as f64 / elapsed,
        total_events as f64 / elapsed,
    );

    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}
