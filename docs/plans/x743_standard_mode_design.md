# V1743 Standard Mode 統合設計 (DPP-CI 撤退)

**作成日:** 2026-04-20
**更新日:** 2026-04-20 — User Manual UM2750 Rev.5 精読結果を反映（決定的エビデンス確保）
**ステータス:** 計画（実装開始前）
**置換対象:** `docs/plans/x743_integration.md` の DPP-CI 部分、`docs/x743_dpp_ci_parameters.md`
**関連:** `TODO/47_v1743_standard_mode_redesign.md`
**権威ソース:** [UM2750 V1743 User Manual Rev.5](../../legacy/UM2750_V1743_User_Manual_rev5.pdf) (May 2025, FW 4.29_2.24)
**レビュー:** CAEN Application Note "Setting CHARGE MODE for the x743 modules" / Gemini 2.5 Pro 協議 (2026-04-20)
**物理要件:** **時間情報から 2D 平面上での粒子入射位置を計算する** → 高時間分解能が最優先。Charge は副次的

---

## 1. 背景 — なぜ DPP-CI を撤退するか

### 1.1 決定的根拠: User Manual UM2750 Rev.5 Fig 10.9

V1743 の **Charge Mode のワイヤフォーマット (Fig 10.9) は TDC を含まない**。
マニュアル Sec 10.10.2 "Running in Charge Mode" + Fig 10.9 "Group Data Format in Charge Mode" より：

```
[ 2 bits=00 | REF_CELL_COLUMN (10 bits) | 1 | CHARGE (23 bits, 2's complement, pC) ]
  × 256 events per FIFO (per channel)
```

チャネルあたり専用の 256-event Charge FIFO が FPGA 内で満タンになるまで PC に転送されない。
**データ要素は charge と SAMLONG 内の参照セル列のみ** — TDC も波形も HIT/TIME カウンタも存在しない。

対して Standard Mode の Fig 10.8 "Group Data Format" は per-group **`TDC` (40-bit @ 5 ns SAMLONG clock)**
を持ち、マニュアル Sec 10.10.3.1 は太字で：

> **IMPORTANT NOTE**: EVENT TIME TAG (in Header) corresponds to the time when the event is created in the digitizer memory
> (i.e. related to readout), **NOT to the time the event occurred at the group level, so it does not correspond to any physical quantity.
> The physical time of arrival of the pulse is the 40-bit counter of TDC field in the data format** (see Fig. 10.8).

と明示。**物理時刻は Standard Mode の per-group TDC 以外からは得られない**。

### 1.2 物理要件との衝突

本プロジェクトの V1743 用途は **時間情報から 2D 平面上の粒子入射位置を計算する位置感応型検出器**。
時間分解能（サブ ns〜数 ns）が主要要件で、電荷は副次的。

- Charge Mode に TDC が無い → 時間計算できない → **物理的に使えない**
- Standard Mode の TDC 40-bit @ 5ns + 波形内 `PosEdgeTimeStamp`/`NegEdgeTimeStamp` のサブサンプル補間で ~20 ps RMS
  （マニュアル Sec 10.9 Time INL Correction 付き）を達成可能

### 1.3 現行実装のバグ

`src/reader/caen_legacy/handle.rs::apply_config_dpp_ci()` は **x720 系 `CAEN_DGTZ_DPP_CI_Params_t`** を使って
`CAEN_DGTZ_SetDPPParameters` を呼んでいる。CAEN のアプリケーションノート (docs/Setting_CHARGE_MODE_for_the_x743_modules.pdf)
が指定する x743 用は別の構造体 **`CAEN_DGTZ_DPP_X743_Params_t`**（`disableSuppressBaseline`, `startCell[16]`,
`chargeLength[16]`, `enableChargeThreshold[16]`, `chargeThreshold[16]`(pC)）。

同様に `GetDPPEvents` 後のイベント構造体も、x720 用 `CAEN_DGTZ_DPP_CI_Event_t` (Format/TimeTag/Charge/Baseline/Waveforms)
ではなく x743 用 **`CAEN_DGTZ_DPP_X743_Event_t`** (`float Charge; int StartIndexCell;` の 8 byte) を使うべき。
両者はサイズもストライドもフィールドもすべて異なるので、現行コードは：

