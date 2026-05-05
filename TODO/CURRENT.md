# Current Sprint - TODO Index

**Updated:** 2026-05-05（PHA2 Phase 1〜5 ほぼ完了。Phase 4 UI surface ✅ + Phase 5 docs ✅ 完了。5/4 後半に **Phase 3 で書いた decoder「修正」が実は回帰でデータを silently drop していた + cache-miss 経路が silent + UI が clamp を隠す + per-probe is_signed が hardcode `false`** の 4 件のバグが連続発覚 → revert + 修正 (commits `e641e99` `e45e0ec` `d980d79` `7ed3285`)。5/5 は再発防止に集中: cache-miss 経路を `info!` + `no_cache` カウンタ + サマリ + regression test、`pha2_simple_test` → `caen_simple_test --firmware` 汎用化、decoder hot-path heuristic 禁止 policy を `decoder/mod.rs` doc + CLAUDE.md に明文化、`pha2_56.json` 未コミット ad-hoc revert + 検証用 `pha2_56_signed_probe_test.json` 分離、TODO/51 更新。Phase 3 残: 校正源テスト (ハード待ち)。Phase 4.5 (probe_type cross-cutting) 設計合意済 follow-up）

このファイルは現在のスプリントの概要を示すインデックスです。
Claudeセッション開始時に必ず読み込まれます。

---

## MVP Status: ✅ 達成 (2026-03-13)

全 Goal 達成。PSD2 + PSD1 + PHA1 全 FW DAQ 稼働中。Grafana モニタリング + ELOG 自動投稿も稼働中。

## Active Tasks

