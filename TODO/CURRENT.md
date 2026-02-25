# Current Sprint - TODO Index

**Updated:** 2026-02-25

このファイルは現在のスプリントの概要を示すインデックスです。
Claudeセッション開始時に必ず読み込まれます。

---

## Active Tasks

| Priority | File | Status | Summary |
|----------|------|--------|---------|
| **1** | [30_mvp_march_roadmap.md](30_mvp_march_roadmap.md) | **📋 計画中** | 3月MVP: PHA1統合 + EB オンライン化 + Grafana + 運用改善 |
| **1** | [event-builder/38_eb_unification_mimalloc.md](event-builder/38_eb_unification_mimalloc.md) | **📋 計画完了** | EB 統一: SliceBuilder→chunk_builder + mimalloc 導入 (Gemini レビュー済) |
| **1** | — | **🔧 実装中** | Online Event Builder v2: チャンク＋Safe Horizon 方式で全面書き直し ([設計書](../docs/plans/online_event_builder_v2.md)) — ROOT出力: oxyroot Vec events + file-per-batch確定 ([bench](../docs/plans/oxyroot_benchmark_results.md)) |
| **2** | [37_grafana_monitoring.md](37_grafana_monitoring.md) | **📋 計画完了** | Grafana モニタリング: InfluxDB v3 Core + チャンネル別レート ([設計書](../docs/plans/grafana_monitoring.md)) |
| **2** | [26_multi_digitizer_scaling.md](26_multi_digitizer_scaling.md) | **📋 計画中** | 10+ デジタイザ対応スケーリング (A1, A3, C3 が MVP 候補) |
| **2** | [33_delila2root_converter.md](33_delila2root_converter.md) | **✅ Phase 1 完了** | delila2root: 10.4億events 2分31秒, 6.9M/s, タイムスタンプ違反0 ([設計書](../docs/plans/delila2root.md)) |
| **3** | [24_l2_filter_implementation.md](24_l2_filter_implementation.md) | **📋 計画中** | L2 Filter — 3-4月実験では不要。将来タスク |
| - | [event-builder/SPECIFICATION.md](event-builder/SPECIFICATION.md) | **参照** | Event Builder 仕様 |

---

## 次のセッション候補

