# AMax FW Integration into delila-rs

**作成日:** 2026-04-27
**ステータス:** ✅ Phase 1 完了 (2026-04-28)、✅ Phase 2 完了 (2026-04-28)、✅ Phase 2.5 完了 (2026-04-29: 新 FW + 波形 + dev 環境)
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

## Phase 2 — Exploration & Polish

### ✅ AxisSource enum + 2D 軸選択 (2026-04-28 完了)
ユーザー判断:
- `AxisSource = Energy | EnergyShort | UserInfo0..=3 | Psd` (FineTime は pipeline EventData に無く dropped)
- ストレージ: `HashMap<(ChannelKey, AxisSource, AxisSource), PlotEntry>` + `last_accessed: Instant`、30s 周期で 60s 以上未アクセス分を evict
- API: 新形式 `?x=energy&y=user_info0`、古い `?type=psd2d|amax2d` は内部 migration で後方互換
- 1D / SetInRun whitelist は今回スキップ（後者はオフライン解析運用のため永続スキップ）

実機検証 (172.18.4.56、TestPulse 2ch ~10kHz)：
- `?type=psd2d` (legacy) → 0 counts (TestPulse で energy=0 なので Psd 未定義、想定動作)
- `?type=amax2d` (legacy) → 156k counts ✅
- `?x=energy&y=user_info0` → 同 156k ✅
- `?x=user_info0&y=user_info1` → **156k counts ✅ (Phase 2 の新機能)**

#### Backend ✅
- [x] **P2-B1.** `AxisSource` enum + `extract(&EventData) -> Option<f64>` — `src/monitor/axis.rs`
- [x] **P2-B2.** `MonitorState.histograms2d: HashMap<(ChannelKey, AxisSource, AxisSource), PlotEntry>` 統合
- [x] **P2-B3.** Event 受信時、その channel の既存 PlotEntry 全部を fill
- [x] **P2-B4.** TTL evictor — 30s 周期、60s 以上未アクセスを削除
- [x] **P2-B5.** `MonitorConfig.histogram2d_overrides: HashMap<AxisSource, HistogramConfig>`、`AxisSource::default_axis()` フォールバック

#### REST API ✅
- [x] **P2-R1.** `GET /api/histograms2d/:m/:c?x=<axis>&y=<axis>` — 無ければ on-demand 作成
- [x] **P2-R2.** 古い `?type=psd2d` → `(energy, psd)`、`?type=amax2d` → `(energy, user_info0)`
- [x] **P2-R3.** OpenAPI schema 更新

#### Frontend ✅
- [x] **P2-F1.** `histogram.types.ts` — `AxisSource` 統一、`HistogramType = 'energy' | 'psd' | '2d'`
- [x] **P2-F2.** `histogram.service.ts` — `fetchHistogram2d(m, c, x, y)`
- [x] **P2-F3.** `setup-tab` — 2D 選択時に X/Y 軸 dropdown 表示
- [x] **P2-F4.** `view-tab` — `tab.{xAxis,yAxis}` 経由で fetch
- [x] **P2-F5.** `histogram-expand-dialog` — 同上、開く際に X/Y を継承
- [x] **P2-F6.** `heatmap-chart` — `xAxisLabel` Input 追加
- [x] **P2-F7.** Migration — `migrateLegacyHistType()` で localStorage / monitor_layout.json を変換

Commits: `cc98d7d` (B1) + `ef5ba7c` (B2..B5 + R1..R3) + `3da6a3f` (F1..F7)

### ✅ 残タスク完了 (2026-04-28)
- [x] **Hex address tooltip** — `ChannelParamDef.tooltip` field を追加、codegen で `"FW reg POLARITY • word 0x02 (ch0 @ 0x800002)"` 形式の文字列を埋める。Settings tab の param-cell に hover で表示。
- [x] **Reset-to-defaults button** — codegen で `AMAX_DEFAULTS: Record<string, number>` を出力、`resetAmaxDefaults()` が defaults + 全 channel に書き込む。FW=AMax の時だけ表示。
- [x] **1D histograms for `UserInfo[0..=3]`** — `MonitorState.userinfo_histograms: HashMap<(ChannelKey, AxisSource), Histogram1D>` を追加（registered channel に pre-create、event ごとに 4 slot 全部 fill）。REST `/api/histograms/:m/:c?type=user_info0..3`、HistogramType に追加、setup-tab dropdown / view-tab fetch / expand-dialog 全部対応。

実機検証 (172.18.4.56、TestPulse 2ch ~10kHz、run 201)：
- Energy 1D: 205,042 counts ✅
- UserInfo[0..=3] 1D: 各 ~205,000 counts ✅
- UserInfo[0] non-zero bins: 508/512（広いスペクトラム、TestPulse 振幅検出らしい分布） ✅

#### スキップ (ユーザー方針)
- ~~Z 軸 Log/Linear トグル~~ — 既に `logScale` プロパティ実装済、ユーザー要望薄
- ~~SetInRun whitelist~~ — オフライン解析中心の運用方針

