# Clean Architecture Evaluation — delila-rs

**作成:** 2026-05-05
**目的:** 7/24 次実験までの 11 週間 refactor sprint 設計のための棚卸し
**前提:** 来週 (~5/12) のランは March bin で固定 → 本 refactor は次実験 (2026-07-24) を gate しない

## Changelog

- **2026-05-05 evening**: D3 (Pure 一枚岩) と D4 (gating benchmark) を **撤回**。 component system 維持で進めると決定 (新 D7)、ZMQ 境界 benchmark は optional に格下げ (新 D8)。 §3 Phase 3 を「monolithic 化」から「component lifecycle 統一 + 延期 reader split + stabilize」に書き換え、 §2.5 R-A* を削除、 新 R-P8 (Component lifecycle trait) を §2.3 に追加。 詳細は [TODO 52](TODO/52_refactor_sprint_2026-q2.md) 「方針 (revised)」参照。
- **2026-05-05 朝**: 初版作成、Gemini Flash の `REFACTORING_SUGGESTIONS.md` を統合 (R-X1 / R-X2 を §2.4 として吸収)。

---

## Executive Summary

7 FW (PSD1/PSD2/PHA1/PHA2/AMax/V1743/X743Std) サポート完了、568 unit test 緑。コードベースは「機能は揃った」状態で、可読性を圧迫している重複が 3 系統蓄積している:

1. **Decoder family duplication** — PSD1 ↔ PHA1 は ~95% 一致 (1878/1816 行)。`read_u32` / `decode_extras_word` / `calculate_timestamp` / `BoardHeader` / `DualChannelHeader` が両方に存在。PSD2 ↔ PHA2 も aggregate header は同一構造、per-event 2nd word のみ差異。
2. **`src/reader/mod.rs` が 4042 行** に肥大 — 3 種類の ReadLoop (DIG1 / DIG2 / X743) + 接続管理 + state machine + decoder dispatch + Reader 本体が 1 ファイルに同居。
3. **`src/config/digitizer.rs` が 2890 行** — `add_channel_params()` の per-FW 6 ブランチ計 366 行は Input / Trigger / Energy / Coincidence / Waveform セクションをそれぞれ繰り返し記述、構造はほぼ同じで path/value だけ違う。`merge_field!` は 38 fields を手動列挙。

副次的に 42 個の binary (production 15 / dev 27)、16 frontend component (`digitizer-settings` だけで 1265 行)、`AppState` RwLock 12 個競合、ECharts wrapper 重複。

**結論:** **3 フェーズ 11 週間 sprint** を推奨 (component system 維持、 D7)。

| Phase | 期間 | 性格 | 対象 R |
|---|---|---|---|
| 1. Mechanical | Week 1–3 | 動作不変、低リスク、PR を寝かせ可能 | R-D4, R-D8, R-D9, R-D10, R-C3, R-C4, R-C5, R-C6, R-P1, R-P2, R-P7, R-X2 |
| 2. Structural | Week 4–8 | 中リスク、テスト依存、 高 impact | R-D1, R-D2, R-D6, R-D7, R-C1, R-C2, R-P3, R-P4, R-P5, R-X1 |
| 3. Component hardening + stabilize | Week 9–11 | delayed reader split + lifecycle 統一 + 7/24 実験前検証 | R-D3, R-D5, R-D11, R-D12 (optional), R-P6, **R-P8 (new)**, hardware dry run, D8 (optional bench) |

Week 8 で baseline freeze、Week 9–11 を hardware-in-the-loop 検証に充てれば 7/24 実験の 3 週間前に安定化できる。

---

## 1. 現状サマリ

### 1.1 規模

| 領域 | LoC | 主要ファイル |
|---|---:|---|
| `src/reader/` | ~5500 | `mod.rs` 4042 (!)、`decoder/{psd1,pha1}.rs` 1878/1816、`decoder/{psd2,pha2}.rs` 1316/1067、`caen/handle.rs` ~1500 |
| `src/config/` | ~3400 | `digitizer.rs` 2890 (!) |
| `src/{merger,recorder,monitor,operator,event_builder,common}/` | 21,371 | `monitor/mod.rs` 1847、`recorder/mod.rs` 1022、`event_builder/chunk_builder.rs` 1046、`operator/routes/*` 4066 |
| `src/bin/` | 42 binary (Cargo.toml 38) | production 15、dev 27 |
| `web/operator-ui/src/app/` | ~14,800 | `digitizer-settings` 1265、`waveform` page 1702 |

