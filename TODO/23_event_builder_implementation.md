# Event Builder 実装計画

**Created:** 2026-02-02
**Status:** ✅ COMPLETED (Phase 7: Time Slice 方式への移行完了)
**Design Doc:** `docs/event_builder_design.md`

---

## 概要

ELIFANT-Event を参考に Rust で Event Builder を実装する。
- 入力: DELILA v2 ファイル + ROOT TTree (oxyroot)
- 出力: ROOT TTree (oxyroot)
- 設定: ELIFANT-Event 互換 JSON
- 将来: Web GUI + MongoDB 統合

**アルゴリズム変更 (2026-02-02):**
- Phase 1-6: Moving Time Window 方式で実装完了
- Phase 7: **Time Slice 方式に変更** — 並列処理対応、メモリ効率向上

---

## Implementation Summary

### Files Created/Modified

| File | Purpose |
|------|---------|
| `src/event_builder/mod.rs` | Module exports |
| `src/event_builder/hit.rs` | Hit structure with timestamps |
| `src/event_builder/built_event.rs` | BuiltEvent and EventHit structures |
| `src/event_builder/config.rs` | ChSettings, TimeCalibration, ChannelConfig |
| `src/event_builder/time_sort.rs` | TimeSortBuffer with BTreeMap |
| `src/event_builder/l1_builder.rs` | L1Builder with coincidence detection (**deprecated**, use SliceBuilder) |
| `src/event_builder/time_calibrator.rs` | TimeCalibrator with histogram-based peak detection |
| `src/event_builder/root_io.rs` | ROOT TTree I/O with oxyroot |
| `src/event_builder/time_slice.rs` | **NEW** TimeSlice 構造体、スライス分割ロジック |
| `src/event_builder/slice_builder.rs` | **NEW** SliceBuilder — Time Slice 方式のイベント構築 |
| `src/bin/event_builder.rs` | CLI with time-calib and build subcommands (SliceBuilder 使用) |

### Test Results

- **67 unit tests** all passing (event_builder module)
- **Clippy** clean with no warnings
- **Performance**: 206M hits processed in ~5 seconds (parallel file reading)

### Key Features

1. **Time Unit Handling**: FineTS in ROOT files is in picoseconds, converted to nanoseconds internally
2. **O(n) Sliding Window**: Optimized algorithm for time-sorted ROOT data
3. **Parallel File Processing**: Using rayon for parallel file reading
4. **Histogram-based Peak Detection**: Centroid calculation for accurate time offsets

---

## 実装フェーズ

### Phase 1: コア構造体 (2日) ✅ COMPLETED

**ファイル:**
- `src/event_builder/mod.rs`
- `src/event_builder/hit.rs`
- `src/event_builder/built_event.rs`
- `src/event_builder/config.rs`

**タスク:**
- [x] Hit 構造体 (EventData → Hit 変換)
- [x] BuiltEvent, EventHit 構造体
- [x] ChSettings (ELIFANT-Event 互換 JSON)
- [x] TimeCalibration (JSON パース with custom serialization)
- [x] 単体テスト (10 tests)

### Phase 2: Time Sort Buffer (1日) ✅ COMPLETED

**ファイル:**
- `src/event_builder/time_sort.rs`

**タスク:**
- [x] TimeSortBuffer (BTreeMap ベース)
- [x] insert(), drain_ready(), flush()
- [x] Watermark-based extraction
- [x] 単体テスト (9 tests)

### Phase 3: L1 Coincidence Detection (2日) ✅ COMPLETED

**ファイル:**
- `src/event_builder/l1_builder.rs`

**タスク:**
- [x] L1Builder 構造体
- [x] process_hits() アルゴリズム
- [x] トリガー優先度システム
- [x] AC ペア判定
- [x] Time calibration 適用
- [x] 単体テスト (10 tests)

### Phase 4: Time Calibration (2日) ✅ COMPLETED

**ファイル:**
- `src/event_builder/time_calibrator.rs`

**タスク:**
- [x] TimeHistogram 構造体
- [x] TimeCalibrator 構造体
- [x] fill() (ヒストグラム構築)
- [x] find_peak_centroid() (ピーク検出)
- [x] process_hits_sorted() (O(n) sliding window for sorted data)
- [x] merge() (並列処理用)
- [x] 単体テスト (11 tests)

### Phase 5: Input/Output (3日) ✅ COMPLETED

**ファイル:**
- `src/event_builder/root_io.rs`

**タスク:**
- [x] read_hits_from_root() (ROOT TTree 読み込み)
- [x] write_events_to_root() (BuiltEvent 出力)
- [x] write_hits_to_root() (Hit 出力)
- [x] FineTS ps → ns 変換
- [x] 単体テスト (2 tests)

### Phase 6: CLI + 統合テスト (1日) ✅ COMPLETED

**ファイル:**
- `src/bin/event_builder.rs`

**タスク:**
- [x] CLI (clap) - time-calib, build サブコマンド
- [x] Parallel file processing with rayon
- [x] Progress reporting
- [x] clippy warnings resolved

---

## 検証結果

### Time Calibration Test

```bash
./target/release/event_builder time-calib \
  -i /path/to/run0113_*.root \
  -o time_calib.json \
  --ref-module 9 --ref-channel 2

# Results:
# - 5 files, 206M hits processed in ~5 seconds
# - 169 channels with histograms
# - 129 channels with valid offsets (>1000 entries)
# - CPU utilization: 332% (parallel processing)
```

### Output Format

```json
{
  "ref_module": 9,
  "ref_channel": 2,
  "offsets": {
    "10_8": -53.54769001490313,
    "4_8": -522.4831932773109,
    ...
  }
}
```

