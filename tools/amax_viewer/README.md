# AMax Viewer - CAEN Digitizer Firmware Development Tool

**Version:** 0.1.0
**Last Updated:** 2026-02-02

## Overview

AMax Viewer is a GUI development tool for real-time adjustment and verification of AMax firmware parameters on CAEN digitizers. It displays a 2D histogram (Energy vs AMax) while allowing you to tune MCA HLS register parameters and immediately observe the results.

### Main Features

- Parameter tuning for AMax firmware
- Real-time visualization via 2D histogram (Energy vs AMax)
- Waveform display for signal verification
- ROOT file output for offline analysis

---

## Requirements

### Hardware
- CAEN Digitizer (DT5725, VX2730, etc.)
- Network connection (dig2:// protocol)

### Software
- **Rust toolchain** (1.70+)
- **CAEN FELib** (`libCAEN_FELib`)
- **CAEN dig2** (dig2:// protocol driver)
- macOS or Linux

---

## Installation

### WSL2 Ubuntu Setup (From Scratch)

Follow these steps to set up the environment using Ubuntu on WSL2.

#### 1. Install Required System Packages

```bash
# Update package list
sudo apt update

# Build tools and basic dependencies
sudo apt install -y build-essential pkg-config git curl

# Libraries required for GUI (egui)
sudo apt install -y libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \
    libxkbcommon-dev libssl-dev libfontconfig1-dev
```

#### 2. Install Rust

```bash
# Install rustup
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Select default option (1)

# Load environment variables
source $HOME/.cargo/env

# Verify installation
rustc --version
cargo --version
```

#### 3. Install CAEN Libraries

Download from the CAEN official website:
https://www.caen.it/download/?filter=FELib

**Required packages:**
- CAEN FELib (Linux version)
- CAEN dig2 (Linux version)

```bash
# Extract and install downloaded files
tar xvf CAEN_FELib-X.X.X-Linux.tgz
cd CAEN_FELib-X.X.X
sudo ./install.sh

tar xvf caen-dig2-X.X.X-Linux.tgz
cd caen-dig2-X.X.X
sudo ./install.sh

# Verify installation
ls /usr/local/lib/libCAEN_FELib*
```

#### 4. Clone Repository and Build

```bash
git clone https://github.com/your-repo/delila-rs.git
cd delila-rs/tools/amax_viewer
cargo build --release
```

#### 5. GUI Display Settings for WSL2

To display GUI applications in WSL2, use WSLg on Windows 11 or set up an X server.

**Windows 11 (with WSLg support):**
- No special configuration required. Run directly.

**Windows 10 or if WSLg is not working:**
```bash
# Install an X server on Windows (e.g., VcXsrv or X410)

# Set DISPLAY environment variable (add to ~/.bashrc)
export DISPLAY=$(cat /etc/resolv.conf | grep nameserver | awk '{print $2}'):0
```

---

### macOS / Native Linux Setup

#### 1. Download and Install CAEN Libraries

Download and install libraries from the CAEN official website:
https://www.caen.it/download/?filter=FELib

**Required packages:**
- **CAEN FELib** - Base library for digitizer control
- **CAEN dig2** - dig2:// protocol driver

After downloading, extract each package and run the included install script.

#### 2. Install Rust (if not already installed)

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env
```

#### 3. Clone Repository and Build

```bash
git clone https://github.com/your-repo/delila-rs.git
cd delila-rs/tools/amax_viewer
cargo build --release
```

---

## Running

```bash
# Run directly
cargo run --release

# Or
./target/release/amax_viewer
```

---

## UI Layout

```
┌─────────────────────────────────────────────────────────────────────┐
│                                                                     │
│  ┌──────────────────────┐  ┌────────────────────────────────────┐  │
│  │ Left Panel           │  │ Central Panel                      │  │
│  │                      │  │                                    │  │
│  │ [Connection]         │  │   ┌────────────────────────────┐   │  │
│  │ URL: dig2://...      │  │   │                            │   │  │
│  │ [Start] [Stop]       │  │   │   2D Histogram             │   │  │
│  │                      │  │   │   Energy (X) vs AMax (Y)   │   │  │
│  │ [Status]             │  │   │                            │   │  │
│  │ Events: 123456       │  │   │   ┌─────────────────────┐  │   │  │
│  │ Rate: 1500 Hz        │  │   │   │                     │  │   │  │
│  │                      │  │   │   │   Color intensity   │  │   │  │
│  │ [MCA Parameters]     │  │   │   │   = event count     │  │   │  │
│  │ Polarity:  [▼ 1]     │  │   │   │                     │  │   │  │
│  │ Offset:    [===]     │  │   │   └─────────────────────┘  │   │  │
│  │ Threshold: [===]     │  │   │                            │   │  │
│  │ ...                  │  │   └────────────────────────────┘   │  │
│  │                      │  │                                    │  │
│  │ [AMax Parameters]    │  └────────────────────────────────────┘  │
│  │ Window Maxim: [===]  │                                          │
│  │ BL Delay:     [===]  │  ┌────────────────────────────────────┐  │
│  │ ...                  │  │ Bottom Panel - Waveform            │  │
│  │                      │  │                                    │  │
│  │ [Histogram Range]    │  │   Sample index vs ADC value        │  │
│  │ Energy Max: [====]   │  │   Energy: 1234                     │  │
│  │ AMax Max:   [====]   │  │                                    │  │
│  │                      │  └────────────────────────────────────┘  │
│  │ [ROOT Output]        │                                          │
│  │ Path: amax_data.root │                                          │
│  │ [x] Recording        │                                          │
│  │ [Save]               │                                          │
│  └──────────────────────┘                                          │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

---

## Feature Details

### 1. Digitizer Connection

**URL Format:**
```
dig2://<IP_ADDRESS>
```

Example: `dig2://172.18.4.56`

**Operation:**
1. Enter the digitizer address in the URL input field
2. Click the `Start` button
3. Data acquisition starts automatically after successful connection

**Notes:**
- Only Channel 0 is enabled on connection (other channels are disabled)
- Parameters are automatically applied on connection

---

### 2. MCA Parameters

Controls the MCA HLS registers (addresses 0x0-0xC).

| Parameter | Description | Range |
|-----------|-------------|-------|
| **Polarity** | Input polarity | 0 / 1 (firmware dependent) |
| **Offset** | DC offset | 0-65535 |
| **Threshold** | Trigger threshold | 0-65535 |
| **Trig K** | Trigger filter K | 0-65535 |
| **Trig M** | Trigger filter M | 0-65535 |
| **Trap K** | Trapezoidal filter K | 0-65535 |
| **Trap M** | Trapezoidal filter M | 0-65535 |
| **Trap Gain** | Trapezoidal gain | 0-15 |
| **BL Len** | Baseline length | 0-65535 |
| **BL Inib** | Baseline inhibit | 0-65535 |
| **Deconv M** | Deconvolution M | 0-65535 |
| **Sample Pos** | Sample position | 0-65535 |
| **Run Cfg** | Run configuration | 0-65535 |

**Note:** The specific values for Polarity (positive/negative polarity) may vary depending on the firmware version. Refer to your firmware documentation.

---

### 3. AMax Parameters

Controls AMax firmware-specific registers.

| Parameter | Address | Description |
|-----------|---------|-------------|
| **Window Maxim** | 0x14000 | Maximum window |
| **BL Delay** | 0x160000 | Baseline delay |
| **BL Length** | 0x160001 | Baseline length |
| **BL Offset** | 0x160002 | Baseline offset |
| **AMax Window** | 0x160003 | AMax window |
| **AMax Delay** | 0x160004 | AMax delay |
| **AMax Len** | 0x160005 | AMax length |

---

### 4. 2D Histogram

**Axes:**
- X-axis: Energy (energy value)
- Y-axis: AMax (from `user_info[0]`)

**Color Map:**
```
Black → Blue → Cyan → Yellow → White
(sqrt scaling based on intensity)
```

**Controls:**
- `Energy Max`: Set the maximum value for X-axis
- `AMax Max`: Set the maximum value for Y-axis
- `Clear`: Clear the histogram
- `Restart`: Quick restart after parameter changes

**⚠️ IMPORTANT: How to Change Numeric Values**

When changing histogram bin counts (`Energy Bins`, `AMax Bins`) or range settings:

- **❌ DO NOT press Enter** - The application may freeze
- **✅ Use click & drag to change values**:
  - Click and hold on the numeric field
  - **Drag up** → Value increases
  - **Drag down** → Value decreases

This method applies to all parameter controls.

---

### 5. Waveform Display

Displays the latest waveform at the bottom of the screen.

**Update rate:** ~10 Hz (throttled to reduce UI load)

**Display content:**
- X-axis: Sample index
- Y-axis: ADC value
- Energy value: Energy corresponding to the waveform

---

### 6. ROOT File Output

Collected events can be saved in ROOT format.

**TTree Structure:**
```
Tree: events
├── channel   (u8)    - Channel number
├── energy    (u16)   - Energy value
├── amax      (u16)   - AMax value
└── timestamp (u64)   - Timestamp
```

**Operation:**
1. Enter file path (default: `amax_data.root`)
2. Enable the `Recording` checkbox
3. Perform data acquisition
4. Click `Save` button after `Stop`

**Notes:**
- Events are buffered only when Recording is enabled
- Save is only available after acquisition is stopped

---

### 7. Parameter Persistence

Parameters are automatically saved and loaded.

**Save location:**
```
~/.config/amax_viewer/params.json
```
(Same for macOS/Linux)

**Save timing:**
- On application exit

**Load timing:**
- On application startup

**Note:** This file is created when the application exits normally (closing the window). It will not be created if the application is force-quit or crashes.

---

## Typical Workflows

### Parameter Tuning

```
1. Enter URL and click Start
2. Observe the 2D histogram
3. Adjust parameters by dragging (do not press Enter)
4. Click Restart to apply parameters
5. Check histogram changes
6. Repeat steps 2-5 to find optimal values
7. Click Stop to finish (parameters are auto-saved)
```

### Data Collection

```
1. Start with optimized parameters
2. Enable the Recording checkbox
3. Collect the required number of events
4. Click Stop
5. Verify file path and click Save
6. Analyze ROOT file offline
```

---

## Troubleshooting

### Cannot Connect

1. Verify the digitizer IP address
2. Check network connection (use `ping` to verify connectivity)
3. Verify CAEN libraries are installed:
   ```bash
   ls /usr/local/lib/libCAEN_FELib*
   ```
4. Verify caen-dig2 driver is installed
5. Check if the digitizer is connected to another application

### Parameters Not Applied

1. Check error messages in the Status display
2. If read-back fails, verify the register address is correct
3. Use the Restart button to retry

### Histogram Not Updating

1. Check if Event count is increasing
2. Check Energy Max / AMax Max range settings
3. Check if Threshold is too high

### Application Freezes on Numeric Input

- **DO NOT press Enter**
- Instead, click and drag up/down to change values

### Cannot Save ROOT File

1. Verify Recording was enabled
2. Check write permissions for the file path
3. Verify Save is executed after stopping acquisition

### Build Error: Library Not Found

```bash
# Set environment variables
export LIBRARY_PATH=/usr/local/lib:$LIBRARY_PATH

# For Linux
export LD_LIBRARY_PATH=/usr/local/lib:$LD_LIBRARY_PATH

# For macOS
export DYLD_LIBRARY_PATH=/usr/local/lib:$DYLD_LIBRARY_PATH

# Rebuild
cargo clean
cargo build --release
```

---

## Technical Details

### Thread Model

```
┌─────────────────┐     Arc<Mutex<SharedState>>     ┌─────────────────┐
│   GUI Thread    │ <────────────────────────────> │ Acquisition     │
│   (eframe/egui) │                                │ Thread          │
└─────────────────┘     Arc<AtomicBool>            └─────────────────┘
                        (shutdown signal)
```

### Data Flow

```
CAEN Digitizer
      ↓
CaenHandle::read_opendpp_event_with_waveform()
      ↓
Acquisition Thread
      ↓
┌─────────────────────────────────────┐
│ SharedState (Mutex protected)       │
│ - Histogram2D (bin data)            │
│ - EventBuffer (for ROOT output)     │
│ - Waveform buffer                   │
│ - Status / metrics                  │
└─────────────────────────────────────┘
      ↓
GUI Thread renders histogram texture
```

### Memory Efficiency

- **Histogram**: Fixed memory based on bin count (512x512 = 1MB)
- **Waveform buffer**: 8192 samples pre-allocated (zero-copy update)
- **Event buffer**: Dynamically allocated only when Recording
- **Texture**: Regenerated only when histogram changes

---

## Related Documentation

- [AMax Firmware Trigger Modification](../../docs/amax_firmware_trigger_modification.md)
- [CAEN OpenDPP Endpoint Support](../../src/reader/caen/README.md)

---

## License

MIT License - See project root LICENSE file