### 1.2 共通インフラの浸透状況

| 仕組み | 浸透度 | 備考 |
|---|---|---|
| `CommandHandlerExt` trait (src/common/state.rs) | 8/10 | Operator 未実装 (RwLock 直接) |
| `command_task.rs` (190 LoC) | 全コンポーネント | OK |
| HWM=0 ZMQ ポリシー | ✓ 全境界 | 各コンポーネントで `set_rcvhwm/sndhwm(0)` 手書き 5 箇所 |
| `RolloverTracker` (rollover.rs) | ✓ 全 FW | 統一済み (V1743 で完了) |
| `Waveform` / `EventData` 共通型 (common.rs) | ✓ 全 FW | 統一済み |
| `param_cache` + `apply_validated_parameters` | ✓ 全 FW | case-insensitive、loopback verify 統一 |
| `sign_extend_14bit` | ✓ common.rs | 統一済み |
| Frontend signals (`signal()` / `computed()`) | ~90% | BehaviorSubject 22 ref残るが大半 signal 化 |

底は思ったより整っている。残った重複は **decoder hot path** と **config apply path** に集中。

### 1.3 既存 benchmark

- ROOT 出力: 0.79M ev/s/writer (実運用 300k に対し 2.6x マージン) [`docs/plans/oxyroot_benchmark_results.md`](docs/plans/oxyroot_benchmark_results.md)
- Decoder hot path: PSD2 7M ev/10s 並列化済 (memory: `DecodeLoop Parallelization 2026-02-23`)
- **未計測**: 単一マシン内 ZMQ 境界 cost (MessagePack encode/decode + memcpy) — D8 で optional 計測。判断 gate ではなく将来の意思決定 (例: monolithic 再検討) のための記録

---

## 2. Refactor Catalog

### 2.1 Reader / Decoder (R-D*)

| ID | Title | 工数 | リスク | Impact | PR数 |
|---|---|---|---|---|---|
| **R-D1** | `read_loop_raw` (1369–1812) → `src/reader/read_loop_dig1.rs` | L | 低 | 中 | 2 |
| **R-D2** | `read_loop_opendpp` (1812–2261) → `src/reader/read_loop_dig2.rs` | L | 低 | 中 | 2 |
| **R-D3** | `read_loop_x743_std` (2261–2741) + `x743_std_event_to_event_data` (2741–2913) → `src/reader/read_loop_x743.rs` | M | 中 | 中 | 2 |
| **R-D4** | `state_rank` / `effective_state_for` / `next_reconnect_cooldown` → `src/reader/state.rs` | S | 低 | 低 | 1 |
| **R-D5** | `try_connect_raw/opendpp` + DeviceConnection → `src/reader/connection.rs` | M | 中 | 低 | 1–2 |
| **R-D6** | **PSD1 ↔ PHA1 generic decoder**: `Decoder<Config: DecoderConfig>` + trait `FineTimeCalculator` で 95% 重複を統合。`decoder/psd1_pha1_common.rs` 新設 | M | 中 | **高** | 3–4 |
| **R-D7** | **PSD2 ↔ PHA2 共用 aggregate parser**: `decoder/dualchannel_common.rs`、per-event word 解釈は trait method で分岐 | M | 中 | 高 | 2–3 |
| **R-D8** | `extract_bits!(word, shift, mask)` macro → `common.rs` | S | 低 | 低 | 1 |
| **R-D9** | 関数名統一 (`decode_charge_word` ↔ `decode_energy_word` 等) | S | 低 | 低 | 1 |
| R-D10 | `opendpp_to_event_data` (mod.rs:386–445) doc + smell 整理 | S | 低 | 低 | 1 |
| R-D11 | X743 waveform analyzer abstraction (`X743WaveformStats::analyze` 80–191) | M | **高** | 中 | 2–3 |
| R-D12 | `DecoderKind` enum → trait object (`Box<dyn Decoder>`) | M | 中 | 中 | 2–3 |

