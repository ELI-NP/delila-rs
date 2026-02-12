# Tune Up 初期化修正プラン

**Date:** 2026-02-10
**Status:** COMPLETED

## Context

Tune Up モード開始時 (`tuneup_start()`) で、デジタイザの設定パラメータがハードウェアに適用されないバグ。
通常の DAQ 開始 (`run_start()`) では Configure 後に `ApplyDigitizerConfig` コマンドを送信するが（"Phase 1.5"）、Tune Up ではこのステップが欠落していた。

**影響:** SetInRun=False パラメータ（pre_trigger, record_length, CFD, gate lengths, trap times 等）が Tune Up 開始時に適用されず、前回の設定がそのまま使われる。

## 修正内容

**対象ファイル:** `src/operator/routes/tuneup.rs`

### 修正後のフロー
```
configure_all_sync() → ApplyDigitizerConfig → Channel Registration → arm_all_sync() → start_all_sync()
```

`tuneup_start()` の configure 成功後、arm の前に `ApplyDigitizerConfig` コマンドを送信するステップを追加。
`status.rs` の `run_start()` "Phase 1.5" と同じパターン。

### 設計判断

- Apply 失敗時はエラー返却 + rollback（Tune Up は1台のみ、失敗なら続行不可）
- config 未登録時は warn のみ（Reader は JSON file から自前ロード済み）
- Gemini レビュー済み: Zombie State 懸念なし、厳格エラーハンドリング採用
