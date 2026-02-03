# Current Sprint - TODO Index

**Updated:** 2026-02-03

このファイルは現在のスプリントの概要を示すインデックスです。
Claudeセッション開始時に必ず読み込まれます。

---

## Active Tasks

| Priority | File | Status | Summary |
|----------|------|--------|---------|
| **1** | [19_settings_ui.md](19_settings_ui.md) | **🔄 実機検証中** | Phase 6: デジタイザ設定 UI |
| **2** | [22_amax_decoder_implementation.md](22_amax_decoder_implementation.md) | **🔄 実装中** | AMax カスタムファームウェア デコーダ |
| **3** | [25_apply_digitizer_via_zmq.md](25_apply_digitizer_via_zmq.md) | **📋 計画中** | Apply Digitizer Config via ZMQ |
| **4** | [24_l2_filter_implementation.md](24_l2_filter_implementation.md) | **📋 計画中** | L2 Filter (Counter/Flag/Accept) |
| - | [event-builder/SPECIFICATION.md](event-builder/SPECIFICATION.md) | **参照** | Event Builder 仕様 |

---

## Phase 6 — デジタイザ設定 UI — 🔄 実機検証中

→ `TODO/19_settings_ui.md`

**状態:** 実機 VX2730 (172.18.4.57) で検証中。バックエンド修正完了、UI 動作確認中。

**残タスク:**
- [ ] Angular UI でのタブ表示確認 (Board / Frequent / Advanced)
- [ ] Apply → Configure E2E テスト (UI → API → Reader → FELib)
- [ ] Save → JSON ファイル書き出し確認

---

## AMax カスタムファームウェア デコーダ — 🔄 実装中

→ `TODO/22_amax_decoder_implementation.md`

Phase 1 (デコーダコア) + Phase 3 (設定管理) 完了。Phase 2 (Reader 統合) と Phase 4 (実機検証) が進行中。
`tools/amax_viewer` で開発・検証ツールを提供。

---

## 次のセッション候補

- **B'':** L2 Filter 実装 (Counter/Flag/Accept) → `TODO/24_l2_filter_implementation.md`
- **C:** HV Gain Matcher DAQ 連携テスト (結線完了後)
- **D:** Angular UI の rust-embed 統合
- **E:** PHA1 パラメータマッピング改善
- **F:** Event Builder の実データ検証 (C++ ELIFANT-Event との出力比較)

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
- PSD2 デコーダ + 実機動作確認 (VX2730, 10kHz)
- PSD1 デコーダ + DT5730B 実機検証
- PHA1 デコーダ + マルチデジタイザ統合テスト (PSD2 + PHA1, マルチマシン ZMQ)
- データ出力検証 (E2E テスト + recover validate/dump + ROOT マクロ)
- データ完全性 + パフォーマンス + 波形堅牢性修正
- HV Gain Matcher Python ツール (SY5527, Phase 1 完了)
- Rust Event Builder (L1 完了, 67 tests, 206M hits in ~5s)
- Gemini レビュー改善

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
| `archive/phase5_decoders_and_testing/` | PHA1 デコーダ、マルチデジタイザ統合テスト |
| `archive/phase5_audit_and_review/` | データ完全性監査、Gemini レビュー改善 |
| `archive/11_operator_web_ui.md` | Operator Web UI (Phase 6 に統合) |
| `archive/15_digitizer_implementation.md` | デジタイザ実装 Phase 1-5 |
| `archive/16_linux_migration_checklist.md` | Linux移行チェックリスト |
| `archive/23_event_builder_implementation.md` | Event Builder L1 実装 (Time Slice 方式) |

---

## Notes

- **MVP目標:** 2026年3月中旬
- **現在のフェーズ:** Phase 6 実機検証中 + AMax デコーダ開発中
- **実機確認済み:**
  - VX2730 (Serial: 52621, DPP_PSD2, 32ch, Ethernet, 172.18.4.57)
  - DT5730B (Serial: 990, DPP_PSD1/PHA1, 8ch, USB)
- **動作環境:** Linux (Ubuntu, Rust 1.93.0) + macOS (クロスマシン統合)

## Reference Documents

| Document | Location | Priority |
|----------|----------|----------|
| **x2730 DPP-PSD CUP Documentation** | `legacy/documentation_2024092000-2/` | ★★★ |
| FELib User Guide | `legacy/GD9764_FELib_User_Guide.pdf` | ★★ |
| Digitizer System Spec | `docs/digitizer_system_spec.md` | ★★★ |
| Event Bridge Wire Format | `docs/event_bridge_wire_format.md` | ★★ |
| Event Builder Spec | `TODO/event-builder/SPECIFICATION.md` | ★★★ |
