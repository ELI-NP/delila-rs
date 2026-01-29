# HV Gain Matcher — 実装詳細書

**Version:** 0.2.0
**Created:** 2026-01-28
**Updated:** 2026-01-28
**Status:** ✅ Phase 1 完了 (HV 接続 + READ/WRITE 検証済み)

---

## 実装状況

| Phase | 内容 | 状態 |
|-------|------|------|
| 1 | HV 接続 + status コマンド | ✅ 完了 |
| 2 | DAQ クライアント + フィッター | ✅ 実装済み (未テスト) |
| 3 | match ループ | ✅ 実装済み (未テスト) |
| 4 | 実運用テスト | 📋 結線完了後 |

### 実装上の注意点 (実機検証で判明)

1. **パラメータ名の違い:** ボードによって `VSet` ではなく `V0Set` を使用
   - 解決: `_get_float_param_fallback()` で両方試す

2. **GetCrateMap の segfault:** ctypes の型定義が CAEN の char** 戻り値と不整合
   - 解決: スロット直接プローブ方式に変更 (`_probe_slot_channels()`)

3. **GetChName の失敗:** 一部スロットで名前取得不可
   - 解決: 例外時に `ch{n}` のデフォルト名を使用

4. **OpenSSL 1.1 依存:** Ubuntu 22.04 は OpenSSL 3.0 のため libcrypto.so.1.1 がない
   - 解決: `LD_LIBRARY_PATH=/snap/caengeco2020/x1/lib:/usr/lib64`

### 実行方法

```bash
cd /tmp/hv_cal
LD_LIBRARY_PATH=/snap/caengeco2020/x1/lib:/usr/lib64 \
python3 gain_matcher.py status --config examples/gain_config.yaml
```

---

## 1. モジュール構成

```
gain_matcher.py    ← CLI エントリポイント (argparse)
       │
       ├── config.py        ← YAML 設定読み込み + チャンネルマッピング展開
       ├── daq_client.py    ← delila-rs REST API クライアント
       ├── hv_control.py    ← CAENHVWrapper ctypes ラッパー
       └── fitter.py        ← ガウシアンフィット + ピーク検出
```

依存方向: `gain_matcher → config, daq_client, hv_control, fitter`
各モジュールは独立してテスト可能。

---

## 2. hv_control.py — CAENHVWrapper ctypes ラッパー

### 2.1 設計方針

- `ctypes.CDLL` で `libcaenhvwrapper.so` をロード
- RAII パターン: `__enter__`/`__exit__` で InitSystem/DeinitSystem
- 全 API コールの戻り値チェック (CAENHV_OK 以外は例外)
- 型変換は ctypes 側で吸収、ユーザーには Python ネイティブ型を返す

### 2.2 クラス設計

