# 28: Tune Up Mode 実装

**Status: ✅ COMPLETED**
**Created:** 2026-02-04
**Priority:** 1 (MVP 運用に必要)

---

## 概要

Waveform ページに Tune Up モードを統合。波形とヒストグラムを見ながらデジタイザパラメータをリアルタイム調整できる。
- 上半分左: Waveform チャート表示
- 上半分右: ChargeLong (ADC) ヒストグラム表示
- 下半分: チャンネルパラメータ編集（選択チャンネルのみ）
- Recording なし（Recorder は起動しない）
- 全パラメータ適用可能（SetInRun 制限なし、自動 Stop→Apply→Start サイクル）
- **1台ずつ Tune Up** — ネットワーク負荷軽減のため、選択デジタイザの Reader のみ起動
- **FullHD フルスクリーン想定** (1920x1080)

## 設計判断

1. **1台ずつ Tune Up**: 10台×32ch の波形データはネットワーク問題を起こすため、選択デジタイザの Reader のみ起動
2. **Waveform ページに統合**: 新タブではなく既存の Waveform ページにパラメータパネルを追加
3. **自動 Stop→Apply→Start**: 非SetInRun パラメータの Apply 時、バックエンドが自動的に Reader を Stop → 全パラメータ適用 → Start
4. **パラメータ永続性**: Tune Up 中の Apply は in-memory + disk に保存。次の通常 Run で自動的に反映
5. **選択チャンネルのみ表示**: channel-table に visibleChannels フィルタを追加
6. **ChargeLong ヒストグラム同時表示**: Waveform と横並びで表示。Monitor REST API (`GET /api/histograms/:module_id/:channel_id`) から取得。パラメータ変更の効果をエネルギースペクトルで即座に確認可能
7. **Apply 時ヒストグラムクリア**: Apply 後に `POST /api/histograms/clear` を呼び、変更後のスペクトルをクリーンに蓄積開始

---

## 実装ステップ

### Phase 1: Backend — AppState + SystemStatus (Rust)

**Files:**
- `src/operator/routes/mod.rs` — AppState に `tuneup_mode: RwLock<bool>` + `tuneup_digitizer_id: RwLock<Option<u32>>` 追加
- `src/operator/mod.rs` — SystemStatus に `tuneup_mode: bool` 追加
- `src/operator/routes/status.rs` — get_status にフィールド追加、start/run_start にガード追加

**詳細:**
- AppState に `tuneup_mode` と `tuneup_digitizer_id` を `RwLock` で追加
- SystemStatus に `#[serde(default)] pub tuneup_mode: bool` 追加
- `get_status()` で `tuneup_mode` を読んでレスポンスに含める
- `start()`, `run_start()` の先頭で tuneup_mode チェック → true なら 409 Conflict

### Phase 2: Backend — Tune Up エンドポイント (Rust)

**New File:** `src/operator/routes/tuneup.rs`

#### `POST /api/tuneup/start`
- リクエストボディ: `{ "digitizer_id": u32 }`
- ガード: `SystemState::Idle` のみ
- コンポーネントフィルタ:
  - 指定 digitizer_id の Reader のみ (`is_digitizer && source_id == Some(id)`)
  - Merger + Monitor は起動
  - Recorder は除外、他のデジタイザの Reader も除外
- `configure_all_sync()` → `arm_all_sync()` → `start_all_sync()` 実行
- RunConfig: `run_number: 0, exp_name: "TuneUp"` (MongoDB 記録なし)

#### `POST /api/tuneup/stop`
- ガード: `tuneup_mode == true` のみ
- 起動中コンポーネントに `stop_all()` → `reset_all()` 送信
- `tuneup_mode = false`, `tuneup_digitizer_id = None`

#### `POST /api/tuneup/apply/{id}`
- ガード: `tuneup_mode == true` のみ
- in-memory config 更新 + disk 保存
- 対象 Reader を特定 (`is_digitizer && source_id == id`)
- 自動リスタートサイクル:
  1. Reader に Stop → Configured 待機
  2. ApplyDigitizerConfig 送信 (Configured なので全パラメータ適用)
  3. Arm → Start 送信
