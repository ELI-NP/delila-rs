//! Fine Timestamp Bit Position Verification for DIG1 (PSD1/PHA1)
//!
//! Usage: fine_ts_verify <URL> [channel]
//!   e.g.: fine_ts_verify "dig1://caen.internal/optical_link?link_num=0&conet_node=0" 0

use delila_rs::reader::CaenHandle;
use std::env;
use std::thread;
use std::time::Duration;

const DEFAULT_URL: &str = "dig1://caen.internal/usb?link_num=0";
const DEFAULT_CH: usize = 0;
const ACQ_SLEEP_MS: u64 = 3000;
const READ_TIMEOUT_MS: i32 = 2000;
const CFD_SETTINGS_BASE: u32 = 0x103C;

// --- Data structures ---

#[allow(dead_code)]
struct HwEvent {
    fine_time: u16,
    extras_raw: u32,
}

struct SwEvent {
    upper_i16: i16,
    lower_i16: i16,
    extras_raw: u32,
}

enum PhaseResult {
    Hw(HwEvent),
    Sw(SwEvent),
}

fn main() {
    println!("=== DIG1 Fine TS Bit Position Verification ===\n");

    let args: Vec<String> = env::args().collect();
    let url = args.get(1).map(|s| s.as_str()).unwrap_or(DEFAULT_URL);
    let ch: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(DEFAULT_CH);
    println!("  URL: {}", url);
    println!("  Channel: {}", ch);

    let handle = CaenHandle::open(url).expect("Failed to connect");
    println!("[OK] Connected");

    if let Ok(m) = handle.get_value("/par/ModelName") { print!("  Model: {}", m); }
    if let Ok(s) = handle.get_value("/par/SerialNum") { print!("  SN: {}", s); }
    println!();

    let fw_type = handle.get_value("/par/FwType").unwrap_or_else(|_| "unknown".to_string());
    println!("  Firmware: {}", fw_type);
    let fw_upper = fw_type.to_uppercase().replace('-', "_");
    let is_pha = fw_upper.contains("PHA");

    if let Ok(tb) = handle.get_value("/par/timebombdowncounter") {
        let secs: u32 = tb.parse().unwrap_or(0);
        if secs == 0 { println!("[!!!] TIMEBOMB EXPIRED!"); return; }
        println!("  Timebomb: {}:{:02}", secs / 60, secs % 60);
    }

    // Drop initial handle so run_phase can open fresh connections
    drop(handle);

    // ===== Phase 1: HW Fine TS =====
    println!("\n{}", "=".repeat(60));
    println!("=== Phase 1: HW Fine TS ===");
    println!("{}", "=".repeat(60));

    let hw_extras_opt = if is_pha { "EXTRAS_OPT_TT48_FINETT" } else { "EXTRAS_OPT_TT48_FLAGS_FINETT" };
    let hw_events = run_phase(url, ch, hw_extras_opt, true);

    if hw_events.is_empty() {
        println!("[ERR] No events. Check signal source on ch {}.", ch);
        return;
    }
    print_hw_stats(&hw_events);

    // ===== Phase 2: SAZC/SBZC =====
    println!("\n{}", "=".repeat(60));
    println!("=== Phase 2: SAZC/SBZC ===");
    println!("{}", "=".repeat(60));

    let sw_extras_opt = if is_pha { "EXTRAS_OPT_EBZC_EAZC" } else { "EXTRAS_OPT_SBZC_SAZC" };
    let sw_events = run_phase(url, ch, sw_extras_opt, false);

    if sw_events.is_empty() {
        println!("[ERR] No events in Phase 2.");
        return;
    }
    print_sw_details(&sw_events);

    // ===== Phase 3: Verdict =====
    println!("\n{}", "=".repeat(60));
    println!("=== Phase 3: Verdict ===");
    println!("{}", "=".repeat(60));
    print_verdict(&hw_events, &sw_events, &fw_type);
}

