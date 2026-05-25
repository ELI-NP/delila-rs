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
//! # Output is always sorted by `timestamp_ns`
//!
//! Files are reordered by `file_sequence` (from each file's header), then
//! events are merged across files with a sliding-window k-way merge: the
//! current file's events are sorted in memory; events whose timestamp is
//! safely past the next file's first event are written; the rest is held
//! in a small carry-over buffer for the next iteration. End-to-end the
//! algorithm is functionally equivalent to the legacy C++ tool.
//!
//! # Memory model
//!
//! Peak memory is ~1 file worth of events in EventData form +
//! the carry-over buffer (typically a small fraction of one file when
//! file boundaries overlap) + ROOT internal basket buffers (~1.5 MB).
//! For typical delila Recorder rotation (1 GB or 10 min per file), a
//! 99-file run can be processed in well under 2 GB resident memory.
//!
//! Column data is **not** materialized into per-branch Vecs anymore;
//! instead 49 lazy branch iterators share a single sorted EventData
//! source via `Rc<RefCell<...>>`. This is safe because oxyroot's
//! WriterTree polls branches in deterministic row-major lock-step at
//! `tree.write()` time (see `oxyroot::WriterTree::write` in
//! `rtree/tree/writer.rs`). If a future oxyroot version parallelises
//! branch writes the `Rc<RefCell>` borrow scheme would panic — this
//! constraint is documented in `BranchIter::next()`.
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
//! # Compression workflow (post-process)
//!
//! oxyroot 0.1.25 cannot write compressed ROOT files. To LZ4-compress the
//! output (~3-5x smaller, fast), pipe through ROOT's `hadd`:
//!     hadd -f404 compressed.root out.root
//! (-f404 = LZ4 level 4, ROOT's default fast compression.) ROOT must be
//! installed on the host running hadd.
//!
//! # Notes
//!
//! - The on-disk schema folds the decoder's `fine_time` into `timestamp_ns`
//!   (= coarse_ns + fine_time/1024 × time_step), so there is no separate
//!   fine-time branch.
//! - Files are read in `file_sequence` order regardless of argv order;
//!   wildcards like `data/run0001_*.delila` work as expected.
//! - Backward compatible with all `.delila` files ever recorded
//!   (FORMAT_VERSION=2, the only version that has shipped). Pre-AMax files
//!   that lack `user_info[4]` and pre-Phase-4.5 files that lack probe-type
//!   fields are deserialized via `#[serde(default)]`, populating the
//!   missing columns with zeros / 0xFF (UNKNOWN_PROBE_TYPE).

use std::cell::RefCell;
use std::collections::VecDeque;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use delila_rs::common::{EventData, Waveform, UNKNOWN_PROBE_TYPE};
use delila_rs::recorder::DataFileReader;
use oxyroot::{RootFile, WriterTree};

// ---------------------------------------------------------------------------
// FileMeta + SortedFileStream
// ---------------------------------------------------------------------------

/// Metadata extracted from a `.delila` file's header + first batch — used
/// to order files and to compute the sliding-window cutoff.
#[derive(Debug, Clone)]
struct FileMeta {
    path: PathBuf,
    file_sequence: u32,
    first_event_ns: f64,
}

/// Read just enough of a file to fill out `FileMeta` (header + first
/// non-empty batch). On error, returns `Err` so the caller can decide
/// whether to skip the file.
fn read_file_meta(path: &Path) -> Result<FileMeta, String> {
    let file = File::open(path).map_err(|e| format!("open {}: {}", path.display(), e))?;
    let reader = BufReader::new(file);
    let mut dfr =
        DataFileReader::new(reader).map_err(|e| format!("header {}: {:?}", path.display(), e))?;
    let file_sequence = dfr
        .header()
        .map(|h| h.file_sequence)
        .ok_or_else(|| format!("missing header in {}", path.display()))?;
    for batch_result in dfr.data_blocks() {
        let batch = batch_result.map_err(|e| format!("batch {}: {:?}", path.display(), e))?;
        if let Some(first) = batch.events.first() {
            return Ok(FileMeta {
                path: path.to_path_buf(),
                file_sequence,
                first_event_ns: first.timestamp_ns,
            });
        }
    }
    Err(format!("no events in {}", path.display()))
}

