//! delila2root — Convert `.delila` files to a flat ROOT TTree.
//!
//! Replaces the legacy C++ tool at `tools/delila2root/`. Each row in the
//! output TTree is one decoded event with the full current EventData +
//! Waveform schema (PHA2/AMax `user_info[4]`, Phase 4.5 probe-type fields,
//! AMax debug FW analog_probe3 + digital_probe5..16 — none of which the C++
//! tool understood).
//!
//! Usage:
//!     cargo run --release --features root --bin delila2root -- \
//!         -o out.root data/run0001_0020_PHA2_Phys.delila [more.delila ...]
//!
//! # Output branches (49 total, 1-indexed throughout)
//!
//! Scalar event branches:
//!     Module (u8), Channel (u8), TimestampNs (f64), Energy (u16),
//!     EnergyShort (u16), Flags (u64),
//!     UserInfo0..UserInfo3 (u64),
//!     HasWaveform (u8) — 0/1 gate; when 0 the waveform branches are empty
//!     and metadata is zero,
//!     AnalogProbeType1..3 (u8 ×3), DigitalProbeType1..16 (u8 ×16).
//!
//! Per-event waveform vectors:
//!     AnalogProbe1..3 (vector<int16_t> ×3) — AnalogProbe3 is empty for
//!     every FW except AMax debug,
//!     DigitalProbe1..16 (vector<UChar_t> ×16) — DigitalProbe1..4 standard
//!     PHA2/PSD2, DigitalProbe5 AMax debug, DigitalProbe6..16 reserved
//!     (always empty today).
//!
//! Per-event waveform metadata:
//!     TimeResolution (u8), TriggerThreshold (u16), NsPerSample (f64),
//!     AnalogProbe1IsSigned, AnalogProbe2IsSigned, AnalogProbe3IsSigned (bool ×3).
//!
//! Probe-type codes are PHA2 canonical encoding (see Waveform doc-comment in
//! `src/common/mod.rs`): 0=ADCInput, 1=TimeFilter, 2=EnergyFilter, … for
//! analog; 0=Trigger, 1=TimeFilterArmed, … for digital. AMax debug FW uses
//! the 0x40+ range. `UNKNOWN_PROBE_TYPE` (=0xFF) is emitted by FW that does
//! not carry typed probe info on the wire (PSD1/PHA1/PSD2 etc.) and for any
//! event without a waveform.
//!
//! # Memory model
//!
//! All events are accumulated in memory before the single `tree.write()`
//! pass. For PHA2 with full waveforms (200-sample 2 analog + 4 digital
//! probes ≈ 1.6 kB/event), ~5M events ≈ several GB of RAM. Stream the
//! input across multiple invocations or split the source files if you hit
//! OOM. (oxyroot's WriterTree API requires `into_iter()` on Vecs, which
//! forecloses true single-pass streaming without a significant rewrite.)
//!
//! # Compression workflow (post-process)
//!
//! oxyroot 0.1.25 cannot write compressed ROOT files. To LZ4-compress the
//! output (~3-5x smaller, fast), pipe through ROOT's `hadd`:
//!     hadd -f404 compressed.root out.root
//! (-f404 = LZ4 level 4, ROOT's default fast compression.) ROOT must be
//! installed on the host running hadd. The original delila2root C++ tool
//! also required ROOT at build time, so this is no new dependency.
//!
//! # Notes
//!
//! - The on-disk schema folds the decoder's `fine_time` into `timestamp_ns`
//!   (= coarse_ns + fine_time/1024 × time_step), so there is no separate
//!   fine-time branch.
//! - Events are written in the order they appear in the input files (no
//!   cross-file time sort — that's `event_builder`'s job).
//! - Backward compatible with all `.delila` files ever recorded
//!   (FORMAT_VERSION=2, the only version that has shipped). Pre-AMax files
//!   that lack `user_info[4]` and pre-Phase-4.5 files that lack probe-type
//!   fields are deserialized via `#[serde(default)]`, populating the
//!   missing columns with zeros / 0xFF (UNKNOWN_PROBE_TYPE).

