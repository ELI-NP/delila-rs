# PHA1 Decoder Implementation Plan

**Created:** 2026-01-29
**Status: COMPLETED** (2026-01-29)
**Spec Document:** `docs/pha1_decoder_spec.md`
**Reference:** `src/reader/decoder/psd1.rs`

---

## Implementation Summary

**実装完了:** 2026-01-29

### 主な成果

1. **PHA1 デコーダ実装** (`src/reader/decoder/pha1.rs`)
   - PSD1 ベースで PHA1 固有の差分を適用
   - 46 テスト pass
   - DIG1 プロトコル対応

2. **Reader 統合**
   - `DecoderKind::Pha1` 追加
   - `FirmwareType::PHA1` 追加 (`dig1://` URL スキーム)
   - `SourceType::Pha1` config 対応

3. **実機検証**
   - DT5730B (Serial: 990, DPP-PHA1, USB)
   - 29,931 events @ 903 evt/s
   - フルパイプライン動作確認

### 修正されたファイル

- `src/reader/decoder/pha1.rs` (新規)
- `src/reader/decoder/mod.rs` (pha1 追加)
- `src/reader/mod.rs` (DecoderKind, FirmwareType 対応)
- `src/config/digitizer.rs` (FirmwareType::PHA1, url_scheme() 修正)
- `config/digitizers/pha1_test.json` (テスト設定)

---

## 1. Overview

DPP-PHA1 (Pulse Height Analysis) ファームウェア用デコーダの実装計画。
PSD1 デコーダをベースに、PHA1 固有の差分を適用する。

---

## 2. Implementation Strategy

### 2.1 Approach: PSD1 Clone + Diff

PSD1 と PHA1 は 90% 以上のコードが共通。以下の戦略を採用:

1. `psd1.rs` をコピーして `pha1.rs` を作成
2. PHA1 固有の差分のみを修正
3. 共通部分の抽出は **行わない** (KISS 原則)

**理由:**
- 共通コードの trait 化は over-engineering
- PSD1/PHA1 は別ファームウェアで今後独立して進化する可能性
- デバッグ時に各デコーダを独立して追跡可能

### 2.2 Alternative Considered (Not Adopted)

trait を使った共通化:
```rust
trait Dig1Decoder {
    fn decode_board_header(...);
    fn decode_dual_channel_header(...);  // PSD1/PHA1 で異なる
    fn decode_event(...);                // PSD1/PHA1 で異なる
}
```

**却下理由:** 抽象化コストが差分の小ささに見合わない。

---

## 3. Diff Analysis: PSD1 → PHA1

### 3.1 Constants Module

| Section | PSD1 | PHA1 | Action |
|---------|------|------|--------|
| `WORD_SIZE` | 4 | 4 | 同一 |
| `board_header::*` | - | - | **同一** |
| `channel_header::DUAL_CHANNEL_SIZE_MASK` | `0x3FFFFF` (22-bit) | `0x7FFFFFFF` (31-bit) | **変更** |
| `channel_header::DIGITAL_PROBE1_*` | bit [18:16] | → `DIGITAL_PROBE` bit [19:16] | **変更** |
| `channel_header::DIGITAL_PROBE2_*` | bit [21:19] | 削除 | **削除** |
| `channel_header::ANALOG_PROBE_*` | bit [23:22] | → `ANALOG_PROBE1` bit [23:22] | **変更** |
| - | - | 追加: `ANALOG_PROBE2` bit [21:20] | **追加** |
| `channel_header::EXTRAS_ENABLED_SHIFT` | 28 | → `EXTRAS2_ENABLED_SHIFT` | **リネーム** |
| `channel_header::CHARGE_ENABLED_SHIFT` | 30 | → `ENERGY_ENABLED_SHIFT` | **リネーム** |
| `event::CHARGE_*` | - | → `event::ENERGY_*` | **変更** |
| `waveform::DP1_SHIFT` | 14 | → `DP_SHIFT` | **リネーム** |
| `waveform::DP2_SHIFT` | 15 | → `TRIGGER_FLAG_SHIFT` | **リネーム** |

### 3.2 Internal Data Structures

#### DualChannelHeader

```rust
// PSD1
struct DualChannelHeader {
    block_size: u32,
    num_samples_wave: u16,
    digital_probe1: u8,      // 3-bit
    digital_probe2: u8,      // 3-bit
    analog_probe: u8,        // 2-bit
    extra_option: u8,
    samples_enabled: bool,
    extras_enabled: bool,    // bit 28
    time_enabled: bool,
    charge_enabled: bool,    // bit 30
    dual_trace: bool,
}

// PHA1 (変更点のみコメント)
struct DualChannelHeader {
    block_size: u32,         // mask変更: 31-bit
    num_samples_wave: u16,
    digital_probe: u8,       // 4-bit (single)  ← 変更
    analog_probe1: u8,       // 2-bit           ← 変更
    analog_probe2: u8,       // 2-bit           ← 追加
    extra_option: u8,
    samples_enabled: bool,
    extras2_enabled: bool,   // bit 28          ← リネーム
    time_enabled: bool,
    energy_enabled: bool,    // bit 30          ← リネーム
    dual_trace: bool,
}
```

### 3.3 Functions

| Function | PSD1 | PHA1 | Action |
|----------|------|------|--------|
| `classify()` | - | - | 同一 |
| `decode()` | - | - | 同一 |
| `decode_board_aggregate()` | - | - | 同一 |
| `decode_board_header()` | - | - | 同一 |
| `decode_dual_channel_block()` | - | - | 同一 |
| `decode_dual_channel_header()` | Parse DP1,DP2,AP | Parse DP,AP1,AP2 | **変更** |
| `event_size_words()` | `charge_enabled` | `energy_enabled` | **変更** |
| `decode_event()` | Call `decode_charge_word` | Call `decode_energy_word` | **変更** |
| `decode_waveform()` | DP1, DP2 extraction | DP, Tn extraction | **変更** |
| `decode_extras_word()` | - | - | 同一 |
| `decode_charge_word()` | - | → `decode_energy_word()` | **変更** |
| `calculate_timestamp()` | - | - | 同一 |

