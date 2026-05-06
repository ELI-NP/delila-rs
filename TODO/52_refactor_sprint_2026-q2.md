# TODO 52: Refactor Sprint 2026-Q2 (post-PHA2 cleanup, pre-7/24 experiment)

**Status:** 🚧 **Phase 1 完了 (2026-05-06)、Phase 2 進行中 (R-D6 完了 2026-05-06)**
**Created:** 2026-05-05 (revised 2026-05-05 evening: 一枚岩撤回 → component system 維持)
**Window:** 2026-05-12 〜 2026-07-03 (8 週間 active refactor) + 2026-07-03 〜 2026-07-24 (3 週間 stabilize)
**Source of truth:** [clean_architecture_evaluation.md](../clean_architecture_evaluation.md)
**Plan file:** `~/.claude/plans/phase1-todo-hidden-panda.md` (Phase 1 詳細)

## Status Summary (2026-05-06)

| Phase | 状態 | 主な成果 |
|---|---|---|
| **Phase 1 — Mechanical Cleanup** | ✅ **完了 (2026-05-06、 1 セッション)** | R-D4/D8/D9/D10/C3/C4/C5/C6/X2/X3/P1/P2/P7 の 13 項目すべて landed。 568 → 598 unit tests (+30)、 ng test 57 → 68 (+11)、 clippy clean。 R-X3 baseline benchmark を Mac M4 Pro + gant Xeon W-3223 両方で取得済 (`docs/plans/zmq_boundary_cost_2026-Q2.md`) |
| **Phase 2 — Structural Refactor** | 🚧 **進行中。 R-D6 + R-D7 + R-D1/D2 完了 (2026-05-06)** | **R-D6 (PSD1/PHA1 generic)** = 3 PR (`649980f` `a5b5398` `37d252a`)、 family -1531 行。 **R-D7 (PSD2/PHA2 generic)** = 3 PR (`23274d4` `14ee5cd` `53696b5`)、 family -680 行。 **R-D1/D2 (read_loop split)** = 1 PR (`1059709`)、 reader/mod.rs 4000→3112 (-22%)、 +read_loop_dig1.rs 463 / read_loop_dig2.rs 470 行。 **decoder family 累計 -2211 行 + reader/mod.rs -888 行**。 560 tests pass、 clippy clean、 公開 API 完全互換。 残 Phase 2: R-C1/C2 (config tables) / R-P3/P4/P5 / R-X1 |
| **Phase 3 — Component Hardening** | 📋 後回し | R-D3/D5/D11/D12/P6/P8/X3-post |

## 方針 (revised 2026-05-05 evening)

**現在の component system (Reader → Merger → Recorder/Monitor、ZMQ 境界、process 隔離) を維持**したまま重複を削減し、可読性を上げる。Pure 一枚岩 (D3) と gating benchmark (D4) は撤回。

### 撤回した理由

- 既存 component アーキテクチャは [docs/component_architecture.md](../docs/component_architecture.md) で確立済 (Receiver/Main/Sender/Command タスク分離 + lock-free shared state)
- cross-machine 構成 (172.18.4.56/76/147 + gant@172.18.6.114) は実運用しており、 ZMQ 境界の配備柔軟性は失えない
- 性能ボトルネックはすでに別の場所 (decode loop は並列化済 7M ev/s、ROOT 出力 0.79M ev/s/writer) で解決済、ZMQ 境界の実コストは「気になる」レベルで投資対効果が見えない
- 一枚岩化のレバレッジは「readable」だが、**重複削減 (R-D6 PSD1/PHA1 generic、R-C1 param tables 等) でも同じ効果が低リスクで得られる** ことが re-evaluation で判明

## ゴール

7 FW (PSD1/PSD2/PHA1/PHA2/AMax/V1743/X743Std) サポート完了後の **重複削減 + 構造整理** を、 component system 維持のまま実施。**動作不変** + **可読性向上** + **OSS 公開に耐える整理状態**。

## 動機

- `src/reader/mod.rs` 4042 行、`src/config/digitizer.rs` 2890 行に肥大
- PSD1 ↔ PHA1 が ~95% 重複 (1878/1816 行)、PSD2 ↔ PHA2 も aggregate header 同一
- `src/bin/` 42 binary (production 15 / dev 27)、デフォルト build noise
- component 自体の構造は健全 (CommandHandlerExt 8/10 浸透、HWM=0 統一、`RolloverTracker` 統一済) — **しかしハンドル間の Receiver/Main/Sender 配線は手書き反復**
- 来週 (2026-05-12〜) のラン中は March bin 使用なので master refactor は影響しない
- 7/24 次実験までに 11 週間の余裕あり

