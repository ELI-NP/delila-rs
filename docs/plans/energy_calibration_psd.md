# Energy Calibration & PSD 表示設計書

**Created:** 2026-02-19
**Status:** Phase 1-2 完了 (2026-02-20), Phase 3 スキップ, Phase 4-5 未着手
**GitHub Issue:** #7
**Reviewed:** Gemini 協議済み (2026-02-19)

---

## 1. 概要

Monitor に以下の機能を追加する:

1. **PSD 2D ヒストグラム**: Energy vs PSD の 2D カラーマップ + ゲート定義
2. **PSD 1D ヒストグラム**: (energy - energy_short) / energy の比率分布
3. **Energy Calibration**: ADC チャンネル → keV/MeV 変換表示

### 設計原則
- **Monitor の責任範囲**: デジタイザデータを読むだけで実現可能なレベル。複雑な解析は Event Builder の役割
- **データ保全**: raw ADC データを常に保持。キャリブレーションは表示層のみ
- **KISS**: 最小限の変更で最大の効果

---

## 2. 設計方針 (Gemini 協議結果)

### 2.1 キャリブレーション係数の適用場所 → **フロントエンド変換**

- バックエンド (Monitor) は **raw ADC のまま**ヒストグラムを蓄積
- フロントエンド (Angular) が X 軸表示を keV/MeV に変換
- bins[i] の X 座標を `a * i + b` に線形変換するだけ（rebinning 不要）

**理由:**
- raw データ保持 → 複数の表示単位を自由に切り替え可能
- Monitor の複雑性を増さない
- ECharts の xAxis formatter で実装可能

### 2.2 PSD → **2D ヒストグラム (Energy vs PSD) を最初から実装**

- `process_event()` 内で PSD = (energy - energy_short) / energy を計算
- **2D ヒストグラム** (Energy vs PSD) をバックエンドで蓄積 → フロントエンドで heatmap 表示
- **1D ヒストグラム** (Energy, PSD) も独立して蓄積（projection 計算より高速）
- 2D プロット上で**多角形ゲート**を定義し、粒子弁別を完結させる

### 2.3 キャリブレーション係数の保存 → **JSON ファイル**

- `config/calibration.json` に保存
- チャンネルごとの線形係数 (a, b)
- 変更頻度: 年に数回程度
- 将来 MongoDB 移行も容易

### 2.4 REST API → **クエリパラメータ方式**

- `/api/histograms/:module/:channel?type=energy` (デフォルト)
- `/api/histograms/:module/:channel?type=psd`
- `/api/histograms2d/:module/:channel` (2D ヒストグラム)

---

## 3. バックエンド実装詳細

### 3.1 Histogram2D 構造体 (新規)

```rust
pub struct Histogram2D {
    pub module_id: u32,
    pub channel_id: u32,
    pub x_config: HistogramConfig,  // Energy: 512 bins, 0-65536
    pub y_config: HistogramConfig,  // PSD: 200 bins, -0.2〜1.2
    pub bins: Vec<u64>,             // flat array: y * x_bins + x
    pub total_counts: u64,
    pub overflow: u64,              // どちらかの軸が範囲外
}
```

- **flat array** (`Vec<u64>`): メモリ連続 → キャッシュ効率が高い
- インデックス計算: `bin = y_bin * x_bins + x_bin`
- 512 × 200 = 102,400 ビン × 8 bytes = **約 800 KB/チャンネル**

### 3.2 MonitorState の変更

```rust
pub struct MonitorState {
    pub histograms: HashMap<ChannelKey, Histogram1D>,          // energy (既存)
    pub psd_histograms: HashMap<ChannelKey, Histogram1D>,      // 1D PSD (新規)
    pub psd2d_histograms: HashMap<ChannelKey, Histogram2D>,    // 2D Energy vs PSD (新規)
    pub latest_waveforms: HashMap<ChannelKey, LatestWaveform>,  // (既存)
    // ... 既存フィールド
    pub psd_histogram_config: HistogramConfig,                 // 1D PSD 用
    pub psd2d_x_config: HistogramConfig,                       // 2D X軸 (Energy)
    pub psd2d_y_config: HistogramConfig,                       // 2D Y軸 (PSD)
}
```

### 3.3 ヒストグラム設定

