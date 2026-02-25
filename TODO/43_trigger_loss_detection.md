# #43 トリガーロス検出 — DIG1 EXTRAS フラグ活用

**Date:** 2026-02-25
**Status: COMPLETED** (Phase 1+2+本番実機検証)
**Priority:** 2
**Design doc:** [docs/plans/trigger_loss_detection.md](../docs/plans/trigger_loss_detection.md)

---

## 背景

DIG1 (DT5730B/VX1730B) の ReadLoop transient error retry (10ms) 中にトリガーが失われる可能性がある。
DIG1 には DIG2 のような統計エンドポイント (`/endpoint/stats`) が存在しないため、
**イベントデータ内の EXTRAS フラグ**からトリガーロス情報を取得する。

**設定変更ゼロ**: 現在強制している `ch_extras_opt=2` (EXTRAS_OPT_TT48_FLAGS_FINETT) のまま使える。

---

## EXTRAS Word 仕様 (UM4380 Rev.6, p.107-108)

`ch_extras_opt=2` (EX=010) の EXTRAS word (32-bit LE):

```
[31:16] = Extended Timestamp (48-bit TS の上位 16 bit)
[15:10] = Flags (6 bits):
  bit[15]: Trigger Lost     — ロスト直後の最初のイベントでセット
  bit[14]: Over-range        — ゲート内で ADC 飽和 (クリッピング)
  bit[13]: N trigger counted — N イベント保存ごとにハイ
  bit[12]: N lost trigger counted — N 回ロストごとにハイ
  bit[11:10]: (未使用)
[9:0]   = Fine Timestamp (T_fine, 10-bit)
```

### PSD1 デコーダでの flags マッピング

`decode_extras_word()` (psd1.rs:647) は `(word >> 10) & 0x3F` で抽出。
EXTRAS bits[15:10] → EventData.flags bits[5:0] にシフトされる:

| EXTRAS bit | EventData.flags bit | 定数     | 意味 |
|-----------|--------------------:|----------|------|
| bit[15]   | bit[5] = `0x20`     | TRIGGER_LOST | ロスト直後の最初のイベント |
| bit[14]   | bit[4] = `0x10`     | OVER_RANGE | ADC 飽和 |
| bit[13]   | bit[3] = `0x08`     | N_TRIGGER_COUNTED | N イベント保存ごと |
| bit[12]   | bit[2] = `0x04`     | N_LOST_COUNTED | N ロストごと |

**注:** Pileup は charge word から bit[15] (0x8000) に別途 OR される。flags bits[5:0] とは衝突しない。

### N 値設定 (UM4380 Rev.6, p.30)

レジスタ **DPP Algorithm Control 2** (0x1n84) の bits[17:16]:

| bits[17:16] | N 値 | 備考 |
|-------------|-----:|------|
| 00 | **1024** | デフォルト |
| 01 | 128 | |
| 10 | 8192 | |
| 11 | (未定義) | 1024 扱い |

レジスタアドレス: `0x1084 + ch × 0x100` (ch0=0x1084, ch1=0x1184, ..., ch15=0x1F84)

### 定量的ロスト数の計算

- **ロスト数推定 = bit[12] カウント × N**
- **保存数推定 = bit[13] カウント × N**
- **ロスト率 ≈ bit[12]_count / (bit[12]_count + bit[13]_count)**

2時間ラン (N=1024) での精度見積もり:

| レート | 総トリガー | bit[13] カウント | 統計精度 |
|--------|-----------|-----------------|---------|
| 1 kHz | 7.2M | ~7,000 | 0.01% |
| 10 kHz | 72M | ~70,000 | 0.004% |
| 100 kHz | 720M | ~703,000 | 0.001% |

---

## Phase 1: テストバイナリ `trigger_loss_test`

### 目的

EXTRAS フラグが実際に機能することをハードウェアで検証する。
意図的にバッファ溢れを起こし、フラグが立つことを確認。

### 設計

**ファイル:** `src/bin/trigger_loss_test.rs` (~250行)

**CLI:**
```
trigger_loss_test <URL> <CONFIG_JSON> [--phase1-secs 10] [--phase2-secs 30] [--delay-ms 1000]
```

**テスト手順:**

1. 本番 JSON コンフィグを読み込んで適用 (`DigitizerConfig::load` + `handle.apply_config`)
2. レジスタ 0x1n84 bits[17:16] から N 値を読み取り表示
3. **Phase 1 (10s)**: 待機なしで連続読み出し
   - イベントレートのベースライン取得
   - bit[15] (Trigger Lost) = 0 であることを確認
   - bit[13] (N trigger counted) × N ≈ total_events であることを検証
