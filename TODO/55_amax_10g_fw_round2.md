# AMax 10G FW Round 2 — 16 ch + 16-bit digital probes + Tune Up sub-mode

**作成日:** 2026-05-07
**ステータス:** 📋 Plan 承認済 (2026-05-07)、実装は **Fri 2026-05-08 / Mon 2026-05-11** 着手予定
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

## Phase G — Codegen (16 ch 命名対応)

新 FW の register 命名は `page_amax_energy_<N>_<NAME>` (アンダースコア区切り、index は中間) — 旧 32-ch FW の `page_amax_energy_<NAME>` (index なし) や legacy slash 区切りとは異なる。`Register::fw_key()` の実装が `page_amax_energy_15_THRS` → `15_THRS` を返す = `THRS` に matchしない → 26 個全 register が unmapped 扱いになる。

### 実装

- [ ] **G1.** `src/bin/amax_codegen.rs::Register::fw_key()` (lines ~83–94) — `_<digits>_` infix strip 追加。3 era の命名 (legacy slash / 32-ch bare / 新 digit-infix) を全部処理。
- [ ] **G2.** `Register::is_per_channel()` (lines ~98–101) — suffix が digit から始まるときのみ true。broadcast page (`page_amax_energy_<NAME>`) は false にする。
- [ ] **G3.** `main()` dedup filter (lines ~174–185) — 「ch0 のみキープ」に汎化 (現状は `page_amax_energy_1/` 固定 skip)。
- [ ] **G4.** Unit test 3 ケース: legacy slash / 32-ch bare / 新 digit-infix。再発防止 pin。
- [ ] **G5.** `cargo run --bin amax_codegen --features dev-tools -- FW/20260507/RegisterFile.json` で resolve 確認。skipped_no_meta が空 (broadcast aliases 除く) であること。
- [ ] **G6.** `tools/amax_viewer/fw_params.json` — 新 FW で増えた register 名の有無確認、必要なら entry 追加。
- [ ] **G7.** `config/digitizers/amax_*.toml` — `num_channels: 16` (FELib `/par/NumCh` で確認)。

## Phase H — `Waveform` 5 → 16 + データ駆動 UI

### H.1 データ駆動 probe panel

- [ ] **H1.** `web/operator-ui/src/app/pages/waveform/waveform.component.ts` — `ProbeConfig` (lines ~63–72) を flat `digital1..5` から `digital: Record<number, boolean>` に refactor。
- [ ] **H2.** Tune Up toolbar (~131–154) + 通常 toolbar (~410–455) の checkbox を `@for` で生成。`activeDigitalProbes()` computed が `digital_probe_type[i] !== UNKNOWN_PROBE_TYPE` のスロットを返す。
- [ ] **H3.** `probeColors` を HSL hue 分散で algorithmic 生成 (16 まで対応、UNKNOWN は neutral gray)。
- [ ] **H4.** `buildChannelCharts()` (~1561–1607) の `digitalProbes` array を `activeDigitalProbes()` 駆動に置換。

### H.2 `Waveform` 構造体 5 → 16 一括拡張

**根拠:** ROOT basket 圧縮 (LZ4/zstd) でゼロ run はほぼ消える。MsgPack 空 `Vec<u8>` = 2 bytes × 11 = 22 bytes/event。typical rate (waveform on, 100k events/sec) で 2.2 MB/sec → 誤差レベル。

- [ ] **H5.** `src/reader/decoder/common.rs` — `digital_probe6..16: Vec<u8>` 追加 (`#[serde(default)]`)。`digital_probe_type: [u8; 5]` → `[u8; 16]`。`default_unknown_digital_probe_types` ヘルパー更新。`Waveform::default()` 更新。
- [ ] **H6.** `src/common/mod.rs` — 同形 mirror 拡張。
- [ ] **H7.** 全 25 `Waveform { ... }` 構築サイト — `digital_probe6..16: Vec::new()` + `digital_probe_type: [..., UNKNOWN; 11]` パディング。`grep -rn "digital_probe5: " --include="*.rs"` で全箇所列挙。
- [ ] **H8.** `WaveformMetadata` (`src/reader/decoder/dualchannel_common.rs`) の `digital_probe_type_padded()` を `[u8; 16]` に bump (PSD2/PHA2 は 0..3 のみ populate、4..15 は UNKNOWN padding)。
- [ ] **H9.** `src/reader/mod.rs::convert_event` — 11 個の新 digital probe を bridge で move。
- [ ] **H10.** `src/reader/decoder/amax.rs` `amax_probe_types` mod — 0x45..0x4F を予約 namespace としてコメント documentation (`// reserved for future bits 10..0`)。

### H.3 ROOT export

- [ ] **H11.** `src/bin/delila_to_root.rs` — `DigitalProbeType5..15` の 11 branch 追加。既存 paste pattern 踏襲。

## Phase I — AMax Tune Up sub-mode

### I.1 Sub-mode signal

- [ ] **I1.** `WaveformPageComponent` — `tuneupView = signal<'standard' | 'amax-debug'>('standard')` 追加。`firmware === 'AMax'` の時だけ `'amax-debug'` 選択可。

### I.2 Template 分岐

- [ ] **I2.** `@if (isTuneUp())` セクション内に Material button-toggle group 追加 (`firmware === 'AMax'` 限定表示)。
- [ ] **I3.** Toolbar 分岐: `tuneupView() === 'amax-debug'` 時、ch0 ロック + debug probe presets + ENABLE_ACQ Quick toggle。
- [ ] **I4.** AMax debug 専用 side panel (新 `AmaxRegisterInspectorComponent` または inline):
  - **Register inspector** — ENABLE_ACQ / AMAX_gol / AMAX_MAIN_CH0 / ENERGY_MAIN_CH0 を 1s polling で表示
  - **Bit-level digital lane** — 16 個の binary timeline を probe-type で色分け、xAxis を chart と同期

### I.3 ENABLE_ACQ runtime toggle

- [ ] **I5.** AMax debug toolbar に "Quick toggle" ボタン → `tuneupApply` で partial config (`amax_board.enable_acq` のみ) 送信。Settings 画面に行かずに切り替え可能。
- [ ] **I6.** `src/operator/routes/tuneup.rs` で partial config 対応確認。SetInRun で動かない場合は Stop → flip → Start を docstring に明示。

### I.4 No new route

- 全部 `web/operator-ui/src/app/pages/waveform/waveform.component.ts` 内で完結。`app.routes.ts` 触らない。

## Phase J — Verification

- [ ] **J1.** Codegen unit test (G4) green。
- [ ] **J2.** 172.18.6.114 で `caen_simple_test --firmware amax --url <new-FW-URL>` smoke。416-write sweep + ENABLE_ACQ toggle 動作確認。
- [ ] **J3.** Tune Up UI walk-through:
  1. AMax digitizer 選択で Tune Up start
  2. "AMax Debug View" toggle → toolbar swap、register inspector 出現
  3. ENABLE_ACQ Quick toggle → Monitor で 3 analog + 5 digital probes 表示
  4. Toggle 戻し → 標準 layout
  5. PSD2 に切り替え → sub-mode toggle 非表示、UI 今と同一
- [ ] **J4.** Pre-commit gate: `cargo fmt && cargo clippy --tests -- -D warnings && cargo test && (cd web/operator-ui && npm run build)` 全部 green。
- [ ] **J5.** `dist/` 含めて 1 commit、master push。

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