/// Reset → configure → set extras_opt → endpoint → acquire → parse
/// Takes URL instead of handle because /cmd/reset invalidates FELib internal state,
/// requiring a fresh connection.
fn run_phase(url: &str, ch: usize, extras_opt: &str, is_hw: bool) -> Vec<PhaseResult> {
    let handle = match CaenHandle::open(url) {
        Ok(h) => h,
        Err(e) => { println!("[ERR] open: {}", e); return vec![]; }
    };
    let _ = handle.send_command("/cmd/reset");
    thread::sleep(Duration::from_millis(200));

    // Configure
    set(&handle, "/par/reclen", "128");
    set(&handle, "/par/waveforms", "FALSE");
    set(&handle, "/par/extras", "TRUE");

    // Use production-like settings; override polarity/threshold per signal
    set(&handle, &format!("/ch/{}/par/ch_enabled", ch), "True");
    set(&handle, &format!("/ch/{}/par/ch_polarity", ch), "POLARITY_POSITIVE");
    set(&handle, &format!("/ch/{}/par/ch_dcoffset", ch), "5");
    set(&handle, &format!("/ch/{}/par/ch_threshold", ch), "200");
    set(&handle, &format!("/ch/{}/par/ch_discr_mode", ch), "DISCR_MODE_CFD");
    set(&handle, &format!("/ch/{}/par/ch_self_trg_enable", ch), "TRUE");
    set(&handle, &format!("/ch/{}/par/ch_cfd_delay", ch), "8");
    set(&handle, &format!("/ch/{}/par/ch_cfd_fraction", ch), "CFD_FRACTLIST_50");
    set(&handle, &format!("/ch/{}/par/ch_cfd_smoothexp", ch), "CFD_SMOOTH_EXP_1");
    set(&handle, &format!("/ch/{}/par/ch_indyn", ch), "INDYN_2_0_VPP");
    set(&handle, &format!("/ch/{}/par/ch_chargesens", ch), "CHARGESENS_640_FC_LSB_VPP");
    set(&handle, &format!("/ch/{}/par/ch_gatepre", ch), "70");
    set(&handle, &format!("/ch/{}/par/ch_gate", ch), "256");
    set(&handle, &format!("/ch/{}/par/ch_gateshort", ch), "45");

    for other in 0..16 {
        if other != ch {
            let _ = handle.set_value(&format!("/ch/{}/par/ch_enabled", other), "False");
        }
    }

    // Set extras opt for all channels (DIG1 requires pair consistency)
    for c in 0..16 {
        let _ = handle.set_value(&format!("/ch/{}/par/ch_extras_opt", c), extras_opt);
    }
    println!("  ch_extras_opt = {}", extras_opt);

    // Print CFD register (only Phase 1)
    if is_hw {
        print_cfd_register(&handle, ch);
    }

    // Configure endpoint
    let endpoint = match handle.configure_endpoint(false) {
        Ok(ep) => ep,
        Err(e) => {
            println!("[ERR] configure_endpoint: {}", e);
            return vec![];
        }
    };

    // Acquire
    let _ = handle.send_command("/cmd/cleardata");
    if let Err(e) = handle.send_command("/cmd/armacquisition") {
        println!("[ERR] Arm: {}", e);
        return vec![];
    }

    println!("  Acquiring for {}ms...", ACQ_SLEEP_MS);
    thread::sleep(Duration::from_millis(ACQ_SLEEP_MS));

    let mut buffer = vec![0u8; 64 * 1024 * 1024];
    let result = match endpoint.read_data(READ_TIMEOUT_MS, &mut buffer) {
        Ok(Some(raw)) => {
            println!("  Read {} bytes", raw.size);
            parse_aggregate(&raw.data, raw.size, is_hw)
        }
        Ok(None) => { println!("  Timeout"); vec![] }
        Err(e) => { println!("  Read error: {}", e); vec![] }
    };

    let _ = handle.send_command("/cmd/disarmacquisition");
    result
}

fn print_cfd_register(handle: &CaenHandle, ch: usize) {
    let addr = CFD_SETTINGS_BASE + (ch as u32) * 0x0100;
    if let Ok(val) = handle.get_user_register(addr) {
        let delay = val & 0xFF;
        let fraction = (val >> 8) & 0x3;
        let interp_pt = (val >> 10) & 0x3;
        println!("\n  CFD Register (0x{:04X}): 0x{:08X}", addr, val);
        println!("    Delay={} samples, Fraction={}%, InterpPt={}",
            delay, [25, 50, 75, 100][fraction as usize], interp_pt);
    }
}

// --- Parsing ---