4. Stop → cleardata → Re-arm → Re-start
5. **Phase 2 (30s)**: read_data の間に 1秒 sleep 挿入
   - バッファ溢れによりトリガーロスト発生
   - bit[15] > 0 を確認
   - bit[12] > 0 を確認 (ロスト数が N を超える場合)
   - レジスタ 0x8104 bit[4] (Event Full) の検出
6. Phase 1 と Phase 2 の比較サマリー出力

**再利用する既存コード:**

| 用途 | ファイル | API |
|------|---------|-----|
| 設定読み込み | `src/config/digitizer.rs:592` | `DigitizerConfig::load(path)` |
| ハンドル | `src/reader/caen/handle.rs` | `CaenHandle::open`, `apply_config`, `configure_endpoint` |
| レジスタ読み | `src/reader/caen/handle.rs:449` | `get_user_register(addr)` |
| RAW 読み出し | `src/reader/caen/handle.rs:1133` | `EndpointHandle::read_data(timeout, buf)` |
| デコーダ | `src/reader/decoder/psd1.rs:190,235` | `Psd1Decoder::new(config)`, `.decode(&raw)` |
| イベント構造体 | `src/reader/decoder/common.rs:93` | `EventData` (flags フィールド) |

**参考パターン:** `src/bin/psd1_timing_test.rs` (DIG1 テストバイナリの実装パターン)

### 期待出力

```
=== DIG1 Trigger Loss Flag Test ===
URL: dig1://caen.internal/usb?link_num=0
Config: config/digitizers/psd1_test.json

--- N Lost Trigger Settings ---
  ch0-15: N = 1024 (register 0x1n84 bits[17:16] = 00)

=== Phase 1: Normal readout (10s) ===
  Ch   Events  TrigLost  OverRange  NTrigCnt  NLostCnt  Est.Lost
  4     50234         0          3        49         0         0
  Rate: 5023 Hz | Buffer Full: 0

=== Phase 2: Delayed readout (30s, 1000ms delay) ===
  Ch   Events  TrigLost  OverRange  NTrigCnt  NLostCnt  Est.Lost
  4     12034        22          1        11         8      8192
  Rate: 401 Hz | Buffer Full: 22

RESULT: Trigger loss flags working correctly.
```

### 注意事項

- Stop フロー: `disarmacquisition` → drain → `cleardata` (reset 禁止、disarm を先にしないと drain が終わらない)
- DIG1 START_MODE_SW: `armacquisition` = arm+start (`swstartacquisition` は DIG2 専用)
- バッファサイズ: 64 MB (既存テストバイナリと同じ)
- PHA1 対応: flags 配置は PSD1 と同一。将来的に FW 判定で Pha1Decoder 切り替え可能
- Waveform なし (12 bytes/event) では 10kHz × 5s delay でもバッファ溢れしない → waveform 有効が必要

### 実機検証結果 (2026-02-25, DT5730B SN:990)

**条件:**
- URL: `dig1://caen.internal/usb?link_num=0`
- Config: `config/digitizers/psd1_test.json` (waveform 有効, record_length=1000)
- ホスト: 172.18.4.147 (Linux)
- パルサー入力: ch4, ~10kHz

**結果:**

| 項目 | Phase 1 (正常, 10s) | Phase 2 (1s delay, 30s) |
|------|-------------------|------------------------|
| イベントレート | 9,753 Hz | 1,798 Hz |
| 総イベント | 98,419 | 55,216 |
| Trigger Lost (bit[15]) | **0** | **26** |
| N Lost Counted (bit[12]) | **0** | **217** |
| 推定ロスト数 (bit[12]×1024) | 0 | **222,208** |
| Buffer Full 検出 | 0 | 27 |
| NTrigCnt sanity (events/est) | 1.02 | 0.20 |

**確認事項:**

1. bit[15] (Trigger Lost): ロスト直後の最初のイベントで正しくセット (**動作確認**)
2. bit[12] (N Lost Counted): N=1024 ロストごとに正しくセット (**動作確認**)
3. bit[13] (N Trigger Counted): Phase 1 で ratio≈1.02 (正常), Phase 2 で ratio=0.20 (80%ロスト) (**動作確認**)
4. 0x8104 bit[4] (Buffer Full): 毎 read で検出 (27/28 reads) (**動作確認**)
5. N カウンタリセット: Phase 1→2 間の Stop/Re-start でリセットされた (**確認済**)
6. Gemini 指摘 #2 (深い飽和時の bit[12]): バッファ 100% FULL でも bit[12] は正しくラッチされ、次の読み出しイベントでセットされた (**確認済**)

**結論:** DIG1 EXTRAS フラグによるトリガーロス検出は**設定変更ゼロで完全に動作する**。

