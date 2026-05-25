//! oxyroot performance benchmark
//!
//! Tests write throughput for ROOT file output in different scenarios:
//! 1. Flat hits (write_hits_to_root) — simple 5-branch TTree
//! 2. Built events (write_events_to_root) — 11-branch TTree with Vec branches
//! 3. File-per-batch pattern — the Online EB approach (many small files)
//! 4. Single file via direct iterator — baseline comparison
//!
//! Run with: cargo run --release --features root --bin oxyroot_bench

use delila_rs::event_builder::{
    read_hits_from_root, write_events_to_root, write_hits_to_root, BuiltEvent, Hit,
};
use oxyroot::{RootFile, WriterTree};
use std::path::Path;
use std::time::Instant;

/// Generate realistic test hits
fn generate_hits(n: usize) -> Vec<Hit> {
    let mut hits = Vec::with_capacity(n);
    let mut ts = 0.0_f64;
    for i in 0..n {
        let module = (i % 6) as u8; // 6 modules
        let channel = (i % 16) as u8; // 16 channels
        let energy = (1000 + (i % 4000)) as u16;
        let energy_short = (500 + (i % 2000)) as u16;
        ts += 200.0; // ~5 MHz overall rate → 200 ns interval
        hits.push(Hit::new(module, channel, energy, energy_short, ts));
    }
    hits
}

/// Generate realistic built events (avg multiplicity ~3)
fn generate_events(n: usize) -> Vec<BuiltEvent> {
    let mut events = Vec::with_capacity(n);
    let mut ts = 0.0_f64;
    for i in 0..n {
        let trigger = Hit::new(
            (i % 6) as u8,
            (i % 16) as u8,
            (1000 + (i % 4000)) as u16,
            (500 + (i % 2000)) as u16,
            ts,
        );
        let mut event = BuiltEvent::new(i as u64, &trigger);

        // Add 2 coincident hits (total multiplicity = 3)
        for j in 1..=2 {
            let hit = Hit::new(
                ((i + j) % 6) as u8,
                ((i + j) % 16) as u8,
                (800 + (i % 3000)) as u16,
                (400 + (i % 1500)) as u16,
                ts + (j as f64) * 30.0,
            );
            event.add_hit(&hit);
        }
        ts += 600.0; // ~1.7 MHz event rate
        events.push(event);
    }
    events
}

/// Bench 1: write_hits_to_root (flat TTree, 5 scalar branches)
fn bench_hits(dir: &Path, n_hits: usize) {
    let hits = generate_hits(n_hits);
    let path = dir.join("bench_hits.root");

    let start = Instant::now();
    write_hits_to_root(&path, "hits", &hits).unwrap();
    let elapsed = start.elapsed();

    let file_size = std::fs::metadata(&path).unwrap().len();
    let rate = n_hits as f64 / elapsed.as_secs_f64();
    let mb_per_s = file_size as f64 / elapsed.as_secs_f64() / 1_048_576.0;

    println!("=== Bench 1: write_hits_to_root (flat) ===");
    println!("  Hits:       {n_hits:>12}");
    println!("  Time:       {:>12.3} s", elapsed.as_secs_f64());
    println!("  Rate:       {:>12.1} hits/s", rate);
    println!("  Rate:       {:>12.2} M hits/s", rate / 1e6);
    println!("  File size:  {:>12.1} MB", file_size as f64 / 1_048_576.0);
    println!("  Throughput: {:>12.1} MB/s", mb_per_s);
    println!();
}

/// Bench 2: write_events_to_root (event TTree, 11 branches with Vec)
fn bench_events(dir: &Path, n_events: usize) {
    let events = generate_events(n_events);
    let path = dir.join("bench_events.root");

    let start = Instant::now();
    write_events_to_root(&path, "events", &events, &[]).unwrap();
    let elapsed = start.elapsed();

    let file_size = std::fs::metadata(&path).unwrap().len();
    let rate = n_events as f64 / elapsed.as_secs_f64();
    let mb_per_s = file_size as f64 / elapsed.as_secs_f64() / 1_048_576.0;

    println!("=== Bench 2: write_events_to_root (event format) ===");
    println!("  Events:     {n_events:>12}  (avg multiplicity=3)");
    println!("  Time:       {:>12.3} s", elapsed.as_secs_f64());
    println!("  Rate:       {:>12.1} events/s", rate);
    println!("  Rate:       {:>12.2} M events/s", rate / 1e6);
    println!("  File size:  {:>12.1} MB", file_size as f64 / 1_048_576.0);
    println!("  Throughput: {:>12.1} MB/s", mb_per_s);
    println!();
}

