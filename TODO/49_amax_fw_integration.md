# AMax FW Integration into delila-rs

**作成日:** 2026-04-27
**ステータス:** 📋 計画完了（プラン承認 → 着手は明日以降）
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

## Phase 1 — MVP

### Backend (Rust)
- [ ] **B1.** `EventData` に `user_info: [u64; 4]` を追加 — [src/reader/decoder/common.rs](../src/reader/decoder/common.rs)
- [ ] **B2.** AMax decoder で User Word の 63-bit data を `user_info[0..=3]` に抽出 — [src/reader/decoder/amax.rs](../src/reader/decoder/amax.rs)。既存 `AMaxEventData { amax_value, baseline }` は `EventData.user_info` 経由に置換
- [ ] **B3.** 全 firmware decoder (psd1/psd2/pha1) で `user_info: [0; 4]` をデフォルト埋め
- [ ] **B4.** `AMaxChannelConfig` 構造体 (24 typed fields) + `ChannelConfig.amax: Option<AMaxChannelConfig>` 追加 — [src/config/digitizer.rs](../src/config/digitizer.rs)
- [ ] **B5.** 新規 `src/reader/caen/amax_registers.rs` — register offset const table + `channel_register_addr(ch, offset)` helper（page base 0x800000 + ch×0x40000）
- [ ] **B6.** `apply_amax_channel_config(&self, config) -> Result<usize, _>` — channel-loop で `set_user_register(addr, value)` を呼ぶ。`params_applied` カウント返却（V1743 と同じ UX）
- [ ] **B7.** `read_loop_opendpp` で AMax の場合に B6 を呼ぶ apply path 分岐 — [src/reader/mod.rs](../src/reader/mod.rs)
- [ ] **B8.** Monitor に `amax2d_histograms: HashMap<ChannelKey, Histogram2D>` 追加（X=energy 0-65536/512bin, Y=user_info[0] 0-16384/200bin）— [src/monitor/mod.rs](../src/monitor/mod.rs)
- [ ] **B9.** `/api/histograms2d/:m/:c?type=amax2d` クエリ対応 — [src/operator/routes/monitor.rs](../src/operator/routes/monitor.rs)

### Frontend (Angular)
- [ ] **F1.** `types.ts`: `AMaxChannelConfig` interface、`ChannelConfig.amax`、`EventData.user_info`、`HistogramType` に `'amax2d'` 追加
- [ ] **F2.** `channel-params.ts`: `AMAX_INPUT_PARAMS` (3) + `AMAX_TRIGGER_PARAMS` (3) + `AMAX_ENERGY_PARAMS` (~14) + `AMAX_WAVEFORM_PARAMS` (4) を定義、CATEGORY_PARAMS に `AMax` エントリ
- [ ] **F3.** `digitizer.service.ts`: `CHANNEL_PARAM_KEYS` allowlist に AMax 24 keys 追加、**flat-to-dotted 変換層**（`'amax.polarity'` ↔ `AMaxChannelConfig.polarity`）
- [ ] **F4.** `digitizer-settings.component.ts`: AMax FW 用 board info セクション追加（`@if (config.firmware === 'AMax')`）
- [ ] **F5.** Monitor: `view-tab` / `setup-tab` に `histogramType === 'amax2d'` 分岐、heatmap 流用

### Tests
- [ ] `channel_register_addr` 単体: ch0+0x02→0x800002, ch1+0x16→0x840016
- [ ] `AMaxChannelConfig` serde roundtrip (full/sparse/MongoDB BSON 互換)
- [ ] AMax decoder の user_info 抽出（既知バイト列 → 期待値）
- [ ] `Histogram2D::fill` for AMax (energy=1000, user_info[0]=500 で正しい bin 位置)
- [ ] `apply_amax_channel_config` モックハンドル経由でレジスタ書き込み順序・値検証
- [ ] flat-to-dotted key conversion 双方向