**重要 citation**:
- PSD1↔PHA1 重複の核: [`src/reader/decoder/psd1.rs:706–820`](src/reader/decoder/psd1.rs) ⟷ [`src/reader/decoder/pha1.rs:715–820`](src/reader/decoder/pha1.rs) (`read_u32`, `decode_extras_word`, `calculate_timestamp` ほぼ同一、唯一の実質差異は `calculate_sw_fine_fraction_psd` (ADC_MIDPOINT=8192.0 baseline) vs `_pha` (signed)).
- mod.rs の section 境界は Reader/Decoder audit の表参照。

### 2.2 Config / Binaries (R-C*)

| ID | Title | 工数 | リスク | Impact | PR数 |
|---|---|---|---|---|---|
| **R-C1** | `add_channel_params()` per-FW 6 branch (366 行) を **static `HashMap<FirmwareType, &[(ConfigField, &str)]>` 駆動の 1 loop** に。Input/Trigger/Energy/Coincidence/Waveform セクションの per-FW 重複を撲滅 | M | 中 | **高** | 3–4 |
| **R-C2** | `merge_field!` 38 fields の手動展開を `#[derive(Merge)]` proc macro に | M | 低 | 中 | 2–3 |
| **R-C3** | `FirmwareCapabilities` 構造体 — `is_dig1()` / `is_x743()` / `is_felib()` / `num_channels` / `api_version` を中央集権化 | S | 低 | 中 | 1 |
| **R-C4** | DevTree path 文字列定数化 (`mod devtree_paths { pub const CHANNELS_TRIGGER_MASK: &str = ...; }` 30+ 箇所) | S | 低 | 低 | 1 |
| **R-C5** | `set_in_run_param_names()` 6 FW branch (PSD2/AMax は 29/36 共通) を base + diff 化 | S | 低 | 低 | 1 |
| **R-C6** | dev binaries (27 個) に `dev-tools` feature-gate を付与 — `Cargo.toml` の `[[bin]]` で `required-features = ["dev-tools"]`。default build から外し、 `cargo build --release` を production 15 binary に絞る (D1 確定) | S | 低 | 中 | 1 |
| R-C7 | per-FW test binary (fine_ts_verify / trigger_loss_test / psd1_waveform_test / x743_*) を共通 harness 化 | L | 中 | 中 | 3 |

**重要 citation**:
- `ChannelConfig` 70 fields, FW 別利用率: 共通 ~22 / PSD2+AMax 17 / PHA2 12 / DIG1 13 / X743 2-nested
- `add_channel_params` PSD2+AMax branch [src/config/digitizer.rs:1719–1842] (123 行)、PHA2 [:1843–2010] (167 行、PSD2 と Input/Trigger 重複)
- `sanitize_for_firmware` [:1065–1092] は DIG1 path で 9 None 代入のみ — PHA2 / AMax 固有 dead field の wipe 漏れ要確認

**Production binary (15)**: emulator, data_sink, merger, controller, reader, recorder, monitor, event_bridge, event_builder, online_event_builder, node_agent, operator, recover, event_dump, delila_to_root

**Dev binary (27, archive 候補)**: amax_codegen, amax_data_test, amax_energy_check, amax_firmware_check, amax_fw_test, amax_opendpp_test, amax_rawdump, amax_register_test, amax_selftrigger_test, amax_testpulse_test, amax_trgout_test, caen_info, caen_simple_test, configure_benchmark, eb_test_sender, fine_ts_verify, oxyroot_bench, psd1_raw_dump, psd1_timing_test, psd1_waveform_test, register_test, storage_bench, trigger_loss_test, trigger_loss_test_dig2, x743_cycle_test, x743_stop, x743_test

### 2.3 Pipeline / Operator (R-P*)

