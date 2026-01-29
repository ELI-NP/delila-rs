# Current Sprint - TODO Index

**Updated:** 2026-01-29 (Gemini レビュー改善完了)

このファイルは現在のスプリントの概要を示すインデックスです。
Claudeセッション開始時に必ず読み込まれます。

---

## ~~最優先: PSD2 デコーダ バグフィックス~~ ✅ 完了 (2026-01-26)

**Linux移行後の実機検証で発見されたバグ** → `TODO/archive/phase3_psd_decoders/00_psd2_decoder_bugfix.md`

C++ リファレンス (`external/caen-dig2/src/endpoints/dpppsd.cpp`) との比較で
以下の重大バグを確認:

1. **[P0] Single-word event 未対応** - 高レート時にデコーダがデシンクしてデータ全損
2. **[P0] Special event 未フィルタ** - 統計イベントが物理データに混入
3. **[P1] STOP シグナル無視** - ハードウェア停止が検出されない
4. **[P2] FLAGS マスク誤り** - flag_low_priority が 11bit (正しくは 12bit)
5. **[P2] Waveform 欠落** - convert_event() が波形データを落としている

**実機確認済み:**
- ハードウェア接続: OK (VX2730, dig2://172.18.4.56)
- データ読み出し: OK (9000イベント/テスト)
- タイムスタンプ: 正常 (10μs間隔)
- energy=0: ゲートパラメータ未設定が原因 (psd2_test.json 適用で解消見込み)

---

## ~~PSD2 実機動作確認~~ ✅ 完了 (2026-01-26)

- DAQフルパイプライン動作確認: Reader → Merger → Recorder → Monitor (10kHz)
- Operator REST API 経由で Configure → Arm → Start → Running 遷移
- ch4 パルサー信号でヒストグラム表示確認 (energy ≈ 34, bin[2])
- Angular UI (port 4200) + Monitor API (port 8081) 動作確認

**ヒストグラム表示バグ修正** (2026-01-26):
- bin[2] (16.7Mカウント) がチャート上で欠落する問題を修正
- 原因: 4096バーをサブピクセル幅 (~0.2px) で描画 → ECharts large-mode で隣接バーに上書きされピーク消失
- 対策: max-value ダウンサンプリング (ROOT TH1::Draw() と同じアプローチ) + largeThreshold 引き上げ
- 修正ファイル: `web/operator-ui/src/app/components/histogram-chart/histogram-chart.component.ts`

**設定ファイル:**
- `config/config_psd2_test.toml` - 実機テスト用 (ChSelfTrigger)
- `config/digitizers/psd2_test.json` - デジタイザ設定 (ch4有効, threshold=1000)

---

## ~~PSD1 デコーダ実装~~ ✅ 全Phase完了 (2026-01-26)

**仕様書:** `docs/psd1_decoder_spec.md`
**実装計画:** `TODO/archive/phase3_psd_decoders/17_psd1_decoder_implementation.md`
**ハードウェア:** DT5730B (Serial: 990, DPP-PSD1, USB, 8ch, 14-bit, 500 MS/s)

### Phase 1: デコーダコア ✅ → Phase 2: Reader統合 ✅ → Phase 3: 実機検証 ✅

**Phase 1 完了 (2026-01-26):** 46 テスト pass, Board/Channel/Event の 3 層デコーダ実装
**Phase 2 完了 (2026-01-26):** DecoderKind enum dispatch, from_config() mapping, Arm=Start 対応
**Phase 3 完了 (2026-01-26):** 実機検証成功 — 14/14 パラメータ適用, ~10.4k evt/s, ヒストグラム表示確認

Phase 3 で修正した主な課題:
1. DIG1 endpoint: DATA+SIZE のみ (N_EVENTS 除外)
2. START_MODE_SW: Arm コマンドを Start フェーズで実行
3. Watch channel 状態スキップ: `(_, Running)` パターンで対応
4. PSD1 パラメータ値フォーマット: ポラリティ/extras/self_trg の値マッピング

---

## ~~PHA1 デコーダ実装 + マルチデジタイザ統合テスト~~ ✅ 完了 (2026-01-29)

**仕様書:** `docs/pha1_decoder_spec.md`
**テスト計画:** `TODO/17_pha1_pipeline_test.md`
**ハードウェア:** DT5730B (Serial: 990, DPP-PHA1, USB, 8ch, 14-bit, 500 MS/s)

### PHA1 デコーダ実装 ✅

- PSD1 ベースで PHA1 固有の差分を適用
- 46 テスト pass
- DIG1 プロトコル対応 (`dig1://` URL スキーム)
- START_MODE_SW: Arm = Start

### Phase 1: 単一マシン PHA1 テスト ✅

**実行環境:** 172.18.4.147 (Linux)
- Reader (PHA1): 29,931 events @ 903 evt/s
- Merger → Recorder → Monitor 全動作確認
- 出力ファイル: 513 KB

### Phase 2: マルチマシン統合テスト (PSD2 + PHA1) ✅

**構成:**
- Machine A (macOS): PSD2 Reader (VX2730) + Merger + Recorder + Monitor + Operator
- Machine B (Linux): PHA1 Reader (DT5730B)

**結果:**
- psd2-local: 239,348 events @ 8,977 evt/s
- pha1-remote: 29,746 events @ 903 evt/s
- ネットワーク越し ZMQ 通信: ✅ 正常動作
- 出力ファイル: 4.6 MB

**修正点:** PSD2 ch16 polarity を Positive → Negative に修正

---

## ~~データ出力検証 (Task B)~~ ✅ 完了 (2026-01-26)

→ `TODO/archive/phase4_data_verification/18_data_verification.md`

- E2E テスト: 4テスト全パス (flags に per-event XOR チェックサム, シード付き乱数)
- `recover validate`: emulator データ 59,560,000 イベント Valid
- `recover dump`: flat binary 変換成功 (22 bytes/event, サイズ整合確認)
- `macros/read_dump.C`: legacy Recorder 互換 TTree (DELILA_Tree)

## ~~データ完全性 & 波形モード堅牢性~~ ✅ 完了 (2026-01-28)

→ `TODO/20_data_integrity_and_performance_audit.md`

**Phase A (データ完全性):** bounded channel, try_send retry, record_drop(), EOS ハンドリング
**Phase B (パフォーマンス):** move セマンティクス, shrink_to_fit(), Monitor owned
**Phase E (波形堅牢性):** Stop ハング修正, DecodeLoop サイレントクラッシュ修正, SIGBUS 修正 (64MB バッファ事前確保)

**実機検証結果:**
- 波形なし 98 kHz: 安定動作, queue=0, Reader-Recorder イベント数一致
- 波形あり 1 kHz: 安定動作, queue=0
- 波形あり 10 kHz: 安定動作, queue=0, ~9,800 Hz, Stop 正常
- 波形あり帯域上限: ~6.6 kHz (1 GbE 飽和, ハードウェア制約)

---

## Phase 6 — デジタイザ設定 UI — 実装完了 / ⚠️ ユーザー検証未実施

→ `TODO/19_settings_ui.md`

**状態:** 実装は完了しているが、**ユーザーによる動作確認が一切行われていない**（画面表示すら未確認）。
実機での検証が必要。

**実装内容:**
1. Reader Detect コマンド (FELib一時接続 → DeviceInfo取得 → 切断)
2. MongoDB スキーマ拡張 (serial_number, model + serial検索)
3. REST API 拡張 (POST /api/digitizers/detect, GET /api/digitizers/by-serial/:serial)
4. Angular チャンネルテーブルコンポーネント (横スクロール, sticky列, override ハイライト)
5. digitizer-settings 3タブ化 (Board / Frequent / Advanced)
6. config expand/compress ロジック (defaults+overrides ↔ flat per-channel)

## Event Builder (C++ 別リポジトリ) — 設計完了 / 実装待ち

**仕様書:** `TODO/event-builder/SPECIFICATION.md` (v0.3.0)
**ワイヤフォーマット:** `docs/event_bridge_wire_format.md`
**Bridge 実装計画:** `TODO/event-builder/IMPLEMENTATION.md`

### 概要

核物理実験のコインシデンスイベント構築を C++ で実装する。
delila-rs パイプラインとは Event Bridge (Rust) 経由で接続。

```
Merger ──PUB (MsgPack)──▶ Event Bridge (Rust) ──PUB (固定バイナリ)──▶ Event Builder (C++)
                          src/bin/event_bridge.rs                     別リポジトリ (CMake + ROOT)
```

### 決定事項

- **言語:** C++ (ROOT/THttpServer との親和性、物理解析コミュニティの共通言語)
- **リポジトリ:** 別リポ (ビルドシステムが完全に異なる: CMake + ROOT vs Cargo)
- **通信:** ZeroMQ PUB/SUB, 固定バイナリ 14 bytes/hit (パディングなし)
- **Event Bridge:** Rust 側の変換ブリッジ — 実装済み (`src/bin/event_bridge.rs`)

### C++ Event Builder の責務

1. Event Bridge の PUB を SUB (tcp://localhost:5600)
2. ヒットデータの時間ソート + バッファリング
3. コインシデンスウィンドウ (±500 ns) によるイベント構築
4. ROOT ファイル出力 (TTree)
5. THttpServer によるヒストグラム Web 公開

### 次のステップ

- [ ] C++ リポジトリ作成 (CMakeLists.txt + cppzmq + ROOT)
- [ ] Hit 受信 + デシリアライズ実装
- [ ] タイムスライス + コインシデンスアルゴリズム実装
- [ ] ROOT TTree 出力
- [ ] THttpServer ヒストグラム

---

## HV Gain Matcher (Python Tool) — ✅ Phase 1 完了 (2026-01-28)

→ `tools/hv_calibration/`

PMT ゲインマッチング自動化ツール。CAEN SY5527 HV 電源を制御し、
DAQ ヒストグラムのピーク位置を目標 ADC チャンネルに揃える。

### 実装状況

| Phase | 内容 | 状態 |
|-------|------|------|
| 1 | HV 接続 + status コマンド | ✅ 完了 (V0Set READ/WRITE 検証済み) |
| 2 | DAQ クライアント + フィッター | ✅ 実装済み (DAQ 連携未テスト) |
| 3 | match ループ | ✅ 実装済み (実運用未テスト) |
| 4 | 実運用テスト | 📋 結線完了後 |

### 検証結果 (2026-01-28)

- SY5527 接続: ✅ (CAENHVWrapper via ctypes)
- 全スロット読み出し: ✅ (10 スロット × 12ch = 120ch)
- V0Set 書き込み: ✅ (955V → 900V → 955V 復元確認)
- SVMax 読み出し: ✅ (GECO2020 設定値、書き込み時自動クランプ)

### 実行方法

```bash
cd /path/to/hv_calibration
LD_LIBRARY_PATH=/snap/caengeco2020/x1/lib:/usr/lib64 \
python3 gain_matcher.py status --config examples/gain_config.yaml
```

---

## 次のセッション

- ~~**A:** Multi-digitizer 統合テスト (PSD1 + PSD2)~~ → ✅ PSD2 + PHA1 で完了 (2026-01-29)
- **B:** C++ Event Builder リポジトリ作成 + 基本受信テスト
- **C:** HV Gain Matcher DAQ 連携テスト (結線完了後)
- **D:** Phase 10: Angular UI の rust-embed 統合
- **E:** PHA1 パラメータマッピング改善 (現在一部パラメータエラーあり)

---

## Active Tasks

| Priority | File | Status | Summary |
|----------|------|--------|---------|
| 1 | [event-builder/SPECIFICATION.md](event-builder/SPECIFICATION.md) | **設計完了** | C++ Event Builder 仕様 + Event Bridge 実装済み |
| 2 | [19_settings_ui.md](19_settings_ui.md) | **⚠️ ユーザー検証未実施** | Phase 6: デジタイザ設定 UI (実装済み・動作未確認) |
| 3 | [21_gemini_review_improvements.md](21_gemini_review_improvements.md) | **✅ 完了** | Gemini レビュー改善 (堅牢性+設計+パフォーマンス) |
| 4 | [17_pha1_pipeline_test.md](17_pha1_pipeline_test.md) | **✅ 完了** | PHA1 実装 + マルチデジタイザ統合テスト (PSD2+PHA1) |
| 5 | [20_data_integrity_and_performance_audit.md](20_data_integrity_and_performance_audit.md) | **✅ 完了** | データ完全性 + パフォーマンス + 波形堅牢性 (Phase A+B+E) |
| 5 | [15_digitizer_implementation.md](15_digitizer_implementation.md) | **✅ Phase 5 完了** | VX2730 (PSD2) + DT5730B (PSD1/PHA1) 実機動作確認済み |
| 6 | [11_operator_web_ui.md](11_operator_web_ui.md) | **In Progress** | Operator Web UI (Angular + Material) |
| - | [16_linux_migration_checklist.md](16_linux_migration_checklist.md) | Reference | Linux移行チェックリスト |

---

## Digitizer Implementation (2026-01-23~)

**Spec:** `docs/digitizer_system_spec.md`
**Target:** VX2730 (DPP-PSD2) via Ethernet (`dig2://`)

### Phases

| Phase | Description | Status |
|-------|-------------|--------|
| 1 | FELib Connection Layer | ✅ Complete |
| 2 | DevTree Read/Write | ✅ Complete |
| 3 | Config Storage & Apply (MongoDB) | ✅ Complete |
| 4 | Data Acquisition | ✅ Complete |
| 5 | Reader + Master/Slave + PSD1 | ✅ Complete (Master/Slave ✅, PSD1 全Phase ✅) ← **MVP完了ライン** |
| 6 | Web UI Settings | ✅ Complete |
| 7 | Future (Templates, Monitoring) | Future |

### Principles
- **KISS:** 最小限の抽象化、動くコードを最短経路で
- **TDD:** テストファーストで実装
- **Clean Architecture:** 依存は内向き（KISSと競合時はKISS優先）

---

## Completed Features (Summary)

- Emulator + ZMQ pipeline
- Merger (zero-copy forwarding)
- Recorder (sorting + file format v2)
- Monitor (Web UI + REST API + ECharts histogram/waveform)
- Operator (control system + pipeline ordering)
- MongoDB統合 (Run履歴、Comment永続化、Notes logbook)
- Source Config Management (SourceType enum, config_file, RuntimeConfig)
- Metrics API + RateTracker
- PSD2 デコーダ バグフィックス (single-word event, special event, STOP signal, flags, waveform)
- PSD2 実機動作確認 (VX2730, ch4 パルサー 10kHz)
- ヒストグラム表示修正 (max-value downsampling for sub-pixel bar rendering)
- PSD1 デコーダ実装 + Reader統合 + DT5730B 実機検証 (10kHz パルサー, 全パラメータ適用成功)
- データ出力検証 (E2E テスト + recover validate/dump + ROOT マクロ)
- Event Bridge (MessagePack → 固定バイナリ変換, C++ Event Builder 向け)
- データ完全性修正 (bounded channel, try_send retry, record_drop, EOS ハンドリング)
- パフォーマンス修正 (move セマンティクス, shrink_to_fit, Monitor owned)
- 波形モード堅牢性修正 (SIGBUS 修正, Stop ハング修正, DecodeLoop エラーハンドリング)
- 波形取り込み実機検証 (1 kHz / 10 kHz, 1 GbE 帯域制限 ~6.6 kHz 確認)
- HV Gain Matcher Python ツール (SY5527 ctypes ラッパー, V0Set READ/WRITE 検証済み)
- PHA1 デコーダ実装 (DIG1 プロトコル, START_MODE_SW 対応)
- マルチデジタイザ統合テスト (PSD2 + PHA1, マルチマシン ZMQ 通信)
- Gemini レビュー改善 (f64 unwrap 安全化, SystemState 優先順位修正, Vec 再利用, Monitor 軽量化)

---

## Archived

| Directory | Contents |
|-----------|----------|
| `archive/phase1_basic_pipeline/` | 基本パイプライン設計 |
| `archive/phase1_components/` | CLIリファクタリング、CAEN FFI、Monitor、Merger |
| `archive/phase1_control_system/` | コントロールシステム設計 |
| `archive/phase2_infrastructure/` | タイムスタンプソート、Metrics API、Source設定管理 |
| `archive/phase3_psd_decoders/` | PSD2 バグフィックス、PSD1 デコーダ実装+実機検証 |
| `archive/phase4_data_verification/` | データ出力検証 (E2E テスト、recover dump、ROOT マクロ) |

---

## Notes

- **MVP目標:** 2026年3月中旬
- **現在のフェーズ:** Phase 6 実装完了 / Event Builder 設計完了 (C++ 実装待ち) / PSD2 + PHA1 マルチデジタイザ統合テスト完了 / HV Gain Matcher Phase 1 完了
- **実機確認済み:**
  - VX2730 (Serial: 52622, DPP_PSD2, 32ch, Ethernet)
  - DT5730B (Serial: 990, DPP_PSD1/PHA1, 8ch, USB)
- **マルチマシン構成確認済み:** macOS (PSD2) + Linux (PHA1) ネットワーク越し ZMQ 通信
- **動作環境:** Linux (Ubuntu, Rust 1.93.0) + macOS (クロスマシン統合)

## Reference Documents

| Document | Location | Priority |
|----------|----------|----------|
| **x2730 DPP-PSD CUP Documentation** | `legacy/documentation_2024092000-2/` | ★★★ |
| FELib User Guide | `legacy/GD9764_FELib_User_Guide.pdf` | ★★ |
| Digitizer System Spec | `docs/digitizer_system_spec.md` | ★★★ |
| Event Bridge Wire Format | `docs/event_bridge_wire_format.md` | ★★ |
| Event Builder Spec | `TODO/event-builder/SPECIFICATION.md` | ★★★ |
