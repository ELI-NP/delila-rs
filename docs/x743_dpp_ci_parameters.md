# V1743 Charge Mode パラメータリファレンス

> **⚠️ SUPERSEDED (2026-04-20)**
>
> DPP-CI (Charge Mode) は DELILA では撤退しました。本ドキュメントは記録として保存しています。
>
> **決定的根拠:** [UM2750 V1743 User Manual Rev.5](../legacy/UM2750_V1743_User_Manual_rev5.pdf) Fig 10.9
> "Group Data Format in Charge Mode" より、Charge Mode のワイヤフォーマットは
> `[REF_CELL_COLUMN | CHARGE]` のみで **TDC が存在しない**。
> 同マニュアル Sec 10.10.3.1 太字注記が「物理時刻は Standard mode の 40-bit TDC のみ」と明示。
> 本プロジェクトは時間情報から 2D 位置計算する要件のため、Charge Mode は物理的に使用不可。
>
> **撤退理由 (全要約):**
> 1. **User Manual Fig 10.9 が Charge Mode に TDC 無しと明示** — 位置計算要件を満たせない
> 2. 実装に構造体ミスマッチバグあり（x720 用 `DPP_CI_Params_t` を x743 に誤流用、`DPP_X743_Params_t` が正解）
> 3. `DPP_X743_Event_t` は `float Charge; int StartIndexCell;` のみで **TimeTag を持たない** → マルチデジタイザ間時刻相関不可
> 4. CAEN App Note `docs/Setting_CHARGE_MODE_for_the_x743_modules.pdf` と異なる API 呼び出し順序・構造体を使用
>
> **⚠️ 既知の矛盾:** 本ドキュメント §0（2026-04-15 実機 probe 結果）は「DPP_CI 切替後も `DecodeEvent → X743_EVENT_t` で
> Charge/TDC が取れる」と主張しているが、User Manual Fig 10.9 のワイヤフォーマットに TDC ビットが無い以上、
> 仮に実機で `X743_EVENT_t.TDC` に値が入っても：
> - 未初期化メモリ or 前 Run の残骸
> - 現 FW の undocumented な副作用
>
> のいずれかで信頼性なし。**本 probe 結果の TDC 部分は採用しない**。
>
> **置換設計:** Standard mode 一本化（`CAEN_DGTZ_X743_GROUP_t.TDC` 40-bit @ 5ns + lib 算出 `Charge` float）
> - [docs/plans/x743_standard_mode_design.md](plans/x743_standard_mode_design.md)
> - [TODO/47_v1743_standard_mode_redesign.md](../TODO/47_v1743_standard_mode_redesign.md)

**作成日:** 2026-04-15
**更新日:** 2026-04-15 — 実機テスト結果反映
**SUPERSEDED:** 2026-04-20
**ソース:** UM1935 Rev.17 (CAENDigitizer Library), UM2750 Rev.5 (V1743 User Manual)
**実機:** VX1743 SN:25, ROC FW 04.29, AMC FW 1.02.24 (Standard FW)

---

## 0. 実機テスト結果 (2026-04-15, x743_ci_probe)

### 重要な発見

V1743 の Charge Mode は **Standard FW に内蔵された機能** である。
`SetSAMAcquisitionMode(DPP_CI)` で切替え、`DecodeEvent → X743_EVENT_t` で Charge/Baseline/Peak が出力される。

V1720 系の DPP-CI とは異なり、V1743 には **別途 DPP-CI ファームウェアは存在しない**。
CAENDigitizerType.h に `V1743_DPP_CI_CODE (0x86)` の定義があるが、
これはライブラリ内部の識別コードであり、対応する FW 製品は確認されていない。

したがって `SetDPPParameters` / `GetDPPEvents` / `DPP_CI_Event_t` 等の
V1720 系 DPP-CI 用 API は V1743 では使用不可。設定は個別 API + レジスタ直書き、
リードアウトは Standard パスを使う。

### API 互換性テスト結果 (Charge Mode 状態で実行)