| ID | Title | 工数 | リスク | Impact | PR数 |
|---|---|---|---|---|---|
| **R-P1** | `network::zmq_helper` で HWM=0 ソケット初期化を encapsulate (5 sites duplicated) | S | 低 | 低 | 1 |
| **R-P2** | ECharts wrapper 統合 — histogram-chart (538) ↔ heatmap-chart (269) は build/tooltip/toolbox で 30% overlap | S | 低 | 中 | 1 |
| **R-P3** | Operator `AppState` RwLock 12 fields → DashMap or RCU read cache、GET /api/status 競合解消 | M | 中 | 中 | 2–3 |
| **R-P4** | digitizer/event-builder/emulator settings の base settings panel パターン化 | M | 中 | 中 | 2–3 |
| **R-P5** | `digitizer.rs` (983) の 14 CRUD handler を `crud_get<T>/crud_update<T>/crud_save<T>` factory 化 | M | 中 | 中 | 2–3 |
| R-P6 | event_builder routes (565) の ch_settings / l2_settings / time_settings parallel structure 統合 | M | 中 | 低 | 1–2 |
| **R-P7** | Operator に `CommandHandlerExt` 実装 (現在は RwLock 直接) — 他 8 component と整合 | S | 低 | 低 | 1 |
| **R-P8** | (NEW 2026-05-05 evening) `Component` trait + `ComponentRunner` builder 抽出。 [docs/component_architecture.md](docs/component_architecture.md) の Receiver/Main/Sender/Command パターンを 4 component (Reader/Merger/Recorder/Monitor) で手書き反復している配線を 1 trait に集約。 各 component 本体はビジネスロジックのみ、 ZMQ 接続・channel 配線・shutdown 伝播は trait の default impl で処理。 component_architecture.md ルールが compile-time invariant に昇格 | M | 中 | **高** | 3-4 |

### 2.4 Cross-cutting / Hygiene (R-X*)

| ID | Title | 工数 | リスク | Impact | PR数 |
|---|---|---|---|---|---|
| **R-X1** | Production `unwrap()` audit & reduction — `Mutex::lock().unwrap()` (poisoning panic propagate) は許容、 `Result::unwrap()` from I/O/parse は `?` 伝播、 `Option::unwrap()` in hot path は invariant 不明なら `expect("...")` に。`reader/mod.rs:30` / `config/mod.rs:30` / `caen/handle.rs:26` / `chunk_builder.rs:18` あたりが top hot spot | M | 低 | 中 | 2-3 |
| **R-X2** | `cargo audit` (security advisories) + `cargo outdated` (依存更新可視化) を pre-commit / CI に追加 | S | 低 | 低 | 1 |

**経緯**: Gemini Flash refactoring suggestion (`REFACTORING_SUGGESTIONS.md`, 2026-05-05) で指摘された 2 件、 本 audit が拾い損ねていたので catalog に追加。

### 2.5 アーキテクチャ判断 (R-A*) — **撤回 (2026-05-05 evening)**

**初版 (2026-05-05 朝) で D3 として「Pure 一枚岩」を採用していたが、 同日 evening に component system 維持で進めると方針変更**。 R-A1 / R-A2 は本 catalog から削除済。

撤回理由:
- 既存 component アーキテクチャ ([docs/component_architecture.md](docs/component_architecture.md)) は健全で、 Receiver/Main/Sender/Command 分離 + lock-free shared state がすでに確立済
- cross-machine 配備 (172.18.4.56/76/147 + gant) は実運用、 process 隔離による配備柔軟性は手放せない
- 既存性能ボトルネックは別の場所で対処済 (decode loop 並列化 7M ev/s、ROOT 出力 0.79M ev/s/writer × 2.6x マージン)
- **重複削減** (R-D6 PSD1/PHA1 generic、 R-C1 param tables 等) で「readable」効果は同等以上に得られる、リスクは低い

将来 monolithic を再検討するときの判断材料として、**D8 で optional に ZMQ 境界 cost を 1 日 benchmark** することは可能 (Phase 3 Week 11 buffer)。 結果は記録のみ、 sprint 設計には連動しない。

その代わり Phase 3 で **R-P8 Component lifecycle trait** (§2.3 参照) を導入し、 component 内部の boilerplate 削減で「component-system のまま readable に」を実現する。

---

## 3. Sprint Plan (11 週間)

### Phase 1 — Mechanical Cleanup (Week 1–3)

低リスク・動作不変な機械的整理。来週ラン (March bin) と並行可、PR を寝かせやすい。

