# Peak Fitting Design — Piecewise Linear Background Model

**Date:** 2026-02-05
**Status:** Implemented

## Overview

Monitor のヒストグラムフィッティングで使用するピーク解析モデルの設計。
γ線スペクトル解析における物理的背景を反映した折線バックグラウンドモデルを採用。

## 物理的動機

エネルギースペクトルにおけるγ線光電ピークのバックグラウンドは、ピークの左右で異なる性質を持つ：

- **左側（低エネルギー側）:** コンプトン散乱による連続分布が重畳 → 傾きが大きい
- **右側（高エネルギー側）:** 基本的に「クリーン」なBG → 傾きが小さい/平坦

単純な1本の直線BGでは、この非対称性を表現できない。

## モデル

### フィッティング関数

```
y(x) = A * exp(-0.5 * ((x - μ) / σ)^2) + BG(x)
```

### 折線バックグラウンド BG(x)

3つの領域から成る折線関数：

```
BG(x) = | b_L + m_L * x                         x < μ - 2.5σ  (左BG領域)
         | linear_interpolation(left, right)      μ - 2.5σ ≤ x ≤ μ + 2.5σ  (ピーク領域)
         | b_R + m_R * x                         x > μ + 2.5σ  (右BG領域)

if BG(x) < 0 then BG(x) = 0
```

ピーク領域内の補間:
```
yLow  = b_L + m_L * (μ - 2.5σ)
yHigh = b_R + m_R * (μ + 2.5σ)
slope = (yHigh - yLow) / (5σ)
BG(x) = yLow + slope * (x - (μ - 2.5σ))
```

### パラメータ (7個、同時フィット)

| Index | Parameter | Description |
|-------|-----------|-------------|
| 0 | A | ガウシアン振幅 |
| 1 | μ | ピーク中心位置 |
| 2 | σ | ガウシアン幅 |
| 3 | b_L | 左BG切片 |
| 4 | m_L | 左BG傾き |
| 5 | b_R | 右BG切片 |
| 6 | m_R | 右BG傾き |

### 定数

- `BG_RANGE = 2.5` — 左右BGの境界 (μ ± 2.5σ)
- `FIT_RANGE` — ユーザーがUI上でドラッグして選択（推奨: ±5σ以上）

## 初期パラメータ推定

1. フィット範囲内でピーク（最大ビン）を探索 → μ, A の初期値
2. 左端15%のビンに線形回帰 → b_L, m_L の初期値
3. 右端15%のビンに線形回帰 → b_R, m_R の初期値
4. 左右BGの中心での外挿平均からBGを推定し、振幅を補正
5. FWHMからσを推定

## 最適化

- **アルゴリズム:** Levenberg-Marquardt (ml-levenberg-marquardt)
- **damping:** 1.5
- **maxIterations:** 200
- **errorTolerance:** 1e-8

## 導出量

| 量 | 計算式 |
|----|--------|
| FWHM | 2.355 × σ |
| Net Area | A × σ × √(2π) |
| χ²/ndf | ndf = N_bins - 7 |
| エラー | 共分散行列からの伝播 |

## チャート描画

フィット結果は5本の曲線として描画：

| 曲線 | 色 | スタイル | 範囲 |
|------|-----|---------|------|
| Total (Gaussian + BG) | 赤 `#e53935` | 実線 2px | 全域 |
| Gaussian 成分 | 緑 `#43a047` | 実線 2px | 全域 |
| BG (折線) | オレンジ `#fb8c00` | 実線 1.5px | 全域 |
| Left BG line | 紫 `#8e24aa` | 破線 1.5px | x ≤ μ - 2.5σ |
| Right BG line | ティール `#00897b` | 破線 1.5px | x ≥ μ + 2.5σ |

## 実装ファイル

| ファイル | 役割 |
|----------|------|
| `web/operator-ui/src/app/services/fitting.service.ts` | フィッティングロジック、`evaluatePiecewiseBg()` |
| `web/operator-ui/src/app/services/fitting.service.spec.ts` | テスト（非対称BG含む） |
| `web/operator-ui/src/app/components/histogram-chart/histogram-chart.component.ts` | 描画 |
| `web/operator-ui/src/app/components/histogram-expand-dialog/histogram-expand-dialog.component.ts` | UI (Fit/Clear ボタン) |
| `web/operator-ui/src/app/models/histogram.types.ts` | `ViewCellFitResult` 型定義 |

## 参考: 元の ROOT マクロ

```cpp
constexpr auto kBGRange = 2.5;
constexpr auto kFitRange = 5.;

Double_t FitFnc(Double_t *pos, Double_t *par) {
  const auto x = pos[0];
  const auto mean = par[1];
  const auto sigma = par[2];

  const auto limitHigh = mean + kBGRange * sigma;
  const auto limitLow = mean - kBGRange * sigma;

  auto val = par[0] * TMath::Gaus(x, mean, sigma);

  auto backGround = 0.;
  if (x < limitLow)
    backGround = par[3] + par[4] * x;
  else if (x > limitHigh)
    backGround = par[5] + par[6] * x;
  else {
    auto xInc = limitHigh - limitLow;
    auto yInc = (par[5] + par[6] * limitHigh) - (par[3] + par[4] * limitLow);
    auto slope = yInc / xInc;
    backGround = (par[3] + par[4] * limitLow) + slope * (x - limitLow);
  }

  if (backGround < 0.) backGround = 0.;
  val += backGround;

  return val;
}
```

## 変更履歴

| Date | Change |
|------|--------|
| 2026-02-05 | 初版: 5パラメータ同時フィット → 2ステップBG推定 → 7パラメータ折線BGモデルに最終変更 |