| ヒストグラム | ビン数 | 範囲 | 備考 |
|------------|-------|------|------|
| Energy 1D | 65536 | 0〜65536 | 既存 |
| PSD 1D | 200 | -0.2〜1.2 | n-γ弁別用 |
| 2D X (Energy) | 512 | 0〜65536 | 粗い分解能で十分 |
| 2D Y (PSD) | 200 | -0.2〜1.2 | 1D PSD と同じ |

設定は **Monitor config TOML** で変更可能にする。

### 3.4 process_event での PSD 計算

```rust
pub fn process_event(&mut self, event: &EventData) {
    let key = ChannelKey { module_id: event.module as u32, channel_id: event.channel as u32 };

    // 1. Energy ヒストグラム (既存)
    if let Some(hist) = self.histograms.get_mut(&key) {
        hist.fill(event.energy as f32);
        self.total_events += 1;
    }

    // 2. PSD 計算 (energy > 0 の場合のみ)
    if event.energy > 0 {
        let psd = (event.energy as f32 - event.energy_short as f32) / event.energy as f32;

        // 2a. 1D PSD ヒストグラム
        if let Some(psd_hist) = self.psd_histograms.get_mut(&key) {
            psd_hist.fill(psd);
        }

        // 2b. 2D Energy vs PSD ヒストグラム
        if let Some(psd2d) = self.psd2d_histograms.get_mut(&key) {
            psd2d.fill(event.energy as f32, psd);
        }
    }

    // 3. Waveform (既存)
    // ...
}
```

**ルール:**
- `energy == 0` → PSD 計算スキップ（ゼロ除算防止）
- `energy_short > energy` (負の PSD) → そのまま fill（物理情報あり）
- ファームウェアタイプによるフィルタリング → 不要。`energy > 0` チェックのみ

### 3.5 REST API エンドポイント

| エンドポイント | メソッド | 説明 |
|-------------|--------|------|
| `GET /api/histograms/:module/:channel` | GET | Energy ヒストグラム (既存) |
| `GET /api/histograms/:module/:channel?type=psd` | GET | 1D PSD ヒストグラム |
| `GET /api/histograms2d/:module/:channel` | GET | 2D Energy vs PSD |
| `GET /api/calibration` | GET | 全キャリブレーション係数 |
| `PUT /api/calibration/:module/:channel` | PUT | キャリブレーション設定 |
| `DELETE /api/calibration/:module/:channel` | DELETE | キャリブレーション削除 |

### 3.6 2D ヒストグラムの転送

- JSON: flat `Vec<u64>` 配列として送信
- **gzip 圧縮** (axum の tower-http compression) で十分（ゼロビンが多く圧縮率高い）
- 800KB → gzip で数十KB に圧縮される見込み
- スパース表現・差分更新は必要になってから検討

### 3.7 キャリブレーション (Operator 側)

```json
// config/calibration.json
{
    "channels": {
        "0:0": { "coefficients": [-10.0, 0.5], "unit": "keV" },
        "0:1": { "coefficients": [-8.0, 0.5, 0.00001], "unit": "keV" }
    }
}
```

- `coefficients`: `[c₀, c₁, c₂, ...]` → `keV = c₀ + c₁*ADC + c₂*ADC² + ...`
- 1次 (2係数) = 線形、2次 (3係数) = 放物線
- 係数の数で次数を自動判定

---

## 4. フロントエンド実装詳細

### 4.1 2D ヒストグラム表示

- **ECharts heatmap** を使用
- カラースケール: **対数スケール必須** (低カウント〜高カウントのダイナミックレンジが大きい)
- X 軸: Energy (ADC), Y 軸: PSD
- リアルタイム更新: 1秒ポーリング
- **表示中のチャンネルのみ fetch** — 全チャンネルの 2D データをポーリングしない

### 4.1.1 Tune Up モードでの表示切り替え

現在の Tune Up レイアウト:
- 上段左: Waveform (波形)
- 上段右: **ChargeLong** (Energy 1D ヒストグラム) ← ここを切り替え可能に
- 下段: パラメータテーブル

上段右パネルのヘッダーに **mat-button-toggle-group** を追加:
- **Energy**: 1D Energy ヒストグラム (既存、デフォルト)
- **PSD 2D**: 2D Energy vs PSD heatmap (新規)

```
┌─────────────────────┐ ┌─────────────────────────────┐
│   Waveform          │ │ [Energy] [PSD 2D]  Log  ▼   │
│                     │ │                             │
│   ~~~waveform~~~    │ │  ████ heatmap ████          │
│                     │ │  ████  or     ████          │
│                     │ │  1D histogram               │
└─────────────────────┘ └─────────────────────────────┘
┌─────────────────────────────────────────────────────┐
│ [All] [Input] [Trigger] [Energy] ...   [Apply]      │
│   Parameter Table                                   │
└─────────────────────────────────────────────────────┘
```

