# AMax FW Integration into delila-rs

**作成日:** 2026-04-27
**ステータス:** ✅ Phase 1 完了 (2026-04-28)、🚧 Phase 2 着手中（軸選択 — `AxisSource` enum）
**プランファイル:** [/Users/aogaki/.claude/plans/valiant-napping-rabin.md](../../.claude/plans/valiant-napping-rabin.md)
**前提:** [V1743 WaveDemo パラメーター追加](../TODO/archive/) は 2026-04-27 完了（trigger_edge / ttf_smoothing / extra_registers / V threshold 全部リモート稼働確認済）

**背景:**
カスタム FPGA ファームウェア "AMax"（DELILA 自作、Trapezoidal Filter MCA + Amplitude-Maximum 検出、CAEN VX2730 用）は現状 `tools/amax_viewer/`（standalone egui ツール、~2000 LOC）でしか操作できない。delila-rs 本体は `FirmwareType::AMax` を認識するものの per-channel パラメーター UI が完全に未定義、ヒストグラムも generic Energy/PSD のみで AMax 特有の **user_info VS Energy 2D plot** が描画できない。

物理計測時のワークフローが「amax_viewer で設定 → delila-rs で記録」と分断されており、operator-ui だけで完結させたい。

**設計の核（Gemini 2.5 Pro 協議済 2026-04-27）:**
- amax_viewer の `fw_params.json` 駆動 UI ではなく、delila-rs の static `ChannelParamDef[]` パターンに合わせて **24 entries 直書き**（既存 PSD2/PSD1/PHA1/X743Std と一貫）
- `ChannelConfig` に `amax: Option<AMaxChannelConfig>` を追加（X743Std の `x743: Option<X743Config>` と同じネスト型安全パターン）
- レジスタ書き込みは FELib SetValue ではなく `set_user_register(byte_addr, value)` 直叩き、アドレス = `0x800000 + ch * 0x40000 + offset`
- `EventData` に `user_info: [u64; 4]` を追加（zero-allocation 固定長、非 AMax FW では全部 0）
- Phase 1 は **Energy × UserInfo[0] の固定 2D ヒストグラム**（既存 `psd2d` と同じパターン）

**ユーザー判断結果（プラン議論時、2026-04-27）:**
- Phase 1 軸 UI = 固定（Energy × UserInfo[0]）、軸選択 dropdown は Phase 2 で
- amax_viewer は併存維持（Phase 1 完了で legacy 化、Phase 2-3 完了でアーカイブ）
- チャンネル数 = **2 ch (ch0/ch1) のみ**（現運用に合わせる、32 ch 拡張は後回し）

---

## Phase 1 — MVP ✅ 完了 (2026-04-28)

実機検証: VX2730 + AMax FW @ 172.18.4.56、TestPulse 1 kHz × 2ch で約 10 kHz、Apply は 7 FELib + 40 AMax custom register writes (合計 47)、1D Energy + 2D Energy×UserInfo[0] heatmap 両方ライブ表示。