/// Bench 3: File-per-batch (Online EB pattern)
fn bench_file_per_batch(dir: &Path, total_events: usize, batch_size: usize) {
    let events = generate_events(total_events);

    let start = Instant::now();
    let mut file_count = 0_usize;
    let mut total_bytes = 0_u64;

    for chunk in events.chunks(batch_size) {
        let path = dir.join(format!("bench_batch_{file_count:04}.root"));
        write_events_to_root(&path, "events", chunk, &[]).unwrap();
        total_bytes += std::fs::metadata(&path).unwrap().len();
        file_count += 1;
    }
    let elapsed = start.elapsed();

    let rate = total_events as f64 / elapsed.as_secs_f64();
    let mb_per_s = total_bytes as f64 / elapsed.as_secs_f64() / 1_048_576.0;

    println!("=== Bench 3: File-per-batch (Online EB pattern) ===");
    println!("  Events:     {total_events:>12}  (batch={batch_size})");
    println!("  Files:      {file_count:>12}");
    println!("  Time:       {:>12.3} s", elapsed.as_secs_f64());
    println!("  Rate:       {:>12.1} events/s", rate);
    println!("  Rate:       {:>12.2} M events/s", rate / 1e6);
    println!(
        "  Total size: {:>12.1} MB",
        total_bytes as f64 / 1_048_576.0
    );
    println!("  Throughput: {:>12.1} MB/s", mb_per_s);
    println!();
}

/// Bench 4: Single file via direct oxyroot API — baseline comparison
fn bench_single_file_direct(dir: &Path, n_events: usize) {
    let path = dir.join("bench_direct.root");
    let events = generate_events(n_events);

    // Pre-flatten event data (same work as write_events_to_root)
    let mut all_event_ids = Vec::with_capacity(n_events);
    let mut all_trigger_times = Vec::with_capacity(n_events);
    let mut all_trigger_mods = Vec::with_capacity(n_events);
    let mut all_trigger_chs = Vec::with_capacity(n_events);
    let mut all_multiplicities = Vec::with_capacity(n_events);
    let mut all_mods: Vec<Vec<u8>> = Vec::with_capacity(n_events);
    let mut all_chs: Vec<Vec<u8>> = Vec::with_capacity(n_events);
    let mut all_energies: Vec<Vec<u16>> = Vec::with_capacity(n_events);
    let mut all_energy_shorts: Vec<Vec<u16>> = Vec::with_capacity(n_events);
    let mut all_rel_times: Vec<Vec<f64>> = Vec::with_capacity(n_events);
    let mut all_with_acs: Vec<Vec<u8>> = Vec::with_capacity(n_events);

    for e in &events {
        all_event_ids.push(e.event_id);
        all_trigger_times.push(e.trigger_time);
        all_trigger_mods.push(e.trigger_module);
        all_trigger_chs.push(e.trigger_channel);
        all_multiplicities.push(e.multiplicity() as u32);
        all_mods.push(e.hits.iter().map(|h| h.module).collect());
        all_chs.push(e.hits.iter().map(|h| h.channel).collect());
        all_energies.push(e.hits.iter().map(|h| h.energy).collect());
        all_energy_shorts.push(e.hits.iter().map(|h| h.energy_short).collect());
        all_rel_times.push(e.hits.iter().map(|h| h.relative_time).collect());
        all_with_acs.push(
            e.hits
                .iter()
                .map(|h| if h.with_ac { 1u8 } else { 0u8 })
                .collect(),
        );
    }

    let start = Instant::now();

    let mut file = RootFile::create(path.to_str().unwrap()).unwrap();
    let mut tree = WriterTree::new("events");

    tree.new_branch("EventID", all_event_ids.into_iter());
    tree.new_branch("TriggerTime", all_trigger_times.into_iter());
    tree.new_branch("TriggerMod", all_trigger_mods.into_iter());
    tree.new_branch("TriggerCh", all_trigger_chs.into_iter());
    tree.new_branch("Multiplicity", all_multiplicities.into_iter());
    tree.new_branch("Mod", all_mods.into_iter());
    tree.new_branch("Ch", all_chs.into_iter());
    tree.new_branch("Energy", all_energies.into_iter());
    tree.new_branch("EnergyShort", all_energy_shorts.into_iter());
    tree.new_branch("RelTime", all_rel_times.into_iter());
    tree.new_branch("WithAC", all_with_acs.into_iter());

    tree.write(&mut file).unwrap();
    file.close().unwrap();

    let elapsed = start.elapsed();

    let file_size = std::fs::metadata(&path).unwrap().len();
    let rate = n_events as f64 / elapsed.as_secs_f64();
    let mb_per_s = file_size as f64 / elapsed.as_secs_f64() / 1_048_576.0;

    println!("=== Bench 4: Single file (direct oxyroot API) ===");
    println!("  Events:     {n_events:>12}  (single file)");
    println!("  Time:       {:>12.3} s", elapsed.as_secs_f64());
    println!("  Rate:       {:>12.1} events/s", rate);
    println!("  Rate:       {:>12.2} M events/s", rate / 1e6);
    println!("  File size:  {:>12.1} MB", file_size as f64 / 1_048_576.0);
    println!("  Throughput: {:>12.1} MB/s", mb_per_s);
    println!();
}

