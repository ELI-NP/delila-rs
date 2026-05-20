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

## Full-dataset run (205 files, 2026-05-20)

| metric              | value           |
|---------------------|-----------------|
| input files         | 205             |
| input hits          | 8,425,960,061   |
| L1 events built     | 2,045,229,984   |
| L2 events kept      | 643,643         |
| ROOT files written  | 2               |
| pipeline runtime    | 14 min 39 s     |

L1 → L2 ratio ≈ 0.031 % (driven by `Si_Both = E_Sector AND dE_Sector`).

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
ΔE-E PID orientation. The macro now emits:

| file | content |
|---|---|
| `si_e_de_kev.png`        | naive any+any pairing, calibrated keV. Two bananas visible, with a vertical stripe at E ≈ 8 MeV that is a cross-sector accidental (the 8.1 MeV peak in one mod 4 channel paired with random mod 0 hits). |
| `si_e_de_raw.png`        | same as above, raw ADC, zoomed. |
| `si_e_de_raw_full.png`   | full 0..65 k × 0..65 k. Saturation stripes at ADC ≈ 32 768 visible. |
| `si_e_de_paired_kev.png` | **anti-diagonal pairing** (mod0_X ↔ mod4_(15−X)), calibrated keV. Clean Bethe-Bloch banana. The 8 MeV stripe is gone. |
| `si_e_de_paired_raw.png` | anti-diagonal pairing, raw ADC. |
| `si_e_de_de_per_channel.png` | mod 4 dE spectrum vs ch. Shows the 8.2 MeV peak lives on multiple channels (mostly odd ch → even sectors after `GetSector`). |
| `si_e_de_e_per_channel.png`  | mod 0 dE spectrum vs ch. |

## Known limitations / next iteration

1. **L2 lacks positional correlation** — current `Si_Both` only requires
   *any* mod 0 hit + *any* mod 4 hit. Real ΔE-E coincidence wants the
   anti-diagonal (mod0_X ↔ mod4_(15−X)) pairing. The analysis macro
   already filters this way; could be pushed into a new L2 op so the
   filter happens in the EB pipeline itself.
2. **No L1 energy gate** — noise hits in both layers pass within 100 ns
   accidentally, inflating the low-energy population in the 2D plot.
   Add `{"type": "energy_gate", ...}` op around each trigger channel.
3. **No timeSettings.json** — every channel runs with offset = 0. Run
   `event_builder time-calib` to produce one and re-run the EB for the
   strictest coincidence check.
4. **Multiplicity ≥ 2 not filtered** — events where multiple sectors
   fire produce multiple anti-diagonal pairs per event. `ring_ring.cpp`
   uses `dECounter == 1 || eCounter == 1` to suppress this; could be
   replicated in the macro for the cleanest banana.
