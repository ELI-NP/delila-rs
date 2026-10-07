# root_sink — parallel ROOT recorder + live monitor + event builder

A single C++ process that subscribes to the DELILA **merger's ZMQ PUB** as an
*additional* consumer and does two jobs at once:

1. **Recorder** — ZSTD-compressed ROOT `TTree`s, one file per run, each tree
   opt-in: the per-hit tree (`--hits-tree`, five scalars per hit) and the
   generic `events` tree (`--event-def`, TODO 71). At least one is required —
   with none, root_sink refuses to start (exit 2). Experiment-specific
   reconstruction runs offline on the events (ThGEM: `thgem_events.C`, see
   *Generic events*).
2. **Monitor** — a coincidence Δt monitor (gamma vs ThGEM1/ThGEM2 on configurable
   channels) served live over `THttpServer`. Browse it with JSROOT — zero
   frontend code.

The Rust **Recorder that writes `.delila` stays the authoritative recorder**.
`root_sink` is a parallel best-effort sink: it never gates, throttles, or blocks
the pipeline, so it cannot cause data loss upstream (see *Design notes*).

Japanese operator manual: [docs/root_sink_manual.md](../../docs/root_sink_manual.md)
(運用手順・ヒストグラム定義・トラブルシューティング).

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
  --hits-tree         write the per-hit tree — OPT-IN since TODO 71 (the
                      .delila holds the time series; delila2root reproduces it)
  --tree NAME         hits tree name               (default delila, same as delila2root)
  --exp-name NAME     experiment name for the output filename (override)
  --operator URL      operator base URL, e.g. http://localhost:9092; experiment
                      name is read from <URL>/api/status at run start (only when
                      --exp-name is not given)
  --hists FILE        histogram definition JSON (see below); replaces the
                      built-in dt1/dt2/dt2_vs_dt1/channels set
  --module M          process only digitizer M's hits (default: all) — the
                      trees, histograms and matchers alike;
                      required with >1 digitizer (see Design notes)
  --gamma-ch N        gamma detector channel       (enables the Δt monitor)
  --thgem1-ch N       ThGEM1 channel
  --thgem2-ch N       ThGEM2 channel
                      all three required; if any is omitted -> recorder only
  --xy1-ch T,XL,XR,YU,YD  delay-line XY monitor 1: trigger + 4 arm channels
  --xy2-ch T,XL,XR,YU,YD  delay-line XY monitor 2
                      5 distinct ints 0..255 each; fills pos1/pos2-scope
                      histograms, so --hists is required (no built-ins)
  --window-ns X       coincidence half-window      (default 1000)
  --margin-ns X       out-of-order tolerance       (default 10000)
  --http-port N       THttpServer port, 0 disables (default 8090)
  --dt-bins N         Δt histogram bins            (default 2000)
  --dt-min X          Δt axis minimum, ns          (default -1000)
  --dt-max X          Δt axis maximum, ns          (default 1000)
  --autosave-sec N    TTree AutoSave interval      (default 30)
  --workers N         event-build worker threads   (default 3)
  --chunk-span-ms X   sorter chunk span, DATA time (default 100)
  --safe-horizon-ms X sorter disorder absorption   (default 50; must exceed the
                      stream's max arrival disorder — side3 measured ~8 ms)
  --time-calib FILE   per-(module,channel) time offsets (timeSettings.json, or
                      legacy time-calib output); aligned = raw - offset_ns for
                      EVERY consumer. Re-read at every run start, frozen per run
                      (see "Time offsets" below)
  --event-def FILE    generic event definition JSON -> the "events" tree. Re-read
                      at every run start, frozen per run (see "Generic events")
  --help
```

At least one of `--hits-tree`, `--event-def` must be given: since TODO 71
(2026-10) the hits tree is no longer written implicitly, and a command line
selecting no tree exits 2 with an ERROR saying so (`--tree NAME` alone only
warns that the hits tree stays off). `--built-tree` was retired (TODO 71):
it exits 2 pointing to `thgem_events.C`, and `start_daq.sh` warns about and
skips a TOML `built_tree`.

Typical ThGEM run (single V1730, gamma on ch0, ThGEMs on ch1/ch2):

```sh
root_sink --zmq tcp://localhost:5557 --out-dir /data/thgem --hits-tree \
          --gamma-ch 0 --thgem1-ch 1 --thgem2-ch 2 \
          --window-ns 500 --http-port 8090
```

Recorder-only (no monitor), just the scalar tree:

```sh
root_sink --out-dir /data --hits-tree --http-port 0
```

### Launched by `start_daq.sh`

When the DAQ config has a `[network.root_sink]` section, `scripts/start_daq.sh`
launches root_sink automatically alongside the other components. The section keys
are the CLI flag names with hyphens turned into underscores (`--out-dir` →
`output_dir`; exceptions: `subscribe` → `--zmq`, `time_calib_file` →
`--time-calib` (named like the Rust event builder's key), `event_def_file` →
`--event-def`, and the boolean `hits_tree = true` → `--hits-tree` (`false` or
absent: no flag)); only present keys are passed, and `--operator` is derived from the
`[operator]` port. The binary is resolved in the order `ROOT_SINK_BIN` (env) >
`PATH` > `~/.local/bin/root_sink` > `tools/root_sink/root_sink`; if none is found
it warns and skips (not fatal). It is managed symmetrically: both
`start_daq.sh` and `stop_daq.sh` now `pkill -x root_sink`, so a stale sink is
cleaned up on restart and stopped on shutdown. See
[docs/root_sink_manual.md](../../docs/root_sink_manual.md) for the section
example and operator notes.

## Output files

While a run is in progress the selected trees (all in one file) are written to
`<out-dir>/run_inprogress_<unixtime>.root` and `AutoSave`d every `--autosave-sec`
so it is already openable in ROOT. On the run's `EndOfStream` the file is closed
and renamed to

```
run%04u_0000_<exp>.root
```

using the EOS-carried run number. This is **identical to the Rust Recorder's
`.delila` filename** (`run{run:04}_{seq:04}_{exp}.delila`) but for the extension:
`root_sink` does not split runs itself, so the sequence field is normally the
literal `0000`. On a name collision it appends `_<unix_ns>` (nanoseconds since the
epoch) before the extension — the same collision scheme the Rust Recorder uses.

One caveat: ROOT itself auto-splits a `TTree`'s file when it crosses
`TTree::GetMaxTreeSize`. root_sink raises that limit to **2 TB** (unreachable
for scalar data — ~10^11 events), and if it is ever crossed anyway, the
ROOT-made continuation files (`<stem>_1.root`, …) are renamed at finalize with
Recorder-style sequence numbers (`run%04u_0001_<exp>.root`, …) plus a WARNING —
verified end-to-end with a tiny test threshold (see *Testing*). The rollover
also invalidates the original `TFile*` (ROOT deletes it in `ChangeFile`), which
is why all closing goes through `TTree::GetCurrentFile()` and rollover detection
compares file NAMES, never pointers (the allocator can reuse the address).
`ChangeFile` moves every tree of the file together, so any tree that exists
reports the current file — the Recorder asks the first one present (hits,
events; both are optional).

`<exp>` (the experiment name) is resolved once at each run start, in priority
order — the resolved name and its source are logged, and any fallback is warned
(never silent):

1. `--exp-name NAME` — explicit override, wins always.
2. `--operator URL` — HTTP `GET <URL>/api/status` and take the top-level
   `experiment_name` string. In normal UI-driven operation this matches the Rust
   Recorder's filename by construction. The fetch is bounded to ~2 s (a
   non-blocking connect + socket timeouts); ZMQ (HWM=0) buffers batches while it
   runs, so it never hangs the sink. Only `http://` is supported — `https://` is
   rejected at startup.
3. Neither given, or the fetch failed / returned no name → `"data"` (with a
   warning to stderr). This mirrors the Rust Recorder's empty-name fallback.

The hits tree (`--hits-tree`) has exactly:

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

**Time offsets.** With `--time-calib` the `timestamp_ns` written here (and every
time in the events tree) is **aligned**: `raw - offset_ns`. The `.delila` stays
raw, so a delila2root tree of the same run differs on the offset channels. Every
file says which applies:

- every tree (hits, events) carries a `TNamed "timeSettings"` in its **UserInfo**
  (`tree->GetUserInfo()->FindObject("timeSettings")->GetTitle()`): a JSON with
  `applied` (true/false), the convention, the source path, `status` (`ok` or
  `stale: …`), the resolved absolute offsets (`%.17g`) and the verbatim source.
  It is part of the tree, so it is in every AutoSave and every rollover part;
- with offsets applied, the hits-tree title says `timestamp_ns ALIGNED`;
- at finalize, `<run file stem>_timeSettings.json` is written next to the run
  file: the table **frozen at that run's start** (unlike `_hists.json`, which
  copies the live file at finalize), directly re-loadable by root_sink or the
  Rust EB.