---

## Phase 2: 本番デコーダへの統合 → **実装完了**

### 実装内容 (2026-02-25)

1. **ComponentMetrics 拡張** (`src/common/mod.rs`)
   - `trigger_loss_count: u64` — 累積ロスト数
   - `trigger_loss_rate: f64` — ロスト率 (%)
   - `#[serde(default)]` で後方互換

2. **DIG1 フラグリマッピング** (`src/reader/mod.rs: remap_dig1_flags()`)
   - Raw EXTRAS bits[15:10] → common::flags 定数にマッピング
   - `convert_event()` で firmware 判定して適用
   - `has_trigger_lost()` 等のヘルパーが正しく動作するように

3. **DIG1 DecodeLoop カウント** (`src/reader/mod.rs: decode_loop()`)
   - FLAG_TRIGGER_LOST (bit[15]) イベント数をカウント
   - FLAG_N_LOST_TRIGGER (bit[12]) × 1024 でロスト数を推定
   - 10秒ごとのレート制限 warn! ログ

4. **DIG2 ReadLoop ポーリング** (`src/reader/mod.rs: poll_dig2_counters()`)
   - 5秒間隔で ChRealtimeMonitor → ChTriggerCnt → ChSavedEventCnt をポーリング
   - 24-bit ラップアラウンド対応 (delta 方式)
   - enabled_channels のみをポーリング (DigitizerConfig から取得)
   - 10秒ごとのレート制限 warn! ログ

5. **フロントエンド表示** (`web/operator-ui/.../status-panel.component.ts`)
   - Reader コンポーネントに LOSS: count (rate%) をオレンジ色で表示
   - ロストなし時は非表示

6. 将来: Grafana メトリクス (InfluxDB) にロスト率を送信

### 本番実機検証結果 (2026-02-25, 172.18.4.76)

**構成:** 5x VX1730B (PSD1) + 1x VX2730 (PSD2), 60Co+252Cf Time alignment run (Run 156)

| デジタイザ | FW | レート | 総イベント | LOSS | LOSS率 | 備考 |
|-----------|-----|--------|-----------|------|--------|------|
| PSD2-57 | PSD2 | 19.5k eve/s | 17.26M | 618 | 0.004% | トリガーホールドオフによるリジェクト（正常動作） |
| PSD1-SN53 | PSD1 | 228.2k eve/s | 231.08M | 0 | 0% | |
| PSD1-SN56 | PSD1 | 1.59M eve/s | 1.52G | 882.7k | 0.06% | 高レートでの本物のトリガーロス |
| PSD1-SN69 | PSD1 | 1.66M eve/s | 1.55G | 0 | 0% | |
| PSD1-SN59 | PSD1 | 13.1k eve/s | 12.04M | 0 | 0% | |
| PSD1-SN54 | PSD1 | 467.1k eve/s | 423.20M | 0 | 0% | |

**システム全体:** 3.81M eve/s, 60.27 GB 記録, 6/6 Online

**A3818 光リンク:** 15分間で ~15回の transient timeout 発生 → 全て 1-2 attempt で自動復旧（ReadLoop retry + ドライバパッチの組み合わせが有効）

**所見:**
- **PSD2-57 の LOSS: 618** — 19.5k Hz で 0.004% はトリガーホールドオフによるリジェクト。ChTriggerCnt はホールドオフ中もカウントするが ChSavedEventCnt はカウントしない。統計誤差よりはるかに小さく実用上無視可能
- **PSD1-SN56 の LOSS: 882.7k (0.06%)** — 1.59M eve/s という高レートでの本物のトリガーロス。同程度レートの SN69 (1.66M, 0%) と差があるため、SN56 固有の問題（信号品質、ケーブル等）の可能性あり
- その他 4 台の PSD1 はロストゼロ
- **結論:** DIG1/DIG2 両方式とも本番環境で正しく動作することを確認

---

## Phase 3: DIG2 統計カウンタのポーリング → **テストバイナリ検証完了**

DIG2 は `ChTriggerCnt` / `ChSavedEventCnt` を `get_value()` で直接読める。
ReadLoop 内で数秒おきにポーリングし、ロスト数を計算。

### テストバイナリ `trigger_loss_test_dig2`

**ファイル:** `src/bin/trigger_loss_test_dig2.rs`

**CLI:**
```
trigger_loss_test_dig2 <URL> <CONFIG_JSON> [options]
  --phase1-secs N         Phase 1 duration (default: 10)
  --phase2-secs N         Phase 2 duration (default: 30)
  --delay-ms N            Phase 2 read delay (default: 1000)
  --record-length-ns N    Override record_length_ns
  --enable-waveform       Force waveform on (WaveTriggerSource=ChSelfTrigger)
```