#### 使用可能 (OK)
| 関数 | 用途 |
|------|------|
| `SetSAMAcquisitionMode(DPP_CI)` | Charge Mode 切替 |
| `SetGroupEnableMask` | グループ有効化 |
| `SetRecordLength` | レコード長 |
| `SetSAMSamplingFrequency` | サンプリング周波数 |
| `SetSAMCorrectionLevel` | SAM 補正 |
| `SetSAMPostTriggerSize` | ポストトリガー |
| `SetMaxNumEventsBLT` | BLT イベント数 |
| `SetIOLevel` | I/O レベル |
| `SetAcquisitionMode` | SW/SIN/FirstTrig |
| `SetChannelDCOffset` | DC オフセット |
| `SetChannelTriggerThreshold` | トリガー閾値 |
| `SetTriggerPolarity` | トリガー極性 (Standard API) |
| `SetChannelPulsePolarity` | パルス極性 (DPP API — Charge Mode でも動く!) |
| `SetChannelSelfTrigger` | セルフトリガー |
| `SetChannelPairTriggerLogic` | ペアトリガーロジック (AND/OR + coincidence window) |
| `SetTriggerLogic` | ボードトリガーロジック (OR/AND/Majority) |
| `SetSWTriggerMode` | ソフトウェアトリガー |
| `SetExtTriggerInputMode` | 外部トリガー |
| `SetDPPPreTriggerSize` | DPP プレトリガー (**レジスタ 0x1038 に反映確認!**) |
| `MallocDPPEvents` | DPP バッファ確保 (成功するが使えるかは別問題) |
| `EnableSAMPulseGen` / `DisableSAMPulseGen` | テストパルス |

#### 使用不可 (FAIL)
| 関数 | エラー | 理由 |
|------|--------|------|
| `SetDPPParameters` | InvalidParam | V1720 系 DPP-CI 用 API。パラメータ値の妥当性は未検証 |
| `SetDPPAcquisitionMode` | InvalidParam | V1720 系 DPP-CI 用 API。V1743 非対応 |
| `SetDPPEventAggregation` | FunctionNotAllowed | V1720 系 DPP-CI 用 API。V1743 非対応 |
| `SetSAMTriggerCountVetoParam` | InvalidParam | 原因不明 (引数の問題?) |

### レジスタ探索結果 (Charge Mode)

| アドレス | 値 | 推定機能 |
|---------|-----|---------|
| `0x8000` | `0x00000114` | Board Configuration (bit4=Charge Mode?) |
| `0x800C` | `0x0000000A` | Buffer Organization |
| `0x8100` | `0x00000000` | Acquisition Control |
| `0x8104` | `0x00000580` | Acquisition Status |
| `0x810C` | `0x8000000F` | Trigger Source Enable |
| `0x8120` | `0x000000FF` | Post Trigger (board level?) |
| `0x8140` | `0x00080109` | Board Info |
| `0x1000` | `0x00000114` | Group 0 Config (Board Config のミラー?) |
| `0x100C` | `0x000000FF` | Group 0: Record Length / Buffer? |
| `0x1028` | `0x00200000` | Group 0: ??? |
| `0x102C` | `0x00000004` | Group 0: ??? |
| `0x1030` | `0x00000014` | Group 0: Post Trigger Size (=20, 設定値と一致) |
| `0x1034` | `0x0001FFFF` | Group 0: ??? (131071) |
| `0x1038` | `0x00000020` | Group 0: **DPP PreTrigger Size (=32, 設定値と一致!)** |
| `0x103C` | `0x00000031` | Group 0: ??? (49) |
| `0x1044` | `0x00000010` | Group 0: ??? (16) |
| `0x1060` | `0x0000000F` | Group 0: Trigger Mask? |
| `0x1068` | `0x00000140` | Group 0: ??? (320) |
| `0x106C` | `0xFFFF0000` | Group 0: ??? |
| `0x1070` | `0x0000001F` | Group 0: ??? (31) |
| `0x1074` | `0x00000064` | Group 0: ??? (100) |
| `0x108C` | `0x00100218` | Group 0: FW Revision? |
| `0x1094` | `0x2404FFFF` | Group 0: ??? |

### 実装方針 (修正版)

Standard FW Charge Mode では:
1. **設定**: 個別 API (`SetChannelTriggerThreshold`, `SetTriggerPolarity` 等) + レジスタ直書き (Charge ゲート幅等)
2. **リードアウト**: Standard パス (`ReadData → GetNumEvents → GetEventInfo → DecodeEvent → X743_EVENT_t`)
3. **データ**: `X743_EVENT_t.DataGroup[g].Charge` / `.Baseline` / `.Peak` / `.TDC` を使用
4. **DPP 関数は使わない**: `SetDPPParameters`, `GetDPPEvents`, `DPP_CI_Event_t` は全て不要

