# V1743 デジタイザサポート (GitHub #6)

**作成日:** 2026-04-06
**ステータス:** Phase 1 完了 → Phase 2 実装中
**設計書:** `docs/plans/x743_integration.md` (2026-02-19)
**リファレンス:**
- `legacy/UM2750_V1743_User_Manual_rev5.pdf` (Rev.5, 2025-05-26, FW 4.29_2.24)
- `legacy/UM1935_CAENDigitizer_U_&_R_Manual_rev17.pdf` (Rev.17, CAENDigitizer Library API)
- `legacy/caenwavedemo_x743-1.2.1/` (CAEN WaveDemo ソース)
- `docs/x743_dpp_ci_parameters.md` — **Charge Mode パラメータリファレンス + 実機テスト結果** (2026-04-15)
- `src/bin/x743_ci_probe.rs` — API 互換性テストツール

---

## 1. 背景と目的

ThGEM 実験用に CAEN V1743 (16ch, 12-bit, 3.2 GS/s SCA) を delila-rs に統合する。
V1743 は FELib 非対応のため、**旧 CAENDigitizer ライブラリ** を使用する。

---

## 2. V1743 ハードウェア要点 (PDF Rev.5 精読結果)

### 2.1 アーキテクチャ
- **ADC**: SAMLONG Switched Capacitor Array, 1024 cells/ch, 12-bit
- **チャンネル**: 16ch (VME) / 8ch (Desktop) = 8/4 グループ (2ch/group)
- **FPGA**: AMC (Cyclone EP3C16, 1 per 4ch) + ROC (Readout/VME/Optical)
- **サンプリング**: 3.2 / 1.6 / 0.8 / 0.4 GS/s (DLL Matrix, 200MHz clock ÷)
- **メモリ**: 7 full events/ch (1024 samples/event), configurable depth
- **デッドタイム**: 13μs + (N_samples/16) * 1.75μs, max 125μs @1024 samples

### 2.2 タイミング (重要)
| カウンタ | ビット幅 | 分解能 | クロック | 最大範囲 | 意味 |
|----------|----------|--------|----------|----------|------|
| **TDC** | 40-bit | 5 ns | SAMLONG 200MHz | ~1h30m | パルスの物理到着時刻 |
| **Event Time Tag** | 31-bit (bit31=rollover) | 10 ns | Trigger 100MHz | — | メモリ書き込み時刻 (物理意味なし) |
| **HIT_COUNTER** | 16-bit | — | Discriminator | — | 前回イベントからの閾値越え回数 |
| **TIME_COUNTER** | 16-bit | 1 μs | 1 MHz clock | — | 前回イベントからの経過時間 |

**注意:** Event Time Tag はグループ間で同一 (メモリ書き込み時刻)。物理タイミングは TDC を使う。

### 2.3 イベントデータ構造

#### Event Header (4 x 32-bit words)
```
Word 0: [31:28]=1010 | [27:0]=TOTAL_EVENT_SIZE (32-bit words)
Word 1: [31:27]=BOARD_ID | [26]=BOARD_FAIL | [25]=RES | [24]=EVENT_MODE(=1) | [23:8]=PATTERN(LVDS) | [7:0]=GROUP_MASK
Word 2: [31:22]=RESERVED | [21:0]=EVENT_COUNTER
Word 3: [31]=ROLLOVER_FLAG | [30:0]=EVENT_TIME_TAG (10ns @100MHz)
```

#### Group Data Format (Waveform Mode, Fig 10.8)
- 16-word 周期構造 (SAMLONG chip column 単位)
- 各 word: `[31:24]=sideband` | `[23:12]=ch1_sample` | `[11:0]=ch0_sample`
- Sideband 内容 (word index in column):
  - 0: `0xAA` (Group Header)
  - 1-2: HIT_COUNTER CH0 (LSB/MSB)
  - 3-4: TIME_COUNTER CH0 (LSB/MSB)
  - 5-6: HIT_COUNTER CH1 (LSB/MSB)
  - 7-8: TIME_COUNTER CH1 (LSB/MSB)
  - 9: SAMPLING_FREQUENCY (2-bit)
  - 10: EVENT_ID (8-bit LSB)
  - 11: RESERVED
  - 12-13: FCR (First Cell Read, 10-bit)
  - 14-18: TDC (40-bit, 5 bytes)
  - 19+: DUMMY
  - Last: `0x55` (Group Trailer)