**DIG1 との主な違い:**

| 項目 | DIG1 (trigger_loss_test) | DIG2 (trigger_loss_test_dig2) |
|------|------------------------|-------------------------------|
| 検出方法 | EXTRAS フラグデコード | ChTriggerCnt - ChSavedEventCnt |
| デコーダ | Psd1Decoder 必要 | 不要 (n_events で十分) |
| 精度 | 推定 (N×カウント) | 正確 (差分) |
| Start | armacquisition のみ | armacquisition + swstartacquisition |
| Endpoint | include_n_events=false | include_n_events=true |
| カウンタ読み | レジスタ 0x8104 | get_value() で ChRealtimeMonitor→ChTriggerCnt→ChSavedEventCnt |

**重要:** ChRealtimeMonitor を先に読む必要あり（FPGA カウンタのラッチトリガー）。

### 実機検証結果 (2026-02-25, VX2730 SN:52622)

**条件:**
- URL: `dig2://172.18.4.56`
- Config: `config/digitizers/psd2_56.json`
- Overrides: record_length_ns=8192, waveform=enabled
- ホスト: macOS (Ethernet 経由で直接アクセス)
- パルサー入力: ch16, ~10kHz

**結果:**

| 項目 | Phase 1 (正常, 10s) | Phase 2 (1s delay, 30s) |
|------|-------------------|------------------------|
| TriggerCnt (FPGA) | 101,951 | 306,724 |
| SavedEventCnt (FPGA) | 101,968 | 109,294 |
| **Lost (差分)** | **0** | **197,430** |
| **Loss rate** | **0.00%** | **64.37%** |
| Deadtime (ch16) | 195.6 ms | 77,798 ms |
| Total bytes | 1,078 MB | 48.6 MB |

**確認事項:**

1. ChTriggerCnt / ChSavedEventCnt: 正常時に差分=0 (**動作確認**)
2. 遅延読み出し時: 差分>0, Loss rate=64.37% (**動作確認**)
3. ChDeadtimeMonitor: Phase 2 で 78s (30s 中) = ほぼ常時ビジー (**動作確認**)
4. ChRealtimeMonitor ラッチ: カウンタ値が正しく更新される (**確認済**)
5. ch8 (トリガーなし): TriggerCnt=0, Lost=0 — 非活性チャンネルは影響なし (**確認済**)
6. 24-bit ラップ: テスト中はラップせず（10kHz×30s=300k < 16.7M）、安全策は実装済

**結論:** DIG2 の ChTriggerCnt/ChSavedEventCnt カウンタで**正確なトリガーロス検出が可能**。
本番 ReadLoop への統合は get_value() ポーリングを追加するだけ。

---

## レジスタ参照

| レジスタ | アドレス | 用途 | UM4380 ページ |
|---------|---------|------|-------------|
| DPP Algorithm Control 2 | 0x1n84 | bits[17:16] = N 値 | p.29-30 |
| Acquisition Status | 0x8104 | bit[4] = Event Full | p.45 |
| DPP Algorithm Control | 0x1n80 | bit[5] = Trigger Counting mode | p.27 |

---

## Gemini レビュー (2026-02-25)

**結果: Approved for implementation (Phase 1)**

### 指摘事項と対応

1. **bit[12] の量子化問題** (⚠️ 重要)
   - ロスト数 < N (デフォルト 1024) の場合、bit[12] は発火しない
   - しかし bit[15] (Trigger Lost) は発火する
   - **対応:** `bit[15] > 0 AND bit[12] == 0` → 「少量のロストあり (< N)」と解釈
   - テスト出力にこの判定ロジックを含める

2. **深い飽和時の bit[12] 動作** (⚠️ 要検証)
   - バッファが 100% FULL のとき、N 番目のロストイベントに bit[12] をタグできるか？
   - bit[12] が「次に書かれるイベントまでラッチされる」か「そのイベント限定」か不明
   - **対応:** Phase 2 のテストでこの動作を確認する。これがテストの主目的の一つ

3. **テストフェーズの段階化** (提案)
   - Phase 2a (10ms delay): 軽い負荷 → ロストなし確認
   - Phase 2b (500ms-1s delay): バッファ溢れ → ロスト検出
   - Phase 2c: 高速読み出し再開 → フラグのクリア確認
   - **対応:** 初回実装はシンプルに 2 フェーズ。結果を見て段階化を検討

4. **N カウンタのリセットタイミング** (要確認)
   - arm/disarm サイクルで N カウンタがリセットされるか？
   - Phase 1 → Phase 2 の間の Stop/Re-start でリセット確認
   - **対応:** テスト出力で Phase 間の初期状態を確認
