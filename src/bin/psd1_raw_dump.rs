//! PSD1 RAW Binary Dump - Verify data structure byte by byte

use delila_rs::reader::CaenHandle;
use std::thread;
use std::time::Duration;

const URL: &str = "dig1://caen.internal/usb?link_num=0";

fn main() {
    println!("=== PSD1 RAW Binary Dump ===\n");

    let handle = CaenHandle::open(URL).expect("Failed to connect");
    println!("[OK] Connected");

    // Check timebomb
    if let Ok(tb) = handle.get_value("/par/timebombdowncounter") {
        let secs: u32 = tb.parse().unwrap_or(0);
        if secs == 0 {
            println!("[!!!] TIMEBOMB EXPIRED!");
            return;
        }
        println!("[OK] Timebomb: {}:{:02}", secs / 60, secs % 60);
    }

    // Reset and configure
    let _ = handle.send_command("/cmd/reset");
    thread::sleep(Duration::from_millis(100));

    // Minimal settings for waveform test
    let ch = 4;
    let reclen_ns = 128; // Small record length for easier analysis: 128ns = 64 samples

    set(&handle, "/par/reclen", &reclen_ns.to_string());
    set(&handle, "/par/waveforms", "TRUE");
    set(&handle, "/par/extras", "TRUE");
    set(&handle, "/vtrace/0/par/vtrace_probe", "VPROBE_INPUT");

    set(&handle, &format!("/ch/{}/par/ch_enabled", ch), "True");
    set(
        &handle,
        &format!("/ch/{}/par/ch_polarity", ch),
        "POLARITY_NEGATIVE",
    );
    set(&handle, &format!("/ch/{}/par/ch_dcoffset", ch), "50");
    set(&handle, &format!("/ch/{}/par/ch_pretrg", ch), "64"); // 64ns = 32 samples
    set(&handle, &format!("/ch/{}/par/ch_threshold", ch), "250");
    set(
        &handle,
        &format!("/ch/{}/par/ch_discr_mode", ch),
        "DISCR_MODE_LED",
    );
    set(
        &handle,
        &format!("/ch/{}/par/ch_self_trg_enable", ch),
        "TRUE",
    );

    // Disable other channels
    for other_ch in 0..8 {
        if other_ch != ch {
            let _ = handle.set_value(&format!("/ch/{}/par/ch_enabled", other_ch), "False");
        }
    }

    // Show expected values
    println!("\n--- Expected Values ---");
    let expected_samples = reclen_ns / 2; // 2ns per sample for DT5730B
    let num_samples_wave = expected_samples / 8;
    let waveform_words = num_samples_wave * 4; // CAEN spec
    println!("  Record Length:     {} ns", reclen_ns);
    println!(
        "  Expected Samples:  {} (reclen_ns / 2ns)",
        expected_samples
    );
    println!("  num_samples_wave:  {} (samples / 8)", num_samples_wave);
    println!(
        "  Waveform Words:    {} (num_samples_wave * 4)",
        waveform_words
    );
    println!("  Waveform Bytes:    {} (words * 4)", waveform_words * 4);

    // Acquire data
    println!("\n--- Data Acquisition ---");
    let endpoint = handle
        .configure_endpoint(false)
        .expect("Failed to configure endpoint");
    let _ = handle.send_command("/cmd/cleardata");
    handle
        .send_command("/cmd/armacquisition")
        .expect("Arm failed");

    println!("Acquiring for 500ms...");
    thread::sleep(Duration::from_millis(500));

    let mut buffer = vec![0u8; 64 * 1024 * 1024];
    match endpoint.read_data(1000, &mut buffer) {
        Ok(Some(raw_data)) => {
            println!("Read {} bytes, {} events", raw_data.size, raw_data.n_events);
            analyze_raw_data(&raw_data.data, raw_data.size);
        }
        Ok(None) => println!("Timeout - no data"),
        Err(e) => println!("Read error: {}", e),
    }

    let _ = handle.send_command("/cmd/disarmacquisition");
    println!("\n=== Done ===");
}

fn set(handle: &CaenHandle, path: &str, value: &str) {
    if let Err(e) = handle.set_value(path, value) {
        println!("  [ERR] {} = {}: {}", path, value, e);
    }
}