fn parse_aggregate(data: &[u8], size: usize, is_hw: bool) -> Vec<PhaseResult> {
    let mut results = vec![];
    let mut offset = 0;
    let mut header_printed = false;

    while offset + 16 <= size {
        let w0 = read_u32(data, offset);
        if (w0 >> 28) & 0xF != 0xA { break; }
        let board_size = (w0 & 0x0FFF_FFFF) as usize;
        let board_end = offset + board_size * 4;
        if board_end > size { break; }
        let ch_mask = read_u32(data, offset + 4) & 0xFF;
        offset += 16;

        for pair in 0..8u8 {
            if ch_mask & (1 << pair) == 0 { continue; }
            if offset + 8 > board_end { break; }

            let cw0 = read_u32(data, offset);
            let cw1 = read_u32(data, offset + 4);
            let ch_block_end = offset + (cw0 & 0x003F_FFFF) as usize * 4;
            if ch_block_end > board_end { break; }

            let nsw = (cw1 & 0xFFFF) as usize;
            let extra_opt = (cw1 >> 24) & 0x7;
            let es = (cw1 >> 27) & 1;
            let ee = (cw1 >> 28) & 1;
            let et = (cw1 >> 29) & 1;
            let eq = (cw1 >> 30) & 1;

            if !header_printed {
                println!("  Channel header: extra_option={} (0b{:03b}), ET={} EE={} EQ={} ES={}",
                    extra_opt, extra_opt, et, ee, eq, es);
                header_printed = true;
            }

            let wf_words = if es == 1 { nsw * 4 } else { 0 };
            offset += 8;

            while offset < ch_block_end {
                if et == 0 || offset + 4 > size { break; }
                offset += 4; // time tag

                if es == 1 { offset += wf_words * 4; }

                if ee == 1 {
                    if offset + 4 > size { break; }
                    let extras = read_u32(data, offset);
                    offset += 4;
                    if is_hw {
                        results.push(PhaseResult::Hw(HwEvent {
                            fine_time: (extras & 0x3FF) as u16,
                            extras_raw: extras,
                        }));
                    } else {
                        results.push(PhaseResult::Sw(SwEvent {
                            upper_i16: ((extras >> 16) & 0xFFFF) as u16 as i16,
                            lower_i16: (extras & 0xFFFF) as u16 as i16,
                            extras_raw: extras,
                        }));
                    }
                }

                if eq == 1 { offset += 4; }
            }
            offset = ch_block_end;
        }
        offset = board_end;
    }
    results
}

// --- Output ---

fn print_hw_stats(events: &[PhaseResult]) {
    let hw: Vec<u16> = events.iter().filter_map(|e| match e {
        PhaseResult::Hw(h) => Some(h.fine_time), _ => None
    }).collect();
    println!("  Collected {} events", hw.len());
    if hw.is_empty() { return; }
    let min = *hw.iter().min().unwrap();
    let max = *hw.iter().max().unwrap();
    let mean: f64 = hw.iter().map(|&x| x as f64).sum::<f64>() / hw.len() as f64;
    println!("  Fine TS: min={} max={} mean={:.1}", min, max, mean);
    println!("  Histogram (10 bins):");
    for i in 0..10 {
        let (lo, hi) = ((i * 1024 / 10) as u16, ((i + 1) * 1024 / 10) as u16);
        let c = hw.iter().filter(|&&v| v >= lo && v < hi).count();
        let bar = "#".repeat((c as f64 / hw.len() as f64 * 40.0) as usize);
        println!("    [{:4}-{:4}): {:5} {}", lo, hi, c, bar);
    }
}

fn print_sw_details(events: &[PhaseResult]) {
    let sw: Vec<&SwEvent> = events.iter().filter_map(|e| match e {
        PhaseResult::Sw(s) => Some(s), _ => None
    }).collect();
    println!("  Collected {} events\n", sw.len());
    if sw.is_empty() { return; }

    println!("  First {} events:", sw.len().min(10));
    for (i, ev) in sw.iter().take(10).enumerate() {
        let (fa, fb) = fractions(ev);
        println!("    #{:3}: raw=0x{:08X}  upper={:6}  lower={:6}  fracA={:7.3}  fracB={:7.3}",
            i, ev.extras_raw, ev.upper_i16, ev.lower_i16, fa, fb);
    }

    let n = sw.len();
    let (mut un_lp, mut up_ln, mut oth) = (0, 0, 0);
    for ev in &sw {
        match (ev.upper_i16 < 0, ev.lower_i16 > 0) {
            (true, true) => un_lp += 1,
            _ if ev.upper_i16 > 0 && ev.lower_i16 < 0 => up_ln += 1,
            _ => oth += 1,
        }
    }
    println!("\n  Sign analysis:");
    println!("    upper<0, lower>0: {:5} ({:.1}%)", un_lp, pct(un_lp, n));
    println!("    upper>0, lower<0: {:5} ({:.1}%)", up_ln, pct(up_ln, n));
    println!("    other:            {:5} ({:.1}%)", oth, pct(oth, n));

    let (mut av, mut bv) = (0, 0);
    for ev in &sw {
        let (fa, fb) = fractions(ev);
        if (0.0..1.0).contains(&fa) { av += 1; }
        if (0.0..1.0).contains(&fb) { bv += 1; }
    }
    println!("\n  Fraction in [0,1):");
    println!("    A (upper=After,  lower=Before): {}/{} ({:.1}%)", av, n, pct(av, n));
    println!("    B (upper=Before, lower=After):  {}/{} ({:.1}%)", bv, n, pct(bv, n));
}