## Generic events (`--event-def`, TODO 71)

The experiment-independent "grouping" layer of event building: triggers and a
window around them bundle hits into events (TODO/71_generic_event_building.md
§4–5). Experiment-specific reconstruction (ThGEM XY/TOF, ELIADE addback, …)
runs offline on top of it (ThGEM: *ThGEM reconstruction* below). The
definition file:

```json
{"version": "1.0",
 "window_ns": {"pre": 1000, "post": 1000},
 "group_ns": 1000,
 "triggers": [
   {"name": "ThGEM1", "priority": 1, "channels": [[0, 1]]},
   {"name": "ThGEM2", "priority": 2, "channels": [[0, 6]]}]}
```

`channels` are `[module, channel]` pairs; a smaller `priority` is the
preferred reference — give it for all triggers or for none (none = the first
trigger of a group is the reference); `_comment` is allowed anywhere. The
ThGEM file is `config/event_def_thgem.json`; the full rules and validation
(every error listed at once, unknown keys rejected, `group_ns ≤ min(pre,
post)`, one trigger per channel, the lookback below `--safe-horizon-ms`) are
in the header comment of `event_def.hpp`. Version `"1.1"` adds a channel
table, anti-Compton flags and L2 (see *Tags, anti-Compton and L2* below); a
1.0 file reads unchanged. The rules:

1. Triggers in time order; an ungrouped trigger `t` opens the group `[t, t+G]`
   (`G = group_ns`, never extended). Every trigger is in exactly one event — no
   chain-loss as with ELIFANT's ID-priority scheme.
2. The group's reference = its best-priority trigger (tie → the earlier).
3. The window is centred on the reference: `[t_ref − pre, t_ref + post]`.
4. A hit inside two windows goes to the NEAREST reference (tie → the earlier);
   a hit in no window is an **orphan** — in no event, still in the `.delila`
   (and the hits tree). Energies are never cut.

Rules 1–2 are sequential, so the Sorter runs them on every chunk in order
(the RunClose / Shutdown flush chunk closes the last groups); rules 3–4 run
in the Workers, and an event is emitted by the chunk whose core holds its
reference (H10), so the output does not depend on `--workers`. The Sorter's
context carry becomes `max(--window-ns, lookback)` per run, lookback = `G +
2·pre + post` (+ `ac_pre + ac_post` with anti-Compton), logged as `sorter
lookback` when it changes. Times are on the aligned axis (`--time-calib` is
applied before the Sorter).

The `events` tree (fixed name; one entry per event, dictionary-free leaflists,
hit arrays in time order):

| branch | leaf | content |
|---|---|---|
| `EventID` | `/l` | per-run counter from 0 (continues across rollover parts) |
| `TriggerTime` | `/D` | the reference trigger's absolute aligned time [ns] |
| `TriggerMod`, `TriggerCh` | `/b` | the reference trigger's channel |
| `TriggerIdx` | `/b` | its index in the file's `triggers[]` (UserInfo lists them) |
| `NTriggers` | `/i` | triggers in the group (≥ 1) |
| `NHits` | `/i` | hits in the event, the reference included |
| `HitMod[NHits]`, `HitCh[NHits]` | `/b` | per hit |
| `HitEnergy[NHits]`, `HitEnergyShort[NHits]` | `/s` | per hit |
| `HitTime[NHits]` | `/D` | hit time − `TriggerTime` [ns] (the reference is at 0) |
| `HitIsTrigger[NHits]` | `/O` | a trigger hit of this event's group |
| `HitWithAC[NHits]` | `/O` | an anti-Compton shield fired around this hit (always present; all false without `anti_compton`) |
| `<counter>` | `/i` | L2 counter value (only with an `l2` section; named by the definition) |
| `<flag>`, `<accept>` | `/O` | L2 flag / accept result (likewise) |
| `L2Accept` | `/O` | OR of all accepts (when at least one accept exists) |

Leaf types for `SetBranchAddress`: `/l` = `ULong64_t`, `/i` = `UInt_t` (NOT
`Int_t` — ROOT refuses the mismatch), `/b` = `UChar_t`, `/s` = `UShort_t`,
`/O` = `Bool_t`; size the hit arrays with `events->GetMaximum("NHits")`.

```cpp
events->Draw("HitTime", "HitCh==0 && TriggerIdx==0");  // LaBr3 vs a ThGEM1 reference
events->Draw("NTriggers");                              // overlap / pile-up of triggers
```

The array buffers grow on demand (never truncated). **Provenance** works like
the time offsets: the file is re-read at every run start and frozen for the
run (a failed reload keeps the previous definition: ERROR, `STALE` in the run
line, status line and UserInfo); the events tree's UserInfo carries a `TNamed
"eventDef"` JSON (status, source path, resolved window/group/triggers with
their `TriggerIdx`, warnings, verbatim source) next to `"timeSettings"`, and
finalize writes `<run file stem>_eventDef.json` — the file frozen at that run's
start, byte for byte.

**Counters** (the overlap watch). Each run logs

```
root_sink: run 56 events: 33463 (NTriggers 1:33463), hits in events 163845, contested 0, orphans 42105
```

— the NTriggers distribution (zero bins skipped, last bin `8+`), the hits put
into events, **contested** = non-trigger hits that sat inside two windows and
went to the nearer reference (it rises when windows start to overlap, i.e.
with rate), and the orphans. `hits in events + orphans` must equal the hits
the Sorter emitted (exactly-once); a mismatch is a WARNING. The 10 s status
line shows the cumulative `ev=N (cont=C orph=O)`, plus `STALE` when the last
reload failed. A mid-run shutdown logs the same line for the unfinalized run.

### Tags, anti-Compton and L2 (`"version": "1.1"`, TODO 71 Phase C)

Design and the choices made: `TODO/71_phase_c_proposal.md` (§2.3 branches,
§2.4 semantics). All three sections are optional:

```json
{"version": "1.1",
 "window_ns": {"pre": 2000, "post": 2000}, "group_ns": 2000,
 "tags": {"core": [[0, 0], [1, 0]], "seg": [[0, 2, 9], [1, 2, 9]],
          "C1": [[0, 0, 9], [1, 0, 9]], "BGO1": [[8, 0, 3]]},
 "triggers": [{"name": "Ge", "tags": ["core"]}],
 "anti_compton": {"window_ns": {"pre": 300, "post": 200},
                  "pairs": [{"name": "C1", "detector": ["C1"], "shield": ["BGO1"]}]},
 "l2": {"counters": [{"name": "nGe", "tags": ["core"]},
                     {"name": "nGeClean", "tags": ["core"], "ac": "clean"}],
        "flags":    [{"name": "GeGe", "counter": "nGeClean", "op": ">=", "value": 2}],
        "accept":   [{"name": "GammaGamma", "op": "AND", "flags": ["GeGe"]}]}}
