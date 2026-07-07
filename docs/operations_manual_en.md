# DELILA DAQ Operations Manual

How to start, run, stop and troubleshoot the DAQ. For the configuration file
(TOML) grammar see the
[Configuration file manual](../manual/config_toml_manual_EN.md).

日本語版: [operations_manual.md](operations_manual.md)

---

## System overview

The DAQ is a single ZMQ pipeline. `scripts/start_daq.sh` launches every
component from one config file.

```
  Reader(s) ──ZMQ──> Merger ──ZMQ──┬──> Recorder  (.delila / ROOT)
  (decode)          (sort/EB)      └──> Monitor    (histograms / waveforms)

  Operator (REST + Web UI, port 9090) controls the whole pipeline
```

| Service | URL / port |
|---------|-----------|
| Operator REST / Swagger UI | http://localhost:9090/swagger-ui/ |
| Monitor Web UI | http://localhost:8081/ |
| Mongo Express (run history) | http://localhost:8082/ |

> **Always control the DAQ through the Operator REST API (web UI).** Never send
> raw ZMQ commands. `scripts/daq_ctl.sh` (the `controller` binary) is a
> low-level developer tool.

For running a Reader on a separate machine see "6. Distributed setup".

---

## 1. Build

```bash
# Production service binaries (reader / merger / recorder / monitor / operator …)
cargo build --release --bins

# ROOT output / Event Builder need the `root` feature
cargo build --release --features root --bin event_builder
```

---

## 2. Starting the DAQ

Start with a single config file:

```bash
./scripts/start_daq.sh config/config_psd1_test.toml       # --no-mongo skips MongoDB
```

`start_daq.sh`:

1. `pkill`s any leftover processes from a previous session
2. starts MongoDB / Docker if needed
3. starts one reader per `[[network.sources]]` in the config
4. starts merger → recorder → monitor → (if enabled) online_event_builder → operator
5. waits until the Operator answers `/api/status`

On success it prints the Web UI URLs.

---

## 3. Sanity check

Open http://localhost:9090 in a browser. Success = all components show **Idle** /
**Online**.

From the command line:

```bash
curl -s http://localhost:9090/api/status | python3 -m json.tool
```

---

## 4. Acquiring and stopping data

The web UI is the normal way to drive runs. From the command line use the
Operator REST API.

State machine:
`Idle → Configure → Configured → Arm → Armed → Start → Running → Stop → Configured`

```bash
# Individual transitions
curl -X POST http://localhost:9090/api/configure       # Idle → Configured
curl -X POST http://localhost:9090/api/arm             # Configured → Armed
curl -X POST http://localhost:9090/api/start           # Armed → Running
curl -X POST http://localhost:9090/api/stop            # Running → Configured
curl -X POST http://localhost:9090/api/reset           # Any → Idle

# One shot (Detect → Configure → Arm → Start)
curl -X POST http://localhost:9090/api/run/start
```

Run numbers and history are managed via `/api/runs*` (the Runs page in the web UI).

---

## 5. Stopping the DAQ

```bash
./scripts/stop_daq.sh
```

Stops every component. Per-component logs live in `logs/latest/*.log`
(`./logs/latest/` is a symlink to the most recent run).

---

## 6. Distributed setup (remote Reader)

To run a Reader on another machine (e.g. a Linux box with a USB-connected
digitizer), set the `host` field of that `[[network.sources]]` to the machine's
IP. Leaving the Merger's `subscribe` empty makes it auto-resolve the connection
from `bind`+`host` (see the
[Configuration file manual](../manual/config_toml_manual_EN.md)).

Start a remote Reader over SSH:

```bash
./scripts/start_remote_reader.sh config/config_psd1_test.toml
```

Then run `start_daq.sh` locally; it starts merger / recorder / monitor / operator
(everything except the remote reader).

---

## 7. Troubleshooting

### After physically power-cycling a digitizer

The Reader hits a connection error. On DIG1 (USB / optical link) a **Close+Open
resets the timestamp**, so never blindly reconnect on a transient error. After a
restart, stop the DAQ, re-launch with `start_daq.sh`, then in the web UI do
**Reset → Configure → Start**.

### A process won't start / stops responding

Check the logs:

```bash
tail -100 logs/latest/operator.log
tail -100 logs/latest/reader_0.log      # reader_<id>.log per source id
tail -f  logs/latest/*.log
```

### Port already in use

A previous process may still be running. `start_daq.sh` auto-`pkill`s on start;
to stop manually:

```bash
./scripts/stop_daq.sh
# if something still lingers
pkill -f target/release/operator
```

### A digitizer is missing from Settings

Check that no two digitizer JSON files share the same `digitizer_id`. The
`digitizer_id` must match the `id` of a `[[network.sources]]` entry in the TOML.
A duplicate lets the later-loaded one overwrite the earlier, so only one shows up.

```
config/digitizers/psd1_test.json → "digitizer_id": 0   (= TOML source id 0)
config/digitizers/psd2_56.json   → "digitizer_id": 1   (= TOML source id 1)
```

### Zero events, counter not increasing

Check the physical side before the software. Typical causes: NIM crate off
(Configure succeeds but events=0), digitizer FW hang (continuous CAEN -6, needs a
power cycle), VME off (Configure fails). PHA firmwares historically wedge; after
Start, check the ADC spectrum and if it looks wrong, power-cycle the crate.

---

## 8. Configuration files

| File | Purpose |
|------|---------|
| `config/config_*.toml` | DAQ topology (pass one at startup) |
| `config/digitizers/*.json` | Per-digitizer parameters |

- Full TOML grammar → [Configuration file manual](../manual/config_toml_manual_EN.md)
- Parameter name mapping → [CoMPASS ↔ DevTree mapping](compass_devtree_mapping.md)

---

## Quick reference

```bash
# === Build ===
cargo build --release --bins

# === Start ===
./scripts/start_daq.sh config/config_psd1_test.toml

# === Status ===
curl -s http://localhost:9090/api/status | python3 -m json.tool

# === Run control (normally via web UI http://localhost:9090) ===
curl -X POST http://localhost:9090/api/run/start     # start in one shot
curl -X POST http://localhost:9090/api/stop          # stop

# === Stop everything ===
./scripts/stop_daq.sh
```
