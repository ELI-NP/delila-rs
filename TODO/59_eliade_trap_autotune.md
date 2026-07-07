# TODO 59 — ELIADE PHA Energy-Resolution Auto-Tune (SW Trapezoid Replay)

**Status: 📋 PLANNING (2026-06-16)**
**Owner:** Aogaki + Claude
**Experiment:** ELIADE (8× Clover HPGe array, ELI-NP)
**Target timeline:** Bench tests start June 2026 · Ge resolution tune-up through 2026 · Beam from Jan 2027

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

---

## 2. Pipeline overview

```
Phase 0  Capture (once, on HW)   : Tuneup mode, probe1=Input, waveform ON + FW energy ON,
                                    known γ source, long-enough waveform window.
Phase 1  SW-trap validation      : replay identical waveforms with the FW's own params,
                                    compare PER-EVENT vs FW energy. This is the trust anchor.
Phase 2  Offline optimization    : grid-search params over the same stored waveforms,
                                    minimize FWHM(keV) per channel. Pure CPU.
Phase 3  HW verification         : burn optimal params into FW, short run, confirm real FWHM
                                    matches the SW prediction. Lock into config.
```

All of Phases 1-2 live in **one offline tool**: a new bin `pha_trap_tune` that reads a
`.delila` file (waveforms + FW energy) and emits a per-channel config patch.

---

## 3. Phase 0 — Capture (constraints that cannot be fixed later)

We confirmed (Aogaki) that the digitizer **can emit waveform + FW-computed energy in the
same event** in Tuneup mode. Set **`analog_probe1 = Input`** so probe1 carries the raw ADC
preamp signal (this is the SW-trap input). `analog_probe2 = Trapezoid` gives the FW's own
intermediate trapezoid trace for sample-level cross-check in Phase 1.

**Hard constraints — get these right at capture time, they cannot be recovered offline:**

1. **Waveform window length ≥ longest shaping you will test.** To sweep rise times up to
   ~8 µs the record must contain baseline + full trapezoid response ≈ `2·(rise+flat) +
   decay`. At 250 MS/s (4 ns/sample) an 8 µs rise needs a long record. Decide the **maximum
   rise to be tested** before capturing and size the window from it. Too-short windows make
   long-shaping evals invalid (truncated response) — silently wrong FWHM.
2. **Sufficient pre-trigger baseline.** The baseline restorer averages pre-pulse samples;
   the window must include enough of them.
3. **Peak statistics.** A stable Gaussian FWHM fit wants **~5k–10k counts in the peak per
   channel.** Source: ⁶⁰Co (1173/1332 keV) or ¹⁵²Eu (multi-line). Capture enough events.

Output: one `.delila` file per channel-group with `{raw input waveform, FW energy, FW
params used, trapezoid probe}` per event.

---

## 4. Phase 1 — SW-trap validation (the part that must be rigorous)

### 4.1 Reuse strategy

The AMax custom FW is itself a trapezoidal-filter MCA and we already have its trap logic;
the developer confirmed **AMax-FW energy ↔ PHA-FW energy are linearly 1:1**. So **reuse the
AMax trapezoid core** as the SW-trap skeleton.

⚠️ **"Linear 1:1 energy correspondence" is NOT sufficient evidence for resolution work.**
A linear correspondence only means the *centroid/gain* matches between two black-box
firmwares. **FWHM is set by the filter's noise-transfer characteristic** (trapezoid
weighting, baseline restorer, exact rise/flat-top in samples, fixed-point rounding,
energy-extraction window). Two filters can have linearly-correlated centroids yet different
FWHM. The constant gain factor between AMax and PHA **is** safely ignorable (resolution is
relative; we re-fit the peak and re-calibrate to keV each eval) — but the surrounding stages
are not.

### 4.2 Validation criterion

Run the SW trap on the captured waveforms **with the exact params the FW used**, then:

- Per-event residual `SW_energy − FW_energy`; require its std **≪ peak FWHM (ideally ±1
  LSB)**. **Use per-event residual, NOT a linear-fit R².** Per-event agreement guarantees
  FWHM reproduction; R² does not.
- Overlay **SW trapezoid trace vs. decoded `analog_probe2`** sample-by-sample (FW exposes
  its own intermediate trapezoid — a strong ground truth).
- Decisive check at the FW operating point: **`FWHM_SW ≈ FWHM_FW`**. Only when FWHM (not
  centroid) matches is the model trustworthy for optimization.

This is exactly the CLAUDE.md doctrine ("spec-page reference + real-hardware verification;
no silently-wrong resolution" — cf. the e641e99 heuristic and e45e0ec silent-cache
incidents). **No optimization until Phase 1 passes.**

### 4.3 Trapezoid math anchor (Jordanov-Knoll; matches UM4380 + AMax impl)

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

### 4.4 Module structure — *the actual content of step "(ii)"*

"(ii)" is **not** heavy API design and **not** a separate topic. It is simply: **implement
the SW trap as separable, inspectable stages so that when Phase 1 fails we can localize which
stage diverges** instead of debugging a monolith. PHA energy processing decomposes as:

