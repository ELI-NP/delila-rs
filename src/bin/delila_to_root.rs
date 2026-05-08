//! delila_to_root — Convert .delila files to a flat ROOT TTree.
//!
//! Each row in the output TTree is one decoded event with the full
//! current EventData schema (including the PHA2/AMax `user_info[4]` and
//! `fine_time` fields that the legacy `tools/delila2root/` C++ tool
//! does not yet handle). Waveform data is intentionally skipped — use
//! the recover/event_builder pipeline if you need waveform export.
//!
//! Usage:
//!     cargo run --release --features root --bin delila_to_root -- \
//!         -o out.root data/run0001_0020_PHA2_Phys.delila [more.delila ...]
//!
//! Output branches:
//!     Module (u8), Channel (u8), TimestampNs (f64), Energy (u16),
//!     EnergyShort (u16), Flags (u64),
//!     UserInfo0..UserInfo3 (u64), HasWaveform (u8),
//!     AnalogProbeType0/1 (u8), DigitalProbeType0..3 (u8) — PHA2 canonical
//!     codes (0=ADCInput / 1=TimeFilter / … for analog, 0=Trigger / … for
//!     digital, 0xFF=`UNKNOWN_PROBE_TYPE` for FW that doesn't carry typed
//!     probe info on the wire). Events without a waveform get all 0xFF.
//!
//! Note: the on-disk schema folds the decoder's `fine_time` into
//! `timestamp_ns` (= coarse_ns + fine_time/1024 × time_step), so there
//! is no separate fine-time branch.
//!
//! Events are written in the order they appear in the input files
//! (no cross-file time sort — that's `event_builder`'s job).

use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::time::Instant;

use delila_rs::common::UNKNOWN_PROBE_TYPE;
use delila_rs::recorder::DataFileReader;
use oxyroot::{RootFile, WriterTree};

fn print_usage_and_exit() -> ! {
    eprintln!(
        "Usage: delila_to_root -o <output.root> [--tree <name>] <file1.delila> [file2.delila ...]"
    );
    std::process::exit(2);
}

