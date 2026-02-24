# DecodeLoop 並列化 設計書

**Date:** 2026-02-23
**Status:** 実装中

## 1. 問題の詳細

### 1.1 パイプラインアーキテクチャ

```
ReadLoop (spawn_blocking) → [bounded mpsc(1000)] → DecodeLoop (async) → [ZMQ PUB] → Merger → Recorder
```

7 Readers (6x PSD1 DT5730B + 1x PSD2 VX2730) → 1 Merger → 1 Recorder

### 1.2 ベンチマーク結果 (2026-02-23, 172.18.4.76, release build)

**PSD2 (reader_0, VX2730, 32ch)**
| ステップ | 10秒間 (ms) | 割合 | 説明 |
|---------|------------|------|------|
| decode_into() | 7,680 | 80.8% | バイナリパース + タイムスタンプ計算 |
| convert_event() | 645 | 6.8% | EventData → CommonEventData 変換 |
| to_msgpack() | 1,120 | 11.8% | MessagePack シリアライズ |
| ZMQ send | 58 | 0.6% | ZMQ PUB ソケット送信 |
| **合計** | **9,500** | **100%** | |

- ~140 バッチ/10s × ~161k events/batch = **~22M events/10s**
- **10秒中 9.5秒が CPU 消費 → リアルタイム処理不可能**
- Stop 時バックログ: **49秒** (Run 72→73, 2026-02-23 実測)

**PSD1 (reader_1〜5, DT5730B, 16ch)**
| ステップ | 10秒間 (ms) | 割合 |
|---------|------------|------|
| decode | 12 | 62% |
| serialize | 4 | 21% |
| zmq_send | 2 | 11% |
| **合計** | **18** | **100%** |

- ~100 バッチ/10s、~67k events/10s
- 現在は余裕あるが **MHz 到達時は PSD2 と同等の負荷になる見込み**

### 1.3 ボトルネック分析

- **decode_into() が 80.8%:** バイナリパースが圧倒的ボトルネック
- **to_msgpack() が 11.8%:** シリアライズも並列化で恩恵
- **ZMQ send は 0.6%:** ネットワークは問題なし（HWM=0 無制限バッファ）
- **CPU bound:** tokio ランタイムを 9.5秒/10秒占有、他タスクを阻害

## 2. デコーダ状態分析

| FW | 状態フィールド | 並列化への影響 |
|----|--------------|--------------|
| **PSD2** | `last_aggregate_counter: u16` (診断のみ) | **影響なし** — 各ワーカーで独立 |
| **PHA1** | `last_aggregate_counter: u32` (診断のみ) | **影響なし** — 各ワーカーで独立 |
| **PSD1** | `board_time_tag_unwrapped: u64` + wrap counter | **逐次依存** — 事前計算で解決 |

### PSD1 ロールオーバー事前計算

PSD1 の `board_time_tag` は 32-bit (wraps every ~8.59s @ 500 MS/s)。
各アグリゲートヘッダの **固定オフセット +12 バイト** に格納:

| Offset | Content |
|--------|---------|
| +0 | Word 0: Type (bits 28-31) + Aggregate Size (bits 0-27) |
| +4 | Word 1: Board ID + Dual Channel Mask |
| +8 | Word 2: Aggregate Counter |
| **+12** | **Word 3: board_time_tag (32-bit, フルワード)** |

Dispatcher で事前計算:
1. w0 (+0) で aggregate_size 読み取り → 次のアグリゲート位置を計算
2. w3 (+12) で board_time_tag 読み取り → wrap detection → unwrapped BTT 計算
3. Vec<u64> として Workers に渡す

**コスト:** 1アグリゲートあたり u32 read × 2 のみ（イベントパース不要、全体の <<1%）

## 3. Gemini AI との協議結果

**協議日:** 2026-02-23
**合意スコア:** 8/10