| Week | タスク | 完了基準 |
|---|---|---|
| 1 | R-D4, R-D8, R-D9, R-C4, **R-X2** (cargo audit + outdated を CI に) | 568 tests 緑、clippy clean、CI で audit/outdated 走る |
| 2 | R-C3, R-C5, R-C6, R-P1, R-P7 | binary 整理 + zmq_helper landed |
| 3 | R-D10, R-P2 (ECharts) + buffer | ng test 緑 |

期待成果: -300 LoC、binary 42→17 active、 utility 集約。

### Phase 2 — Structural Refactor (Week 4–8)

中リスク、各 PR で test を厚くする。Phase 2 末尾 (Week 8) で baseline freeze。

| Week | タスク | 完了基準 |
|---|---|---|
| 4 | R-D6 PSD1/PHA1 generic Decoder<Config> (3–4 PR) | 既存 PSD1/PHA1 unit test 全緑 |
| 5 | R-D7 PSD2/PHA2 dualchannel_common (2–3 PR) | 同上 |
| 6 | R-D1, R-D2 read_loop split (4 PR) | mod.rs ~4042→2000 行 |
| 7 | R-C1 param map tables + R-C2 derive macro | digitizer.rs ~2890→1800 行 |
| 8 | R-P3, R-P4, R-P5, **R-X1** (production unwrap audit + reduction) + **baseline freeze** | clippy + ng test + cargo test 全緑、production `Result::unwrap()` 0 |

期待成果: 重複 -30%、`reader/mod.rs` -50%、`digitizer.rs` -38%。

### Phase 3 — Component System Hardening + Stabilize (Week 9–11)

D7 で component system 維持決定。 Phase 3 は **delayed reader split + component lifecycle 統一 (R-P8) + 7/24 実験前 stabilize**。

| Week | タスク |
|---|---|
| 9 | **R-D3** (X743 read_loop split) + **R-D5** (Connection mgmt extract) — Phase 2 で X743 hardware-only テスト不足 + reader 周辺 churn を避けるため delay。 並行で **R-P6** (event_builder routes consolidation) |
| 10 | **R-P8** (NEW) Component lifecycle trait + ComponentRunner builder。 4 component (Reader/Merger/Recorder/Monitor) に展開、 1 component (Merger 推奨) で先行 PR → 緑になってから残り 3 へ。 buffer で **R-D11** (X743 waveform analyzer) と **R-D12** (DecoderKind trait object、 hot path 影響を bench で確認) |
| 11 | hardware-in-the-loop dry run (PSD2 / PHA2 / V1743 各 10 分以上)、回帰テスト全緑、 余裕あれば **D8 optional benchmark** で ZMQ 境界 cost を記録 (将来 monolithic 再検討の出発点)、 `git tag refactor-sprint-2026-q2-complete` |

期待成果: component system 維持のまま `reader/mod.rs` < 1500 行、 4 component が `ComponentRunner` 経由起動、 boilerplate 削減、 7/24 実験で March bin 同等以上の安定動作。

---

## 4. Out of Scope / Deferred

- **Pure 一枚岩 (旧 D3、撤回)** — D7 で component system 維持に決定、 7/24 後も再検討予定なし。 将来再検討するなら D8 optional benchmark 結果が出発点
- **新 FW 追加 (V2745, DT5725 等)** — 7/24 実験以降に検討、capability table (R-C3) 整備後の方が楽
- **ROOT スキーマ変更 / wire format breaking change** — `EventData` は既に Phase 4.5 で probe_type 追加済、今 sprint では追加変更しない
- **C++ EventBuilder 撤退** — 別タスク (online EB Phase 4)
- **PHA FW wedge SOP の自動化** — memory `pha_fw_misbehavior_sop`、現状の post-Start 確認運用を維持
- **TIME_STEP_NS 動的化** — DT5725 (250 MS/s) サポート時、別 PR
- **process 境界の変更** — D7 で凍結。 R-P8 ComponentRunner は process 数を変えない (boilerplate 削減のみ)、 cross-machine 配備柔軟性を保持

---

## 5. Decisions (2026-05-05 確定、 evening 改版)

合意済の judgment call。以下を sprint 設計の基礎とする。