---

## Phase 7: Time Slice 方式への移行 ✅ COMPLETED

**背景:**
- Phase 1-6 で実装した Moving Time Window 方式は ELIFANT-Event と同じアルゴリズム
- 元々の仕様書 (`TODO/event-builder/SPECIFICATION.md`) では Time Slice 方式を採用予定だった
- Time Slice 方式の利点: 並列処理、メモリ効率、CBM/FLES での実績

### 7.1 Time Slice 方式の概要

```
時間軸 ─────────────────────────────────────────────────────────▶

Slice N:   [========= Core =========][= Overlap =]
                                      ↑
                                 coincidence_window

Slice N+1:                     [= Overlap =][======= Core =======]
```

**パラメータ:**
- `slice_duration_ns`: 10 ms (10,000,000 ns) — デフォルト
- `overlap_ns`: coincidence_window (例: 500 ns)

**境界処理ルール:**
- トリガーが Core 領域内 → このスライスで処理
- トリガーが Overlap 領域内 → 次スライスで処理（重複防止）

### 7.2 新規ファイル

| ファイル | 目的 |
|---------|------|
| `src/event_builder/time_slice.rs` | TimeSlice 構造体、スライス分割ロジック |
| `src/event_builder/slice_builder.rs` | SliceBuilder — Time Slice 方式のイベント構築 |

### 7.3 タスク

- [x] TimeSlice 構造体 (`src/event_builder/time_slice.rs`)
  - [x] `start_ns`, `end_ns`, `overlap_ns` フィールド
  - [x] `hits: Vec<Hit>` — スライス内のヒット
  - [x] `is_in_core(timestamp)` — Core 領域判定
  - [x] `is_in_overlap(timestamp)` — Overlap 領域判定
  - [x] `create_slices()` — ヒット配列からスライス生成

- [x] SliceBuilder 構造体 (`src/event_builder/slice_builder.rs`)
  - [x] `new(slice_duration_ns, coincidence_window_ns)` コンストラクタ
  - [x] `add_trigger()`, `add_ac_pair()`, `set_time_calibration()`
  - [x] `build_events(hits: Vec<Hit>) -> Vec<BuiltEvent>` — rayon 並列処理
  - [x] `process_slice(slice: &TimeSlice) -> Vec<BuiltEvent>` — スライス内でイベント構築
  - [x] 重複防止ロジック (Core/Overlap 境界、時間優先度)

- [x] CLI 更新 (`src/bin/event_builder.rs`)
  - [x] `--slice-duration` オプション追加 (デフォルト 10ms)
  - [x] `--max-hits` オプション (旧 `--max-events`)
  - [x] `run_event_building()` を SliceBuilder 使用に変更
  - [x] AC ペア設定読み込み対応

- [x] 単体テスト (18 tests for time_slice + slice_builder)
  - [x] `test_create_slices_*` — スライス分割の正確性
  - [x] `test_create_slices_overlap_distribution` — オーバーラップ境界処理
  - [x] `test_prior_trigger_skip` — 重複イベント防止
  - [x] `test_parallel_consistency` — 並列処理での一貫性

### 7.4 既存コードとの関係

| コンポーネント | 変更 |
|---------------|------|
| `L1Builder` | **削除または非推奨** — SliceBuilder に置き換え |
| `TimeCalibrator` | **変更なし** — Moving Time Window のまま（Time Calibration には最適） |
| `TimeSortBuffer` | **変更なし** — オンラインモード用に維持 |
| `root_io.rs` | **変更なし** — I/O は共通 |

### 7.5 アルゴリズム詳細

```rust
/// Time Slice 方式のイベント構築
pub fn build_events_parallel(&self, hits: &[Hit]) -> Vec<BuiltEvent> {
    // 1. ヒットをスライスに分割
    let slices = self.create_slices(hits);

    // 2. スライスを並列処理
    let events: Vec<Vec<BuiltEvent>> = slices
        .par_iter()
        .map(|slice| self.process_slice(slice))
        .collect();

    // 3. 結果を結合
    events.into_iter().flatten().collect()
}

/// 単一スライスの処理
fn process_slice(&self, slice: &TimeSlice) -> Vec<BuiltEvent> {
    let core_end = slice.end_ns - slice.overlap_ns;
    let mut events = Vec::new();

    for (idx, hit) in slice.hits.iter().enumerate() {
        // トリガーチャンネルでない場合はスキップ
        if !self.is_trigger(hit) {
            continue;
        }

        // Overlap 領域内のトリガーは次スライスで処理
        if hit.timestamp_ns >= core_end {
            continue;
        }

        // 先行トリガーチェック（時間優先）
        if self.has_prior_trigger(slice, idx) {
            continue;
        }

        // コインシデンスウィンドウ内のヒットを収集
        let event = self.collect_coincident_hits(slice, idx);
        events.push(event);
    }

    events
}
```

---

## 将来の拡張 (今回はスコープ外)

- [ ] オンラインモード (ZMQ SUB from Event Bridge)
- [ ] L2 フィルタリング (Counter/Flag/Acceptance)
- [ ] Web GUI 設定画面
- [ ] MongoDB 設定保存

---

## 依存関係

```toml
# Cargo.toml
[dependencies]
oxyroot = { version = "0.1", optional = true }
rayon = "1.10"

[dev-dependencies]
tempfile = "3"

[features]
root = ["oxyroot"]
```

---

## 参照

- 設計ドキュメント: `docs/event_builder_design.md`
- C++ リファレンス: `legacy/ELIFANT-Event/`