- **17word 毎に extra word** が挿入される → `DecodeEvent()` が自動除去

#### Group Data Format (Charge Mode, Fig 10.9)
- 512 words/group (256 events × 2 channels)
- 各 word: `[31:30]=00` | `[29:24]=REF_CELL_COLUMN(0-63)` | `[23]=1` | `[22:0]=CHARGE(23-bit 2の補数, pC)`

### 2.4 データ補正 (ソフトウェア適用必須)
1. **Line Offset**: baseline → ~0.95 mV RMS (工場固定, 変更不可)
2. **Individual Pedestal**: → ~0.75 mV RMS, dynamic range 11.7 bits (ユーザー修正可)
3. **Time INL**: sampling timing → ~5 ps RMS (ON/OFF のみ)
4. **Trigger Threshold DAC Offset**: discriminator 精度向上 (工場固定)
- `CAEN_DGTZ_SetSAMCorrectionLevel()` で一括制御
- `LoadSAMCorrectionData()` は `OpenDigitizer2()` 時に自動実行 (数秒かかる)

### 2.5 マルチボード同期
- **CLK**: CLK-IN/CLK-OUT daisy chain (AD9510 PLL, AMPMODU connectors)
- **Run/Stop**: LVDS I/O (Pin 11=Run Status, Pin 8=Memory Full, Pin 13=Memory Clear)
- **Timestamp Reset**: S-IN front panel (全ボード同時パルスで TDC/TimeTag リセット)
- **Busy/Veto**: LVDS I/O で dead time 同期 → TRG-OUT 抑制 (Register 0x8100 bit12)
- **Run Delay**: Register 0x8170 = 2 * (NumBoards - 1 - i) per board

### 2.6 接続方式
| 方式 | API LinkType | 速度 | 備考 |
|------|-------------|------|------|
| Optical Link (CONET2) | PCI / USB_A4818 | 80 MB/s | Daisy chain 8 boards/link |
| USB (V3718/V4718 bridge) | USB | — | Desktop/VME |
| VME | — | BLT32 70MB/s, 2eSST 200MB/s | Base Address 設定 |

---

## 3. WaveDemo ソースコード解析結果

### 3.1 プログラミングシーケンス (`ProgramBoard()`)
```
1. Reset (optional)
2. BoardFailStatus check (0x8178)
3. SetGroupEnableMask (channelEnable → group mask)
4. SetSAMPostTriggerSize (per group, 1-255)
5. SetSAMSamplingFrequency (0=3.2G, 1=1.6G, 2=0.8G, 3=0.4G)
6. Pulser config (EnableSAMPulseGen / DisableSAMPulseGen)
7. SetChannelTriggerThreshold (16-bit DAC, inverted range)
8. SetChannelSelfTrigger (per trigger type)
9. SetSWTriggerMode / SetExtTriggerInputMode
10. SetTriggerPolarity (per channel)
11. SetChannelDCOffset (16-bit DAC, inverted range)
12. SetSAMCorrectionLevel
13. SetMaxNumEventsBLT (1000)
14. SetRecordLength (16-1024, 16 刻み)
15. SetIOLevel (NIM/TTL)
16. SetAcquisitionMode (SW_CONTROLLED)
17. Generic register writes
```

### 3.2 Readout Loop
```
ReadData(SLAVE_TERMINATED_READOUT_MBLT) → buffer
GetNumEvents(buffer, size) → num_events
for each event:
    GetEventInfo(buffer, size, i) → EventInfo + EventPtr
    DecodeEvent(EventPtr) → CAEN_DGTZ_X743_EVENT_t
    // Process/Save/Plot
FreeEvent() at cleanup
```

