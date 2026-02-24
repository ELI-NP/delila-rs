# Event Builder 統一 + mimalloc 導入

**Created:** 2026-02-23
**Status:** 📋 計画完了（Gemini レビュー済み）
**Priority:** 1 (MVP Phase 2)

## Context

SliceBuilder（オフライン用）は `create_slices()` の O(hits × slices) + clone がボトルネックで、
オンラインでは使い物にならなかった。代わりに chunk_builder（pure 関数 + Safe Horizon）を開発し、
online.rs で稼働中。現在 SliceBuilder は `bin/event_builder.rs`（オフライン CLI）のみが使用。

**目標:** オフライン CLI も chunk_builder に統一し、SliceBuilder を削除。合わせて mimalloc を導入。

**検証データ:** `data/run0061.root` (2.1 GB) + `data/run0061_*.delila` (145 files, ~144 GB)

---

## Phase 1: オフライン CLI を chunk_builder に移行

### 1.1 TriggerConfig 構築ヘルパーの共有化

**現状:** `online.rs:648` に `load_trigger_config_from_file()` が private で存在。
オフライン CLI にも同じロジックが必要。

**変更:**
- `chunk_builder.rs` の `impl TriggerConfig` に `pub fn from_channel_config(settings: &ChannelConfig, coincidence_window_ns: f64) -> Self` を追加
  - ファイルI/O を含まない pure 関数（テスト容易、chunk_builder の pure 性を維持）
  - `ChannelConfig` = `Vec<Vec<ChSettings>>`（既存の型エイリアス）
  - `online.rs` の `load_trigger_config_from_file()` は `load_channel_config()` + `TriggerConfig::from_channel_config()` に変更
- `mod.rs` からの re-export は不要（`chunk_builder` は既に `pub mod`）

### 1.2 `bin/event_builder.rs` の `run_event_building()` を書き換え

**現在の流れ (SliceBuilder):**
```
SliceBuilder::new() → add_trigger/add_ac_pair → set_time_calibration → read all hits → build_events()
```

**新しい流れ (chunk_builder):**
```
1. TriggerConfig 構築（config JSON or --trigger CLI args）
2. TimeCalibration ロード
3. 全ヒット読み込み（既存ロジックそのまま）
4. タイムキャリブレーション適用（hit.timestamp_ns -= offset）
5. sort_and_flush() → SortedChunk (core_end = f64::MAX)
6. build_events_from_chunk() → Vec<BuiltEvent>
7. イベントID 連番付与
8. write_events_to_root()
```

**対象ファイル:** `src/bin/event_builder.rs`

**変更点:**
- import: `SliceBuilder` → `chunk_builder::{build_events_from_chunk, sort_and_flush, TriggerConfig}`
- `--slice-duration` CLI arg: 削除（SliceBuilder 固有の概念）
- `run_event_building()` 本体: 上記の新しい流れに書き換え
- time-calib サブコマンド: **変更なし**（rayon 並列ファイルI/O はそのまま維持）

### 1.3 回帰テスト

`data/run0061.root` を使って:
1. **移行前:** 現行 SliceBuilder 版でイベントビルド → イベント数を記録
2. **移行後:** chunk_builder 版でイベントビルド → イベント数を比較
3. 完全一致は期待しない（アルゴリズムが微妙に異なる）が、0.1% 以内の差を確認

---

## Phase 2: SliceBuilder + time_slice.rs 削除

**対象ファイル:**
- 削除: `src/event_builder/slice_builder.rs` (482 lines)
- 削除: `src/event_builder/time_slice.rs` (324 lines)
- 編集: `src/event_builder/mod.rs`
  - `mod slice_builder;` / `mod time_slice;` 削除
  - `pub use slice_builder::{SliceBuilder, SliceBuilderStats};` 削除
  - モジュールドキュメントコメント更新

**chunk_builder.rs の `test_offline_root_data` テスト:**
- SliceBuilder 参照を削除
- chunk_builder 単体の検証テストに変更（イベント数 > 0、trigger_time 昇順等）

---

## Phase 3: mimalloc 導入

**変更:**
- `Cargo.toml`: `mimalloc = { version = "0.1", default-features = false }` 追加
- `src/bin/event_builder.rs`: 先頭に `#[global_allocator]` 追加
- `src/bin/online_event_builder.rs`: 同上

