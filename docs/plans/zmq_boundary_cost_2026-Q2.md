# ZMQ Boundary Cost Benchmark (R-X3, Phase 1 Refactor Sprint 2026-Q2)

**Status:** Scaffold complete (Phase 1 Week 1). Baseline measurement pending — run on `gant@172.18.6.114` before any structural refactor lands.

**Tool:** [`src/bin/zmq_boundary_bench.rs`](../../src/bin/zmq_boundary_bench.rs) (dev-tools-gated)

**Why two-point measurement:** D8 commitment — answer the long-standing question *"ZMQ 境界 cost が実際どれくらいか?"* with concrete numbers, both before refactor (Phase 1) and after (Phase 3 Week 11). The sprint design itself is **not** gated by these numbers (D7 keeps component system regardless), but the data informs future decisions like a possible monolithic re-evaluation post-7/24 experiment.

---

## How to run

On `gant@172.18.6.114` (the canonical bench host — same machine for baseline and post-refactor so the comparison is fair):

```sh
ssh gant@172.18.6.114 'cd /media/raid1/delila-rs && \
  source ~/.cargo/env && \
  git pull && \
  cargo build --release --features dev-tools --bin zmq_boundary_bench'
```

Then run the three rate points back-to-back and capture each report:

```sh
for rate in 10000 100000 1000000; do
  cargo run --release --features dev-tools --bin zmq_boundary_bench -- \
    --rate $rate --duration 60 --batch-size 100 --waveform-samples 0 \
    --label "baseline-${rate}-$(git rev-parse --short HEAD)" \
    --output "docs/plans/zmq_baseline_${rate}.json"
done
```

Each run dumps a JSON report (`BenchReport` schema in `zmq_boundary_bench.rs`). Append the headline numbers to the tables below.

---

## Schema

The bench writes one JSON per run. Stable field names so the post-refactor diff stays mechanical:

```jsonc
{
  "label": "baseline-100000-abc1234",
  "timestamp_unix": 1747000000,
  "git_commit": "abc1234",
  "rust_version": "rustc 1.93.0 ...",
  "args": { "rate": 100000, "duration_s": 60, "batch_size": 100, "waveform_samples": 0 },
  "pipeline": {
    "events_sent": 6000000,
    "events_received": 6000000,
    "drops": 0,
    "throughput_eps": 99987.4,
    "throughput_mbps": 11.2,
    "latency_p50_us": 73,
    "latency_p95_us": 142,
    "latency_p99_us": 358
  },
  "per_boundary_us": { "encode_mean": 5.1, "send_mean": 1.4, "recv_mean": 0.9, "decode_mean": 4.7 },
  "bytes": { "total_wire_bytes": 672000000, "bytes_per_event": 112.0 }
}
```

---

## Baseline (Phase 1 pre-refactor, 2026-05-06)

Two-host run executed before any Phase 2 structural refactor (R-D6/D7/C1/C2 etc.) lands. Both hosts show **0 drops** across 30 s × 3 rate points; throughput tracks the target rate exactly because pacing is the cap, not the pipeline.

### Host info

| Host | CPU | OS / kernel | Rust | Commit |
|---|---|---|---|---|
| **Mac** (local) | Apple M4 Pro | macOS 25.4.0 (Tahoe) | rustc 1.93.0 stable | `d5ae2c4` + uncommitted Phase 1 changes |
| **gant** (Linux dev) | Intel Xeon W-3223 @ 3.50 GHz | Ubuntu 5.15.0-176-generic | cargo 1.95.0 (2026-03-21) | `d5ae2c4-phase1pre` (rsync of Mac src) |

Bench config for every row: `--duration 30 --batch-size 100 --waveform-samples 0`. Single inproc:// PUB→SUB boundary, synthetic events (no waveform), no concurrent load.

### Mac (Apple M4 Pro)

| Rate (ev/s) | Throughput (ev/s) | Throughput (MB/s) | p50 lat | p95 lat | p99 lat | encode µs | send µs | decode µs | bytes/ev | drops |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 10 000 | 10 003 | 0.2 | 155 | 229 | 485 | 22.29 | 68.11 | 22.32 | 22.2 | 0 |
| 100 000 | 100 001 | 2.2 | 104 | 177 | 401 | 17.12 | 37.73 | 18.81 | 22.2 | 0 |
| 1 000 000 | 999 991 | 23.4 | 85 | 158 | 232 | 10.71 | 19.60 | 14.52 | 23.4 | 0 |

