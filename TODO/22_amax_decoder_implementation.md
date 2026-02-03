# AMax カスタムファームウェア デコーダ実装

**作成日:** 2026-01-29
**状態:** 🔄 実装中

## 概要

DELILA グループが開発中の AMax (Trapezoidal Filter MCA) カスタムファームウェア用デコーダを実装する。
標準 CAEN DPP とは異なる独自フォーマットで、振幅最大値検出 (AMax) とトラペゾイダルフィルタを組み合わせた核分光用信号処理を行う。

## ハードウェア情報

- **ターゲット:** VX2730 (DIG2 プロトコル) at 172.18.4.56
- **ファームウェア:** `TrapezoidalfilterMCA_dpp_tests` (FWアップロード済み)
- **CUP ファイル:** `TrapezoidalfilterMCA_dpp_tests/output/output/V2730-OpenDPP-TrapezoidalfilterMCAdpptests-2026012754.cup`
- **チャンネル:** **1ch のみ (A0 = ch0)**

## ドキュメント

- **CUP Documentation:** `TrapezoidalfilterMCA_dpp_tests/documentation_2026012754/`
- **C++ Reference:** `legacy/DELILA2/lib/digitizer/`

## レジスタアクセステスト結果 ✅

| カテゴリ | 結果 |
|---------|------|
| Core Registers (0x0-0xC) | 13/13 成功 |
| AMax Registers (0x14000, 0x160000-0x160005) | 7/7 成功 |
| Trigger AND (0xC00A-0xC00B) | 書き込み専用 (WR) |
| **合計** | 20/22 読み書き可能 |

---

## データフォーマット (CUP Documentation より)

### エンドポイント

| Endpoint | Path | 用途 |
|----------|------|------|
| RAW | `/endpoint/raw` | 生データ (DATA, SIZE, N_EVENTS) |
| OpenDPP | `/endpoint/opendpp` | デコード済み |
| ActiveEndpoint | `/endpoint/par/activeendpoint` | 切り替え |

### RAW イベント構造 (Open-DPP Individual Trigger Mode)

```
Word 0: [Header]
  ├─ bit  63     - 0x0 (not last word)
  ├─ bits 62-56  - Channel (7-bit)
  ├─ bit  55     - Special Event flag
  ├─ bits 54-51  - Info
  └─ bits 47-0   - Timestamp (48-bit, 1 LSB = 8 ns)

Word 1: [Data]
  ├─ bit  63     - Last Word flag (1 if no user words/waveform)
  ├─ bit  62     - Waveform Present flag
  ├─ bits 61-50  - Flags B (12-bit)
  ├─ bits 49-42  - Flags A (8-bit)
  ├─ bits 41-26  - PSD (16-bit)
  ├─ bits 25-16  - Fine Timestamp (10-bit)
  └─ bits 15-0   - Energy (16-bit)

Word 2+: [User Words] (optional, variable length)
  ├─ bit  63     - Last Word flag (0x1 on final user word)
  └─ bits 62-0   - User data (63-bit)
  → AMax値、Baseline等がここに格納される

Wave Header: (if bit 62 of Word 1 is set)
  ├─ bit  63     - Truncated flag
  ├─ bits 62-12  - Reserved
  └─ bits 11-0   - Waveform word count (N/4 samples)

Wave Data:
  4 samples per 64-bit word (16-bit each)
  [sample3:48-63][sample2:32-47][sample1:16-31][sample0:0-15]
```

### タイムスタンプ計算

```
timestamp_ns = (coarse_time × 8) + (fine_time / 1024 × 8)
```
- Coarse: 48-bit, 1 LSB = 8 ns
- Fine: 10-bit, 1/1024 interpolation

### Start/Stop シグナル

**Start Run (4 words):**
```
Word 0: [0x3][0x0][reserved][0x3][0x4]
Word 1: [0x2][reserved][dec.factor][n.traces][acq.width]
Word 2: [0x1][reserved][channel mask 31:0]
Word 3: [0x1][reserved][channel mask 63:32]
```

**Stop Run (3 words):**
```
Word 0: [0x3][0x2][reserved][0x2][0x3]
Word 1: [0x0][reserved][timestamp (1 LSB = 8 ns)]
Word 2: [0x1][reserved][dead time (1 LSB = 8 ns)]
```

---

## 設定パラメータ

### トラペゾイダルフィルタ

| パラメータ | アドレス | 説明 |
|-----------|---------|------|
| `TRAP_K` | 0x05 | Rise time (サンプル数) |
| `TRAP_M` | 0x06 | Decay time (サンプル数) |
| `DECONV_M` | 0x07 | デコンボリューション係数 |
| `TRAP_GAIN` | 0x08 | デジタルゲイン |

### AMax パラメータ

