# TODO 59 — ELIADE PHA Energy-Resolution Auto-Tune (SW Trapezoid Replay)

**Status: 📋 PLANNING (2026-06-16, revised 2026-07-13)**
**Owner:** Aogaki + Claude
**Experiment:** ELIADE (8× Clover HPGe array, ELI-NP)
**Target timeline:** Bench tests start June 2026 · Ge resolution tune-up through 2026 · Beam from Jan 2027

> **Revision 2026-07-13:** folds in the es2 hardware + code-verification discussion. Main
> changes: ① Phase 0 rewritten around a "low online threshold + `adc_min` gate" capture
> strategy (incl. a 64 GB data-volume design). ② **The AMax trap-core reuse plan is dead**
> (email to the developer went unanswered = politely declined; and code inspection showed
> `amax.rs` is a decoder only — there never was a SW trap in the tree) → switched to
> **from-scratch implementation + probe-overlay reverse engineering**. ③ 20 µs waveforms
> verified on hardware and through the whole DELILA chain (decoder / `.delila` v3 /
> delila2root). ④ Scope and limits of offline RCCR2 trigger optimization captured in §6b.

---

## 0. Context — the ELIADE DAQ in one paragraph

ELIADE runs **4× CAEN V1725 (DPP-PHA, 16 ch, 250 MS/s, 10 ch used each)** for the HPGe
clovers plus **1× V1730 (DPP-PSD)** per host, on up to **8 hosts** (1-host and 8-host
operation must both work). All digitizers share a **common external clock** (no drift —
only a constant per-board start-phase offset) and a **daisy-chained RUN signal** started by
one software-controlled board. γ-γ coincidence is mandatory across the 8 clovers.

This TODO covers **only the energy-resolution auto-tune** (the 2026 deliverable). The other
three ELIADE work-blocks are tracked separately and summarized in §7:

1. `start_delay` global auto-calibration (HW timestamp-origin alignment)
2. Chain-topology management (full vs. subset, topology-tagged calibration tables)
3. Event-builder time offset (PHA↔PSD constant, derived empirically from first-run data)
4. **PHA trap auto-tune ← THIS FILE**

---

## 1. Goal

Fully automate per-channel DPP-PHA trapezoidal-filter parameter selection so that energy
resolution (FWHM at a known γ peak) is minimized, **with zero manual grid-poking by the
operator.** The operator's only manual steps become: take one waveform run (Phase 0) and
nod at the hardware verification (Phase 3).

### Why this is now tractable (the key insight)

The historical blocker was that brute-force tuning is *online* and **acquisition time is the
bottleneck** — every candidate parameter set needs a fresh run with enough counts for a
stable FWHM. That is why Bayesian optimization looked necessary (minimize # of expensive
evals).

**We sidestep it entirely with offline waveform replay.** The trapezoidal filter sits
*downstream* of the digitized preamp waveform. If we record the raw input waveform once, we
can re-apply *any* trap parameters to the *same stored waveforms* in software at **~ms per
eval**. Acquisition is no longer per-eval. With that bottleneck gone:

- **Plain grid search is enough** — no BO needed.
- Optimization runs **offline on a laptop**, no hardware, anytime.
- It fits the existing "raw `.delila` always saved, offline re-processing is canonical"
  philosophy (same as `delila2root`, offline EB replay).
- **Every grid point is evaluated on the same event set**, so statistical errors are
  correlated across grid points and cancel in the ranking. This is the fundamental advantage
  over an online scan (where each point has its own statistics and its own time window —
  temperature/HV drift mixes in).

---

## 2. Pipeline overview

```
Phase 0  Capture (once, on HW)   : normal Run + waveforms_enabled, probe=Input, FW energy
                                    co-recorded, LOW online threshold + adc_min gate,
                                    known γ source, 20 µs window.
Phase 1  SW-trap validation      : replay identical waveforms with the FW's own params,
                                    compare PER-EVENT vs FW energy + sample-level overlay
                                    against the probe2 trapezoid trace.
Phase 2  Offline optimization    : grid-search params over the same stored waveforms,
                                    minimize FWHM(keV) per channel. Pure CPU.
Phase 3  HW verification         : burn optimal params into FW, short run, confirm real FWHM
                                    matches the SW prediction. Lock into config.
```

All of Phases 1-2 live in **one offline tool**: a new bin `pha_trap_tune` (`dev-tools`
feature, same offline family as `delila2root`) that reads a `.delila` file (waveforms + FW
energy) and emits a per-channel config patch.

---

## 3. Phase 0 — Capture (constraints that cannot be fixed later)

### 3.1 Hardware + implementation pre-verification (done 2026-07-13)

- **20 µs windows work on real hardware**: verified on es2 (172.18.4.132, V1725 SN217, PHA)
  in Tune Up with record_length 20000 ns = 5000 samples @ 4 ns.
