# 60. AMax FW 更新を「1コマンド・自己設定」化

**Status: COMPLETED（2026-06-26 実装・検証済）**
作成: 2026-06-25 / 承認済プラン: `~/.claude/plans/plan-mode-encapsulated-swan.md`

## 実装結果（2026-06-26）
全4フェーズ完了。検証: `cargo test --bin amax_codegen` 21 pass（17June ゴールデン・安全弁 Err・prefer 切替含む）/ `cargo clippy --features dev-tools --tests -- -D warnings` クリーン / release build OK / `npm run build` OK。
- **Phase 1** `src/bin/amax_codegen.rs`: `derive_layout()`（page_base/stride/broadcast_base 自動導出、ch0 優先デフォルト）+ `assert_broadcast_layout_matches()`（両群レイアウト不一致で Err）+ フラグ Option override 化（hex パーサ `parse_u32_auto`）+ `--broadcast-base` + stderr 変更サマリ + 生成物に `BROADCAST_BASE`/`broadcast_register_byte_addr()` + TS に `AMAX_PARAMS_BY_CATEGORY`。
- **Phase 2** `handle.rs`: `r::broadcast_register_byte_addr(offset)` 参照に置換（手書き const 削除）。
- **Phase 3** `scripts/update_amax_fw.sh`: codegen→fmt→(opt gen_defs)→release build→npm build→status。`--with-viewer`/`--no-ui` 対応。
- **Phase 4** UI: `channel-params.ts`（ChannelCategory open enum 化、`channelCategoryLabel()` fallback、`CATEGORY_PARAMS.AMax` を `AMAX_PARAMS_BY_CATEGORY` から動的構築、`getFirmwareCategories()`）、`digitizer-settings`（`extraAmaxCategories` で generic タブ `@for`）、`waveform`（categoryGrid を firmware 認識化）。試験カテゴリ `debug` を流して新タブが手編集なしで出ることをビルドで実証。
- 生成回帰: フラグ無し codegen→`amax_registers_generated.rs` の diff は BROADCAST_BASE/helper のみ、struct・他は不変＝17June 既知良好出力を再現。
- **注意（罠ハマり1件）**: Angular `template:` バッククォート内の HTML コメントに `` `debug` `` とバッククォートを書くとテンプレート文字列が閉じてビルド崩壊。コメント内バッククォート厳禁。
- **未コミット**: 生成 Rust/TS + `web/operator-ui/dist/`(19 files) を同一コミットに含めること（CLAUDE.md dist ポリシー）。実機 AMax での apply ログ確認（`r::BROADCAST_BASE==0x200`）は HW 接続時に。

## gant 実機検証で判明 → 追加修正（2026-06-26）
gant(172.18.6.114, AMax dev機)で一連を実行確認。**node/npm 無し**（cargo 1.95 のみ）。
1. **script を npm graceful degrade 化**: npm 不在なら die せず警告してUIスキップ（TS ソースは更新済、dist は node機で再ビルド＋commit）。`scripts/update_amax_fw.sh`。gant では Rust側(codegen+build)のみ完走し、dist は別機案件。
2. **handle.rs の全フィールド手書きマージを codegen 化（重要）**: `apply_amax_channel_config` が `merged = AMaxChannelConfig{...24フィールド...}` を手書き列挙していたため、FW がレジスタを**増減**すると struct とズレてビルド不能（gant の 11june=subset で E0560/E0609 発生）。→ codegen が `merge_amax_channel_config(override, defaults)` を生成、handle.rs は1行呼び出しに。これで add/remove も handle.rs 編集不要に（当初の目標を本当に達成）。
   - bootstrap 注意: merge fn を持たない状態から持つ状態へ移行する初回のみ、handle.rs が未生成 fn を参照して lib がコンパイルできず codegen が走れない鶏卵。一時スタブで回避済。**コミット後は生成物に merge fn が入るので以後問題なし**。
3. gant 再検証: 一貫4ファイル+script を push し 11june で実行 → **cargo build 成功**（前回の E0560 解消）+ npm graceful skip 確認。検証後 gant は git checkout で完全復元（クリーン）。
- 罠追記: `cargo run --bin amax_codegen` は lib(handle.rs含む)をコンパイルするので、生成物と handle.rs は常に整合させること。

## gant に Node 導入（2026-06-26）— 前提訂正
**重要な前提**: AMax 開発者は **amax_viewer を使わず Operator UI(Angular) で FW チューニング**する方針。だから本作業をしている。よって gant でも web UI が新レジスタを反映できる必要があり、Angular(dist ビルド)が要る。
- ユーザー決定: **gant に node を入れる**（user-local, sudo不要）。
- 導入済: **Node 22.23.1 + npm 10.9.8** を `~/.local/node` に展開、PATH を `~/.profile`+`~/.bashrc` に追記。`web/operator-ui/node_modules` も `npm ci` 済。
- 検証: `scripts/update_amax_fw.sh FW/.../RegisterFile.json` が gant 単体で **codegen→cargo build→`npm run build`(dist 生成) まで1コマンド完走**。npm skip 警告は出ず dist まで焼ける。→ 二マシン運用は不要に。
- 新マシン移行時の再現: `curl https://nodejs.org/dist/latest-v22.x/node-v22*-linux-x64.tar.xz` を `~/.local/node` へ展開＋PATH追記＋`cd web/operator-ui && npm ci`。gant は nodejs.org/npm registry 到達可・glibc 2.35。
- 注意: 本機能一式はまだ **Mac でローカル未コミット**。gant が使うにはコミット→gant で pull が必要。