---

---

## 1. モード切替 — Standard vs DPP-CI

V1743 は 2 つの取得モードを持つ。モード切替は以下の API で行う。

### SetSAMAcquisitionMode (UM1935 p.57)

```c
CAEN_DGTZ_SetSAMAcquisitionMode(handle, mode);

typedef enum {
    CAEN_DGTZ_AcquisitionMode_STANDARD = 0,  // デジタルオシロスコープ（波形）
    CAEN_DGTZ_AcquisitionMode_DPP_CI   = 1,  // 電荷積分（FPGA が計算）
} CAEN_DGTZ_AcquisitionMode_t;
```

**重要:** Standard モードでは波形データが出力され、ソフトウェアで電荷積分する。
DPP-CI モードでは FPGA が電荷積分を行い、Charge/Baseline 等の数値が直接出力される。

---

## 2. DPP-CI 専用パラメータ — CAEN_DGTZ_DPP_CI_Params_t

`SetDPPParameters()` で設定する構造体。全フィールドはチャンネル配列 `[MAX_DPP_CI_CHANNEL_SIZE]` (=16)。

### SetDPPParameters (UM1935 p.68-69)

```c
CAEN_DGTZ_SetDPPParameters(handle, channelMask, &params);

typedef struct {
    int blthr  [16]; // deprecated（使用しない）
    int bltmo  [16]; // deprecated（使用しない）
    int trgho  [16]; // Trigger Hold Off (samples)
    int thr    [16]; // Trigger Threshold (LSB)
    int selft  [16]; // Channel self-trigger enable (0=Disabled, 1=Enabled)
    int csens  [16]; // Charge Sensitivity
    int gate   [16]; // Charge Integration Gate width (samples)
    int pgate  [16]; // Gate Offset (samples)
    int tvaw   [16]; // Trigger Validation Acceptance Window (samples)
    int nsbl   [16]; // Number of Samples for Baseline Mean
    CAEN_DGTZ_DPP_TriggerConfig_t trgc; // deprecated（1 に設定すること）
} CAEN_DGTZ_DPP_CI_Params_t;
```

### 各フィールド詳細

| フィールド | 説明 | 範囲 | 単位 | 用途 |
|-----------|------|------|------|------|
| `thr` | トリガー閾値 | 0-65535 | LSB (ADC counts) | ディスクリミネータ閾値。信号がこの値を超えるとトリガー生成 |
| `gate` | 電荷積分ゲート幅 | >0 | samples | CHARGE_LENGTH: 積分するサンプル数。Fig.10.6 参照 |
| `pgate` | ゲートオフセット | ≥0 | samples | ゲート開始位置のオフセット（トリガー点からの前方シフト）|
| `csens` | 電荷感度 | 0-3 | enum | 0=40, 1=160, 2=640, 3=2560 fC/LSB |
| `nsbl` | ベースライン平均サンプル数 | 0-3 | enum | 0=FIXED, 1=8, 2=32, 3=128 samples |
| `trgho` | トリガーホールドオフ | ≥0 | samples | トリガー後、次のトリガーを受け付けるまでの待ち時間 |
| `selft` | セルフトリガー有効化 | 0/1 | bool | 0=無効, 1=有効。チャンネルごとに設定 |
| `tvaw` | トリガー検証ウィンドウ | ≥0 | samples | コインシデンスモード用の時間窓 |
| `trgc` | (deprecated) | — | — | **常に 1 に設定すること** |
| `blthr` | (deprecated) | — | — | 使用しない |
| `bltmo` | (deprecated) | — | — | 使用しない |

### Charge Sensitivity (csens) 変換表

| 値 | 感度 | 備考 |
|----|------|------|
| 0 | 40 fC/LSB | 最高感度 |
| 1 | 160 fC/LSB | |
| 2 | 640 fC/LSB | |
| 3 | 2560 fC/LSB | 最低感度、大信号用 |

### Baseline Mean (nsbl) 変換表 (x720)

| 値 | サンプル数 | 備考 |
|----|----------|------|
| 0 | FIXED | 固定ベースライン |
| 1 | 8 | |
| 2 | 32 | |
| 3 | 128 | |

---

## 3. DPP 取得モード — SetDPPAcquisitionMode

DPP 取得時のデータ形式を選択する。