```

- **`tags`** — tag → channels; `[m, c]` or an inclusive range `[m, first,
  last]`. `triggers[].tags` adds the tags' channels to `channels` (still one
  trigger per channel). The resolved table is in the provenance.
- **`anti_compton`** — a hit on a pair's `detector` channel gets `HitWithAC`
  when a hit on that pair's `shield` channels lies in `[t − pre, t + post]` of
  **the flagged hit itself** (inclusive), whatever event — or none — that
  shield hit went to. It does not depend on triggers or windows (unlike
  ELIFANT's "partner inside the same event", which would let a Ge pass when
  rule 4 gave its BGO to the neighbouring event). Nothing is dropped: the
  shield hits stay in their events, and `HitWithAC` is only a mark. Pairs are
  many-to-many (≤ 64), but a channel may not be detector and shield of the same pair.
- **`l2`** — evaluated per event on its own hits, **flag only: every event is
  written**, select with the branches (`events->Draw("HitEnergy",
  "L2Accept")`). A counter counts HITS on its tags' channels (a channel that
  fired twice counts 2); `"ac"` = `"any"` (default) / `"clean"` (no
  `HitWithAC`) / `"vetoed"`. A flag compares one counter (`== != < <= > >=`)
  with an integer; an accept ANDs or ORs flags; `L2Accept` = OR of all
  accepts. Every reference is resolved at load time — an unknown tag /
  counter / flag is a load error, never a silent false.
- **Branch names** are the definition's names, so the tree's L2 branches are
  fixed per run (frozen at RunOpen; a TChain over runs with different
  definitions needs care — the provenance says which). Names must be C
  identifiers, unique over counters/flags/accepts and none of the fixed
  branches (`event_def.hpp` `edef_detail::is_fixed_branch`; the Writer checks
  its own branch list against it at the first run and prints an ERROR on drift).
- **Chunk contract.** Deciding `HitWithAC` for a hit near the end of an
  event's window needs shield hits up to `ac_post` later, so a chunk stops
  emitting events `ac_post` earlier than in Phase A, and the lookback grows by
  `ac_pre + ac_post` (ThGEM without AC: unchanged; the 140Ce example below:
  8500 ns vs 8000).
- **Counters.** The run summary appends `AC-flagged hits N` (hits in events
  with `HitWithAC`) and adds a line
  `run N L2: <flag>=<events> … | <accept>=<events> … | L2Accept=<events>`; the
  status line gains `ac=` / `l2acc=` inside `ev=(…)` while the run's
  definition has them.

### ThGEM reconstruction (`thgem_events.C`)

The ThGEM `built` tree is no longer written by the sink (retired in TODO 71;
`--built-tree` exits 2). `thgem_events.C` rebuilds it offline from a run's
`events` tree into a separate file — same branches as the old sink tree, the
input's UserInfo (`timeSettings`, `eventDef`) copied:

```sh
root -l -b -q 'thgem_events.C+("run0056_0000_X.root","run0056_0000_X_built.root")'
#   further args: xy1 "T,XL,XR,YU,YD" (default "1,2,3,5,4"), xy2 (default
#   "6,7,8,9,10"; "" = single plane), gamma ch (default 0; -1 = no LaBr3/TOF),
#   window_ns (arm / LaBr3 match window, default 1000)
```

One row per event holding a ThGEM trigger (`thgem_from_event`,
`thgem_reco.hpp`): per plane the arm hits nearest to that plane's own trigger
within ±window (the offline ground-truth rule of `macros/grid_resolution.C`),
the LaBr3 hit nearest to the earlier trigger; a missing piece is `NaN`
(times/derived) or `0` (energies) — completeness is an offline cut, partial
rows ARE the arm-efficiency measurement. Branches: `trig1_t trig2_t labr_t /D`,
`x1 y1 x2 y2 /F` (ns, aligned axis; `x = t_XR − t_XL`, `y = t_YU − t_YD`),
per-arm `dt_* /F` + `e_* /s`, `dt_trig/F` (`t(det2) − t(det1)`), `tof1 tof2 /F`
(raw `t(trig) − t(labr)`), `e_trig* e_labr /s`, `e_sum1 e_sum2 /i`,
`n_arms1 n_arms2 /b`. Typical cuts: 4-fold plane 1 = `!TMath::IsNaN(x1) &&
!TMath::IsNaN(y1)`; both-plane track = `!TMath::IsNaN(dt_trig)`.

It reproduces the retired sink tree bit for bit (run 56: 33,463 rows; run0023:
40,045 rows, 15,644 four-fold) when the event window covers `group_ns +
window_ns` on both sides and no hit is contested; two triggers of one plane in
one event make one row (the earliest). `delayline_offsets.C`'s
`delayline_centres_built()` reads its output file directly.

## Live monitor (THttpServer / JSROOT)

Point a browser at `http://<host>:8090/`. ROOT serves the built-in JSROOT UI;
the whole tree auto-refreshes every 2 s (`_monitoring`), and each 2D histogram
draws with `colz` by default (`_drawopt`). The default (no `--hists`) objects:

- **`dt1`**  — `t(ThGEM1) − t(gamma)` [ns]
- **`dt2`**  — `t(ThGEM2) − t(gamma)` [ns]
- **`dt2_vs_dt1`** — 2D, X = Δt₁, Y = Δt₂ (each axis capped at 500 bins)
- **`channels`** — channel occupancy 0..63 (a freebie; filled for every event)
- **`/Reset`** — a command button that zeroes every currently live histogram

Histograms are **never auto-cleared on a run boundary** — they accumulate physics
across runs until you hit `/Reset` (a log line marks each boundary). The
coincidence *timing* state, on the other hand, is reset per run, because the
digitizer clock can restart between runs.

## Custom histograms (`--hists FILE`)

By default the four objects above are hard-coded. Pass `--hists FILE` and that
JSON file **fully defines** the displayed set instead (the built-ins are not
created). Without `--hists` behaviour is exactly as before — full backward
compatibility. `channels` is expressible as `x = channel`, so there is no special
case. If `--hists` is given while the HTTP server is disabled (`--http-port 0`)
the file is warned about and ignored (histograms only live on the server). If the
file has coincidence histograms but the Δt matcher is off (no `--gamma-ch /
--thgem1-ch / --thgem2-ch`), a prominent startup warning notes they will stay
empty (not fatal).

Any parse error at startup prints **all** problems and exits with code 2. On
finalize the current histogram file is copied next to the run's ROOT file as
`<final root path minus ".root">_hists.json`, so each run keeps a snapshot of the
config that produced it (it inherits any collision suffix and stays paired).

### File format

Top-level object with a `histograms` array. Each entry is one histogram:

```json
{ "histograms": [
  { "name": "dt1", "type": "TH1D", "fill": "dt1", "bins": 2000, "min": -1000, "max": 1000 }
] }
```

Common keys: `name` (required, ROOT-safe: non-empty, no `/`, unique), `title`
(optional, defaults to `name`; ROOT parses an embedded `";xaxis;yaxis"`), `type`
(`"TH1D"` or `"TH2D"`), `drawopt` (optional; empty means `colz` for 2D, nothing
for 1D).

- **1D (`TH1D`)** — `x` (the fill variable) plus `xbins` / `xmin` / `xmax`.
  Aliases: `fill` for `x`, and `bins` / `min` / `max` for the axis keys.
- **2D (`TH2D`)** — `x`, `y`, and `xbins/xmin/xmax` + `ybins/ymin/ymax`
  (no aliases).

Validation collects every error at once: unknown JSON keys (typo protection),
unknown type/variable, duplicate or unsafe names, `bins ≤ 0`, `min ≥ max`, and
**scope mixing** (see below).

### Vocabulary

Every histogram lives entirely in one **scope**; `x`, `y`, and the cut must all
belong to the same scope (mixing is an error). This is what makes "gate Δt on
gamma energy" expressible.

**hit scope** — one row per decoded event:

| kind     | names |
|----------|-------|
| variable | `energy`, `energy_short`, `channel`, `module` |
| cut      | `channel` (int: keep events on that channel), `energy_range` (`[min,max]` inclusive) |

**coinc scope** — one row per *ripe gamma* coincidence result (needs the Δt
matcher enabled):

| kind     | names |
|----------|-------|
| variable | `dt1`, `dt2`, `gamma_energy`, `thgem1_energy`, `thgem2_energy` |
| cut      | `gamma_energy_range`, `thgem1_energy_range`, `thgem2_energy_range` (each `[min,max]` inclusive) |

Coinc semantics: `gamma_energy` is always present for a ripe gamma; `dt1` /
`thgem1_energy` only fill when a ThGEM1 partner was found (likewise dt2/ThGEM2). A
`thgem1_energy_range` cut therefore fails when that partner is absent. At most one
cut key per histogram.

**pos1 / pos2 scopes** — one row per *ripe trigger* of the corresponding
delay-line XY monitor (pos1 needs `--xy1-ch`, pos2 needs `--xy2-ch`):

| kind     | names |
|----------|-------|
| variable | pos1: `xy1_x`, `xy1_y` — pos2: `xy2_x`, `xy2_y` |
| cut      | (none yet) |

Pos semantics: `xy1_x = t(XR) − t(XL)` fills only when **both** X arms matched
the trigger within ±window; `xy1_y = t(YU) − t(YD)` likewise needs both Y arms.
A 2D `x:"xy1_x", y:"xy1_y"` image therefore requires the full 4-arm coincidence,
while a 1D projection fills on its own arm pair. There are no built-in fallback
histograms for these scopes — the XY monitors only display through `--hists`.

### Standard config (reproduces the built-ins)

The repo ships `histograms.json` that reproduces the four built-in histograms
exactly — `--hists histograms.json` renders the same display as the defaults:

```json
{ "histograms": [
  { "name": "dt1",        "type": "TH1D",
    "title": "t(ThGEM1) - t(gamma) [ns];#Deltat_{1} [ns];counts",
    "fill": "dt1",     "bins": 2000, "min": -1000, "max": 1000 },
  { "name": "dt2",        "type": "TH1D",
    "title": "t(ThGEM2) - t(gamma) [ns];#Deltat_{2} [ns];counts",
    "fill": "dt2",     "bins": 2000, "min": -1000, "max": 1000 },
  { "name": "dt2_vs_dt1", "type": "TH2D",
    "title": "#Deltat_{2} vs #Deltat_{1};#Deltat_{1} [ns];#Deltat_{2} [ns]",
    "x": "dt1", "y": "dt2",
    "xbins": 500, "xmin": -1000, "xmax": 1000,
    "ybins": 500, "ymin": -1000, "ymax": 1000 },
  { "name": "channels",   "type": "TH1D",
    "title": "channel occupancy;channel;counts",
    "fill": "channel", "bins": 64, "min": 0, "max": 64 }
]}
```

### Advanced examples

Add these to your own `--hists` file (they are examples, not part of the standard
config):

Gamma energy spectrum, only for events on channel 0 (hit scope, `channel` cut):

```json
{ "name": "E_gamma", "type": "TH1D", "x": "energy", "channel": 0,
  "xbins": 4096, "xmin": 0, "xmax": 16384 }
```

Gamma energy vs Δt₁ — both axes are coinc-scope, so this is allowed:

```json
{ "name": "E_vs_dt1", "type": "TH2D", "x": "dt1", "y": "gamma_energy",
  "xbins": 400, "xmin": -1000, "xmax": 1000,
  "ybins": 512, "ymin": 0, "ymax": 16384 }
```

Δt₁ gated by gamma energy (the headline use case):

```json
{ "name": "dt1_gated", "type": "TH1D", "fill": "dt1",
  "bins": 400, "min": -200, "max": 200,
  "gamma_energy_range": [800, 1200] }
```

Delay-line XY image (pos1 scope; needs `--xy1-ch T,XL,XR,YU,YD`). Axis ranges
follow the delay line: ±window covers any arm, the physical spread is what
matters (the ThGEM measured ±250 ns):

```json
{ "name": "xy1_pos", "type": "TH2D",
  "title": "ThGEM1 XY;x = t_{XR} - t_{XL} [ns];y = t_{YU} - t_{YD} [ns]",
  "x": "xy1_x", "y": "xy1_y",
  "xbins": 500, "xmin": -250, "xmax": 250,
  "ybins": 500, "ymin": -250, "ymax": 250 }
```

### Reloading live (`/ReloadHists`)

With `--hists`, a **`/ReloadHists`** command button appears (and can be triggered
without the UI):

```sh
curl 'http://<host>:8090/ReloadHists/cmd.json'
```

It re-reads the file: on success the old histograms are unregistered and the new
set is built + registered (drawopt applied); on **any** parse error it prints them
all and **keeps the current set** unchanged. A one-line summary is logged either
way. Edit the file, hit reload — no restart, no lost data.

`/Reset` is unchanged: it zeroes every currently live histogram (built-in or
dynamic). Both commands run on the main loop (a flag flipped by the button), so a
reload can safely delete and rebuild the live objects.

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
- **One digitizer per root_sink (`--module`).** The Δt / XY matchers select
  hits by `channel` alone, and the Sorter and the watermark matchers assume
  ONE time base. A second board
  breaks both: its channels land in the ThGEM/LaBr3 slots, and an independently
  started board's clock drifts away from the first. Once the drift exceeds
  `--safe-horizon-ms`, the lagging board's hits arrive behind an emitted core
  and never reach the `.root` (E2E, 2026-10-05: board 0 lagging 200 ms → 1489
  of 4000 hits missing). `--module M` drops the other boards right after
  decoding, before any consumer. Hits behind an emitted core are counted
  (`late=` in the status line, one WARNING per run) — never silently lost; the
  `.delila` from the Recorder has every board in any case.
- **Monitor disabled without channel flags.** Omit any of `--gamma-ch /
  --thgem1-ch / --thgem2-ch` and the Δt matcher is off (recorder-only). The
  `THttpServer` (if `--http-port` ≠ 0) still shows channel occupancy.
- **Thread pipeline (TODO 66 §5).** Since the 2026-08 restructure root_sink runs
  five thread roles:

  ```
  Receiver ──unbounded──▶ Sorter ──unbounded──▶ work_q ×N workers ──▶ Writer
      │                                                    (seq reorder inside)
      └── bounded(1000) tee ──▶ Display(main): TH1s + matchers + THttpServer
  ```

  - **Receiver** owns the ZMQ socket + run state; it decodes batches into owned
    hits and sends control (RunOpen/RunClose/Shutdown) IN-BAND with the data.
  - **Sorter** absorbs arrival disorder (`--safe-horizon-ms`) and emits
    time-sorted chunks per `--chunk-span-ms` of DATA time, each with a sequence
    number. **The hits tree is therefore globally time-sorted now** — offline
    analysis can scan linearly instead of per-channel bucketing. With
    `--event-def` it also runs the stateful trigger grouping (generic-event
    rules 1–2) on every chunk and attaches the plan to it.
  - **Workers** run the pure-function generic-event rules 3–4 (`eb_core.hpp`)
    per chunk.
  - **Writer** is the only thread touching the run `TFile`; a sequence reorder
    makes the output deterministic in `--workers` (1 or 8 → identical trees) and
    turns RunClose into a barrier for free. Autosave runs here.
  - **Display** (the main thread) owns every TH1 + the `THttpServer`
    (`ProcessEvents`), fed in arrival order by a bounded tee. Record-path
    queues are unbounded (the no-drop rule); display DATA may drop under
    overload — counted and shown as `display_drops` in the status line (the
    project's Monitor exception). Control markers never drop.
  - `TRootSniffer::SetScanGlobalDir(kFALSE)` is mandatory: the default scan
    would expose the Writer's live run file over HTTP (cross-thread reads from
    a browser click). Registered histograms are explicit, so nothing is lost.