### Integration test (実機 AMax FW 搭載 VX2730)
- [ ] デプロイ → Settings tab で AMax 24 params が Input/Trigger/Energy/Waveform に展開
- [ ] ch0 で `polarity=1`/`thrs=40`/`trap_k=500` 設定 → Apply → "Applied N parameters" N≥3
- [ ] amax_viewer の Read All で同じ値が読める（**併存併用テスト**）
- [ ] Run start → ch0 にパルス → Energy 1D / AMax 2D 両方に events
- [ ] amax_viewer のスクリーンショットと AMax 2D heatmap の分布が一致

---

## Phase 2 — Exploration & Polish (defined, deferred)

- [ ] Generic `AxisSource` enum (`Energy | EnergyShort | Psd | UserInfo(u8) | FineTime`)、`Histogram2D` を `HashMap<(ChannelKey, PlotId), Histogram2D>` に拡張、TTL eviction
- [ ] Frontend: 2D plot に X/Y 軸 dropdown、Z 軸 Log/Linear トグル
- [ ] 1D histograms for `UserInfo[0..=3]`（既存 `psd_histograms` と同パターン）
- [ ] Tooltip with hex address (Settings tab、senior physicist の信頼感のため)
- [ ] Reset-to-defaults button（per-channel + board-level、`fw_params.json` の default 値復元）
- [ ] SetInRun whitelist: test_pulse 系のみ Running 中変更可

---

## Phase 3 — Future (documented, not scheduled)

- [ ] Lasso/polygon graphical gate (Tukey-style EDA): 2D plot 上で囲んだ範囲だけの 1D histogram を即座に横に表示
- [ ] Backend gated histograms（投げ縄座標を bitmask 化、filtered 1D plot 生成）
- [ ] **32 ch 拡張**（FW は対応、UI もスケール可能設計）
- [ ] Dynamic JSON-driven UI（FW 改訂時に `fw_params.json` 更新だけで UI 追従）
- [ ] ROOT export integration（oxyroot で amax_viewer 互換 12-branch TTree）

---

## Risks / Caveats

1. **`user_info: [u64; 4]` 固定長は将来制約**: amax_viewer は 1024 slot 確保。Phase 1 は [u64; 4] start、足りなければ拡張
2. **`0xFFFFFFFF` を `<input type="number" step="1">` に渡すと UI 固まる可能性**: max を実用範囲（例 10M）に絞るか `bits` field から計算
3. **flat-to-dotted key 変換のバグ**: V1743 trigger_edge で類似ミスあり。十分な unit test 必須
4. **Histogram2D の y_config range**: amax_viewer デフォルト 0-16384 は AMax FW セットアップ依存。Phase 2 で auto-fit
5. **既存 `AMaxEventData` 廃止のインパクト**: monitor / recorder / event_builder で参照箇所を grep で全洗い、`EventData.user_info` への置き換えは慎重に
6. **MsgPack 互換**: `user_info: [u64; 4]` 追加で全 ZMQ consumer (merger/recorder/event_builder) 同時再ビルド・再デプロイ必須
7. **後方互換**: 古い `.delila` ファイルは `user_info` フィールド無し → recorder format で missing-field default-zero ハンドリング確認

---

## 参照リソース

- **プラン詳細**: `/Users/aogaki/.claude/plans/valiant-napping-rabin.md` （AMax 用に上書き済み、V1743 プランは git log を参照）
- **canonical reference**:
  - [tools/amax_viewer/src/main.rs](../tools/amax_viewer/src/main.rs) — apply_changes(), EventBuffer の正解実装
  - [tools/amax_viewer/fw_params.json](../tools/amax_viewer/fw_params.json) — 24 params の bits/default
  - [tools/amax_viewer/MANUAL.md](../tools/amax_viewer/MANUAL.md) — UI セマンティクス、ROOT branch list
  - [AMAX_firmware32_channel_4input_caenlist/2channels_parameters_05032026.txt](../AMAX_firmware32_channel_4input_caenlist/2channels_parameters_05032026.txt) — register address table
- **Gemini 協議ログ**: 2026-04-27 brainstorm セッション（プランファイル末尾の "Conversation History" 節）

---

**Next session で開始**: Phase 1 B1（EventData.user_info 追加）から TDD で順番に。プランファイルの Phase 1 — Detailed implementation 節をそのまま実行可能な粒度に。
