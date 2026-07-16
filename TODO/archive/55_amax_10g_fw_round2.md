# AMax 10G FW Round 2 — 16 ch + 16-bit digital probes + Tune Up sub-mode

**作成日:** 2026-05-07
**ステータス:** ✅ **COMPLETED (2026-07-09 クローズ)** — コード作業完了 (2026-05-08、commits `8786008` + `1984bd3`、master push 済)。残っていた J2/J3 (5/11 実機検証) は**対象 FW (20260507) が 6月の 11june/17june FW に置換されたため陳腐化**。後継 FW は first-light + 実機検証済 ([TODO 60](60_amax_fw_selfconfig.md) の self-config workflow で完結)。Round 2 のコード (16ch/16 digital probes/Tune Up sub-mode/register inspector) はその後の FW サイクルで実運用済み。
**プランファイル:** [/Users/aogaki/.claude/plans/lazy-herding-naur.md](../../.claude/plans/lazy-herding-naur.md) Round 2 セクション (Phase G–J)
**前提:** Round 1 完了 (commits `d336203` + `b1e9aa3`、master push 済)
**FW ソース:** `FW/20260507/RegisterFile.json` + `V2730-OpenDPP10GUDP-AMAXfirmware32channel4inputcaenlist-2026050752.cup`

---

## 背景

Rebeca (FW 開発者) が後継 FW を提供:
- **10 Gigabit UDP transport** (旧 1 GbE から大幅増強)
- per-channel page を 8 → **16** に倍増 (`page_amax_energy_0` … `_15`)
- broadcast page (`page_amax_energy_<NAME>` 接尾辞なし) 追加 — 全チャンネル一括書き込み
- 将来計画: 16-bit digital lane の **全 16 bit を digital probe として使用** (現状 5 bit のみ)

DELILA 側の課題: 16 ch + 16 digital probes に対応しつつ、他 FW (PSD2/PHA2/PHA1/PSD1) のオペレーター UI に余計な debug 情報を出さないこと。

## ユーザー判断結果 (2026-05-07 プラン議論時)

- **Sub-mode in shared Tune Up, NOT new route.** AMax 選択時のみ "AMax Debug View" トグルが現れる。Chart + Parameter Table は共有、ツールバー/プローブパネル/レジスタインスペクタだけ swap。
- **長期的 Tune Up 方針:** 共有コンポーネントを FW 条件分岐で拡張し続ける。フォーク禁止。
- **`Waveform` 構造体 5 → 16 を一括拡張** (Round 1 で 4 → 5 だったのを今回 5 → 16 に飛ばす)。理由: ROOT 圧縮 + MsgPack 空 Vec オーバヘッド微小 + 将来 Rebeca が bit を増やす度の 25-site 触り直しを回避。Decoder のみ段階的に bit 抽出を増やす。
- **データ駆動 probe panel** (Phase H.1)。`digital_probe_type[i] !== UNKNOWN` で表示判定 → 他 FW は今と同じ 5 個 (D0..D4)、AMax だけ 16 個までスケール。

## 関連 memory

- [`amax_10g_round2_queued`](`/Users/aogaki/.claude/projects/-Users-aogaki-WorkSpace-delila-rs/memory/amax_10g_round2_queued.md`) — 再開時のスタート地点
- [`feedback_preallocate_over_incremental`](`/Users/aogaki/.claude/projects/-Users-aogaki-WorkSpace-delila-rs/memory/feedback_preallocate_over_incremental.md`) — 一括拡張 vs 段階拡張の判断則

---

## 完了サマリー (2026-05-08 セッション)

**コード作業 (Fri):**
- ✅ Phase G: codegen `_<N>_<NAME>` infix 対応 + `channel_index()` + broadcast canonical filter + 11 unit tests (5 era 別 + 6 integration)
- ✅ Phase H.2: `Waveform` 5 → 16 digital probes 一括拡張 (25 sites + WaveformMetadata + delila_to_root + tests)
- ✅ Phase H.1: データ駆動 `activeDigitalProbeSlots` + `digitalProbeColor/Label/Visible/Toggle` + ProbeConfig refactor + chart loop refactor
- ✅ Phase I.1-I.3: `tuneupView` signal + AMax sub-mode toggle + ENABLE_ACQ Quick toggle + amax-debug ch0 lock
- ✅ Phase I.2 stretch: **Register inspector side panel** — `Command::ReadAmaxBoardRegisters` + `ReadLoopRequest` variant + `CommandHandlerExt::on_read_amax_board_registers` + dig1/dig2/V1743 handlers + REST `GET /api/digitizers/:id/amax-board-registers` + DigitizerService method + 1Hz polling effect + drift detection + mat-card UI
- ✅ Phase J: pre-commit gate (clippy + 578 tests + ng build) + commit `8786008` (master push)