- Monitor/Merger はそのまま Running 継続 (データが 2-3 秒途切れるだけ)

**ルート登録:** `src/operator/routes/mod.rs` に `mod tuneup;` + 3 ルート追加

### Phase 3: Frontend — Types + Service (Angular)

**Files:**
- `web/operator-ui/src/app/models/types.ts` — SystemStatus に `tuneup_mode?: boolean`
- `web/operator-ui/src/app/services/operator.service.ts` — isTuneUp + 3 API メソッド

**メソッド:**
- `readonly isTuneUp = computed(() => this.status()?.tuneup_mode ?? false)`
- `tuneupStart(digitizerId: number): Observable<ApiResponse>`
- `tuneupStop(): Observable<ApiResponse>`
- `tuneupApply(config: DigitizerConfig): Observable<ApiResponse>`

### Phase 4: Frontend — ChannelTable visibleChannels フィルタ (Angular)

**File:** `web/operator-ui/src/app/components/channel-table/channel-table.component.ts`

- `readonly visibleChannels = input<number[] | null>(null)` 追加
- `channelIndices` computed: visibleChannels 指定時はそのチャンネルのみ、未指定時は全チャンネル
- 後方互換性: Settings ページの既存使用に影響なし
- "All" 列は常に表示

### Phase 5: Frontend — パラメータ定義の共有モジュール抽出 (Angular)

**New File:** `web/operator-ui/src/app/models/channel-params.ts`
**Modified:** `web/operator-ui/src/app/components/digitizer-settings/digitizer-settings.component.ts`

- カテゴリ別パラメータ定義を共有モジュールに移動
- ファームウェア別フィルタ関数 `getParamsForFirmware(category, firmware)` を含める
- digitizer-settings と waveform ページの両方からインポート

### Phase 6: Frontend — Waveform ページ Tune Up 統合 (Angular, メイン変更)

**File:** `web/operator-ui/src/app/pages/waveform/waveform.component.ts`

**レイアウト (FullHD 1920x1080 フルスクリーン想定):**
```
┌──────────────────────────────────────────────┐
│ Toolbar: [Channel Select] [Probes] ...       │
│         [Start/Stop Tune Up ボタン]           │
├──────────────────────┬───────────────────────┤
│                      │                       │
│   Waveform チャート  │  ChargeLong (ADC)     │
│   (~960 x 480)       │  ヒストグラム         │
│   (選択チャンネル)   │  (~960 x 480)         │
│                      │  (選択チャンネル)     │
├──────────────────────┴───────────────────────┤ ← Tune Up 時のみ表示
│ [デジタイザ名 + FW] [Apply All] [Clear Hist] │
│ ┌─ Input ─┬─ Trigger ─┬─ Energy ─┬─ ... ──┐ │
│ │ channel-table (選択チャンネルのみ)        │ │
│ │ (~1920 x 450, 横幅フル活用)              │ │
│ └──────────────────────────────────────────┘ │
└──────────────────────────────────────────────┘
```

**Tune Up 非アクティブ時:** 従来通り Waveform が全画面表示（変更なし）

**状態管理:**
- `tuneUpActive` = computed from `operator.isTuneUp()`
- `tuneUpLoading` / `applyLoading` = スピナー用
- `selectedDigitizerId` = 選択チャンネルの moduleId から自動判定
- `defaultValues` / `channelValues` = DigitizerService の展開/圧縮ロジック使用
- `histogramData` = HistogramService 経由でポーリング (選択チャンネルの ChargeLong)

**CSS スプリットレイアウト:**
- Tune Up 非アクティブ: 波形全高 (現状と同じ)
- Tune Up アクティブ:
  - 上半分 (flex: 1, min-height: 250px): 左右2分割 (Waveform | Histogram)
  - 下半分 (flex: 1, min-height: 300px): パラメータテーブル (横幅フル活用)

**ヒストグラム表示:**
- Monitor REST API: `GET /api/histograms/:module_id/:channel_id` (500ms ポーリング、既存 HistogramService 使用)
- ECharts bar chart、X 軸 = ADC channel、Y 軸 = counts
- 選択チャンネル切替時にヒストグラムも自動切替
- Apply 時に `POST /api/histograms/clear` でリセット → 新パラメータの効果をクリーンに確認

