# TODO 52: Refactor Sprint 2026-Q2 (post-PHA2 cleanup, pre-7/24 experiment)

**Status:** 📋 計画中 (2026-05-12 着手予定)
**Created:** 2026-05-05 (revised 2026-05-05 evening: 一枚岩撤回 → component system 維持)
**Window:** 2026-05-12 〜 2026-07-03 (8 週間 active refactor) + 2026-07-03 〜 2026-07-24 (3 週間 stabilize)
**Source of truth:** [clean_architecture_evaluation.md](../clean_architecture_evaluation.md)

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

## Phase 1 — Mechanical Cleanup (Week 1–3, 2026-05-12 〜 2026-05-30)

低リスク・動作不変。各 PR は独立 test 緑、来週ラン (March bin) と並行可。**変更なし**。

- [ ] **R-X3 baseline (NEW)**: ZMQ 境界 cost benchmark の baseline 測定 (Week 1、 重複削減着手前)。 `bin/zmq_boundary_bench` 新設、 emulator → reader → merger → recorder で 10k / 100k / 1M ev/s の 3 段階で計測。 結果を `docs/plans/zmq_boundary_cost_2026-Q2.md` に「Baseline (refactor 前)」として記録
- [ ] **R-D4**: `state_rank` / `effective_state_for` / `next_reconnect_cooldown` → `src/reader/state.rs`
- [ ] **R-D8**: `extract_bits!(word, shift, mask)` macro → `common.rs`
- [ ] **R-D9**: 関数名統一 (`decode_charge_word` ↔ `decode_energy_word` 等)
- [ ] **R-D10**: `opendpp_to_event_data` doc + smell 整理
- [ ] **R-C3**: `FirmwareCapabilities` 構造体 (is_dig1 / is_x743 / is_felib / num_channels / api_version)
- [ ] **R-C4**: DevTree path 文字列定数化 (`mod devtree_paths`)
- [ ] **R-C5**: `set_in_run_param_names()` を base + diff 化
- [ ] **R-C6**: `dev-tools` feature-gate 27 dev binary (D1)
- [ ] **R-P1**: `network::zmq_helper` で HWM=0 socket init を encapsulate
- [ ] **R-P2**: ECharts wrapper 統合 (histogram-chart ↔ heatmap-chart)
- [ ] **R-P7**: Operator に `CommandHandlerExt` 実装
- [ ] **R-X2**: `cargo audit` + `cargo outdated` を CI に追加

**完了基準**: 568 tests 緑、clippy clean、binary 整理、CI で audit/outdated 走る、`-300 LoC` 程度。

## Phase 2 — Structural Refactor (Week 4–8, 2026-06-02 〜 2026-07-03)

中リスク。各 PR で test を厚く張ってから着手。Week 8 末で baseline freeze。**変更なし**。

- [ ] **R-D6**: PSD1/PHA1 `Decoder<C: DecoderConfig>` generic 化 (D2、 3-4 PR)
  - `decoder/psd1_pha1_common.rs` 新設
  - `trait FineTimeCalculator` で `calculate_sw_fine_fraction_psd` ⟷ `_pha` を分岐
  - `trait DecoderConfig` で field naming (`charge_long` ⟷ `energy` 等) を抽象化
- [ ] **R-D7**: PSD2/PHA2 共用 aggregate parser (`decoder/dualchannel_common.rs`)
- [ ] **R-D1**: `read_loop_raw` (1369–1812) → `src/reader/read_loop_dig1.rs`
- [ ] **R-D2**: `read_loop_opendpp` (1812–2261) → `src/reader/read_loop_dig2.rs`
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
