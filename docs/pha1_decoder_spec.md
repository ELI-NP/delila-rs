# PHA1 Decoder Specification

**Document Version:** 1.1
**Last Updated:** 2026-02-18
**Status:** 実機検証済み (DT5730B, Serial: 990)
**Hardware Target:** DT5730 (DPP-PHA1, 8/16ch, 14-bit, 500 MS/s)

---

## 1. Overview

### 1.1 Purpose

DPP-PHA1 ファームウェア (x725/x730 シリーズ) の RAW データデコーダ仕様。
DELILA-RS DAQ の Reader コンポーネントに統合し、既存の PSD1/PSD2 デコーダと同じ `EventData` 出力に変換する。

### 1.2 PHA vs PSD

| Firmware | 用途 | 出力 |
|----------|------|------|
| DPP-PHA (Pulse Height Analysis) | エネルギー測定 | Energy のみ |
| DPP-PSD (Pulse Shape Discrimination) | 粒子識別 | Charge Long + Charge Short |

PHA1 は PSD1 と同じデータ構造（階層型 Board → Channel Pair → Event）を持つが、以下が異なる：
1. **Energy Word**: `charge_long + charge_short` → `energy + extra`
2. **Digital Probes**: 2つ (DP1, DP2) → 1つ (DP) + Trigger Flag
3. **Analog Probes**: 1つ (AP) → 2つ (AP1, AP2)
4. **Enable Flags**: `EE=Extras, EQ=Charge` → `E2=Extras2, EE=Energy`

### 1.3 Hardware

| Property | Value |
|----------|-------|
| Model | DT5730 |
| Firmware | DPP-PHA (PHA1) |
| Library | CAEN_Dig1 (`libCAEN_Dig1.so`) |
| FELib URL | `dig1://` scheme |
| Channels | 8 or 16 |
| ADC Resolution | 14-bit |
| Sampling Rate | 500 MS/s |
| Word Size | **32-bit** (Little-Endian) |

### 1.4 Connection

PSD1 と同一。`dig1://` スキームを使用。

| Interface | URL Format |
|-----------|------------|
| USB | `dig1://caen.internal/usb?link_num=<N>` |
| Optical Link | `dig1://caen.internal/optical_link?link_num=<N>&conet_node=<M>` |

---

## 2. Data Format

### 2.1 Word Format

- **Word size:** 32-bit (4 bytes)
- **Byte order:** Little-Endian (x86 ネイティブ、バイトスワップ不要)
- **PSD1 と同一**

### 2.2 Data Hierarchy

PSD1 と同一の階層構造:

```
┌─────────────────────────────────────────┐
│ Board Aggregate Block                    │
│ ┌─────────────────────────────────────┐ │
│ │ Board Header (4 words)              │ │
│ ├─────────────────────────────────────┤ │
│ │ Dual Channel Block (pair 0)         │ │
│ │ ┌─────────────────────────────────┐ │ │
│ │ │ Channel Header (2 words)       │ │ │
│ │ ├─────────────────────────────────┤ │ │
│ │ │ Event 0                        │ │ │
│ │ │ Event 1                        │ │ │
│ │ │ ...                            │ │ │
│ │ └─────────────────────────────────┘ │ │
│ ├─────────────────────────────────────┤ │
│ │ Dual Channel Block (pair 1)         │ │
│ │ ...                                 │ │
│ └─────────────────────────────────────┘ │
└─────────────────────────────────────────┘
```

---

## 3. Board Aggregate Header (4 words)

**PSD1 と完全に同一**

### 3.1 Word 0: Type + Size

```
┌──────────────────────────────────────────┐
│ 31  30  29  28 │ 27              ... 0   │
│  1   0   1   0 │   Aggregate Size        │
│  (Type=0xA)    │   (in 32-bit words)     │
└──────────────────────────────────────────┘
```

| Field | Bits | Mask | Description |
|-------|------|------|-------------|
| Type | [31:28] | `0xF << 28` | Header type identifier = **0xA** |
| Aggregate Size | [27:0] | `0x0FFFFFFF` | Size of entire board block (words) |

### 3.2 Word 1: Board Info