**残: 月曜 (5/11) 実機検証**
- Phase J.2: 172.18.6.114 で `caen_simple_test --firmware amax --url <new-FW-URL>` smoke
- Phase J.3: Tune Up UI walk-through (Standard/Debug toggle、ENABLE_ACQ Quick toggle、register drift indicator 確認)

---

## Phase G — Codegen (16 ch 命名対応)

新 FW の register 命名は `page_amax_energy_<N>_<NAME>` (アンダースコア区切り、index は中間) — 旧 32-ch FW の `page_amax_energy_<NAME>` (index なし) や legacy slash 区切りとは異なる。`Register::fw_key()` の実装が `page_amax_energy_15_THRS` → `15_THRS` を返す = `THRS` に matchしない → 26 個全 register が unmapped 扱いになる。

### 実装

- [x] **G1.** `src/bin/amax_codegen.rs::Register::fw_key()` (lines 99–104) — `_<digits>_` infix strip 追加 (3 era 全対応: legacy slash / 32-ch bare / 新 digit-infix)。
- [x] **G2.** `Register::is_per_channel()` (lines 110–113) — `channel_index() → None` で broadcast page (`page_amax_energy_<NAME>` 接尾辞なし) を per-channel から除外。
- [x] **G3.** `select_canonical_per_channel()` (lines 172–189) — 「ch0 のみキープ + broadcast キープ」に汎化済。
- [x] **G4.** Unit test 11 本: `fw_key_handles_three_eras` + `channel_index_distinguishes_broadcast_from_per_channel` + `canonical_filter_2026_03/04/05_era_*` (3 era × fixture coverage)。再発防止 pin。
- [x] **G5.** `cargo run --bin amax_codegen --features dev-tools -- FW/20260507/RegisterFile.json` clean: 26 per-channel writable + 1 board-level + 1 read-only skipped。
- [x] **G6.** `tools/amax_viewer/fw_params.json` 確認・更新済 (5,110 bytes、 2026-05-07 14:59)。
- [x] **G7.** `config/digitizers/amax_56.json` 更新 (`num_channels: 32` — broadcast page で全 ch 一括書き込み + 新 SHAP params `delay_shaping`/`shap_trigg`/`shap_bl_hold` plumb 済、 commit `8786008`/`1984bd3` の派生 working-tree 編集)。

## Phase H — `Waveform` 5 → 16 + データ駆動 UI

### H.1 データ駆動 probe panel

- [x] **H1.** `web/operator-ui/src/app/pages/waveform/waveform.component.ts` — `ProbeConfig` を flat `digital1..5` から `digital: Record<number, boolean>` に refactor 済。
- [x] **H2.** Tune Up toolbar + 通常 toolbar の checkbox を `@for` で生成、 `activeDigitalProbeSlots()` computed が `digital_probe_type[i] !== UNKNOWN_PROBE_TYPE` のスロットを返す。
- [x] **H3.** `digitalProbeColor` を HSL hue rotation で 16 色 algorithmic 生成。
- [x] **H4.** `buildChannelCharts()` の digital probe loop を `activeDigitalProbeSlots()` 駆動に置換済。

### H.2 `Waveform` 構造体 5 → 16 一括拡張

**根拠:** ROOT basket 圧縮 (LZ4/zstd) でゼロ run はほぼ消える。MsgPack 空 `Vec<u8>` = 2 bytes × 11 = 22 bytes/event。typical rate (waveform on, 100k events/sec) で 2.2 MB/sec → 誤差レベル。

- [x] **H5.** `src/reader/decoder/common.rs:99-119` — `digital_probe6..16: Vec<u8>` 追加 (`#[serde(default)]`)、 `digital_probe_type: [u8; 16]` (line 67-68 helper `default_unknown_digital_probe_types()` 同期更新)。
- [x] **H6.** `src/common/mod.rs:122` — 同形 mirror 拡張済。
- [x] **H7.** 全 25 `Waveform { ... }` 構築サイト更新済 (decoder/dualchannel_common.rs:791–819 で全 fields + `metadata.digital_probe_type_padded()` 経由のパディング、 amax/psd1/pha1/mod.rs サイト全部対応)。
- [x] **H8.** `WaveformMetadata::digital_probe_type_padded()` (`src/reader/decoder/dualchannel_common.rs:184–201`) が `[u8; 16]` を返却 (slots 0..3 populate、 4..15 UNKNOWN)。
- [x] **H9.** `src/reader/mod.rs:1389–1399` `convert_event` — 11 個の新 digital probe (`digital_probe6..16`) を bridge で move 済。
- [x] **H10.** `src/reader/decoder/amax.rs:35–38` `amax_probe_types` mod — 0x45..0x4F 予約 namespace コメント済。

