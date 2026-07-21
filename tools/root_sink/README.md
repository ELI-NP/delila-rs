# root_sink — parallel ROOT recorder + live Δt monitor

A single C++ process that subscribes to the DELILA **merger's ZMQ PUB** as an
*additional* consumer and does two jobs at once:

1. **Recorder** — decode every event to five scalar fields and write a flat,
   ZSTD-compressed ROOT `TTree` (one file per run). Cheaper than the two-step
   `.delila` → `delila2root` path when only scalars are needed.
2. **Monitor** — a coincidence Δt monitor (gamma vs ThGEM1/ThGEM2 on configurable
   channels) served live over `THttpServer`. Browse it with JSROOT — zero
   frontend code.

The Rust **Recorder that writes `.delila` stays the authoritative recorder**.
`root_sink` is a parallel best-effort sink: it never gates, throttles, or blocks
the pipeline, so it cannot cause data loss upstream (see *Design notes*).

## Requirements

- **ROOT** 6.20+ (for ZSTD). Provides `TFile`/`TTree`/`THttpServer`.
- **libzmq** 4.3.x (the C API only — no `cppzmq`).
- A C++17 compiler.

`sink_core.hpp` (all the logic: envelope parse, decode, matcher, run state) needs
**neither ROOT nor ZMQ** — only `../delila2root/TDelila.hpp` and a C++17
compiler — so it is unit-testable on any box.

## Build

Standard (system libzmq, e.g. `apt install libzmq3-dev`):

```sh
g++ -O2 -std=c++17 root_sink.cxx $(root-config --cflags --libs) -lRHTTP -lzmq -o root_sink
```

No-sudo variant used on **gant / side3** (no `-dev` package): drop a matching
`zmq.h` (libzmq 4.3.x) into `~/.local/include` and link the runtime `.so`
directly:

```sh
g++ -O2 -std=c++17 -I$HOME/.local/include root_sink.cxx \
    $(root-config --cflags --libs) -lRHTTP \
    /lib/x86_64-linux-gnu/libzmq.so.5 -o root_sink
```

On **side3** ROOT is `/opt/ROOT` and the binary goes in `~daq/.local/bin` (same
convention as `delila2root`); on **gant** source `thisroot.sh` first
(`.bashrc` early-returns for non-interactive shells).

## Usage

```
root_sink [options]
  --zmq ADDR          merger PUB endpoint          (default tcp://localhost:5557)
  --out-dir DIR       output directory             (default .)
  --tree NAME         TTree name                   (default tr)
  --gamma-ch N        gamma detector channel       (enables the Δt monitor)
  --thgem1-ch N       ThGEM1 channel
  --thgem2-ch N       ThGEM2 channel
                      all three required; if any is omitted -> recorder only
  --window-ns X       coincidence half-window      (default 1000)
  --margin-ns X       out-of-order tolerance       (default 10000)
  --http-port N       THttpServer port, 0 disables (default 8090)
  --dt-bins N         Δt histogram bins            (default 2000)
  --dt-min X          Δt axis minimum, ns          (default -1000)
  --dt-max X          Δt axis maximum, ns          (default 1000)
  --autosave-sec N    TTree AutoSave interval      (default 30)
  --help
```

Typical ThGEM run (single V1730, gamma on ch0, ThGEMs on ch1/ch2):

```sh
root_sink --zmq tcp://localhost:5557 --out-dir /data/thgem \
          --gamma-ch 0 --thgem1-ch 1 --thgem2-ch 2 \
          --window-ns 500 --http-port 8090
```

Recorder-only (no monitor), just the scalar tree:

```sh
root_sink --out-dir /data --http-port 0
```

## Output files

While a run is in progress the tree is written to
`<out-dir>/run_inprogress_<unixtime>.root` and `AutoSave`d every `--autosave-sec`
so it is already openable in ROOT. On the run's `EndOfStream` the file is closed
and renamed to `run%04d_scalar.root` (using the EOS-carried run number;
collisions get a `_1`, `_2`, … suffix). The tree has exactly:

| branch         | leaf | type      |
|----------------|------|-----------|
| `module`       | `/b` | `UChar_t` |
| `channel`      | `/b` | `UChar_t` |
| `energy`       | `/s` | `UShort_t`|
| `energy_short` | `/s` | `UShort_t`|
| `timestamp_ns` | `/D` | `Double_t`|

If the process is stopped mid-run (Ctrl-C / SIGTERM), the file is written and
closed but **keeps its `run_inprogress_*` name** (it was never finalized) — a log
line reports it.

## Live monitor (THttpServer / JSROOT)

Point a browser at `http://<host>:8090/`. ROOT serves the built-in JSROOT UI;
click a histogram to draw it, and it updates live (JSROOT's *Monitor* toggle).
Registered objects:

- **`dt1`**  — `t(ThGEM1) − t(gamma)` [ns]
- **`dt2`**  — `t(ThGEM2) − t(gamma)` [ns]
- **`dt2_vs_dt1`** — 2D, X = Δt₁, Y = Δt₂ (each axis capped at 500 bins)
- **`channels`** — channel occupancy 0..63 (a freebie; filled for every event)
- **`/Reset`** — a command button that zeroes all four histograms

Histograms are **never auto-cleared on a run boundary** — they accumulate physics
across runs until you hit `/Reset` (a log line marks each boundary). The
coincidence *timing* state, on the other hand, is reset per run, because the
digitizer clock can restart between runs.

## Design notes

- **Parallel sink, `.delila` untouched.** `root_sink` is one more ZMQ subscriber.
  It uses `ZMQ_RCVHWM = 0` (the project's HWM=0 rule) so ZMQ never drops on its
  socket; if it ever falls behind, the merger's PUB simply buffers for it and the
  authoritative `.delila` Recorder is unaffected. Losing the monitor never loses
  data.
- **Wire format.** Each ZMQ message is one Rust `Message` enum value, encoded by
  `rmp-serde` as `fixmap(1){ variant : payload }`:
  - `Data` → `EventDataBatch` = positional array(4) `[source_id, sequence_number,
    timestamp, events]`; each `EventData` is a positional array whose first five
    fields are `module, channel, energy, energy_short, timestamp_ns` (the rest —
    `flags`, `user_info`, `waveform` — are skipped). This mirrors
    `src/common/delila_schema.rs`; unlike a `.delila` file the ZMQ stream carries
    no schema header, so the order is hardcoded in `sink_core.hpp`.
  - `EndOfStream` → array(2) `[source_id, run_number]`.
  - `Heartbeat` → skipped. Unknown variants → one-shot warning, then skipped.
- **Run-boundary semantics.** First `Data` opens a file; the run closes once
  **every** `source_id` seen in `Data` has sent its `EndOfStream`. Single source
  ⇒ one EOS closes. This set-based rule deliberately avoids the first-EOS-latch
  trap of naive multi-source consumers. A stale EOS received while idle is logged
  and ignored.
- **Monitor disabled without channel flags.** Omit any of `--gamma-ch /
  --thgem1-ch / --thgem2-ch` and the Δt matcher is off (recorder-only). The
  `THttpServer` (if `--http-port` ≠ 0) still shows channel occupancy.
- **Threading.** HTTP requests are serviced on the main thread via
  `gSystem->ProcessEvents()`, the same thread that `Fill`s the histograms — so
  there is no lock needed between serving and filling.
- **Reuses `TDelila.hpp`.** The MessagePack reader (`tdelila::mp::Reader`, typed
  fast-path decode) is shared by `#include`, not copied. `sink_core.hpp`'s event
  decoder must stay in sync with `TDelila`'s `Schema::build_default()` and with
  `src/common/delila_schema.rs`.

## Testing

`sink_core.hpp` is pure logic and compiles with plain `g++ -std=c++17`. A
self-test (envelope parse, batch decode, coincidence matcher, run state) can be
built against just this header and `../delila2root/TDelila.hpp` — no ROOT, no ZMQ.