- Params 側: x743 FW に存在しないレジスタに書き込み（無意味）
- Event 側: メモリストライドが合わずイベントを取り違え、`TimeTag` / `Baseline` は存在しないバイトを拾っている

ただしバグを直しても §1.1 で撤退決定されるため、この実装を完成させる意味はない。

### 1.4 既存 probe テスト記述との矛盾

`docs/x743_dpp_ci_parameters.md` (2026-04-15) は「`SetSAMAcquisitionMode(DPP_CI)` 後も `DecodeEvent → X743_EVENT_t`
で Charge/Baseline/Peak/TDC が取れる」と主張しているが、User Manual Fig 10.9 のワイヤフォーマットとは**矛盾**する。
ワイヤに TDC ビットが存在しない以上、仮に `X743_EVENT_t.TDC` に値が入っていても：

- 未初期化メモリ or 前 Run の残骸
- 現 FW の undocumented な副作用（保証されない）

のいずれかで信頼できない。CAEN は Charge Mode での TDC 取得を公式に保証していないため、
実測値が偶然正しく見えても運用には採用できない。

### 1.5 Standard Mode の優位性

CAEN WaveDemo ソース (`legacy/caenwavedemo_x743-1.2.1/`) は **`SetSAMAcquisitionMode` を呼ばず Standard mode を使用**。
`DataGroup[g].TDC * 5` で ns 単位の絶対時刻を計算し、クロスボードでは `TDC - TDC_min` で時差測定している。
CAEN の公式リファレンス実装が Standard 一本。

`CAEN_DGTZ_X743_GROUP_t` は Standard mode で以下を提供する（FW 4.29_2.24 / Rev.5）：

```c
typedef struct {
    uint32_t  ChSize;
    float    *DataChannel[2];           // 1024 sample 波形 (float)
    uint16_t  TriggerCount[2];          // 前回 event からの discriminator hit 数
    uint16_t  TimeCount[2];             // 1 μs 単位 inter-event Δt (16-bit ≒ 65 ms)
    uint8_t   EventId;
    uint16_t  StartIndexCell;
    uint64_t  TDC;                      // ★ 40-bit @ 5 ns の物理絶対時刻
    float     PosEdgeTimeStamp;         // ★ CFD-ish サブサンプル rising  edge
    float     NegEdgeTimeStamp;         // ★ CFD-ish サブサンプル falling edge
    uint16_t  PeakIndex;
    float     Peak;                     // ★ ピーク値
    float     Baseline;                 // ★ ベースライン
    float     Charge;                   // ★ 電荷積分値（CAEN lib 計算済み）
} CAEN_DGTZ_X743_GROUP_t;
```

ポイント：

- **TDC は 40-bit @ 5 ns = 約 91 分レンジ**。1 run = 数時間の運用なら 16-bit software extension で十分（BTT ロジックは必要だが PSD1 ほど複雑ではない）
- **Charge/Baseline/Peak は CAEN lib 側が既に波形から算出**。Rust 側で波形積分を再実装する必要はない
- `PosEdgeTimeStamp` / `NegEdgeTimeStamp` は float 型のサブサンプル精度 edge 時刻
- 波形データも同時に取れる → 検算・波形保存・オフライン再解析可能

WaveDemo では `DataGroup[g].TDC * 5` を ns 単位絶対時刻として採用し、クロスボードでは
`TDC - TDC_min` で時差測定している。これが CAEN 公式の想定運用。

### 1.6 バッファとレートの比較（マニュアル Sec 10.3, 10.10.2, 10.11 より）