```python
class CAENHVError(Exception):
    """CAENHVWrapper エラー"""
    def __init__(self, code: int, message: str): ...

class HVChannel:
    """1チャンネルの状態 (読み取り専用データクラス)"""
    slot: int
    channel: int
    name: str         # GECO2020 で設定したチャンネル名
    v_set: float      # 設定電圧 (V0Set or VSet)
    v_mon: float      # 実電圧
    i_mon: float      # 電流 (μA)
    status: int       # ステータスビット
    pw: int           # ON=1 / OFF=0
    sv_max: float     # ソフトウェア最大電圧 (GECO2020 で設定)

class HVController:
    """SY5527 制御クラス"""

    def __init__(self, host: str, username: str, password: str):
        self._lib = ctypes.CDLL("libcaenhvwrapper.so")
        self._handle = ctypes.c_int(-1)
        self._host = host
        self._username = username
        self._password = password

    def __enter__(self) -> "HVController":
        """接続"""
        # CAENHV_InitSystem(SY5527=3, LINKTYPE_TCPIP=0, host, user, pass, &handle)
        ...

    def __exit__(self, *args):
        """切断"""
        # CAENHV_DeinitSystem(handle)
        ...

    def get_crate_map(self) -> list[dict]:
        """スロット/ボード構成を取得
        Returns: [{"slot": 0, "model": "A1733P", "channels": 24, "serial": 12345}, ...]
        """
        # CAENHV_GetCrateMap(handle, ...)
        ...

    def get_channel_params(self, slot: int, channels: list[int]) -> list[HVChannel]:
        """複数チャンネルのパラメータを一括取得
        VSet, VMon, IMon, Status, Pw を一度に読む
        """
        # CAENHV_GetChParam(handle, slot, "VSet", n, ch_list, values)
        # CAENHV_GetChParam(handle, slot, "VMon", n, ch_list, values)
        # ... 各パラメータについて呼び出し
        ...

    def set_voltage(self, slot: int, channel: int, voltage: float):
        """1チャンネルの電圧を設定"""
        # CAENHV_SetChParam(handle, slot, "VSet", 1, [ch], [voltage])
        ...

    def set_voltages(self, slot: int, channels: list[int], voltages: list[float]):
        """複数チャンネルの電圧を一括設定"""
        # CAENHV_SetChParam(handle, slot, "VSet", n, ch_list, voltage_list)
        ...

    def set_power(self, slot: int, channel: int, on: bool):
        """チャンネルの ON/OFF"""
        # CAENHV_SetChParam(handle, slot, "Pw", 1, [ch], [1 if on else 0])
        ...

    def wait_ramp(self, slot: int, channels: list[int],
                  tolerance: float = 1.0, timeout: float = 60.0) -> bool:
        """全チャンネルの VMon が VSet ± tolerance に収まるまで待つ"""
        ...

    def _check_error(self, result: int):
        """エラーチェック"""
        if result != 0:  # CAENHV_OK
            msg = self._lib.CAENHV_GetError(self._handle)
            raise CAENHVError(result, msg.decode() if msg else f"Error {result}")
```

### 2.3 ctypes 型マッピング

```python
# CAENHVWrapper API のシグネチャ定義
def _setup_prototypes(self):
    # InitSystem
    self._lib.CAENHV_InitSystem.argtypes = [
        ctypes.c_int,           # system type (SY5527=3)
        ctypes.c_int,           # link type (TCPIP=0)
        ctypes.c_void_p,        # arg (host string)
        ctypes.c_char_p,        # username
        ctypes.c_char_p,        # password
        ctypes.POINTER(ctypes.c_int)  # handle
    ]
    self._lib.CAENHV_InitSystem.restype = ctypes.c_int

    # GetChParam (float params like VSet, VMon)
    self._lib.CAENHV_GetChParam.argtypes = [
        ctypes.c_int,           # handle
        ctypes.c_ushort,        # slot
        ctypes.c_char_p,        # param name
        ctypes.c_ushort,        # num channels
        ctypes.POINTER(ctypes.c_ushort),  # channel list
        ctypes.c_void_p         # values (float* or int*)
    ]
    self._lib.CAENHV_GetChParam.restype = ctypes.c_int

    # SetChParam
    self._lib.CAENHV_SetChParam.argtypes = [
        ctypes.c_int,           # handle
        ctypes.c_ushort,        # slot
        ctypes.c_char_p,        # param name
        ctypes.c_ushort,        # num channels
        ctypes.POINTER(ctypes.c_ushort),  # channel list
        ctypes.c_void_p         # values
    ]
    self._lib.CAENHV_SetChParam.restype = ctypes.c_int
```

### 2.4 注意点

- **VSet/VMon/IMon** は `float` 型 (ctypes.c_float の配列)
- **Status/Pw** は `unsigned int` 型 (ctypes.c_uint の配列)
- `CAENHV_GetCrateMap` の戻り値メモリは `CAENHV_Free()` で解放必要
- SY5527 の system type は `3` (enum CAENHV_SYSTEM_TYPE_t の SY5527)

---

## 3. daq_client.py — REST API クライアント

### 3.1 設計