fn main() {
    let mut args = std::env::args().skip(1).collect::<Vec<String>>();
    let mut out_path: Option<PathBuf> = None;
    let mut tree_name = String::from("delila");
    let mut inputs: Vec<PathBuf> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-o" => {
                if i + 1 >= args.len() {
                    print_usage_and_exit();
                }
                out_path = Some(PathBuf::from(args.remove(i + 1)));
                args.remove(i);
            }
            "--tree" => {
                if i + 1 >= args.len() {
                    print_usage_and_exit();
                }
                tree_name = args.remove(i + 1);
                args.remove(i);
            }
            "-h" | "--help" => print_usage_and_exit(),
            other => {
                inputs.push(PathBuf::from(other));
                i += 1;
            }
        }
    }

    let out_path = out_path.unwrap_or_else(|| {
        eprintln!("error: -o <output.root> is required");
        print_usage_and_exit()
    });
    if inputs.is_empty() {
        eprintln!("error: no input .delila files");
        print_usage_and_exit();
    }

    println!(
        "delila_to_root: {} input file(s) → {}",
        inputs.len(),
        out_path.display()
    );

    // Per-branch flat columns. We pre-allocate to keep the hot loop tight.
    let mut module: Vec<u8> = Vec::new();
    let mut channel: Vec<u8> = Vec::new();
    let mut timestamp_ns: Vec<f64> = Vec::new();
    let mut energy: Vec<u16> = Vec::new();
    let mut energy_short: Vec<u16> = Vec::new();
    let mut flags: Vec<u64> = Vec::new();
    let mut user_info0: Vec<u64> = Vec::new();
    let mut user_info1: Vec<u64> = Vec::new();
    let mut user_info2: Vec<u64> = Vec::new();
    let mut user_info3: Vec<u64> = Vec::new();
    let mut has_waveform: Vec<u8> = Vec::new();
    let mut analog_probe_type0: Vec<u8> = Vec::new();
    let mut analog_probe_type1: Vec<u8> = Vec::new();
    let mut analog_probe_type2: Vec<u8> = Vec::new();
    let mut digital_probe_type0: Vec<u8> = Vec::new();
    let mut digital_probe_type1: Vec<u8> = Vec::new();
    let mut digital_probe_type2: Vec<u8> = Vec::new();
    let mut digital_probe_type3: Vec<u8> = Vec::new();
    let mut digital_probe_type4: Vec<u8> = Vec::new();
    let mut digital_probe_type5: Vec<u8> = Vec::new();
    let mut digital_probe_type6: Vec<u8> = Vec::new();
    let mut digital_probe_type7: Vec<u8> = Vec::new();
    let mut digital_probe_type8: Vec<u8> = Vec::new();
    let mut digital_probe_type9: Vec<u8> = Vec::new();
    let mut digital_probe_type10: Vec<u8> = Vec::new();
    let mut digital_probe_type11: Vec<u8> = Vec::new();
    let mut digital_probe_type12: Vec<u8> = Vec::new();
    let mut digital_probe_type13: Vec<u8> = Vec::new();
    let mut digital_probe_type14: Vec<u8> = Vec::new();
    let mut digital_probe_type15: Vec<u8> = Vec::new();

    let start = Instant::now();
    let mut total_events = 0usize;
    let mut total_batches = 0usize;
    let mut total_bytes_in = 0u64;
    let mut decode_errors = 0usize;

    for path in &inputs {
        let file = match File::open(path) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("  error: cannot open {}: {}", path.display(), e);
                continue;
            }
        };
        total_bytes_in += std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        let reader = BufReader::new(file);
        let mut dfr = match DataFileReader::new(reader) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("  error: cannot read header of {}: {:?}", path.display(), e);
                continue;
            }
        };

        if let Some(h) = dfr.header() {
            println!(
                "  [hdr] {} run={} exp={:?} seq={}",
                path.display(),
                h.run_number,
                h.exp_name,
                h.file_sequence
            );
        }

        let mut file_events = 0usize;
        for batch_result in dfr.data_blocks() {
            match batch_result {
                Ok(batch) => {
                    total_batches += 1;
                    for ev in batch.events.iter() {
                        module.push(ev.module);
                        channel.push(ev.channel);
                        timestamp_ns.push(ev.timestamp_ns);
                        energy.push(ev.energy);
                        energy_short.push(ev.energy_short);
                        flags.push(ev.flags);
                        user_info0.push(ev.user_info[0]);
                        user_info1.push(ev.user_info[1]);
                        user_info2.push(ev.user_info[2]);
                        user_info3.push(ev.user_info[3]);
                        has_waveform.push(if ev.waveform.is_some() { 1 } else { 0 });
                        // Probe-type codes carried by PHA2 wf-extras header.
                        // Other FW emit UNKNOWN_PROBE_TYPE (=0xFF), and an
                        // event without a waveform also gets 0xFF.
                        let (apt, dpt) = match ev.waveform.as_ref() {
                            Some(wf) => (wf.analog_probe_type, wf.digital_probe_type),
                            None => ([UNKNOWN_PROBE_TYPE; 3], [UNKNOWN_PROBE_TYPE; 16]),
                        };
                        analog_probe_type0.push(apt[0]);
                        analog_probe_type1.push(apt[1]);
                        analog_probe_type2.push(apt[2]);
                        digital_probe_type0.push(dpt[0]);
                        digital_probe_type1.push(dpt[1]);
                        digital_probe_type2.push(dpt[2]);
                        digital_probe_type3.push(dpt[3]);
                        digital_probe_type4.push(dpt[4]);
                        digital_probe_type5.push(dpt[5]);
                        digital_probe_type6.push(dpt[6]);
                        digital_probe_type7.push(dpt[7]);
                        digital_probe_type8.push(dpt[8]);
                        digital_probe_type9.push(dpt[9]);
                        digital_probe_type10.push(dpt[10]);
                        digital_probe_type11.push(dpt[11]);
                        digital_probe_type12.push(dpt[12]);
                        digital_probe_type13.push(dpt[13]);
                        digital_probe_type14.push(dpt[14]);
                        digital_probe_type15.push(dpt[15]);
                        file_events += 1;
                    }
                }
                Err(e) => {
                    decode_errors += 1;
                    eprintln!("  warn: decode error in {}: {:?}", path.display(), e);
                }
            }
        }
        println!("  [done] {} events from {}", file_events, path.display());
        total_events += file_events;
    }

    let read_elapsed = start.elapsed();
    println!(
        "Read {} events in {} batches from {} file(s) in {:.2}s ({:.1} M ev/s)",
        total_events,
        total_batches,
        inputs.len(),
        read_elapsed.as_secs_f64(),
        (total_events as f64) / read_elapsed.as_secs_f64() / 1e6,
    );
    if decode_errors > 0 {
        eprintln!("warning: {} batch decode error(s) (skipped)", decode_errors);
    }
    if total_events == 0 {
        eprintln!("error: 0 events decoded — refusing to write empty ROOT file");
        std::process::exit(1);
    }

    // Now write the ROOT TTree. We move the per-branch vectors directly
    // into oxyroot's iterator API; nothing copies under the hood.
    let write_start = Instant::now();
    let mut file = RootFile::create(out_path.to_str().unwrap_or(""))
        .unwrap_or_else(|e| panic!("RootFile::create({}) failed: {:?}", out_path.display(), e));

    let mut tree = WriterTree::new(&tree_name);
    tree.new_branch("Module", module.into_iter());
    tree.new_branch("Channel", channel.into_iter());
    tree.new_branch("TimestampNs", timestamp_ns.into_iter());
    tree.new_branch("Energy", energy.into_iter());
    tree.new_branch("EnergyShort", energy_short.into_iter());
    tree.new_branch("Flags", flags.into_iter());
    tree.new_branch("UserInfo0", user_info0.into_iter());
    tree.new_branch("UserInfo1", user_info1.into_iter());
    tree.new_branch("UserInfo2", user_info2.into_iter());
    tree.new_branch("UserInfo3", user_info3.into_iter());
    tree.new_branch("HasWaveform", has_waveform.into_iter());
    tree.new_branch("AnalogProbeType0", analog_probe_type0.into_iter());
    tree.new_branch("AnalogProbeType1", analog_probe_type1.into_iter());
    tree.new_branch("AnalogProbeType2", analog_probe_type2.into_iter());
    tree.new_branch("DigitalProbeType0", digital_probe_type0.into_iter());
    tree.new_branch("DigitalProbeType1", digital_probe_type1.into_iter());
    tree.new_branch("DigitalProbeType2", digital_probe_type2.into_iter());
    tree.new_branch("DigitalProbeType3", digital_probe_type3.into_iter());
    tree.new_branch("DigitalProbeType4", digital_probe_type4.into_iter());
    tree.new_branch("DigitalProbeType5", digital_probe_type5.into_iter());
    tree.new_branch("DigitalProbeType6", digital_probe_type6.into_iter());
    tree.new_branch("DigitalProbeType7", digital_probe_type7.into_iter());
    tree.new_branch("DigitalProbeType8", digital_probe_type8.into_iter());
    tree.new_branch("DigitalProbeType9", digital_probe_type9.into_iter());
    tree.new_branch("DigitalProbeType10", digital_probe_type10.into_iter());
    tree.new_branch("DigitalProbeType11", digital_probe_type11.into_iter());
    tree.new_branch("DigitalProbeType12", digital_probe_type12.into_iter());
    tree.new_branch("DigitalProbeType13", digital_probe_type13.into_iter());
    tree.new_branch("DigitalProbeType14", digital_probe_type14.into_iter());
    tree.new_branch("DigitalProbeType15", digital_probe_type15.into_iter());

    tree.write(&mut file)
        .unwrap_or_else(|e| panic!("tree.write failed: {:?}", e));
    file.close()
        .unwrap_or_else(|e| panic!("file.close failed: {:?}", e));
    let write_elapsed = write_start.elapsed();
    let out_size = std::fs::metadata(&out_path).map(|m| m.len()).unwrap_or(0);

    println!(
        "Wrote {} events to {} in {:.2}s ({:.1} MB, {:.1} MB/s)",
        total_events,
        out_path.display(),
        write_elapsed.as_secs_f64(),
        out_size as f64 / 1_048_576.0,
        out_size as f64 / write_elapsed.as_secs_f64() / 1_048_576.0,
    );
    println!("Input  size: {:.1} MB", total_bytes_in as f64 / 1_048_576.0);
}