### SetDPPAcquisitionMode (UM1935 p.70)

```c
CAEN_DGTZ_SetDPPAcquisitionMode(handle, mode, param);

typedef enum {
    CAEN_DGTZ_DPP_ACQ_MODE_Oscilloscope = 0, // 波形取得
    CAEN_DGTZ_DPP_ACQ_MODE_List         = 1, // タイムスタンプ + エネルギーのみ
    CAEN_DGTZ_DPP_ACQ_MODE_Mixed        = 2, // 波形 + エネルギー両方
} CAEN_DGTZ_DPP_AcqMode_t;

typedef enum {
    CAEN_DGTZ_DPP_SAVE_PARAM_EnergyOnly     = 0,
    CAEN_DGTZ_DPP_SAVE_PARAM_TimeOnly       = 1,
    CAEN_DGTZ_DPP_SAVE_PARAM_EnergyAndTime  = 2,
    CAEN_DGTZ_DPP_SAVE_PARAM_None           = 3,
    // ChargeAndTime = 4 は使用不可
} CAEN_DGTZ_DPP_SaveParam_t;
```

**delila-rs では `List` + `EnergyAndTime` を使用する（波形不要、最大レート）。**

**注意:** `SetDPPAcquisitionMode()` は `SetDPPEventAggregation()` の前に呼ぶこと。

---

## 4. DPP トリガーモード — SetDPPTriggerMode

DPP-PSD と DPP-CI でのみ使用可能。

### SetDPPTriggerMode (UM1935 p.71)

```c
CAEN_DGTZ_SetDPPTriggerMode(handle, mode);

typedef enum {
    CAEN_DGTZ_DPP_TriggerMode_Normal      = 0, // 通常トリガー
    CAEN_DGTZ_DPP_TriggerMode_Coincidence = 1, // コインシデンストリガー
} CAEN_DGTZ_DPP_TriggerMode_t;
```

---

## 5. DPP イベント集約 — SetDPPEventAggregation

### SetDPPEventAggregation (UM1935 p.66)

```c
CAEN_DGTZ_SetDPPEventAggregation(handle, threshold, maxsize);
```

| 引数 | 説明 |
|------|------|
| `threshold` | ボードメモリにイベントが溜まってからリードアウトする閾値。0=ライブラリ自動 |
| `maxsize` | PC 側イベントバッファの最大サイズ (bytes)。0=自動 |

**注意:** 以下の関数を**先に**呼んでから `SetDPPEventAggregation()` を呼ぶこと:
- `SetRecordLength()`
- `SetChannelEnableMask()`
- `SetNumEventsPerAggregate()`
- `SetDPPAcquisitionMode()`

---

## 6. DPP イベント数/集約 — SetNumEventsPerAggregate

### SetNumEventsPerAggregate (UM1935 p.67)

```c
CAEN_DGTZ_SetNumEventsPerAggregate(handle, numEvents, channel);
```

| 引数 | 説明 |
|------|------|
| `numEvents` | 1 集約あたりのイベント数 |
| `channel` | チャンネルインデックス（DPP-CI では無視される可能性あり）|

---

## 7. DPP Pre-Trigger — SetDPPPreTriggerSize

### SetDPPPreTriggerSize (UM1935 p.61)

```c
CAEN_DGTZ_SetDPPPreTriggerSize(handle, ch, samples);
```

| 引数 | 説明 |
|------|------|
| `ch` | チャンネル。`ch=-1` で全チャンネル一括設定。**DPP-CI では全チャンネル同一値が必須** |
| `samples` | プレトリガーサイズ (samples) |

---

## 8. DPP-CI 出力データ構造

### CAEN_DGTZ_DPP_CI_Event_t (UM1935 p.63)

```c
typedef struct {
    uint32_t Format;     // データフォーマット
    uint32_t TimeTag;    // タイムスタンプ
    int16_t  Charge;     // 積分電荷
    int16_t  Baseline;   // ベースライン
    uint32_t *Waveforms; // 波形ポインタ（DecodeDPPWaveforms で使用）
    uint32_t Extras;     // 追加情報
} CAEN_DGTZ_DPP_CI_Event_t;
```

**読み出しフロー (DPP):**
```
ReadData() → GetDPPEvents() → [MallocDPPEvents で確保した配列に格納]
                              → チャンネルごとに numEventsArray[ch] 個のイベント
```