## 確定 Decisions (2026-05-05 evening 改版)

| # | 項目 | 決定 |
|---|---|---|
| D1 | Dev binary archive | `dev-tools` feature-gate (`required-features` に付与、 default build から外す) |
| D2 | PSD1/PHA1 共通化深さ | trait + generic (zero-cost monomorphization) |
| ~~D3~~ | ~~単一マシン内 component 境界 = Pure 一枚岩~~ | **撤回**。 component system 維持 (D7 参照) |
| ~~D4~~ | ~~D3 の gating benchmark~~ | **撤回**。判断 gate ではなくなったので不要 |
| D5 | 旧 `clean_architecture_evaluation.md` (2026-04-15) | 削除のまま、新規 (2026-05-05) が正規版 |
| D6 | Branch 戦略 | Daily merge to master、`git tag march-bin-baseline` 運用 |
| **D7** | アーキテクチャ方針 (NEW) | **現行 component system (Reader/Merger/Recorder/Monitor/Operator が ZMQ で繋がる process 隔離型) を維持**。重複削減・構造整理は component 内部 + 横断 trait 化で対処、 process 境界は触らない |
| **D8** | ZMQ 境界 cost benchmark (NEW、 **commit、 2 点測定**) | **R-X3 として実施**。 baseline (Phase 1 Week 1) + post-refactor (Phase 3 Week 11) の 2 点で測定し、差分を `docs/plans/zmq_boundary_cost_2026-Q2.md` に記録。 Throughput / Latency / per-boundary encode-decode + ZMQ 送受信時間 / per-event bytes。 sprint 設計を gate しないが、 「ZMQ 境界 cost が実際どれくらいか」という長年の宿題に数字で答え、 将来の意思決定材料にする |

### Decision review — D7 reaffirmed (2026-05-06)

R-X3 baseline 取得後、 「monolithic 化を将来検討するか」を再議論し **「現状しない」** で再確定:

- Mac M4 Pro: encode+decode = 0.25 µs/event。 4 MHz aggregate で 1 コアちょうど飽和
- gant Xeon W-3223: 0.44 µs/event。 4 MHz で ~1.76 コア相当
- Production 単一デジタイザ rate ~700k ev/s (memory: PSD2 並列化 record) なので現状余裕は ~5x
- **Monolithic 化のレバレッジは「同一マシン内の 2 ZMQ hop 削減」に絞られる**。 fan-out コストは Arc<EventDataBatch> なら Arc::clone 数 ns で済むので逆に減る方向。 ただし **クロスマシン境界 (172.18.4.56 ↔ 76 ↔ 147 等) は monolithic 化しても消えない** ので encode/decode 関数自体は維持必須
- 加えて `.delila` ファイル / recover ツール / online EB の wire format 互換性が MessagePack 前提なので、 monolithic 化と並行して別の serializer を維持する負担が発生

→ **D7 (component system 維持) を継続**。 Phase 3 R-X3 post-refactor で再計測し、 R-P8 ComponentRunner の boilerplate 削減が encode/decode 時間に影響しないことを確認できれば、 「D7 = 0.5 µs/event の境界コストを許容した判断」と明文化する。 7/24 後の monolithic 再検討議論があった場合の出発点になる。

## Phase 1 — Mechanical Cleanup ✅ 完了 (2026-05-06)

低リスク・動作不変。 1 セッションで 13 項目すべて landed (動作不変 + テスト緑 + clippy clean)。