### gant@172.18.6.114 (Xeon W-3223)

| Rate (ev/s) | Throughput (ev/s) | Throughput (MB/s) | p50 lat | p95 lat | p99 lat | encode µs | send µs | decode µs | bytes/ev | drops |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 10 000 | 10 003 | 0.2 | 119 | 125 | 148 | 23.57 | 26.81 | 30.56 | 22.2 | 0 |
| 100 000 | 99 998 | 2.2 | 99 | 120 | 140 | 21.31 | 16.15 | 29.52 | 22.2 | 0 |
| 1 000 000 | 999 987 | 23.4 | 98 | 133 | 242 | 19.05 | 4.17 | 25.25 | 23.4 | 0 |

### Takeaways

- **Both hosts hit the 1 M ev/s pacing cap with 0 drops.** The bench's tokio-sleep granularity is the throughput limit on both, not the ZMQ boundary itself.
- **Mac latency p99 is more skewed** (485 µs at 10 k) than gant (148 µs). At higher rates Mac actually gets *better* p50 (85 vs 98 µs) — pacing-idle dominates the low-rate tail rather than ZMQ cost.
- **encode + decode ≈ 30–50 µs / batch of 100 events** on both hosts → roughly 0.3–0.5 µs / event for MessagePack round-trip, which is the floor any monolithic refactor would have to clear to be worth it.
- **bytes/ev = 22.2** without waveform. Confirms the EventData wire format size matches the 14-byte fixed-binary used by `event_bridge` plus MessagePack framing overhead (~8 B / event).
- **Caveat — `recv_mean` is not a true cost**: the bench measures `Instant::now()` *before* the next-message await, so it absorbs inter-batch idle time (10 ms at 10 k ev/s; ~80 µs at 1 M). The encode/send/decode columns are clean.

The Phase 3 Week 11 re-run will use these as baseline and report a diff in the next section.

**Raw JSON:** [`docs/plans/zmq_bench_results/`](zmq_bench_results/) (`mac_*.json`, `gant_*.json`).

---

## Post-refactor (Phase 3 Week 11, after R-D6/D7/C1/C2/P3-P5/P8)

> **TODO:** populate after Phase 3 close.

| Rate (ev/s) | Throughput (ev/s) | Throughput (MB/s) | p50 lat | p95 lat | p99 lat | encode µs | send µs | recv µs | decode µs | bytes/ev | drops |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 10 000 |  |  |  |  |  |  |  |  |  |  |  |
| 100 000 |  |  |  |  |  |  |  |  |  |  |  |
| 1 000 000 |  |  |  |  |  |  |  |  |  |  |  |

---

## Diff & conclusion

> **TODO:** fill after both data points are in. Format suggestion:
>
> ```
> | Metric | Baseline (commit X) | Post-refactor (commit Y) | Δ % | Interpretation |
> ```
>
> Conclusion paragraph: tie the numbers back to D7 (component system kept) and R-P8 (ComponentRunner consolidation). Did boilerplate cleanup affect encode/decode/send/recv cost? Was the ZMQ boundary itself ever a real cost concern, or did the absolute numbers stay below the existing ROOT-writer / decoder-loop limits we already know? Use the conclusion to inform whether D7 should be revisited at a future date.

---

## Caveats

- **In-process (`inproc://`) measurement.** The bench skips kernel/TCP cost so we can isolate (encode + ZMQ buffer + decode). Production cross-machine runs will show ~+30-200 µs of extra TCP latency depending on network topology, but the per-boundary mean we measure here is the encode/decode floor that no infrastructure choice can avoid.
- **Single-PUB, single-SUB.** The production pipeline has multi-source merge (5x reader → merger). The single-boundary measurement is a conservative lower bound — multi-boundary cost stacks roughly linearly.
- **Synthetic events.** No CAEN driver in the loop. Rate cap is set by tokio-sleep granularity (~µs) on Linux, not by hardware.
- **No CPU pinning.** Numbers vary ±5% across runs depending on host load. Run 3× and median.
