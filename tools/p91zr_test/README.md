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

### Output PNGs

The macro emits three views of the Si E vs dE 2D:

| file | content |
|---|---|
| `si_e_de_raw.png`      | raw ADC, **zoomed to the physics region** (0..10 k × 0..6 k). The two clear bananas (heavy / light charged particles) live here. |
| `si_e_de_raw_full.png` | raw ADC, full 0..65 k × 0..65 k. The physics is squashed into the bottom-left corner; horizontal/vertical stripes at ADC ≈ 32 768 are channels saturating their 15-bit ADC. |
| `si_e_de_kev.png`      | calibrated using `chSettings.p0..p3`. Same shape as raw, axis label keV. |

## Known limitations / next iteration

1. **L2 lacks positional correlation** — current `Si_Both` only requires
   *any* mod 0 hit + *any* mod 4 hit. Real ΔE-E coincidence wants matching
   sectors. Either extend L2 with a "same-channel" op, or filter at the
   analysis stage.
2. **No L1 energy gate** — noise hits in both layers pass within 100 ns
   accidentally, inflating the low-energy population in the 2D plot.
   Add `{"type": "energy_gate", ...}` op around each trigger channel.
3. **Raw ADC** in the macro — `chSettings.p0..p3` calibration not yet
   applied at the analysis stage.
4. **No timeSettings.json** — every channel runs with offset = 0. Run
   `event_builder time-calib` to produce one and re-run the EB.