- **No hidden sample-count limits anywhere in the DELILA chain** (code-verified):
  - PHA1 decoder: `num_samples_wave: u16` × 8 samples/unit → **max ~524k samples**
    (`src/reader/decoder/psd1_pha1_common.rs:118`)
  - `.delila` v3: "fixed-length" means a **fixed field count**; the waveform arrays are
    variable-length `Vec<i16>` (`src/recorder/format.rs:30`)
  - `delila2root`: `std::vector` branches, no fixed-sample-count assumption
- **Event size ≈ 20 kB** (at 5000 samples): 10 kB analog probe + 10 kB digital probes.
  **PHA1 digital probes cannot be disabled** — bit15=Tn (D0, Trigger, fixed) / bit14=DP
  (D1, selectable) are embedded in every waveform word by the FW, and the decoder
  unconditionally expands them to unpacked u8/sample (`src/reader/decoder/pha1.rs:134-168`).
  The missing "None" option in the UI is faithful to the FW. → However, per §6b, **D0=Trigger
  is the ground truth for the trigger emulator**, so for this campaign the digital probes are
  not waste — they are worth recording.

### 3.2 Two run types (separate capture from validation)

| Run | vtrace | dtrace | Rate | Purpose |
|-----|--------|--------|------|---------|
| **capture run** (Phase 0 proper) | probe_0 = **Input** only | D0=Trigger (fixed), D1 any | full 250 MS/s (4 ns/sample) | Phase 2 optimization scan data |
| **validation run** (for Phase 1) | **dual trace: Input + Trapezoid** | D1 = **Peaking** | effectively halved by dual trace (to confirm on HW: interleave → ~8 ns/sample) | SW-trap cross-check vs FW |

The validation run carries the FW's own trapezoid trace (probe2) + the energy-extraction
window (D1=Peaking) + FW energy per event — a reverse-engineering microscope for the SW
implementation (§4). When transferring parameters validated at half rate to full rate, **do
not mix up the samples conversion (4 vs 8 ns/sample)**.

### 3.3 Threshold and adc_min — discard noise before the disk

**Strategy: push the online (RCCR2) threshold way down, and let `adc_min` discard the noise
before it reaches disk.**

- **Why lower the threshold:** ① recorded data is conditioned on the online trigger having
  fired, so for the §6b trigger optimization the online threshold must sit **below the lowest
  candidate threshold to be scanned** (making the dataset a superset); ② the same data then
  also serves low-energy-efficiency studies.
- **`adc_min` (reader-side, per-board):** the extra noise triggers from the low threshold
  carry small FW energy, so the `adc_min` energy floor drops them before disk. Controls both
  total volume and write rate.
- **Handling the per-board constraint — gain alignment:** individual-crystal channels and the
  analog-sum channels differ in gain by several ×, so a per-board `adc_min` needs the FW
  energy coordinates aligned first:
  1. **Absorb the coarse ×4 step with `coarse_gain`** (X1/X4 = input dynamic range
     2/0.5 Vpp). This is the **analog lever** and improves ADC-range utilization itself
     (measured on es2: raw pulse amplitude used only ~12% of the 14-bit range, i.e.
     1 LSB ≈ 0.7 keV — quantization is not negligible against a ~2 keV FWHM. Push the
     individual channels toward X4 to gain amplitude).
  2. **Trim the remainder with `energy_fine_gain`** (×1.0–10.0) to align FW-energy
     coordinates only. **Fine gain is a digital multiplier and has zero effect on the offline
     analysis** (which recomputes from raw waveforms) — its only job here is the adc_min
     coordinate alignment. Two caveats: min is 1.0, so alignment is **upward only**; watch
     the 15-bit energy ceiling (32767) headroom if the 2.5 MeV sum peak matters.
- **⁶⁰Co gate placement:** the Compton edge of 1332 keV is **1118 keV < the 1173 keV peak**.
  Placing `adc_min` just below 1173 keeps **essentially only the two photopeaks**, cutting
  event count by ×10–20. Either back-compute per-channel effective thresholds through the
  fine-gain alignment, or place one conservative common threshold slightly lower and accept
  ~20–30% extra volume.

### 3.4 Statistics and volume design

- **100k gated events/ch** → ~50k counts per peak → FWHM fit precision ~0.6%. More than
  enough for ranking grid points (the same-event-set error correlation helps further).