### 3.4 Tests

PSD1 のテストをコピーして以下を修正:
- `make_charge_word()` → `make_energy_word()`
- `charge_long/charge_short` → `energy/extra_data`
- Waveform digital probe テスト修正

---

## 4. Implementation Steps

### Phase 1: File Creation & Constants (30 min)

- [ ] `src/reader/decoder/pha1.rs` を `psd1.rs` からコピー
- [ ] モジュールドキュメント更新 (`//! PHA1 Decoder for DT5730 (DPP-PHA1)`)
- [ ] Constants 修正
  - [ ] `channel_header::DUAL_CHANNEL_SIZE_MASK` → `0x7FFFFFFF`
  - [ ] Digital probe constants 変更
  - [ ] Analog probe constants 変更
  - [ ] Enable flag constants リネーム
  - [ ] Energy word constants 追加

### Phase 2: Data Structures (15 min)

- [ ] `DualChannelHeader` 構造体修正
  - [ ] `digital_probe1/2` → `digital_probe`
  - [ ] `analog_probe` → `analog_probe1/2`
  - [ ] `extras_enabled` → `extras2_enabled`
  - [ ] `charge_enabled` → `energy_enabled`

### Phase 3: Decoder Functions (30 min)

- [ ] `decode_dual_channel_header()` 修正
  - [ ] Probe 解析変更
  - [ ] Enable flag 解析変更
- [ ] `DualChannelHeader::event_size_words()` 修正
- [ ] `decode_event()` 修正
  - [ ] `charge_enabled` → `energy_enabled`
  - [ ] `decode_charge_word()` → `decode_energy_word()` 呼び出し
- [ ] `decode_waveform()` 修正
  - [ ] DP1, DP2 → DP, Tn

### Phase 4: Helper Functions (15 min)

- [ ] `decode_charge_word()` → `decode_energy_word()` 変更
  - [ ] Return type: `(energy, extra_data, pileup)`
- [ ] EventData マッピング修正
  - [ ] `energy` ← energy
  - [ ] `energy_short` ← extra_data

### Phase 5: Tests (45 min)

- [ ] Test helper functions 修正
  - [ ] `make_charge_word()` → `make_energy_word()`
  - [ ] `DualChFlags` フィールド修正
- [ ] 既存テスト修正
  - [ ] フィールド名変更に伴う修正
  - [ ] 期待値の修正
- [ ] 新規テスト追加
  - [ ] Energy word decode テスト
  - [ ] Extra data 抽出テスト
  - [ ] Waveform DP/Tn テスト

### Phase 6: Integration (15 min)

- [ ] `src/reader/decoder/mod.rs` 更新
  - [ ] `pub mod pha1;`
  - [ ] `pub use pha1::{Pha1Config, Pha1Decoder};`
- [ ] `DecoderKind` enum への PHA1 追加 (必要に応じて)
- [ ] `cargo check` & `cargo clippy`
- [ ] `cargo test` (全テスト pass)

---

## 5. Estimated Effort

| Phase | Time |
|-------|------|
| Phase 1: Constants | 30 min |
| Phase 2: Structures | 15 min |
| Phase 3: Functions | 30 min |
| Phase 4: Helpers | 15 min |
| Phase 5: Tests | 45 min |
| Phase 6: Integration | 15 min |
| **Total** | **~2.5 hours** |

---

## 6. Risk Assessment

| Risk | Mitigation |
|------|------------|
| C++ reference と Rust 実装の不整合 | C++ コードと逐次比較 |
| 実機テスト環境なし | 単体テストを充実させる |
| PSD1 とのフィールド混乱 | 明確なコメントと命名 |

---

## 7. Acceptance Criteria

- [ ] `cargo test decoder::pha1` が全 pass
- [ ] `cargo clippy` で警告なし
- [ ] 仕様書 (`docs/pha1_decoder_spec.md`) との整合性確認
- [ ] C++ 実装 (`PHA1Decoder.cpp`) との動作一致

---

## 8. Future Work (Out of Scope)

- Reader への PHA1 統合 (`DecoderKind::Pha1`)
- Config ファイルでの PHA1 選択サポート
- 実機テスト (ハードウェア入手後)

---

## Appendix: Key Code Snippets

### A.1 Energy Word Decoding (PHA1)

```rust
/// Decode energy word (PHA1 specific)
///
/// Returns (energy, extra_data, pileup)
fn decode_energy_word(word: u32) -> (u16, u16, bool) {
    let energy = (word & constants::event::ENERGY_MASK) as u16;
    let pileup = ((word >> constants::event::PILEUP_SHIFT) & 1) != 0;
    let extra_data = ((word >> constants::event::EXTRA_SHIFT)
                      & constants::event::EXTRA_MASK) as u16;
    (energy, extra_data, pileup)
}
```

### A.2 Waveform Decoding (PHA1)

```rust
// PHA1: DP (bit 14) + Tn (bit 15) per sample
let s1_dp = ((w >> constants::waveform::DP_SHIFT) & 1) as u8;
let s1_tn = ((w >> constants::waveform::TRIGGER_FLAG_SHIFT) & 1) as u8;

// Map to Waveform struct
digital_probe1.push(s1_dp);   // DP → digital_probe1
digital_probe2.push(s1_tn);   // Tn → digital_probe2 (reuse field)
```
