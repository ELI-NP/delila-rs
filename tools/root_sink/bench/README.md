# root_bench — ROOT parallel-write benchmark (TTree / TBufferMerger / RNTuple)

Synthetic DAQ-like hit stream (plain random data, not a detector simulation):
~1 MHz aggregate Poisson gaps, 10 modules x 15 channels with uneven rates,
per-channel Gaussian-peak + exponential energies, CAEN-like 2 ns + 10-bit fine
timestamps. Hit = module/channel (u8), energy/energy_short (u16),
timestamp_ns (double) = 14 bytes raw, same as delila-rs root_sink.

Files

| file | purpose |
|---|---|
| `hits_gen.hpp` | generator (`sorted` or `board` order = per-board batches, unsorted across modules) |
| `bench.cxx` | one binary, all modes, prints one CSV line per run |
| `run_suite.sh` | the full suite (sections A-H), writes `out/results.csv` |
| `verify_read.C` | read-back sanity macro (TTree::Draw / RDataFrame on RNTuple / RNTuple->TTree Snapshot) |
| `crash_test.cxx` | kill the writer mid-run (`_exit(1)`) and see what a reader can still open: TTree + AutoSave vs RNTuple |
| `results_*.csv` | recorded runs of `run_suite.sh` (host and ROOT version in the name) |

Findings and the design decision they fed: [`docs/root_parallel_write_research.md`](../../../docs/root_parallel_write_research.md),
[TODO 80](../../../TODO/80_generic_root_sink.md).

## Build and run

```bash
source /opt/ROOT/bin/thisroot.sh            # or /path/to/root/bin/thisroot.sh (needs imt + root7)
g++ -O2 -std=c++17 bench.cxx $(root-config --cflags --libs) -lROOTNTuple -o bench
./run_suite.sh 30000000 ./out               # ~10 min on M4 Pro; N=50000000 recommended on the DAQ host
THREADS_MAX=16 ./run_suite.sh 50000000 ./out_xeon   # try more threads on a 28-core host
```

The hit stream is generated once per (order, N) and cached as `out/cache_hits.<order>.<N>.bin`
(16 bytes/hit: 30M hits = 480 MB, 50M = 800 MB; lives in RAM during the run).
Output ROOT files are deleted after each run unless `--keep` is given.
Delete `out/cache_hits.*` when done.

## Modes (bench MODE [options])

| mode | what |
|---|---|
| `ttree` | 1 thread, TTree with 5 scalar branches; `--imt T` enables `ROOT::EnableImplicitMT(T)` |
| `tbm` | `ROOT::TBufferMerger`, `--threads T` filler TTrees -> one output TTree; `--ordered` serialises the merges in block order (entries stay time-sorted); `--fast-merge` = `SetMergeOptions("fast")` |
| `rntuple` | 1 `RNTupleWriter`; `--imt T` parallel page compression (default kOn for the sequential writer, `--imt 0` forces kOff) |
| `rntuple-par` | `RNTupleParallelWriter`, `--threads T` fill contexts, one cluster per `--block` hits; `--ordered` uses staged clusters + in-order `CommitStagedClusters()`; `--imt T` additionally turns IMT on inside each context |
| `events-ttree` / `events-rntuple` | event layout: ~200k events/s with 1..64 hits each (`HitMod[NHits]/b` ... arrays, or `std::vector<T>` fields) |
| `ttree2` | 1 thread, hits + events TTrees interleaved in ONE file |
| `twofile` | 2 threads, hits TTree -> file A and events TTree -> file B concurrently |

Common options: `--n N`, `--order sorted|board`, `--comp C` (ROOT algo*100+level: 505 ZSTD-5,
404 LZ4-4, 101 ZLIB-1, 0 none), `--basket B`, `--autoflush N`, `--cluster-mb M`,
`--block N`, `--check` (read back the timestamp column, count adjacent inversions), `--keep`.

CSV columns: `mode,params,order,n_hits,wall_s,cpu_s,cores,hits_per_s,file_MB,file_MB_s,ratio,inversions,maxrss_MB`
(`cores` = cpu_s/wall_s; `ratio` = raw bytes / file bytes; `inversions` = -1 when `--check` not given).

## Notes

* The data is already in memory, so the numbers are pure ROOT serialisation +
  compression + file write cost (an upper bound for an online sink that also
  receives/decodes/sorts).
* `ROOT::EnableThreadSafety()` is always called; `EnableImplicitMT` only with `--imt`.
* On Linux, `maxrss_MB` is from `ru_maxrss` in KiB (handled in the code).