fn print_verdict(hw_events: &[PhaseResult], sw_events: &[PhaseResult], fw: &str) {
    let hw_fracs: Vec<f64> = hw_events.iter().filter_map(|e| match e {
        PhaseResult::Hw(h) => Some(h.fine_time as f64 / 1024.0), _ => None
    }).collect();
    let sw: Vec<&SwEvent> = sw_events.iter().filter_map(|e| match e {
        PhaseResult::Sw(s) => Some(s), _ => None
    }).collect();
    if hw_fracs.is_empty() || sw.is_empty() {
        println!("  Not enough data."); return;
    }

    let n = sw.len();
    let (mut av, mut bv) = (0, 0);
    for ev in &sw {
        let (fa, fb) = fractions(ev);
        if (0.0..1.0).contains(&fa) { av += 1; }
        if (0.0..1.0).contains(&fb) { bv += 1; }
    }
    let (ap, bp) = (pct(av, n), pct(bv, n));

    println!("\n  HW events: {}, SW events: {}", hw_fracs.len(), n);
    println!("  Interp A valid: {}/{} ({:.1}%)", av, n, ap);
    println!("  Interp B valid: {}/{} ({:.1}%)", bv, n, bp);

    let use_a = ap > bp;
    let sw_fracs: Vec<f64> = sw.iter().map(|ev| {
        let (fa, fb) = fractions(ev);
        (if use_a { fa } else { fb }).clamp(0.0, 1.0)
    }).collect();

    println!("\n  Distribution (10 bins):");
    println!("  {:>12} {:>8} {:>8}", "Range", "HW", "SW");
    for i in 0..10 {
        let (lo, hi) = (i as f64 * 0.1, (i + 1) as f64 * 0.1);
        let hc = hw_fracs.iter().filter(|&&v| v >= lo && v < hi).count();
        let sc = sw_fracs.iter().filter(|&&v| v >= lo && v < hi).count();
        println!("  [{:.1}-{:.1}): {:>8} {:>8}", lo, hi, hc, sc);
    }

    println!("\n  ============ VERDICT ============");
    if ap > 80.0 && bp < 50.0 {
        println!("  Correct: A (upper=After, lower=Before)");
        println!("  {} FW: bits[31:16]=After ZC, bits[15:0]=Before ZC", fw);
    } else if bp > 80.0 && ap < 50.0 {
        println!("  Correct: B (upper=Before, lower=After)");
        println!("  {} FW: bits[31:16]=Before ZC, bits[15:0]=After ZC", fw);
    } else if ap > 80.0 && bp > 80.0 {
        println!("  AMBIGUOUS: Both valid. Check sign analysis.");
    } else {
        println!("  INCONCLUSIVE: Check CFD params, signal, polarity.");
    }
    println!("  ==================================");
}

// --- Helpers ---

/// Compute fine fractions using two interpretations.
/// Values are 14-bit unsigned ADC centered at 8192 (= zero crossing point).
/// Interpretation A: upper=After ZC, lower=Before ZC (PSD1 doc)
///   fraction = (baseline - lower) / (upper - lower)
/// Interpretation B: upper=Before ZC, lower=After ZC (PHA1 doc)
///   fraction = (baseline - upper) / (lower - upper)
fn fractions(ev: &SwEvent) -> (f64, f64) {
    const BASELINE: f64 = 8192.0; // 14-bit ADC midpoint
    let u = ev.upper_i16 as i32 as f64; // use i32 to preserve sign from i16
    let l = ev.lower_i16 as i32 as f64;

    // Interp A: upper=After, lower=Before → fraction = (BL - Before) / (After - Before)
    let da = u - l;
    let fa = if da.abs() > f64::EPSILON { (BASELINE - l) / da } else { f64::NAN };

    // Interp B: upper=Before, lower=After → fraction = (BL - Before) / (After - Before)
    let db = l - u;
    let fb = if db.abs() > f64::EPSILON { (BASELINE - u) / db } else { f64::NAN };

    (fa, fb)
}

fn pct(c: usize, t: usize) -> f64 {
    if t == 0 { 0.0 } else { c as f64 / t as f64 * 100.0 }
}

fn set(handle: &CaenHandle, path: &str, value: &str) {
    if let Err(e) = handle.set_value(path, value) {
        println!("  [ERR] {} = {}: {}", path, value, e);
    }
}

fn read_u32(data: &[u8], offset: usize) -> u32 {
    if offset + 4 > data.len() { return 0; }
    u32::from_le_bytes([data[offset], data[offset + 1], data[offset + 2], data[offset + 3]])
}