### 3.3 DAC 値計算
```c
// Trigger Threshold (inverted range)
reg_val = (1.25 - (threshold_V + dc_offset_V)) / 2.50 * 65535

// DC Offset (inverted range)
reg_val = (1.25 + dc_offset_V) / 2.50 * 65535
```

### 3.4 CAEN_DGTZ_X743_EVENT_t 構造
```c
typedef struct {
    uint8_t  GrPresent[MAX_V1743_GROUP_SIZE];  // Group 有効フラグ
    struct {
        uint32_t ChSize;                        // サンプル数
        float    DataChannel[2][ChSize];        // 補正済み ADC データ (float!)
        uint16_t TriggerCount[2];               // HIT_COUNTER
        uint16_t TimeCount[2];                  // TIME_COUNTER
        uint8_t  EventId;                       // Event ID (8-bit)
        uint16_t StartIndexCell;                // FCR (First Cell Read)
        uint64_t TDC;                           // 40-bit Trigger Time Tag
        float    PosEdgeTimeStamp;              // Positive edge fine time (ns)
        float    NegEdgeTimeStamp;              // Negative edge fine time (ns)
        uint16_t PeakIndex;
        float    Peak;
        float    Baseline;
        float    Charge;
    } DataGroup[MAX_V1743_GROUP_SIZE];
} CAEN_DGTZ_X743_EVENT_t;
```

**重要:** `DataChannel` は `float` 型 (補正適用済み)。12-bit ADC 値ではない。

---

## 4. 実装計画 (フェーズ別)

### Phase 1: FFI 基盤 + 接続確認 — ✅ 完了 (2026-04-08)

**タスク:**
- [x] `src/reader/caen_legacy/` モジュール作成
  - `ffi.rs`: bindgen で CAENDigitizer.h (v2.19.0) バインディング自動生成
  - `handle.rs`: `X743Handle` RAII wrapper (Open/Close/Reset/GetInfo + 全設定API)
  - `error.rs`: bindgen 生成 `CAEN_DGTZ_ErrorCode` → `Result` 変換
  - `mod.rs`: 公開 re-exports
- [x] `build.rs` 拡張: `#[cfg(feature = "x743")]` で bindgen + リンク
- [x] `Cargo.toml`: `x743` feature flag 追加
- [x] `FirmwareType` に `X743CI` / `X743Std` 追加 + `is_legacy_api()`, `is_group_based()`, `is_felib()`
- [x] `SourceType` に `X743CI` / `X743Std` 追加
- [x] 接続テストバイナリ: `src/bin/x743_test.rs` (--trigger-test, --pulse-test オプション付き)
- [x] 既存コード全 match 分岐を x743 対応に更新 (digitizer.rs, reader/mod.rs, operator/routes)

**成果物:** 実機で Open → GetInfo → Reset → Close 成功

**実機テスト結果 (2026-04-08, daq@172.18.4.147):**
- 接続: Optical Link, port=0, node=0
- Model: VX1743, Serial: 25, Channels: 8 (4 groups)
- ADC: 12-bit, Family Code: 9 (XX743)
- ROC FW: 04.29, AMC FW: 1.02.24
- SAM Correction: loaded (Open に ~1.7秒)
- Board Fail Status: 0x10 (bit4) — 要調査
- Register Read/Write: OK

### Phase 2: DPP_CI (Charge Integration) モード

**タスク:**
- [ ] `handle.rs` 拡張: Configure / Start / Stop / ReadData
- [ ] `x743_read_loop.rs`: read + decode + EventData 変換ループ
  - ReadData → GetNumEvents → GetEventInfo → DecodeEvent → EventData
  - handle 内で decode まで完結 (thread safety)
- [ ] `decoder/x743_ci.rs`: `CAEN_DGTZ_X743_EVENT_t` → `EventData` マッピング
- [ ] `src/config/digitizer.rs`: `X743Config` 構造体追加
- [ ] TOML 設定ファイル例: `config/config_x743_test.toml`
- [ ] Reader `mod.rs` の FirmwareType 分岐追加
- [ ] 統合テスト: x743 → Merger → Recorder

