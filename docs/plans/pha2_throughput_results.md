# PHA2 Throughput Benchmark Results

**Date:** 2026-05-04
**Hardware:** VX2730 SN:52622 @ 172.18.4.56, FwType=`DPP_PHA`, FPGA FW 1.0.88
**Software:** delila-rs at commit `183cefb` (post-Phase-3 truncation-resync + sweep-robustness)
**Network:** Reader host on the same LAN segment as the digitizer (1 Gb/s ethernet)

## Setup

- 1 channel (ch0) enabled, TestPulse internal generator
- Reader → Merger → Monitor pipeline (no Recorder; `--no-mongo`)
- `events_processed` measured from `/api/status` after `--warmup` seconds, then again after `--duration` seconds; `(end − start) / duration` = delivered rate
- Rates / record-lengths swept by editing `config/digitizers/pha2_thrput.json` between iterations
- Configure → Arm → Start → measure → Stop → Reset between every iteration (rapid Configure cycles)

## Single-shot fresh-start results

When the FW is freshly Configured and not in a wedged state, throughput tracks
target rate cleanly up to the network bandwidth ceiling.

| Target rate | record_length | Duration | Events delivered | Rate | Bytes/event | Loss | Notes |
|---:|---:|---:|---:|---:|---:|---:|---|
|  10 kHz | 400 samples | 10 s | 101,206 | 10.12 k/s | 1631 |   0 | 100 % delivery |
|  50 kHz | 400 samples | 10 s | 506,136 | 50.61 k/s | 1631 |   1 | 100 % delivery |
| 100 kHz | 400 samples | 10 s | 734,005 | 73.40 k/s | 1487 |   1 | 73 % — FW-side ceiling |
| 200 kHz | 1000 samples | 8 s | 486,399 | 60.80 k/s | 1800 |   0 | 30 % — bandwidth-limited (108 MB/s) |
| 500 kHz | 200 samples | 8 s | 1,182,560 | 147.82 k/s | 740  |   0 | **0 decoder errors at 147 k/s sustained** |

**Interpretation:**
- record_length=400 samples (1631 B/event): network ceiling at ~67 kHz
- record_length=1000 samples (4032 B/event): network ceiling at ~27 kHz
- record_length=200 samples (740 B/event): network ceiling at ~150 kHz
- All ceilings line up with `(108 MB/s) / bytes_per_event`, so PHA2 + decoder
  on this host is **network-limited, not CPU- or FW-limited**, in the
  ranges we exercised.

## Bandwidth-saturated 36-iter sweep (samples ∈ {400, 600, 800, 1000} × rates ∈ {1, 2, 5, 10, 20, 30, 50, 70, 100} kHz)

After the post-`183cefb` STUCK / SAT / OK classifier:

| Outcome | Count | Notes |
|---|---:|---|
| OK (≥ 90 % of target) | 26 | low-rate iterations + record_length=400 up to 50 kHz |
| SAT (events flowing, < 90 % of target) | 9 | network-bandwidth-saturated, e.g. 1000-sample × 50–100 kHz |
| STUCK after 1 retry | 1 | samples=400 × 70 kHz, FW wedged → 0 events even after 30 s idle |
|  Hard-stuck rate | **~3 %** | matches what's expected of CAEN PHA firmware historically |

The single hard-stuck case is the known PHA1/PHA2 FW quirk (rare random
wedge after rapid Configure cycles). Operational SOP for production runs:
**check the ADC spectrum on Monitor right after Start; if it doesn't look
right, power-cycle the crate.** No software workaround is fully reliable
beyond extending `--stuck-cool-down` to several minutes.

## Energy resolution baseline

For the **Phase 3 physical pulser** test on ch0 (`config/config_pha2_56_phys.toml`):

- Input: 1 kHz pulser, Positive polarity, ~1.5k ADC step amplitude
- Trapezoid: rise 5000 ns, flat-top 1000 ns, pole-zero 50000 ns
- Result via Monitor Gaussian fit: **FWHM / Center = 6.4 / 6327.6 = 0.101 %**
- Cross-check via offline ROOT (`delila2root` → ROOT TTree, 5950 events):
  Mean=6317.14, StdDev=2.61 → **FWHM = 2.355 × σ = 6.13** ≈ Monitor fit
- This is the noise floor with a clean pulser; real detectors will
  add their own contribution.

## Tooling

- `scripts/throughput_sweep.py` — generic 2-D sweep (rate × record_length)
  with cool-down + STUCK-only retry. `--json-path` selects PSD2 vs PHA2
  config.
- `scripts/pha2_transition_stress.py` — stress-tests rapid Configure
  cycles to surface the FW wedge state for diagnostics.
- `target/release/delila2root` — flat-TTree exporter for offline analysis
  (waveforms included; build with `cargo build --release --features root
  --bin delila2root`). Handles the current `user_info[4]` + Phase 4.5
  probe-type + AMax-debug 16-digital-probe schema. The legacy C++
  `tools/delila2root` was retired in TODO 56 (2026-05-15).

## Cross-references

- [TODO/51_pha2_integration.md](../../TODO/51_pha2_integration.md) — full integration plan
- [docs/digitizer_system_spec.md](../digitizer_system_spec.md) — supported-digitizers table
- [docs/compass_devtree_mapping.md](../compass_devtree_mapping.md) — PHA2 channel + board params
- Memory: [pha_fw_misbehavior_sop.md](/Users/aogaki/.claude/projects/-Users-aogaki-WorkSpace-delila-rs/memory/pha_fw_misbehavior_sop.md) — PHA FW operational SOP