use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::time::Instant;

use delila_rs::common::{EventData, UNKNOWN_PROBE_TYPE};
use delila_rs::recorder::DataFileReader;
use oxyroot::{RootFile, WriterTree};

/// All per-branch flat columns kept side-by-side. One method (`push`)
/// appends one event's contribution to every column, so by construction
/// every column ends with the same length even when waveforms are absent.
/// Field naming is 1-indexed throughout (matches the legacy C++ tool).
#[derive(Default)]
struct Columns {
    module: Vec<u8>,
    channel: Vec<u8>,
    timestamp_ns: Vec<f64>,
    energy: Vec<u16>,
    energy_short: Vec<u16>,
    flags: Vec<u64>,
    user_info0: Vec<u64>,
    user_info1: Vec<u64>,
    user_info2: Vec<u64>,
    user_info3: Vec<u64>,
    has_waveform: Vec<u8>,
    analog_probe_type1: Vec<u8>,
    analog_probe_type2: Vec<u8>,
    analog_probe_type3: Vec<u8>,
    digital_probe_type1: Vec<u8>,
    digital_probe_type2: Vec<u8>,
    digital_probe_type3: Vec<u8>,
    digital_probe_type4: Vec<u8>,
    digital_probe_type5: Vec<u8>,
    digital_probe_type6: Vec<u8>,
    digital_probe_type7: Vec<u8>,
    digital_probe_type8: Vec<u8>,
    digital_probe_type9: Vec<u8>,
    digital_probe_type10: Vec<u8>,
    digital_probe_type11: Vec<u8>,
    digital_probe_type12: Vec<u8>,
    digital_probe_type13: Vec<u8>,
    digital_probe_type14: Vec<u8>,
    digital_probe_type15: Vec<u8>,
    digital_probe_type16: Vec<u8>,
    analog_probe1: Vec<Vec<i16>>,
    analog_probe2: Vec<Vec<i16>>,
    analog_probe3: Vec<Vec<i16>>,
    digital_probe1: Vec<Vec<u8>>,
    digital_probe2: Vec<Vec<u8>>,
    digital_probe3: Vec<Vec<u8>>,
    digital_probe4: Vec<Vec<u8>>,
    digital_probe5: Vec<Vec<u8>>,
    digital_probe6: Vec<Vec<u8>>,
    digital_probe7: Vec<Vec<u8>>,
    digital_probe8: Vec<Vec<u8>>,
    digital_probe9: Vec<Vec<u8>>,
    digital_probe10: Vec<Vec<u8>>,
    digital_probe11: Vec<Vec<u8>>,
    digital_probe12: Vec<Vec<u8>>,
    digital_probe13: Vec<Vec<u8>>,
    digital_probe14: Vec<Vec<u8>>,
    digital_probe15: Vec<Vec<u8>>,
    digital_probe16: Vec<Vec<u8>>,
    time_resolution: Vec<u8>,
    trigger_threshold: Vec<u16>,
    ns_per_sample: Vec<f64>,
    analog_probe1_is_signed: Vec<bool>,
    analog_probe2_is_signed: Vec<bool>,
    analog_probe3_is_signed: Vec<bool>,
}

impl Columns {
    fn new() -> Self {
        Self::default()
    }

    /// Number of rows accumulated so far. Used by tests to verify that
    /// every column stays the same length even when waveforms are absent.
    #[cfg(test)]
    fn len(&self) -> usize {
        self.module.len()
    }

