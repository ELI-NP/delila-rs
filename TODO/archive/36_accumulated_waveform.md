# Issue #4: Waveform積算表示（Accumulated Waveform）

**GitHub Issue:** #4
**Status: ✅ 完了**
**Updated:** 2026-02-18
**Completed:** 2026-02-18 (commit a1603bd)

## 目的

Tune Upモードで複数のwaveformを重ね書き表示（accumulated/overlay）し、信号パターンを視覚的に把握する。
トグルスイッチで単一トレース/積算表示を切り替え、イベント数を設定可能。

## 設計

### 変更ファイル

1. `web/operator-ui/src/app/pages/waveform/waveform.component.ts` — 積算ロジック + UI追加

### バックエンド変更: なし

既存APIの `GET /api/waveforms/:module/:channel` がlatestを返す仕組みはそのまま。
フロントエンドがローカルにFIFOバッファで履歴を保持する。

### 実装詳細

**Step 1: 積算状態signal追加**

```typescript
readonly accumulateEnabled = signal(false);    // ON/OFF
readonly accumulateMax = signal(20);           // 最大イベント数
readonly waveformHistory = signal<LatestWaveform[]>([]);  // FIFOバッファ
```

**Step 2: `fetchWaveforms()` 修正**

- Tune Up + 積算有効時: 新waveformをFIFOバッファに追加
- `timestamp_ns` で重複検出（同じイベントの二重追加防止）
- `accumulateMax` を超えたら古い方から削除

**Step 3: `buildChannelCharts()` に積算描画ロジック追加**

- 履歴トレースは **analog probe のみ** 描画
  - デジタルプローブ(trigger等)は **最新トレースのみ** 表示（markAreaのクラッター回避）
- opacity: 古いトレース 0.1〜0.4、最新 1.0
- 履歴トレースは `silent: true`（tooltip無効）+ legend名なし

**Step 4: UIコントロール追加**（Tune Upツールバー内）

```
[Acc toggle] [N: input] [5/20] [clear button]
```

- `mat-slide-toggle`: 積算ON/OFF ("Acc")
- `input[type=number]`: 積算数 (2〜100)
- `span`: 現在の積算数 ("5/20")
- `mat-icon-button`: クリアボタン (delete_sweep)

**Step 5: 状態クリア処理**

以下のタイミングで `waveformHistory` をクリア:
- 積算トグルOFF時
- チャンネル切り替え時
- パラメータApply時

### パフォーマンス

| 条件 | データ量 | 評価 |
|------|---------|------|
| 20 traces x 1000 samples | 20,000点 | EChartsで余裕 |
| 50 traces x 1000 samples | 50,000点 | 問題なし |
| 100 traces x 1000 samples | 100,000点 | 低スペックマシンで要確認 |

- `silent: true` でイベントハンドラ省略
- `animation: false` は既存設定
- `computed()` によるメモ化

## テスト

- [ ] `ng build` — ビルド成功
- [ ] 手動: テストパルスでTune Up → 積算ON → 複数トレースが重なって表示
- [ ] 手動: トグルOFF → 単一トレースに戻る
- [ ] 手動: チャンネル切替 → 履歴クリア
- [ ] 手動: パラメータApply → 履歴クリア
- [ ] パフォーマンス: 50トレースでもスムーズにzoom/pan可能
- [ ] 重複検出: 低レートでも同じイベントが二重追加されないこと

## コスト見積もり

- 追加行数: 800-1000行
- 変更ファイル: 1
- 新規ファイル: 0
- リスク: 中（パフォーマンス、Y軸スケーリング）