/// Bench 5: Hit-per-row flat format (event build results as scalar branches)
///
/// Instead of Vec branches per event, we write one row per hit with EventID
/// to group them. This avoids variable-length serialization overhead.
fn bench_hit_per_row(dir: &Path, n_events: usize) {
    let path = dir.join("bench_hit_per_row.root");
    let events = generate_events(n_events);

    // Flatten to hit-per-row: each hit becomes one TTree entry
    let total_hits: usize = events.iter().map(|e| e.hits.len()).sum();
    let mut event_ids = Vec::with_capacity(total_hits);
    let mut trigger_times = Vec::with_capacity(total_hits);
    let mut mods = Vec::with_capacity(total_hits);
    let mut chs = Vec::with_capacity(total_hits);
    let mut energies = Vec::with_capacity(total_hits);
    let mut energy_shorts = Vec::with_capacity(total_hits);
    let mut rel_times = Vec::with_capacity(total_hits);
    let mut with_acs = Vec::with_capacity(total_hits);
    let mut multiplicities = Vec::with_capacity(total_hits);

    for e in &events {
        let mult = e.multiplicity() as u32;
        for h in &e.hits {
            event_ids.push(e.event_id);
            trigger_times.push(e.trigger_time);
            mods.push(h.module);
            chs.push(h.channel);
            energies.push(h.energy);
            energy_shorts.push(h.energy_short);
            rel_times.push(h.relative_time);
            with_acs.push(if h.with_ac { 1u8 } else { 0u8 });
            multiplicities.push(mult);
        }
    }

    let start = Instant::now();

    let mut file = RootFile::create(path.to_str().unwrap()).unwrap();
    let mut tree = WriterTree::new("events");

    tree.new_branch("EventID", event_ids.into_iter());
    tree.new_branch("TriggerTime", trigger_times.into_iter());
    tree.new_branch("Multiplicity", multiplicities.into_iter());
    tree.new_branch("Mod", mods.into_iter());
    tree.new_branch("Ch", chs.into_iter());
    tree.new_branch("Energy", energies.into_iter());
    tree.new_branch("EnergyShort", energy_shorts.into_iter());
    tree.new_branch("RelTime", rel_times.into_iter());
    tree.new_branch("WithAC", with_acs.into_iter());

    tree.write(&mut file).unwrap();
    file.close().unwrap();

    let elapsed = start.elapsed();

    let file_size = std::fs::metadata(&path).unwrap().len();
    let event_rate = n_events as f64 / elapsed.as_secs_f64();
    let hit_rate = total_hits as f64 / elapsed.as_secs_f64();
    let mb_per_s = file_size as f64 / elapsed.as_secs_f64() / 1_048_576.0;

    println!("=== Bench 5: Hit-per-row flat format ===");
    println!("  Events:     {n_events:>12}  (total hits={total_hits})");
    println!("  Time:       {:>12.3} s", elapsed.as_secs_f64());
    println!("  Rate:       {:>12.2} M events/s", event_rate / 1e6);
    println!("  Rate:       {:>12.2} M hits/s", hit_rate / 1e6);
    println!("  File size:  {:>12.1} MB", file_size as f64 / 1_048_576.0);
    println!("  Throughput: {:>12.1} MB/s", mb_per_s);
    println!();
}

