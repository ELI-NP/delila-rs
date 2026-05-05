# Clean Architecture Evaluation — delila-rs

**作成:** 2026-05-05
**目的:** 7/24 次実験までの 11 週間 refactor sprint 設計のための棚卸し
**前提:** 来週 (~5/12) のランは March bin で固定 → 本 refactor は次実験 (2026-07-24) を gate しない

---

## Executive Summary

7 FW (PSD1/PSD2/PHA1/PHA2/AMax/V1743/X743Std) サポート完了、568 unit test 緑。コードベースは「機能は揃った」状態で、可読性を圧迫している重複が 3 系統蓄積している:

1. **Decoder family duplication** — PSD1 ↔ PHA1 は ~95% 一致 (1878/1816 行)。`read_u32` / `decode_extras_word` / `calculate_timestamp` / `BoardHeader` / `DualChannelHeader` が両方に存在。PSD2 ↔ PHA2 も aggregate header は同一構造、per-event 2nd word のみ差異。
2. **`src/reader/mod.rs` が 4042 行** に肥大 — 3 種類の ReadLoop (DIG1 / DIG2 / X743) + 接続管理 + state machine + decoder dispatch + Reader 本体が 1 ファイルに同居。
3. **`src/config/digitizer.rs` が 2890 行** — `add_channel_params()` の per-FW 6 ブランチ計 366 行は Input / Trigger / Energy / Coincidence / Waveform セクションをそれぞれ繰り返し記述、構造はほぼ同じで path/value だけ違う。`merge_field!` は 38 fields を手動列挙。

副次的に 42 個の binary (production 15 / dev 27)、16 frontend component (`digitizer-settings` だけで 1265 行)、`AppState` RwLock 12 個競合、ECharts wrapper 重複。

**結論:** **3 フェーズ 11 週間 sprint** を推奨。

| Phase | 期間 | 性格 | 対象 R |
|---|---|---|---|
| 1. Mechanical | Week 1–3 | 動作不変、低リスク、PR を寝かせ可能 | R-D4, R-D8, R-D9, R-C3, R-C4, R-C5, R-C6, R-P1, R-P2, R-P7 |
| 2. Structural | Week 4–8 | 中リスク、テスト依存、 高 impact | R-D1, R-D2, R-D6, R-D7, R-C1, R-C2, R-P3, R-P4, R-P5 |
| 3. Architectural | Week 9–11 | benchmark 必須、判断 gate | R-A1 (in-process pipeline), R-D11/D12 残作業 |

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
- **未計測**: 単一マシン内 ZMQ 境界 cost (MessagePack encode/decode + memcpy) — Phase 3 の判断材料が欠けている

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

### 2.4 Cross-cutting / Hygiene (R-X*)

| ID | Title | 工数 | リスク | Impact | PR数 |
|---|---|---|---|---|---|
| **R-X1** | Production `unwrap()` audit & reduction — `Mutex::lock().unwrap()` (poisoning panic propagate) は許容、 `Result::unwrap()` from I/O/parse は `?` 伝播、 `Option::unwrap()` in hot path は invariant 不明なら `expect("...")` に。`reader/mod.rs:30` / `config/mod.rs:30` / `caen/handle.rs:26` / `chunk_builder.rs:18` あたりが top hot spot | M | 低 | 中 | 2-3 |
| **R-X2** | `cargo audit` (security advisories) + `cargo outdated` (依存更新可視化) を pre-commit / CI に追加 | S | 低 | 低 | 1 |

**経緯**: Gemini Flash refactoring suggestion (`REFACTORING_SUGGESTIONS.md`, 2026-05-05) で指摘された 2 件、 本 audit が拾い損ねていたので catalog に追加。

### 2.5 アーキテクチャ判断 (R-A*)

D3 で **Pure 一枚岩採用済** (2026-05-05)。Phase 3 で実装する。

| ID | Title | 工数 | リスク | Impact | PR数 | 備考 |
|---|---|---|---|---|---|---|
| **R-A1** | Monolithic 化 — Reader/Merger/Recorder/Monitor/Operator を 1 process、tokio mpsc + `Arc<[u8]>` zero-copy。 cross-machine は別 binary (aggregator) に縮退 | L | 中 | **高** | 4–6 | D3 で採用済、Phase 3 |
| R-A2 | Quick benchmark (D4) — ZMQ vs in-process の Throughput / Latency 比較を 1 日で取得 | S | 低 | 低 | 1 | Phase 3 Week 9 頭で実施、 結果は社内記録 |

**Cross-machine 構成**: 各物理マシンで monolithic delila-rs を起動、 マシン間は ZMQ で edge 接続 (現状の online_event_builder と同方向性、自然な階層化)。

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

### Phase 3 — Monolithic Consolidation + Stabilize (Week 9–11)

D3 (Pure 一枚岩) 採用済なので Phase 3 は **monolithic 化実装** + **実機 stabilize**。

