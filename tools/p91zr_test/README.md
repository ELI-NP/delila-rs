# p91Zr — end-to-end EB validation

End-to-end smoke test of the delila-rs EB on the ELIFANT2025 p91Zr data set.

## Inputs

- **Raw data:** `/Users/aogaki/WorkSpace/ELIFANT2025/p91Zr/data/run0113_*.root`
  (214 files, ELIADE_Tree, ~373 MB each — already time-sorted upstream).
- **Reference configs:**
  `/Users/aogaki/WorkSpace/ELIFANT2025/ELIFANT-Event/all_run_p91Zr/`
  (chSettings.json, L2Settings.json, settings.json).

## Workflow

1. **Convert ELIFANT configs → delila-rs SPEC v0.5.1 form:**

   ```bash
   python3 convert_elifant_config.py --out-dir .
   ```

   Generates:
   - `chSettings.json` — slim (ID/Module/Channel/DetectorType/Tags/p0..p3)
   - `eb_config.json`  — timing + L1 or-of-80-trigger-channels + L2
     (Counter/Flag/Accept chain that requires both `E_Sector` and
     `dE_Sector` hits).

   `timeSettings.json` is **not** generated here — produce it later from
   the delila-rs `event_builder time-calib` subcommand once we have an EB
   pass we trust.

2. **Run the EB on raw ROOT input:**

   ```bash
   cargo build --release --features root --bin event_builder

   ../../target/release/event_builder build \
       -o ./eb_output \
       -c ./chSettings.json \
       --eb-config ./eb_config.json \
       --run-id 113 \
       --root-input \
       -i /path/to/run0113_*.root
   ```

   Useful options: `--workers N`, `--writers N`, `--events-per-file N`.

3. **Analyse:**

   ```bash
   root -l -b -q 'analyse_si_e_de.C("eb_output", "si_e_de.root")'
   ```

   Produces:
   - `si_e_de.root` — multiplicity / per-detector ADC spectra / Si E vs dE 2D
   - `si_e_de.png` — quick visual of the 2D plot

## Full-dataset runs (205 files, 2026-05-20)

Three EB passes were made on the same input, each tightening the
configuration. The biggest jump came from time calibration; L1 energy
gate added a small refinement on top.

| pass | timeSettings | L1 gate `min_adc` | L2 events kept | mult==1 paired | notes |
|------|--------------|-------------------|----------------|----------------|-------|
| v1   | none         | 0                 | **643,643**    | **18,210**     | initial — 95 % of real coincidences missed because of channel-level timing offsets |
| v2   | tree (ELIFANT) | 0               | **12,399,990** | **150,721**    | ×19 in kept events; clean Bethe-Bloch banana emerges in the mult==1 view |
| v3   | tree         | 100               | **12,392,064** | **153,731**    | noise-floor trim, marginal change at this threshold |

### Setup (per the user, 2026-05-20)

- **2 telescope** configuration (Det A, Det B).
- **mod 0 = dE (Det A front sectors), mod 4 = E (Det B front sectors)** —
  follow the `ring_ring.cpp` variable naming, not the `chSettings.json`
  tags (which use the opposite labels and are misleading).
- **Geometric correspondence: mod0 ch X ↔ mod4 ch (15 − X)**
  (an anti-diagonal in the sector-vs-sector plot, confirming back-to-back
  2-body kinematics). The naive "any mod 0 + any mod 4" pairing also
  produces cross-sector accidentals, so the anti-diagonal cut is needed
  to clean up the PID banana.

### Output PNGs

Convention throughout: **X = E (mod 4), Y = dE (mod 0)** — the canonical
ΔE-E PID orientation. The macro emits, in increasing cleanliness:

| file | filter stack | entries (v3) |
|---|---|---|
| `si_e_de_raw_full.png`        | naive any+any, full 0..65 k × 0..65 k. Saturation stripes at ADC ≈ 32 768 visible. | 12.4 M |
| `si_e_de_raw.png` / `si_e_de_kev.png` | naive any+any, zoomed/calibrated. Multiple bananas + vertical stripe at E ≈ 8 MeV (cross-sector accidental between the 8.1 MeV mod-4 peak and random mod-0 hits). | 12.4 M |
| `si_e_de_paired_raw.png` / `si_e_de_paired_kev.png` | **anti-diagonal pairing** (mod0_X ↔ mod4_(15−X)). Single Bethe-Bloch banana, 8 MeV stripe gone. | 814 k |
| `si_e_de_paired_mult1_raw.png` / `si_e_de_paired_mult1_kev.png` | anti-diag **+ mult == 1** per telescope. The cleanest PID — single-particle events, no sector ambiguity. | 154 k |
| `si_e_de_de_per_channel.png`  | dE_ch (mod 0) vs ADC. Diagnostic — shows the per-channel structure that drove the v1 "stripe" mystery. |  |
| `si_e_de_e_per_channel.png`   | E_ch  (mod 4) vs ADC. Same diagnostic for the other telescope. |  |

## Known limitations / next iteration

All four items from the initial run-through have landed (v2/v3) **except**
the EB-side anti-diagonal cut, which SPEC § 1.4 explicitly forbids —
positional pairing is experiment-specific physics, not a generic EB filter.

Still open:

1. **Higher trigger energy gate** — v3 uses `min_adc = 100`. Empirically
   makes almost no difference because the mult==1 + anti-diagonal cuts
   already remove the lowest-energy noise. A higher value (~500 ADC,
   ~500 keV) might cut more, at the risk of losing real low-energy
   particles. Physics judgement call.
2. **Same-telescope ΔE-E** — what's currently labelled "Si E vs dE" is
   actually a 2-body kinematic correlation between two telescopes' front
   sectors. The single-telescope PID would be `(mod 0|4) × (rings of the
   matching back module)` — needs the ring map from `ring_ring.cpp` to
   be ported.
3. **Inter-telescope ring-ring correlation** — the `histRingRing[i][j]`
   plots in ELIFANT's output show clear kinematic ridges between specific
   ring pairs. Reproducing those would close the loop.