**ワークフロー:**
1. PSD パラメータ調整 (short gate, long gate) → Apply
2. PSD 2D に切り替え → n-γ 分離を確認
3. Energy に切り替え → スペクトル形状を確認
4. 満足するまで繰り返し

### 4.2 ゲート定義 UI

- **多角形ゲート** を最初から実装（矩形は多角形の特殊ケース）
- マウス操作: クリックで頂点追加、ダブルクリックで閉じる、ドラッグで頂点移動
- **ゲート内カウント**: フロントエンドで計算（2D ヒストグラムデータは既にクライアントにある）
  - 102,400 ビンに対する point-in-polygon テストは JS で十分高速
- ゲートパラメータ: JSON で保存/読み込み（`config/gates.json` or localStorage）
- ゲートは Monitor の表示機能。Event Builder への連携は将来課題

### 4.3 X 軸キャリブレーション変換 (多項式対応)

```typescript
// 多項式変換: keV = c₀ + c₁*ADC + c₂*ADC² + ...
function calibrate(adc: number, coefficients: number[]): number {
  return coefficients.reduce((sum, c, n) => sum + c * adc ** n, 0);
}

const xAxisData = bins.map((_, i) => calibration
  ? calibrate(i, calibration.coefficients)
  : i);
```

- XAxisLabel 切り替え: Channel (raw) / keV / MeV
- keV → MeV は /1000 の追加変換
- 係数の数で次数を自動判定（2個=線形, 3個=2次）

### 4.4 キャリブレーション設定 UI (Monitor ページ内)

**UI フロー:**
1. 「Auto Detect」ボタン → ヒストグラムの微分でピーク位置を自動検出
   - smoothing + 微分ゼロ交差 + threshold でピーク候補を抽出
   - 各ピーク周辺を自動で Gaussian Fit → center (ADC) を取得
2. 検出されたピーク一覧を表示、ユーザーが既知エネルギー値 (keV) を入力
   - 手動でピーク追加も可能（従来通り Fit → 「キャリブレーションに追加」）
3. 多項式次数を選択 (1次=線形 / 2次)
4. 「計算」→ 最小二乗法で係数を自動計算
5. 結果表示: coefficients, R² → 「適用」で保存

キャリブレーションパネルは Monitor ページ内のサイドバー or ダイアログ。

### 4.5 新規サービス

```typescript
// CalibrationService
@Injectable({ providedIn: 'root' })
export class CalibrationService {
    calibrations = signal<Map<string, Calibration>>(new Map());
    loadCalibrations(): void { ... }
    saveCalibration(module: number, channel: number, cal: Calibration): Observable<void> { ... }
    calibrate(adc: number, module: number, channel: number): number | null { ... }
}

interface Calibration {
    coefficients: number[];  // [c₀, c₁, c₂, ...] → keV = c₀ + c₁*ADC + c₂*ADC² + ...
    unit: 'keV' | 'MeV';
}

// PeakDetectionService (FittingService に追加 or 新規)
function detectPeaks(bins: number[], options?: { threshold?: number; minDistance?: number }): PeakCandidate[];

interface PeakCandidate {
    binIndex: number;     // ピーク位置 (bin)
    height: number;       // ピーク高さ
    fitResult?: GaussianFitResult;  // 自動 Fit 結果
}
```

---

## 5. 実装フェーズ

### Phase 1: PSD バックエンド (1D + 2D ヒストグラム) — **完了** (2026-02-20)
- [x] `Histogram2D` 構造体 + `fill(x, y)` メソッド
- [x] `MonitorState` に `psd_histograms` + `psd2d_histograms` 追加
- [x] PSD config を TOML から読み込み
- [x] `process_event()` に PSD 計算ロジック追加
- [x] チャンネル登録時に PSD/2D ヒストグラムも作成
- [x] REST API: `?type=psd` + `/histograms2d/` エンドポイント
- [x] gzip 圧縮 — 既に `CompressionLayer` で有効化済み

### Phase 2: PSD フロントエンド (1D + 2D 表示) — **完了** (2026-02-20)
- [x] HistogramService に `fetchPsdHistogram()` + `fetchHistogram2d()` 追加
- [x] 2D heatmap コンポーネント (ECharts heatmap + viridis カラースケール + log 対応)
- [ ] Monitor: Setup Tab に histogram type 選択追加 — 将来タスク
- [ ] Monitor: 1D PSD ヒストグラム表示 — 将来タスク
- [x] **Tune Up: 上段右パネルの表示切り替え (Energy 1D ↔ PSD 2D)**

