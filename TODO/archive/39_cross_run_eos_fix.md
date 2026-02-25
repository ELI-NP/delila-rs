# #39: Cross-Run EOS 汚染修正

**Created:** 2026-02-23
**Status: ✅ 完了** (2026-02-23, 本番検証済み)
**Priority:** 1 (Critical — データ消失バグ)

## 問題

PSD2 高レート時に DecodeLoop のバックログが 20 秒以上溜まり、古い Run の EOS が
次の Run の実行中に到着する。Recorder は EOS を受け取ると即座にファイルを閉じるため、
新 Run のデータが全て失われる。

### 発生条件
- 高レートデータ取得中に Stop → 5秒以内に次の Run を Start
- PSD2 の DecodeLoop バックログが 20+ 秒

### 根本原因 (3つの複合バグ)
1. **EOS 遅延配送:** ReadLoopOutput::Stop が bounded channel(1000) のデータ後に並ぶ
2. **EOS に run_number なし:** Recorder が stale EOS を区別できない
3. **単一 EOS で全録音停止:** 7ソース中1つの EOS で Recorder がファイルを閉じる

## 修正方針

### Phase 1 (今回)
- `Message::EndOfStream` に `run_number: u32` フィールドを追加
- `ReadLoopOutput::Stop(u32)` に run_number を伝播
- Recorder で stale EOS をフィルタリング (run_number 不一致なら無視)
- OpenDPP パスの Stop 信号配送を修正 (単発 try_send → 3秒リトライ)

### Phase 2 (将来)
- Out-of-band Stop チャンネル (DecodeLoop への即座の Stop 通知)
- Per-source EOS 完了トラッキング
- run_number in EventDataBatch (完全なデータ系統管理)

## 影響範囲
- `src/common/mod.rs` — Message enum
- `src/reader/mod.rs` — ReadLoopOutput, decode_loop, ReadLoop
- `src/recorder/mod.rs` — WriterCommand, stale EOS フィルタ
- `src/data_source_emulator/mod.rs` — EOS 生成
- `src/monitor/mod.rs`, `src/data_sink/mod.rs`, `src/merger/mod.rs` — パターンマッチ更新
- `src/bin/event_bridge.rs`, `src/bin/eb_test_sender.rs` — 機械的更新
- `src/event_builder/online.rs` — パターンマッチ更新

## テスト
- `cargo test` 全テスト通過
- 172.18.4.76 で高レート Run → Stop → 即座に次の Run → "IGNORING stale EOS" ログ確認

## Gemini 協議メモ
- Gemini: バッファ内データは有効な物理データ、破棄不可
- Gemini: HashMap<RunId, RunContext> は Phase 2 で検討
- Gemini: blocking_send を推奨 → Phase 1 では 3秒リトライで妥協 (Phase 2 で改善)
