# HV Gain Matcher — 仕様書

**Version:** 0.2.0
**Created:** 2026-01-28
**Updated:** 2026-01-28
**Author:** Aogaki / Claude
**Status:** ✅ 実機検証完了 (READ/WRITE)

---

## 実機検証結果 (2026-01-28)

| 機能 | 結果 | 備考 |
|------|------|------|
| SY5527 接続 | ✅ | CAENHVWrapper via ctypes |
| 全スロット読み出し | ✅ | 10 スロット × 12ch = 120ch |
| チャンネル名取得 | ✅ | GECO2020 で設定した名前 |
| V0Set/VMon/IMon/SVMax 読み出し | ✅ | ボード種別による V0Set/VSet 差異に対応 |
| V0Set 書き込み | ✅ | 955V → 900V → 955V 復元確認 |
| SVMax 制限 | ✅ | 書き込み時に自動クランプ |

**注意:** OpenSSL 1.1 依存のため実行時に `LD_LIBRARY_PATH=/snap/caengeco2020/x1/lib:/usr/lib64` が必要。

---

## 1. 目的

PMT アレイ (~100ch) のゲインマッチングを自動化する。
線源のフォトピーク位置を指定された ADC チャンネルに揃えるために、
HV を自動調整するツール。

---

## 2. システム構成

```
┌──────────────────┐          ┌──────────────────┐
│  delila-rs DAQ   │          │  CAEN SY5527     │
│  (172.18.4.76)   │          │  (172.18.5.215)  │
│                  │          │                  │
│  Operator :8080  │          │  CAENHVWrapper   │
│  Monitor  :8081  │          │  (TCP/IP)        │
└────────┬─────────┘          └────────┬─────────┘
         │ REST API                    │ libcaenhvwrapper.so
         │                            │
         └──────────┐  ┌──────────────┘
                    │  │
              ┌─────▼──▼──────┐
              │ gain_matcher   │
              │ (Python 3.10)  │
              │ @ 172.18.4.76  │
              └────────────────┘
```

**実行環境:** Linux DAQ マシン (172.18.4.76)
- CAENHVWrapper ライブラリが必要 (`/usr/lib64/libcaenhvwrapper.so`)
- delila-rs の Monitor REST API にローカルアクセス

---

## 3. ハードウェア