|  | Standard Mode | Charge Mode |
|---|---|---|
| SAMLONG 1024-cell 取得 | 毎イベント必要 | 毎イベント必要（FPGA 積分のため） |
| デジタル SRAM 消費 | **7 events/ch × 1024 samples × 12-bit** | 波形は破棄、**Charge FIFO 256 events/ch × ~23-bit** のみ |
| 律速要因 | PC readout 帯域（16 ch × 1024 sample 転送） | SAMLONG 変換デッドタイム（~125 μs @1024 samples） |
| 最大 rate | 帯域次第（実質 kHz 台） | **~7 kHz "for full events"**（マニュアル Sec 10.10.2）|
| TDC | ✅ per-group 40-bit @ 5ns | ❌ **存在しない** |
| 波形 | ✅ | ❌ |

バッファ観点だけ見れば Charge Mode が軽い。が、TDC が無い時点で本プロジェクトでは選択肢にならない。

### 1.7 Standard Mode のレート対策

`SetRecordLength()` でイベント当たりサンプル数を 1024 から削減可能（マニュアル Sec 10.3）：

> "It is possible to configure the board to read less than 1024 samples per event, so extending the number of
> events consecutively storable in the Digital Memory."

波形長が短くなる分 PC 帯域も軽減され、buffer depth（7 events/ch の上限）の物理的容量が増える。
物理要件が許す最小 record_length を採用して rate 上限を引き上げる。

---

## 2. 確定前提

DELILA 側の設計確定事項：

| 項目 | 値 |
|---|---|
| 取得モード | **`CAEN_DGTZ_AcquisitionMode_STANDARD`** 一本化 |
| トリガ | 各チャンネル **self-trigger**（leading edge discriminator, 閾値 + 極性） |
| クロック | デジタイザ間 **デイジーチェーン伝搬**（位相ズレ無視可能） |
| Arm sync | `CAEN_DGTZ_RUN_SYNC_SinFanout` + `S_IN_CONTROLLED` acquisition mode |
| 同期 Master | **既存 PSD Master の S_OUT を V1743 群の S_IN にも分配**（システム単一マスター） |
| 台数 | 当面 1 台 → 将来 3 台（最大） |
| Sync Pulse S1 | 全 V1743 モジュールの ch0 or ch1 に配線済み。クロスボード較正用途 |
| Energy 算出 | CAEN lib の `Charge` (float) を採用（Standard mode でも FW が計算、Rust 側積分は不要） |
| Waveform | 取得するが onboard 保存は selective（`save_waveform` per channel） |
| **EVENT_MODE bit** | **header word 2 bit[24] を 1 に設定必須**（マニュアル Sec 10.10.3.1）— 自前 SW では default=0 なので手動で立てる |
| Record length | 物理要件最小（例 128-256）に設定し SRAM buffer depth 拡張 |

---

## 3. タイムスタンプ設計

### 3.0 時間分解能の二階層構造（★ V1743 採用の最大理由）

V1743 の売りは **5 ps RMS 時間分解能**（マニュアル Sec 10.9 Time INL Correction）。
これを得るための時刻は coarse + fine の 2 要素合成：

```
total_time_ns = TDC * 5                    // coarse: 40-bit SAMLONG clock tick
              + fine_time_ns               // fine:  波形エッジのサブサンプル補間
```

| 要素 | 分解能 | ソース | 必須補正 |
|---|---|---|---|
| Coarse (TDC) | 5 ns LSB、~5 ns RMS | `DataGroup[g].TDC * 5` | Line Offset（factory） |
| Fine | **5 ps RMS** @3.2 GSa/s | `DataGroup[g].PosEdgeTimeStamp` (CAEN lib 計算) | **Time INL Correction 必須**（factory） |

**5 ps RMS を得る条件**（マニュアル Sec 10.9）:
1. `SetSAMCorrectionLevel(CAEN_DGTZ_SAM_CORRECTION_ALL)` ← factory 補正を全有効化
   - Line Offset Correction → baseline noise ~0.95 mV RMS, **sampling time ~20 ps RMS**
   - Individual Pedestal Correction → baseline noise ~0.75 mV RMS（user 側で thermal 後に再較正推奨）
   - **Time INL Correction → sampling time ~5 ps RMS**（セル間時間分散補正）
   - Trigger Threshold DAC Offset Correction → small signal の閾値精度