```
┌──────────────────────────────────────────┐
│ 31:27 │ 26 │ 25:23 │ 22:8      │ 7:0    │
│BoardID│Fail│ Rsv   │LVDS Ptn   │DualChM │
└──────────────────────────────────────────┘
```

| Field | Bits | Mask/Shift | Description |
|-------|------|------------|-------------|
| Board ID | [31:27] | `0x1F << 27` | Board identifier (0-31) |
| Board Fail | [26] | `0x1 << 26` | Board failure flag |
| LVDS Pattern | [22:8] | `0x7FFF << 8` | LVDS pattern |
| Dual Channel Mask | [7:0] | `0xFF` | Active channel pairs |

### 3.3 Word 2: Counter

| Field | Bits | Mask | Description |
|-------|------|------|-------------|
| Aggregate Counter | [22:0] | `0x7FFFFF` | Monotonic counter |

### 3.4 Word 3: Time Tag

| Field | Bits | Description |
|-------|------|-------------|
| Board Time Tag | [31:0] | Board-level time tag |

---

## 4. Dual Channel Header (2 words)

### 4.1 Word 0: Size

**PSD1 と同一だが、サイズマスクが異なる可能性**

```
┌──────────────────────────────────────────┐
│ 31 │ 30:0                                 │
│  1 │ Dual Channel Size (31-bit)           │
└──────────────────────────────────────────┘
```

| Field | Bits | Mask | Description |
|-------|------|------|-------------|
| Header Flag | [31] | `0x1 << 31` | Always 1 (reserved) |
| Dual Channel Size | [30:0] | `0x7FFFFFFF` | Size of this channel block (words) |

**注意:** PSD1 は [21:0] (22-bit)、PHA1 は [30:0] (31-bit) サイズ。

### 4.2 Word 1: Configuration (PHA1 固有)

**PSD1 と異なる！**

```
┌──────────────────────────────────────────────────────────────┐
│ 31 │ 30 │ 29 │ 28 │ 27  │ 26:24     │ 23:22 │ 21:20 │ 19:16 │ 15:0        │
│ DT │ EE │ ET │ E2 │ ES  │ExtraOpt   │  AP1  │  AP2  │   DP  │ numSampWave │
└──────────────────────────────────────────────────────────────┘
```

| Field | Bits | Mask/Shift | Description |
|-------|------|------------|-------------|
| DT (Dual Trace) | [31] | `0x1 << 31` | Dual trace enabled |
| EE (Energy Enable) | [30] | `0x1 << 30` | **Energy word enabled** (PSD1: EQ=Charge) |
| ET (Time Enable) | [29] | `0x1 << 29` | Time tag enabled |
| E2 (Extras2 Enable) | [28] | `0x1 << 28` | **Extras2 word enabled** (PSD1: EE=Extras) |
| ES (Samples Enable) | [27] | `0x1 << 27` | Waveform enabled |
| Extra Option | [26:24] | `0x7 << 24` | Extras word format |
| AP1 (Analog Probe 1) | [23:22] | `0x3 << 22` | **Analog probe 1 type** |
| AP2 (Analog Probe 2) | [21:20] | `0x3 << 20` | **Analog probe 2 type** (PSD1: DP2) |
| DP (Digital Probe) | [19:16] | `0xF << 16` | **4-bit digital probe** (PSD1: DP1=3bit+DP2=3bit) |
| Num Samples Wave | [15:0] | `0xFFFF` | Waveform samples / 8 |

### 4.3 PSD1 vs PHA1 Enable Flags 比較

| Bit | PSD1 | PHA1 |
|-----|------|------|
| [31] | DT (Dual Trace) | DT (Dual Trace) |
| [30] | EQ (Charge Enabled) | **EE (Energy Enabled)** |
| [29] | ET (Time Enabled) | ET (Time Enabled) |
| [28] | EE (Extras Enabled) | **E2 (Extras2 Enabled)** |
| [27] | ES (Samples Enabled) | ES (Samples Enabled) |

### 4.4 PSD1 vs PHA1 Probe 比較

| Field | PSD1 | PHA1 |
|-------|------|------|
| [23:22] | AP (2-bit) | AP1 (2-bit) |
| [21:19] | DP2 (3-bit) | AP2 [21:20] (2-bit) |
| [18:16] | DP1 (3-bit) | DP [19:16] (4-bit) |

