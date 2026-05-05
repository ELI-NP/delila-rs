# TODO 52: Refactor Sprint 2026-Q2 (post-PHA2 cleanup, pre-7/24 experiment)

**Status:** 📋 計画中 (2026-05-05 着手予定)
**Created:** 2026-05-05
**Window:** 2026-05-12 〜 2026-07-03 (8 週間 active refactor) + 2026-07-03 〜 2026-07-24 (3 週間 stabilize)
**Source of truth:** [clean_architecture_evaluation.md](../clean_architecture_evaluation.md)

## ゴール

7 FW (PSD1/PSD2/PHA1/PHA2/AMax/V1743/X743Std) サポート完了後の重複削減 + 単一 binary monolithic 化。**動作不変** + **可読性向上** + **OSS 公開に耐える整理状態**。

## 動機

- `src/reader/mod.rs` 4042 行、`src/config/digitizer.rs` 2890 行に肥大
- PSD1 ↔ PHA1 が ~95% 重複 (1878/1816 行)、PSD2 ↔ PHA2 も aggregate header 同一
- `src/bin/` 42 binary (production 15 / dev 27)、デフォルト build noise
- ZMQ 5+ 境界 + MessagePack encode/decode が単一マシン内で発生
- 来週 (2026-05-12〜) のラン中は March bin 使用なので master refactor は影響しない
- 7/24 次実験までに 11 週間の余裕あり

## 確定 Decisions (2026-05-05)

| # | 項目 | 決定 |
|---|---|---|
| D1 | Dev binary archive | `dev-tools` feature-gate (`required-features` に付与、 default build から外す) |
| D2 | PSD1/PHA1 共通化深さ | trait + generic (zero-cost monomorphization) |
| D3 | 単一マシン内 component 境界 | **Pure 一枚岩** (1 process, tokio task + `Arc<[u8]>`) |
| D4 | D3 の benchmark | Quick (1 日) — Throughput + Latency、社内記録用 |
| D5 | 旧 `clean_architecture_evaluation.md` (2026-04-15) | 削除のまま、新規 (2026-05-05) が正規版 |
| D6 | Branch 戦略 | Daily merge to master、`git tag march-bin-baseline` 運用 |

## Phase 1 — Mechanical Cleanup (Week 1–3, 2026-05-12 〜 2026-05-30)

低リスク・動作不変。各 PR は独立 test 緑、来週ラン (March bin) と並行可。

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

中リスク。各 PR で test を厚く張ってから着手。Week 8 末で baseline freeze。

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

## Phase 3 — Monolithic Consolidation + Stabilize (Week 9–11, 2026-07-06 〜 2026-07-24)

D3 (Pure 一枚岩) 採用済の実装 + 7/24 実験前 stabilize。

- [ ] Week 9 day 1: **R-A2** Quick benchmark (D4) — ZMQ vs in-process baseline
  - Throughput (events/s)
  - Latency (Stop → EOS、 Apply → Configured、 Start → first event)
  - 結果は `docs/plans/monolithic_benchmark_2026-07.md` に記録
- [ ] **R-A1**: Monolithic skeleton — Reader/Merger/Recorder/Monitor/Operator を 1 process、tokio mpsc + `Arc<[u8]>`、ZMQ 境界は cross-machine config-flag 用に縮退 (4-6 PR)
  - `src/bin/{merger,recorder,monitor,operator}.rs` を thin wrapper or 撤廃
  - cross-machine 用 aggregator は既存 `online_event_builder` を流用
- [ ] **R-D3**: `read_loop_x743_std` → `src/reader/read_loop_x743.rs` + `x743_decode_params.rs`
- [ ] **R-D5**: `try_connect_*` + DeviceConnection → `src/reader/connection.rs`
- [ ] **R-A1 後計測**: monolithic 化後の Throughput + Latency 取得、 改善値を記録
- [ ] 実機 dry run + 回帰テスト (PSD2 / PHA2 / V1743 各 10 分以上)

**完了基準**: 単一 delila-rs binary、 ZMQ 境界 5+ → 1-2、 7/24 実験で March bin と同等以上の rate で安定動作。

## Out of Scope

- 完全モノリシック化 (cross-machine 統合) — 7/24 後検討
- 新 FW 追加 (V2745, DT5725) — 7/24 後、capability table (R-C3) 整備後
- ROOT スキーマ変更 / wire format breaking change
- C++ EventBuilder 撤退 (online EB Phase 4 別タスク)
- PHA FW wedge SOP の自動化 (memory `pha_fw_misbehavior_sop` 維持)
- TIME_STEP_NS 動的化 (DT5725 サポート時別 PR)

## Risk Register

| Risk | Mitigation |
|---|---|
| Phase 2 で X743 hardware-only path を壊す | R-D3 を Phase 3 後半に回す、Week 9-10 で実機検証 |
| `merge_field!` proc macro (R-C2) で serde 互換崩す | snapshot test + JSON round-trip test を先に追加してから着手 |
| Monolithic 化 (R-A1) で multi-machine 構成を壊す | feature flag `single-process` を導入、 既存 ZMQ path を残す |
| 来週ランで March bin と master の挙動差が見つかる | refactor PR を一旦 stash、March bin 側 patch → master 反映 |
| Phase 3 benchmark で in-process が逆に遅い病的 case | 着手前に snapshot 取り、回帰時は revert で対処 |

## Success Criteria

- `cargo test --release` 568 → 600+ tests pass
- `cargo clippy --release --tests -- -D warnings` clean
- `ng test` 57+ tests pass、 `ng build` clean
- `src/reader/mod.rs` < 2000 行
- `src/config/digitizer.rs` < 1900 行
- `src/bin/` production 15 / dev 27 を `dev-tools` feature-gate で隔離
- 重複検出: PSD1↔PHA1 / PSD2↔PHA2 で意味的同一の関数ゼロ
- production `Result::unwrap()` ゼロ
- 7/24 実験で **新 binary が March bin と同等以上の rate で安定動作**

## 関連ドキュメント

- [clean_architecture_evaluation.md](../clean_architecture_evaluation.md) — 28 candidates 詳細 + 工数/リスク/Impact
- [REFACTORING_SUGGESTIONS.md](../REFACTORING_SUGGESTIONS.md) — Gemini Flash の高レベル提案 (R-X1/R-X2 の起点)
- [docs/component_architecture.md](../docs/component_architecture.md) — タスク分離 + mpsc の現状アーキテクチャ
- memory: [architecture_reflection](/Users/aogaki/.claude/projects/-Users-aogaki-WorkSpace-delila-rs/memory/architecture_reflection.md) — 2026-04-22 の前段議論
- memory: [layering_principle_clock_sync](/Users/aogaki/.claude/projects/-Users-aogaki-WorkSpace-delila-rs/memory/layering_principle_clock_sync.md) — refactor 中も守る原則
