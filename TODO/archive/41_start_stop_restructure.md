# #41 Reader Start/Stop フロー再構成 — DIG1 タイムスタンプリセット

**Created:** 2026-02-24
**Status:** 🔧 実装中
**Design Doc:** [docs/plans/start_stop_restructure.md](../docs/plans/start_stop_restructure.md)

---

## 問題

DIG1 (PSD1/PHA1) のタイムスタンプカウンタが Run 間でリセットされない。
`/cmd/reset`, `/cmd/disarmacquisition`, `/cmd/cleardata`, S-IN いずれも効果なし。
唯一の解: `CAEN_FELib_Close` + `CAEN_FELib_Open`。

## 方針

1. **Stop 時に connection を Close** — DeviceConnection の Drop で CAEN_FELib_Close
2. **次回 Start 時に自動 Open + Configure** — 既存の state catch-up が処理
3. **Arm フェーズを Start に統合** — Legacy C++ と同じ arm+start 一括方式
4. **Operator から Arm 呼び出しを除去** — Configured → Running 直接遷移

## 実装ステップ

- [x] Step 0: ドキュメント作成 (TODO + 設計書 + CURRENT.md)
- [ ] Step 1: State Machine 変更 (`src/common/command.rs`)
- [ ] Step 2: `send_start_acquisition` 統合 (`src/reader/mod.rs`)
- [ ] Step 3: `read_loop_raw` 改修 — Stop 後に connection 切断
- [ ] Step 4: `read_loop_opendpp` 改修 — 同上
- [ ] Step 5: Operator Arm フェーズ削除 (`src/operator/routes/status.rs`)
- [ ] Step 6: Tuneup arm_all_sync 削除 (`src/operator/routes/tuneup.rs`)
- [ ] Step 7: テスト + ビルド + ハードウェア検証

## 検証

1. `cargo fmt && cargo clippy -- -D warnings && cargo test`
2. DT5730B (PSD1) Start → Stop → Start — タイムスタンプ 0.000 確認
3. Tune Up Apply サイクル正常動作確認
