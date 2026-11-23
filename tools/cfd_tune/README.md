# cfd_tune — offline CFD parameter tuning for DPP-PSD (TODO 90)

Software model of the V1730 / V1725 DPP-PSD digital CFD (`cfd_emu.hpp`), checked
against the firmware, for scanning CFD delay / fraction / smoothing on recorded
waveforms instead of taking one run per setting. First use: ThGEM delay lines,
whose time sums `t_XL + t_XR - 2 t_T` and `t_YU + t_YD - 2 t_T` do not depend on
the hit position, so their width is the timing resolution. Plan and status:
`TODO/90_psd_cfd_waveform_tune.md`.

| file | what |
|---|---|
| `cfd_emu.hpp` | the model: baseline + polarity, moving-average smoothing, `c = f*s[i] - s[i-D]`, crossing + to - after the arming threshold, linear interpolation (UM4380 rev6 p.14/29/30/38) |
| `cfd_scan.hpp` | scan logic: digitizer JSON -> FW setting per channel, re-timing a hit, ThGEM grouping (nearest arm, thgem_sink rule), time-sum width (sigma68), coordinate descent, JSON patch |
| `cfd_scan.cxx` | the tool: reads `.delila`, scans each ThGEM, prints the widths and the best settings, writes a patch |
| `test_cfd_emu.cpp`, `test_cfd_scan.cpp` | unit tests (no ROOT) |
| `p2_check.C` | ROOT macro: software crossing vs the FW fine time, per event (the model check, pulser) |
| `p2_delila.cxx` | the same check straight from `.delila`, every ThGEM channel of the map at its JSON setting; `--from` skips noise before the trigger; dual-trace records compared as recorded / rebuilt linear / cubic |
| `amp_check.cxx` | trigger threshold vs noise: pulse heights of real (Qlong >= cut) and noise hits per channel, fraction left above candidate thresholds |
| `coinc_arms.cxx` | heights / charges / delays of the arm hits that belong to a real trigger (what an arm threshold or charge cut would lose) |
| `waveform_run.py` | take one run with single-trace waveforms on some boards, restore their settings exactly |
| `apply_patch.py` | apply a cfd_scan patch through the Operator REST API (dry run by default) |

## Build and test

```
cd tools/cfd_tune
g++ -std=c++17 -O0 -g -Wall -Wextra test_cfd_emu.cpp -o /tmp/tce && /tmp/tce
g++ -std=c++17 -O0 -g -Wall -Wextra -I../delila2root -I../root_sink/thgem test_cfd_scan.cpp -o /tmp/tcs && /tmp/tcs
g++ -O2 -std=c++17 -Wall -Wextra -I../delila2root -I../root_sink/thgem cfd_scan.cxx -o cfd_scan
python3 -I apply_patch.py --self-test
```

## Procedure

1. **Waveforms** (DAQ Idle/Configured, no Tune Up). Single trace only: dual trace
   halves the sampling (every value written twice, while the record still says
   2 ns/sample — `cfd_scan` refuses such records). Keep the record below 1792
   samples (couple arbiter). ThGEM pulses are slow (~100 ns rise): give them a
   pre-trigger of 500 ns so the baseline is clean. If noise triggers dominate
   (pf-qaq 2026-11-21: 35 kHz a board, 670 MB/s), raise the thresholds for the
   run only (`--patch`, values from `amp_check` / `coinc_arms`):

   ```
   python3 -I waveform_run.py --modules 1,2,3 --seconds 1800 --record-ns 1200 \
       --pre-trigger-ns 500 --patch thresholds.json
   ```

2. **Model check on these signals** with the settings the run was taken with
   (for a `--patch` run: the JSON + the patch):

   ```
   ./p2_delila --map MAP --digitizer JSON1 --digitizer JSON2 --digitizer JSON3 --baseline 64 run.delila
   ```

   On the pulser the RMS was 0.002 ns. On noisy ThGEM signals (2026-11-21) it is
   0.2-0.27 samples: the FW baseline averages 256 samples before the trigger,
   mostly outside the record, so its noise cannot be reproduced. The scan is
   still usable when the time-sum widths are far larger (8-35 ns there).

3. **Scan** with the digitizer JSONs the run was taken with:

   ```
   ./cfd_scan --map config/channel_map_11B_208Pb.txt \
       --digitizer config/digitizers/11B_208Pb_v1730_psd1_1.json \
       --digitizer config/digitizers/11B_208Pb_v1730_psd1_2.json \
       --digitizer config/digitizers/11B_208Pb_v1730_psd1_3.json \
       --min-energy 4500 --delays-ns 2:200 --baseline 64 [--thgem N] \
       --out /tmp/cfd_patch.json /data/DAQ_Test/runNNNN_000?_DAQ_Test.delila
   ```

   Per ThGEM: sigma68 of S_X / S_Y at the FW setting and at the best setting, the
   best trigger setting and the best arm setting (common to the four arms).

4. **Apply** (dry run first), then **check** on a new run without waveforms:

   ```
   python3 -I apply_patch.py /tmp/cfd_patch.json
   python3 -I apply_patch.py /tmp/cfd_patch.json --apply
   ./cfd_scan --fw-only --map config/channel_map_11B_208Pb.txt runMMMM_0000_X.delila
   ```

## Results so far (2026-11-20, pf-qaq, TTL pulser on V1730 ch15)

- Model vs FW fine time: RMS 0.002 ns, one integer offset, 3 boards x 5000 events.
  Moving average confirmed (exponential smoothing is off by 0.48 samples).
- The FW CFD probe (dual trace) goes + then - through its 8191 midline, as the
  model; it is recorded about 28 samples (55 ns) later than the input trace.
- Smoke test of the whole chain with a fake "ThGEM" made of the three pulser
  channels: 3000 events, 1201 settings in 8 s, patch written, waveform_run.py
  restored every setting exactly.

## ThGEM with a Cf source (2026-11-21, pf-qaq runs 17-18, TODO 90 §4.3)

- Noise triggers dominate at thresholds ~100: damped 15 MHz ringing bursts and
  50-60 MHz pickup. Their Qlong sits at the pedestal (1500-3000); real pulses
  5000-15000, about 1 per second per ThGEM.
- Thresholds T 250 / arms 400 cut the ThGEM boards from ~35 kHz to a few Hz to
  ~270 Hz and lose no real arm pulse (arm pulses of real events: >= 470 ADC).
- Scan, D4 (82 / 62 events): sigma68 S_X 13.7 -> 8.0 ns, S_Y 10.7 -> 7.5 ns.
  The other ThGEMs had too few complete events in 30 min.