- **A:** Energy Calibration + PSD 表示 (GitHub #7) → **設計完了** (2026-02-19, [設計書](../docs/plans/energy_calibration_psd.md)) — Phase 1 から開始
- **B:** x743 統合 (GitHub #6) → **設計完了** ([設計書](../docs/plans/x743_integration.md))
- **C:** Event Builder オンラインパイプライン統合 (EB-1〜EB-4) → Phase 2
- **D:** Grafana モニタリング (InfluxDB v3 Core + Grafana) → **計画完了** (2026-02-19, [設計書](../docs/plans/grafana_monitoring.md) + [TODO](37_grafana_monitoring.md))
- **G:** 設定自動生成スクリプト (3-4) + デプロイスクリプト改善 (3-5)
- **H:** ~~トリガーロス・ビジー検出~~ — **✅ COMPLETED** (2026-02-25, [TODO](43_trigger_loss_detection.md) + [設計書](../docs/plans/trigger_loss_detection.md)) — 本番 Run 156 で検証済: DIG1 フラグカウント + DIG2 5sポーリング + フロントエンド表示

---

## Recently Completed

| File | Completed | Summary |
|------|-----------|---------|
| [43_trigger_loss_detection.md](43_trigger_loss_detection.md) | 2026-02-25 | トリガーロス検出: DIG1 EXTRAS フラグ + DIG2 カウンタポーリング, 本番 Run 156 (6台, 3.81M eve/s) で検証済 |
| — | 2026-02-25 | oxyroot ベンチマーク: Vec events 0.79M events/s, 実運用300k/sに対して2.6xマージン, file-per-batch確定 ([結果](../docs/plans/oxyroot_benchmark_results.md)) |
| — | 2026-02-25 | ReadLoop transient error retry: 30s timeout + 10ms interval, 5MHz 6modules 10min検証済, DIG1 ch_extras_opt コード強制 |
| [archive/42_a3818_driver_patch.md](archive/42_a3818_driver_patch.md) | 2026-02-24 | A3818 ドライバパッチ v1.6.12-delila1: バッファオーバーフロー防止 (1MB→16MB) + off-by-one + セマフォバグ + PCIe障害検出、76にデプロイ済 |
| [archive/40_decode_loop_parallelization.md](archive/40_decode_loop_parallelization.md) | 2026-02-23 | DecodeLoop 並列化: 4 Workers (crossbeam) + ReorderBuffer, PSD2 2.2M→7.0M events/10s (3.2x), Stop→EOS 49s→<1ms |
| [archive/39_cross_run_eos_fix.md](archive/39_cross_run_eos_fix.md) | 2026-02-23 | Cross-Run EOS 汚染修正: run_number タグ + stale EOS フィルタ |
| — | 2026-02-19 | GitHub #5: Run History ビューワー (アコーディオン + config snapshot) + BSON HashMap キー修正 |

## Cancelled (archived)

| File | Reason |
|------|--------|
| [archive/22_amax_decoder_implementation.md](archive/22_amax_decoder_implementation.md) | FELib OpenDPP エンドポイントから直接イベント取得で十分。デコーダ不要 |
| [archive/41_start_stop_restructure.md](archive/41_start_stop_restructure.md) | DIG1 タイムスタンプリセット問題は別原因が判明。Start/Stop フロー再構成は不要 |

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
- PHA1 Settings UI修正 (FirmwareType PHA→PHA1統一 + Virtual Probe データ駆動型 + 単位修正)
- Stop コマンドタイムアウト修正 (decode_loop yield + Recorder writer std::thread 分離)
- Tune Up Apply スペクトラム混在修正 (Merger sender_task ステート対応 + 全パイプライン Stop→Start)
- パラメータバリデーション (DevTree snap_to_step + UI step属性)
- delila2root Phase 1 (POD Event + two-pointer merge, 10.4億events 2m31s, 6.9M/s)
- PHA1 Waveform Decoder 修正 (sign_extend_14bit 符号拡張 + digital probe D0/D1 マッピング修正)
- ROOT マクロ ns_per_sample 対応 (Waveform X軸 ns 表示)
- Operator デフォルトポート 9090 移行 + docker mongo-express 8083
- Tune Up ソフトウェアトリガー強制 (clone-and-modify, SyncConfig 両方上書き)
- Run Start Waveform Recording 警告 (MatDialog, DigitizerService チェック)
- Tune Up Waveform 積算表示 (FIFO バッファ + timestamp 重複検出 + ECharts replaceMerge)
- Run History ビューワー (Runs タブ, アコーディオン展開, config snapshot 表示)
- BSON config snapshot 修正 (HashMap<u8/u32> キーの string 変換 serde モジュール)
- Config snapshot 診断ログ追加 (起動時ロード数 + 空時 warn)
- Cross-Run EOS 汚染修正 (run_number タグ + stale EOS フィルタ)
- DecodeLoop 並列化 (4 Workers crossbeam, PSD2 3.2x改善, 全FW統一パス)
- A3818 ドライバパッチ v1.6.12-delila1 (バッファオーバーフロー+off-by-one+セマフォ+PCIe障害, 76デプロイ済)
- ReadLoop transient error retry (30s timeout, 10ms interval, 5MHz×6modules 10min検証, 20回リカバリ全成功)
- DIG1 ch_extras_opt コード強制 (PSD1/PHA1 の 48-bit timestamp を JSON 設定に依存せず handle.rs で保証)
- oxyroot ROOT 出力ベンチマーク (Vec events 0.79M/s, file-per-batch確定, Stop <130ms)

---

## Archived

| Directory/File | Contents |
|----------------|----------|
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
| `archive/19_settings_ui.md` | デジタイザ設定 UI Phase 6 |
| `archive/22_amax_decoder_implementation.md` | AMax デコーダ (Cancelled) |
| `archive/25_apply_digitizer_via_zmq.md` | Apply Digitizer Config via ZMQ |
| `archive/27_settings_ui_v2.md` | Settings UI v2 (Phase 1-6) |
| `archive/28_tuneup_mode.md` | Tune Up Mode 実装 |
| `archive/28_psd1_pha1_parameter_overhaul.md` | PSD1/PHA1 パラメータ整理 |
| `archive/29_channel_registration.md` | Channel Registration + チャンネル名 |
| `archive/29_psd1_waveform_debug.md` | PSD1 Waveform デバッグ |
| `archive/31_parameter_validation.md` | パラメータバリデーション (Phase 1-3) |
| `archive/32_stop_command_timeout.md` | Stop タイムアウト修正 |
| `archive/34_tuneup_software_trigger.md` | Tune Up ソフトウェアトリガー |
| `archive/35_waveform_recording_warning.md` | Waveform Recording 警告 |
| `archive/36_accumulated_waveform.md` | 積算 Waveform 表示 |
| `archive/39_cross_run_eos_fix.md` | Cross-Run EOS 汚染修正 |
| `archive/40_decode_loop_parallelization.md` | DecodeLoop 並列化 |
| `archive/41_start_stop_restructure.md` | Start/Stop フロー再構成 (Cancelled) |
| `archive/42_a3818_driver_patch.md` | A3818 ドライバパッチ v1.6.12-delila1 |

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