/// Read every event from a file into a flat Vec (no sorting).
fn read_file_events(path: &Path) -> Result<Vec<EventData>, String> {
    let file = File::open(path).map_err(|e| format!("open {}: {}", path.display(), e))?;
    let reader = BufReader::new(file);
    let mut dfr =
        DataFileReader::new(reader).map_err(|e| format!("header {}: {:?}", path.display(), e))?;
    let mut events = Vec::new();
    for batch_result in dfr.data_blocks() {
        match batch_result {
            Ok(batch) => events.extend(batch.events.into_iter()),
            Err(e) => eprintln!("  warn: decode error in {}: {:?}", path.display(), e),
        }
    }
    Ok(events)
}

/// Merge two already-sorted EventData Vecs by timestamp_ns. Stable for
/// equal timestamps (left side wins, matching the carry-over-then-current
/// invariant).
fn merge_sorted(mut left: Vec<EventData>, right: Vec<EventData>) -> Vec<EventData> {
    if left.is_empty() {
        return right;
    }
    if right.is_empty() {
        return left;
    }
    let mut out = Vec::with_capacity(left.len() + right.len());
    let mut li = 0usize;
    let mut ri = 0usize;
    let mut left_drained = left.drain(..);
    let mut right_drained = right.into_iter();
    let mut l_next = left_drained.next();
    let mut r_next = right_drained.next();
    while l_next.is_some() || r_next.is_some() {
        match (&l_next, &r_next) {
            (Some(l), Some(r)) => {
                if l.timestamp_ns <= r.timestamp_ns {
                    out.push(l_next.take().unwrap());
                    l_next = left_drained.next();
                    li += 1;
                } else {
                    out.push(r_next.take().unwrap());
                    r_next = right_drained.next();
                    ri += 1;
                }
            }
            (Some(_), None) => {
                out.push(l_next.take().unwrap());
                l_next = left_drained.next();
                li += 1;
            }
            (None, Some(_)) => {
                out.push(r_next.take().unwrap());
                r_next = right_drained.next();
                ri += 1;
            }
            (None, None) => break,
        }
    }
    debug_assert_eq!(li + ri, out.len());
    out
}

/// Stream EventData yielded in monotonic-timestamp order across multiple
/// `.delila` files using a per-file sort + sliding-window k-way merge.
struct SortedFileStream {
    /// All input files, sorted by `file_sequence`.
    files: Vec<FileMeta>,
    /// Index of the next file to load (== files.len() after exhaustion).
    next_idx: usize,
    /// Sorted events ready to yield. Refilled by `advance_to_next_file`.
    current_events: VecDeque<EventData>,
    /// First event timestamp of `files[next_idx]`, or `None` if no file
    /// remains. Events in `current_events` whose ts >= this cutoff are
    /// pushed to `carry_over` instead of yielded.
    next_first_ts: Option<f64>,
    /// Events deferred from previous iteration because their ts >= the
    /// then-next file's first ts. Always sorted.
    carry_over: Vec<EventData>,
}

impl SortedFileStream {
    fn new(paths: &[PathBuf]) -> Result<Self, String> {
        if paths.is_empty() {
            return Err("no input files".to_string());
        }
        let mut files: Vec<FileMeta> = Vec::with_capacity(paths.len());
        for p in paths {
            match read_file_meta(p) {
                Ok(meta) => files.push(meta),
                Err(e) => eprintln!("  warn: skipping {} ({})", p.display(), e),
            }
        }
        if files.is_empty() {
            return Err("no readable input files".to_string());
        }
        files.sort_by_key(|f| f.file_sequence);
        Ok(Self {
            files,
            next_idx: 0,
            current_events: VecDeque::new(),
            next_first_ts: None,
            carry_over: Vec::new(),
        })
    }