- 20 kB/event × 100k = **2 GB/ch → 64 GB for all 32 channels**. Per-channel optimization
  data for every crystal fits in this budget (no need for the "record only representative
  channels" compromise).
- Throughput: source rates are low and the gate cuts another ×10, so the Recorder write path
  is not a concern. The `adc_min` gate is **volume control, not bandwidth control** (without
  it, accumulating 50k counts/peak means writing 1–2M events/ch = TB-scale).

### 3.5 Constraints not recoverable offline (unchanged principles)

1. **Waveform window ≥ the longest shaping to be tested.** To sweep rise up to ~8 µs the
   record must contain baseline + full trapezoid response ≈ `2·(rise+flat) + decay`. Current
   plan: **20 µs window + a few µs pre-trigger**. Too-short windows silently corrupt
   long-shaping evals.
2. **Sufficient pre-trigger baseline** (the baseline restorer averages pre-pulse samples;
   2–4 µs as a guide).
3. **No clipping**: after any coarse_gain change, confirm the highest energies of interest
   (incl. the sum peak) stay inside the ADC range.
4. **No decimation** (it breaks filter-response equivalence).

Output: `.delila` files per channel-group with `{raw input waveform, FW energy, FW params
used}` per event (validation run additionally: trapezoid probe + Peaking bit).

---

## 4. Phase 1 — SW-trap validation (the part that must be rigorous)

### 4.1 Implementation strategy (revised 2026-07-13: AMax reuse is dead; build from scratch)

**History of the old plan:** the AMax custom FW is a trapezoidal MCA and the plan assumed we
could obtain its trap logic (code) from the developer. **The email inquiry went unanswered =
treated as a polite decline.** Code inspection additionally showed that the in-tree
`src/reader/decoder/amax.rs` is a **decoder of FW output — there never was a SW trapezoid
implementation in the tree** (950 lines of probe-lane / user-word unpacking only).

**What we lost is a skeleton, not the answer:**

- The recursion itself (§4.3) is fully public (Jordanov-Knoll + UM4380). ~30 lines.
- The FWHM-deciding stages 2/4 (baseline restorer / energy-extraction window) were marked
  "**verify vs 725 PHA**" even under the reuse plan — the target is CAEN's PHA
  implementation; AMax code would have been a reference for a *different* FW. The
  verification work was always all ours.

**New approach: probe-overlay reverse engineering.** The FW's own behavior is observable
per event:

| Ground truth | What it gives | Stage validated |
|---|---|---|
| probe2 = Trapezoid trace | the FW trapezoid **sample-by-sample, incl. fixed-point rounding** | stages 2+3 (localize by which sample diverges first) |
| D1 = Peaking bit | position/width of the FW energy-extraction window | stage 4 |
| FW energy (every event) | final output | end-to-end (per-event residual) |

Overlaying against observable outputs is **more reliable than reading someone's VHDL**
(no risk of misreading it, either).

**Expected iteration points:** BLR holdoff/freeze behavior, fixed-point scaling of the trap
output. Do not expect a first-shot match. Effort estimate: recursion + framework ~half a
day; the real work is BLR + stage-4 refinement (probe-overlay iterations), a few days.

⚠️ **"Linear 1:1 energy correspondence" is NOT sufficient evidence for resolution work**
(principle retained). A linear correspondence only means centroid/gain matches. **FWHM is
set by the filter's noise-transfer characteristic** (trapezoid weighting, baseline restorer,
exact rise/flat-top in samples, fixed-point rounding, energy-extraction window). The constant
gain factor is safely ignorable (resolution is relative; we re-fit the peak and re-calibrate
to keV each eval).

### 4.2 Validation criterion

Run the SW trap on the captured waveforms **with the exact params the FW used**, then:

- Per-event residual `SW_energy − FW_energy`; require its std **≪ peak FWHM (ideally ±1
  LSB)**. **Use per-event residual, NOT a linear-fit R².** Per-event agreement guarantees
  FWHM reproduction; R² does not.
- Overlay **SW trapezoid trace vs. decoded `analog_probe2`** sample-by-sample.
- **SW energy-extraction window vs. the D1=Peaking bit** — positions must agree.
- Decisive check at the FW operating point: **`FWHM_SW ≈ FWHM_FW`**. Only when FWHM (not
  centroid) matches is the model trustworthy for optimization.

This is exactly the CLAUDE.md doctrine ("spec-page reference + real-hardware verification;
no silently-wrong resolution" — cf. the e641e99 heuristic and e45e0ec silent-cache
incidents). **No optimization until Phase 1 passes.**

### 4.3 Trapezoid math anchor (Jordanov-Knoll; per UM4380)

```
l = k + m                                  # k = rise, m = flat-top (in SAMPLES; 725 = 4 ns/sample)
d[n] = v[n] - v[n-k] - v[n-l] + v[n-k-l]
p[n] = p[n-1] + d[n]
r[n] = p[n] + M·d[n]                        # M = pole-zero (decay τ in samples)
s[n] = s[n-1] + r[n]
energy ∝ average of s over the flat-top (peaking) window
```

**Pole-zero M is measured, not searched:** fit the exponential decay of the preamp tail in
the captured waveform → τ → M, per channel. (This also auto-replaces the manual PZ step from
CoMPASS — a useful standalone win.)

### 4.4 Module structure — stage decomposition + per-stage introspection

