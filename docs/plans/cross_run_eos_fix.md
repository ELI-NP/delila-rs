# Cross-Run EOS 汚染修正 設計書

**Date:** 2026-02-23
**Status:** Phase 1 実装中

## 1. 問題の詳細

### 1.1 パイプラインアーキテクチャ

```
ReadLoop (spawn_blocking) → [bounded mpsc(1000)] → DecodeLoop (async) → [ZMQ PUB] → Merger → Recorder
```

7 Readers (6x PSD1 DT5730B + 1x PSD2 VX2730) → 1 Merger → 1 Recorder

### 1.2 障害タイムライン (Run 70→71, 2026-02-23)

| 時刻 | イベント |
|------|---------|
| 16:56:21 | Operator が Run 70 の Stop を全コンポーネントに送信 |
| 16:56:22 | PSD2 ReadLoop が Stop 信号をチャンネルに投入 |
| 16:56:26 | Run 71 開始 (PSD2 DecodeLoop はまだ Run 69/70 のバックログ処理中) |
| 16:56:48 | PSD2 DecodeLoop がやっと Stop に到達 → EOS 発行 |
| 16:56:48 | Recorder が EOS 受信 → Run 71 のファイルを閉じる → **データ消失** |

### 1.3 根本原因分析

**Bug 1: EOS 遅延配送**
- `ReadLoopOutput::Stop` は data と同じ bounded channel(1000) に送信される
- PSD2 は ~2M events/sec を処理。各 Raw メッセージは数千イベントの aggregate
- channel capacity 1000 でも、DecodeLoop の処理がデータレートに追いつかない場合、
  バックログが溜まり Stop 信号が 20+ 秒遅延する

**Bug 2: EOS に run_number なし**
- `Message::EndOfStream { source_id: u32 }` に run 識別子がない
- Recorder は「どの Run の EOS か」を判別できない

**Bug 3: 単一 EOS で全録音停止**
- `writer_task_blocking` で最初の EOS が `end_run()` を呼び、`run_active = false` に設定
- 以降、全ソースからのデータが `write_batch()` で無視される

**Bug 4: Stop 信号ドロップ (OpenDPP パス)**
- RAW パス: `try_send` + 3秒リトライ → タイムアウトで Stop ドロップの可能性
- OpenDPP パス: **単発 `try_send`、リトライなし** → channel full なら即ドロップ

## 2. 設計方針

### 2.1 Gemini AI との協議結果

| 論点 | Claude | Gemini | 結論 |
|------|--------|--------|------|
| バッファ内データ破棄 | 検討 | 不可（物理データ） | **破棄しない** |
| Out-of-band Stop | Phase 1 で実装 | 不要（タグ付けで十分） | **Phase 2** |
| run_id vs run_number | run_number 流用 | 同意 | **run_number** |
| EOS のみ vs 全メッセージ | EOS のみ | 全メッセージ推奨 | **Phase 1: EOS のみ** |
| Recorder HashMap 方式 | Phase 2 | Phase 1 で必要 | **Phase 1: フィルタのみ** |

### 2.2 Phase 1 アプローチ: Stale EOS フィルタリング

最小限の変更で即座にデータ消失を防止する。

1. `Message::EndOfStream` に `run_number: u32` を追加
2. Reader が EOS 発行時に自身の run_number を埋め込む
3. Recorder が EOS 受信時に `run_number` を比較、不一致なら無視

**Wire format 互換性:**
- MessagePack: `EndOfStream` は struct-as-array でシリアライズ
- 旧: `[source_id]` (fixarray 0x91) → 新: `[source_id, run_number]` (fixarray 0x92)
- Merger の `parse_eos_header()` は fixarray 0x90-0x9f を受け入れ、source_id のみ抽出 → **後方互換**
- Merger は raw bytes をそのまま転送 → run_number は透過的に下流へ渡る

## 3. 修正詳細

### 3.1 Message::EndOfStream 変更

```rust
// src/common/mod.rs
EndOfStream { source_id: u32, run_number: u32 },

pub fn eos(source_id: u32, run_number: u32) -> Self {
    Self::EndOfStream { source_id, run_number }
}
```

### 3.2 ReadLoopOutput::Stop 変更

```rust
// src/reader/mod.rs
enum ReadLoopOutput {
    Raw(decoder::RawData),
    Decoded(decoder::EventData),
    Start,
    Stop(u32),  // run_number
}
```

Reader に `run_number: Arc<AtomicU32>` を追加。
`ReaderCommandExt::on_start()` で `store(run_number, Relaxed)`。
ReadLoop の Stop セクションで `load(Relaxed)` して `Stop(run)` を送信。

### 3.3 Recorder stale EOS フィルタ

```rust
// src/recorder/mod.rs — writer_task_blocking
WriterCommand::EndOfStream { source_id, run_number } => {
    if run_number != current_run_number {
        warn!(source_id, eos_run = run_number, current_run = current_run_number,
              "IGNORING stale EOS from previous run");
    } else {
        info!(source_id, run_number, "Writer received valid EOS");
        writer.end_run();
        eos_received = true;
    }
}
```

### 3.4 OpenDPP Stop リトライ修正

```rust
// src/reader/mod.rs — read_loop_opendpp Stop section
// Before: let _ = tx.try_send(ReadLoopOutput::Stop);  // 単発、リトライなし
// After: RAW パスと同じ 3秒リトライループ
```

## 4. Phase 2 設計メモ (将来)

- **Out-of-band Stop:** `tokio::sync::watch<Option<u32>>` で DecodeLoop に即座通知
- **Per-source EOS:** Recorder が `HashMap<u32, bool>` で全ソース EOS 追跡
- **run_number in Data:** `EventDataBatch` に `run_number` 追加 → Recorder HashMap<RunId, RunContext>
- **DecodeLoop state reset:** `ReadLoopOutput::Start` で `wall_s`, `total_events` リセット