2. Sampling frequency 3.2 GS/s（LSB 0.3125 ns）
3. Record length を rising edge が含まれる長さ以上に（エッジ補間のため）

現コード (`src/reader/caen_legacy/handle.rs:1054`) は `correction_level: "all"` をパース可能。
**config のデフォルト値を `"all"` に固定**し、`"disabled"` 設定時は warn を出す実装とする。

### 3.0.1 Fine time のデコード方式

CAEN lib は `DataGroup[g].PosEdgeTimeStamp` / `NegEdgeTimeStamp` (float, ns) を自動計算する。
WaveDemo ソース（`WDWaveformProcess.c::SW_WaveformProcessor`）は独自に CFD / LED 補間を
再実装しているが、**lib が提供する edge timestamp で 5 ps RMS に到達可能**。

実装戦略：

| Phase | 方式 | 用途 |
|---|---|---|
| 初期 | `PosEdgeTimeStamp` (or `NegEdgeTimeStamp` for negative pulses) をそのまま採用 | シンプル、実装コスト最小 |
| 必要時 | Rust 側で CFD 補間（WaveDemo 参考、per-channel CFD delay/fraction config） | amplitude walk 対策が必要な場合のみ |

EventData の `fine_time: u16` フィールドに PosEdgeTimeStamp を 7.5 ns / 2^16 = ~114 fs LSB で詰めれば
TDC 1 tick (5 ns) に対して十分な精度を保持できる。

### 3.0.2 Trigger Jitter 対策

セルフトリガは SAMLONG クロック境界で latch されるので、トリガ位置自体に ±数サンプルのジッタ。
マニュアル Fig 10.9 直下の注記：
> "waveforms in memory allows for a potential ΔT between the instant when the trigger physically arrives
> and when it is sensed by the digitizer"

**対策**：TDC は「トリガが見えた時刻」、波形内 PosEdgeTimeStamp は「実信号エッジ位置」なので、
`fine_time = PosEdgeTimeStamp - trigger_position_in_waveform` とすれば jitter 吸収可能
（WaveDemo の TrgShift 相当）。実装優先度は低（初期は PosEdgeTimeStamp 直用で十分）。

### 3.1 主時刻ソース: `DataGroup[g].TDC`

- LSB 5 ns、raw は 40-bit（`uint64_t` 型だが FW から 40-bit）
- **Per-group**（group 内 2 ch は同一 TDC = 同一トリガ）
- `ns = TDC * 5`

### 3.2 Software 上位拡張（64-bit extension）

40-bit wrap は 91 分で起きる。通常 1 run < 60 分だが安全マージンとして：

```
per-board state:
  last_tdc40: u64      // 前イベントの 40-bit 値
  rollover_count: u64  // 40-bit wrap の回数

on event:
  tdc40 = event.TDC & 0xFFFFFFFFFF
  if tdc40 < last_tdc40 && (last_tdc40 - tdc40) > (1 << 39):
      rollover_count += 1       // wrap 検出
  timestamp_ns = ((rollover_count << 40) | tdc40) * 5
  last_tdc40 = tdc40
```

**per-board で十分**（TDC は board 内全 group 共通クロック由来）。
PSD1 のような per-channel BTT は不要。

**rollover 検出の頑健性**：40-bit レンジのうち 1 ms でもイベント間隔があれば tdc40 差は 2^18 tick 以内。
wrap 判定の閾値を「差が 2^39 以上」にすれば、誤検出は起きない。

### 3.3 イベント到着順の非保証

`CAEN_DGTZ_ReadData` + `CAEN_DGTZ_GetEventInfo` で取れるイベント列は**時系列順とは限らない**
（複数 group を一括転送するため）。Reader は decoded EventData を **channel ごとに時刻順ソート**
する必要がある。既存 Merger / Online EB が既にこの前提で動くので変更不要。

### 3.3.1 Fine time とクロスボード較正の組み合わせ

S1 較正（§3.4）を fine time まで使って行うと、定数オフセットを **ps オーダー**で較正できる。
各 V1743 で S1 を取得 → total_time_ns (= TDC*5 + PosEdge) をペアリング → ヒストグラム中心値を board 間オフセットとして保存。