| パラメータ | アドレス | 説明 |
|-----------|---------|------|
| `WINDOW_MAXIM` | 0x14000 | 最大値検索ウィンドウ |
| `AMAX_window` | 0x160003 | AMax 測定ウィンドウ |
| `AMAX_delay` | 0x160004 | AMax 遅延アライメント |
| `AMAX_len` | 0x160005 | AMax 測定長 |

### トリガー/CFD

| パラメータ | アドレス | 説明 |
|-----------|---------|------|
| `THRS` | 0x02 | トリガー閾値 |
| `TRIG_K` | 0x03 | 高速トリガー rise time |
| `TRIG_M` | 0x04 | 高速トリガー decay time |

### ベースライン

| パラメータ | アドレス | 説明 |
|-----------|---------|------|
| `BL_LEN` | 0x09 | ベースライン平均長 |
| `BL_INIB` | 0x0A | ベースライン初期化 |
| `baseline_delay` | 0x160000 | ベースライン計算遅延 |
| `baseline_offset` | 0x160002 | ベースラインオフセット |

---

## 実装計画

### Phase 1: デコーダコア ✅ 完了

**目標:** `src/reader/decoder/amax.rs` 実装

- [x] `AMaxConfig` 構造体
- [x] `AMaxDecoder` 構造体
- [x] `AMaxEventData` 構造体 (EventData + amax_value, baseline)
- [x] イベントデコード (4 words → AMaxEventData)
- [x] タイムスタンプ計算 (coarse + fine time, 1 LSB = 8ns)
- [x] Start/Stop シグナル検出
- [x] User Words 解析 (AMax値、ベースライン)
- [x] 波形デコード対応
- [x] 単体テスト (7 テスト pass)

**実装済みファイル:**
- `src/reader/decoder/amax.rs` - AMax デコーダ
- `src/reader/decoder/mod.rs` - モジュール登録
- `src/bin/amax_data_test.rs` - データ取得テスト

### Phase 2: Reader 統合 (進行中)

**目標:** Reader で AMax デコーダを使用可能に

- [ ] `DecoderKind::AMax` 追加
- [ ] `SourceType::AMax` 追加
- [ ] `from_config()` マッピング
- [x] DIG2 プロトコル使用 (`dig2://` URL)

### Phase 3: 設定管理 ✅ 完了

**目標:** AMax 固有パラメータの設定

- [x] `config/digitizers/amax_test.json` 作成
- [x] レジスタマップ (C++ リファレンス参照)
- [ ] パラメータ適用ロジック

### Phase 4: 実機検証 (部分完了)

**目標:** VX2730 + AMax FW での動作確認

- [x] ファームウェアアップロード (CUP ファイル)
- [x] パラメータ設定・読み出し (20/22 レジスタ成功)
- [x] データ取得テスト (`amax_data_test` で確認)
- [ ] ヒストグラム表示確認
- [ ] イベントレート測定

**実機テスト結果 (2026-01-29):**
```
Total bytes: 12864, Events: 5
Event structure: 4 words (Header + Data + 2 User Words)
User Word 0x3D70 = 15728 (baseline or AMax value)
```

---

## PSD2 との差異

| 項目 | PSD2 | AMax |
|------|------|------|
| イベントサイズ | 16 bytes (2 words) | 32 bytes (4 words) |
| エネルギー | Short + Long gate | トラペゾイダルフィルタ出力 |
| 追加データ | なし | AMax 値、ベースライン |
| シグナル処理 | 標準 DPP-PSD | カスタム MCA |
| プロトコル | DIG2 | DIG2 (同じ) |

---

## ファイル構成

```
src/reader/decoder/
├── mod.rs          # AMax 追加
├── amax.rs         # 新規: AMax デコーダ
└── common.rs       # EventData 拡張 (amax_value, baseline)

config/digitizers/
└── amax_test.json  # 新規: AMax テスト設定

docs/
└── amax_decoder_spec.md  # 新規: 仕様書
```

---

## 参考資料

- `TrapezoidalfilterMCA_dpp_tests/` - 最新ファームウェア
- `legacy/DELILA2/lib/digitizer/` - C++ リファレンス実装
- `TrapezoidalfilterMCA_dpp_tests/test_param27012026_caenlist.txt` - 設定例
- `TrapezoidalfilterMCA_dpp_tests/output/output/RegisterFile.json` - レジスタファイル

---

## 注意事項

1. **タイポ保持:** レジスタ名 (`M_lenght`, `shapung_len`, `offsett`) は FPGA 互換性のため元のスペルを保持
2. **ビッグエンディアン:** デジタイザ出力はビッグエンディアン、デコーダでリトルエンディアンに変換
3. **波形:** ほとんどのイベントは波形なし (32 bytes)、波形フラグで判定
4. **Fine Time:** 1/1024 精度でサブナノ秒タイムスタンプ