    /// Append one event's contribution to every column. When `ev.waveform`
    /// is None we still push to every branch (empty Vec for probe data,
    /// 0/0.0/false for scalars, UNKNOWN_PROBE_TYPE for type codes) so
    /// every column stays the same length.
    fn push(&mut self, ev: &EventData) {
        self.module.push(ev.module);
        self.channel.push(ev.channel);
        self.timestamp_ns.push(ev.timestamp_ns);
        self.energy.push(ev.energy);
        self.energy_short.push(ev.energy_short);
        self.flags.push(ev.flags);
        self.user_info0.push(ev.user_info[0]);
        self.user_info1.push(ev.user_info[1]);
        self.user_info2.push(ev.user_info[2]);
        self.user_info3.push(ev.user_info[3]);
        self.has_waveform
            .push(if ev.waveform.is_some() { 1 } else { 0 });
        match ev.waveform.as_ref() {
            Some(wf) => {
                self.analog_probe_type1.push(wf.analog_probe_type[0]);
                self.analog_probe_type2.push(wf.analog_probe_type[1]);
                self.analog_probe_type3.push(wf.analog_probe_type[2]);
                self.digital_probe_type1.push(wf.digital_probe_type[0]);
                self.digital_probe_type2.push(wf.digital_probe_type[1]);
                self.digital_probe_type3.push(wf.digital_probe_type[2]);
                self.digital_probe_type4.push(wf.digital_probe_type[3]);
                self.digital_probe_type5.push(wf.digital_probe_type[4]);
                self.digital_probe_type6.push(wf.digital_probe_type[5]);
                self.digital_probe_type7.push(wf.digital_probe_type[6]);
                self.digital_probe_type8.push(wf.digital_probe_type[7]);
                self.digital_probe_type9.push(wf.digital_probe_type[8]);
                self.digital_probe_type10.push(wf.digital_probe_type[9]);
                self.digital_probe_type11.push(wf.digital_probe_type[10]);
                self.digital_probe_type12.push(wf.digital_probe_type[11]);
                self.digital_probe_type13.push(wf.digital_probe_type[12]);
                self.digital_probe_type14.push(wf.digital_probe_type[13]);
                self.digital_probe_type15.push(wf.digital_probe_type[14]);
                self.digital_probe_type16.push(wf.digital_probe_type[15]);

                self.analog_probe1.push(wf.analog_probe1.clone());
                self.analog_probe2.push(wf.analog_probe2.clone());
                self.analog_probe3.push(wf.analog_probe3.clone());
                self.digital_probe1.push(wf.digital_probe1.clone());
                self.digital_probe2.push(wf.digital_probe2.clone());
                self.digital_probe3.push(wf.digital_probe3.clone());
                self.digital_probe4.push(wf.digital_probe4.clone());
                self.digital_probe5.push(wf.digital_probe5.clone());
                self.digital_probe6.push(wf.digital_probe6.clone());
                self.digital_probe7.push(wf.digital_probe7.clone());
                self.digital_probe8.push(wf.digital_probe8.clone());
                self.digital_probe9.push(wf.digital_probe9.clone());
                self.digital_probe10.push(wf.digital_probe10.clone());
                self.digital_probe11.push(wf.digital_probe11.clone());
                self.digital_probe12.push(wf.digital_probe12.clone());
                self.digital_probe13.push(wf.digital_probe13.clone());
                self.digital_probe14.push(wf.digital_probe14.clone());
                self.digital_probe15.push(wf.digital_probe15.clone());
                self.digital_probe16.push(wf.digital_probe16.clone());

                self.time_resolution.push(wf.time_resolution);
                self.trigger_threshold.push(wf.trigger_threshold);
                self.ns_per_sample.push(wf.ns_per_sample);
                self.analog_probe1_is_signed
                    .push(wf.analog_probe1_is_signed);
                self.analog_probe2_is_signed
                    .push(wf.analog_probe2_is_signed);
                self.analog_probe3_is_signed
                    .push(wf.analog_probe3_is_signed);
            }
            None => {
                self.analog_probe_type1.push(UNKNOWN_PROBE_TYPE);
                self.analog_probe_type2.push(UNKNOWN_PROBE_TYPE);
                self.analog_probe_type3.push(UNKNOWN_PROBE_TYPE);
                self.digital_probe_type1.push(UNKNOWN_PROBE_TYPE);
                self.digital_probe_type2.push(UNKNOWN_PROBE_TYPE);
                self.digital_probe_type3.push(UNKNOWN_PROBE_TYPE);
                self.digital_probe_type4.push(UNKNOWN_PROBE_TYPE);
                self.digital_probe_type5.push(UNKNOWN_PROBE_TYPE);
                self.digital_probe_type6.push(UNKNOWN_PROBE_TYPE);
                self.digital_probe_type7.push(UNKNOWN_PROBE_TYPE);
                self.digital_probe_type8.push(UNKNOWN_PROBE_TYPE);
                self.digital_probe_type9.push(UNKNOWN_PROBE_TYPE);
                self.digital_probe_type10.push(UNKNOWN_PROBE_TYPE);
                self.digital_probe_type11.push(UNKNOWN_PROBE_TYPE);
                self.digital_probe_type12.push(UNKNOWN_PROBE_TYPE);
                self.digital_probe_type13.push(UNKNOWN_PROBE_TYPE);
                self.digital_probe_type14.push(UNKNOWN_PROBE_TYPE);
                self.digital_probe_type15.push(UNKNOWN_PROBE_TYPE);
                self.digital_probe_type16.push(UNKNOWN_PROBE_TYPE);

                self.analog_probe1.push(Vec::new());
                self.analog_probe2.push(Vec::new());
                self.analog_probe3.push(Vec::new());
                self.digital_probe1.push(Vec::new());
                self.digital_probe2.push(Vec::new());
                self.digital_probe3.push(Vec::new());
                self.digital_probe4.push(Vec::new());
                self.digital_probe5.push(Vec::new());
                self.digital_probe6.push(Vec::new());
                self.digital_probe7.push(Vec::new());
                self.digital_probe8.push(Vec::new());
                self.digital_probe9.push(Vec::new());
                self.digital_probe10.push(Vec::new());
                self.digital_probe11.push(Vec::new());
                self.digital_probe12.push(Vec::new());
                self.digital_probe13.push(Vec::new());
                self.digital_probe14.push(Vec::new());
                self.digital_probe15.push(Vec::new());
                self.digital_probe16.push(Vec::new());

                self.time_resolution.push(0);
                self.trigger_threshold.push(0);
                self.ns_per_sample.push(0.0);
                self.analog_probe1_is_signed.push(false);
                self.analog_probe2_is_signed.push(false);
                self.analog_probe3_is_signed.push(false);
            }
        }
    }