---

## Phase 2.5 — 新 FW + 波形 + dev 環境 (2026-04-29 完了)

Phase 2 締め後にやってきた追加作業。AMax FW の世代交代 + waveform 表示の精度向上 + gant 開発機セットアップを 1 日でまとめて消化。

### 新 FW 移植（caenlist firmware32_4input、register `Name` schema）✅
FW 開発者が `RegisterFile_21last.json` をリリース。スキーマが per-channel `Path: page_amax_energy_0/POLARITY` から単一 `Name: page_amax_energy_POLARITY` に変わり、page base が `0x800000` → `0x100000`、`AMAX_delay` → `DELAY_SHAPING` rename + `SHAP_TRIGG` / `SHAP_BL_HOLD` 追加。

- [x] **Codegen 全面対応**: `Register` 構造体が `Path` / `Name` 両対応、`fw_key()` で `_<digit>/` 接頭辞を剥がして fw_params キー一致。`--page-base` default を `0x100000` / `--page-stride` default を `0` (ch0 only) に変更。レガシー FW は引数で旧値を渡せば動く
- [x] **`channel_writes()` helper 自動生成**: `handle.rs` の hand-written 24-field 配列を削除、codegen が `if let Some(v) = config.<field> { writes.push(...); }` 列を吐く。FW が register を追加・削除しても `handle.rs` には触らずに済む
- [x] **`fw_params.json` 更新**: `AMAX_delay` 削除 + `DELAY_SHAPING` / `SHAP_TRIGG` / `SHAP_BL_HOLD` 追加 + `Delayed_READ` を readonly_patterns に追加
- [x] **`amax_56.json` 更新**: 26 fields の新 FW 値、`num_channels: 2` キープ (PAGE_STRIDE=0 だが apply ループは 2 回回す必要あり — 1 回だけだと FW が triggers を発火しない)

実機検証 (run 300-303 @ 172.18.4.56): 26 AMax registers × 2ch = 52 writes Apply、ch0 1 kHz、ch1 0、波形・1D・2D 全部 amax_viewer と一致。

### Waveform 表示の精度向上 ✅
- [x] **`waveforms_enabled` を OpenDPP endpoint format JSON に plumb** — AMax FW は WAVEFORM フィールドを format に含めないと波形を吐かない。`BoardConfig.waveforms_enabled: bool` を `configure_opendpp_endpoint` 呼び出しに連動。Settings → Waveform tab に AMax 用 toggle 追加
- [x] **`selector_wave=0` (FW デフォルト) で生 ADC 1 stream** — 1 にすると 4-値 interleaved デバッグデータが返る。FW dev は今 1 信号だけ使う
- [x] **`opendpp_to_event_data` で waveform を `analog_probe1` に転送** — 1024 sample × `& 0x3FFF` mask
- [x] **`Waveform.analog_probe1_is_signed` / `analog_probe2_is_signed` フラグ** — PHA1 のみ true (sign_extend_14bit)、PSD1/PSD2/AMax は false (`& 0x3FFF`)。frontend は flag を見て signed のときだけ +8191 centering 適用、unsigned は raw scale で描画
- [x] **2D heatmap auto-zoom + visualMap 改善**: bin left-edge 軸ラベル化、populated `(xi>0 && yi>0)` のみ表示+ズーム計算、visualMap range を毎 poll で `[min, max]` にリセット (drag は transient)、`outOfRange.color` で範囲外を薄いグレー表示

実機検証: ch0 入力信号で baseline ~8228、peak ~9712、p2p ~1500 ADC のステップパルスがクリアに見える。signed/unsigned 切替で PHA1 trapezoid 互換も維持。

### Dev 環境: gant@172.18.6.114 ✅
- [x] **`/media/raid1/delila-rs` を `origin/master` に同期** — local 修正は `git reset --hard` で破棄、build 1m46s、546 tests pass
- [x] **MongoDB を TOML から読めるよう拡張**: `OperatorFileConfig.mongodb: Option<MongoConfig>` 追加、CLI > TOML > None の優先順位。`config_amax_56.toml` に `[operator.mongodb]` ブロック追加 (Docker MongoDB 上の `delila` database)
- [x] **AMax_56 用 config (toml + json) を gant に scp** — `./target/release/operator --config config/config_amax_56.toml` だけで Mongo 接続まで完結

### 関連 commits (Phase 2.5)
- `009af4e` config(amax_56): match amax_viewer signal-verified register set
- `0ba78f6` feat(amax): support new 32-channel FW (page 0x100000) + heatmap polish
- `89a0cab` fix(amax): keep num_channels=2 — the new FW needs duplicate writes
- `5737b1d` feat(waveform): per-probe is_signed flag, drop unconditional +8191 offset
- `9d49f63` feat(operator): mongodb config from TOML (CLI flags still override)

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
