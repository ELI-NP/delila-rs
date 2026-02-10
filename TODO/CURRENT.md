# Current Sprint - TODO Index

**Updated:** 2026-02-09

このファイルは現在のスプリントの概要を示すインデックスです。
Claudeセッション開始時に必ず読み込まれます。

---

## Active Tasks

| Priority | File | Status | Summary |
|----------|------|--------|---------|
| **1** | [30_mvp_march_roadmap.md](30_mvp_march_roadmap.md) | **📋 計画中** | 3月MVP: PHA1統合 + EB オンライン化 + Grafana + 運用改善 |
| **2** | [26_multi_digitizer_scaling.md](26_multi_digitizer_scaling.md) | **📋 計画中** | 10+ デジタイザ対応スケーリング (A1, A3, C3 が MVP 候補) |
| **2** | [27_settings_ui_v2.md](27_settings_ui_v2.md) | **✅ 完了** | Settings UI v2 全Phase完了 (Phase 1-6)。PSD1/PHA1 ns変換含む |
| **3** | [24_l2_filter_implementation.md](24_l2_filter_implementation.md) | **📋 計画中** | L2 Filter — 3-4月実験では不要。将来タスク |
| - | [event-builder/SPECIFICATION.md](event-builder/SPECIFICATION.md) | **参照** | Event Builder 仕様 |

---

## 次のセッション候補

- **A:** ステータス並列化 + start_daq.sh 改善 + タイムアウト設定化 → Phase 1 基盤整備
- **B:** PHA1 コンフィグテンプレート + Settings UI パラメータ確認
- **C:** Event Builder オンラインパイプライン統合 (EB-1〜EB-4) → Phase 2
- **D:** Grafana モニタリング (Prometheus exporter + HV exporter) → Phase 3
- **E:** PHA1 実機テスト (ハードウェア確定後)
- **F:** フロントパネル信号伝搬の実機検証 (TrgOut/SyncOut/GPIO)

---

## Recently Completed (archived)

| File | Completed | Summary |
|------|-----------|---------|
| [archive/29_channel_registration.md](archive/29_channel_registration.md) | 2026-02-05 | Channel Registration: Monitor チャンネル事前登録 + 個別チャンネル名 |
| [archive/28_tuneup_mode.md](archive/28_tuneup_mode.md) | 2026-02-05 | Tune Up Mode: Waveform + ヒストグラム + パラメータ調整 (3-panel FullHD レイアウト) |
| [27_settings_ui_v2.md](27_settings_ui_v2.md) | 2026-02-04 | Settings UI v2: 6カテゴリ再編 + SetInRun対応 (Phase 1-4, Phase 6 残) |
| [archive/25_apply_digitizer_via_zmq.md](archive/25_apply_digitizer_via_zmq.md) | 2026-02-04 | Apply Digitizer Config via ZMQ — Idle/Configured で全適用 + Running で SetInRun のみ適用 |
| [archive/19_settings_ui.md](archive/19_settings_ui.md) | 2026-02-03 | Phase 6: デジタイザ設定 UI (Detect, チャンネルテーブル, Apply/Save) |

## Cancelled (archived)

| File | Reason |
|------|--------|
| [archive/22_amax_decoder_implementation.md](archive/22_amax_decoder_implementation.md) | FELib OpenDPP エンドポイントから直接イベント取得で十分。デコーダ不要 |

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
- Settings UI v2 (6カテゴリ + SetInRun + Apply via ZMQ)
- Apply Digitizer Config via ZMQ (Idle/Configured/Running 全対応)
- Tune Up Mode (Waveform + ヒストグラム + パラメータ 3-panel UI)
- Channel Registration (Monitor チャンネル事前登録 + 個別チャンネル名)
- AMax Viewer (スタンドアロン GUI ツール: 2D Histogram + Waveform + パラメータ調整 + ROOT出力)
- Monitor Quick Create (デジタイザ選択で全チャンネルビュー自動生成)
- Monitor レイアウト永続化 (Operator REST API + ファイル保存, 全ブラウザ共有)

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
| `archive/19_settings_ui.md` | デジタイザ設定 UI Phase 6 (Detect, チャンネルテーブル, Apply/Save) |
| `archive/22_amax_decoder_implementation.md` | AMax デコーダ (Cancelled: OpenDPP で十分) |
| `archive/25_apply_digitizer_via_zmq.md` | Apply Digitizer Config via ZMQ |
| `archive/28_tuneup_mode.md` | Tune Up Mode 実装 |
| `archive/29_channel_registration.md` | Channel Registration + チャンネル名 |

---

## Known Issues / Future Work

- **TIME_STEP_NS ハードコード:** `src/config/digitizer.rs` の `TIME_STEP_NS = 2` は DT5730 (500 MS/s) 固定。DT5725 (250 MS/s → 4 ns/sample) など別サンプリングレートのデジタイザに対応する場合、`DeviceInfo.sampling_rate_sps` から動的計算するか、`DigitizerConfig` にサンプリングレートを持たせる必要あり。

## Notes

- **MVP目標:** 2026年3月中旬
- **現在のフェーズ:** MVP 最終段階 — 主要機能は全て実装済み。残りはスケーリング改善、実機検証
- **実機確認済み:**
  - VX2730 (Serial: 52621, DPP_PSD2, 32ch, Ethernet, 172.18.4.57)
  - DT5730B (Serial: 990, DPP_PSD1/PHA1, 16ch, USB)
  - 5x VX1730B (PSD1, 16ch each) + 1x VX2730 (PSD2, 32ch) on 172.18.4.76
- **動作環境:** Linux (Ubuntu, Rust 1.93.0) + macOS (クロスマシン統合)

## Reference Documents

| Document | Location | Priority |
|----------|----------|----------|
| **x2730 DPP-PSD CUP Documentation** | `legacy/documentation_2024092000-2/` | ★★★ |
| FELib User Guide | `legacy/GD9764_FELib_User_Guide.pdf` | ★★ |
| Digitizer System Spec | `docs/digitizer_system_spec.md` | ★★★ |
| Event Bridge Wire Format | `docs/event_bridge_wire_format.md` | ★★ |
| Event Builder Spec | `TODO/event-builder/SPECIFICATION.md` | ★★★ |