### 3.4 クロスボード較正

3 台構成時、各ボードの TDC は arm 時刻（`S_IN` 立ち上がり）から 0 start するはずだが、
実際には arm のスキュー + ケーブル遅延で **定数オフセット**が残る。

**S1 を利用した較正**：

1. S1 パルスは全ボードの ch0/1 に配線済み → 全ボードで同一物理事象を記録
2. 各ボードから S1 チャンネルの TDC 列を収集
3. ボード i と ボード 0 の S1 TDC を時系列でペアリング（最近傍マッチング）
4. ΔTDC = TDC_i - TDC_0 の中央値を **定数オフセット**として較正
5. ΔTDC の時間発展（slope）が ≠ 0 なら**クロック失同期の自己診断** → warning

この較正値は run ごとに再計算してよい（config にハードコードしない）。オフライン EB で適用。

### 3.5 BTT heartbeat 不要

**重要**：今回は S1 heartbeat 不要。40-bit TDC は十分広く、`TimeCount` (1μs, 16-bit) BTT 方式ではない
ため、S1 停止 → heartbeat 停止 → BTT 失敗の連鎖は起きない。S1 はあくまで **クロスボード較正用**。

---

## 4. Hardware トリガ/同期設計

### 4.1 Self-trigger 設定

V1743 Standard mode の self-trigger は **leading edge discriminator**（CFD ではない）。
API：

| 項目 | API | 値域 |
|---|---|---|
| 閾値 | `CAEN_DGTZ_SetChannelTriggerThreshold(h, ch, thr_adc)` | 12-bit ADC count (0-4095) |
| 極性 | `CAEN_DGTZ_SetTriggerPolarity(h, ch, pol)` | `TriggerOnRisingEdge` / `TriggerOnFallingEdge` |
| 有効化 | group-level `CAEN_DGTZ_SetGroupSelfTrigger(h, group, mask)` | per-group 2-bit mask |
| Self-trigger logic | `CAEN_DGTZ_SetTriggerLogic` | board-wide OR (default) |

Fine time 精度は `PosEdgeTimeStamp` / `NegEdgeTimeStamp` が float 補間を提供する。
必要に応じて EventData の `fine_time` に乗せる。

### 4.2 Arm 同期 (SinFanout)

既存 PSD 同期チェーンに接続：

```
[ PSD Master digitizer ]
     │ S_OUT
     ├──► PSD slave  S_IN
     ├──► PSD slave  S_IN
     ├──► V1743 #0   S_IN   (new)
     ├──► V1743 #1   S_IN   (new)
     └──► V1743 #2   S_IN   (new)
```

Slave 側の Rust コード設定：
```rust
h.set_run_synchronization_mode(SinFanout)?;   // 同期モード
h.set_acquisition_mode(SInControlled)?;       // S_IN trigger で start
```

### 4.3 S_IN レベルトリガの罠（既知問題）

PSD1 で経験した「S_IN レベルトリガ → arm 直後に HIGH 検出 → ボード間 650ms オフセット」問題
（MEMORY.md "DIG1 S_IN タイムコリレーション修正 2026-03-02"）は V1743 でも再発リスクあり。
Operator 側 `arm_all_sync()` が **全ボード（PSD + V1743）** が hw Armed になってから Master の
S_OUT を立ち上げる順序を保証する必要がある。既存実装で既にこのロジックはあるので、
V1743 の `hw_state` が hw Armed を正しく報告するよう実装する。

---

## 5. 電荷算出

### 5.1 CAEN lib の `Charge` を採用

波形積分は CAEN lib 側で実行済み。`DataGroup[g].Charge` を float として取り出し、
Rust の EventData に格納する際に固定小数点化：

```rust
let energy_u16 = (group.Charge * scale + offset).clamp(0.0, 65535.0) as u16;
```

`scale` / `offset` は config でチャンネルごとに指定可能（calibration）。

### 5.2 波形積分パラメータの制御性