| 論点 | Claude | Gemini | 結論 |
|------|--------|--------|------|
| ワーカー方式 | spawn_blocking | 専用スレッドプール (crossbeam) | **crossbeam** (tokio 隔離) |
| sequence_number 付与 | Collector | Collector | **合意: Collector で付与** |
| PSD1 並列化 | 事前計算方式 | 逐次でOK (18ms) | **事前計算** (MHz 対応) |
| Reorder Buffer | BTreeMap | BTreeMap or MinHeap | **BTreeMap** (simpler) |
| チャンネル容量 | 適度 | 小さめ (メモリ圧力) | **Dispatcher→Worker: 8, Worker→Collector: 16** |
| Start/Stop | Collector 経由 | EndToken 方式 | **合意: Collector で同期** |

## 4. アーキテクチャ

```
ReadLoop (spawn_blocking)
    ↓ [bounded mpsc(1000)]
Dispatcher (async task)
    ├─ classify (全FW共通、軽量)
    ├─ PSD1のみ: BTT ヘッダースキャン (逐次、軽量)
    ↓
    [crossbeam bounded(8)] → Worker Pool (N=4 std::thread)
                                   ↓
                         decode + convert + serialize
                                   ↓
                         [crossbeam bounded(16)] → Collector (async task)
                                                      ↓
                                            Reorder Buffer (BTreeMap)
                                                      ↓
                                            ZMQ Send (sequence_number 付与)
```

### 4.1 Dispatcher
- ReadLoop から `ReadLoopOutput::Raw` を受信
- `batch_index: u64` を単調増加で割り当て
- classify → Event: ワーカーに dispatch、Start/Stop: Collector に直接送信
- PSD1: ヘッダースキャンで `Vec<u64>` (unwrapped BTTs) を事前計算
- ロールオーバー状態 (`last_btt`, `wraps`) は Dispatcher が保持

### 4.2 Worker Pool (N=4)
- std::thread + crossbeam_channel で実装
- 各ワーカーが独自の Decoder インスタンスを保持
- PSD1: `decode_into_with_btts(raw, btts, events)` — 事前計算済み BTT 使用
- PSD2/PHA1: `decode_into(raw, events)` — 既存ロジック
- 処理: decode → convert_event → EventDataBatch → to_msgpack
- 出力: `(batch_index, Vec<u8>, n_events)`

### 4.3 Collector + ReorderBuffer
- `BTreeMap<u64, (Vec<u8>, usize)>` でバッチを順序通りに再組立
- next_expected_batch を管理、一致したら ZMQ send
- sequence_number はここで付与
- Start: リセット、Stop: バッファドレイン後 EOS 送信

## 5. 本番検証結果 (2026-02-23, 172.18.4.76, release build)

### PSD2 (reader_0, VX2730, 32ch) — Before vs After

| 指標 | Before (逐次) | After (並列4w) | 改善 |
|------|-------------|--------------|------|
| **イベントレート** | ~2.2M events/10s | **~7.0M events/10s** | **3.2x** |
| **batches_pending** | N/A | **0** (常時) | バックログなし |
| **Stop→EOS 遅延** | **49秒** | **<1秒 (同一ms)** | **50x 改善** |

- Run 75: 10秒間で 69,736,231 events = **~7.0 MHz**
- Run 76: 10秒間で 68,765,425 events = **~6.9 MHz**
- Stop→EOS: Run 75 `18:44:58.840` Stop → `18:44:58.840` EOS（同一ミリ秒）

### 結論
- リアルタイム処理に余裕あり（batches_pending = 0 を維持）
- Stop→EOS の 49秒バックログが完全解消
- Out-of-band Stop (Phase 2) は不要になった可能性が高い

## 6. 影響ファイル

- `Cargo.toml` — crossbeam-channel 追加
- `src/reader/mod.rs` — decode_loop 改修 (Dispatcher/Collector/Worker)
- `src/reader/decoder/psd1.rs` — scan_aggregate_headers + decode_into_with_btts
- `src/reader/decoder/mod.rs` — DecoderKind 新メソッド

## 7. Phase 2 (将来)

- Out-of-band Stop: DecodeLoop のバックログに関係なく即座に EOS 発行
- ワーカー数の動的調整（データレートに応じて）
- NUMA-aware スレッドピンニング（マルチソケット環境）