```rust
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;
```

**ベンチマーク:** `data/run0061.root` で before/after の処理時間を計測

---

## Phase 4: split_off 最適化 — キャンセル

分析の結果、適用不可。chunk は全ヒットを保持する必要があり（core_end 以降のヒットも
coincidence window 参照用に必要）、`split_off()` で truncate すると壊れる。
現在の `buffer[split_idx..].to_vec()` + buffer move が最適。

---

## 実装順序とコミット計画

| # | 内容 | コミット |
|---|------|----------|
| 1 | chunk_builder.rs に `TriggerConfig::from_channel_config()` 追加 | 単独コミット |
| 2 | online.rs の `load_trigger_config_from_file()` を新メソッド利用に変更 | 同上に含める |
| 3 | bin/event_builder.rs 書き換え（SliceBuilder → chunk_builder） | 単独コミット |
| 4 | 回帰テスト（run0061.root で比較） | 手動確認 |
| 5 | slice_builder.rs + time_slice.rs 削除、mod.rs 更新 | 単独コミット |
| 6 | Cargo.toml に mimalloc 追加 + global_allocator 設定 | 単独コミット |
| 7 | ベンチマーク（before/after） | 手動確認 |

---

## 検証手順

```bash
# Phase 1 回帰テスト
cargo build --release --features root --bin event_builder
./target/release/event_builder build -i data/run0061.root --trigger 0:0 -o /tmp/events_new.root

# Phase 2 後のテスト
cargo fmt && cargo clippy -- -D warnings && cargo test
cargo test --features root

# Phase 3 ベンチマーク
time ./target/release/event_builder build -i data/run0061.root --trigger 0:0 -o /tmp/events.root
```

---

## Gemini レビュー結果 (2026-02-23)

### 承認事項
- **sort_and_flush はオフラインで正しいアプローチ** — 全データが揃っている場合は Safe Horizon 不要
- **mimalloc が最適** — jemalloc より 20-60% 高速、軽量、Rust との統合が容易
- **rayon 維持は正解** — time-calib の CPU-bound 並列処理に最適（tokio::spawn_blocking は不適）
- **split_off キャンセルは正しい判断**

### 注意点・推奨事項
1. **メモリ使用量**: 60M hits × ~24B = ~1.5GB + 出力 BuiltEvent で合計 3-4GB 必要な可能性。Mac (16GB+) なら問題なし
2. **ソート時間**: 60M hits で ~1-2.5秒（500K の線形スケールではない、L3 キャッシュミスが支配的）
3. **total_cmp**: f64 ソートには `total_cmp` を使用（既に実装済み、NaN 安全）
4. **将来の最適化**: `partition_point` をスライディングウィンドウ/two-pointer 方式に置き換えれば O(N log N) → O(N) に改善可能（今回はスコープ外）
5. **release プロファイル**: mimalloc の効果を最大化するために `lto = true, codegen-units = 1` を検討
6. **TriggerConfig の置き場所**: Gemini は config.rs を推奨したが、TriggerConfig 自体が chunk_builder.rs に定義されているため、`impl TriggerConfig` として chunk_builder.rs に pure メソッドを追加する方がシンプル（ファイルI/O は呼び出し側に残す）

---

## 影響範囲

- **変更ファイル:** 5 ファイル (chunk_builder.rs, online.rs, bin/event_builder.rs, mod.rs, Cargo.toml)
- **削除ファイル:** 2 ファイル (slice_builder.rs, time_slice.rs) = 806 行削減
- **追加依存:** mimalloc (Cargo.toml)
- **rayon:** 維持（time-calib サブコマンドで使用）
- **オンラインパイプライン:** 影響なし（online.rs は既に chunk_builder を使用）

## 実装完了後に更新するファイル

- `TODO/CURRENT.md` — Online EB v2 ステータス更新、SliceBuilder 統一を完了に移動
- `TODO/30_mvp_march_roadmap.md` — EB 関連タスクのステータス更新
- `TODO/event-builder/SPECIFICATION.md` — SliceBuilder 参照を chunk_builder に変更
- `docs/plans/online_event_builder_v2.md` — 統一の結果を反映