---

## 5. Event Structure

### 5.1 Event Layout

```
Event:
  [Word 0] Trigger Time Tag (ET=1 の場合)
  [Word 1] Extras (E2=1 の場合)
  [Words ] Waveform data (ES=1 の場合, numSamplesWave*2 words)
  [Word N] Energy (EE=1 の場合)
```

**順序:** `Time → Extras → Waveform → Energy` (PSD1 と同一順序、最後が Charge→Energy に変更)

### 5.2 Trigger Time Tag Word (ET=1)

**PSD1 と同一**

```
┌──────────────────────────────────────────┐
│ 31       │ 30:0                           │
│ Ch Flag  │ Trigger Time Tag               │
└──────────────────────────────────────────┘
```

| Field | Bits | Mask | Description |
|-------|------|------|-------------|
| Channel Flag | [31] | `0x1 << 31` | 0 = even channel, 1 = odd channel |
| Trigger Time Tag | [30:0] | `0x7FFFFFFF` | 31-bit coarse timestamp |

### 5.3 Extras Word (E2=1)

**PSD1 と同一フォーマット** (Extra option = 0b010 の場合)

```
┌──────────────────────────────────────────┐
│ 31:16            │ 15:10 │ 9:0            │
│ Extended Time    │ Flags │ Fine Time      │
└──────────────────────────────────────────┘
```

| Field | Bits | Mask | Description |
|-------|------|------|-------------|
| Extended Time | [31:16] | `0xFFFF << 16` | 上位16ビット時刻 |
| Flags | [15:10] | `0x3F << 10` | 6-bit event flags |
| Fine Time | [9:0] | `0x3FF` | 10-bit fine timestamp |

### 5.4 Energy Word (EE=1) - PHA1 固有

**PSD1 の Charge Word とは異なる！**

```
┌──────────────────────────────────────────┐
│ 31:26    │ 25:16      │ 15      │ 14:0   │
│ Reserved │ Extra Data │ Pileup  │ Energy │
└──────────────────────────────────────────┘
```

| Field | Bits | Mask | Description |
|-------|------|------|-------------|
| Energy | [14:0] | `0x7FFF` | **15-bit energy value** |
| Pileup | [15] | `0x1 << 15` | Pileup detection flag |
| Extra Data | [25:16] | `0x3FF << 16` | **10-bit extra data** |
| Reserved | [31:26] | - | Reserved |

### 5.5 PSD1 vs PHA1 Energy/Charge 比較

| Field | PSD1 (Charge Word) | PHA1 (Energy Word) |
|-------|-------------------|-------------------|
| [14:0] | Charge Short (15-bit) | **Energy (15-bit)** |
| [15] | Pileup | Pileup |
| [31:16] | Charge Long (16-bit) | **Extra Data (10-bit) + Reserved** |

**EventData マッピング:**
- PSD1: `energy = charge_long`, `energy_short = charge_short`
- PHA1: `energy = energy`, `energy_short = extra_data` (用途違い注意)

---

## 6. Waveform Data (ES=1)

### 6.1 Sample Packing (PHA1 固有)

**PSD1 と異なる！**

```
┌──────────────────────────────────────────┐
│ 31 │ 30 │ 29:16                │ 15 │ 14 │ 13:0           │
│ Tn │ DP │ Analog Sample (odd)  │ Tn │ DP │ Analog Sample  │
│ s2 │ s2 │ 14-bit               │ s1 │ s1 │ (even) 14-bit  │
└──────────────────────────────────────────┘

Lower half (bits [15:0]) = Sample 2N
Upper half (bits [31:16]) = Sample 2N+1
```

| Field | Bits | Mask | Description |
|-------|------|------|-------------|
| Analog Sample 1 | [13:0] | `0x3FFF` | **14-bit 2の補数** analog value (sample 2N) |
| DP (s1) | [14] | `0x1 << 14` | **Digital Probe for sample 2N** (configurable, vtrace/3) |
| Tn (s1) | [15] | `0x1 << 15` | **Trigger Flag for sample 2N** (fixed, vtrace/2) |
| Analog Sample 2 | [29:16] | `0x3FFF << 16` | **14-bit 2の補数** analog value (sample 2N+1) |
| DP (s2) | [30] | `0x1 << 30` | **Digital Probe for sample 2N+1** |
| Tn (s2) | [31] | `0x1 << 31` | **Trigger Flag for sample 2N+1** |