**Implement the SW trap as separable, inspectable stages so that when Phase 1 fails we can
localize which stage diverges** instead of debugging a monolith. Each stage exposes its
intermediate trace (returning a plain `Vec<f64>` is enough — KISS). PHA energy processing
decomposes as:

| Stage | Content | Affects FWHM? | Implementation plan (revised) |
|-------|---------|:---:|------------|
| 1 | Input (probe_0=Input raw waveform) | — | as-is |
| 2 | Baseline computation / restorer (subtract pre-pulse mean) | **yes** | from scratch; **reverse-engineer BLR behavior via probe2 overlay** |
| 3 | Trapezoid recursion (d,p,r,s with PZ M) | yes (shaping) | from scratch (the public 30 lines) |
| 4 | Energy extraction (sample at peaking position, average over Npk) | **yes** (noise averaging) | from scratch; **window directly observable via D1=Peaking** |
| 5 | Gain normalization → LSB | — | **ignore (linear factor, Aogaki correct)** |

FWHM is decided more by stages **2 and 4** (baseline + energy window) than by the trapezoid
itself. The stage decomposition enables localization of the form "trapezoid matches probe2,
but energy doesn't → it's stage 4's window".

### 4.5 What the pulser can and cannot do (2026-07-13)

**Constraint**: the ELIADE pulser is **directly connected to the digitizer, fixed amplitude,
fixed rate** (no preamp test input available). This kills all preamp-side uses — ENC
decomposition (electronic noise vs. charge collection), absolute sub-threshold trigger
efficiency via amplitude sweeps, linearity calibration, rate/pileup studies.

What survives:

1. **Phase 1 validation accelerator**: identical repeated waveforms make the per-event
   residual ±1 LSB criterion crisp (statistical spread vanishes; what remains is pure
   implementation mismatch). The pulse shape differing from the preamp exponential does not
   weaken the validation (it is an agreement check on the same samples with the same params).
2. **Asynchronous phase scan (the one genuinely new item)**: the pulser is asynchronous to
   the sampling clock, so every pulse lands at a different sub-sample phase = **a free phase
   scan**. The spread of FW energy over identical-amplitude pulses = the filter's
   sampling-phase sensitivity + trigger-jitter leakage into energy. **Measuring this spread
   per flat-top candidate observes the §5.3 trap × trigger cross-term directly, without a
   detector.**
3. **Periodic TRG-IN trigger source** (a use that never feeds the pulser signal into a
   channel): trigger output → TRG-IN forces recording on all channels → **unbiased pure
   baseline samples** of the detector+preamp-connected channels = the mechanism for the
   §6b.4 noise run.
4. **Timing reference (§6b.3) and inter-board common anchor (§7) are unaffected** (both work
   at fixed amplitude/rate).

**ENC-decomposition substitute** (no pulser needed): plot FWHM² vs. E over several γ peaks
within the capture run; the E→0 intercept ≈ electronic noise + quantization. Less rigorous,
but sufficient for interpreting the tuning results.

---

## 5. Phase 2 — Offline optimization (grid is enough)

### 5.1 Search-space pruning with physics priors

- **Pole-zero M:** measured from waveform (§4.3) → fixed, not searched.
- **Trigger threshold:** affects efficiency/noise floor, not resolution at a strong peak →
  fixed sensibly (optimized separately in §6b).
- **Baseline-mean / peak-mean windows:** more averaging = less noise with diminishing
  returns vs. pile-up → set to a reasonable max, optionally coarse-checked.
- **Effective search ≈ rise × flat-top (+ peaking position): 2–3 dims.** The classic
  "resolution vs. shaping time" U-curve.

### 5.2 The optimizer

With ~ms/eval offline, even a fine grid is trivial:

- Coarse grid e.g. `rise ∈ {0.5,1,2,3,4,6,8} µs × flat-top ∈ {0.5,1,1.5,2} µs` ≈ 28 points,
  **< 1 s per channel.**
- Refine with a fine grid around the best point.
- **Channels are independent** → optimize each from its own waveforms, in parallel.
- **BO is unnecessary** (the original motivation, online acquisition cost, is gone).

### 5.3 Scan strategy — deep on 2 channels, narrow rollout (added 2026-07-13)

Hypothesis (Aogaki): **same-type clover crystals have nearby rise/flat optima** (the U-curve
is flat near the optimum). Use it in a testable form:

1. **Deep full-range grid on ~2 representative channels** (rise × flat, plus a PZ
   neighborhood as a cross-check) → establish the optimal region
2. **All remaining channels get only a narrow grid around that region** (saves CPU)
3. PZ is still measured per channel from the waveforms (preamp unit-to-unit variation, §4.3)
4. Any channel whose narrow-scan optimum pins to the region boundary is itself a diagnostic
   (that crystal/preamp is off) → rerun that channel on the full grid