| Week | タスク |
|---|---|
| 9 | **Quick benchmark (1 日)** ZMQ vs in-process baseline 取得 (D4)。続けて monolithic skeleton: 1 process に Reader/Merger/Recorder/Monitor/Operator を tokio task で集約、tokio mpsc + `Arc<[u8]>` 接続、 ZMQ 境界は cross-machine config-flag 用に縮退 |
| 10 | monolithic 移行 + 既存 binary 整理 (`src/bin/{merger,recorder,monitor,operator}.rs` を thin wrapper 化 or 撤廃)、 cross-machine 用 `delila-rs-aggregator` (or 既存 online_event_builder 拡張) 整備 |
| 11 | 7/24 実験前 stabilize、回帰テスト + 実機 dry run + benchmark 後計測 (Latency 改善値を記録) |

期待成果: 単一プロセス delila-rs binary、 ZMQ 境界 5+ → 1-2 (cross-machine edge のみ)、 readable 向上。

---

## 4. Out of Scope / Deferred

- **新 FW 追加 (V2745, DT5725 等)** — 7/24 実験以降に検討、capability table (R-C3) 整備後の方が楽
- **ROOT スキーマ変更 / wire format breaking change** — `EventData` は既に Phase 4.5 で probe_type 追加済、今 sprint では追加変更しない
- **C++ EventBuilder 撤退** — 別タスク (online EB Phase 4)
- **PHA FW wedge SOP の自動化** — memory `pha_fw_misbehavior_sop`、現状の post-Start 確認運用を維持
- **TIME_STEP_NS 動的化** — DT5725 (250 MS/s) サポート時、別 PR
- **cross-machine aggregator 高度化** — Phase 3 では既存 online_event_builder を流用、専用 binary 切出しは 7/24 後

---

## 5. Decisions (2026-05-05 確定)

合意済の judgment call。以下を sprint 設計の基礎とする。

| # | 項目 | 決定 |
|---|---|---|
| D1 | Dev binary archive (R-C6) | **`dev-tools` feature-gate**。27 binary は `Cargo.toml` で `required-features = ["dev-tools"]` を付け、デフォルト build から外す。production 15 binary だけ `cargo build --release` で生成。 |
| D2 | R-D6 PSD1/PHA1 generic 深さ | **trait + generic (zero-cost)**。`Decoder<C: DecoderConfig>` を monomorphize、hot path overhead ゼロ。既存 `RolloverTracker<const BITS: u32>` と同パターン。 |
| D3 | 単一マシン内 component 境界 | **Pure 一枚岩**。Reader / Merger / Recorder / Monitor / Operator を 1 process に集約、tokio task + `Arc<[u8]>` zero-copy。ZMQ 境界は cross-machine deployment 用に縮退。 |
| D4 | D3 の benchmark | **Quick (1 日)**。 monolithic 化前後で Throughput + Latency (Stop→EOS / Apply→Configured / Start→first event) を測定、社内合意・記録用。設計判断 gate ではなく裏付け。 |
| D5 | 旧 `clean_architecture_evaluation.md` | **削除のまま、本ドキュメントが正規版**。旧は abstract Clean Architecture rubric (2026-04-15)、actionable 度ゼロのため merge 不要。 |
| D6 | Refactor 中の branch 戦略 | **Daily merge to master**。Solo dev、各 PR atomic & 独立 & test 緑を維持。`git tag march-bin-baseline` を切って来週ランとの境界を明示、 hot-fix 要時は March bin 側で patch。 |

D3 (Pure 一枚岩) が一番大きな design 変更なので、§3 Sprint Plan と §4 Out of Scope を以下のように更新する。

---

## 6. Risk Register

| Risk | Mitigation |
|---|---|
| Phase 2 でテストが弱い領域 (X743 hardware-only path) を壊す | R-D3/D11 を Phase 3 後半に回す、Phase 9-10 で実機検証 |
| `merge_field!` proc macro (R-C2) で serde 互換崩す | snapshot test + JSON round-trip test を先に追加してから着手 |
| in-process pipeline 化 (R-A1) で multi-machine 構成を壊す | feature flag `single-process` を導入、既存 ZMQ path を残す |
| 来週の実験で March bin と 5/5 master の挙動差が見つかる | refactor PR を一旦 stash、March bin 側にバックポートしてから master を進める |

---

## 7. Success Criteria

Phase 終了時 (2026-07-17 想定):
- `cargo test --release` 568 → 600+ tests pass
- `cargo clippy --release --tests -- -D warnings` clean
- `ng test` 57+ tests pass、 `ng build` clean
- `src/reader/mod.rs` < 2000 行
- `src/config/digitizer.rs` < 1900 行
- `src/bin/` production 15 / dev は隔離
- 重複検出ツール (例: `dust` + 手動レビュー) で「PSD1 vs PHA1 / PSD2 vs PHA2 で意味的同一の関数」がゼロ
- 7/24 実験で **新 binary が March bin と同等以上の rate** で安定動作

---

## Appendix A — 元の audit reports

3 並列 Explore agent 出力が source。本ドキュメントはその consolidate。詳細 file:line citation は各 audit に保持。