    /// Move all columns into a freshly-built WriterTree as branches.
    fn into_branches(self, tree: &mut WriterTree) {
        tree.new_branch("Module", self.module.into_iter());
        tree.new_branch("Channel", self.channel.into_iter());
        tree.new_branch("TimestampNs", self.timestamp_ns.into_iter());
        tree.new_branch("Energy", self.energy.into_iter());
        tree.new_branch("EnergyShort", self.energy_short.into_iter());
        tree.new_branch("Flags", self.flags.into_iter());
        tree.new_branch("UserInfo0", self.user_info0.into_iter());
        tree.new_branch("UserInfo1", self.user_info1.into_iter());
        tree.new_branch("UserInfo2", self.user_info2.into_iter());
        tree.new_branch("UserInfo3", self.user_info3.into_iter());
        tree.new_branch("HasWaveform", self.has_waveform.into_iter());
        tree.new_branch("AnalogProbeType1", self.analog_probe_type1.into_iter());
        tree.new_branch("AnalogProbeType2", self.analog_probe_type2.into_iter());
        tree.new_branch("AnalogProbeType3", self.analog_probe_type3.into_iter());
        tree.new_branch("DigitalProbeType1", self.digital_probe_type1.into_iter());
        tree.new_branch("DigitalProbeType2", self.digital_probe_type2.into_iter());
        tree.new_branch("DigitalProbeType3", self.digital_probe_type3.into_iter());
        tree.new_branch("DigitalProbeType4", self.digital_probe_type4.into_iter());
        tree.new_branch("DigitalProbeType5", self.digital_probe_type5.into_iter());
        tree.new_branch("DigitalProbeType6", self.digital_probe_type6.into_iter());
        tree.new_branch("DigitalProbeType7", self.digital_probe_type7.into_iter());
        tree.new_branch("DigitalProbeType8", self.digital_probe_type8.into_iter());
        tree.new_branch("DigitalProbeType9", self.digital_probe_type9.into_iter());
        tree.new_branch("DigitalProbeType10", self.digital_probe_type10.into_iter());
        tree.new_branch("DigitalProbeType11", self.digital_probe_type11.into_iter());
        tree.new_branch("DigitalProbeType12", self.digital_probe_type12.into_iter());
        tree.new_branch("DigitalProbeType13", self.digital_probe_type13.into_iter());
        tree.new_branch("DigitalProbeType14", self.digital_probe_type14.into_iter());
        tree.new_branch("DigitalProbeType15", self.digital_probe_type15.into_iter());
        tree.new_branch("DigitalProbeType16", self.digital_probe_type16.into_iter());

        tree.new_branch("AnalogProbe1", self.analog_probe1.into_iter());
        tree.new_branch("AnalogProbe2", self.analog_probe2.into_iter());
        tree.new_branch("AnalogProbe3", self.analog_probe3.into_iter());
        tree.new_branch("DigitalProbe1", self.digital_probe1.into_iter());
        tree.new_branch("DigitalProbe2", self.digital_probe2.into_iter());
        tree.new_branch("DigitalProbe3", self.digital_probe3.into_iter());
        tree.new_branch("DigitalProbe4", self.digital_probe4.into_iter());
        tree.new_branch("DigitalProbe5", self.digital_probe5.into_iter());
        tree.new_branch("DigitalProbe6", self.digital_probe6.into_iter());
        tree.new_branch("DigitalProbe7", self.digital_probe7.into_iter());
        tree.new_branch("DigitalProbe8", self.digital_probe8.into_iter());
        tree.new_branch("DigitalProbe9", self.digital_probe9.into_iter());
        tree.new_branch("DigitalProbe10", self.digital_probe10.into_iter());
        tree.new_branch("DigitalProbe11", self.digital_probe11.into_iter());
        tree.new_branch("DigitalProbe12", self.digital_probe12.into_iter());
        tree.new_branch("DigitalProbe13", self.digital_probe13.into_iter());
        tree.new_branch("DigitalProbe14", self.digital_probe14.into_iter());
        tree.new_branch("DigitalProbe15", self.digital_probe15.into_iter());
        tree.new_branch("DigitalProbe16", self.digital_probe16.into_iter());

        tree.new_branch("TimeResolution", self.time_resolution.into_iter());
        tree.new_branch("TriggerThreshold", self.trigger_threshold.into_iter());
        tree.new_branch("NsPerSample", self.ns_per_sample.into_iter());
        tree.new_branch(
            "AnalogProbe1IsSigned",
            self.analog_probe1_is_signed.into_iter(),
        );
        tree.new_branch(
            "AnalogProbe2IsSigned",
            self.analog_probe2_is_signed.into_iter(),
        );
        tree.new_branch(
            "AnalogProbe3IsSigned",
            self.analog_probe3_is_signed.into_iter(),
        );
    }
}

