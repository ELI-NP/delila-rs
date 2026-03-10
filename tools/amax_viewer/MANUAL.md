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
```

On the first run, configuration files are created automatically (see [Settings Persistence](#settings-persistence)).

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
    { "Name": "THRS", "Address": 2, ... },
    { "Name": "WINDOW_MAXIM", "Address": 81920, ... }
  ]
}
```

### Usage

```bash
# From the repository root
cargo run --release --bin gen_defs -- \
  path/to/RegisterFile.json \
  registers/register_20260310.json

# Output:
# Wrote 27 register definitions to registers/register_20260310.json
# Edit min/max/default values as needed before using.
```

The second argument (output path) defaults to `register_defs.json` if omitted.

### Section Auto-Detection

Each register is automatically classified into a UI section:

| Section | Rule |
|---------|------|
| **AMax** | Address ≥ 1,441,792 (0x160000), or name starts with `AMAX` / `baseline`, or name is `WINDOW_MAXIM` |
| **Core** | Everything else |

You can change the section names by editing the output JSON.

### Post-Generation Editing (Required)

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

Refer to the firmware documentation (`AMAX_firmware.../documentation_.../a00103.html`)
for parameter ranges and descriptions.

### Complete Workflow (New Firmware)

```bash
# 1. Generate from Sci-Compiler output
cargo run --release --bin gen_defs -- \
  path/to/new_fw/RegisterFile.json \
  registers/register_YYYYMMDD.json

# 2. Edit min/max/default values in the generated file

# 3. Launch amax_viewer with the new definitions
./target/release/amax_viewer registers/register_YYYYMMDD.json
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
| All current register values | same |

These are restored on the next launch.

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
| CLI register file not loading | Check stderr for error messages. The path is relative to the working directory |
