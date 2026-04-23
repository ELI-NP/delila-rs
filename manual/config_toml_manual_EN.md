# DELILA-RS Configuration File (TOML) Manual

Full specification for the TOML configuration file loaded by `scripts/start_daq.sh`.
Source: [src/config/mod.rs](../src/config/mod.rs)

```bash
./scripts/start_daq.sh config/config_psd1_test.toml
```

If the argument is omitted, `config.toml` is used. Pass `--no-mongo` to skip MongoDB startup.

---

## Table of Contents

- [1. Overall Structure](#1-overall-structure)
- [2. `[operator]` — Operator (Web UI) Settings](#2-operator--operator-web-ui-settings)
- [3. `[network]` — Network Topology](#3-network--network-topology)
- [4. `[[network.sources]]` — Data Sources (Required)](#4-networksources--data-sources-required)
- [5. `[network.merger]` — Merger](#5-networkmerger--merger)
- [6. `[network.recorder]` — Recorder](#6-networkrecorder--recorder)
- [7. `[network.monitor]` — Monitor](#7-networkmonitor--monitor)
- [8. `[network.event_builder]` — Online Event Builder (Optional)](#8-networkevent_builder--online-event-builder-optional)
- [9. `[operator.influxdb]` / `[operator.elog]` — External Integrations](#9-operatorinfluxdb--operatorelog--external-integrations)
- [10. `[settings]` — Emulator / Batch Settings](#10-settings--emulator--batch-settings)
- [11. Port Allocation Rules](#11-port-allocation-rules)
- [12. Complete Example: Local PSD1 (single digitizer)](#12-complete-example-local-psd1-single-digitizer)
- [13. Complete Example: Distributed (Remote Reader)](#13-complete-example-distributed-remote-reader)
- [14. Digitizer URL Reference](#14-digitizer-url-reference)

---

## 1. Overall Structure

```toml
[operator]       # Web UI / REST API
  [operator.influxdb]   # (optional) Grafana integration
  [operator.elog]       # (optional) ELOG integration

[network]
  [[network.sources]]   # One or more. One block per Reader/Emulator
  [network.merger]
  [network.recorder]
  [network.monitor]
  [network.event_builder]   # (optional)

[settings]       # (optional) Emulator / batch parameters
  [settings.file]
```

The startup script iterates over `[[network.sources]]` and launches either an emulator or a reader for each source. Sources with `host != "localhost"` are skipped automatically (i.e. distributed mode — run them manually on the remote machine).

---

## 2. `[operator]` — Operator (Web UI) Settings

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `experiment_name` | string | `"DefaultExp"` | Experiment name. Server-authoritative (not editable from UI). Used for ROOT filenames, etc. |
| `port` | u16 | `9090` | HTTP port for REST API / Web UI. Swagger at `http://localhost:PORT/swagger-ui/` |
| `web_ui_dir` | string | auto-detect | Directory with built Angular UI. If omitted, uses `web/operator-ui/dist/operator-ui/browser/`. **`dist/` is committed to this repo — no Node.js needed on deploy targets.** Developers who modify UI `src/` must rebuild (`cd web/operator-ui && npm run build`) and commit `dist/` together. |
| `configure_timeout_ms` | u64 | `5000` | Timeout for the Configure phase (ms) |
| `arm_timeout_ms` | u64 | `5000` | Timeout for the Arm phase (ms) |
| `start_timeout_ms` | u64 | `5000` | Timeout for the Start phase (ms) |
| `reset_timeout_ms` | u64 | `5000` | Timeout for the Reset phase (ms) |

```toml
[operator]
experiment_name = "PSD1_Test"
port = 9090
configure_timeout_ms = 10000   # extend to 10s for slow optical links
```

---

## 3. `[network]` — Network Topology

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `cluster_name` | string | `"default"` | Cluster identifier (display only) |
| `port_base_data` | u16 | `7000` | Base port for data (PUB) sockets. When `bind` is omitted in a source, port = `port_base_data + id` |
| `port_base_command` | u16 | `7100` | Base port for command (REP) sockets. When `command` is omitted, port = `port_base_command + id` |

Child tables:
- `[[network.sources]]` — array (one or more)
- `[network.merger]` — single
- `[network.recorder]` — single
- `[network.monitor]` — single
- `[network.event_builder]` — optional (if present, `online_event_builder` binary is started)

---

## 4. `[[network.sources]]` — Data Sources (Required)

Write one block for every Reader or Emulator process to be launched.

| Key | Type | Required | Default | Description |
|-----|------|----------|---------|-------------|
| `id` | u32 | ✅ | — | Unique source ID. Basis for auto port allocation and module_id |
| `name` | string | | `""` | Display name (shown in UI) |
| `type` | enum | | `"emulator"` | `emulator` / `psd1` / `psd2` / `pha1` / `amax` / `x743ci` / `x743std` / `zle` |
| `bind` | string | | `tcp://*:{port_base_data + id}` | Data PUB bind. **Omit this** (use auto-allocation from section 11) |
| `command` | string | | `tcp://*:{port_base_command + id}` | Command REP bind. **Omit this** (use auto-allocation from section 11) |
| `digitizer_url` | string | if type≠emulator | — | Digitizer URL. See section 14 |
| `config_file` | string | if type≠emulator | — | Path to digitizer JSON config (relative or absolute) |
| `module_id` | u8 | | same as `id` | Module ID used for event tagging |
| `time_step_ns` | f64 | | auto | ADC sample period (ns). `4.0` for PHA1 (250 MS/s); `2.0` for PSD1/PSD2 (500 MS/s) |
| `pipeline_order` | u32 | | `1` | Start/Stop ordering. Smaller = more upstream |
| `host` | string | | `"localhost"` | Host where the Reader runs. If not `localhost`, start_daq.sh skips launch → run manually on the remote machine |
| `adc_min` | u16 | | `0` | Lower energy filter (inclusive). Events with energy `< adc_min` are dropped |

> **Recommended** — Leave `bind` / `command` out. Ports are auto-allocated from `id` as `tcp://*:{7000+id}` / `tcp://*:{7100+id}`, which avoids port collisions. If you need to shift the base ports, change `[network] port_base_data` / `port_base_command` in one place (see section 11).

### Meaning of `type`

| Value | Hardware / Library |
|-------|-------------------|
| `emulator` | Dummy data generator (for testing) |
| `psd1` | CAEN DPP-PSD (legacy). DT5730/VX1730 etc., via CAENDigitizer library |
| `psd2` | CAEN DPP-PSD (new). VX2730 etc., via dig2 library |
| `pha1` | CAEN DPP-PHA. V1725S etc., via CAENDigitizer library |
| `amax` | DELILA custom AMax FW (trapezoidal-filter MCA), DPP_OPEN |
| `x743ci` | V1743 Charge Integration mode |
| `x743std` | V1743 Standard waveform mode |
| `zle` | DPP-ZLE (not yet implemented) |

### Minimal example (emulator)

```toml
[[network.sources]]
id = 0
name = "emu-0"
type = "emulator"
```

### Minimal example (PSD1 USB)

```toml
[[network.sources]]
id = 0
name = "psd1-dt5730"
type = "psd1"
digitizer_url = "dig1://caen.internal/usb?link_num=0"
config_file = "config/digitizers/psd1_test.json"
```

---

## 5. `[network.merger]` — Merger

Component that merges data from multiple sources into a single PUB stream.

| Key | Type | Required | Default | Description |
|-----|------|----------|---------|-------------|
| `subscribe` | [string] | | auto-generated | Array of upstream PUB connect addresses. If empty, auto-generated from each source's `host:port_base_data+id` |
| `publish` | string | ✅ | — | PUB bind for the downstream stream (e.g. `tcp://*:5557`) |
| `command` | string | | — | Command REP bind |
| `pipeline_order` | u32 | | `2` | Start/Stop ordering |

Recommended: **omit** `subscribe` and let auto-resolution handle it. As long as each source has a correct `host` field, the Merger will connect to the right machines automatically.

```toml
[network.merger]
publish = "tcp://*:5557"
command = "tcp://*:5570"
```

---

## 6. `[network.recorder]` — Recorder

Writes raw `.delila` binary data to disk.

| Key | Type | Required | Default | Description |
|-----|------|----------|---------|-------------|
| `subscribe` | string | ✅ | — | Connect address for the Merger PUB |
| `command` | string | | — | Command REP bind |
| `output_dir` | string | | `"./data"` | Output directory (created if missing) |
| `max_file_size_mb` | u64 | | `1024` | File rotation: size limit (MB) |
| `max_file_duration_sec` | u64 | | `600` | File rotation: time limit (s) |
| `pipeline_order` | u32 | | `3` | Start/Stop ordering |

```toml
[network.recorder]
subscribe = "tcp://localhost:5557"
command = "tcp://*:5580"
output_dir = "./data"
max_file_size_mb = 2048
max_file_duration_sec = 1800
```

---

## 7. `[network.monitor]` — Monitor

Serves live histograms / waveforms over HTTP.

| Key | Type | Required | Default | Description |
|-----|------|----------|---------|-------------|
| `subscribe` | string | ✅ | — | Connect address for the Merger PUB |
| `command` | string | | — | Command REP bind |
| `http_port` | u16 | | `8081` | HTTP / Web UI port |
| `pipeline_order` | u32 | | `3` | Start/Stop ordering |
| `psd_bins` | u32 | | `200` | PSD 1D histogram bin count |
| `psd_min` | f32 | | `-0.2` | PSD 1D minimum value |
| `psd_max` | f32 | | `1.2` | PSD 1D maximum value |
| `psd2d_x_bins` | u32 | | `512` | PSD 2D: X (Energy) bins |
| `psd2d_y_bins` | u32 | | `200` | PSD 2D: Y (PSD) bins |

```toml
[network.monitor]
subscribe = "tcp://localhost:5557"
command = "tcp://*:5590"
http_port = 8081
```

---

## 8. `[network.event_builder]` — Online Event Builder (Optional)

Started only if this section is present **and** `target/release/online_event_builder` exists (ROOT output requires a `--features root` build).

| Key | Type | Required | Default | Description |
|-----|------|----------|---------|-------------|
| `subscribe` | string | ✅ | — | Connect address for the Merger PUB |
| `command` | string | | — | Command REP bind |
| `output_dir` | string | | `"./data/events"` | ROOT event file output directory |
| `coincidence_window_ns` | f64 | | `500.0` | Coincidence window width (ns) |
| `slice_duration_ns` | f64 | | `10_000_000.0` | Time slice length (ns) = 10 ms |
| `buffer_delay_ns` | f64 | | `5_000_000.0` | TimeSortBuffer delay (ns) = 5 ms |
| `ch_settings_file` | string | | — | Channel settings JSON (detector type, thresholds, etc.) |
| `time_calib_file` | string | | — | Time calibration JSON |
| `pipeline_order` | u32 | | `3` | Start/Stop ordering |

```toml
[network.event_builder]
subscribe = "tcp://localhost:5557"
command = "tcp://*:5595"
output_dir = "./data/events"
coincidence_window_ns = 500.0
ch_settings_file = "config/chSettings.json"
```

---

## 9. `[operator.influxdb]` / `[operator.elog]` — External Integrations

### InfluxDB (Grafana metrics)

| Key | Type | Required | Default | Description |
|-----|------|----------|---------|-------------|
| `url` | string | ✅ | — | InfluxDB v3 endpoint (e.g. `http://localhost:8181`) |
| `database` | string | | `"delila"` | DB name |
| `interval_secs` | u64 | | `2` | Polling interval (seconds) |

```toml
[operator.influxdb]
url = "http://localhost:8181"
database = "delila"
interval_secs = 2
```

### ELOG (electronic logbook)

| Key | Type | Required | Default | Description |
|-----|------|----------|---------|-------------|
| `url` | string | ✅ | — | ELOG server URL |
| `logbook` | string | ✅ | — | Logbook name |
| `author` | string | | `"DELILA-DAQ"` | Author used for auto-posted entries |

```toml
[operator.elog]
url = "http://localhost:8082"
logbook = "3MV_2026"
```

---

## 10. `[settings]` — Emulator / Batch Settings

Mainly Emulator parameters. Usually omitted for real digitizer runs.

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `source` | enum | `"file"` | `"file"` (recommended) / `"mongodb"` (not yet supported) |

`[settings.file]`:

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `events_per_batch` | u32 | `100` | Events per batch |
| `batch_interval_ms` | u64 | `100` | Batch interval (ms) |
| `num_modules` | u32 | `2` | Module count (Emulator) |
| `channels_per_module` | u32 | `16` | Channels per module (Emulator) |
| `enable_waveform` | bool | `false` | Waveform generation (Emulator) |
| `waveform_probes` | u8 | `3` | Waveform probe bitmask (1=analog1, 2=analog2, 3=both, 63=all) |
| `waveform_samples` | usize | `512` | Waveform sample count |

```toml
[settings]
source = "file"

[settings.file]
events_per_batch = 200
batch_interval_ms = 50
enable_waveform = true
waveform_samples = 1024
```

---

## 11. Port Allocation Rules

### Auto allocation (recommended)

If a source omits `bind` / `command`, ports are auto-allocated as follows:

- Data: `tcp://*:{port_base_data + id}` (default `7000 + id`)
- Command: `tcp://*:{port_base_command + id}` (default `7100 + id`)

```toml
[network]
port_base_data = 7000
port_base_command = 7100

[[network.sources]]
id = 0              # data: 7000, command: 7100
[[network.sources]]
id = 1              # data: 7001, command: 7101
```

### Default port quick reference

| Component | Port | Kind |
|-----------|------|------|
| Operator HTTP | 9090 | REST/Web UI |
| Monitor HTTP | 8081 | REST/Web UI |
| Source data (auto) | 7000+id | ZMQ PUB |
| Source command (auto) | 7100+id | ZMQ REP |
| Merger publish | 5557 | ZMQ PUB |
| Merger command | 5570 | ZMQ REP |
| Recorder command | 5580 | ZMQ REP |
| Monitor command | 5590 | ZMQ REP |
| EventBuilder command | 5595 | ZMQ REP |
| MongoDB | 27017 | TCP |

> **Warning** — Port collisions fail silently with no warning (only the ZMQ bind error shows up in logs). When running multiple DAQ instances on the same host, always separate `[operator] port`, `[network.monitor] http_port`, the `publish` port, and `port_base_*`.

---

## 12. Complete Example: Local PSD1 (single digitizer)

Structure of `config/config_psd1_test.toml`:

```toml
[operator]
experiment_name = "PSD1_Test"
port = 9090

[network]

[[network.sources]]
id = 0
name = "psd1-dt5730"
type = "psd1"
# bind/command omitted → auto-allocated to data: 7000, command: 7100
digitizer_url = "dig1://caen.internal/usb?link_num=0"
config_file = "config/digitizers/psd1_test.json"
pipeline_order = 1

[network.merger]
publish = "tcp://*:5557"
command = "tcp://*:5570"
pipeline_order = 2

[network.recorder]
subscribe = "tcp://localhost:5557"
command = "tcp://*:5580"
output_dir = "./data"
pipeline_order = 3

[network.monitor]
subscribe = "tcp://localhost:5557"
command = "tcp://*:5590"
http_port = 8081
pipeline_order = 3
```

---

## 13. Complete Example: Distributed (Remote Reader)

Layout with the Reader running on a separate machine. Declared via `host`.

```toml
[operator]
experiment_name = "PSD1_Distributed"
port = 9090

[network]

[[network.sources]]
id = 0
name = "psd1-dt5730"
type = "psd1"
host = "172.18.4.147"        # ← remote host where the Reader runs
# bind/command omitted → auto-resolved to 172.18.4.147:7000 (data), :7100 (command)
digitizer_url = "dig1://caen.internal/usb?link_num=0"
config_file = "config/digitizers/psd1_test.json"
pipeline_order = 1

# subscribe is omitted → auto-resolved from host
[network.merger]
publish = "tcp://*:5557"
command = "tcp://*:5570"
pipeline_order = 2

[network.recorder]
subscribe = "tcp://localhost:5557"
output_dir = "./data"

[network.monitor]
subscribe = "tcp://localhost:5557"
http_port = 8081
```

Startup:

```bash
# Local (Operator host)
./scripts/start_daq.sh config/config_psd1_distributed.toml
# → Reader launch is skipped; instructions are printed

# Remote (172.18.4.147)
./target/release/reader --config config/config_psd1_distributed.toml --source-id 0
```

---

## 14. Digitizer URL Reference

**Authoritative source:** [legacy/GD9764_FELib_User_Guide.pdf](../legacy/GD9764_FELib_User_Guide.pdf) Rev.2 Chapter 6

URLs follow RFC 3986: `<scheme>://<authority>/<path>?<queries>`. Case-insensitive.
Multiple queries are joined with `&`.

### 14.1 Scheme Overview

| type | Prefix | Implementing library | Supported hardware |
|------|--------|---------|---------|
| `psd1` / `pha1` | `dig1://` | CAEN Dig1 (FELib v1 compat) | V17xx / VX17xx / DT57xx (FW 1.0) |
| `psd2` / `amax` | `dig2://` | CAEN Dig2 (FELib v2) | V27xx / VX27xx / DT27xx (FW 2.0) |
| `x743ci` / `x743std` | **(no URL)** | CAENDigitizer (direct) | V1743 — connection set in `[x743]` section |

**Important:** Only **one connection per digitizer** at a time. `CAEN_FELib_Open()`
from another process forcibly disconnects the existing session. Within the same process,
`DeviceAlreadyOpen` is returned.

---

### 14.2 `dig2://` (Digitizer 2.0)

Authority is an IP / hostname or the reserved `caen.internal`.

| Connection | URL example | Notes |
|---|---|---|
| **Ethernet (IPv4)** | `dig2://192.0.2.1` | Most common. Prefer explicit IP |
| **Ethernet (IPv6)** | `dig2://[2001:db8::1]` | Brackets required |
| **Ethernet (mDNS)** | `dig2://caendgtz-eth-<pid>` | OS-dependent (Linux needs `.local`). Not recommended |
| **USB (short form)** | `dig2://caendgtz-usb-<pid>` | `<pid>` = device S/N |
| **USB (`caen.internal`)** | `dig2://caen.internal/usb/<pid>` | Alternative form |
| **OpenARM (embedded ARM)** | `dig2://caen.internal/openarm` | Used from DT27xx on-board ARM instead of 172.17.0.1 |

Examples:
```
dig2://172.18.4.56           # VX2730 on LAN
dig2://caendgtz-usb-52622    # same box via USB 3.0
```

---

### 14.3 `dig1://` (Digitizer 1.0)

The `path` encodes the **connection type**, mapping to `CAEN_DGTZ_ConnectionType` enum.
Query parameters specify the target.

| Path | enum (CAEN_DGTZ_*) | Meaning | Authority |
|---|---|---|---|
| `/usb` | `USB` | Direct USB 2.0 (V1720 / V1730 USB port, etc.) | `caen.internal` |
| `/optical_link` | `OpticalLink` | A2818 / A3818 PCIe + optical CONET | `caen.internal` |
| `/usb_a4818` | `USB_A4818` | USB → A4818 → optical → digitizer | `caen.internal` |
| `/usb_a4818_v2718` | `USB_A4818_V2718` | A4818 → V2718 VME bridge | `caen.internal` |
| `/usb_a4818_v3718` | `USB_A4818_V3718` | A4818 → V3718 VME bridge | `caen.internal` |
| `/usb_a4818_v4718` | `USB_A4818_V4718` | A4818 → V4718 VME bridge | `caen.internal` |
| `/eth_v4718` | `ETH_V4718` | Ethernet → V4718 VME bridge | **V4718's IP** |
| `/usb_v4718` | `USB_V4718` | USB → V4718 VME bridge | `caen.internal` |

#### Query parameters

| Key | Meaning | Applicable paths |
|---|---|---|
| `link_num=<N>` | A3818/A4818/USB link number. For A4818 / USB V4718: the bridge's PID | `/optical_link`, `/usb`, `/usb_a4818*`, `/usb_v4718` |
| `conet_node=<N>` | CONET daisy-chain node number (0–7) | `/optical_link`, A4818 bridges |
| `vme_base_address=<addr>` | VME base address (0x-prefixed OK) | VME bridge paths (V17xx VME modules) |

#### URL examples

```
# Direct USB (DT5730B, V1720 USB port, ...)
dig1://caen.internal/usb?link_num=0

# A3818 PCIe optical, port 0, first node on daisy chain
dig1://caen.internal/optical_link?link_num=0&conet_node=0

# A3818 port 2 → V3718 VME bridge → V1730 at VME base 0x32100000
dig1://caen.internal/optical_link?link_num=2&vme_base_address=0x32100000

# A4818 USB bridge to an optical digitizer
dig1://caen.internal/usb_a4818?link_num=<A4818_PID>&conet_node=0

# V4718 Ethernet bridge (IP 10.1.2.3) → VME V1730
dig1://10.1.2.3/eth_v4718?vme_base_address=0x00100000

# V4718 USB bridge
dig1://caen.internal/usb_v4718?link_num=<V4718_PID>&vme_base_address=0x00100000
```

---

### 14.4 Connection Quick Reference

| Hardware | Recommended URL |
|---|---|
| **DT5730B (USB)** | `dig1://caen.internal/usb?link_num=0` |
| **V1730 (optical + A3818 PCIe)** | `dig1://caen.internal/optical_link?link_num=<port>&conet_node=<node>` |
| **VX1730B (optical + A3818)** | same |
| **V1730 VME module (A3818 → V3718 VME)** | `dig1://caen.internal/optical_link?link_num=<port>&vme_base_address=<addr>` |
| **VX2730 (Ethernet)** | `dig2://<IP>` |
| **VX2730 (USB 3.0)** | `dig2://caendgtz-usb-<SN>` |
| **DT2730 OpenARM embedded** | `dig2://caen.internal/openarm` |
| **V1743** (DPP-CI/Standard) | `digitizer_url` unused — set `link_num` / `conet_node` / `connection_type` in `[x743]` |

### 14.5 Notes

- **One connection per digitizer**: If another process `CAEN_FELib_Open()`s an already-connected board, the existing side is disconnected. Can cause unexpected DAQ Error states
- **`caen.internal` is a reserved authority**: not a real IP/hostname. Used as a placeholder for non-network connections (USB / optical / VME bridges)
- **A3818 driver**: on Linux requires kernel module (`a3818.ko`) and `/etc/udev/rules.d/`. DELILA uses patched `v1.6.12-delila1` (see `docs/a3818_driver_analysis.md`)
- **Ethernet digitizer discovery**: mDNS (`caendgtz-eth-<pid>`) is OS-dependent and may not work. Prefer direct IP
- **Case sensitivity**: URLs are NOT case-sensitive (`DIG2://` == `dig2://`)

---

## References

- Examples: [config/](../config/) — many per-use-case samples
- Digitizer JSON spec: [docs/digitizer_system_spec.md](../docs/digitizer_system_spec.md)
- Overall architecture: [docs/architecture/config_and_deployment.md](../docs/architecture/config_and_deployment.md)
- Source: [src/config/mod.rs](../src/config/mod.rs)
- Startup script: [scripts/start_daq.sh](../scripts/start_daq.sh)
