# Issue #2: Run Start時にWaveform Recording警告

**GitHub Issue:** #2
**Status: ✅ 完了**
**Updated:** 2026-02-18
**Completed:** 2026-02-18 (commit a1603bd)

## 目的

本番Run開始時にwaveform recordingが有効なデジタイザがある場合、MatDialogで警告を表示する。
Tune Up後にwaveformを無効にし忘れるのを防ぐ安全装置。

## 設計

### 変更ファイル

1. `web/operator-ui/src/app/components/control-panel/control-panel.component.ts` — MatDialog呼び出し追加
2. `web/operator-ui/src/app/components/control-panel/waveform-warning-dialog.component.ts` — 新規: 警告ダイアログ

### フロー

```
Start ボタン → waveforms_enabled チェック
  ├─ 全てfalse → 直接 start()
  └─ 1つ以上true → MatDialog 表示
       ├─ "Start Anyway" → start()
       └─ "Cancel" → 何もしない
```

### 実装詳細

**Step 1:** 警告ダイアログコンポーネント作成（standalone, inline template/styles）
- `MAT_DIALOG_DATA` でデジタイザ名リストを受け取る
- "Start Anyway" (warn color) / "Cancel" の2ボタン
- 既存パターン: `histogram-expand-dialog.component.ts` に倣う

**Step 2:** `control-panel.component.ts` に `DigitizerService` と `MatDialog` をinject
- コンストラクタで `loadDigitizers()` 呼び出し

**Step 3:** `onStart()` にチェックロジック挿入
- `digitizerService.digitizers().filter(d => d.board.waveforms_enabled === true)`
- 該当あり → `dialog.open()` → `afterClosed()` で結果を処理

### チェック対象

| Firmware | チェック項目 |
|----------|-------------|
| PSD1/PHA1 | `board.waveforms_enabled === true` |
| PSD2 | `board.waveforms_enabled === true`（将来: `wave_trigger_source` も） |

## テスト

- [ ] `ng build` — ビルド成功
- [ ] 手動: waveform有効時 → MatDialog表示
- [ ] 手動: Cancel → start中止
- [ ] 手動: Start Anyway → start実行
- [ ] 手動: waveform無効時 → ダイアログなしで直接start

## コスト見積もり

- 追加行数: 100-130行（ダイアログ + チェックロジック）
- 新規ファイル: 1
- 変更ファイル: 1
- リスク: 低