CAEN lib の `Charge` 積分 gate / baseline window は Standard mode の場合、
**FW デフォルト設定**で動く（DPP_CI_Params_t 経由で変える仕組みは x743 には無い）。

実機で `Charge` の値域・ゲート幅を確認：

- パルサを食わせて `Charge` の値と波形を比較
- ゲート幅が適切でないなら、**Rust 側で波形再積分するオプション**を持つ（`energy_source: "lib" | "soft"`）

### 5.3 波形保存

- `save_waveform: true` の channel のみ EventData に波形を含める
- 他は CAEN lib から取れても Rust 側で捨てる（1024 float = 4 KB/event は重い）

---

## 6. Config スキーマ（新）

`X743Config` のフィールドを刷新：

```toml
[x743]
connection_type = "optical_link"    # optical_link / usb / a4818
link_num = 0
conet_node = 0
vme_base_address = 0

sampling_frequency = "3.2GHz"       # 3.2GHz / 1.6GHz / 0.8GHz / 0.4GHz（時間分解能最高は 3.2GHz）
correction_level = "all"            # ★ "all" 強制推奨（5 ps RMS を得るには必須）
                                    # disabled / pedestal / inl / all
record_length = 1024                # edge 補間が要るので rising edge が含まれる長さ必須
post_trigger_size = 50              # %
max_num_events_blt = 100

group_enable_mask = 0xFF            # bit per group (8 groups)
io_level = "nim"                    # nim / ttl

# Sync (新)
run_sync_mode = "sin_fanout"        # disabled / sin_fanout / trgout_sin_daisy
clock_source = "daisy_in"           # internal / daisy_in / external
trigger_source = "self"             # software / external / self

# self-trigger（既存 channel_defaults / channel_overrides を利用）
# 各ch: trigger_threshold (ADC count), pulse_polarity (positive/negative), self_trigger

# Charge / Energy 算出
energy_source = "lib"               # lib (CAEN計算) / soft (Rust再計算)
energy_scale = 1.0
energy_offset = 0.0

# Fine time（時間分解能の要）
fine_time_source = "pos_edge"       # pos_edge / neg_edge / cfd_soft
                                    # negative pulse 検出器なら "neg_edge"
                                    # amplitude walk 抑制が必要なら "cfd_soft"
# CFD parameters (only used if fine_time_source = "cfd_soft")
cfd_delay_ns = 2.0
cfd_fraction = 0.5

# Waveform 保存
save_waveform = false               # per-channel override 可能

# ADC calibration (既存)
adc_calibration_at_configure = true # configure 終端で SAM_ADC_Calibration 実行
```

**削除するフィールド**：`dpp_ci_threshold`, `dpp_ci_gate`, `dpp_ci_pgate`, `dpp_ci_csens`,
`dpp_ci_nsbl`, `dpp_ci_trgho`, `dpp_ci_tvaw`, `dpp_ci_pre_trigger`（全 DPP_CI 関連）。

---

## 7. 実装ステップ

| Step | 内容 | 成功条件 |
|---|---|---|
| **1** | `apply_config_dpp_ci` と DPP_CI_Params_t/Event_t 依存コードを全削除。`apply_config_standard` のみ残す。config schema から dpp_ci_* を削除 | `cargo build --features x743` 成功、`cargo clippy -- -D warnings` OK |
| **2** | Reader V1743 ReadLoop を Standard mode 一本化：`decode_event` → `X743_EVENT_t` → `DataGroup[g].{TDC, Charge, Baseline, ...}` から EventData を生成 | 1 台で外部パルサ相手にエネルギースペクトル + タイムスタンプが線形に進む |
| **3** | TDC の 40→64-bit ソフト拡張（per-board rollover）を実装。長時間 run で timestamp 単調増加を確認 | 2h run でタイムスタンプ単調、wrap なし |
| **4** | self-trigger 閾値 + 極性の per-channel config、`SinFanout` + `S_IN_CONTROLLED` 同期を実装。`hw_state` が hw Armed を正しく報告 | 1 台単体 Arm→Start で即実行、arm_all_sync と協調動作 |
| **5** | 2-3 台構成に拡張、S1 チャンネルの `(TDC*5 + PosEdgeTimeStamp)` ΔT ヒストグラムで board 間オフセットを ps 精度較正 | S1 ΔT 中心固定、FWHM < 100 ps |
| **6** | `Charge` 値のキャリブレーション（パルサ較正）、`energy_source: "soft"` オプション（必要なら）実装 | エネルギースペクトル形状が物理的に妥当 |
| **7** | 時間分解能測定（2 ch 間の S1 ΔT FWHM）、必要なら `fine_time_source: "cfd_soft"` へ切替 | S1 ΔT FWHM ~ √2 × 5 ps ≈ 7 ps RMS を確認 |