```python
class DAQClient:
    """delila-rs REST API クライアント"""

    def __init__(self, operator_url: str = "http://localhost:8080",
                       monitor_url: str = "http://localhost:8081"):
        self._operator = operator_url
        self._monitor = monitor_url

    def get_daq_status(self) -> dict:
        """Operator API: DAQ 状態取得
        GET /api/status
        Returns: {"components": [{"name": ..., "state": ...}, ...]}
        """
        ...

    def is_running(self) -> bool:
        """DAQ が Running 状態か"""
        ...

    def get_histogram(self, module: int, channel: int) -> dict:
        """Monitor API: ヒストグラム取得
        GET /api/histograms/{module}/{channel}
        Returns: {
            "module_id": int,
            "channel_id": int,
            "bins": [int; 65536],
            "total_counts": int,
            "config": {"num_bins": 65536, "min_value": 0, "max_value": 65536}
        }
        """
        ...

    def get_all_histograms(self) -> dict:
        """Monitor API: 全チャンネル一覧
        GET /api/histograms
        Returns: {"channels": [{"module_id": m, "channel_id": c, "total_counts": n}, ...]}
        """
        ...

    def clear_histograms(self):
        """Monitor API: ヒストグラムクリア
        POST /api/histograms/clear
        """
        ...
```

### 3.2 エラーハンドリング

- HTTP 404: チャンネル未検出 → スキップ
- HTTP 500: Monitor 内部エラー → リトライ (最大3回)
- 接続エラー: DAQ 未起動 → 明確なエラーメッセージ

---

## 4. fitter.py — ガウシアンフィット

### 4.1 設計

```python
@dataclass
class FitResult:
    peak_position: float    # ガウシアンの中心 (ADC ch)
    peak_sigma: float       # ガウシアンの幅
    peak_amplitude: float   # 高さ
    chi_squared: float      # フィットの良さ
    success: bool           # フィット成功フラグ
    message: str            # エラーメッセージ (失敗時)

def gaussian(x, amplitude, center, sigma, offset):
    """ガウシアン + 定数バックグラウンド"""
    return amplitude * np.exp(-(x - center)**2 / (2 * sigma**2)) + offset

def fit_peak(bins: list[int], region: tuple[int, int],
             min_counts: int = 1000) -> FitResult:
    """指定範囲内でガウシアンフィット

    Args:
        bins: ヒストグラムデータ (65536 bins)
        region: (low, high) フィット範囲 (ADC ch)
        min_counts: 最低カウント数 (不足ならフィット失敗)

    Returns:
        FitResult

    Algorithm:
        1. bins[low:high] を切り出し
        2. 範囲内の合計カウント確認 (< min_counts なら失敗)
        3. 初期値推定:
           - center = 最大値のインデックス
           - amplitude = 最大値
           - sigma = FWHM/2.35 (半値幅から推定)
           - offset = 端の平均値
        4. scipy.optimize.curve_fit で最小二乗フィット
        5. フィットパラメータとカイ二乗を返す
    """
    ...

def find_peaks_auto(bins: list[int], min_height: int = 100,
                    min_distance: int = 50) -> list[int]:
    """自動ピーク検出 (scan モード用)

    Args:
        bins: ヒストグラムデータ
        min_height: ピーク最低高さ
        min_distance: ピーク間最低距離 (ADC ch)

    Returns:
        ピーク位置のリスト (ADC ch)

    Uses: scipy.signal.find_peaks
    """
    ...
```

### 4.2 フィット初期値推定

フィットの成否は初期値に強く依存する。以下の戦略:

```python
# region 内のデータ
x = np.arange(low, high)
y = np.array(bins[low:high], dtype=float)

# 初期値
i_max = np.argmax(y)
center0 = x[i_max]
amplitude0 = y[i_max]
offset0 = (y[0] + y[-1]) / 2

# FWHM 推定 (半値幅)
half_max = (amplitude0 - offset0) / 2 + offset0
above_half = np.where(y > half_max)[0]
if len(above_half) > 1:
    fwhm = x[above_half[-1]] - x[above_half[0]]
    sigma0 = fwhm / 2.35
else:
    sigma0 = (high - low) / 6  # fallback
```

---

## 5. config.py — 設定管理

### 5.1 設計

