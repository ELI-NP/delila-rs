# delila2root — read `.delila` files from ROOT

Two ways to get DELILA data into ROOT:

1. **`TDelila.hpp`** — a single, dependency-free header. `#include` it in a ROOT
   macro and loop over events directly (no conversion step, no giant tree).
2. **`delila2root`** — a converter (macro or compiled tool) built on `TDelila`
   that writes a **ZSTD-compressed** ROOT `TTree` with one branch per field.

This replaces the old Rust/oxyroot `delila2root`, which could not compress and
emitted a fixed, mostly-empty tree.

## Requirements

- ROOT (6.20+ for ZSTD). `TDelila.hpp` itself needs **only a C++17 compiler** —
  no ROOT, no msgpack, no JSON library.

## Direct analysis (recommended)

```cpp
#include "TDelila.hpp"

tdelila::TDelila d("run0003_0000_X743_ThGEM_Test.delila");
tdelila::Event ev;
while (d.next(ev)) {
    int    ch = ev.channel();
    int    e  = ev.energy();
    double t  = ev.timestamp_ns();
    if (ev.has_waveform()) {
        const auto& wf = ev.waveform();               // materialized lazily
        const std::vector<short>&   a0 = wf.analog_probe(1);
        const std::vector<uint8_t>& d0 = wf.digital_probe(1);
        double ns = wf.ns_per_sample();
    }
}
```

Run the shipped example:

```sh
root -l 'example_analysis.C("run0003_0000_X743_ThGEM_Test.delila")'
```

Any field is also reachable generically by its schema name, e.g.
`ev.field("energy")->as_u64()` or `wf.field("digital_probe_type")`.

## Convert to a ROOT tree

As a macro:

```sh
root -l -b -q 'delila2root.C("run0003_0000.delila")'          # -> run0003_0000.root
root -l -b -q 'delila2root.C("in.delila","out.root")'
```

As a compiled tool:

```sh
g++ -O2 -std=c++17 delila2root.C $(root-config --cflags --libs) -o delila2root
./delila2root in.delila                    # -> in.root
./delila2root in.delila out.root
./delila2root run_0000.delila out.root run_0001.delila run_0002.delila   # merge a run
./delila2root -o out.root --tree tr run_00*.delila   # old Rust-CLI compatible form
```

The default tree name is `delila`; `--tree` overrides it (existing converter
scripts written for the old Rust tool passed `--tree tr`).

The output tree `delila` has a branch per event field: scalars
(`module`, `channel`, `energy`, `energy_short`, `timestamp_ns`, `flags`,
`user_info[4]`, `has_waveform`) plus the waveform fields
(`analog_probe1..3` as `vector<short>`, `digital_probe1..16` as `vector<short>`
of 0/1, `analog_probe_type[3]`, `digital_probe_type[16]`, `ns_per_sample`,
`trigger_threshold`, `time_resolution`, `analog_probeN_is_signed`). Branches that
a given firmware doesn't fill are empty and compress to almost nothing.

## How it stays in sync with the writer

`.delila` files (format v3+) embed a **self-describing schema** in the header
(`metadata["event_schema"]`). `TDelila` reads that schema to learn the exact
field order, so a new field added on the Rust side does not break the reader.
Legacy v2 files (no embedded schema) are read via a built-in fallback layout.

If the Rust side adds a field that `delila2root.C` doesn't yet write to a branch,
the converter prints a `NOTE unhandled … field` line (never silently dropped) —
add a branch for it.

## Performance

For waveform-rich runs the converter uses a **typed decode path**
(`Event::decode_waveform(DecodedWaveform&)`) instead of the generic
`waveform()` accessor. `decode_waveform` walks the waveform's MessagePack bytes
once, driven by a plan built per file from the schema (field position → action),
and writes samples straight into reused `vector<short>` buffers — it builds no
per-sample value objects and does no per-event allocation. The generic
`waveform()` DOM access (and `ev.field(...)` / `wf.field(...)`) remains fully
intact for interactive/macro use — only the converter switched paths. The
converter also calls `ROOT::EnableImplicitMT()` so basket ZSTD compression runs
across cores instead of on the decode thread.

Measured 2026-07-20 on side3 (16-core, AlmaLinux, ROOT /opt/ROOT), ThGEM test
data, identical inputs per row:

| workload | old Rust/oxyroot tool | C++ pre-typed | C++ typed+IMT |
|---|---|---|---|
| 7-file run0008 (101.9 M events, 2.6 GB) | ≈12 min, RSS 9.5 GB, **34 GB** out | — | **3 m 30 s**, RSS 1.2 GB, **6.1 GB** out |
| X743 waveform file (2.67 M ev, 1.1 GB) | — | 66.4 s | **54.2 s** |
| X730 scalar file (18.2 M ev, 404 MB) | — | 39.1 s | 36.6 s |

Correctness of the typed path was verified against the DOM path event-by-event
(164 k V1725 PHA1 waveform events byte-identical) and converter old-vs-new on
X743 (2.67 M entries: sampled exact compare + full-tree aggregate sums equal),
X730, and AMax runs. Remaining converter cost on scalar-heavy runs is MessagePack
scalar parse + `TTree::Fill` volume, not the waveform path.

## Format reference

`["DELILA02"][u32 LE len][MsgPack FileHeader]`, then repeated
`[u32 LE len][MsgPack EventDataBatch]`, then a fixed 64-byte footer
(`DLEND002`, total event count, checksum, timestamp range, completion flag).
Records are rmp-serde compact MessagePack (each struct is a positional array).
See `src/recorder/format.rs` and `src/common/delila_schema.rs` in the repo.