---

## 8. 要実機検証項目

以下は仕様書からは確定できず、VX1743 SN:25 実機で確認する：

| 項目 | 手段 |
|---|---|
| **TDC フィールドが 0 以外を返すか**（マニュアル記載通り動くか） | 1 台で Run し `event.DataGroup[0].TDC` を print。複数イベントで単調増加 |
| **TDC が SW_StartAcquisition / S_IN トリガ時にゼロリセットされるか** | Sec 10.19.3 に「reset at acquisition start or external signal」と記載あり。実測で Run2 の最初の TDC を確認 |
| **EVENT_MODE bit (header word 2 bit[24]) を 1 に設定しないと何が起きるか** | マニュアル注記では「customized SW では manual 設定必須」。忘れた場合のデコード失敗モードを把握 |
| **`Charge` の値域とゲート幅** | パルサを CH に入れ、波形と `Charge` を比較。Standard mode での gate は FW 固定値 |
| **self-trigger discriminator の応答** | 閾値を上げ下げして rate が変化するか |
| **SinFanout slave 動作確認** | PSD Master と V1743 slave を接続し、同時に start するか |
| **TriggerPolarity の ch 単位 / group 単位** | ch 単位で設定可能か確認（API ドキュメントが曖昧） |
| **ADC Calibration のタイミング** | Configure 中 vs Arm 中、どこで走らせても tune-up との衝突がないか |
| **record_length を削減した際の実レート** | 1024 → 256 → 128 と下げてサステイン可能な rate と FULL 発生頻度を計測 |

---

## 9. 落とし穴 (Gemini 指摘 + DELILA 既知)

1. **イベント順序非保証**: `ReadData` のイベント列は時系列順ではない → Merger 側でソート前提
2. **データスループット**: 3 台 × 高 rate で USB/Optical/CPU が律速。波形保存 ON だと特に重い → `save_waveform` は selective に
3. **FW 依存性**: TDC / Charge / edge timestamp が float で返るのは FW 4.29_2.24 以降。SN:25 の実搭載 FW を確認必須
4. **PSD Master S_OUT の負荷**: 既存 PSD slave 分に加えて V1743 3 台も食う。信号分配器の段数を確認
5. **S_IN レベルトリガ問題**: arm_all_sync で全ボードの hw Armed 完了を待つ運用を V1743 にも徹底
6. **エラー時のシステム停止**: 1 台のリンク断が全体の同期を崩す → ReadLoop transient retry は既存どおり、fatal error は arm_all_sync で停止
7. **ADC calibration タイミング**: 現 PSD1 修正で「Arm ブロックで走ると 650ms 遅延」問題があり、Configure 終端に移した経緯あり。V1743 も同じ方針で Configure 末尾実行

---

## 10. 撤退する資産

- `src/reader/caen_legacy/handle.rs::apply_config_dpp_ci()` → 削除
- `set_dpp_parameters` (DPP_CI_Params_t 版) → 削除
- `DppEventBuffer::get_channel_events` → 削除
- `malloc_dpp_events` / `get_dpp_events` / `DppEventBuffer` の DPP-CI 経路 → 削除
- `set_dpp_acquisition_mode`, `set_dpp_event_aggregation`, `set_dpp_pre_trigger_size` → x743 からは呼ばない
- `FirmwareType::X743CI` → deprecated（schema compatibility のため残すが警告）
- `src/bin/x743_ci_probe.rs` → 実機での API 互換性検証用として保持（参考）
- `docs/x743_dpp_ci_parameters.md` → Archive / 撤退理由を冒頭に明記