| Stage | Content | Affects FWHM? | Reuse plan |
|-------|---------|:---:|------------|
| 1 | Input (probe1=Input raw waveform) | — | as-is |
| 2 | Baseline computation / restorer (subtract pre-pulse mean) | **yes** | AMax has one — **verify vs 725 PHA** |
| 3 | Trapezoid recursion (d,p,r,s with PZ M) | yes (shaping) | **reuse AMax core ✓** |
| 4 | Energy extraction (sample at peaking position, average over Npk) | **yes** (noise averaging) | AMax has one — **verify vs 725 PHA** |
| 5 | Gain normalization → LSB | — | **ignore (linear factor, Aogaki correct)** |

FWHM is decided more by stages **2 and 4** (baseline + energy window) than by the trapezoid
itself. If AMax's stages 2/4 match 725 PHA, dropping the whole AMax trap in passes Phase 1
on the first try — best case, "(ii)" is ~zero work. If they differ (baseline droop, peak
position), the stage decomposition lets us pinpoint and fix only the divergent stage
("trapezoid matches probe2, but energy doesn't → it's stage 4's window").

**Pragmatic order:** drop the AMax trap in whole → run Phase 1 → if it passes, go straight to
Phase 2; if not, fix the single offending stage. Keeping per-stage intermediate traces
exposed is the only "design" requirement.

---

## 5. Phase 2 — Offline optimization (grid is enough)

### 5.1 Search-space pruning with physics priors

- **Pole-zero M:** measured from waveform (§4.3) → fixed, not searched.
- **Trigger threshold:** affects efficiency/noise floor, not resolution at a strong peak →
  fixed sensibly.
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
- **BO is unnecessary** (the original motivation, online acquisition cost, is gone). Reserve
  it only if dimensionality stays high for some reason.

### 5.3 FWHM metric — avoid the moving-peak trap

Changing rise time changes the **gain**, so the **peak centroid moves** with the params. The
fitter must **find the peak freely every eval** (Gaussian + linear background), map the
centroid to the known energy (e.g. 1332 keV), and report **FWHM in keV**. Minimizing FWHM at
a *fixed* channel window would be wrong. (For the inner loop a robust half-max width estimate
is fine; do the full Gaussian fit for final reporting.)

### 5.4 Output

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

Reuses the **same offline-replay harness**: the trigger path (TTF = RC-CR² + smoothing, then
LED/CFD discrimination) is a deterministic function of the input waveform, exactly like the
trapezoid. Config plumbing partly exists already (`ttf_smoothing`, `trigger_edge`,
`trigger_threshold_v` from V1743 work).

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

- **Noise run** — random/software/pulser trigger, pure baseline → σ_TTF, false-rate. Without
  this, false triggers cannot be measured (source-triggered waveforms are selection-biased —
  they lack the noise events a lower threshold would create).
- **Source run** — γ source → efficiency on real pulses + energy distribution (reuse the
  energy-tune capture).
- **Timing run** — emulation→Ge ch + pulser-trigger→ref ch, both waveforms recorded → CFD
  jitter.

### 6b.5 Validation (same trust pattern as the trapezoid)

- SW trigger fires on the **same sample as the FW Trigger digital probe**.
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

- [ ] **Phase 0 capture spec:** fix max-rise-to-test → window length + pre-trigger; pick
      source (⁶⁰Co / ¹⁵²Eu); set counts/peak. Record one validation `.delila`.
- [ ] **`pha_trap_tune` bin skeleton:** read `.delila` (waveform + FW energy + FW params).
- [ ] **SW trap, stage-separated** (§4.4): drop in AMax core, expose per-stage traces.
- [ ] **Phase 1 validation harness:** per-event residual + probe2 overlay + FWHM_SW vs FWHM_FW.
- [ ] **Phase 2 grid search + free-peak Gaussian fit** → per-channel config patch.
- [ ] **Phase 3 HW verify** loop + lock-in.
- [ ] **Phase 5 capture:** noise run (random trigger) + timing run (emulation→Ge, pulser-trigger→ref).
- [ ] **SW TTF** (RC-CR² + smoothing) + LED/CFD discriminator, stage-separated; validate vs
      FW Trigger probe + FW CFD timestamp.
- [ ] **Metric A** (S/N → min threshold) + **Metric B** (CFD jitter vs pulser ref).
- [ ] **Constrained opt:** min threshold s.t. timing ≤ window budget & false-rate ≤ budget;
      emit Pareto front + per-channel pick.

---

## References

- `legacy/UM4380_725-730_DPP_PSD_Registers_rev6.pdf` — register map; Start Delay step = 16/32 ns
- `src/reader/decoder/amax.rs` — AMax trapezoidal MCA (reuse source for SW trap core)
- `src/reader/decoder/pha1.rs` — PHA1 waveform decode (`analog_probe1` = Input, `analog_probe2` = Trapezoid)
- `src/config/digitizer.rs:801` — `start_delay` per-board config field
- CLAUDE.md — "Decoder hot-path heuristic policy" + "no silent failure" doctrine (e641e99, e45e0ec)
