# TODO 57: delila2root streaming sort + streaming write

**Status: COMPLETED (2026-05-15)**
**Created:** 2026-05-15
**Completed:** 2026-05-15 (1 セッション、 TODO 56 と連続)
**Hardware:** N/A (offline tool only)
**Plan file:** `~/.claude/plans/resilient-frolicking-boole.md`
**Predecessor:** TODO 56 (commit `0e77d92`) で C++ → Rust 移行済、 本タスクは時刻ソート機能の追加

## 検証結果 (2026-05-15)

### ローカル (Mac)
- `cargo clippy --features root --bin delila2root --tests -- -D warnings`: 緑
- `cargo test --features root --bin delila2root`: **8/8 PASS**
  - `sorted_stream_single_file_yields_in_order` (in-file sort)
  - `sorted_stream_two_files_no_overlap_concatenates`
  - `sorted_stream_two_files_with_overlap_uses_carry_over` (sliding window 検証)
  - `sorted_stream_reorders_files_by_file_sequence` (argv 順依存しない)
  - `branch_iter_lockstep_returns_consistent_row` (49 lazy iter shared state)
  - `branch_iter_all_exhaust_simultaneously`
  - `merge_sorted_handles_empty_inputs`
  - `merge_sorted_is_stable`
- `cargo build --release --features root --bin delila2root`: 緑

### gant 実機検証
- **Single file** (run0001_0000_PHA2_Test.delila, 309 MB, 9585 events):
  - peak RSS = **311 MB**, 2.4 sec
- **Multi-file** (run0001_*PHA2_Test.delila, 2 ファイル 622 MB, 19597 events):
  - peak RSS = **326 MB** (= 311 + 15 MB のみ、 sliding window が効いてる証拠)
  - 4.8 sec
- ROOT で **out_of_order = 0** 確認 (timestamp_ns monotonic)
- **entries = 19597** (= 9585 + 10012、 全 events 出力)
- AnalogProbe1 spot-check: 4096 sample/event、 ADC 値 1536 (PHA2 baseline ~1535)
- hadd 圧縮: 621 MB → 109 MB (**5.7x LZ4**)

### 比較 (TODO 56 vs TODO 57)
- TODO 56: Columns に全 events 貯めるため、 N 個ファイルなら ~N × 1 file 分メモリ
- TODO 57: sliding window でメモリ peak ~1 file 分、 N 個ファイルでもほぼ一定
- 99 ファイル run も理論上 ~1 GB peak で処理可能

### Deployment
- gant `/usr/local/bin/delila2root` に上書き install

## ゴール

`delila2root` を **常に時刻ソート済み** ROOT を出力するように改修。 同時に「全 events をメモリに乗せてから write」モデル (TODO 56 の制約) を撤廃し、 1 file 分のメモリで多数ファイル変換を可能にする。

旧 C++ `tools/delila2root/` のアルゴリズム (per-file sort + sliding-window two-pointer merge + per-event TTree::Fill) と機能等価。

## 背景

TODO 56 で旧 C++ tool を Rust 化したが、 旧版にあった **時刻ソート** 機能が落ちていた (argv 順で書き出し)。 ユーザーから「`--sort` フラグではなく必ずソート」+「複数ファイルがメモリに乗らない実情を考慮」という追加要望。

