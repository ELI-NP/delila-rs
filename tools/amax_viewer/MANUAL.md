# AMax Viewer — User Manual

A firmware development tool for AMax DPP. Provides real-time waveform display, register parameter tuning, a 2D AMax-vs-Energy histogram, and ROOT file output.

---

## Build

```bash
# From the delila-rs repository root
cd tools/amax_viewer
cargo build --release
```

Binary: `target/release/amax_viewer`

---

## Launch

```bash
# Basic (uses saved or embedded register definitions)
cargo run --release --bin amax_viewer
./target/release/amax_viewer

# With a specific register definitions file
./target/release/amax_viewer registers/register_20260310.json

# Test Pulse mode
./target/release/amax_viewer -t
./target/release/amax_viewer registers/register_20260310.json -t

# Force-reset all parameters to defaults (useful after firmware change)
./target/release/amax_viewer --reset-params
./target/release/amax_viewer registers/register_new_fw.json --reset-params
```

On the first run, configuration files are created automatically (see [Settings Persistence](#settings-persistence)).

### Firmware Change Auto-Detection

When switching firmware, the register definition file changes. The viewer automatically detects this by comparing a hash of the loaded `register_defs.json` content with the hash stored in `settings.json`. If the hash differs:

1. All saved parameter values are cleared (reset to `register_defs.json` defaults)
2. On the next acquisition start, **all registers are force-written** to the digitizer (no diff-write optimization)
3. A status message "Parameters reset to defaults (firmware change detected)" is displayed

This prevents stale parameters from a previous firmware from corrupting the new firmware's state.

If auto-detection does not trigger (e.g., same definition file with updated defaults), use `--reset-params` to force a manual reset.

---

## UI Layout

```
┌─────────────────┬──────────────────────────────────┐
│   Side Panel    │                                  │
│  (scrollable)   │     AMax vs Energy  (2D hist)    │
│                 │                                  │
│  Connection     │                                  │
│  Parameters     │                                  │
│  Histogram Range│                                  │
│  Bin Settings   ├──────────────────────────────────┤
│  ROOT Output    │        Waveform (latest)          │
└─────────────────┴──────────────────────────────────┘
```

---

## Side Panel

### Connection

| Element | Description |
|---------|-------------|
| URL field | Digitizer connection URL. Example: `dig2://172.18.4.56` |
| **Start** | Connect to the digitizer, enable ch0 only, write and verify all registers, then start acquisition |
| **Stop** | Stop acquisition (`disarmacquisition` is sent, then the thread exits) |
| **Clear** | Reset the histogram (recorded buffer is retained) |
| Status | Shows connection state, errors, and register apply results |
| Events | Cumulative event count in the histogram |
| Rate | Event rate over the last 1 second [Hz] |

### Parameters

Generated dynamically from `register_defs.json`. Registers are grouped under section headings (Core / AMax).

Each row: `RegisterName: [DragValue]`

- **DragValue**: drag or type to change the value. Range is clamped to `min`/`max` from the JSON.
- Changes are held in memory and **not yet written** to the digitizer.

> **To apply changes**: click **Restart to Apply** (or press **Enter**).
> This performs Stop → write registers → Start automatically.

### Histogram Range

| Parameter | Default | Description |
|-----------|---------|-------------|
| Energy Max | 65536 | Upper bound of the X axis (Energy) |
| AMax Max | 16384 | Upper bound of the Y axis (AMax = user_info[0]) |

Changes take effect immediately without restarting.

### Bin Settings

| Parameter | Default | Range | Description |
|-----------|---------|-------|-------------|
| Energy Bins | 512 | 16–4096 | X axis resolution |
| AMax Bins | 512 | 16–4096 | Y axis resolution |

Changing bin counts clears the histogram.
The current bin width (= Range / Bins) is shown for reference.

### ROOT Output

| Element | Description |
|---------|-------------|
| File field | Output ROOT file path (e.g. `amax_data.root`) |
| **Record** checkbox | When checked, events are accumulated in memory. Unchecking does not stop acquisition |
| Event count | Number of events currently in the buffer |
| **Save ROOT File** | Appears when stopped and buffer is non-empty. Writes buffer to file and clears it |

---

## Main Panel (2D Histogram)

- X axis: Energy (OpenDPP `ENERGY` field)
- Y axis: AMax (`USER_INFO[0]` — slot 1 of the Sci-Compiler CAEN LIST 2)
- Color scale: blue (low) → cyan → yellow → white (high), sqrt-normalized

Zoom and pan with standard egui_plot controls (drag, scroll).
Double-click to reset the view.

---

## Waveform Panel (bottom)

Displays the ADC waveform of the most recent event, updated every 100 ms.
The title shows `(Energy: N)` for that event.
Drag the top edge of the panel to resize it.

---

## Typical Workflow

```
1. Enter the digitizer URL
2. Set register values in the Parameters section
3. Click Start
4. Inspect the waveform and histogram
5. Adjust parameters → click Restart to Apply (or press Enter)
6. To save to ROOT:
   a. Check Record
   b. Collect data
   c. Click Stop
   d. Click Save ROOT File
7. URL, output path, and all register values are saved automatically on exit
```

---

## Register Definition File (register_defs.json)

### File Location

Register definitions are loaded in the following priority order:

| Priority | Source | Example |
|----------|--------|---------|
| 1 — CLI argument | First positional argument | `./amax_viewer registers/register_20260310.json` |
| 2 — user config | `~/.config/amax_viewer/register_defs.json` (Linux)<br>`~/Library/Application Support/amax_viewer/register_defs.json` (macOS) | — |
| 3 — fallback | Default embedded in the binary at compile time | — |

- **CLI argument**: Any path (absolute or relative). The loaded file count is printed to stderr on startup.
- **User config**: On the first run, if this file does not exist, the embedded default is copied there automatically. Edit freely.
- **Fallback**: Used if both above are unavailable or contain parse errors.

### Format

```json
[
  {
    "section": "Core",
    "name": "THRS",
    "address": 2,
    "min": 0,
    "max": 16383,
    "default": 100
  },
  {
    "section": "AMax",
    "name": "WINDOW_MAXIM",
    "address": 81920,
    "min": 0,
    "max": 4095,
    "default": 200
  }
]
```

| Field | Description |
|-------|-------------|
| `section` | UI section heading. Registers with the same section name are grouped together |
| `name` | Register name — used as the UI label and as the HashMap key |
| `address` | **Word address** (the CAEN API receives `address × 4` as the byte address) |
| `min` / `max` | DragValue input range |
| `default` | Initial value on first load (overridden by any previously saved value) |

> **Note**: Use the same word addresses as in the Sci-Compiler `RegisterFile.json`.
> The tool multiplies by 4 internally before calling `CAEN_FELib_SetUserRegister`.
>
> **WARNING**: Old firmware addresses (0x0–0x30) are invalid on new page-based firmware.
> Writing to them returns no error but **corrupts the firmware's internal state**,
> causing TestPulse event generation to stall. Always use `gen_defs` to generate
> the correct addresses from the new firmware's `RegisterFile.json`.

### Adding or Modifying Registers

1. Edit the JSON file directly (either the CLI-specified file or the user config file).
2. Restart the application — the UI rebuilds from the updated JSON.

### Managing Multiple Firmware Versions

Keep separate files per firmware version and specify the appropriate one on launch:

```bash
registers/
├── register_opendpp_v1.json     # Old FW
├── register_20260310.json       # New FW (32ch_4input)
└── register_experimental.json   # Testing
```

```bash
./target/release/amax_viewer registers/register_20260310.json
```

---

## gen_defs Tool (RegisterFile.json → register_defs.json)

Converts Sci-Compiler's `RegisterFile.json` into amax_viewer's register definition format.
Use this whenever the firmware is updated and the register map changes.

### Input: RegisterFile.json

Sci-Compiler outputs `RegisterFile.json` in its project's `output/output/` directory.
The file contains a `Registers` array with `Name` and `Address` fields:

```json
{
  "Registers": [
    { "Name": "page_amax_energy_0_THRS", "Address": 8388612, ... },
    { "Name": "page_amax_energy_0_WINDOW_MAXIM", "Address": 8388629, ... }
  ]
}
```

### Usage

```bash
# Basic: generate with default values (requires manual editing)
cargo run --release --bin gen_defs -- \
  path/to/RegisterFile.json \
  -o registers/register_YYYYMMDD.json

# With parameter table: auto-apply bit widths, defaults, and readonly flags
cargo run --release --bin gen_defs -- \
  path/to/RegisterFile.json \
  -p fw_params.json \
  -o registers/register_YYYYMMDD.json
```

| Option | Description |
|--------|-------------|
| `<RegisterFile.json>` | Sci-Compiler register file (required, positional) |
| `-p <fw_params.json>` | Firmware parameter table (bit widths, defaults, readonly patterns) |
| `-o <output.json>` | Output file path (default: `register_defs.json`) |
| `-h` / `--help` | Show usage |

### Section Auto-Detection

Each register is automatically classified into a UI section:

| Section | Rule |
|---------|------|
| **AMax** | Address ≥ 1,441,792 (0x160000), or name starts with `AMAX` / `baseline`, or name is `WINDOW_MAXIM` |
| **Core** | Everything else |

You can change the section names by editing the output JSON.

### Without `-p`: Manual Editing Required

The generated file has `min=0, max=4294967295, default=0` for **all** registers.
You must edit these values manually before use:

```json
{
  "section": "Core",
  "name": "THRS",
  "address": 2,
  "min": 0,           ← set appropriate minimum
  "max": 16383,        ← set appropriate maximum (e.g. 14-bit = 16383)
  "default": 100       ← set a reasonable startup value
}
```

### With `-p`: Firmware Parameter Table (Recommended)

The parameter table (`fw_params.json`) encodes firmware-specific knowledge: bit widths,
default values, and readonly patterns. This eliminates manual post-editing.

#### Parameter Table Format

```json
{
  "params": {
    "POLARITY":    { "bits": 1,  "default": 1 },
    "THRS":        { "bits": 32, "default": 20 },
    "TRAP_K":      { "bits": 16, "default": 500 },
    "DECONV_M":    { "bits": 24, "default": 3499000 },
    "AMAX_window": { "bits": 32, "default": 1000 }
  },
  "readonly_patterns": [
    "ENERGY_MAIN",
    "AMAX_MAIN",
    "debug_amax_out",
    "READ_ENERGY",
    "maxim_outt"
  ]
}
```

| Field | Description |
|-------|-------------|
| `params` | Map of register keyword → `{bits, default}`. Keywords are matched as **substrings** of register names (e.g. `"THRS"` matches `page_amax_energy_0_THRS`, `page_amax_energy_1_THRS`, etc.). When multiple keywords match, the **longest** match wins |
| `params[].bits` | Number of data bits. Used to compute `max = 2^bits - 1` (e.g. 16 bits → max 65535) |
| `params[].default` | Default value for this register. Validated: error if `default > max` |
| `readonly_patterns` | List of substrings. Any register name containing one of these patterns is marked `readonly: true` |

#### Creating a Parameter Table for a New Firmware

1. Open the firmware source or documentation
2. For each register, determine:
   - The base name (without channel prefix, e.g. `THRS` not `page_amax_energy_0_THRS`)
   - The number of data bits
   - A reasonable default value
3. Add status/output registers to `readonly_patterns`
4. Save as `fw_params.json` and pass to `gen_defs` with `-p`

> **Note**: The parameter table is firmware-family-specific (shared across Trapezoid_simple,
> Trapezoid_2channels, Trapezoid_32channels if they use the same register names).
> Only update it when register names or bit widths change.

A reference parameter table is included: [`fw_params.json`](fw_params.json).

### Complete Workflow (New Firmware)

```
┌──────────────────┐     ┌──────────────┐     ┌──────────────────┐
│  Sci-Compiler    │     │  fw_params   │     │   amax_viewer    │
│  RegisterFile.json├────►│  .json       ├────►│  register_defs   │
│  (addresses)     │     │  (bits/defs) │     │  .json (complete)│
└──────────────────┘     └──────────────┘     └──────────────────┘
        gen_defs -p merges both sources
```

```bash
# 1. Flash new firmware to the digitizer
#    (using CAEN Upgrader or Sci-Compiler)

# 2. Generate register definitions with parameter table
cargo run --release --bin gen_defs -- \
  path/to/new_fw/output/output/RegisterFile.json \
  -p fw_params.json \
  -o registers/register_YYYYMMDD.json

# Output example:
# Wrote 54 register definitions to registers/register_YYYYMMDD.json (6 readonly)
# Parameter table applied: 48 matched, 0 unmatched

# 3. Launch amax_viewer with the new definitions
./target/release/amax_viewer registers/register_YYYYMMDD.json

# → "Register definitions changed (firmware update detected). Parameters reset to defaults."
# → All registers are force-written to hardware on first Start
```

#### Switching Between Firmware Versions

Keep separate register definition files per firmware version:

```
registers/
├── register_trapezoid_simple.json    # 1-channel
├── register_trapezoid_2ch.json       # 2-channel (current)
└── register_trapezoid_32ch.json      # 32-channel
```

```bash
# Switch firmware
./target/release/amax_viewer registers/register_trapezoid_32ch.json

# The viewer detects the register definition change automatically:
# → parameters are reset to defaults
# → all registers are force-written to the digitizer
```

No manual cleanup of `settings.json` or `~/.config` files is needed.
If auto-detection does not trigger, use `--reset-params`:

```bash
./target/release/amax_viewer registers/register_trapezoid_32ch.json --reset-params
```

---

## ROOT File Structure

Tree name: `events`

| Branch | Type | Content |
|--------|------|---------|
| `channel` | Int_t | Channel number (typically 0) |
| `energy` | Int_t | Energy from the trapezoid filter (14–16 bit) |
| `timestamp` | Long64_t | Timestamp [ns] |
| `fine_timestamp` | Int_t | Fine timestamp (interpolated resolution) |
| `flags_a` | Int_t | FLAGS_A (peak search status, etc.) |
| `flags_b` | Int_t | FLAGS_B (over-range, etc.) |
| `psd` | Int_t | PSD value (unused in AMax FW = 0) |
| `user_info_0` | Long64_t | CAEN LIST 2 slot 1 (AMax value) |
| `user_info_1` | Long64_t | CAEN LIST 2 slot 2 |
| `user_info_2` | Long64_t | CAEN LIST 2 slot 3 |
| `user_info_3` | Long64_t | CAEN LIST 2 slot 4 |

### Reading with Python (uproot)

```python
import uproot
import numpy as np

with uproot.open("amax_data.root") as f:
    t = f["events"]
    print(t.keys())

    energy = t["energy"].array(library="np")
    amax   = t["user_info_0"].array(library="np")
    print(f"{len(energy)} events, energy mean = {energy.mean():.1f}")
```

---

## Settings Persistence

On exit, the following are saved automatically:

| Item | Location |
|------|----------|
| URL | `~/.config/amax_viewer/settings.json` |
| Output file path | same |
| Register values (non-default only) | same |
| Register definitions hash | same |

These are restored on the next launch.

### Firmware Change Detection

The hash of the loaded `register_defs.json` content is stored in `settings.json`.
On the next launch, if the current `register_defs.json` hash differs from the stored hash:

1. All saved register values are discarded
2. Registers are initialized from the new `register_defs.json` defaults
3. All registers are force-written to hardware (bypassing diff-write)

This prevents stale parameters from a previous firmware from being applied to new hardware.

### Embedded Default Auto-Update

When the binary is recompiled with an updated `register_defs.json`:

- If the user relies on `~/.config/amax_viewer/register_defs.json` (no CLI argument),
  the config file is automatically updated to match the new embedded default
- This only happens when the embedded default differs from the config file

---

## Troubleshooting

| Symptom | Cause / Action |
|---------|----------------|
| `Connection failed` | Wrong URL, or digitizer is powered off / unreachable |
| `Init: N err, M mismatch` | One or more registers failed to write or read back correctly. The first failure is shown in the status bar. Check for unsupported addresses in `register_defs.json` |
| Histogram remains empty | No triggers arriving. Check THRS, POLARITY, and cabling. Verify waveform panel shows a signal |
| ROOT file is empty | **Record** was not checked during acquisition |
| Parameter changes have no effect | Must press **Restart to Apply** or **Enter** to re-write registers |
| `register_defs.json` load error on startup | JSON syntax error. Fix or delete the user config file. The embedded default will be used as a fallback |
| TestPulse stops generating events | Old register addresses (0x0–0x30) may have corrupted FW state. Power-cycle the digitizer or send `/cmd/reset`, then use correct page-based addresses |
| Parameters from previous FW persist | Normally auto-detected via hash. If not, use `--reset-params` to force-clear saved parameters |
| `gen_defs` shows "N unmatched" | The parameter table (`fw_params.json`) is missing entries for some registers. Add the missing keywords to the `params` section |
| CLI register file not loading | Check stderr for error messages. The path is relative to the working directory |
