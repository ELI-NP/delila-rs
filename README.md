# DELILA-rs

Distributed data-acquisition (DAQ) system for CAEN digitizers, written in Rust.
A ZeroMQ pipeline moves hits from the digitizers through merging, recording and
online monitoring; a single Operator process exposes a REST API and web UI for
control.

> **Absolute rule — never drop data.** All ZMQ sockets run with `HWM=0`
> (unbounded buffers) and internal channels are unbounded. If the system cannot
> keep up it stalls; it never silently discards hits. See `CLAUDE.md`.

```
                 ┌──────────┐     ┌──────────┐     ┌────────────┐
  CAEN digitizer │  Reader  │ ZMQ │  Merger  │ ZMQ │  Recorder  │→ .delila / ROOT
  (optical/USB) →│ (decode) │────→│ (sort/EB)│──┬─→│            │
                 └──────────┘     └──────────┘  │  └────────────┘
                                                │  ┌────────────┐
                        Operator (REST + Web UI)└─→│  Monitor   │→ histograms / waveforms
                        controls the whole pipeline └────────────┘
```

## Quick start

```bash
# Build the production service binaries (reader, merger, recorder, monitor, operator, …)
cargo build --release --bins

# Event Builder / ROOT output additionally need the `root` feature
cargo build --release --features root --bin event_builder

# Launch a full pipeline from a config file
./scripts/start_daq.sh config/config_psd1_test.toml     # --no-mongo to skip MongoDB
```

Once running:

| Service | URL |
|---------|-----|
| Operator REST / Swagger UI | http://localhost:9090/swagger-ui/ |
| Monitor (histograms/waveforms) | http://localhost:8081/ |
| Mongo Express (run history) | http://localhost:8082/ |

Control the DAQ **only through the Operator REST API** (web UI or
`./scripts/daq_ctl.sh`); never send raw ZMQ commands. Stop everything with
`./scripts/stop_daq.sh`.

State machine: `Idle → Configure → Configured → Arm → Armed → Start → Running → Stop → Configured`.

## Documentation

**Language policy:** operator-facing manuals (operations, config, AMax FW
update) are kept in both Japanese and English — Japanese is canonical and
the English version carries an `_EN` / `_en` suffix; keep the two in sync when
editing. Internal design docs and specifications are single-language. Start with
the operations manual.

### Operating the DAQ — start here

| Document | Lang | Purpose |
|----------|------|---------|
| [Operations manual](docs/operations_manual.md) ([EN](docs/operations_manual_en.md)) | JP/EN | Start/stop, run control, troubleshooting |
| [Config file (TOML) manual](manual/config_toml_manual_JP.md) ([EN](manual/config_toml_manual_EN.md)) | JP/EN | Full `config/*.toml` reference |

Site-specific shift manuals (containing local credentials) are kept out of this
repository under a local-only directory.

### Event Building

| Document | Lang | Purpose |
|----------|------|---------|
| [Offline Event Builder manual](docs/offline_event_builder_manual.md) | JP | End-to-end `.delila` → ROOT built events |
| [Offline Event Builder guide](docs/event_builder_guide.md) | EN | Condensed English walkthrough |

### Firmware & hardware

| Document | Lang | Purpose |
|----------|------|---------|
| [AMax firmware update manual](docs/amax_fw_update_manual.md) ([EN](docs/amax_fw_update_manual_en.md)) | JP/EN | One-command codegen→build→UI→deploy after an AMax FW rebuild |
| [AMax firmware trigger modification](docs/amax_firmware_trigger_modification.md) | EN | FW-side energy-gate / trigger changes |
| [A3818 driver analysis](docs/a3818_driver_analysis.md) | EN | Optical-link driver build & pitfalls |
| [PSD1 debug tool](docs/psd1_debug_tool.md) | JP | Raw-dump / decode debugging helper |

### Design & architecture

| Document | Purpose |
|----------|---------|
| [Component architecture](docs/component_architecture.md) | Task-separation + mpsc channel design (mandatory pattern) |
| [Control system design](docs/control_system_design.md) | Operator / state machine internals |
| [Event Builder design](docs/event_builder_design.md) | EB responsibilities & boundaries |
| [Peak fitting design](docs/peak_fitting_design.md) | Monitor peak-fit feature |
| [architecture/](docs/architecture/) | Config & deployment architecture notes |

### Specifications & references

| Document | Purpose |
|----------|---------|
| [Digitizer system spec](docs/digitizer_system_spec.md) | DevTree, parameters, per-firmware behaviour |
| [CoMPASS ↔ DevTree mapping](docs/compass_devtree_mapping.md) | Parameter name correspondence (all firmwares) |
| [PHA1 decoder spec](docs/pha1_decoder_spec.md) · [PSD1 decoder spec](docs/psd1_decoder_spec.md) | Data-format decode references |
| [PSD1/PHA1 parameter reference](docs/psd1_pha1_parameter_reference.md) | Channel parameter meanings |
| [Waveform decoding](docs/waveform_decoding.md) · [x743 DPP-CI parameters](docs/x743_dpp_ci_parameters.md) | Format details |
| [devtree_examples/](docs/devtree_examples/) | Real DevTree JSON captured from hardware |

### Plans & future work

- [docs/plans/](docs/plans/) — design plans and benchmark results
- [docs/future_plan/](docs/future_plan/) — forward-looking proposals
- [TODO/CURRENT.md](TODO/CURRENT.md) — current sprint index

## Repository layout

```
src/            Rust sources (reader, merger, recorder, monitor, operator, …)
  bin/          Service binaries + hardware-in-the-loop dev tools (--features dev-tools)
config/         Example TOML configs (config_*.toml) + config/digitizers/
scripts/        start_daq.sh / stop_daq.sh / daq_ctl.sh / update_amax_fw.sh …
web/operator-ui/ Angular Operator UI (built bundle committed under dist/)
docs/           Manuals, design docs and specifications (see index above)
manual/         Config-file manual (JP/EN)
external/       CAEN vendor libraries as git submodules (caen-felib, caen-dig2, caen-a3818-driver)
delila-derive/  Procedural-macro crate
```

## Development

```bash
cargo fmt && cargo clippy --tests -- -D warnings && cargo test
```

- `unsafe` is confined to the CAEN FFI wrapper layer; production code uses
  `Result<T, E>` + `?` and never `.unwrap()`.
- Contributor and design guidelines live in [CLAUDE.md](CLAUDE.md).
- License: BSD-3-Clause (see [LICENSE](LICENSE)).