### GetDPPEvents (UM1935 p.62)

```c
CAEN_DGTZ_GetDPPEvents(handle, buffer, buffsize, events, numEventsArray);
```

- `events`: `MallocDPPEvents` で確保した `CAEN_DGTZ_DPP_CI_Event_t` 配列
- `numEventsArray`: チャンネルごとのイベント数配列

---

## 9. Charge Mode ハードウェア動作 (UM2750 Sec. 10.10.2)

FPGA がイベントごとに以下を計算:

1. `REF_CELL_FOR_CHARGE` — 積分開始セル位置
2. `CHARGE_LENGTH` — 積分するセル数 (`gate` パラメータ)
3. 積分結果が `CHARGE_FIFO` に格納 (256 イベント/ch)
4. `CHARGE_THRESHOLD` で積分結果をフィルタ可能

**データ形式 (Charge Mode, Fig. 10.9):**
- 512 words/group (256 events x 2 channels)
- 各 word: `[31:30]=00 | [29:24]=REF_CELL_COLUMN(0-63) | [23]=1 | [22:0]=CHARGE(23-bit 2の補数, pC)`

**最大レート:** ~7 kHz（デッドタイム 13us + (N_samples/16) * 1.75us）

---

## 10. Standard モードのパラメータ

Standard モードで使用する場合、DPP 関数は使わず通常の Acquisition 関数を使用する。

### SetRecordLength (UM1935 p.40)

```c
CAEN_DGTZ_SetRecordLength(handle, size);
```

- x743: `size mod 16 == 0` かつ `size > 4*16` (最小 64 samples)
- **DPP-CI では channel パラメータ非対応**

### SetSAMPostTriggerSize (UM1935 p.54)

```c
CAEN_DGTZ_SetSAMPostTriggerSize(handle, SamIndex, value);
```

- `SamIndex`: SAMLONG チップインデックス (0-7)
- `value`: 1-255。単位はサンプリング周期 x16

---

## 11. SAM (SAMLONG) 固有パラメータ

### SetSAMCorrectionLevel (UM1935 p.53)

```c
typedef enum {
    CAEN_DGTZ_SAM_CORRECTION_DISABLED      = 0,
    CAEN_DGTZ_SAM_CORRECTION_PEDESTAL_ONLY = 1,
    CAEN_DGTZ_SAM_CORRECTION_INL           = 2,
    CAEN_DGTZ_SAM_CORRECTION_ALL           = 3,
} CAEN_DGTZ_SAM_CORRECTION_LEVEL_t;
```

補正種別 (UM2750 Sec. 10.9):
- **Line Offset**: ~0.95 mV RMS、工場固定、変更不可
- **Individual Pedestal**: ~0.75 mV RMS、ユーザー修正可
- **Time INL**: ~5 ps RMS、ON/OFF のみ
- **Trigger Threshold DAC Offset**: 工場固定、変更不可

### SetSAMSamplingFrequency (UM1935 p.54)

```c
typedef enum {
    CAEN_DGTZ_SAM_3_2GHz  = 0,  // 3.2 GS/s (デフォルト)
    CAEN_DGTZ_SAM_1_6GHz  = 1,
    CAEN_DGTZ_SAM_800MHz  = 2,
    CAEN_DGTZ_SAM_400MHz  = 3,
} CAEN_DGTZ_SAMFrequency_t;
```

### Enable/DisableSAMPulseGen (UM1935 p.56)

```c
CAEN_DGTZ_EnableSAMPulseGen(handle, channel, pulsePattern, pulseSource);
CAEN_DGTZ_DisableSAMPulseGen(handle, channel);

typedef enum {
    CAEN_DGTZ_SAMPulseSoftware = 0, // SendSAMPulse() で手動送信
    CAEN_DGTZ_SAMPulseCont     = 1, // FPGA 内部発振器から連続送信
} CAEN_DGTZ_SAMPulseSourceType_t;
```

- `pulsePattern`: 16-bit パターン。200MHz クロックの各ビットが 1 クロック幅のパルスに対応
- パルス振幅: ~0.7 V（ケーブル接続時は半分）
- `pulseSource` はチャンネルペア (SAMLONG チップ) 共通

---

## 12. トリガー管理 — 3 層構造 (UM2750 Sec. 10.12)

V1743 のトリガーは 3 層で構成される:

```
チャンネルディスクリミネータ (per channel)
    ↓
ペアトリガーロジック (per 2-channel group: AND/OR + coincidence window)
    ↓ TRG_REQ[0..7]
ボードトリガーロジック (OR / AND / Majority)
    ↓
COMMON TRIGGER → 全チャンネル同時取得
```

### 12.1 チャンネルレベル

#### SetChannelTriggerThreshold (UM1935 p.33)

```c
CAEN_DGTZ_SetChannelTriggerThreshold(handle, channel, Tvalue);
```

**x743 の DAC 変換 (反転):**
```
0x0000 → +1.25V
0x7FFF → 0V
0xFFFF → -1.25V
```

**WaveDemo の変換式 (電圧 → DAC):**
```c
// DC Offset
reg_val = (int)((1.25 + dc_offset_V) / 2.50 * 65535);

// Trigger Threshold（DC Offset に結合）
reg_val = (int)((1.25 - (threshold_V + dc_offset_V)) / 2.50 * 65535);
```

#### SetChannelDCOffset (UM1935 p.42)

```c
CAEN_DGTZ_SetChannelDCOffset(handle, channel, Tvalue);  // 0x0000-0xFFFF
```

- デフォルト: `0x7FFF` (中央 = -Vpp/2 → 入力範囲 -Vpp/2 ~ +Vpp/2)
- `0x0000`: DC offset なし → 入力範囲 -Vpp ~ 0
- `0xFFFF`: DC offset = -Vpp → 入力範囲 0 ~ +Vpp

#### SetTriggerPolarity (UM1935 p.35)

```c
CAEN_DGTZ_SetTriggerPolarity(handle, channel, polarity);

typedef enum {
    CAEN_DGTZ_TriggerOnRisingEdge  = 0,
    CAEN_DGTZ_TriggerOnFallingEdge = 1,
} CAEN_DGTZ_TriggerPolarity_t;
```

**注意:** DPP FW では使用不可。DPP-CI では `SetChannelPulsePolarity()` を使用。

#### SetChannelPulsePolarity (UM1935 p.62, DPP 専用)

```c
CAEN_DGTZ_SetChannelPulsePolarity(handle, channel, pol);

typedef enum {
    CAEN_DGTZ_PulsePolarityPositive = 0,
    CAEN_DGTZ_PulsePolarityNegative = 1,
} CAEN_DGTZ_PulsePolarity_t;
```

#### SetChannelSelfTrigger (UM1935 p.31)

```c
CAEN_DGTZ_SetChannelSelfTrigger(handle, mode, channelmask);

typedef enum {
    CAEN_DGTZ_TRGMODE_DISABLED       = 0,
    CAEN_DGTZ_TRGMODE_ACQ_ONLY       = 1, // トリガーのみ (TRG-OUT には出さない)
    CAEN_DGTZ_TRGMODE_EXTOUT_ONLY    = 2, // TRG-OUT のみ
    CAEN_DGTZ_TRGMODE_ACQ_AND_EXTOUT = 3, // 両方
} CAEN_DGTZ_TriggerMode_t;
```

**x743 注意:** 偶数/奇数チャンネルはペア。ペアごとに 1 回だけ呼ぶこと（2 回呼ぶと後の呼び出しで上書きされる）。

### 12.2 ペアトリガーロジック

#### SetChannelPairTriggerLogic (UM1935 p.58)

```c
CAEN_DGTZ_SetChannelPairTriggerLogic(handle, channelA, channelB, logic, coincidenceWindow);

typedef enum {
    CAEN_DGTZ_LOGIC_OR  = 0, // ペア内の OR
    CAEN_DGTZ_LOGIC_AND = 1, // ペア内の AND
} CAEN_DGTZ_TrigerLogic_t;
```

| 引数 | 説明 |
|------|------|
| `channelA/B` | ペアのチャンネル番号 (CH0-CH1, CH2-CH3, ...) |
| `logic` | OR (=0) または AND (=1) |
| `coincidenceWindow` | コインシデンスゲート長 (ns)。≥15 ns、5 の倍数推奨。最大 5*255=1275 ns |

### 12.3 ボードトリガーロジック

#### SetTriggerLogic (UM1935 p.59)