メモリ制約の確認:
- Recorder ([src/recorder/mod.rs:380](src/recorder/mod.rs#L380)) は write-as-received、 1 GB or 10 min で file rotation
- File header に `is_sorted: false` ([src/recorder/format.rs:84](src/recorder/format.rs#L84)) と明示 — 同一ファイル内も Merger 出力 jitter で局所反転あり
- TODO 56 後の実装は Columns に全 events 貯めて `tree.write()` 一発 → 多数ファイル時は OOM 必至

## 設計判断

oxyroot 0.1.25 の writer ([rtree/tree/writer.rs:115-153](file:///Users/aogaki/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/oxyroot-0.1.25/src/rtree/tree/writer.rs#L115)) を解析した結果:

- **row-major、 lock-step で 49 branches を deterministic 順に poll** することが判明
- `Marshaler` trait に Send 要件なし → `Rc<RefCell<...>>` で shared lazy iterator パターン可能

これにより **A2 デザイン** を採用:

```
[*.delila files (argv)]
       │
       ▼
[file_sequence 順に並べ替え]   ← header の file_sequence + footer の first_event_ns で sort
       │
       ▼
[SortedFileStream]   ← Iterator<Item=EventData>
       │ - 現在 file の sorted Vec<EventData> を保持 (1 file 分)
       │ - next file の first_event_ts を peek
       │ - current.front().ts < next.first_ts なら yield、 そうでなければ carry-over
       ▼
[Rc<RefCell<SharedRowSource>>]
       │ struct { source: SortedFileStream, current_row: Option<EventData> }
       ▼
49 × BranchIter<T>   ← branch_idx + extract closure
       │ - branch_idx == 0 → source.next() で current_row 更新
       │ - branch_idx > 0 → current_row 読み出しのみ
       │ - source 枯渇したら全 branch が None
       ▼
[oxyroot WriterTree]   ← row-major poll、 1 event = 1 row
       ▼
[ROOT file]
```

## メモリ peak

- 1 file 分の events (sort 用): typical ~100 MB-1 GB
- carry-over (前ファイル末尾 overlap、 通常 ms オーダーで数百 events): ~MB
- ROOT internal basket buffers (32 KB × 49 branches): ~1.5 MB
- **合計 ~1 GB 上限** で 99 ファイル × 10M events 処理可能 (旧 C++ 版と同等)

## Implementation steps

### Step 0 — TODO/57 + CURRENT.md ✅ (本ファイル)

### Step 1 — `SortedFileStream` 実装 (90 min)
- 構造体 + sliding-window アルゴリズム
- 4 unit tests (single file / two-file no overlap / two-file overlap / out-of-order in-file)

### Step 2 — `SharedRowSource` + `BranchIter<T, F>` 実装 (60 min)
- shared lazy iterator パターン
- 2 unit tests (lockstep polling / exhaust simultaneously)

### Step 3 — 49 branch wiring (30 min)
- `register_branches(tree, shared)` ヘルパー
- macro_rules! で 49 行の branch 登録を簡潔に

### Step 4 — main 統合 (30 min)
- 旧 Columns ベースの実装を削除
- 新ストリーミングパイプラインに置き換え

### Step 5 — Doc-comment 更新 (20 min)
- "Memory model" を sliding-window 説明に書き換え
- "Always sorted" 明記

### Step 6 — Tests のリストア (15 min)
- TODO 56 の 4 Columns tests を削除
- 新規 tests でカバレッジ維持

### Step 7 — ローカル検証 (30 min)

### Step 8 — gant 実機検証 (30 min)
- multi-file 変換 + monotonic check + memory check

### Step 9 — Install + commit + close (15 min)

## 検証チェックリスト

- [ ] `cargo clippy --features root --bin delila2root --tests -- -D warnings` 緑
- [ ] `cargo test --features root --bin delila2root` 全 PASS
- [ ] gant で 2 PHA2 files (19597 events) 変換、 全 events 出力
- [ ] ROOT で `out-of-order = 0` 確認
- [ ] `/usr/bin/time -v` で peak RSS が大幅減
- [ ] hadd LZ4 圧縮も動作

## Risks / caveats

1. **`Rc<RefCell>` borrow conflicts**: oxyroot の writer は sequential なので OK だが将来 parallel 化したら panic
2. **carry-over 膨張**: pathological multi-Recorder 並列で carry-over が肥大化、 通常運用ではない
3. **stable sort**: 同一 timestamp は file 内順序維持
4. **header 全読みコスト**: 99 file で ~1 秒、 visible に log
5. **Marshaler Send 不要**: 検証済、 Rc<RefCell> OK
6. **Columns 削除の breaking**: なし (private)

## 関連参照

- Plan: `~/.claude/plans/resilient-frolicking-boole.md`
- 既存: [src/bin/delila_to_root.rs](../src/bin/delila_to_root.rs) (TODO 56 commit `0e77d92`)
- 旧 C++: 既に削除済 (TODO 56 で `tools/delila2root/` 全削除)、 アルゴリズムだけ TODO 56 doc に記録あり
- TODO 56: [TODO/56_delila2root_waveform_support.md](56_delila2root_waveform_support.md)
- oxyroot writer 解析: [rtree/tree/writer.rs:115-153](file:///Users/aogaki/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/oxyroot-0.1.25/src/rtree/tree/writer.rs#L115)