## 目的
AMax FW 開発者が「1コマンド」で本体・UI・amax_viewer を更新できるようにする。
FW 再ビルド毎の儀式（page-base/stride/prefer フラグ手当て + handle.rs の BROADCAST_BASE 手編集 + build×2 + commit）を撤廃。

ユーザー確定の制約：① 静的型付け維持 ② 1コマンドなら再コンパイル可 ③ アドレス移動もレジスタ追加も両対応 ④ UI 新カテゴリ自動タブ化。

## 設計の地盤（実データ確定済み）
`FW/20260617/RegisterFile_17june.json` 実測：
- broadcast 群 `page_amax_energy/<NAME>` 29件 min=**0x200**、per-channel 群 `page_amax_energy_4_<N>/<NAME>` ch0=**0x8000** stride=**0x200**。
- 共通名29件で `(addr_bc−0x200)==(addr_ch0−0x8000)` mismatch ゼロ → 両ページ in-page レイアウト一致。
- committed 生成物 = per-channel ch0 群（PAGE_BASE=0x8000 / PAGE_STRIDE=0x200 / REG_PRETRIGGER_INPUT=0x0）。
- → 正準=ch0群、broadcast_base=min(bc)=0x200 は導出可能。「ch0群あれば優先」が新デフォルト（**現行の broadcast 優先から挙動変更**→既存 era テスト期待値更新が要る。17June ゴールデンが安全網）。

## 実装フェーズ（推奨順）
- **Phase 1** `src/bin/amax_codegen.rs`: `derive_layout()` 新設で page_base/stride/broadcast_base 自動導出、安全弁 `assert_broadcast_layout_matches()`（両群レイアウト不一致で Err 停止＝FW破壊ガード）、既存フラグを Option override 化＋`--broadcast-base` 追加、変更サマリ stderr 出力、生成物に `pub const BROADCAST_BASE` + `broadcast_register_byte_addr()` を emit、回帰テスト（17June ゴールデン / 安全弁 Err / prefer 切替 / 既存テスト更新）。
- **Phase 2** `src/reader/caen/handle.rs`（~L1056）: ローカル `const BROADCAST_BASE=0x200` 削除 → `r::broadcast_register_byte_addr(offset)` 参照。
- **Phase 3** `scripts/update_amax_fw.sh` 新規（`scripts/setup_caen_felib.sh` の house style）: codegen→（任意 gen_defs で viewer register_defs 再生成）→cargo build --release→npm run build→git add 案内。
- **Phase 4** UI 自動タブ化: `channel-params.ts`（ChannelCategory 許容化・ラベル PascalCase fallback・CATEGORY_PARAMS.AMax を `AMAX_PARAM_CATEGORIES` から動的構築、`AMAX_CH_TRIGGER_MASK` splice 保持）、`digitizer-settings.component.ts`（input/trigger/energy/amax 以外を `@for` で generic タブ追加、既存タブ温存）、`waveform.component.ts`（カテゴリ源を firmware 認識に）。

## 次に着手すること（明日ここから）
1. TodoWrite でフェーズ分解 → Phase 1 着手。
2. `amax_codegen.rs` の `main()` 全体・既存 `tests` モジュールを再読 → `derive_layout()` + 安全弁を実装。
3. フラグ無し codegen 再実行で `git diff` が BROADCAST_BASE 追加以外で空（17June 再現）を確認。

## 検証
- `cargo test --bin amax_codegen` / `cargo fmt && cargo clippy --tests -- -D warnings && cargo build --release` / `cd web/operator-ui && npm run build`。
- 生成回帰: フラグ無し codegen→`git diff` が BROADCAST_BASE 追加以外で空。
- handle.rs: `r::BROADCAST_BASE==0x200`。
- UI: fw_params に試験カテゴリ追加→新タブが手編集なしで出る→戻す。

## critical files
`src/bin/amax_codegen.rs` / `src/reader/caen/handle.rs` / `src/reader/caen/amax_registers_generated.rs`+`amax_registers.rs` / `FW/20260617/RegisterFile_17june.json` / `scripts/update_amax_fw.sh`(新規) / `tools/amax_viewer/src/bin/gen_defs.rs`+`fw_params.json` / `web/operator-ui/src/app/models/channel-params.ts` ほか UI 3 ファイル。