fn analyze_raw_data(data: &[u8], size: usize) {
    println!("\n========== RAW DATA ANALYSIS ==========");
    println!("Total bytes: {}", size);

    if size < 16 {
        println!("Not enough data for board header");
        return;
    }

    let mut offset = 0;

    // Board Aggregate Header (4 words = 16 bytes)
    println!("\n--- Board Aggregate Header ---");
    let w0 = read_u32(data, offset);
    let w1 = read_u32(data, offset + 4);
    let w2 = read_u32(data, offset + 8);
    let w3 = read_u32(data, offset + 12);

    let board_size = w0 & 0x0FFF_FFFF;
    let magic = (w0 >> 28) & 0xF;
    let ch_mask = w1 & 0xFF;
    let board_fail = (w1 >> 26) & 1;
    let board_id = (w1 >> 27) & 0x1F;
    let aggr_counter = w2 & 0x007F_FFFF;

    println!(
        "  [0] 0x{:08X} - size={} words, magic=0x{:X} (expect 0xA)",
        w0, board_size, magic
    );
    println!(
        "  [1] 0x{:08X} - ch_mask=0b{:08b}, board_id={}, fail={}",
        w1, ch_mask, board_id, board_fail
    );
    println!("  [2] 0x{:08X} - aggr_counter={}", w2, aggr_counter);
    println!("  [3] 0x{:08X} - time_tag={}", w3, w3);

    if magic != 0xA {
        println!("  [!!!] Invalid magic number!");
        return;
    }

    println!(
        "  Board aggregate size: {} words = {} bytes",
        board_size,
        board_size * 4
    );
    offset += 16;

    // Channel Aggregate Header (2 words = 8 bytes)
    println!("\n--- Channel Aggregate Header ---");
    let cw0 = read_u32(data, offset);
    let cw1 = read_u32(data, offset + 4);

    let ch_size = cw0 & 0x003F_FFFF;
    let ch_magic = (cw0 >> 31) & 1;
    let num_samples_wave = cw1 & 0xFFFF;
    let dp1 = (cw1 >> 16) & 0x7;
    let dp2 = (cw1 >> 19) & 0x7;
    let ap = (cw1 >> 22) & 0x3;
    let extra_opt = (cw1 >> 24) & 0x7;
    let es = (cw1 >> 27) & 1;
    let ee = (cw1 >> 28) & 1;
    let et = (cw1 >> 29) & 1;
    let eq = (cw1 >> 30) & 1;
    let dt = (cw1 >> 31) & 1;

    println!(
        "  [4] 0x{:08X} - ch_size={} words, magic={}",
        cw0, ch_size, ch_magic
    );
    println!(
        "  [5] 0x{:08X} - num_samples/8={} (samples={})",
        cw1,
        num_samples_wave,
        num_samples_wave * 8
    );
    println!(
        "      DT={} EQ={} ET={} EE={} ES={} AP={} DP1={} DP2={} Extra={}",
        dt, eq, et, ee, es, ap, dp1, dp2, extra_opt
    );
    offset += 8;

    // Calculate expected sizes
    let waveform_words = if es == 1 {
        num_samples_wave as usize * 4
    } else {
        0
    };
    let time_words = if et == 1 { 1 } else { 0 };
    let extras_words = if ee == 1 { 1 } else { 0 };
    let charge_words = if eq == 1 { 1 } else { 0 };
    let event_words = time_words + waveform_words + extras_words + charge_words;

    println!("\n--- Event Size Calculation ---");
    println!("  Time words:     {} (ET={})", time_words, et);
    println!(
        "  Waveform words: {} (ES={}, num_samples_wave={} * 4)",
        waveform_words, es, num_samples_wave
    );
    println!("  Extras words:   {} (EE={})", extras_words, ee);
    println!("  Charge words:   {} (EQ={})", charge_words, eq);
    println!(
        "  Event total:    {} words = {} bytes",
        event_words,
        event_words * 4
    );

    // Calculate number of events
    let ch_data_words = ch_size as usize - 2; // subtract header
    let n_events = if event_words > 0 {
        ch_data_words / event_words
    } else {
        0
    };
    println!("  Channel data:   {} words (ch_size - 2)", ch_data_words);
    println!("  Events:         {} (ch_data / event_size)", n_events);

    if !ch_data_words.is_multiple_of(event_words) {
        println!(
            "  [!!!] WARNING: ch_data_words not divisible by event_words! Remainder = {}",
            ch_data_words % event_words
        );
    }

    // Dump first event
    if n_events > 0 && event_words > 0 {
        println!("\n--- First Event Data ---");
        dump_event(
            data,
            offset,
            et == 1,
            es == 1,
            ee == 1,
            eq == 1,
            waveform_words,
            dt == 1,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn dump_event(
    data: &[u8],
    mut offset: usize,
    et: bool,
    es: bool,
    ee: bool,
    eq: bool,
    waveform_words: usize,
    dual_trace: bool,
) {
    let start_offset = offset;

    // Time tag
    if et {
        let time = read_u32(data, offset);
        let ch_bit = (time >> 31) & 1;
        let time_val = time & 0x7FFF_FFFF;
        println!(
            "  Time tag: 0x{:08X} (ch_bit={}, time={})",
            time, ch_bit, time_val
        );
        offset += 4;
    }

    // Waveform
    if es && waveform_words > 0 {
        println!(
            "  Waveform ({} words = {} bytes):",
            waveform_words,
            waveform_words * 4
        );

        // Show first 8 words in detail
        let show_words = std::cmp::min(8, waveform_words);
        for i in 0..show_words {
            let w = read_u32(data, offset + i * 4);
            let s0 = w & 0x3FFF;
            let dp1_0 = (w >> 14) & 1;
            let dp2_0 = (w >> 15) & 1;
            let s1 = (w >> 16) & 0x3FFF;
            let dp1_1 = (w >> 30) & 1;
            let dp2_1 = (w >> 31) & 1;

            if dual_trace {
                println!(
                    "    Word[{}]: 0x{:08X} - AP1[{}]={} AP2[{}]={}",
                    i, w, i, s0, i, s1
                );
            } else {
                println!(
                    "    Word[{}]: 0x{:08X} - S[{}]={} S[{}]={} (DP1={},{} DP2={},{})",
                    i,
                    w,
                    i * 2,
                    s0,
                    i * 2 + 1,
                    s1,
                    dp1_0,
                    dp1_1,
                    dp2_0,
                    dp2_1
                );
            }
        }

        if waveform_words > 8 {
            println!("    ... {} more words ...", waveform_words - 8);
            // Show last 2 words
            for i in (waveform_words - 2)..waveform_words {
                let w = read_u32(data, offset + i * 4);
                let s0 = w & 0x3FFF;
                let s1 = (w >> 16) & 0x3FFF;
                println!(
                    "    Word[{}]: 0x{:08X} - S[{}]={} S[{}]={}",
                    i,
                    w,
                    i * 2,
                    s0,
                    i * 2 + 1,
                    s1
                );
            }
        }

        // Find min/max in waveform
        let mut min_val = i16::MAX;
        let mut max_val = i16::MIN;
        let mut min_pos = 0;
        for i in 0..waveform_words {
            let w = read_u32(data, offset + i * 4);
            let s0 = (w & 0x3FFF) as i16;
            let s1 = ((w >> 16) & 0x3FFF) as i16;

            if s0 < min_val {
                min_val = s0;
                min_pos = i * 2;
            }
            if s1 < min_val {
                min_val = s1;
                min_pos = i * 2 + 1;
            }
            if s0 > max_val {
                max_val = s0;
            }
            if s1 > max_val {
                max_val = s1;
            }
        }
        println!(
            "  Waveform stats: min={} max={} range={} min_pos={}",
            min_val,
            max_val,
            max_val - min_val,
            min_pos
        );

        offset += waveform_words * 4;
    }

    // Extras
    if ee {
        let extras = read_u32(data, offset);
        let fine_time = extras & 0x3FF;
        let flags = (extras >> 10) & 0x3F;
        let ext_time = (extras >> 16) & 0xFFFF;
        println!(
            "  Extras: 0x{:08X} (ext_time={}, flags=0b{:06b}, fine_time={})",
            extras, ext_time, flags, fine_time
        );
        offset += 4;
    }

    // Charge
    if eq {
        let charge = read_u32(data, offset);
        let charge_short = charge & 0x7FFF;
        let pileup = (charge >> 15) & 1;
        let charge_long = (charge >> 16) & 0xFFFF;
        println!(
            "  Charge: 0x{:08X} (long={}, short={}, pileup={})",
            charge, charge_long, charge_short, pileup
        );
        offset += 4;
    }

    println!("  Event size: {} bytes", offset - start_offset);
}

fn read_u32(data: &[u8], offset: usize) -> u32 {
    if offset + 4 > data.len() {
        return 0;
    }
    u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}