/// Bench 6: Hit-per-row file-per-batch (practical Online EB pattern)
fn bench_hit_per_row_batched(dir: &Path, total_events: usize, batch_size: usize) {
    let events = generate_events(total_events);

    let start = Instant::now();
    let mut file_count = 0_usize;
    let mut total_bytes = 0_u64;
    let mut total_hits = 0_usize;

    for event_chunk in events.chunks(batch_size) {
        let path = dir.join(format!("hit_row_batch_{file_count:04}.root"));

        // Flatten chunk to hit-per-row
        let chunk_hits: usize = event_chunk.iter().map(|e| e.hits.len()).sum();
        total_hits += chunk_hits;
        let mut event_ids = Vec::with_capacity(chunk_hits);
        let mut trigger_times = Vec::with_capacity(chunk_hits);
        let mut mods = Vec::with_capacity(chunk_hits);
        let mut chs = Vec::with_capacity(chunk_hits);
        let mut energies = Vec::with_capacity(chunk_hits);
        let mut energy_shorts = Vec::with_capacity(chunk_hits);
        let mut rel_times = Vec::with_capacity(chunk_hits);
        let mut with_acs = Vec::with_capacity(chunk_hits);
        let mut multiplicities = Vec::with_capacity(chunk_hits);

        for e in event_chunk {
            let mult = e.multiplicity() as u32;
            for h in &e.hits {
                event_ids.push(e.event_id);
                trigger_times.push(e.trigger_time);
                mods.push(h.module);
                chs.push(h.channel);
                energies.push(h.energy);
                energy_shorts.push(h.energy_short);
                rel_times.push(h.relative_time);
                with_acs.push(if h.with_ac { 1u8 } else { 0u8 });
                multiplicities.push(mult);
            }
        }

        let mut file = RootFile::create(path.to_str().unwrap()).unwrap();
        let mut tree = WriterTree::new("events");

        tree.new_branch("EventID", event_ids.into_iter());
        tree.new_branch("TriggerTime", trigger_times.into_iter());
        tree.new_branch("Multiplicity", multiplicities.into_iter());
        tree.new_branch("Mod", mods.into_iter());
        tree.new_branch("Ch", chs.into_iter());
        tree.new_branch("Energy", energies.into_iter());
        tree.new_branch("EnergyShort", energy_shorts.into_iter());
        tree.new_branch("RelTime", rel_times.into_iter());
        tree.new_branch("WithAC", with_acs.into_iter());

        tree.write(&mut file).unwrap();
        file.close().unwrap();

        total_bytes += std::fs::metadata(&path).unwrap().len();
        file_count += 1;
    }
    let elapsed = start.elapsed();

    let event_rate = total_events as f64 / elapsed.as_secs_f64();
    let hit_rate = total_hits as f64 / elapsed.as_secs_f64();
    let mb_per_s = total_bytes as f64 / elapsed.as_secs_f64() / 1_048_576.0;

    println!("=== Bench 6: Hit-per-row file-per-batch ===");
    println!("  Events:     {total_events:>12}  (batch={batch_size}, hits={total_hits})");
    println!("  Files:      {file_count:>12}");
    println!("  Time:       {:>12.3} s", elapsed.as_secs_f64());
    println!("  Rate:       {:>12.2} M events/s", event_rate / 1e6);
    println!("  Rate:       {:>12.2} M hits/s", hit_rate / 1e6);
    println!(
        "  Total size: {:>12.1} MB",
        total_bytes as f64 / 1_048_576.0
    );
    println!("  Throughput: {:>12.1} MB/s", mb_per_s);
    println!();
}