```python
@dataclass
class ChannelConfig:
    name: str
    hv_slot: int
    hv_ch: int
    dig_module: int
    dig_ch: int
    peak_region: tuple[int, int]
    target_position: int
    skip: bool = False

@dataclass
class GainMatcherConfig:
    # HV
    hv_host: str
    hv_username: str
    hv_password: str

    # DAQ
    operator_url: str
    monitor_url: str

    # Matching
    measure_time: int
    max_iterations: int
    tolerance_percent: float
    pmt_alpha: float
    min_counts: int
    hv_step_limit: float

    # Channels
    channels: list[ChannelConfig]

def load_config(path: str) -> GainMatcherConfig:
    """YAML 設定ファイルを読み込み、channel_ranges を展開"""
    ...
```

### 5.2 channel_ranges の展開

```yaml
channel_ranges:
  - name_prefix: "LaBr3"
    hv_slot: 0
    hv_ch_start: 0
    hv_ch_end: 23
    dig_module: 0
    dig_ch_start: 0
```

↓ 展開結果:

```python
[
    ChannelConfig("LaBr3-00", hv_slot=0, hv_ch=0, dig_module=0, dig_ch=0, ...),
    ChannelConfig("LaBr3-01", hv_slot=0, hv_ch=1, dig_module=0, dig_ch=1, ...),
    ...
    ChannelConfig("LaBr3-23", hv_slot=0, hv_ch=23, dig_module=0, dig_ch=23, ...),
]
```

`skip_channels` に含まれるチャンネルは `skip=True` に設定。

---

## 6. gain_matcher.py — メインロジック

### 6.1 CLI

```python
def main():
    parser = argparse.ArgumentParser(description="PMT Gain Matcher")
    subparsers = parser.add_subparsers(dest="command")

    # scan
    scan_parser = subparsers.add_parser("scan", help="Scan all channels")
    scan_parser.add_argument("--measure-time", type=int, default=10)
    scan_parser.add_argument("--output", type=str, default=None)

    # match
    match_parser = subparsers.add_parser("match", help="Run gain matching")
    match_parser.add_argument("--config", required=True)
    match_parser.add_argument("--max-iterations", type=int, default=None)
    match_parser.add_argument("--dry-run", action="store_true")
    match_parser.add_argument("--yes", action="store_true")

    # status
    status_parser = subparsers.add_parser("status", help="Show HV status")
    status_parser.add_argument("--config", required=True)
```

### 6.2 match ループ

```python
def run_match(config: GainMatcherConfig, dry_run: bool = False):
    daq = DAQClient(config.operator_url, config.monitor_url)
    active_channels = [ch for ch in config.channels if not ch.skip]

    with HVController(config.hv_host, config.hv_username, config.hv_password) as hv:
        for iteration in range(1, config.max_iterations + 1):
            print(f"\n=== Iteration {iteration}/{config.max_iterations} ===")

            # 1. ヒストグラムクリア + データ蓄積
            daq.clear_histograms()
            print(f"Accumulating data for {config.measure_time}s...")
            time.sleep(config.measure_time)

            # 2. フィット
            results = {}
            for ch in active_channels:
                hist = daq.get_histogram(ch.dig_module, ch.dig_ch)
                fit = fit_peak(hist["bins"], ch.peak_region, config.min_counts)
                results[ch.name] = fit

            # 3. HV 補正計算
            adjustments = []
            all_converged = True
            for ch in active_channels:
                fit = results[ch.name]
                if not fit.success:
                    print(f"  {ch.name}: fit failed ({fit.message}), skipping")
                    continue

                delta = fit.peak_position - ch.target_position
                tolerance = ch.target_position * config.tolerance_percent / 100

                if abs(delta) <= tolerance:
                    print(f"  {ch.name}: CONVERGED (peak={fit.peak_position:.1f})")
                    continue

                all_converged = False

                # V_new = V_current * (target / current)^(1/alpha)
                hv_info = hv.get_channel_params(ch.hv_slot, [ch.hv_ch])[0]
                v_current = hv_info.v_set
                ratio = ch.target_position / fit.peak_position
                v_new = v_current * (ratio ** (1.0 / config.pmt_alpha))

                # 変更量制限
                dv = v_new - v_current
                if abs(dv) > config.hv_step_limit:
                    v_new = v_current + config.hv_step_limit * (1 if dv > 0 else -1)

                adjustments.append((ch, v_current, v_new))
                print(f"  {ch.name}: peak={fit.peak_position:.1f} "
                      f"target={ch.target_position} "
                      f"V: {v_current:.1f} → {v_new:.1f}")

            if all_converged:
                print("\nAll channels converged!")
                break

            # 4. HV 設定
            if not dry_run and adjustments:
                for ch, v_old, v_new in adjustments:
                    hv.set_voltage(ch.hv_slot, ch.hv_ch, v_new)

                # Ramp 待ち
                print("Waiting for HV ramp...")
                slots = set(ch.hv_slot for ch, _, _ in adjustments)
                for slot in slots:
                    chs = [ch.hv_ch for ch, _, _ in adjustments if ch.hv_slot == slot]
                    hv.wait_ramp(slot, chs)

        # 結果保存
        save_results(config, results, iteration)
```