| Priority | File | Status | Summary |
|----------|------|--------|---------|
| **0** | [51_pha2_integration.md](51_pha2_integration.md) | **🚧 Phase 1〜5 ほぼ完了 (2026-05-05)、校正源 + Phase 4.5 probe_type cross-cutting のみ残** | DPP-PHA on VX2730 (SN:52622 @ 172.18.4.56) 統合。Phase 1〜3 (5/4 commits `fb8f66e` `512368c` `a3aa0be` `e423692`) は decoder + apply loopback + ROOT 出力 + sweep 堅牢化まで。**Phase 4 surface (`b258ab0`) + Phase 5 docs (`6ba6cea`) も 5/4 にコミット済**。5/4 後半に Phase 3 で書いた "truncation 検出" が実は回帰でデータ drop、cache-miss が silent、UI が clamp を隠す、per-probe is_signed が hardcode の 4 件のバグが連続発覚し fix (`e641e99` `e45e0ec` `d980d79` `7ed3285`)。5/5 は **再発防止 hardening**: cache-miss → `info!` + `no_cache` counter + summary + 2 regression tests、`pha2_simple_test` → `caen_simple_test --firmware {pha2,psd2,pha1}`、decoder hot-path heuristic 禁止 policy を `decoder/mod.rs` doc + CLAUDE.md に明文化、`pha2_56.json` 未コミット revert + signed-probe 検証用 `pha2_56_signed_probe_test.json` 分離。Phase 3 残: 校正源 (ハード待ち)。Phase 4.5 (`EventData.Waveform` に `analog_probe_type[2]` + `digital_probe_type[4]` 追加; `#[serde(default)]` + u8 PHA2-canonical encoding) は設計合意済の follow-up |
| **0 (done)** | [49_amax_fw_integration.md](49_amax_fw_integration.md) | **✅ Phase 1+2+2.5 完了 (master merged)** | Phase 1 (surface migration + codegen bridge) + Phase 2 (AxisSource enum / 2D 軸 dropdown / TTL eviction / 1D UserInfo / Reset / Hex tooltip) + Phase 2.5 (新 FW page 0x100000 移植、`channel_writes()` codegen 自動化、waveform per-probe is_signed flag、heatmap polish、gant dev env + MongoDB TOML)。実機検証 @172.18.4.56 (run 300-303): 26 AMax registers Apply、ch0 1 kHz、波形・1D・2D 全部 amax_viewer と一致。Commits: `c07c11e` `5185856` `68c3b5b` `cc98d7d` `ef5ba7c` `da7f73e` `7960066` `be0c7a3` `009af4e` `0ba78f6` `89a0cab` `5737b1d` `9d49f63` |
| **0 (done)** | [48_v1743_tuneup_double_apply_crash.md](48_v1743_tuneup_double_apply_crash.md) | **✅ FINAL RESOLUTION (2026-04-27)** | **真因**: `CAEN_DGTZ_MallocReadoutBuffer` を `apply_config_standard` の前に呼んでいた → 35 KB / 35 MB の 1000× サイズ食い違い → CAEN DMA が user buffer を踏み越え → libCAENDigitizer.so 内部破壊 → SIGSEGV。4/24 fix (SIGTERM + Apply skip) は症状緩和のみ。修正: `45bb325` (真因 fix) + `8f6ce55` `838dfbb` `d976daf` (関連 fix)。**1 セッション内 30/30 Run + 5/5 Tune Up cycles PASS** |
| **0 (done)** | — (V1743 WaveDemo params, 2026-04-27) | **✅ デプロイ稼働確認** | trigger_edge / pulse_polarity 分離 + ttf_smoothing + extra_registers + DC-offset-aware V threshold (`trigger_threshold_v`)。ChannelConfig + ChannelParamDef + apply_channel_config + 4 unit test pass。172.18.4.147 SN:25 で稼働確認済。`start_daq.sh` の `cargo build --release --features x743` 修正済 (ビルド時 features 落ち再発防止) |
| **1** | [47_v1743_standard_mode_redesign.md](47_v1743_standard_mode_redesign.md) | **🎯 Step 1-3 完了 (2026-04-23) / Step 4-7 (多台同期 + S1 較正 + 5 ps RMS) はハードウェア拡張待ち** | V1743 Standard mode 再設計 — RolloverTracker 統一 + V1743 組込 + PSD1/PHA1 移行 + 旧 TimestampTracker 削除 + **95 min 長時間ランで 40-bit TDC rollover 通過確認** (120M events) |
| 1-old | [45_v1743_support.md](45_v1743_support.md) | **⚠️ Phase 2 は 47 に移管** | V1743 Phase 1 (FFI+接続) ✅。Phase 2 以降は 47 へ |
| **2** | [30_mvp_march_roadmap.md](30_mvp_march_roadmap.md) | **✅ MVP達成** | 3月MVP: 全Goal達成 — 全FW DAQ稼働 + Grafana + ELOG |
| **3** | [26_multi_digitizer_scaling.md](26_multi_digitizer_scaling.md) | **📋 計画中** | 10+ デジタイザ対応スケーリング — ポストMVP |
| **4** | [24_l2_filter_implementation.md](24_l2_filter_implementation.md) | **📋 計画中** | L2 Filter — 3-4月実験では不要。将来タスク |
| **5** | [50_mac_usb_eem_driver.md](50_mac_usb_eem_driver.md) | **📋 計画中 (低優先 / 週末プロジェクト)** | macOS userspace USB CDC-EEM ドライバ (libusb + utun)。macOS Tahoe で AppleUSBEEM が codeless 化、Linux VM 経由は動作確認済 (2026-04-30)。USB 280 MB/s で 1Gb Ethernet 117 MB/s 天井を突破するルート |
| - | [event-builder/SPECIFICATION.md](event-builder/SPECIFICATION.md) | **参照** | Event Builder 仕様 |

---

## ポスト MVP — 次のセッション候補