### 6.2 Analog Sample 符号処理

**PHA1 は 14-bit 2の補数（符号あり）。PSD1 は 14-bit 符号なし。**

- PHA1: `sign_extend_14bit(value)` → `i16` 範囲 [-8192, +8191]
  - 負パルス入力 → 負の波形値として表示される
  - 実装: `((value << 18) as i32 >> 18) as i16` (算術右シフトで符号拡張)
- PSD1: `(value & 0x3FFF) as i16` → 常に正値 [0, 16383]
  - 上位ビットが0なので符号拡張不要

### 6.3 PSD1 vs PHA1 Waveform 比較

| Bit | PSD1 | PHA1 |
|-----|------|------|
| [13:0] | Analog (符号なし) | **Analog (2の補数、符号拡張必要)** |
| [14] | DP1 | **DP (configurable, vtrace/3)** |
| [15] | DP2 | **Tn (fixed trigger, vtrace/2)** |

### 6.3 Total Waveform Words

PSD1 と同一:
```
total_samples = numSamplesWave × 8
total_words = numSamplesWave × 2
```

### 6.4 Dual Trace Mode (DT=1)

PSD1 と同一:
- 偶数サンプル (2N): Analog Probe 1
- 奇数サンプル (2N+1): Analog Probe 2

### 6.5 Analog Probe Types (PHA1)

| Value | Description |
|-------|-------------|
| 0 | Input |
| 1 | Trapezoid |
| 2 | Energy (Trapezoid after processing) |
| 3 | Timestamp |

### 6.6 Digital Probe Types (PHA1)

4-bit (0-15) - 詳細は CAEN マニュアル参照。代表的な値:

| Value | Description |
|-------|-------------|
| 0 | Trigger |
| 1 | Trapezoid (reduced) |
| 2 | Peaking |
| 3 | Baseline |
| ... | ... |

---

## 7. Timestamp Calculation

**PSD1 と同一**

### 7.1 Coarse Timestamp

```
combined = (extended_time << 31) | trigger_time_tag
timestamp_ns = combined × time_step_ns
```

**time_step_ns:** DT5730 = 2 ns (500 MS/s)

### 7.2 Fine Timestamp

```
fine_time_ns = fine_time × (time_step_ns / 1024.0)
final_timestamp_ns = timestamp_ns + fine_time_ns
```

---

## 8. Start/Stop Signal Handling

**PSD1 と同一** - RAW データに Start/Stop シグナルは含まれない。

---

## 9. PHA1 vs PSD1 vs PSD2 比較表

| Feature | PHA1 (DPP-PHA) | PSD1 (DPP-PSD) | PSD2 (VX2730) |
|---------|----------------|----------------|---------------|
| **用途** | Energy measurement | Particle ID (PSD) | Particle ID (PSD) |
| **Word size** | 32-bit LE | 32-bit LE | 64-bit BE |
| **Data structure** | Hierarchical | Hierarchical | Flat |
| **Board header** | 4 words (0xA) | 4 words (0xA) | 1 word (0x2) |
| **Ch Header Size mask** | [30:0] (31-bit) | [21:0] (22-bit) | N/A |
| **Enable bit[30]** | EE (Energy) | EQ (Charge) | - |
| **Enable bit[28]** | E2 (Extras2) | EE (Extras) | - |
| **Digital Probes** | 1 × 4-bit DP | 2 × 3-bit (DP1, DP2) | 4 DP |
| **Analog Probes** | 2 × 2-bit (AP1, AP2) | 1 × 2-bit AP | 2 AP |
| **Energy/Charge output** | `energy[14:0]` | `charge_long[31:16]` + `charge_short[14:0]` | `energy[15:0]` + `energy_short[25:16]` |
| **Extra field** | `extra[25:16]` (10-bit) | N/A | N/A |
| **Waveform analog** | **14-bit 2の補数** | 14-bit 符号なし | 14-bit 符号あり |
| **Waveform bit[14]** | DP → D1 | DP1 | DP1 |
| **Waveform bit[15]** | Tn → D0 (Trigger) | DP2 | DP2 |
| **Fine time** | 10-bit | 10-bit | 10-bit |
| **Time step** | 2 ns | 2 ns | 8 ns |