### Backend (Rust) ✅
- [x] **B1.** `EventData.user_info: [u64; 4]` 追加 — [src/reader/decoder/common.rs](../src/reader/decoder/common.rs)
- [x] **B2.** AMax decoder で User Word の 63-bit data を `user_info[0..=3]` に抽出
- [x] **B3.** 全 firmware decoder (psd1/psd2/pha1) で `user_info: [0; 4]` デフォルト埋め
- [x] **B4.** `AMaxChannelConfig` (24 typed fields) + `ChannelConfig.amax: Option<AMaxChannelConfig>`
- [x] **B5.** 新規 `src/reader/caen/amax_registers.rs` — REG_* + `channel_register_byte_addr()`
- [x] **B6.** `apply_amax_channel_config(&self, &DigitizerConfig) -> Result<usize, CaenError>` — count 返却
- [x] **B7.** `read_loop_opendpp` の AMax 分岐から B6 を呼ぶ
- [x] **B8.** Monitor に `amax2d_histograms` (X=energy 0-65536/512bin, Y=user_info[0] 0-16384/**512** bin)
- [x] **B9.** `/api/histograms2d/:m/:c?type=amax2d` クエリ対応

### Frontend (Angular) ✅
- [x] **F1.** `types.ts` 同期 — `AMaxChannelConfig` interface、`ChannelConfig.amax`、`HistogramType` に `'amax2d'`
- [x] **F2.** `channel-params.ts` — AMAX 24 params を Input/Trigger/Energy/Waveform に分配
- [x] **F3.** `digitizer.service.ts` — flat-to-dotted (`'amax.polarity'`) 変換層
- [x] **F4.** Settings tab で AMax 24 params が自動展開（CATEGORY_PARAMS）
- [x] **F5.** Monitor — `view-tab` / `setup-tab` に `'amax2d'` heatmap 分岐、heatmap-chart に `yAxisLabel` Input

### Phase 1 Polish (2026-04-28) ✅
- [x] `generateUUID()` 無限再帰 fix（自分自身を呼んでいた → `crypto.randomUUID()` を呼ぶ）
- [x] heatmap-chart に `yAxisLabel` Input 追加（amax2d は "UserInfo[0]"、psd2d は "PSD"）
- [x] BoardConfig に `test_pulse_low_level` / `test_pulse_high_level` 追加
- [x] amax2d Y bins 256 → 512（amax_viewer 互換）
- [x] integration-test reference: `config/config_amax_56.toml` + `config/digitizers/amax_56.json`

### Codegen Bridge (Phase 1 → Phase 2、2026-04-28) ✅
新規 `src/bin/amax_codegen.rs` 導入。FW 開発者が新 RegisterFile.json を出してきても、UI メタを `tools/amax_viewer/fw_params.json` に追加するだけで Rust struct + Rust register const + TS interface + TS ChannelParamDef[] が一括再生成される。Phase 2 以降のレジスタ追加が手書き 4 ファイル同期から解放。

- [x] `tools/amax_viewer/fw_params.json` を UI メタ拡張（label/category/type/options/ui_max/unit）— amax_viewer の `gen_defs` は不変動作確認 ("48 matched, 0 unmatched")
- [x] `src/bin/amax_codegen.rs` 実装、3 ファイル生成 (Rust struct + Rust registers + TS interface/PARAMS)
- [x] 手書き → 生成版に置換、`pub use` で既存 import 互換維持
- [x] Final test: 47 params Apply、event rate 一致、動作完全互換

### Integration test (実機 AMax FW 搭載 VX2730 @ 172.18.4.56) ✅
- [x] Settings tab で AMax 24 params が Input/Trigger/Energy/Waveform に展開
- [x] Apply → "Applied 47 parameters to hardware"、log で 7 FELib + 40 AMax custom 確認
- [x] DevTree validation: `testpulsewidth: 10 → 8` 自動調整
- [x] Run start → TestPulse 2ch で約 10 kHz、Energy 1D / AMax 2D 両方に events
- [x] heatmap Y 軸ラベル "UserInfo[0]"、tooltip も同名

---

## Phase 2 — Exploration & Polish (in progress)

### 着手中: AxisSource enum + 2D 軸選択
ユーザー判断 (2026-04-28):
- `AxisSource = Energy | EnergyShort | FineTime | UserInfo0..=3 | Psd` (派生量 Psd 含む、PSD2 互換のため)
- ストレージ: `HashMap<(ChannelKey, AxisSource, AxisSource), Histogram2D>` + `last_accessed: Instant`、30s 周期で 60s 以上未アクセス分を evict
- API: 新形式 `?x=energy&y=user_info0` に統一、古い `?type=psd2d|amax2d` は内部で migration
- 1D は今回手付かず（Energy/PSD 固定、UserInfo 1D は別タスク）
- SetInRun whitelist もスキップ（オフライン解析でやる方針）

### タスク

#### Backend
- [ ] **P2-B1.** `AxisSource` enum + `extract(&EventData) -> Option<f64>` — 新規 `src/monitor/axis.rs`
- [ ] **P2-B2.** `MonitorState` の `psd2d_histograms` + `amax2d_histograms` を `histograms2d: HashMap<PlotKey, PlotEntry>` に統合
- [ ] **P2-B3.** Event 受信時、その channel の既存 PlotEntry 全部を `extract` で fill。secondary index `plots_by_channel`
- [ ] **P2-B4.** TTL evictor — 30s 周期、`last_accessed > 60s` を削除
- [ ] **P2-B5.** `MonitorConfig.histogram2d_default` で軸別 (min, max, bins) を持つ

#### REST API
- [ ] **P2-R1.** `GET /api/histograms2d/:m/:c?x=<axis>&y=<axis>` — `last_accessed` 更新、無ければ作成して空ヒスト返す
- [ ] **P2-R2.** 古い `?type=psd2d` → `?x=energy&y=psd`、`?type=amax2d` → `?x=energy&y=user_info0`（内部 migration）
- [ ] **P2-R3.** OpenAPI schema 更新

#### Frontend
- [ ] **P2-F1.** `histogram.types.ts` — `AxisSource` string literal、`PlotConfig` 形式
- [ ] **P2-F2.** `histogram.service.ts` — `fetchHistogram2d(m, c, x, y)` シグネチャ変更
- [ ] **P2-F3.** `setup-tab` — 2D 選択時に X/Y 軸 dropdown 表示
- [ ] **P2-F4.** `view-tab` — `tab.plotConfig.{x,y}` を fetchHistogram2d に渡す
- [ ] **P2-F5.** `histogram-expand-dialog` — 同上
- [ ] **P2-F6.** `heatmap-chart` — `xAxisLabel` Input も追加
- [ ] **P2-F7.** Migration — localStorage / monitor_layout.json の古い形式を新形式に変換

#### Tests
- [ ] AxisSource roundtrip (serde、URL query、TS string literal)
- [ ] `extract()` で各 axis が EventData から正しい値を取り出す
- [ ] TTL evictor: 60s 経過後に entry が消える
- [ ] 旧 `?type=psd2d` → `(energy, psd)` への migration
- [ ] Histogram2D fill: 同一イベントで複数 PlotEntry に並列 fill

### 後続（AxisSource 完了後）
- [ ] 1D histograms for `UserInfo[0..=3]`（既存 `psd_histograms` と同パターン）
- [ ] Reset-to-defaults button（per-channel + board-level、`fw_params.json` の default 値復元 — codegen で取得済）
- [ ] Tooltip with hex address (Settings tab、senior physicist の信頼感のため)

#### スキップ (ユーザー方針)
- ~~Z 軸 Log/Linear トグル~~ — 既に `logScale` プロパティ実装済、ユーザー要望薄
- ~~SetInRun whitelist~~ — オフライン解析中心の運用方針

---

## Phase 3 — Future (documented, not scheduled)

- [ ] Lasso/polygon graphical gate (Tukey-style EDA): 2D plot 上で囲んだ範囲だけの 1D histogram を即座に横に表示
- [ ] Backend gated histograms（投げ縄座標を bitmask 化、filtered 1D plot 生成）
- [ ] **32 ch 拡張**（FW は対応、UI もスケール可能設計）
- [ ] Dynamic JSON-driven UI（FW 改訂時に `fw_params.json` 更新だけで UI 追従）
- [ ] ROOT export integration（oxyroot で amax_viewer 互換 12-branch TTree）

---

## Risks / Caveats

1. **`user_info: [u64; 4]` 固定長は将来制約**: amax_viewer は 1024 slot 確保。Phase 1 で実用上問題なし。足りなければ拡張
2. **`0xFFFFFFFF` を `<input type="number" step="1">` に渡すと UI 固まる可能性**: codegen で `ui_max` を fw_params.json から取得（解決済）
3. **flat-to-dotted key 変換のバグ**: V1743 trigger_edge で類似ミスあり。Phase 1 で動作確認済
4. **Histogram2D の y_config range**: Phase 2 で auto-fit 検討（ただし TTL eviction で高頻度切替に対応）
5. **MsgPack 互換**: `user_info: [u64; 4]` 追加で全 ZMQ consumer (merger/recorder/event_builder) 同時再ビルド・再デプロイ必須（Phase 1 で検証済）
6. **Phase 2 メモリ**: PlotKey 全組み合わせは worst case 2ch × 7² × 512KB ≒ 50MB。実際は表示中のみ生きるので 1-数 MB（TTL eviction）

---

## 参照リソース

- **プラン詳細**: `/Users/aogaki/.claude/plans/valiant-napping-rabin.md` （AMax 用に上書き済み、V1743 プランは git log を参照）
- **canonical reference**:
  - [tools/amax_viewer/src/main.rs](../tools/amax_viewer/src/main.rs) — apply_changes(), EventBuffer の正解実装
  - [tools/amax_viewer/fw_params.json](../tools/amax_viewer/fw_params.json) — 24 params の bits/default + UI メタ (Phase 1 で拡張)
  - [tools/amax_viewer/MANUAL.md](../tools/amax_viewer/MANUAL.md) — UI セマンティクス、ROOT branch list
  - [AMAX_firmware32_channel_4input_caenlist/output/output/RegisterFile.json](../AMAX_firmware32_channel_4input_caenlist/output/output/RegisterFile.json) — FW 開発者の正本（54 reg = 27/ch × 2ch）
- **Gemini 協議ログ**: 2026-04-27 brainstorm セッション（プランファイル末尾の "Conversation History" 節）
- **Phase 1 commits**: `c07c11e` (surface migration) + `5185856` (polish) + `68c3b5b` (codegen bridge)

---

**Phase 2 開始**: P2-B1 (`AxisSource` enum + extract) から TDD で順番に。