| # | 項目 | 決定 |
|---|---|---|
| D1 | Dev binary archive (R-C6) | **`dev-tools` feature-gate**。27 binary は `Cargo.toml` で `required-features = ["dev-tools"]` を付け、デフォルト build から外す。production 15 binary だけ `cargo build --release` で生成。 |
| D2 | R-D6 PSD1/PHA1 generic 深さ | **trait + generic (zero-cost)**。`Decoder<C: DecoderConfig>` を monomorphize、hot path overhead ゼロ。既存 `RolloverTracker<const BITS: u32>` と同パターン。 |
| ~~D3~~ | ~~単一マシン内 component 境界 = Pure 一枚岩~~ | **撤回 (2026-05-05 evening)**。 D7 (component system 維持) に置き換え。 旧 D3 採用理由は「readable + zero overhead」だったが、 重複削減 (R-D6 等) でも同じ効果が低リスクで得られる + cross-machine 配備柔軟性を失えない、 と再評価。 |
| ~~D4~~ | ~~D3 の gating benchmark~~ | **撤回 (2026-05-05 evening)**。 判断 gate ではなくなったので不要。 D8 で optional 計測のみ残す。 |
| D5 | 旧 `clean_architecture_evaluation.md` | **削除のまま、本ドキュメントが正規版**。旧は abstract Clean Architecture rubric (2026-04-15)、actionable 度ゼロのため merge 不要。 |
| D6 | Refactor 中の branch 戦略 | **Daily merge to master**。Solo dev、各 PR atomic & 独立 & test 緑を維持。`git tag march-bin-baseline` を切って来週ランとの境界を明示、 hot-fix 要時は March bin 側で patch。 |
| **D7** | アーキテクチャ方針 (NEW 2026-05-05 evening) | **現行 component system (Reader/Merger/Recorder/Monitor/Operator が ZMQ で繋がる process 隔離型) を維持**。重複削減・構造整理は component 内部 + 横断 trait 化 (R-P8 ComponentRunner) で対処、 process 境界は触らない。 cross-machine 配備柔軟性を保持。 |
| **D8** | ZMQ 境界 cost benchmark (NEW 2026-05-05 evening) | **optional、 低優先**。 Phase 3 Week 11 buffer に余裕があれば 1 日かけて記録のみ取る。 設計判断 gate ではなく将来 monolithic 再検討時の出発点。 |

---

## 6. Risk Register

| Risk | Mitigation |
|---|---|
| Phase 2 でテストが弱い領域 (X743 hardware-only path) を壊す | R-D3/D11 を Phase 3 後半に回す、Week 9-11 で実機検証 |
| `merge_field!` proc macro (R-C2) で serde 互換崩す | snapshot test + JSON round-trip test を先に追加してから着手 |
| **R-P8 ComponentRunner で 4 component 同時マイグレーション失敗** | 1 component (Merger 推奨、最も小さい) で先行 PR、緑になってから他 3 へ展開。 PR ごとに smoke test (start_daq / stop_daq) |
| 来週の実験で March bin と 5/5 master の挙動差が見つかる | refactor PR を一旦 stash、March bin 側にバックポートしてから master を進める |
| Phase 3 D8 benchmark でハードウェア占有競合 | 7/24 実験前 hardware validation と日程衝突しないよう Week 11 buffer 内で実施、 priority (validation > benchmark) を厳守 |

---

## 7. Success Criteria

Phase 終了時 (2026-07-20 想定):
- `cargo test --release` 568 → 600+ tests pass
- `cargo clippy --release --tests -- -D warnings` clean
- `ng test` 57+ tests pass、 `ng build` clean
- `src/reader/mod.rs` < 1500 行 (Phase 3 で R-D3/R-D5 完了後)
- `src/config/digitizer.rs` < 1900 行
- `src/bin/` production 15 / dev は `dev-tools` feature-gate で隔離
- 重複検出ツール (例: `dust` + 手動レビュー) で「PSD1 vs PHA1 / PSD2 vs PHA2 で意味的同一の関数」がゼロ
- production `Result::unwrap()` ゼロ (R-X1)
- 4 pipeline component (Reader / Merger / Recorder / Monitor) が `ComponentRunner` 経由で起動 (R-P8)
- 7/24 実験で **新 binary が March bin と同等以上の rate** で安定動作

---

## Appendix A — 元の audit reports

3 並列 Explore agent 出力が source。本ドキュメントはその consolidate。詳細 file:line citation は各 audit に保持。