**Caution — the trap × trigger cross-term:** DPP-PHA samples the trapezoid at "Peaking Time
after the trigger", so **trigger jitter/walk becomes sampling-position jitter on the flat
top**. A tight flat top exposes trigger jitter in the resolution tail (flat-top width ×
jitter coupling). The same dataset also drives the §6b trigger scan, so this coupling is
observable — finalize the flat-top value only after seeing the trigger-side results.

### 5.4 FWHM metric — avoid the moving-peak trap

Changing rise time changes the **gain**, so the **peak centroid moves** with the params. The
fitter must **find the peak freely every eval** (Gaussian + linear background), map the
centroid to the known energy (e.g. 1332 keV), and report **FWHM in keV**. Minimizing FWHM at
a *fixed* channel window would be wrong. (For the inner loop a robust half-max width estimate
is fine; do the full Gaussian fit for final reporting.)

### 5.5 Output

Per channel: optimal `{rise, flat-top, peaking, PZ M}` + the FWHM curve (for the colleague
to sanity-check the U-shape), emitted as a **config patch** ready to apply via the existing
`start_delay`-style per-channel config path.

---

## 6. Phase 3 — Hardware verification

Burn the optimal params into FW, take a short run, measure the **real** FWHM, confirm it
matches the SW prediction. Match → lock into config. Mismatch → SW-model gap; the stage
decomposition (§4.4) tells us where to look. This closes the spec-ref + real-HW-verify loop.

---

## 6b. Phase 5 — Trigger & Timing Optimization (efficiency AND timing)

Reuses the **same offline-replay harness**: the trigger path (RCCR2 = RC-CR² + smoothing,
then LED/CFD discrimination; DELILA params `threshold` / `input_rise_time` /
`fast_disc_smooth`) is a deterministic function of the input waveform, exactly like the
trapezoid. Config plumbing partly exists already (`ttf_smoothing`, `trigger_edge`,
`trigger_threshold_v` from V1743 work).

### 6b.0 Fundamental limits of offline trigger optimization (made explicit 2026-07-13)

The decisive difference from the trapezoid: **the trigger decides which events exist in the
dataset.** Recorded data is conditioned on the online trigger, therefore:

**Offline CAN (and powerfully):**
- brute-force the RCCR2 emulator over threshold × input_rise_time × fast_disc_smooth
- evaluate trigger timing jitter/walk (software CFD as the reference clock)
- measure noise-crossing rate: apply RCCR2 to the **pre-trigger region (pure noise)** of
  recorded waveforms and count upward crossings vs. threshold (immune to trigger bias)

**Offline CANNOT close:**
- absolute efficiency for events **below** the online threshold (what isn't in the data
  cannot be measured)
- retrigger / pileup-guard / dead-time behavior at real rates

**The bias countermeasure is already built into Phase 0** (§3.3): the capture run's online
threshold sits below the lowest candidate, making the dataset a superset. The extra noise is
absorbed by adc_min.

**Implementation order: trapezoid → RCCR2.** Establish the probe-overlay validation
framework (§4.1) first, then roll the same structure over to the trigger. The RCCR2 ground
truth is **D0 = Trigger (fixed on PHA1, stamped on every event for free, §3.1)** — validate
the SW emulator by matching the FW's firing sample.

### 6b.1 Structure — why both goals fit one framework (decouple along parameters)

- **Threshold** → pure *efficiency / false-rate* axis. Does **not** affect timing (CFD is
  amplitude-independent). Set as low as the false-rate budget allows: `threshold = k·σ_TTF`,
  k ≈ 4–5.
- **Smoothing + trigger rise time** → the *shared* knob. More smoothing lowers σ_TTF (→ lower
  threshold → better low-E efficiency) BUT widens edges (→ worse timing). **This is the A↔B
  tension.**
- **CFD params (delay, fraction, smoothing/interpolation)** → timing axis only.

So it is a **constrained optimization**, not a vague trade-off:

> **Minimize threshold (maximize low-E efficiency) subject to: timing resolution ≤
> coincidence-window budget AND false-trigger rate ≤ budget.**

The coincidence-window budget is a **physics decision per run-type** (2026 energy tune-up =
loose timing → push threshold very low; 2027 beam coincidence = tight timing → less
smoothing). The tool emits the **Pareto front (efficiency floor vs. timing σ, parametrized by
smoothing)** for inspection, plus the single constrained pick per channel.

### 6b.2 Metric A — low-energy efficiency / false rate

Per {smoothing, rise}: figure of merit = **trigger-filter S/N = (TTF amplitude of the
minimum-energy-of-interest pulse) / σ_TTF**. Maximize → choose smoothing → set threshold at
`k·σ_TTF` → low-E efficiency follows. False-rate = threshold crossings on baseline per unit
time, **with holdoff modeled** (deterministic). Turns endless threshold-fiddling into one
computed optimum per channel.

### 6b.3 Metric B — timing resolution via the pulser as reference

The Ge-emulator pulser already provides a **trigger output** (the one leading the emulation
signal by 200 ns). Use it as the timing reference:

- emulation signal → Ge channel; pulser trigger output → a reference channel.
- Offline: replay CFD on the Ge channel, take `Δt = t_CFD(Ge) − t(ref)` per event, histogram
  → **σ_t = timing resolution**.
- The constant **200 ns offset is irrelevant — only the spread (jitter) matters** (the 200 ns
  cancels out of a jitter measurement).
- Caveat: measured `σ = √(σ_Ge² + σ_ref² + σ_pulser²)`. Keep the ref channel high-S/N (use the
  clean fast trigger pulse) and confirm pulser jitter ≪ digitizer resolution, so σ_ref /
  σ_pulser are negligible/boundable. Otherwise you measure the reference, not the Ge channel.

Sweep CFD {delay, fraction, smoothing} + the shared smoothing offline → timing-resolution
surface.

### 6b.4 Capture additions (Phase 0 for trigger)

- **Noise run** — random/software/pulser trigger, pure baseline → σ_TTF, false-rate.
  (Partly substitutable by the §6b.0 pre-trigger-region analysis, but a dedicated noise run
  tightens the statistics. Mechanism: pulser trigger output → TRG-IN forced recording,
  §4.5 item 3.)
- **Source run** — γ source → efficiency on real pulses + energy distribution (**reuse the
  §3 capture run as-is** — its low-threshold superset already covers the full candidate
  threshold range).
- **Timing run** — emulation→Ge ch + pulser-trigger→ref ch, both waveforms recorded → CFD
  jitter.

### 6b.5 Validation (same trust pattern as the trapezoid)

- SW trigger fires on the **same sample as D0 = the FW Trigger digital probe** (fixed on
  PHA1 — free ground truth on every event).
- SW CFD timestamp matches the **FW fine (CFD-interpolated) timestamp per event** — the FW
  already outputs the interpolated timestamp, a strong anchor.

### 6b.6 Diagnostic bonus

σ_TTF vs. the expected ENC triages **tunable electronic noise (→ smoothing helps)** vs.
**external pickup / ground loop / microphonics (→ no parameter helps, fix in hardware first)**.
If σ_TTF ≫ ENC, stop tuning and chase the hardware. Directly addresses the "lowering threshold
floods us with noise" frustration — it tells you whether the fix is even in parameter space.

---

## 7. Relationship to the other ELIADE blocks (not in scope here)

- **`start_delay` auto-cal:** common clock ⇒ zero drift, constant offset only ⇒ solvable in
  one analytic shot: `StartDelay_b = TS_b − min_b(TS_b)` (clock units, all ≥ 0; the measured
  timestamps reveal the daisy-chain order, no need to know cabling). `start_delay` already
  exists as a per-board config field (`src/config/digitizer.rs:801`, 0–4080 ns, written to
  `/par/start_delay`). Needs an 8-host aggregation coordinator (global across all 40 boards).
- **Topology management:** full vs. subset are **topology-tagged calibration tables**, not
  hand-maintained twin config files. Guard against applying the wrong table. A subset that
  keeps the physical chain intact reuses the full table; only a subset that changes the start
  path (e.g. the trailing V1730 without its upstream chain) needs its own calibration.
- **EB time offset:** kept **separate from `start_delay` by design** (HW origin alignment vs.
  software event-build offset). The PHA↔PSD constant (different trigger-point definitions /
  filter latencies) is derived **empirically from first-run data** — which requires a
  **common anchor in the first run** (e.g. pulser emulation signal fanned into one PHA ch +
  one PSD ch) so the EB has something to align against.

---

## 8. Concrete next steps

- [ ] **Finalize the Phase 0 capture spec:** fix max-rise-to-test → window length (current
      plan 20 µs) + pre-trigger; pick source (⁶⁰Co / ¹⁵²Eu); back-compute per-channel adc_min
      (incl. coarse_gain / fine_gain alignment).
- [ ] **Confirm dual-trace effective sampling** (interleave → ~8 ns/sample?) on hardware.
- [ ] **Revisit coarse_gain**: raise the ADC-range utilization of individual channels
      (currently ~12%; push toward X4). Confirm no clipping (incl. sum peak).
- [ ] **Record one validation run + one capture run** (start with 1 crystal on es2 and close
      the full loop ①–④ once).
- [ ] **`pha_trap_tune` bin skeleton:** read `.delila` (waveform + FW energy + FW params)
      (`dev-tools` feature, offline family).
- [ ] **From-scratch, stage-separated SW trap** (§4.4): the public recursion + per-stage
      traces. No AMax core reuse (it does not exist).
- [ ] **Phase 1 validation harness:** per-event residual + probe2 overlay + D1=Peaking window
      match + FWHM_SW vs FWHM_FW. Feed a pulser run (identical waveforms) first to speed up
      convergence (§4.5).