| 機器 | モデル | 接続 | 備考 |
|------|--------|------|------|
| HV Supply | CAEN SY5527 | TCP/IP (172.18.5.215) | admin/eli-np |
| Digitizer | CAEN VX2730 | Ethernet (dig2://172.18.4.56) | DPP-PSD2, 32ch |
| Digitizer | CAEN DT5730B | USB | DPP-PSD1, 8ch |
| 検出器 | PMT アレイ | ~100ch | LaBr3, NaI 等 |

---

## 4. 対応線源

| 線源 | ピークエネルギー | 用途 |
|------|-----------------|------|
| 137Cs | 662 keV | 単一ピーク、標準的なゲイン校正 |
| 60Co | 1173 keV, 1332 keV | 2ピーク、高エネルギー側校正 |
| 152Eu | 121, 244, 344, 779, 964, 1112, 1408 keV | 多ピーク、エネルギー校正 |

ユーザーが `peak_region` (ADC チャンネル範囲) で使用するピークを指定する。

---

## 5. 動作モード

### 5.1 scan モード

全チャンネルのヒストグラムを取得し、ピーク位置を自動検出して設定テンプレートを生成。

```bash
$ python3 gain_matcher.py scan --measure-time 10
```

**フロー:**
1. DAQ が Running 状態であることを確認 (Operator API)
2. ヒストグラムをクリア (Monitor API)
3. 指定時間待機 (データ蓄積)
4. 全チャンネルのヒストグラムを取得 (Monitor API)
5. 各チャンネルで最大ピークを自動検出 (`scipy.signal.find_peaks`)
6. 結果テーブルを表示
7. YAML 設定ファイルとして保存

**出力例:**
```
Module | Ch | Counts | Peak ADC | Suggested Region
-------|-----|--------|----------|------------------
     0 |   0 |  15234 |      412 | [362, 462]
     0 |   1 |  12891 |      398 | [348, 448]
     0 |   2 |      0 |     none | (skip)
...
Saved to: scan_result_20260128_153045.yaml
```

### 5.2 match モード

設定ファイルに基づいてゲインマッチングを実行。

```bash
$ python3 gain_matcher.py match --config gain_config.yaml --max-iterations 10
```

**フロー:**
1. 設定ファイル読み込み
2. SY5527 に接続 (CAENHVWrapper)
3. **反復ループ:**
   a. ヒストグラムクリア
   b. データ蓄積 (measure_time 秒)
   c. 各チャンネルのヒストグラム取得
   d. `peak_region` 内でガウシアンフィット → ピーク位置
   e. 目標位置との差を計算
   f. HV 補正量を計算 (PMT ゲインモデル: Gain ∝ V^α)
   g. 新しい HV を設定
   h. 収束判定 (全チャンネルが許容範囲内なら終了)
4. 結果レポート出力

**収束判定:**
- 全チャンネルのピーク位置が `target_position ± tolerance` 内
- デフォルト: tolerance = target_position の 2%

### 5.3 status モード

SY5527 の全チャンネル状態を表示。

```bash
$ python3 gain_matcher.py status
```

---

## 6. 設定ファイル (YAML)

```yaml
# gain_config.yaml

# --- HV Supply ---
hv:
  host: "172.18.5.215"
  username: "admin"
  password: "eli-np"
  system_type: "SY5527"

# --- DAQ ---
daq:
  operator_url: "http://localhost:8080"
  monitor_url: "http://localhost:8081"

# --- Gain Matching Parameters ---
matching:
  measure_time: 10          # データ蓄積時間 (秒)
  max_iterations: 10        # 最大反復回数
  tolerance_percent: 2.0    # 収束判定 (target の ±%)
  pmt_alpha: 7.0            # PMT ゲイン指数 (Gain ∝ V^α)
  min_counts: 1000          # フィットに必要な最低カウント数
  hv_step_limit: 50.0       # 1回の最大HV変更量 (V)

# --- Channel Mapping ---
# デフォルト設定 (全チャンネルに適用)
defaults:
  peak_region: [300, 500]     # フィット範囲 (ADC ch)
  target_position: 1000       # 目標ピーク位置 (ADC ch)

# チャンネルマッピング: HV ch ↔ Digitizer ch
channels:
  - name: "LaBr3-01"
    hv_slot: 0
    hv_ch: 0
    dig_module: 0
    dig_ch: 0
  - name: "LaBr3-02"
    hv_slot: 0
    hv_ch: 1
    dig_module: 0
    dig_ch: 1
    peak_region: [400, 600]   # このchだけ範囲が異なる
  # ... 100ch 分 ...

# チャンネルを一括生成する場合
channel_ranges:
  - name_prefix: "LaBr3"
    hv_slot: 0
    hv_ch_start: 0
    hv_ch_end: 23             # 24ch
    dig_module: 0
    dig_ch_start: 0
  - name_prefix: "NaI"
    hv_slot: 1
    hv_ch_start: 0
    hv_ch_end: 23
    dig_module: 1
    dig_ch_start: 0

# スキップするチャンネル
skip_channels:
  - { hv_slot: 0, hv_ch: 15 }  # 未接続
  - { hv_slot: 1, hv_ch: 20 }
```

---

## 7. HV 補正アルゴリズム

PMT のゲイン特性:

```
Gain = k × V^α    (α ≈ 7-8, PMT 依存)
```

現在のピーク位置 `P_current` を目標位置 `P_target` に移動するための HV:

```
V_new = V_current × (P_target / P_current)^(1/α)
```

**安全制約:**
- `|V_new - V_current| ≤ hv_step_limit` (1回の変更量制限)
- `V_new ≤ V_max` (ボードの最大電圧を超えない)
- `V_new ≥ 0`
- HV 変更後は ramp 完了を待つ (VMon が VSet の ±1V 以内)

---

## 8. Monitor REST API 使用

| エンドポイント | 用途 |
|---------------|------|
| `GET /api/histograms` | チャンネル一覧 + 統計 |
| `GET /api/histograms/:module/:ch` | ヒストグラムデータ (bins[65536]) |
| `POST /api/histograms/clear` | ヒストグラムクリア |
| `GET /api/status` | Monitor 状態確認 |

Operator API:

| エンドポイント | 用途 |
|---------------|------|
| `GET /api/status` | DAQ 状態確認 (Running であること) |

---

## 9. CAENHVWrapper API 使用

| 関数 | 用途 |
|------|------|
| `CAENHV_InitSystem(SY5527, TCPIP, host, user, pass)` | 接続 |
| `CAENHV_DeinitSystem(handle)` | 切断 |
| `CAENHV_GetCrateMap(handle)` | スロット/ボード構成取得 |
| `CAENHV_GetChParam(handle, slot, "VSet", ...)` | 設定電圧読み取り |
| `CAENHV_GetChParam(handle, slot, "VMon", ...)` | 実電圧読み取り |
| `CAENHV_GetChParam(handle, slot, "IMon", ...)` | 電流読み取り |
| `CAENHV_GetChParam(handle, slot, "Status", ...)` | ステータス読み取り |
| `CAENHV_SetChParam(handle, slot, "VSet", ...)` | 電圧設定 |
| `CAENHV_SetChParam(handle, slot, "Pw", ...)` | ON/OFF |

Python からは `ctypes` で `libcaenhvwrapper.so` を直接呼び出す。

---

## 10. 出力・ログ

### 反復ログ (標準出力)

```
=== Iteration 1/10 ===
Clearing histograms...
Accumulating data for 10s...
Fitting peaks...
Ch  | Peak ADC | Target | Delta  | V_old  | V_new  | Status
----|----------|--------|--------|--------|--------|--------
  0 |    412.3 |   1000 | -587.7 | 1200.0 | 1412.5 | ADJUSTING
  1 |    398.1 |   1000 | -601.9 | 1180.0 | 1398.2 | ADJUSTING
  2 |    995.2 |   1000 |   -4.8 | 1350.0 | 1350.0 | CONVERGED
...
Waiting for HV ramp (15s)...

=== Iteration 5/10 ===
...
All channels converged! Final results saved to: result_20260128_153045.yaml
```

### 結果ファイル (YAML)

```yaml
timestamp: "2026-01-28T15:30:45"
iterations: 5
converged: true
channels:
  - name: "LaBr3-01"
    final_hv: 1352.4
    final_peak: 998.7
    target: 1000
    delta_percent: 0.13
```

---

## 11. 安全機能

1. **HV 変更量制限:** 1回の最大変更量を `hv_step_limit` (デフォルト 50V) に制限
2. **最大電圧制限:** ボードの VMax を超えない
3. **ドライラン:** `--dry-run` フラグで HV 変更なしにシミュレーション
4. **確認プロンプト:** 初回 HV 変更前にユーザー確認 (`--yes` でスキップ可能)
5. **異常検出:** フィット失敗、ピーク消失、電流異常時は該当チャンネルをスキップ
6. **ログ:** 全 HV 変更を記録

---

## 12. 依存関係

| パッケージ | バージョン | 用途 |
|-----------|-----------|------|
| Python | >= 3.10 | 実行環境 |
| scipy | >= 1.7 | ガウシアンフィット、ピーク検出 |
| numpy | >= 1.21 | 数値計算 |
| requests | >= 2.25 | REST API クライアント |
| PyYAML | >= 5.4 | 設定ファイル |
| matplotlib | (optional) | フィット結果のプロット |

CAENHVWrapper (`libcaenhvwrapper.so`) は `ctypes` で直接呼び出し。
追加の Python パッケージは不要。

---

## 13. ファイル構成

```
tools/hv_calibration/
├── SPECIFICATION.md          # 本仕様書
├── IMPLEMENTATION.md         # 実装詳細書
├── gain_matcher.py           # メインスクリプト (CLI)
├── hv_control.py             # CAENHVWrapper ctypes ラッパー
├── daq_client.py             # delila-rs REST API クライアント
├── fitter.py                 # ガウシアンフィット + ピーク検出
├── config.py                 # 設定ファイル読み込み
├── requirements.txt          # pip dependencies
└── examples/
    └── gain_config.yaml      # サンプル設定
```