    /// Drain `current_events`, yielding sorted EventData until the next
    /// boundary cutoff is hit or the stream is exhausted.
    fn advance_to_next_file(&mut self) -> bool {
        // Append carry_over to a fresh load. On the final tail (no more
        // files), just drain carry_over.
        if self.next_idx >= self.files.len() {
            if self.carry_over.is_empty() {
                return false;
            }
            self.current_events = std::mem::take(&mut self.carry_over).into();
            self.next_first_ts = None;
            return !self.current_events.is_empty();
        }

        let path = self.files[self.next_idx].path.clone();
        let mut events = match read_file_events(&path) {
            Ok(e) => e,
            Err(e) => {
                eprintln!(
                    "  warn: failed reading {} ({}), skipping",
                    path.display(),
                    e
                );
                self.next_idx += 1;
                self.next_first_ts = self.files.get(self.next_idx).map(|f| f.first_event_ns);
                return self.advance_to_next_file();
            }
        };
        events.sort_by(|a, b| a.timestamp_ns.total_cmp(&b.timestamp_ns));

        let merged = merge_sorted(std::mem::take(&mut self.carry_over), events);
        self.current_events = merged.into();

        self.next_idx += 1;
        self.next_first_ts = self.files.get(self.next_idx).map(|f| f.first_event_ns);
        !self.current_events.is_empty() || !self.carry_over.is_empty()
    }
}

impl Iterator for SortedFileStream {
    type Item = EventData;