- [ ] **Pulser phase scan:** per flat-top candidate, measure the FW-energy spread over
      identical-amplitude pulses → detector-free direct observation of the trap × trigger
      cross-term (§4.5 item 2).
- [ ] **Phase 2 grid search + free-peak Gaussian fit** → per-channel config patch (deep on 2
      channels → narrow rollout, §5.3).
- [ ] **Phase 3 HW verify** loop + lock-in.
- [ ] **(Later) reader option to omit digital probes** from EventData — halves event size
      (for production after trigger validation is done; a few lines + a config flag).
- [ ] **Phase 5 capture:** noise run (random trigger) + timing run (emulation→Ge,
      pulser-trigger→ref).
- [ ] **SW RCCR2** (RC-CR² + smoothing) + LED/CFD discriminator, stage-separated; validate vs
      D0=Trigger probe + FW CFD timestamp (roll the trapezoid framework over).
- [ ] **Metric A** (S/N → min threshold) + **Metric B** (CFD jitter vs pulser ref).
- [ ] **Constrained opt:** min threshold s.t. timing ≤ window budget & false-rate ≤ budget;
      emit Pareto front + per-channel pick.

---

## 9. Research extensions (optional) — Bayesian / ML playground (2026-07-13 discussion)

**Premise: the deliverable of this TODO (grid + probe-overlay) does not depend on any of
this.** The §1/§5 conclusion that BO is unnecessary is unchanged (ms evals). These extensions
exploit the fact that the capture run **is already a training/analysis dataset**. In priority
order:

### 9.1 Hierarchical Bayes across the 32 crystals (~half a day; the principled form of §5.3)

The "same-type crystals have nearby optima" hypothesis IS a hierarchical model:
`per-channel optimum (rise, flat) ~ N(population mean μ, crystal-to-crystal variance τ²)`.
- Partial pooling: channels with weak data borrow strength from the ensemble (justifies the
  deep-2ch → narrow-rollout strategy)
- **A crystal whose posterior falls outside the population = free anomaly detection**
  (quantifies "this preamp/crystal is off")
- PyMC/Stan; input is just the grid results

### 9.2 Sequential Bayesian stopping rule for Phase 3 (saves real beam time — the one place
where Bayes does its native job)

Offline evals are cheap, but **Phase 3 hardware verification is acquisition-time-expensive
again**. Update the posterior of the low-statistics peak FWHM sequentially and stop the run
as soon as the credible interval decides agreement/disagreement with the SW prediction —
hundreds of counts instead of 10k, ×32 crystals.

### 9.3 GP surrogate (~zero effort garnish)

Fit a Gaussian Process to the grid → uncertainty-aware interpolation, diagnoses whether the
optimum sits on a ridge/boundary.

### 9.4 DL: challenging the trapezoid itself (the real playground; feeds the 2027 beam)

The trapezoid is only near-optimal **among linear filters** — it discards the per-event
charge-collection shape (rise time).
1. **Ballistic-deficit correction net** (small, safe): tiny MLP on (trap energy, rise-time
   features) → corrected energy. ML version of classic BD correction. **Lets the flat top
   shrink → pileup tolerance → beam rate.**
2. **End-to-end waveform → energy regression**: the label is the crux. Training against
   low-rate trapezoid energy caps you at imitation → a **spectral-sharpness loss**
   (differentiable width of known peaks in the histogram of predicted energies + linearity
   constraints) makes physics the teacher, with headroom beyond the trapezoid in principle.
   Literature scan (2026-07-13): no direct prior art of this exact form for HPGe (a gap);
   the physics-constraints-as-supervision framework of arXiv:2606.29466 (2026) is the
   closest relative.
3. **Pileup detection/unfolding CNN**: finer than the FW pileup flag; for 2027 rates.

**Traps**: ① beating the pulser is memorization (evaluate on source data + held-out runs);
② the baseline must be **BD-corrected trapezoid** (beating the plain trapezoid proves
little); ③ train/operate run drift (pair with per-run recalibration); ④ can't burn into FW →
positioning is "trapezoid = online, ML = offline precision layer".

### 9.5 Pulse-shape discrimination axes — the HPGe translation of Si-style particle ID
(added 2026-07-13)

Si-style PSA (FAZIA-type rise-time vs E for Z/A identification) assumes charged particles
enter from outside and stop in the active volume — that premise does not transfer to HPGe
(endcap + dead layer stop external particles; clovers are unsegmented coax crystals with
weak position sensitivity). **But HPGe has four discrimination axes of its own, and ② is
ELI-NP-specific value:**

1. **SSE/MSE → software Compton suppression**: the mainstream of HPGe PSD
   (GERDA/Majorana A/E). Translated to γ spectroscopy = suppress the Compton continuum by
   MSE-ness. ML demonstrations exist on BEGe (arXiv:2412.08750; CNN peak-to-Compton
   0.238→0.547 = NIM A 2024, §9.6). Acts as a "second Compton suppressor" filling the solid-
   angle gaps of the BGO shields. Caveat: coax crystals separate worse than point-contact.