/// Bench 7: File-per-batch with varying batch sizes
fn bench_batch_sweep(dir: &Path, total_events: usize) {
    println!("=== Bench 7: Batch size sweep ===");
    println!(
        "  {:>10} | {:>10} | {:>10} | {:>10} | {:>10}",
        "Batch", "Files", "Time (s)", "M events/s", "MB/s"
    );
    println!(
        "  {:-<10}-+-{:-<10}-+-{:-<10}-+-{:-<10}-+-{:-<10}",
        "", "", "", "", ""
    );

    let events = generate_events(total_events);

    for &batch_size in &[10_000, 50_000, 100_000, 500_000, 1_000_000] {
        if batch_size > total_events {
            continue;
        }
        let start = Instant::now();
        let mut file_count = 0_usize;
        let mut total_bytes = 0_u64;

        for chunk in events.chunks(batch_size) {
            let path = dir.join(format!("sweep_{batch_size}_{file_count:04}.root"));
            write_events_to_root(&path, "events", chunk, &[]).unwrap();
            total_bytes += std::fs::metadata(&path).unwrap().len();
            file_count += 1;
        }
        let elapsed = start.elapsed();

        let rate = total_events as f64 / elapsed.as_secs_f64();
        let mb_per_s = total_bytes as f64 / elapsed.as_secs_f64() / 1_048_576.0;

        println!(
            "  {:>10} | {:>10} | {:>10.3} | {:>10.2} | {:>10.1}",
            batch_size,
            file_count,
            elapsed.as_secs_f64(),
            rate / 1e6,
            mb_per_s
        );
    }
    println!();
}

fn main() {
    println!("oxyroot performance benchmark");
    println!("=============================\n");

    // Use a temp directory under /tmp
    let dir = std::env::temp_dir().join("oxyroot_bench");
    if dir.exists() {
        std::fs::remove_dir_all(&dir).unwrap();
    }
    std::fs::create_dir_all(&dir).unwrap();
    println!("Output directory: {}\n", dir.display());

    // Bench 1: Flat hits — 10M hits
    bench_hits(&dir, 10_000_000);

    // Bench 2: Built events — 1M events (3M hits)
    bench_events(&dir, 1_000_000);

    // Bench 3: File-per-batch — 1M events, 100k/file
    let batch_dir = dir.join("batch");
    std::fs::create_dir_all(&batch_dir).unwrap();
    bench_file_per_batch(&batch_dir, 1_000_000, 100_000);

    // Bench 4: Single file via direct oxyroot API — 1M events
    bench_single_file_direct(&dir, 1_000_000);

    // Bench 5: Hit-per-row flat — 1M events (3M rows)
    bench_hit_per_row(&dir, 1_000_000);

    // Bench 6: Hit-per-row file-per-batch — 1M events, 100k/file
    let hit_row_dir = dir.join("hit_row_batch");
    std::fs::create_dir_all(&hit_row_dir).unwrap();
    bench_hit_per_row_batched(&hit_row_dir, 1_000_000, 100_000);

    // Bench 7: Batch size sweep — 2M events
    let sweep_dir = dir.join("sweep");
    std::fs::create_dir_all(&sweep_dir).unwrap();
    bench_batch_sweep(&sweep_dir, 2_000_000);

    // Bonus: verify ROOT readback
    println!("=== Verification ===");
    let verify_path = dir.join("verify_hits.root");
    let hits = generate_hits(1000);
    write_hits_to_root(&verify_path, "hits", &hits).unwrap();
    let read_back = read_hits_from_root(&verify_path, "hits").unwrap();
    println!(
        "  Write 1000 hits → read back {} hits: {}",
        read_back.len(),
        if read_back.len() == 1000 {
            "OK"
        } else {
            "FAIL"
        }
    );
    // Check first and last timestamp
    println!(
        "  First timestamp: expected={:.1}, got={:.1}",
        200.0, read_back[0].timestamp_ns
    );
    println!(
        "  Last timestamp:  expected={:.1}, got={:.1}",
        200.0 * 1000.0,
        read_back[999].timestamp_ns
    );
    println!();

    // Cleanup
    std::fs::remove_dir_all(&dir).unwrap();
    println!("Done. Temp files cleaned up.");
}