---

## 11. 決定の要約

| 決定 | 理由 |
|---|---|
| DPP-CI (Charge Mode) 撤退 | **マニュアル Fig 10.9 が Charge Mode に TDC 無しと明示**。位置計算には時間情報必須 |
| Standard mode 一本化 | TDC 40-bit @ 5ns が唯一の物理時刻ソース（マニュアル Sec 10.10.3.1 太字注記） |
| BTT は 40→64-bit の軽量版 | 40-bit ≈ 91 min、PSD1 のような per-channel 複雑 BTT は不要 |
| Energy は CAEN lib Charge を基本採用 | Standard mode でも FW が算出、Rust 側積分不要。実機で妥当性確認後に soft 切替オプション追加 |
| S1 は較正アンカー、heartbeat ではない | TDC 広レンジのため heartbeat 不要。クロスボード定数オフセット補正（ps 精度）に使用 |
| Master は PSD 側を流用 | システム単一マスターで運用・デバッグ単純化 |
| record_length を可能な限り短縮 | PC 帯域と SRAM buffer depth の両方を改善（マニュアル Sec 10.3）— ただし rising edge 補間に必要な長さは維持 |
| EVENT_MODE bit 手動設定 | 自前 SW では default=0 のため、Reader 初期化で必ず 1 に立てる（マニュアル Sec 10.10.3.1）|
| **時間分解能は 5 ps RMS** | `correction_level="all"` + `TDC*5 + PosEdgeTimeStamp` 合成で達成。**V1743 採用の主要目的**。本プロジェクト（2D 位置計算）の核心要件 |

---

## 付録: User Manual UM2750 Rev.5 からの引用根拠

| 節 | 内容 | 本設計への反映 |
|---|---|---|
| 10.3 Digital Memory Buffer | "Up to 7 full events per channel ... It is possible to configure the board to read less than 1024 samples per event" | record_length 削減で buffer depth 拡張 |
| **10.9 Data Correction** | **Time INL Correction で ~5 ps RMS**、Line Offset で ~20 ps RMS、Individual Pedestal で baseline 0.75 mV RMS | **`correction_level = "all"` を config デフォルトに固定**（本プロジェクトの時間分解能要件のため）|
| 10.10.1 | SW / S-IN / FIRST TRIGGER / LVDS I/O CONTROLLED の 4 起動モード | S-IN CONTROLLED を採用 |
| **10.10.2 Running in Charge Mode** | **"up to 7 kHz for full events"、readout は 256-event FIFO が埋まってから** | Charge Mode の性能特性を確認（ただし本プロジェクトでは未採用）|
| 10.10.3.1 Header | **"EVENT_MODE (Bit[24])... must be set to 1"、"default=0, customized SW では manual 設定必須"** | Reader 初期化で bit 立てを実装 |
| 10.10.3.1 Header | **"物理時刻は TDC 40-bit (Fig 10.8)、EVENT TIME TAG は読み出し時刻であり物理量でない"** | 時刻は必ず TDC を使う |
| 10.10.3.2 Data | TDC = 40-bit @ SAMLONG 200MHz clock, 最大 1h30, reset at acquisition start or external signal | TDC を主時刻に採用、40→64-bit ソフト拡張で十分 |
| **Fig 10.9 Group Data Format in Charge Mode** | **`[REF_CELL_COLUMN | CHARGE]` のみ、TDC 無し、256 events/FIFO** | Charge Mode 撤退の決定的根拠 |
| 10.11 Acquisition Synchronization | "When the Digital Memory Buffer is filled, the board is considered FULL: no trigger is accepted" | FULL ハンドリングと BUSY LED 監視を Reader に実装 |
| 10.11.1 BUSY LED | "conversion dead-time (SAMLONG)" or "at least one of the memories is full" | BUSY rate 監視で現状把握 |