### H.3 ROOT export

- [x] **H11.** `src/bin/delila_to_root.rs:257,266` — `DigitalProbeType5..15` 11 branch 追加、 全 16 branch (0..15) 完備。

## Phase I — AMax Tune Up sub-mode

### I.1 Sub-mode signal

- [x] **I1.** `WaveformPageComponent:1109` — `readonly tuneupView = signal<'standard' | 'amax-debug'>('standard')` 追加済。`firmware === 'AMax'` 時のみ `'amax-debug'` 選択可。

### I.2 Template 分岐

- [x] **I2.** `@if (tuneUpConfig()?.firmware === 'AMax')` 限定で Material button-toggle group 表示 (lines 125–139)。
- [x] **I3.** Toolbar 分岐: `tuneupView() === 'amax-debug'` 時 (line 144) — ch0 lock effect + debug probe presets + ENABLE_ACQ Quick toggle (line 148)。
- [x] **I4.** AMax debug 専用 side panel — `.amax-register-inspector` mat-card (lines 246+) + 1Hz polling effect (amax-debug 中のみ) + `amaxBoardRegistersDrifting()` で config↔hardware mismatch detection (amber `sync_problem` indicator)。**Phase I.2 stretch — Register inspector** (新 stretch goal): backend `Command::ReadAmaxBoardRegisters` + `ReadLoopRequest` variant + `CommandHandlerExt::on_read_amax_board_registers` + dig1/dig2/V1743 各 read_loop の handler + REST `GET /api/digitizers/:id/amax-board-registers` + DigitizerService.readAmaxBoardRegisters + codegen が emit する `all_board_registers()` helper で将来 board param 増えても auto-extend (commit `1984bd3` Round 2 follow-up)。

### I.3 ENABLE_ACQ runtime toggle

- [x] **I5.** AMax debug toolbar の "Quick toggle" → `onAmaxEnableAcqToggle()` (line 1678) が `tuneupApply()` で partial config (optimistic update + rollback) 送信。
- [x] **I6.** `src/operator/routes/tuneup.rs` partial config 対応確認済 (apply path に restrictive rejection なし、 SetInRun 経路 OK)。

### I.4 No new route

- [x] 全部 `web/operator-ui/src/app/pages/waveform/waveform.component.ts` 内で完結。`app.routes.ts` 触らない。

## Phase J — Verification

- [x] **J1.** Codegen unit test (G4) 11 本 green (5 era 別 + 6 integration、 3 era fixture coverage)。
- [ ] **J2.** 172.18.6.114 で `caen_simple_test --firmware amax --url <new-FW-URL>` smoke。416-write sweep + ENABLE_ACQ toggle 動作確認。 **(Mon 5/11 実機)**
- [ ] **J3.** Tune Up UI walk-through: **(Mon 5/11 実機)**
  1. AMax digitizer 選択で Tune Up start
  2. "AMax Debug View" toggle → toolbar swap、register inspector 出現
  3. ENABLE_ACQ Quick toggle → Monitor で 3 analog + 5 digital probes 表示
  4. Toggle 戻し → 標準 layout
  5. PSD2 に切り替え → sub-mode toggle 非表示、UI 今と同一
- [x] **J4.** Pre-commit gate: 11 codegen unit tests + 578 lib tests + clippy `-D warnings` + `ng build` 全部 green。
- [x] **J5.** `dist/` 含めて 2 commit (`8786008` core + `1984bd3` follow-up)、 master push 済。

---

## Risk & Rollback

- **Codegen regression risk: medium.** `_<N>_` infix strip が legacy era logic と overlap。G4 unit test を必ず先に書いて pin。
- **Non-AMax UI regression risk: low.** Sub-mode toggle は `firmware === 'AMax'` 限定。data-driven probe panel は UNKNOWN の時 today's 5-checkbox に fallback。
- **Round 1 → Round 2 互換: clean.** Round 1 の struct 形は変わらず、fw_params.json + codegen rule + UI 描画のみ進化。`.delila` 互換性 OK。
- **10G transport caveat:** OpenDPP FFI / read-loop の CAEN endpoint URL が `10g://` 等に変わる可能性あり。`.cup` ファイル付随 doc + Rebeca のデプロイメモで確認、必要なら per-digitizer config の URL swap のみ (コード変更なし)。

## 参考リンク

- Round 1 commits: `d336203`, `b1e9aa3` (master)
- 古い AMax 統合: [TODO 49](49_amax_fw_integration.md) (Phase 1-3 完了)
- FW dev 環境: 172.18.6.114 (memory: deployment notes)