- **Time offsets (`--time-calib`, TODO 70).** A constant propagation delay per
  `(module, channel)` (cables, delay lines) is removed once, in the Receiver,
  right after `--module` and BEFORE the Sorter, the display tee, the event
  layer and the hits tree — so all of them work on one aligned axis. It must stay before
  the Sorter: it sorts on `timestamp_ns`, and its core ranges / H10 gates use the
  same field. Format = the Rust EB's `timeSettings.json`
  (`src/event_builder/time_offsets.rs`, `schemas/timeSettings.schema.json`):
  tree form, absolute offset = sum along the `ref` path to the root (the root's
  own value included); the legacy `event_builder time-calib` output
  `{"ref_module","ref_channel","offsets":{"MM_CC":ns}}` is accepted too, chosen by
  which key is present (never by silent fallback). Convention
  `aligned_ts = raw_ts - offset_ns` — **positive = the channel arrives late**
  (as `src/event_builder/hit.rs`). Policy:
  - every error is collected and listed (duplicate / dangling ref / cycle /
    bad types / unknown keys / bad legacy keys); at startup any error is fatal
    (exit 2);
  - the file is re-read at every run start and frozen for the run, so an edit
    takes effect at the next run without a restart (write a temp file + `mv`);
    if that reload fails, the PREVIOUS table is kept for the run, an ERROR is
    logged, the status line shows `toff=N STALE` and the run's UserInfo says
    `stale` — never a silent fall back to "no offsets";
  - a spread (max − min offset) ≥ `--safe-horizon-ms` is an error: aligning adds
    that much arrival disorder, and the Sorter would drop hits as late (spread ≥
    `--margin-ns` only warns — display matchers);
  - a channel with no entry keeps its raw time and is reported once per run
    (WARNING + `toff=N (k unmapped)`).
  This is a correction of the data time axis, not an analysis constant (those —
  mm calibration, the −70 ns TOF display offset — stay in macros); note that
  moving a trigger's or the LaBr3 channel's offset shifts TOF. Offsets fix a
  CONSTANT delay only: independently started boards drift, which no table can
  correct (hardware clock sync, TODO 70 Phase 2).
  `delayline_offsets.C` measures the offsets that centre each delay-line image
  (split each axis' raw mid-of-edges centre over its two arms; triggers 0) and
  writes a ready-to-use file; run it on a root_sink `.root` to see the
  residuals.
- **Reuses `TDelila.hpp`.** The MessagePack reader (`tdelila::mp::Reader`, typed
  fast-path decode) is shared by `#include`, not copied. `sink_core.hpp`'s event
  decoder must stay in sync with `TDelila`'s `Schema::build_default()` and with
  `src/common/delila_schema.rs`.

## Testing

`sink_core.hpp` and `hist_config.hpp` are pure logic and compile with plain
`g++ -std=c++17` — no ROOT, no ZMQ. `test_sink_core.cpp` covers envelope parse,
batch decode, the coincidence matcher (with energies), run state, the HTTP
helpers (`split_http_response` / `extract_experiment_name`), and the histogram
config parser (all error classes + `value_of` / `pass_cut` for both scopes):

```sh
cd tools/root_sink
g++ -std=c++17 -O0 -g test_sink_core.cpp -o /tmp/ts && /tmp/ts
```

It prints `N passed, 0 failed` and exits non-zero on any failure.

The pipeline core (`eb_core.hpp`: Channel, SeqReorder, chunk splitter with the
H10 boundary rules, Sorter, the generic event layer; `thgem_reco.hpp`) has its own suite —
also plain g++, but `-pthread` (the Channel tests spawn threads):

```sh
cd tools/root_sink
g++ -std=c++17 -O0 -g -pthread test_eb_core.cpp -o /tmp/teb && /tmp/teb
```

For E2E work there is a committed replay feeder, `test_publisher.cxx`: it plays
the merger's role, republishing a recorded hits tree over ZMQ in the wire
format (stored order → real arrival disorder), with `--repeat K` (multi-run,
per-run clock restart), `--no-eos` (mid-run SIGTERM tests) and `--rate`:

```sh
g++ -O2 -std=c++17 test_publisher.cxx $(root-config --cflags --libs) -lzmq -o test_publisher
./test_publisher --file run0023_0000_X730_ThGEM_Test.root --run 23
```

`compare_hits.C` is the matching order-INDEPENDENT equality check between two
hits trees (per-channel counts, energy sums, timestamp XOR + monotonicity) —
the multithreaded sink writes the same hit SET as the old one but time-sorted,
so entry-by-entry diffing is useless. A sink run with `--time-calib` writes
aligned times: pass the same file as the 4th argument to align the raw tree
first (bit-identical for a correct run); without it the offset channels show
`tsxor` MISMATCH while counts and energy sums still agree:

```sh
root -l -b -q 'compare_hits.C("raw.root","aligned.root","delila","ts.json")'
```

The time-offset logic (`time_offsets.hpp`: parse/resolve/validate both formats,
apply, reload/STALE policy, provenance JSON) is covered in `test_sink_core.cpp`
(incl. a run-56 fixture that centres both ThGEM images), and its Sorter contract
(aligned before the Sorter, spread vs horizon) in `test_eb_core.cpp`. E2E
(2026-10-07, replay of pf-qaq run 56): no flag / all-zero file → IDENTICAL;
`delayline_offsets.C` table → IDENTICAL after alignment and both images centred
at (0.000, 0.000) ns with unchanged 4-fold counts; UserInfo in both trees and in
all 6 parts of a forced rollover; a run-start reload picks up an edit and a bad
edit keeps the previous table (STALE); resolved tables equal the Rust
`eb_offsets` output (incl. the 208-entry `tools/p91zr_test/timeSettings.json`).

```sh
root -l -b -q 'delayline_offsets.C("run0056_*.root","1,2,3,5,4","6,7,8,9,10","ts.json","run 56")'
root -l -b -q -e '.L delayline_offsets.C' -e 'delayline_centres_built("run0057_0000_X_built.root")'
```

(the `_built.root` file is `thgem_events.C`'s output).

Generic events (TODO 71) have a run-file check, ACLiC-compiled against
`eb_core.hpp`: `check_events.C` rebuilds the run's events from its hits tree in
ONE chunk (no chunk boundary at all) with the run's own `_eventDef.json` and
compares them entry by entry with the `events` tree; it also checks hits in
events + orphans == hits. Needs `--hits-tree`. Use it after a deployment's
first run.

```sh
root -l -b -q 'check_events.C+("run0056_0000_X.root")'
```

E2E (2026-10-07): run 56 (pf-qaq, offsets) and run0023 (side3) replays are
IDENTICAL to the single-chunk rebuild for workers 1/3/8, 0.2 ms chunks and
~130k chunks, also with LaBr3 as a 3rd trigger (7,177 two-trigger events, 44
contested hits). Historical (Phase B, before the retirement): the ThGEM rows
rebuilt from events matched the then in-sink built tree row by row — run0023's
15,644 four-fold events, all 40,045 rows (window 500, events ±1000 / G 500).

The `--event-def` wiring (E2E, 2026-10-07, replay of run 56 with `--hits-tree
--event-def config/event_def_thgem.json`): hits entries unchanged (205,950);
the `events` tree has the branches above, both UserInfo objects and a
byte-identical `_eventDef.json`; `hits in events + orphans` = 205,950. The
same events digest with the hit buffers starting at capacity 1 (growth path),
and from a forced-rollover build without the hits tree (`events` as the lead
tree; UserInfo in every part). A bad edit
between runs keeps the previous definition (STALE in the run line, status line
and UserInfo; the sidecar is the previous file).

Phase C wiring (E2E, 2026-10-07): run 56 with the unchanged ThGEM file →
the events content digest equals Phase A, `HitWithAC` present and all false,
no L2 branches, `check_events` IDENTICAL. ELIFANT p+¹⁴⁰Ce run0112 (41.3 M hits,
11 modules) with a v1.1 definition generated from its `chSettings.json` (the
Phase A triggers/window, 24 AC pairs = every `HasAC` channel → the BGO with the
same position tag, AC window −300/+200 ns from the measured Ge−BGO Δt, 6
counters / 3 flags / 2 accepts): `check_events` IDENTICAL incl. `HitWithAC` and
L2 at `--workers 6` and at `--workers 1 --chunk-span-ms 1` (21,099 chunks); an
independent (non-`eb_core`) recomputation of `HitWithAC` from the hits tree
agrees on every in-event hit; the non-AC/L2 content equals the Phase A run;
14.1 s vs 14.0 s for the Phase A definition. `check_events.C` compares
`HitWithAC` per hit and every L2 branch per event (a pre-Phase C file without
`HitWithAC` compares as all false).

The `--operator` HTTP client and the remaining ROOT wiring in `root_sink.cxx`
are not unit-tested (they need sockets / ROOT); they are covered by the
replay-based E2E gates and the live runs on gant and side3.

The MaxTreeSize rollover path can be E2E-tested by compiling with a tiny
threshold and running any emulator stack:

```sh
g++ ... -DROOTSINK_TEST_MAX_TREE_SIZE=300000 root_sink.cxx ... -o root_sink_rolltest
```

A short run must then finalize into `run%04u_0000_…`, `run%04u_0001_…`, … with
the part entries summing to the Recorder's event count.