fn print_usage_and_exit() -> ! {
    eprintln!(
        "Usage: delila2root -o <output.root> [--tree <name>] <file1.delila> [file2.delila ...]"
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

    // All accumulator columns — see Columns struct definition above.
    let mut cols = Columns::new();

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
                        cols.push(ev);
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
    cols.into_branches(&mut tree);

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
    println!();
    println!("Note: this ROOT file is uncompressed (oxyroot 0.1.25 writer limitation).");
    println!("To LZ4-compress (~3-5x smaller, fast):");
    println!(
        "    hadd -f404 {}.lz4.root {}",
        out_path.display(),
        out_path.display()
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use delila_rs::common::Waveform;

    /// Build a Waveform that exercises every newly-added field: 3 analog
    /// probes populated, digital probes 1..5 populated (PHA2 standard 1..4
    /// + AMax-debug slot 5), digital probes 6..16 left empty (reserved
    ///   slots — they should still appear as branches in the Columns row).
    fn sample_waveform() -> Waveform {
        Waveform {
            analog_probe1: vec![100, 101, 102, 103],
            analog_probe2: vec![200, 201, 202, 203],
            analog_probe3: vec![-1, -2, -3, -4],
            digital_probe1: vec![1, 0, 1, 0],
            digital_probe2: vec![0, 1, 0, 1],
            digital_probe3: vec![1, 1, 0, 0],
            digital_probe4: vec![0, 0, 1, 1],
            digital_probe5: vec![1, 1, 1, 1],
            time_resolution: 2,
            trigger_threshold: 4096,
            ns_per_sample: 8.0,
            analog_probe1_is_signed: false,
            analog_probe2_is_signed: true,
            analog_probe3_is_signed: true,
            analog_probe_type: [0, 1, 2], // ADCInput, TimeFilter, EnergyFilter
            digital_probe_type: [
                0,
                1,
                2,
                3,
                4, // PHA2 standard 5
                UNKNOWN_PROBE_TYPE,
                UNKNOWN_PROBE_TYPE,
                UNKNOWN_PROBE_TYPE,
                UNKNOWN_PROBE_TYPE,
                UNKNOWN_PROBE_TYPE,
                UNKNOWN_PROBE_TYPE,
                UNKNOWN_PROBE_TYPE,
                UNKNOWN_PROBE_TYPE,
                UNKNOWN_PROBE_TYPE,
                UNKNOWN_PROBE_TYPE,
                UNKNOWN_PROBE_TYPE,
            ],
            ..Waveform::default()
        }
    }

    #[test]
    fn push_event_with_waveform_populates_every_column() {
        let mut cols = Columns::new();
        let ev = EventData::with_waveform(7, 11, 1234, 567, 1_500.0, 0xAA, sample_waveform());
        cols.push(&ev);

        assert_eq!(cols.len(), 1);
        assert_eq!(cols.module, vec![7]);
        assert_eq!(cols.channel, vec![11]);
        assert_eq!(cols.energy, vec![1234]);
        assert_eq!(cols.energy_short, vec![567]);
        assert_eq!(cols.timestamp_ns, vec![1_500.0]);
        assert_eq!(cols.flags, vec![0xAA]);
        assert_eq!(cols.has_waveform, vec![1]);

        // Probe-type 1-indexed branches carry the canonical PHA2 codes.
        assert_eq!(cols.analog_probe_type1, vec![0]); // ADCInput
        assert_eq!(cols.analog_probe_type2, vec![1]); // TimeFilter
        assert_eq!(cols.analog_probe_type3, vec![2]); // EnergyFilter
        assert_eq!(cols.digital_probe_type1, vec![0]); // Trigger
        assert_eq!(cols.digital_probe_type5, vec![4]); // 5th slot
        assert_eq!(cols.digital_probe_type16, vec![UNKNOWN_PROBE_TYPE]);

        // Waveform vectors come through in order.
        assert_eq!(cols.analog_probe1, vec![vec![100, 101, 102, 103]]);
        assert_eq!(cols.analog_probe3, vec![vec![-1, -2, -3, -4]]);
        assert_eq!(cols.digital_probe5, vec![vec![1u8, 1, 1, 1]]);
        // Reserved slots 6..16 stay empty (still pushed for column alignment).
        assert_eq!(cols.digital_probe6, vec![Vec::<u8>::new()]);
        assert_eq!(cols.digital_probe16, vec![Vec::<u8>::new()]);

        // Metadata + IsSigned bools.
        assert_eq!(cols.time_resolution, vec![2]);
        assert_eq!(cols.trigger_threshold, vec![4096]);
        assert_eq!(cols.ns_per_sample, vec![8.0]);
        assert_eq!(cols.analog_probe1_is_signed, vec![false]);
        assert_eq!(cols.analog_probe2_is_signed, vec![true]);
        assert_eq!(cols.analog_probe3_is_signed, vec![true]);
    }

    #[test]
    fn push_event_without_waveform_pads_with_defaults() {
        let mut cols = Columns::new();
        let ev = EventData::new(3, 5, 999, 0, 42.0, 0);
        cols.push(&ev);

        assert_eq!(cols.has_waveform, vec![0]);
        assert_eq!(cols.analog_probe_type1, vec![UNKNOWN_PROBE_TYPE]);
        assert_eq!(cols.analog_probe_type3, vec![UNKNOWN_PROBE_TYPE]);
        assert_eq!(cols.digital_probe_type16, vec![UNKNOWN_PROBE_TYPE]);
        assert_eq!(cols.analog_probe1, vec![Vec::<i16>::new()]);
        assert_eq!(cols.analog_probe3, vec![Vec::<i16>::new()]);
        assert_eq!(cols.digital_probe1, vec![Vec::<u8>::new()]);
        assert_eq!(cols.digital_probe16, vec![Vec::<u8>::new()]);
        assert_eq!(cols.time_resolution, vec![0]);
        assert_eq!(cols.trigger_threshold, vec![0]);
        assert_eq!(cols.ns_per_sample, vec![0.0]);
        assert_eq!(cols.analog_probe1_is_signed, vec![false]);
    }

    #[test]
    fn mixed_events_keep_every_column_aligned() {
        // Mix waveform-bearing and waveform-less events; every column
        // must end up with len() == 3 (one row per event).
        let mut cols = Columns::new();
        cols.push(&EventData::with_waveform(
            0,
            0,
            10,
            0,
            0.0,
            0,
            sample_waveform(),
        ));
        cols.push(&EventData::new(0, 0, 20, 0, 1.0, 0));
        cols.push(&EventData::with_waveform(
            0,
            0,
            30,
            0,
            2.0,
            0,
            sample_waveform(),
        ));

        // Spot-check a sample of columns from across the schema.
        assert_eq!(cols.len(), 3);
        assert_eq!(cols.energy.len(), 3);
        assert_eq!(cols.has_waveform, vec![1, 0, 1]);
        assert_eq!(cols.analog_probe1.len(), 3);
        assert_eq!(cols.digital_probe16.len(), 3);
        assert_eq!(cols.analog_probe1_is_signed.len(), 3);
        assert_eq!(cols.digital_probe_type1, vec![0, UNKNOWN_PROBE_TYPE, 0]);
    }

    #[test]
    fn pre_amax_eventdata_via_serde_default_round_trips() {
        // Older `.delila` files lack `user_info[4]`. serde's #[serde(default)]
        // on the field means the Rust reader fills [0;4] for those rows; this
        // test pins that contract by simulating a freshly-deserialized event
        // whose user_info defaults ran. We don't actually re-encode the wire
        // here — just confirm that an EventData built via the public ctor
        // (which mimics the deserialized state with user_info=[0;4]) is
        // pushed without losing other fields.
        let mut cols = Columns::new();
        let ev = EventData::new(2, 3, 555, 0, 100.0, 0xFF);
        cols.push(&ev);
        assert_eq!(cols.user_info0, vec![0]);
        assert_eq!(cols.user_info1, vec![0]);
        assert_eq!(cols.user_info2, vec![0]);
        assert_eq!(cols.user_info3, vec![0]);
        assert_eq!(cols.flags, vec![0xFF]);
    }
}