### Phase 3: ゲート機能 — **スキップ**
> 3月 MVP では不要。ゲート弁別は Event Builder 側で実装する方が適切。必要になった時点で再検討する。

- ~~多角形ゲート描画 UI~~
- ~~ゲート内カウント計算 (point-in-polygon)~~
- ~~ゲートパラメータ保存/読み込み~~
- ~~ゲート頂点のドラッグ移動~~

### Phase 4: Energy Calibration (バックエンド)
- [ ] `config/calibration.json` スキーマ定義
- [ ] Operator REST API: キャリブレーション CRUD
- [ ] 起動時に calibration.json 読み込み

### Phase 5: Energy Calibration (フロントエンド)
- [ ] CalibrationService 実装 (多項式対応: coefficients 配列)
- [ ] HistogramChartComponent: X 軸多項式キャリブレーション変換
- [ ] XAxisLabel 切り替え機能 (Channel/keV/MeV)
- [ ] キャリブレーション設定パネル (Monitor ページ内)
- [ ] 自動ピーク検出 (微分ゼロ交差 + threshold + 自動 Gaussian Fit)
- [ ] 多項式次数選択 (1次/2次) + 最小二乗法で係数計算
- [ ] Gaussian Fit 結果 → キャリブレーションペア連携

---

## 6. 影響範囲

### バックエンド変更ファイル
| ファイル | 変更内容 |
|---------|---------|
| `src/monitor/mod.rs` | Histogram2D + MonitorState + process_event + PSD config |
| `src/monitor/routes.rs` | REST API: type パラメータ + histograms2d + gzip |
| `src/operator/routes/` | Calibration CRUD API (新規ファイル) |

### フロントエンド変更ファイル
| ファイル | 変更内容 |
|---------|---------|
| `models/histogram.types.ts` | Histogram2D, Calibration, Gate 型追加 |
| `services/histogram.service.ts` | type パラメータ + 2D fetch |
| `services/calibration.service.ts` | 新規: キャリブレーション管理 |
| `components/histogram-chart/` | X 軸キャリブレーション変換 |
| `components/heatmap-chart/` | 新規: 2D heatmap + ゲート描画 |
| `pages/monitor/` | Setup Tab + キャリブレーションパネル |
| `pages/waveform/waveform.component.ts` | Tune Up: Energy ↔ PSD 2D 表示切り替え |

---

## 7. 将来の拡張

| 機能 | 担当 | 優先度 |
|------|------|--------|
| ゲート → Event Builder 連携 | Event Builder | 中 |
| カウントレートトレンド | Monitor | 低 |

---

## 8. 決定済み事項

1. ~~PHA1 で PSD = 1.0~~ → **注意喚起不要** (正常動作として扱う)
2. PSD ヒストグラム設定 → **Monitor config TOML で設定可能にする** (ビン数・範囲)
3. キャリブレーション UI → **Monitor ページ内に配置**
   - Fit → ピーク入力 → 係数計算 → 適用を画面遷移なしで完結させる
   - HistogramCell の Fit 結果から「キャリブレーションに追加」ボタンで直接転送
   - キャリブレーションパネル (サイドバー or ダイアログ) で係数管理
4. **2D ヒストグラム (Energy vs PSD) を最初から実装** — 1D PSD だけでは弁別不十分
5. **ゲート**: 多角形ゲート、Monitor UI 機能のみ (Event Builder 連携は将来)
6. **ゲート内カウント計算**: フロントエンドで実施（データは既にクライアントにある）
7. **2D データ転送**: gzip 圧縮のみ（スパース/差分は必要になってから）
8. **1D ヒストグラム**: 2D の projection ではなく独立して蓄積（高速 + 異なるビン設定可能）
9. **2D データ fetch**: 表示中のチャンネルのみ fetch（バックエンドは全チャンネル蓄積）
10. **Tune Up モード**: 上段右パネルで Energy 1D ↔ PSD 2D を切り替え可能にする
11. **キャリブレーション**: 最初から多項式対応 (`coefficients: [c₀, c₁, c₂, ...]`)
12. **自動ピーク検出**: 微分ゼロ交差 + 自動 Gaussian Fit → Phase 5 に含める