    fn next(&mut self) -> Option<EventData> {
        loop {
            if let Some(front) = self.current_events.front() {
                match self.next_first_ts {
                    Some(cutoff) if front.timestamp_ns >= cutoff => {
                        // Unsafe — defer until next file is merged in.
                        self.carry_over
                            .push(self.current_events.pop_front().unwrap());
                        continue;
                    }
                    _ => return self.current_events.pop_front(),
                }
            }
            if !self.advance_to_next_file() {
                return None;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Shared row source + lazy branch iterators
// ---------------------------------------------------------------------------

/// Backing state shared by all 49 branch iterators. `current_row` is
/// rewound every time branch index 0 polls — branches 1..48 read from it.
/// `events_yielded` counts how many rows have been produced (for the
/// post-write summary line).
struct SharedRowSource {
    source: SortedFileStream,
    current_row: Option<EventData>,
    events_yielded: u64,
}

/// Lazy iterator wrapping a single field of `SharedRowSource::current_row`.
/// `branch_idx == 0` is responsible for advancing the source on every
/// `next()` call; other branches just read the cached row.
///
/// Safety: oxyroot's `WriterTree::write` polls branches sequentially in
/// row-major lock-step (verified at writer.rs:115-153), so the borrows
/// nest cleanly. A future parallel writer would break this and panic on
/// the `borrow_mut()` below.
struct BranchIter<T, F>
where
    F: Fn(&EventData) -> T,
{
    shared: Rc<RefCell<SharedRowSource>>,
    branch_idx: usize,
    extract: F,
}

impl<T, F> BranchIter<T, F>
where
    F: Fn(&EventData) -> T,
{
    fn new(shared: Rc<RefCell<SharedRowSource>>, branch_idx: usize, extract: F) -> Self {
        Self {
            shared,
            branch_idx,
            extract,
        }
    }
}

impl<T, F> Iterator for BranchIter<T, F>
where
    F: Fn(&EventData) -> T,
{
    type Item = T;

    fn next(&mut self) -> Option<T> {
        let mut s = self.shared.borrow_mut();
        if self.branch_idx == 0 {
            s.current_row = s.source.next();
            if s.current_row.is_some() {
                s.events_yielded += 1;
            }
        }
        s.current_row.as_ref().map(|ev| (self.extract)(ev))
    }
}

/// Format an events/sec rate for human display, picking k/M scale.
fn format_event_rate(events: u64, secs: f64) -> String {
    if secs <= 0.0 {
        return "n/a".to_string();
    }
    let r = events as f64 / secs;
    if r >= 1.0e6 {
        format!("{:.2}M ev/s", r / 1.0e6)
    } else if r >= 1.0e3 {
        format!("{:.1}k ev/s", r / 1.0e3)
    } else {
        format!("{:.0} ev/s", r)
    }
}

// ---------------------------------------------------------------------------
// Per-event helpers
// ---------------------------------------------------------------------------

/// Run `get` against the event's waveform if present, else return `default`.
fn wf_or<T>(ev: &EventData, get: impl Fn(&Waveform) -> T, default: T) -> T {
    ev.waveform.as_ref().map(get).unwrap_or(default)
}

// ---------------------------------------------------------------------------
// 49-branch wiring
// ---------------------------------------------------------------------------

/// Register all 49 branches against a `WriterTree`, all reading from the
/// shared row source. `branch_idx` increments monotonically and is what
/// gates the source advance (see `BranchIter::next`).
fn register_branches(tree: &mut WriterTree, shared: Rc<RefCell<SharedRowSource>>) {
    let mut idx = 0usize;
    macro_rules! reg {
        ($name:literal, $ty:ty, $extract:expr) => {{
            let it: BranchIter<$ty, _> = BranchIter::new(shared.clone(), idx, $extract);
            tree.new_branch($name, it);
            idx += 1;
        }};
    }

    // Scalar event branches.
    reg!("Module", u8, |ev: &EventData| ev.module);
    reg!("Channel", u8, |ev: &EventData| ev.channel);
    reg!("TimestampNs", f64, |ev: &EventData| ev.timestamp_ns);
    reg!("Energy", u16, |ev: &EventData| ev.energy);
    reg!("EnergyShort", u16, |ev: &EventData| ev.energy_short);
    reg!("Flags", u64, |ev: &EventData| ev.flags);
    reg!("UserInfo0", u64, |ev: &EventData| ev.user_info[0]);
    reg!("UserInfo1", u64, |ev: &EventData| ev.user_info[1]);
    reg!("UserInfo2", u64, |ev: &EventData| ev.user_info[2]);
    reg!("UserInfo3", u64, |ev: &EventData| ev.user_info[3]);
    reg!("HasWaveform", u8, |ev: &EventData| {
        if ev.waveform.is_some() {
            1
        } else {
            0
        }
    });

    // Probe-type code branches (1-indexed).
    reg!("AnalogProbeType1", u8, |ev: &EventData| wf_or(
        ev,
        |w| w.analog_probe_type[0],
        UNKNOWN_PROBE_TYPE
    ));
    reg!("AnalogProbeType2", u8, |ev: &EventData| wf_or(
        ev,
        |w| w.analog_probe_type[1],
        UNKNOWN_PROBE_TYPE
    ));
    reg!("AnalogProbeType3", u8, |ev: &EventData| wf_or(
        ev,
        |w| w.analog_probe_type[2],
        UNKNOWN_PROBE_TYPE
    ));
    for i in 0..16usize {
        let name: &'static str = match i {
            0 => "DigitalProbeType1",
            1 => "DigitalProbeType2",
            2 => "DigitalProbeType3",
            3 => "DigitalProbeType4",
            4 => "DigitalProbeType5",
            5 => "DigitalProbeType6",
            6 => "DigitalProbeType7",
            7 => "DigitalProbeType8",
            8 => "DigitalProbeType9",
            9 => "DigitalProbeType10",
            10 => "DigitalProbeType11",
            11 => "DigitalProbeType12",
            12 => "DigitalProbeType13",
            13 => "DigitalProbeType14",
            14 => "DigitalProbeType15",
            15 => "DigitalProbeType16",
            _ => unreachable!(),
        };
        let it: BranchIter<u8, _> = BranchIter::new(shared.clone(), idx, move |ev: &EventData| {
            wf_or(ev, |w| w.digital_probe_type[i], UNKNOWN_PROBE_TYPE)
        });
        tree.new_branch(name, it);
        idx += 1;
    }

    // Per-event waveform vectors.
    reg!("AnalogProbe1", Vec<i16>, |ev: &EventData| wf_or(
        ev,
        |w| w.analog_probe1.clone(),
        Vec::new()
    ));
    reg!("AnalogProbe2", Vec<i16>, |ev: &EventData| wf_or(
        ev,
        |w| w.analog_probe2.clone(),
        Vec::new()
    ));
    reg!("AnalogProbe3", Vec<i16>, |ev: &EventData| wf_or(
        ev,
        |w| w.analog_probe3.clone(),
        Vec::new()
    ));
    macro_rules! reg_dp {
        ($name:literal, $field:ident) => {
            reg!($name, Vec<u8>, |ev: &EventData| wf_or(
                ev,
                |w| w.$field.clone(),
                Vec::new()
            ));
        };
    }
    reg_dp!("DigitalProbe1", digital_probe1);
    reg_dp!("DigitalProbe2", digital_probe2);
    reg_dp!("DigitalProbe3", digital_probe3);
    reg_dp!("DigitalProbe4", digital_probe4);
    reg_dp!("DigitalProbe5", digital_probe5);
    reg_dp!("DigitalProbe6", digital_probe6);
    reg_dp!("DigitalProbe7", digital_probe7);
    reg_dp!("DigitalProbe8", digital_probe8);
    reg_dp!("DigitalProbe9", digital_probe9);
    reg_dp!("DigitalProbe10", digital_probe10);
    reg_dp!("DigitalProbe11", digital_probe11);
    reg_dp!("DigitalProbe12", digital_probe12);
    reg_dp!("DigitalProbe13", digital_probe13);
    reg_dp!("DigitalProbe14", digital_probe14);
    reg_dp!("DigitalProbe15", digital_probe15);
    reg_dp!("DigitalProbe16", digital_probe16);

    // Waveform metadata.
    reg!("TimeResolution", u8, |ev: &EventData| wf_or(
        ev,
        |w| w.time_resolution,
        0
    ));
    reg!("TriggerThreshold", u16, |ev: &EventData| wf_or(
        ev,
        |w| w.trigger_threshold,
        0
    ));
    reg!("NsPerSample", f64, |ev: &EventData| wf_or(
        ev,
        |w| w.ns_per_sample,
        0.0
    ));
    reg!("AnalogProbe1IsSigned", bool, |ev: &EventData| wf_or(
        ev,
        |w| w.analog_probe1_is_signed,
        false
    ));
    reg!("AnalogProbe2IsSigned", bool, |ev: &EventData| wf_or(
        ev,
        |w| w.analog_probe2_is_signed,
        false
    ));
    reg!("AnalogProbe3IsSigned", bool, |ev: &EventData| wf_or(
        ev,
        |w| w.analog_probe3_is_signed,
        false
    ));

    debug_assert_eq!(idx, 49, "expected 49 branches, registered {}", idx);
}

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

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
        "delila2root: {} input file(s) → {}",
        inputs.len(),
        out_path.display()
    );

    let total_bytes_in: u64 = inputs
        .iter()
        .map(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0))
        .sum();