- [x] **R-X3 baseline**: `src/bin/zmq_boundary_bench.rs` 新設 (in-process inproc:// PUB→SUB)。 Mac M4 Pro + gant Xeon W-3223 で 10k/100k/1M ev/s × 30s 計測、 全 0 drops。 `docs/plans/zmq_boundary_cost_2026-Q2.md` に Baseline テーブル記録 (raw JSON: `docs/plans/zmq_bench_results/`)。 主要発見: encode+decode = 0.25 µs/ev (Mac) / 0.44 µs/ev (gant) — **4 MHz aggregate で 1 コア飽和ライン** に来る
- [x] **R-D4**: `state_rank` / `effective_state_for` / `next_reconnect_cooldown` + 4 const → `src/reader/state.rs` (10 unit test)
- [x] **R-D8**: `extract_bits!` macro → `decoder/common.rs` (3 unit test) + PHA2 で 6 site パイロット採用
- [x] **R-D9**: PSD1 `decode_charge_word` ↔ PHA1 `decode_energy_word` の semantic equivalence を doc 化 (R-D6 への布石)
- [x] **R-D10**: `opendpp_to_event_data` smell 整理 — magic constants 命名、 silent truncation を `info!`-once 化、 4 unit test 追加
- [x] **R-C3**: `FirmwareCapabilities` struct + `FirmwareApi` enum (`Dig1` / `Dig2` / `CaenDigitizer`) を `digitizer.rs` に追加。 7 FW 全部 legacy helper と整合 (1 cross-check test)
- [x] **R-C4**: `src/config/devtree_paths.rs` 新設、 `cmd::*` 6 const + `par::*` 13 const。 `/cmd/` 18 site (`reader/mod.rs`) + `/par/activeendpoint` 2 site (`caen/handle.rs`) + `startmode` 2 site を定数化
- [x] **R-C5**: `set_in_run_param_names()` を 5 個の static `&'static [&'static str]` に抽出。 PSD2 と AMax は同一 slice 共有 (regression test 付き)
- [x] **R-C6**: `dev-tools` feature gate 追加。 27 dev binary を gate、 未登録 6 binary (`amax_firmware_check` / `amax_fw_test` / `amax_trgout_test` / `configure_benchmark` / `event_dump` / `register_test`) を Cargo.toml 登録 (`event_dump` のみ production 維持、 残り 5 は dev-tools)
- [x] **R-P1**: `src/common/zmq_helper.rs` 新設、 `pub_no_hwm` / `sub_no_hwm` で HWM=0 設定の **12 箇所** 手書きを統合 (Reader/Merger/Recorder/Monitor/DataSink/Emulator/online EB/event_bridge/eb_test_sender/x743_cycle_test)
- [x] **R-P2**: `web/operator-ui/src/app/components/echarts-base/echarts-base.utils.ts` 新設、 `siCountFormatter('linear'|'log')` / `defaultGrid` / `buildDualSliderDataZoom`。 11 spec、 histogram-chart で 2 site migrate (twin SI formatter → 1 関数)
- [x] **R-P7**: `src/operator/command_ext.rs` で `OperatorCommandExt` stateless shim 実装。 R-P8 ComponentRunner (Phase 3) で 7th component として enumerate される下地
- [x] **R-X2**: `.github/workflows/ci.yml` に `outdated` job 追加 (warn-only, `continue-on-error: true`)。 `cargo-deny` は既存で CVE をカバー済 → `cargo audit` は冗長で skip

**完了基準達成**:
- ✅ 568 → 598 tests pass (+30 new)、 clippy clean (`--release --tests --features dev-tools,root -- -D warnings`)
- ✅ ng test 57 → 68 pass (+11 new spec)、 ng build clean、 dist/ 再ビルド済
- ✅ `cargo build --release` (default features) で production 15 binary のみコンパイル、 `--features dev-tools` で +27 dev binary
- ✅ CI で `cargo outdated` warn-only job 追加
- ✅ 副次の clippy fix 2 件: `caen_legacy/handle.rs:204` unused-assignment、 `x743_cycle_test.rs` unused `error` import + dead store (どちらも pre-existing、 `--all-features` で surface)

**LoC 影響**:
- `+706 / -428 = net +278` (測定: `git diff --stat`)。 期待 `-300` から増えた理由は (a) 新規 module の doc + test が大きい (R-D4 state.rs / R-C4 devtree_paths / R-P1 zmq_helper / R-P7 command_ext + bench bin)、 (b) R-C5 の base+diff 化で各 slice に doc コメント付加、 (c) bench bin (zmq_boundary_bench) 自体が ~370 LoC。 機械的削減は予定通りだが、 Phase 1 で**追加された** test/doc/bench の方が大きかった
- 主要ファイルの収縮: `reader/mod.rs` 4042 → ~3700 行 (state extract + opendpp 整理 + import 整理)、 残り削減は Phase 2 R-D1/D2 で実現

## Phase 2 — Structural Refactor (Week 4–8, 2026-06-02 〜 2026-07-03)

中リスク。各 PR で test を厚く張ってから着手。Week 8 末で baseline freeze。**変更なし**。

- [x] **R-D6**: PSD1/PHA1 `Dig1Decoder<V: Dig1Variant>` generic 化 — 3 PR で完了 (2026-05-06、 1 セッション)
  - **PR1** (`649980f`): `decoder/psd1_pha1_common.rs` 新設 (additive)。 `Dig1Variant` trait (5 アイテム: `FW_NAME` / `DUAL_CHANNEL_SIZE_MASK` / `calculate_sw_fine_fraction` / `decode_physics_word` / `decode_waveform`)、 `Dig1Decoder<V>` generic (zero-cost monomorphization)、 `Dig1ChannelHeader` で PSD1 EE/EQ vs PHA1 E2/EE 命名統合 (bit 27-31 は同位置)、 共通 framing 関数 + 31 unit tests via MockVariant
  - **PR2** (`a5b5398`): `Psd1Decoder = Dig1Decoder<Psd1Variant>` 型エイリアス化。 psd1.rs 1886 → 405 行 (-78%)。 PSD1 固有 = waveform layout (unsigned 14-bit + DP1@14 / DP2@15) + `decode_charge_word` + `calculate_sw_fine_fraction_psd` (8192 baseline)
  - **PR3** (`37d252a`): `Pha1Decoder = Dig1Decoder<Pha1Variant>` 型エイリアス化。 pha1.rs 1824 → 455 行 (-75%)。 PHA1 固有 = waveform layout (sign-extended 14-bit + DP@14 / Tn@15、 Tn→D0 / DP→D1) + `decode_energy_word` + `calculate_sw_fine_fraction_pha` (signed 補間)
  - **Total**: family net -1531 行 (3710 → 2179 incl. common)、 公開 API 不変 (Psd1Config/Pha1Config の struct リテラル + ::new + .decode 等)、 561 tests pass、 clippy clean、 R-D9 で予告した命名統一 (`decode_physics_word`) 実装
- [x] **R-D7**: PSD2/PHA2 `Dig2Decoder<V: Dig2Variant>` generic 化 — 3 PR で完了 (2026-05-06、 1 セッション)
  - **PR1** (`23274d4`): `decoder/dualchannel_common.rs` 新設 (additive、 既存 psd2.rs / pha2.rs 不変)。 `Dig2Variant` trait (3 アイテム: `FW_NAME` + `decode_energy_short` + `parse_waveform_metadata`) + `Dig2Decoder<V>` generic + `WaveformMetadata` struct で per-probe `is_signed` + probe-type を一度に lift。 logging を `tracing::{info,debug,warn}` に統一 (PSD2 が println! → tracing にアップグレード)。 19 MockVariant + 2 trait-hook (BeefVariant + SignedAp1Variant) unit tests
  - **PR2** (`14ee5cd`): `Psd2Decoder = Dig2Decoder<Psd2Variant>` 型エイリアス化。 psd2.rs 1316 → 196 行 (-85%)。 PSD2 固有のみ retain: `decode_energy_short` (bits[41:26] charge_short) + `parse_waveform_metadata` 既定値 (unsigned + UNKNOWN) + 7 PSD2 tests
  - **PR3** (`53696b5`): `Pha2Decoder = Dig2Decoder<Pha2Variant>` 型エイリアス化。 pha2.rs 1086 → 392 行 (-64%)。 PHA2 固有のみ retain: `decode_energy_short = 0` + `parse_waveform_metadata` で wf-header 低 16 bits 解析 (analog/digital probe-type + is_signed flag) + 10 PHA2 tests。 **critical regressions 全保持**: `dp4_set_in_sample_does_not_truncate_waveform` (2026-05-04 e641e99 truncation 事案) + `analog_probe_is_signed_flag_is_parsed_from_wf_header` (7ed3285 hardcoded-false 事案) + `classify_start_signal_real_bytes` (172.18.4.56 captured bytes)
  - **Total**: family LoC 2402 → 1722 (-680、 -28%)、 公開 API 不変、 560 tests pass、 clippy clean。 R-D6 と合算で **decoder family 累計 -2211 行**
- [x] **R-D1**: `read_loop_raw` → `src/reader/read_loop_dig1.rs` (`1059709`、 2026-05-06)
  - Pure mechanical move: `Reader::read_loop_raw` (associated fn、 no `&self`) → `read_loop_dig1::run` 自由関数。 関数本体バイト同一
  - 463 行新ファイル、 PSD1/PSD2/PHA1/PHA2 (RAW endpoint) 全部の DIG1/DIG2 read loop を担当
  - 共有 helper を `pub(crate)` にバンプ: `DeviceConnection` (struct + 9 fields) / `try_connect_raw` / `try_connect_opendpp` / `Dig2PollState` / `poll_dig2_counters` / `send_arm_command` / `send_start_command` / `get_enabled_channels_from_config` / `opendpp_to_event_data` / `ReadLoopOutput` / `ReadLoopRequest`
- [x] **R-D2**: `read_loop_opendpp` → `src/reader/read_loop_dig2.rs` (`1059709`、 2026-05-06、 R-D1 と同 PR)
  - 同パターンで `read_loop_dig2::run` 自由関数化、 470 行
  - AMax / DPP_OPEN firmware 用
  - **総合**: reader/mod.rs 4000 → 3112 行 (-888、 -22%)。 Phase 2 目標 (~2000 行) には R-D3 (X743 read_loop、 Phase 3 へ delay) + R-D5 (connection extract、 Phase 3) で到達予定
- [ ] **R-C1**: `add_channel_params` 366 行を `HashMap<FirmwareType, &[(ConfigField, &str)]>` 駆動 1 loop に
- [ ] **R-C2**: `merge_field!` 38 fields を `#[derive(Merge)]` proc macro に
- [ ] **R-P3**: Operator `AppState` RwLock 12 fields → DashMap or RCU read cache
- [ ] **R-P4**: digitizer/event-builder/emulator settings の base panel 抽出
- [ ] **R-P5**: `digitizer.rs` (983 行) の 14 CRUD handler を generic factory 化
- [ ] **R-X1**: production `unwrap()` audit (Mutex は除外、Result/Option の bad case のみ修正)

**完了基準**: `reader/mod.rs` ~4042 → 2000 行、`config/digitizer.rs` ~2890 → 1800 行、production `Result::unwrap()` 0、 baseline freeze + `git tag refactor-phase2-complete`。

## Phase 3 — Component System Hardening + Stabilize (Week 9–11, 2026-07-06 〜 2026-07-24) ★REVISED★

D7 で component system 維持決定済。**Phase 3 は monolithic 化ではなく、 component 内部の delayed structural refactor + component 横断の lifecycle 整理 + 7/24 実験前 stabilize**。

### Week 9 (2026-07-06〜) — Delayed Reader split + 横断統一

- [ ] **R-D3**: `read_loop_x743_std` (mod.rs:2261–2741) + `x743_std_event_to_event_data` → `src/reader/read_loop_x743.rs` + `x743_decode_params.rs`。Phase 2 で X743 hardware-only path のテスト不足リスクのため delayed
- [ ] **R-D5**: `try_connect_raw/opendpp` + `DeviceConnection` struct → `src/reader/connection.rs`。Phase 1/2 で他の reader 整理が済んでから着手するのが安全
- [ ] **R-P6**: event_builder routes (565 行) の `ch_settings` / `l2_settings` / `time_settings` parallel structure を generic factory に統合 (R-P5 完了後)

### Week 10 (2026-07-13〜) — Component lifecycle 統一 (NEW)

- [ ] **R-P8 (NEW)**: `Component` trait + `ComponentRunner` builder 抽出
  - 各 component (Reader / Merger / Recorder / Monitor) は `docs/component_architecture.md` の Receiver/Main/Sender/Command パターンに従っているが、 配線は **手書き反復** (4 component 分)
  - `ComponentRunner::new(name).receiver(...).main(...).sender(...).command_ext(...).run()` のような builder で boilerplate を 1 箇所に集約
  - 各 component 本体は **ビジネスロジックのみ** に専念、 ZMQ 接続・channel 配線・shutdown 伝播は trait の default impl が処理
  - 効果: component_architecture.md のルールが trait の compile-time invariant になり、新規コンポーネント追加時の boilerplate ゼロ
  - 工数 M、リスク 中 (4 component 同時マイグレーション)、PR 3-4
- [ ] **R-D11**: `X743WaveformStats::analyze` (mod.rs:80–191) を `src/reader/x743_waveform/` サブモジュールへ抽出。 R-D3 後の延長
- [ ] **R-D12** (optional, buffer): `DecoderKind` enum → `Box<dyn Decoder>` trait object 化。hot path 影響を bench で確認してから判断、 影響あれば revert

### Week 11 (2026-07-20〜) — Stabilize + 7/24 実験前準備

- [ ] **R-X3 post-refactor (NEW)**: Phase 1 baseline と同条件で再測定し、 `docs/plans/zmq_boundary_cost_2026-Q2.md` に「Post-refactor (Phase 3 後)」として追記。 baseline ⟷ post の差分表 + 解釈 (ComponentRunner で boilerplate 削った効果が encode/decode/送受信時間にどう出たか) を結論として記述
- [ ] hardware-in-the-loop dry run: PSD2 / PHA2 / V1743 各 10 分以上、 0 events 検出、loopback mismatch 0
- [ ] 回帰テスト: `cargo test --release && cargo clippy --release --tests -- -D warnings && ng test`
- [ ] `git tag refactor-sprint-2026-q2-complete`、 7/24 実験本番に投入

**完了基準**:
- component system 維持のまま `reader/mod.rs` < 1500 行
- ComponentRunner 経由で 4 component の boilerplate 削減
- 7/24 実験で **新 binary が March bin と同等以上の rate で安定動作**

## Out of Scope

- **Pure 一枚岩 (D3 撤回)** — 性能・可読性の両面で投資対効果が見えず、 cross-machine 配備柔軟性も失うので 7/24 後も再検討予定なし。 将来再検討するなら D8 (= R-X3) benchmark 結果が出発点
- **新 FW 追加 (V2745, DT5725)** — 7/24 後、capability table (R-C3) 整備後の方が楽
- **ROOT スキーマ変更 / wire format breaking change** — `EventData` は既に Phase 4.5 で probe_type 追加済、今 sprint では追加変更しない
- **C++ EventBuilder 撤退** — 別タスク (online EB Phase 4)
- **PHA FW wedge SOP の自動化** — memory `pha_fw_misbehavior_sop`、現状の post-Start 確認運用を維持
- **TIME_STEP_NS 動的化** — DT5725 (250 MS/s) サポート時、別 PR
- **process 境界の変更** — D7 で凍結、Phase 3 R-P8 は process 数を変えない (boilerplate 削減のみ)

## Risk Register

| Risk | Mitigation |
|---|---|
| Phase 2 で X743 hardware-only path を壊す | R-D3/D11 を Phase 3 に delay、Week 9-11 で実機検証 |
| `merge_field!` proc macro (R-C2) で serde 互換崩す | snapshot test + JSON round-trip test を先に追加してから着手 |
| **R-P8 ComponentRunner で 4 component 同時マイグレーション失敗** (NEW) | 1 component (Merger 推奨、最も小さい) で先行 PR、緑になってから他 3 へ展開。 PR ごとに smoke test (start_daq / stop_daq) |
| 来週ランで March bin と master の挙動差が見つかる | refactor PR を一旦 stash、March bin 側 patch → master 反映 |
| Phase 3 D8 benchmark でハードウェア占有競合 | 7/24 実験前の hardware-in-the-loop と日程衝突しないよう Week 11 buffer 内で実施、優先度 (validation > benchmark) を厳守 |

## Success Criteria

- `cargo test --release` 568 → 600+ tests pass
- `cargo clippy --release --tests -- -D warnings` clean
- `ng test` 57+ tests pass、 `ng build` clean
- `src/reader/mod.rs` < 1500 行 (Phase 3 後)
- `src/config/digitizer.rs` < 1900 行
- `src/bin/` production 15 / dev 27 を `dev-tools` feature-gate で隔離
- 重複検出: PSD1↔PHA1 / PSD2↔PHA2 で意味的同一の関数ゼロ
- production `Result::unwrap()` ゼロ
- 4 pipeline component (Reader / Merger / Recorder / Monitor) が `ComponentRunner` 経由で起動、boilerplate 削減
- `docs/plans/zmq_boundary_cost_2026-Q2.md` に baseline + post-refactor の 2 点 benchmark + 差分結論
- 7/24 実験で **新 binary が March bin と同等以上の rate で安定動作**

## 関連ドキュメント

- [clean_architecture_evaluation.md](../clean_architecture_evaluation.md) — 28 candidates 詳細 + 工数/リスク/Impact (revised 2026-05-05 evening、§2.5 R-A* 削除、§3 Phase 3 書き換え、§5 Decisions に D7/D8 追加)
- [docs/component_architecture.md](../docs/component_architecture.md) — タスク分離 + mpsc 現状アーキテクチャ (D7 で維持決定)
- memory: [architecture_reflection](/Users/aogaki/.claude/projects/-Users-aogaki-WorkSpace-delila-rs/memory/architecture_reflection.md) — 2026-04-22 の前段議論、2026-05-05 evening に「component 維持で進む」と決定
- memory: [layering_principle_clock_sync](/Users/aogaki/.claude/projects/-Users-aogaki-WorkSpace-delila-rs/memory/layering_principle_clock_sync.md) — refactor 中も守る原則