**EventData マッピング (DPP_CI):**
| EventData | x743 ソース | 備考 |
|-----------|------------|------|
| timestamp_ns | TDC * 5.0 | 5ns resolution |
| module | digitizer_id | config から |
| channel | group * 2 + ch_in_group | 物理 ch 番号 |
| energy | Charge (float→u16 変換) | 積分電荷 |
| energy_short | 0 | CI mode ではなし |
| fine_time | PosEdgeTimeStamp | ns 単位 float |
| flags | Peak | (Baseline << 16) で pack |
| waveform | None | CI mode では波形なし |

### Phase 3: STANDARD (波形) モード

**タスク:**
- [ ] `decoder/x743_std.rs`: 波形データ → EventData + Waveform
- [ ] ソフトウェア信号処理 (WaveDemo WDWaveformProcess.c 参照):
  - ベースライン計算 (移動平均, NsBaseline サンプル)
  - LED / CFD ディスクリミネータ
  - ゲート積分 (energy 計算)
- [ ] Monitor 波形表示 (12-bit float, group→ch 変換)
- [ ] Tune Up 対応

**成果物:** フル波形取得 + ソフトウェア解析

### Phase 4: フロントエンド + 運用

**タスク:**
- [ ] Angular Settings UI: x743 専用コンポーネント
- [ ] Monitor: 12-bit ヒストグラム表示
- [ ] マルチボード同期テスト
- [ ] ドキュメント更新

---

## 5. 設計上の決定事項

### 5.1 read_loop 内でデコード完結
- `DecodeEvent(handle, ...)` が handle を引数に取る → デコードは handle 所有者内で完結必須
- CAENDigitizer は thread-safe 保証なし (2012 年設計)
- x743 は ~7 kHz (max) と低レート → read_loop 内デコードで性能問題なし

### 5.2 Feature flag `x743`
- CAENDigitizer Library は Linux 実機のみ → macOS ビルドに影響させない
- `#[cfg(feature = "x743")]` で全 x743 コードをゲート
- Config の `X743Config` は `Option<X743Config>` として常に定義 (TOML 互換性)

### 5.3 既存パイプラインへの影響
- Merger / Recorder / Monitor: **変更なし** (EventData 共通形式)
- Reader のみ FirmwareType 分岐追加 (小規模)

### 5.4 HWM=0 堅持
- `SetMaxNumEventsBLT(1000)` で board 側バッファリング
- ZMQ HWM=0 はそのまま維持 (データ保全ポリシー)

---

## 6. 未確認事項 (実機テストで確認)

1. ~~DPP_CI モードの `CAEN_DGTZ_X743_EVENT_t` 出力内容~~ → **確認済み** (x743_ci_probe で検証。Standard パスで Charge/Baseline/Peak/TDC 出力)
2. `PosEdgeTimeStamp` / `NegEdgeTimeStamp` の Charge Mode 時の値
3. ~~libCAENDigitizer.so が 172.18.4.147 にインストールされているか~~ → **確認済み** (Phase 1 で接続成功)
4. Charge 値の単位と範囲 (23-bit 2の補数, pC)
5. ~~V1743 実機の FW バージョンと CONET/USB 接続方式~~ → **確認済み** (VX1743 SN:25, ROC 04.29, AMC 1.02.24, Optical Link)

---

## 7. 参考ファイル

| ファイル | 用途 |
|----------|------|
| `legacy/UM2750_V1743_User_Manual_rev5.pdf` | ハードウェア仕様 |
| `legacy/caenwavedemo_x743-1.2.1/src/WaveDemo.c` | ProgramBoard, ReadLoop, DecodeEvent の実装例 |
| `legacy/caenwavedemo_x743-1.2.1/src/WDconfig.c` | 設定パーサー |
| `legacy/caenwavedemo_x743-1.2.1/include/WaveDemo.h` | データ構造定義 |
| `legacy/caenwavedemo_x743-1.2.1/WaveDemoConfig.ini` | 設定ファイル例 |
| `docs/plans/x743_integration.md` | 設計書 (2026-02-19) |
| `src/reader/caen/handle.rs` | 既存 FELib handle (参考) |
| `src/reader/decoder/psd2.rs` | 既存デコーダ (参考) |