---

## 7. 実装順序

### Phase 1: HV 接続テスト
1. `hv_control.py` の `HVController` 実装
2. `gain_matcher.py status` コマンド
3. SY5527 接続 → CrateMap → チャンネル状態表示

**検証:** `python3 gain_matcher.py status --config gain_config.yaml`

### Phase 2: DAQ クライアント + フィッター
1. `daq_client.py` 実装
2. `fitter.py` 実装
3. `gain_matcher.py scan` コマンド

**検証:** DAQ Running 中に `python3 gain_matcher.py scan`

### Phase 3: ゲインマッチング
1. `config.py` の channel_ranges 展開
2. `gain_matcher.py match` ループ
3. `--dry-run` でシミュレーション

**検証:** `python3 gain_matcher.py match --config gain_config.yaml --dry-run`

### Phase 4: 実運用テスト
1. 数チャンネルで実 HV 調整テスト
2. 全チャンネルゲインマッチング
3. 結果検証 (フィット結果のプロット)

---

## 8. テスト戦略

### ユニットテスト (ハードウェア不要)

```python
# test_fitter.py
def test_gaussian_fit():
    """既知のガウシアンデータでフィット精度確認"""
    x = np.arange(0, 1000)
    y = 500 * np.exp(-(x - 400)**2 / (2 * 30**2)) + 10
    bins = [0] * 65536
    for i, v in enumerate(y):
        bins[i] = int(v)
    result = fit_peak(bins, (300, 500))
    assert result.success
    assert abs(result.peak_position - 400) < 1.0
    assert abs(result.peak_sigma - 30) < 2.0

def test_fit_no_peak():
    """ピークなしデータでフィット失敗を確認"""
    bins = [10] * 65536
    result = fit_peak(bins, (300, 500), min_counts=1000)
    assert not result.success

def test_config_expand_ranges():
    """channel_ranges の展開テスト"""
    ...
```

### 統合テスト (ハードウェア必要)

```python
# test_hv_integration.py (要 SY5527 接続)
def test_connect_and_read():
    with HVController("172.18.5.215", "admin", "eli-np") as hv:
        crate_map = hv.get_crate_map()
        assert len(crate_map) > 0
        # Read first populated slot
        slot = crate_map[0]
        channels = hv.get_channel_params(slot["slot"], [0])
        assert channels[0].v_mon >= 0
```

---

## 9. エラーハンドリング方針

| 状況 | 対応 |
|------|------|
| SY5527 接続失敗 | 即座に終了、エラーメッセージ表示 |
| DAQ 未起動 | 即座に終了、「DAQ を起動してください」 |
| DAQ が Running でない | scan/match 時に警告、続行確認 |
| フィット失敗 (特定ch) | そのチャンネルをスキップ、ログに記録 |
| カウント不足 | そのチャンネルをスキップ、「蓄積時間を延ばしてください」 |
| HV 変更量が大きすぎる | step_limit で制限、複数イテレーションで追従 |
| VMon が VSet に追いつかない | タイムアウト後にスキップ、警告 |
| 全チャンネルフィット失敗 | 即座に終了、設定の見直しを提案 |