---

## 10. EventData Mapping

### 10.1 Field Mapping

```rust
pub struct EventData {
    pub timestamp_ns: f64,      // ← (ext << 31 + ttt) × step + fine × (step/1024)
    pub module: u8,             // ← config.module_id
    pub channel: u8,            // ← pair * 2 + channel_flag
    pub energy: u16,            // ← energy (15-bit)
    pub energy_short: u16,      // ← extra_data (10-bit) ※用途が異なる
    pub fine_time: u16,         // ← fine_time (10-bit)
    pub flags: u32,             // ← 6-bit flags + pileup
    pub waveform: Option<Waveform>,
}
```

**注意:** PHA1 の `energy_short` フィールドには PSD の charge_short ではなく、extra_data が格納される。用途が異なるため、アプリケーション側での解釈に注意。

### 10.2 Waveform Mapping

```rust
pub struct Waveform {
    pub analog_probe1: Vec<i16>,   // ← 14-bit 2の補数 (sign_extend_14bit)
    pub analog_probe2: Vec<i16>,   // ← DT=1 の場合のみ (同上)
    pub digital_probe1: Vec<u8>,   // ← Tn (Trigger flag, fixed, vtrace/2) = D0
    pub digital_probe2: Vec<u8>,   // ← DP (configurable, vtrace/3) = D1
    pub digital_probe3: Vec<u8>,   // ← 未使用 (empty)
    pub digital_probe4: Vec<u8>,   // ← 未使用 (empty)
    pub time_resolution: u8,       // ← 0
    pub trigger_threshold: u16,    // ← 0
}
```

**Digital Probe マッピング:**
- `digital_probe1` (D0) = bit15 Tn (trigger flag, 固定)
- `digital_probe2` (D1) = bit14 DP (configurable digital probe)
- vtrace UI の表示順序 (D0=Trigger, D1=DP) と一致させるため、ビット位置と逆順にマッピング
```

---

## 11. Error Handling

**PSD1 と同一**

---

## 12. References

| Document | Location | Description |
|----------|----------|-------------|
| PHA1 C++ Constants | `legacy/DELILA2/lib/digitizer/include/PHA1Constants.hpp` | ビットマスク定義 |
| PHA1 C++ Structures | `legacy/DELILA2/lib/digitizer/include/PHA1Structures.hpp` | データ構造体 |
| PHA1 C++ Decoder | `legacy/DELILA2/lib/digitizer/src/PHA1Decoder.cpp` | リファレンス実装 |
| PSD1 Rust Decoder | `src/reader/decoder/psd1.rs` | PSD1 Rust 実装 (ベース) |
| PSD1 Spec | `docs/psd1_decoder_spec.md` | PSD1 仕様書 |

---

## Appendix A: 実装チェックリスト

### A.1 PSD1 との差分ポイント

- [x] Dual Channel Header Word 1 の解析変更
  - [x] `EE` → Energy Enable (bit 30)
  - [x] `E2` → Extras2 Enable (bit 28)
  - [x] `DP` → 4-bit single digital probe (bits 19:16)
  - [x] `AP1`, `AP2` → 2×2-bit analog probes (bits 23:22, 21:20)
  - [x] Size mask → [30:0] (31-bit)
- [x] Energy Word 解析 (PSD1 の decode_charge_word を変更)
  - [x] `energy[14:0]` 抽出
  - [x] `pileup[15]` 抽出
  - [x] `extra[25:16]` 抽出
- [x] Waveform 解析
  - [x] bit[14] = DP → digital_probe2 (D1, configurable)
  - [x] bit[15] = Tn → digital_probe1 (D0, fixed trigger)
  - [x] **14-bit 2の補数 → sign_extend_14bit()** (PSD1は符号なし)
- [x] EventData マッピング
  - [x] `energy` ← energy
  - [x] `energy_short` ← extra_data