    // Build sorted stream (reads each file's header + first batch).
    let meta_start = Instant::now();
    let stream = match SortedFileStream::new(&inputs) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {}", e);
            std::process::exit(1);
        }
    };
    if stream.files.len() > 1 {
        println!(
            "Reordered {} files by file_sequence in {:.3}s",
            stream.files.len(),
            meta_start.elapsed().as_secs_f64()
        );
    }

    let shared = Rc::new(RefCell::new(SharedRowSource {
        source: stream,
        current_row: None,
        events_yielded: 0,
    }));

    // Wire branches and write — this consumes the stream lazily.
    let write_start = Instant::now();
    let mut file = RootFile::create(out_path.to_str().unwrap_or(""))
        .unwrap_or_else(|e| panic!("RootFile::create({}) failed: {:?}", out_path.display(), e));

    let mut tree = WriterTree::new(&tree_name);
    register_branches(&mut tree, shared.clone());

    tree.write(&mut file)
        .unwrap_or_else(|e| panic!("tree.write failed: {:?}", e));
    file.close()
        .unwrap_or_else(|e| panic!("file.close failed: {:?}", e));
    let write_elapsed = write_start.elapsed();

    // Pull the event count out of the shared cell before dropping. oxyroot
    // doesn't expose `tree.entries` post-write, so we count via branch 0
    // advance ticks (see `BranchIter::next`).
    let total_events = shared.borrow().events_yielded;
    drop(shared);

    let out_size = std::fs::metadata(&out_path).map(|m| m.len()).unwrap_or(0);
    let secs = write_elapsed.as_secs_f64();

    println!(
        "Wrote {} events → {} ({:.1} MB) in {:.2}s ({:.1} MB/s, {})",
        total_events,
        out_path.display(),
        out_size as f64 / 1_048_576.0,
        secs,
        out_size as f64 / secs / 1_048_576.0,
        format_event_rate(total_events, secs),
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use delila_rs::recorder::{ChecksumCalculator, FileFooter, FileHeader};
    use std::io::Write;
    use tempfile::tempdir;

    fn ev(ts_ns: f64, ch: u8) -> EventData {
        EventData::new(0, ch, 100, 0, ts_ns, 0)
    }

    /// Write a minimal `.delila` file (header + N batches + footer) with
    /// the given events and `file_sequence`. The helper hand-rolls just
    /// enough of the on-disk format to exercise our reader; production
    /// recording goes through `Recorder` which uses the same format
    /// primitives.
    fn write_delila(
        path: &Path,
        file_sequence: u32,
        events: Vec<EventData>,
    ) -> std::io::Result<()> {
        use delila_rs::common::EventDataBatch;
        let mut f = File::create(path)?;
        let mut header = FileHeader::new(1, "test".to_string(), file_sequence);
        header.is_sorted = false;
        header.write_to(&mut f).expect("header");
        let mut checksum = ChecksumCalculator::new();
        if !events.is_empty() {
            let mut batch = EventDataBatch::new(0, 0);
            for e in events {
                batch.push(e);
            }
            let bytes = batch.to_msgpack().expect("encode batch");
            let len_bytes = (bytes.len() as u32).to_le_bytes();
            f.write_all(&len_bytes)?;
            f.write_all(&bytes)?;
            checksum.update(&len_bytes);
            checksum.update(&bytes);
        }
        let mut footer = FileFooter::new();
        footer.data_checksum = checksum.finalize();
        footer.finalize();
        footer.write_to(&mut f).expect("footer");
        Ok(())
    }

    #[test]
    fn sorted_stream_single_file_yields_in_order() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("a.delila");
        // Out-of-order events in the file (Merger jitter case).
        write_delila(&p, 0, vec![ev(30.0, 1), ev(10.0, 2), ev(20.0, 3)]).unwrap();
        let stream = SortedFileStream::new(&[p]).unwrap();
        let ts: Vec<f64> = stream.map(|e| e.timestamp_ns).collect();
        assert_eq!(ts, vec![10.0, 20.0, 30.0]);
    }

    #[test]
    fn sorted_stream_two_files_no_overlap_concatenates() {
        let dir = tempdir().unwrap();
        let p1 = dir.path().join("a.delila");
        let p2 = dir.path().join("b.delila");
        write_delila(&p1, 0, vec![ev(10.0, 1), ev(20.0, 2)]).unwrap();
        write_delila(&p2, 1, vec![ev(30.0, 3), ev(40.0, 4)]).unwrap();
        let stream = SortedFileStream::new(&[p1, p2]).unwrap();
        let ts: Vec<f64> = stream.map(|e| e.timestamp_ns).collect();
        assert_eq!(ts, vec![10.0, 20.0, 30.0, 40.0]);
    }

    #[test]
    fn sorted_stream_two_files_with_overlap_uses_carry_over() {
        let dir = tempdir().unwrap();
        let p1 = dir.path().join("a.delila");
        let p2 = dir.path().join("b.delila");
        // file 0 has 10, 20, 50 — but file 1 starts at 30
        // → 50 must be carried over and yielded after file 1's events
        write_delila(&p1, 0, vec![ev(10.0, 1), ev(20.0, 2), ev(50.0, 3)]).unwrap();
        write_delila(&p2, 1, vec![ev(30.0, 4), ev(40.0, 5), ev(60.0, 6)]).unwrap();
        let stream = SortedFileStream::new(&[p1, p2]).unwrap();
        let ts: Vec<f64> = stream.map(|e| e.timestamp_ns).collect();
        assert_eq!(ts, vec![10.0, 20.0, 30.0, 40.0, 50.0, 60.0]);
    }

    #[test]
    fn sorted_stream_reorders_files_by_file_sequence() {
        let dir = tempdir().unwrap();
        let p1 = dir.path().join("late.delila"); // alphabetically later
        let p2 = dir.path().join("early.delila");
        // Pass argv in alphabetical order, but file_sequence opposite.
        write_delila(&p1, 0, vec![ev(10.0, 1), ev(20.0, 2)]).unwrap(); // seq=0
        write_delila(&p2, 1, vec![ev(30.0, 3), ev(40.0, 4)]).unwrap(); // seq=1
                                                                       // Pass argv as [p2, p1] (sequence 1, then 0). Stream should
                                                                       // reorder to [p1, p2] internally (sequence 0, then 1).
        let stream = SortedFileStream::new(&[p2, p1]).unwrap();
        let ts: Vec<f64> = stream.map(|e| e.timestamp_ns).collect();
        assert_eq!(ts, vec![10.0, 20.0, 30.0, 40.0]);
    }

    #[test]
    fn branch_iter_lockstep_returns_consistent_row() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("a.delila");
        write_delila(&p, 0, vec![ev(10.0, 1), ev(20.0, 2)]).unwrap();
        let stream = SortedFileStream::new(&[p]).unwrap();
        let shared = Rc::new(RefCell::new(SharedRowSource {
            source: stream,
            current_row: None,
            events_yielded: 0,
        }));
        // Simulate oxyroot's row-major poll: branch 0 first (advances),
        // branches 1, 2 read the cached row.
        let mut b0: BranchIter<u8, _> = BranchIter::new(shared.clone(), 0, |e| e.module);
        let mut b1: BranchIter<u8, _> = BranchIter::new(shared.clone(), 1, |e| e.channel);
        let mut b2: BranchIter<f64, _> = BranchIter::new(shared.clone(), 2, |e| e.timestamp_ns);

        // Row 0
        assert_eq!(b0.next(), Some(0));
        assert_eq!(b1.next(), Some(1));
        assert_eq!(b2.next(), Some(10.0));
        // Row 1
        assert_eq!(b0.next(), Some(0));
        assert_eq!(b1.next(), Some(2));
        assert_eq!(b2.next(), Some(20.0));
        // Exhausted
        assert_eq!(b0.next(), None);
        assert_eq!(b1.next(), None);
        assert_eq!(b2.next(), None);
        // Counter only advanced on Some (2 successful rows)
        assert_eq!(shared.borrow().events_yielded, 2);
    }

    #[test]
    fn branch_iter_all_exhaust_simultaneously() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("a.delila");
        write_delila(&p, 0, vec![ev(10.0, 1)]).unwrap();
        let stream = SortedFileStream::new(&[p]).unwrap();
        let shared = Rc::new(RefCell::new(SharedRowSource {
            source: stream,
            current_row: None,
            events_yielded: 0,
        }));
        let mut b0: BranchIter<u8, _> = BranchIter::new(shared.clone(), 0, |e| e.module);
        let mut b1: BranchIter<u8, _> = BranchIter::new(shared.clone(), 1, |e| e.channel);
        // Pull 1 row
        assert!(b0.next().is_some());
        assert!(b1.next().is_some());
        // Both exhaust on next poll
        assert_eq!(b0.next(), None);
        assert_eq!(b1.next(), None);
    }

    #[test]
    fn merge_sorted_handles_empty_inputs() {
        let a: Vec<EventData> = vec![];
        let b = vec![ev(10.0, 1), ev(20.0, 2)];
        let r = merge_sorted(a, b);
        let ts: Vec<f64> = r.iter().map(|e| e.timestamp_ns).collect();
        assert_eq!(ts, vec![10.0, 20.0]);
    }

    #[test]
    fn merge_sorted_is_stable() {
        // Equal timestamps — left side wins.
        let l = vec![ev(10.0, 1)];
        let r = vec![ev(10.0, 2)];
        let m = merge_sorted(l, r);
        assert_eq!(m[0].channel, 1, "left side should come first on tie");
        assert_eq!(m[1].channel, 2);
    }
}