- **A:** Energy Calibration + PSD 表示 (GitHub #7) → **設計完了** (2026-02-19, [設計書](../docs/plans/energy_calibration_psd.md)) — Phase 1 から開始
- **B:** x743 統合 (GitHub #6) → **Active** ([TODO](45_v1743_support.md), [設計書](../docs/plans/x743_integration.md))
- **C:** Online EB 統合 (Phase 4: ZmqHitSource + Pipeline) — 夏以降の実験で必要
- **G:** 設定自動生成スクリプト (3-4) + デプロイスクリプト改善 (3-5)
- **I:** FELib/Dig2 Rust 移植検討 — JSON-RPC プロトコル直接実装 (ポスト MVP)

---

## Recently Completed

| File | Completed | Summary |
|------|-----------|---------|
| [51_pha2_integration.md](51_pha2_integration.md) | 2026-05-05 | **PHA2 5/4 後半バグ群 → 5/5 再発防止 hardening**: 5/4 後半は ① Phase 3 で書いた decoder「mid-loop wf-header truncation 検出」がが DP4+AP2 baseline を hit して全イベントの波形後半 silent drop (`e641e99` revert + `caen_simple_test` で実機検証) ② `param_cache` が case-sensitive で全 CamelCase 書き込みが silent fallthrough → unvalidated set_value で clamp bypass (`e45e0ec`) ③ channel-table が clamp を「隠す」設計を flash + re-emit に (`d980d79`) ④ PHA2 wf-extras header の per-probe `is_signed` を hardcode `false` のまま放置 → Time-filter probe wrap (`7ed3285`) の **4 件** が連続発覚。Phase 4 surface (`b258ab0`) + Phase 5 docs (`6ba6cea`) は完了済。5/5 は再発防止: cache-miss 経路を `info!` + 新カウンタ `no_cache` + `ParamApplyStatus::NoCache` + サマリログ + 2 regression tests (`unknown_param_name_misses_cache`, `apply_config_result_round_trips_no_cache_status`)、`pha2_simple_test` → `caen_simple_test --firmware {pha2,psd2,pha1}` 汎用化、decoder hot-path heuristic 禁止 policy を `decoder/mod.rs` doc + CLAUDE.md に明文化、`pha2_56.json` 未コミット ad-hoc revert + signed-probe 検証用 `pha2_56_signed_probe_test.json` 分離。561 unit tests 緑、`cargo clippy --release --tests -- -D warnings` 緑。Phase 4.5 (probe_type cross-cutting `EventData` 拡張) は設計合意済 follow-up |
| [49_amax_fw_integration.md](49_amax_fw_integration.md) | 2026-04-29 | **AMax Phase 2.5 (新 FW + 波形 + dev 環境)**: ① 新 caenlist 32-channel FW (`RegisterFile_21last.json`) 移植 — `Path` → `Name` schema、page base `0x800000` → `0x100000`、`AMAX_delay` → `DELAY_SHAPING` rename + `SHAP_TRIGG` / `SHAP_BL_HOLD` 追加、`channel_writes()` codegen helper で `handle.rs` の hand-written 24-field 配列を撤廃。② 波形 visibility 修正 — `BoardConfig.waveforms_enabled` を OpenDPP endpoint format JSON に plumb (Settings tab toggle 追加)、`selector_wave=0` で生 1 stream 確認。③ `Waveform.analog_probe1_is_signed` per-probe フラグ — PHA1 (sign-extended) のみ +8191 centering、PSD1/PSD2/AMax は raw scale。④ Heatmap polish — bin left-edge 軸ラベル、`(xi>0 && yi>0)` フィルタ + auto-zoom、visualMap range 毎 poll リセット、`outOfRange.color`。⑤ gant@172.18.6.114 開発環境セットアップ — `git reset --hard origin/master` + `cargo build --release` + `OperatorFileConfig.mongodb` 追加で TOML 経由で MongoDB 接続。実機 run 300-303 @172.18.4.56: 26 AMax registers Apply、ch0 1 kHz、波形・1D・2D 全部 amax_viewer と一致。Commits: `009af4e` `0ba78f6` `89a0cab` `5737b1d` `9d49f63`。Memory: [mongodb_gant.md](/Users/aogaki/.claude/projects/-Users-aogaki-WorkSpace-delila-rs/memory/mongodb_gant.md) |
| [49_amax_fw_integration.md](49_amax_fw_integration.md) | 2026-04-28 | **AMax Phase 1 (surface + codegen) + Phase 2 (AxisSource + 2D 軸)**: Phase 1 = `EventData.user_info: [u64; 4]`、`AMaxChannelConfig` (24 typed fields)、`apply_amax_channel_config`、Monitor `amax2d_histograms`、Settings tab AMax 24 params 自動展開、heatmap-chart `yAxisLabel`、`/api/histograms2d/:m/:c?type=amax2d`。Phase 1 polish = generateUUID 無限再帰 fix、test_pulse_low/high_level、amax2d Y bins 256→512。Codegen bridge = `src/bin/amax_codegen.rs` で RegisterFile.json + fw_params.json から Rust struct + REG_const + TS interface + ChannelParamDef[] 自動生成、手書き 4 ファイル同期 撤廃。Phase 2 = `AxisSource` enum (Energy/EnergyShort/UserInfo0..3/Psd) + `MonitorState.histograms2d: HashMap<(ChannelKey, AxisSource, AxisSource), PlotEntry>` + 30s/60s TTL eviction + 2D X/Y dropdown UI + `migrateLegacyHistType()` (psd2d/amax2d → 2d)。残タスク = Hex address tooltip、Reset AMax defaults button (codegen で `AMAX_DEFAULTS`)、UserInfo[0..=3] 1D histograms。実機 @172.18.4.56 run 201/206-209 で全機能検証、TestPulse 2ch ~10kHz。Commits: `c07c11e` `5185856` `68c3b5b` `cc98d7d` `ef5ba7c` `da7f73e` `7960066` `be0c7a3` |
| [48_v1743_tuneup_double_apply_crash.md](48_v1743_tuneup_double_apply_crash.md) | 2026-04-27 | **V1743 SIGSEGV 真因確定 + 4 commits**: T7 (`x743_cycle_test --alloc-before-apply`) で `MallocReadoutBuffer` を apply_config 前に呼ぶと毎 cycle CAEN_DGTZ_OutOfMemory + DMA overflow → SIGSEGV を 100% 再現。修正 commits: `8f6ce55` (operator/run_start Phase 1.5 X743Std skip), `45bb325` (★ reader/v1743 buffer alloc を apply 後に移動), `838dfbb` (bin/x743_cycle_test に T1–T7 isolation flags), `d976daf` (UM1935 + WaveDemo 監査による 4 fix: self-trigger 全 disable→enable, SW trigger ACQ_ONLY 維持, ClearData before SWStart 削除, LoadSAMCorrection conditional)。Hardware 30/30 Run + 5/5 Tune Up PASS, 0 core dumps, apply latency ~2.06s で regression 無し。Plan: `~/.claude/plans/gemini-cli-peppy-turtle.md`。Memory: [caen_buffer_alloc_order.md](/Users/aogaki/.claude/projects/-Users-aogaki-WorkSpace-delila-rs/memory/caen_buffer_alloc_order.md), [v1743_tuneup_double_apply_crash.md](/Users/aogaki/.claude/projects/-Users-aogaki-WorkSpace-delila-rs/memory/v1743_tuneup_double_apply_crash.md) |
| — | 2026-04-23 夜 | **V1743 Step 3 完全完了**: 95 分連続ラン (run 6001) で 40-bit TDC rollover (91.63 min) 通過確認。120M events, rollover_count 0→1, backward=0, ERROR=0。副次発見: パルサー vs V1743 水晶の温度由来 ~65 min 周期うねり (~1500 ppm pp)、1D Δt 分布の "2 peak" の原因。V1743 energy は flat-top broadening あるが timing 用途なので simple amplitude 維持の判断。メモリ: [rollover_tracker_validated_2026-04-23.md](/Users/aogaki/.claude/projects/-Users-aogaki-WorkSpace-delila-rs/memory/rollover_tracker_validated_2026-04-23.md), [v1743_pulser_clock_beat.md](/Users/aogaki/.claude/projects/-Users-aogaki-WorkSpace-delila-rs/memory/v1743_pulser_clock_beat.md), [v1743_energy_known_limitation.md](/Users/aogaki/.claude/projects/-Users-aogaki-WorkSpace-delila-rs/memory/v1743_energy_known_limitation.md) |
| — | 2026-04-23 昼 | **V1743 Step 3-6 (PSD1/PHA1 shadow 検証 + 旧 TimestampTracker 完全削除)**: 本番 DT5730 で旧 `TimestampTracker` の host-clock safety net が startup latency を "drift" と誤解釈して off-by-one を注入する実測証拠を shadow mode で取得。旧実装を削除し、PSD1/PHA1 を新 `RolloverTracker` 単独に移行。151 単体テスト緑、実機 safety_net_fires=0。設計原則: [layering_principle_clock_sync.md](/Users/aogaki/.claude/projects/-Users-aogaki-WorkSpace-delila-rs/memory/layering_principle_clock_sync.md) — ソフト時刻で hw 時刻を補正しない |
| — | 2026-04-23 朝 | **V1743 Step 3 (RolloverTracker 実装 + 組込)**: `src/reader/decoder/rollover.rs` 新規 (21 単体テスト + proptest)。V1743 `x743_std_event_to_event_data` に per-group 8 tracker 組込、SW Start で reset。実機 run3001 = 421 MB, 21 kHz 100%, underflow ログ 0 件 |
| — | 2026-04-22 夕 | **V1743 Step 2 実機確認**: SN:25 @ 172.18.4.147 で 10 kHz パルサ 100% 取得。record_length=256, post_trigger=40, threshold=45874 (-0.5V) 確定。波形 peak@33ns/80ns窓、E ヒスト FWHM 8 bins、run2001 = 448MB |
| — | 2026-04-22 夕 | **V1743 Step 3 設計確定**: Rollover tracker 統一設計 ([docs/plans/rollover_tracker_design.md](../docs/plans/rollover_tracker_design.md))。Gemini 2.5 Pro 協議反映: u64 内部/modulo 演算/per-group 4 tracker/late arrival 対応。SW Fine TS と共用。明日実装着手可 |
| — | 2026-04-22 | **V1743 Step 2 実装完了**: `x743_std_event_to_event_data` を Standard mode 専用に刷新 — `timestamp_ns = TDC*5 + PosEdge/NegEdge ns` 合成、`fine_time` 10-bit packing、`energy = Charge*scale + offset` クランプ、`save_waveform` で `DataChannel[ch]` → i16 コピー、`X743DecodeParams` を ReadLoop にキャッシュ、`X743Config` に `fine_time_source`/`energy_source`/`energy_scale`/`energy_offset`/`save_waveform` 追加。ユニットテスト 4 本 (fine_time / neg_edge / energy_clamp / absent_event) 追加。`cargo clippy --features x743 --tests -- -D warnings` OK。**SN:25 実機でのパルサ E スペクトル + TDC 線形性確認は Linux host で要実施** |
| — | 2026-04-22 | **V1743 Step 1 完了**: DPP-CI (Charge Mode) コード全削除 — `apply_config_dpp_ci`/`read_loop_x743_ci`/`x743_dpp_ci_event_to_event_data` 削除、`X743Config` から `dpp_ci_*`/`pair_*`/`board_*` 除去、`x743_ci_probe` binary 削除、`SourceType::X743CI` → `FirmwareType::X743Std` に warn + マップ。`cargo clippy --features x743 -- -D warnings` OK |
| — | 2026-04-20 | **V1743 DPP-CI 撤退決定**: CAEN App Note 精読で DPP-CI 実装バグ (x720/x743 構造体ミスマッチ) + 設計限界 (TimeTag 無し) を発見。Standard mode 一本化へ方針転換 (TDC 40-bit + Charge float)。設計書: [x743_standard_mode_design.md](../docs/plans/x743_standard_mode_design.md), TODO: [47](47_v1743_standard_mode_redesign.md) |
| [30_mvp_march_roadmap.md](30_mvp_march_roadmap.md) | 2026-03-13 | **MVP 達成**: 全FW DAQ稼働 (PSD2+PSD1+PHA1) + Grafana + ELOG自動投稿 |
| [37_grafana_monitoring.md](37_grafana_monitoring.md) | 2026-03-12 | Grafana モニタリング: InfluxDB v3 Core + Grafana, DAQ Overview + Channel Rate ダッシュボード, 192.168.147.98 デプロイ済 |
| — | 2026-03-12 | `is_master` 削除: TOML/Operator config から冗長な `is_master` を除去、Reader の `startmode` に一元化 (SSOT)。3MV config Start タイムアウト修正 |
| [event-builder/38_eb_unification_mimalloc.md](event-builder/38_eb_unification_mimalloc.md) | 2026-02-26 | EB 統一パイプライン Phase 0-3: HitSource trait + pipeline.rs + DelilaFileHitSource + Offline CLI rewrite (.delila 直接入力) + time alignment histogram 出力 |
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
- EB 統一パイプライン Phase 0-3 (HitSource trait, pipeline.rs, source.rs, .delila直接入力CLI, time alignment histogram)
- Grafana モニタリング (InfluxDB v3 Core + Grafana 2ダッシュボード: DAQ Overview + Channel Rate 48ch Stat)
- `is_master` 削除 (TOML/Operator config → Reader `startmode` に一元化、3MV Start タイムアウト修正)
- PHA1 実機稼働 (VX1730B 光リンク, 全FW DAQ完成)
- ELOG 統合 (Docker + Rust クライアント + Run Stop 自動投稿)
- A3818 scheduling-while-atomic 修正 (spin_lock→mutex, 76デプロイ済)
- V1743 Standard mode 単体構成 (VX1743 SN:25, optical link, CFD soft fine time, 95 min long-run で 40-bit TDC rollover 通過確認)
- RolloverTracker 統一実装 (PSD1/PHA1 SW Fine TS + V1743 TDC を modulo 演算で共通処理, 旧 Instant-ベース TimestampTracker 完全削除)

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
| [44_a3818_open_fix.md](44_a3818_open_fix.md) | A3818 scheduling-while-atomic 修正 |

---

## Known Issues / Future Work

- **TIME_STEP_NS ハードコード:** `src/config/digitizer.rs` の `TIME_STEP_NS = 2` は DT5730 (500 MS/s) 固定。DT5725 (250 MS/s → 4 ns/sample) など別サンプリングレートのデジタイザに対応する場合、`DeviceInfo.sampling_rate_sps` から動的計算するか、`DigitizerConfig` にサンプリングレートを持たせる必要あり。

## Notes

- **MVP: ✅ 達成** (2026-03-13)
- **現在のフェーズ:** ポスト MVP — 安定運用中、改善・拡張フェーズ
- **実機確認済み:**
  - VX2730 (Serial: 52621, DPP_PSD2, 32ch, Ethernet, 172.18.4.57)
  - DT5730B (Serial: 990, DPP_PSD1/PHA1, 16ch, USB) — ライセンス 30 min 制限あり
  - 5x VX1730B (PSD1, 16ch each) + 1x VX2730 (PSD2, 32ch) on 172.18.4.76
  - VX1743 (Serial: 25, SAMLONG 12-bit, 8ch/4groups, Optical Link) on 172.18.4.147 — Standard mode + 95 min long-run 完了 (2026-04-23)
- **動作環境:** Linux (Ubuntu, Rust 1.93.0) + macOS (クロスマシン統合)

## Reference Documents

| Document | Location | Priority |
|----------|----------|----------|
| **x2730 DPP-PSD CUP Documentation** | `legacy/documentation_2024092000-2/` | ★★★ |
| FELib User Guide | `legacy/GD9764_FELib_User_Guide.pdf` | ★★ |
| Digitizer System Spec | `docs/digitizer_system_spec.md` | ★★★ |
| Event Bridge Wire Format | `docs/event_bridge_wire_format.md` | ★★ |
| Event Builder Spec | `TODO/event-builder/SPECIFICATION.md` | ★★★ |