**アクション:**
- `startTuneUp()`: moduleId → digitizerId → `operator.tuneupStart(digitizerId)`
- `stopTuneUp()`: `operator.tuneupStop()`
- `applyTuneUp()`: `compressConfig()` → `operator.tuneupApply(config)` → `histogramService.clearHistograms()`
- `onDefaultChange()` / `onChannelChange()`: digitizer-settings と同じロジック

**重要ルール:**
- `[disabledKeys]="[]"` — 全パラメータ編集可能 (SetInRun 制限なし)
- `[visibleChannels]="visibleChannelIndices()"` — 選択チャンネルのみ
- デジタイザ切替時: 自動 Stop → 新デジタイザで Start (確認ダイアログ付き)

### Phase 7: Frontend — ヘッダー Tune Up インジケータ (Angular)

**File:** `web/operator-ui/src/app/app.ts`
- Tune Up 中はステータスバッジ横に `TUNE UP` ラベル表示 (オレンジバッジ)

---

## 修正ファイルサマリー

| Area | File | 変更内容 |
|------|------|----------|
| Backend | `src/operator/routes/mod.rs` | AppState に tuneup_mode、ルート登録 |
| Backend | `src/operator/mod.rs` | SystemStatus に tuneup_mode |
| Backend | `src/operator/routes/status.rs` | get_status に tuneup_mode、start/run_start のガード |
| Backend | `src/operator/routes/tuneup.rs` **(NEW)** | 3 エンドポイント |
| Frontend | `web/.../models/types.ts` | SystemStatus に tuneup_mode |
| Frontend | `web/.../models/channel-params.ts` **(NEW)** | パラメータ定義の共有モジュール |
| Frontend | `web/.../services/operator.service.ts` | isTuneUp + 3 API メソッド |
| Frontend | `web/.../components/channel-table/...` | visibleChannels input 追加 |
| Frontend | `web/.../components/digitizer-settings/...` | パラメータ定義を共有モジュールからインポート |
| Frontend | `web/.../pages/waveform/...` | Tune Up 統合（メイン変更） |
| Frontend | `web/.../app.ts` | Tune Up インジケータ |

---

## 検証方法

1. **Emulator で動作確認:**
   - `/start-daq` でシステム起動
   - Waveform ページで "Start Tune Up" → 波形 + ヒストグラム表示確認
   - パラメータ変更 → Apply → 波形が一時停止後に復帰、ヒストグラムがクリアされて再蓄積確認
   - "Stop Tune Up" → Idle に戻ることを確認
   - 通常の Run Start → Tune Up で設定したパラメータが反映されていることを確認

2. **ガードの確認:**
   - Tune Up 中に通常の Start を押す → 409 エラー確認
   - Running 中に Tune Up Start → 拒否されることを確認
   - Idle 以外から Tune Up Start → 拒否されることを確認

3. **ビルド:**
   - `cargo clippy -- -D warnings && cargo test`
   - `cd web/operator-ui && ng build`

---

## データフロー図

```
[Tune Up Start]
     │
     ▼
  Operator → Filter components:
     │         ✓ Reader (digitizer_id のみ)
     │         ✓ Merger
     │         ✓ Monitor
     │         ✗ Recorder
     │         ✗ 他の Reader
     │
     ▼
  Configure → Arm → Start (filtered components)
     │
     ▼
  Reader(PUB) → Merger(SUB/PUB) → Monitor(SUB)
     │                                    │
     │ waveform + event data              │ REST API :8081
     │                                    │ GET /api/waveforms/:m/:c
     │                                    │ GET /api/histograms/:m/:c
     │                                    ▼
     │                             Browser (Tune Up page)
     │                             ┌──────────┬──────────┐
     │                             │ Waveform │Histogram │
     │                             ├──────────┴──────────┤
     │                             │    Parameters       │
     │                             └─────────────────────┘
     │
[Apply: non-SetInRun param]
     │
     ▼
  Reader: Stop → Apply all params → Arm → Start
     │ (Monitor/Merger はそのまま Running)
     ▼
  POST /api/histograms/clear → ヒストグラムリセット
     ▼
  Waveforms + Histograms resume with new params
```