```c
CAEN_DGTZ_SetTriggerLogic(handle, logic, majorityLevel);

typedef enum {
    CAEN_DGTZ_LOGIC_OR       = 0, // いずれかのペアがトリガー
    CAEN_DGTZ_LOGIC_AND      = 1, // 全ペアが同時にトリガー
    CAEN_DGTZ_LOGIC_MAJORITY = 2, // Majority（過半数）
} CAEN_DGTZ_TrigerLogic_t;
```

| 引数 | 説明 |
|------|------|
| `majorityLevel` | 0 ~ (ペア数-1)。0 = 1 ペア以上でトリガー |

### 12.4 外部トリガー・ソフトウェアトリガー

#### SetSWTriggerMode (UM1935 p.30)

```c
CAEN_DGTZ_SetSWTriggerMode(handle, mode);  // DISABLED / ACQ_ONLY / EXTOUT_ONLY / ACQ_AND_EXTOUT
```

#### SetExtTriggerInputMode (UM1935 p.30)

```c
CAEN_DGTZ_SetExtTriggerInputMode(handle, mode);  // 同上
```

#### SendSWtrigger (UM1935 p.29)

```c
CAEN_DGTZ_SendSWtrigger(handle);
```

---

## 13. SAM Trigger Counter Veto (UM1935 p.60)

チャンネルごとにトリガーカウンタの veto を設定。

```c
CAEN_DGTZ_SetSAMTriggerCountVetoParam(handle, channel, enable, vetoWindow);

typedef enum {
    CAEN_DGTZ_ENABLE  = 1,
    CAEN_DGTZ_DISABLE = 0,
} CAEN_DGTZ_EnaDis_t;
```

| 引数 | 説明 |
|------|------|
| `enable` | Veto 有効/無効 |
| `vetoWindow` | Veto 時間窓 (ns) |

---

## 14. DPP-CI Virtual Probes (UM1935 p.76)

波形モード (Mixed/Oscilloscope) 時のプローブ設定。

```c
CAEN_DGTZ_SetDPP_CI_VirtualProbe(handle, mode, vp, dp1, dp2);
```

**Analog Probe:**
- `CAEN_DGTZ_DPP_CI_VIRTUALPROBE_Baseline = 0` (ベースラインのみ)

**Digital Probe 1 (FW ≤ 130.20):**
- `B1OutSafeBand = 0`, `B1Timeout = 1`, `CoincidenceMet = 2`, `Tvaw = 3`

**Digital Probe 1 (FW ≥ 130.22):**
- `ExtTrg = 4`, `OverThr = 5`, `TrigOut = 6`, `CoincWin = 7`, `Coincidence = 9`

---

## 15. 取得制御

### SetAcquisitionMode (UM1935 p.41)

```c
typedef enum {
    CAEN_DGTZ_SW_CONTROLLED        = 0, // ソフトウェア Start/Stop
    CAEN_DGTZ_S_IN_CONTROLLED      = 1, // S-IN 信号で Start (High=Run)
    CAEN_DGTZ_FIRST_TRG_CONTROLLED = 2, // TRG-IN の最初のパルスで Start
} CAEN_DGTZ_AcqMode_t;
```

### SetIOLevel (UM1935 p.35)

```c
typedef enum {
    CAEN_DGTZ_IOLevel_NIM = 0,
    CAEN_DGTZ_IOLevel_TTL = 1,
} CAEN_DGTZ_IOLevel_t;
```

### SetRunSynchronizationMode (UM1935 p.34)

マルチボード同期用。

```c
typedef enum {
    CAEN_DGTZ_RUN_SYNC_Disabled,
    CAEN_DGTZ_RUN_SYNC_TrgOutTrgInDaisyChain,
    CAEN_DGTZ_RUN_SYNC_TrgOutSinDaisyChain,
    CAEN_DGTZ_RUN_SYNC_SinFanout,
    CAEN_DGTZ_RUN_SYNC_GpioGpioDaisyChain,
} CAEN_DGTZ_RunSyncMode_t;
```

---

## 16. DPP-CI プログラミングシーケンス

マニュアルと DPP Example Codes (UM1935 p.78) に基づく推奨シーケンス:

```
 1. OpenDigitizer()
 2. Reset()
 3. SetSAMAcquisitionMode(DPP_CI)          ← モード切替
 4. SetDPPParameters(channelMask, &params)  ← CI パラメータ一括設定
 5. SetChannelPulsePolarity(ch, pol)        ← DPP 用 polarity
 6. SetChannelDCOffset(ch, dac)             ← DC オフセット
 7. SetGroupEnableMask(mask)                ← グループ有効化
 8. SetRecordLength(size)                   ← レコード長 (Oscilloscope/Mixed 時のみ)
 9. SetDPPPreTriggerSize(-1, samples)       ← プレトリガー
10. SetDPPAcquisitionMode(List, EnergyAndTime) ← List モード
11. SetNumEventsPerAggregate(numEvents)     ← イベント集約
12. SetDPPEventAggregation(0, 0)            ← 自動集約
13. SetSAMSamplingFrequency(freq)           ← サンプリング周波数
14. SetSAMCorrectionLevel(ALL)              ← SAM 補正
15. SetSAMPostTriggerSize(group, value)     ← ポストトリガー
16. SetChannelPairTriggerLogic(chA, chB, logic, window) ← ペアロジック
17. SetTriggerLogic(logic, majorityLevel)   ← ボードロジック
18. SetSWTriggerMode / SetExtTriggerInputMode ← トリガーソース
19. SetIOLevel(NIM/TTL)
20. SetAcquisitionMode(SW_CONTROLLED)
21. Enable/DisableSAMPulseGen()             ← テストパルス (オプション)
22. MallocDPPEvents()                       ← イベントバッファ確保
23. MallocReadoutBuffer()                   ← リードアウトバッファ確保

-- 取得ループ --
24. ClearData()
25. SWStartAcquisition()
26. ReadData() → GetDPPEvents() → 処理
27. SWStopAcquisition()

-- クリーンアップ --
28. FreeDPPEvents()
29. FreeReadoutBuffer()
30. CloseDigitizer()
```

---

## 17. Standard モード vs Charge モード (V1743)

V1743 では両モードとも同一 FW 上で動作し、リードアウトパスも共通。

| 機能 | Standard モード | Charge モード |
|------|---------------|-------------|
| モード設定 | `SetSAMAcquisitionMode(STANDARD)` | `SetSAMAcquisitionMode(DPP_CI)` |
| パラメータ | 個別 API (Threshold, DC Offset 等) | 同左 + レジスタ直書き (ゲート幅等) |
| トリガー極性 | `SetTriggerPolarity()` | 同左 (`SetChannelPulsePolarity()` も動作) |
| バッファ確保 | `MallocReadoutBuffer()` + `AllocateEvent()` | 同左 |
| イベント取得 | `GetNumEvents()` + `GetEventInfo()` + `DecodeEvent()` | 同左 |
| イベント型 | `CAEN_DGTZ_X743_EVENT_t` | 同左 |
| 出力データ | 波形 (float[] per channel) + TDC + metadata | Charge + Baseline + Peak + TDC (波形なし) |
| 最大レート | ~7 kHz (デッドタイム制限) | ~7 kHz (同じ HW 制限) |
| 集約制御 | `SetMaxNumEventsBLT()` | 同左 |

**注意:** V1720 系の DPP-CI 用 API (`SetDPPParameters`, `GetDPPEvents`, `DPP_CI_Event_t` 等) は V1743 では使用不可。

---

## 18. delila-rs 実装への影響

### apply_config() で必要な API 呼び出し

| API | 重要度 | 用途 |
|-----|--------|------|
| `SetSAMAcquisitionMode(DPP_CI)` | **必須** | Charge Mode 切替 |
| `SetChannelTriggerThreshold()` | **必須** | トリガー閾値 |
| `SetTriggerPolarity()` | **必須** | トリガー極性 |
| `SetChannelDCOffset()` | **必須** | DC オフセット |
| `SetChannelSelfTrigger()` | **必須** | セルフトリガー |
| `SetChannelPairTriggerLogic()` | 推奨 | ペアトリガーロジック |
| `SetTriggerLogic()` | 推奨 | ボードトリガーロジック |
| `SetDPPPreTriggerSize()` | 推奨 | プレトリガー (レジスタ 0x1038 に反映確認済み) |
| Charge ゲート幅等 | 推奨 | レジスタ直書き (API なし) |

### read_loop_x743 のリードアウトパス

Charge Mode でも Standard と同じリードアウトパスを使用:
`ReadData() → GetNumEvents() → GetEventInfo() → DecodeEvent() → X743_EVENT_t`

`X743_EVENT_t.DataGroup[g]` の `.Charge` / `.Baseline` / `.Peak` / `.TDC` を使用。