2. **Neutron-event tagging (ELI-NP-specific, the prize)**: γ-beam operation implies an
   unavoidable (γ,n) neutron environment. Ge(n,n'γ) triangle peaks (596/692 keV sawtooth) +
   recoil give a distinctive "ultra-local high-density deposit + de-excitation γ" signature
   → waveform classification tags them → **in-beam background cleansing**. Training-label
   design is the open problem (neutron-source vs γ-source run contrast, weak labels from the
   triangle-peak region) — itself a research topic.
3. **Slow-pulse (surface / n+ layer) rejection**: cleans the low-energy tail = effective
   lineshape improvement. Same objective side as this TODO (resolution).
4. **Microphonics / HV-transient anomaly detection** (autoencoder): operational janitor.

**The one place Si-like physics survives**: in-crystal photonuclear reactions
Ge(γ,p)/(γ,α). The produced charged particle deposits ultra-locally at high density, where
the **plasma delay** known in Ge (heavy ions / fission fragments) shows up on the rising
edge → "internal-reaction event vs normal γ" discrimination is a cousin of Si-style PSA.
Niche, but a very ELI-NP-flavored internal-background study.

Infrastructure is fully shared with §9.4 (same capture run, CNN stack,
delila2root→PyTorch). 250 MS/s / 14-bit sampling is better than GERDA's PSD baseline
(100 MS/s) — no data-quality obstacle.

### 9.6 Literature anchors (scanned 2026-07-13)

**NN pulse-height extraction vs shaping filters (directly relevant):**
- Regadío et al., "Unfolding using deep learning and its application on pulse height
  analysis and pile-up management", NIM A 1005 (2021) 165403 — DNN unfolder with a
  **BD-aware delayed-output loss**
- Regadío et al., "Three topologies of deep neural networks for pulse height extraction",
  arXiv:2401.05109 (2024) — U-Net/GRU/attention compared; MMSE loss handles BD (δ[n−k]);
  CNN best at high noise / RNN cheap at low noise / attention marginal for 1D
- "Trapezoidal pile-up nuclear pulse parameter identification method based on deep learning
  transformer model", Radiat. Phys. Chem. (2022) — **pulse-height rel. error ~0.64% = 27%
  better than trapezoidal shaping**
- "Deep Learning Based Pile-Up Correction Algorithm for Spectrometric Data Under
  High-Count-Rate Measurements", Sensors 25 (2025) 1464 — 2D attention U-Net spectrum
  recovery
- "FPGA implementation of a deep learning algorithm for real-time signal reconstruction in
  radiation detectors under high pile-up conditions", arXiv:1903.02439 — edge-inference
  precedent

**HPGe waveform DL (PSD/discrimination dominates; energy regression is thin = the gap):**
- Holl et al., "Deep learning based pulse shape discrimination for germanium detectors",
  Eur. Phys. J. C 79 (2019) 450 / arXiv:1903.01462 — GERDA; autoencoder + classifier
- "A gamma-ray events discrimination method based on CNN in a HPGe spectrometer",
  NIM A (2024) — peak-to-Compton 0.238 → 0.547
- "Machine learning-assisted techniques for Compton-background discrimination in BEGe",
  EPJ C (2025) / arXiv:2412.08750
- "Efficient machine learning approach for optimizing the timing resolution of a HPGe
  detector", NIM A (2020) — SOM waveform clustering, γ-γ timing 4.3 ns @ 511 keV

**Self-supervised / physics constraints as the teacher:**
- "Self-Supervised Calibration of Scientific Instruments Using Physical Consistency
  Constraints", arXiv:2606.29466 (2026) — the conceptual neighbor of the §9.4-2 sharpness
  loss

---

## References

- `legacy/UM4380_725-730_DPP_PSD_Registers_rev6.pdf` — register map; Start Delay step = 16/32 ns
- `src/reader/decoder/pha1.rs` — PHA1 waveform decode (probe_0=Input, probe_1=Trapezoid;
  D0=Trigger fixed bit15 / D1 selectable bit14; digital probes expanded to unpacked u8/sample)
- `src/reader/decoder/psd1_pha1_common.rs:118` — `num_samples_wave: u16` (×8 = max ~524k samples)
- `src/recorder/format.rs` — `.delila` v3 (fixed field count; waveform arrays variable-length)
- `tools/delila2root/` — C++ converter (vector branches, no fixed-sample-count assumption)
- `src/reader/decoder/amax.rs` — **decoder** of AMax FW output (contains no SW trap —
  reuse plan withdrawn 2026-07-13)
- `src/config/digitizer.rs:801` — `start_delay` per-board config field
- CLAUDE.md — "Decoder hot-path heuristic policy" + "no silent failure" doctrine (e641e99, e45e0ec)
