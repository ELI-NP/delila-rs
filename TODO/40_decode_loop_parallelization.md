# #40: DecodeLoop 並列化

**Created:** 2026-02-23
**Status: ✅ 完了** (2026-02-23, 本番検証済み)
**Priority:** 1 (Performance — DecodeLoop がリアルタイム処理に追いつかない)

## 問題

PSD2 の DecodeLoop が CPU バウンドで 10秒中 9.5秒を消費。
Stop 時に 49秒のバックログが発生し、cross-run EOS 汚染の根本原因となっている。
PSD1 も MHz オーダーに挑戦予定のため、全デコーダを統一的に並列化する。

## ベンチマーク結果 (2026-02-23, 172.18.4.76)

### PSD2 (reader_0, VX2730, 32ch)
| ステップ | 10秒間 (ms) | 割合 |
|---------|------------|------|
| decode_into() | 7,680 | 80.8% |
| convert_event() | 645 | 6.8% |
| to_msgpack() | 1,120 | 11.8% |
| ZMQ send | 58 | 0.6% |
| **合計** | **9,500** | **100%** |
- ~140 バッチ/10s、~22M イベント/10s

### PSD1 (reader_1, DT5730B, 16ch)
| ステップ | 10秒間 (ms) | 割合 |
|---------|------------|------|
| decode | 12 | 62% |
| serialize | 4 | 21% |
| zmq_send | 2 | 11% |
| **合計** | **18** | **100%** |
- ~100 バッチ/10s、~67k イベント/10s

## 設計方針

### Gemini 協議結果 (合意スコア 8/10)
- decode + serialize (99.4%) をワーカープールにオフロード
- Reorder Buffer で sequence_number の単調増加を保証
- 専用スレッドプール（crossbeam）推奨（tokio ランタイム隔離）
- PSD1 のロールオーバー: Dispatcher でヘッダースキャン事前計算

### アーキテクチャ: Uniform Parallel Pipeline
```
ReadLoop → [mpsc(1000)] → Dispatcher → [crossbeam(8)] → Workers(N=4)
                                                            ↓
                                                  decode+convert+serialize
                                                            ↓
                                                  [crossbeam(16)] → Collector
                                                                       ↓
                                                             ReorderBuffer → ZMQ
```
- 全 FW 統一パス（PSD2, PSD1, PHA1）
- PSD1: Dispatcher で BTT ヘッダースキャン → Workers にテーブル渡し

## 影響範囲
- `Cargo.toml` — crossbeam-channel 追加
- `src/reader/mod.rs` — decode_loop 大幅改修
- `src/reader/decoder/psd1.rs` — scan_aggregate_headers + decode_into_with_btts
- `src/reader/decoder/mod.rs` — DecoderKind 新メソッド

## テスト
- `cargo test` 全テスト通過
- 172.18.4.76 デプロイ → PERF ログで ~4x 改善確認
- delila2root でタイムスタンプ違反ゼロ確認

## 設計書
[docs/plans/decode_loop_parallelization.md](../docs/plans/decode_loop_parallelization.md)
