# TODO 64 — AMax UI に OpenDPP（標準 DevTree）パラメータ設定を追加

**Status: 🚧 Phase 0 ✅実装+実機検証済 / Phase A ✅完了 (2026-07-16) — 残 = ChGain FWHM 実測 + FW 開発者確認**
**発端:** AMax FW 開発者からのリクエスト。「レジスタベースのパラメータセッター UI は素晴らしい。
加えて **OpenDPP（DPP_OPEN 標準 DevTree）のパラメータも設定できるようにしたい」
**実スコープ（開発者ヒアリング 2026-07-13）:** **必須 = DC Offset のみ。あれば嬉しい = ゲイン**。
将来の柔軟性は Phase D（DevTree 駆動 codegen）で担保する方針。

---

## 1. 調査結果（2026-07-13、コード検証済み）

AMax の設定は 3 系統。カバレッジ:

| 系統 | バックエンド | UI | 状態 |
|---|---|---|---|
| ① カスタム FW レジスタ | `apply_amax_channel_config`（WriteUserRegister、codegen） | `AMAX_PARAMS_BY_CATEGORY` 4 タブ | ✅ 完備 |
| ② 標準 DevTree board | `add_board_parameters` + `board.extra`（clocksource 等） | isDig2 ガードで AMax にも表示 | ✅ ほぼ済み |
| ③ 標準 DevTree channel（= OpenDPP） | **`PSD2_AMAX_PARAMS` を共有**（`src/config/channel_param_tables.rs:45`） | **AMax タブに出ない**（例外 = `ch_trigger_mask` の手動 splice のみ） | ❌ 本件のギャップ |

**重要な発見:**

1. **DCOffset / ChGain のバックエンド送信経路は既に生きている**。
   `PSD2_AMAX_PARAMS` に `("DCOffset", dc_offset)` `("ChGain", vga_gain)` が入っており、AMax は
   このテーブルを使う。実際 `config/digitizers/amax_56.json` は `dc_offset` を populate して
   運用中 → **ギャップは UI 露出だけ**（channel-params.ts の AMax タブ構成に flat param が無い）。
2. **silent drop の実害例あり（TODO 58 M6 に直結）**: `config/digitizers/amax_test.json` の
   `signal_offset` / `wave_data_source` / `itl_connect` / `ch_gain`（channel）、
   `clock_source` / `en_clock_out_fp` 等（board トップレベル）は **ChannelConfig/BoardConfig に
   存在しないキーで serde が無言で捨てている**（`extra` は flatten でなく名前付きキー）。
   過去に OpenDPP パラメータを設定しようとして無言で効かなかった形跡。
3. **record length は意図的ブロック — 維持必須**: AMax の probe window は FW 固定。
   `chrecordlengths` を送ると恒久 CAEN -6（`add_board_parameters` にコメントあり）。
4. **DPP_OPEN の DevTree ダンプがリポジトリに無い**（`docs/devtree_examples/` は
   PSD1/PHA1/PSD2/PHA2 のみ）。AMax FW が公開する標準パラメータと allowedvalues の
   一次資料が不在。PSD2 テーブル共有が事故っていないのは AMax config が該当フィールドを
   None のままにしているからで、構造的には脆い。

## 2. 実装フェーズ

### Phase 0 — 最小パス: DC Offset（+ ゲイン）を AMax タブに出す【本命・小】

**✅ 実装済み（2026-07-16）:**

- **FW レジスタ OFFSET のラベルリネーム（codegen）**: `tools/amax_viewer/fw_params.json` の
  `"OFFSET"` を `"label": "DC Offset"` → `"label": "Offset (Trapezoid)"` に変更し
  `amax_codegen -- FW/20260617/RegisterFile_17june.json` で再生成。3 つの生成ファイル
  （`src/config/amax_generated.rs` / `src/reader/caen/amax_registers_generated.rs` /
  `web/operator-ui/src/app/models/amax-generated.ts`）の差分は **ラベル/doc コメントのみ**
  （アドレス・フィールド名 `offset` は不変）。
- **UI splice**: `web/operator-ui/src/app/models/channel-params.ts` に `AMAX_CH_TRIGGER_MASK`
  と同じパターンで `AMAX_DC_OFFSET`（key `dc_offset`, `%`, 0–100, tooltip でトラペゾイド
  オフセットと区別）+ `AMAX_VGA_GAIN`（key `vga_gain`, dB, 0–29, 「分光では 0 dB」tooltip）を
  追加し、AMax の Input タブへ splice（手動 splice が 3 件 = DC Offset / VGA Gain /
  Ch Trigger Mask に増えたので周辺コメントも更新）。
- **config デフォルト**: `config/digitizers/amax_56.json` の channel_defaults に `"vga_gain": 0`
  を追加（`dc_offset: 50.0` の隣）。
- **命名判断**: 「DC Offset」の名前は **DevTree 側（入力信号 DC オフセット）に残し**、FW レジスタ側を
  「Offset (Trapezoid)」にリネームした。理由 = ① DevTree には別途 `SignalOffset` param が存在し
  「DC Offset」は DevTree DCOffset を指すのが自然、② CoMPASS / 他 FW とのラベル一貫性
  （どの FW でも「DC Offset」= 入力 DC オフセット）。
- 品質ゲート: `npm run build`（dist コミット済）+ `cargo fmt && cargo clippy --tests -D warnings
  && cargo test`（689 tests pass）通過。

**✅ 実機検証済み（2026-07-16、gant + VX2730 SN52622 / 13july trigger-logic FW）:**

- Tune Up mode で `POST /api/tuneup/apply/0` に `dc_offset: 30.0` + `vga_gain: 0` → 成功。
  **DevTree 直読み（caen_info --devtree）で全 ch `dcoffset = 30.001` / `chgain = 0` を確認**
  （ground truth）。DevTree の range/increment（dcoffset 0–100/0.001、chgain 0–29/1）は
  UI 定義と完全一致。検証後 `dc_offset: 50.0` に復元済み。
- 注意: 検証は 13 July FW への統合後（gant の未コミット FW 統合 + チューニングを commit
  `96e111c` として取り込み、`RegisterFile13iulie.json` で codegen 再実行した状態）。

**⚠️ 運用上の落とし穴（2026-07-16 発見）**: reader が dig2 接続を保持中に `caen_info --devtree`
を実行すると **reader の接続が死ぬ（以後の Apply が CAEN -15）**。Tune Up は -15 を自動再接続
しないので DAQ 再起動が必要。**DevTree ダンプは DAQ 停止中に取ること**。

**残（後続）:** ChGain × trapezoid の FWHM 実測（0/6/12 dB）、FW 開発者への確認事項（下記）。

---

- `web/operator-ui/src/app/models/channel-params.ts`: `AMAX_CH_TRIGGER_MASK` と同じ splice
  パターンで、PSD2 の `dc_offset`（+ `vga_gain`）の ChannelParamDef を AMax の Input タブへ。
- DigitizerService の flat-key map は `dc_offset` を既知キーとして持つはず（PSD2 と共有）→
  ほぼ UI 定義の追加だけで end-to-end が繋がる見込み。
- **✅ 確定（2026-07-13、x2730 Open DPP CUP doc 2025022602 + ユーザー確認）: 2つの「オフセット」は別物**。
  - カスタムレジスタ `amax.offset`（FW の OFFSET、Input タブ既存）= **Trapezoid フィルタの
    オフセット**（FW デジタル処理側）
  - DevTree `DCOffset`（Input Signal Conditioning）= **入力信号の DC オフセット**
    （アナログフロントエンド側）
  - 両方が Input タブに並ぶので **UI ラベル/tooltip で明確に区別必須**。
    例: "DC Offset (input, DevTree)" vs "Offset (trapezoid, FW)"。
  - 同カテゴリに `SignalOffset` / `GainFactor` / `ADCToVolts`(RO) / `EnOffsetCalibration` も存在。
- **✅ 確定: `ChGain` は x2730 Open DPP に実在**（"Sets the gain of the Variable Gain
  Amplifiers (VGA)"、0–29 dB / 1 dB step / Set-in-Run 可）。実態は波高を変えるデジタルアンプ。
  - **運用上の注意（PSD/チャージ積分の実績）**: チャージ積分では **0 dB 必須** — 上げると
    高エネルギー側のエネルギー分解能が著しく劣化（ヘッドルーム喪失/クリップ + FW 内部演算の
    飽和）。
  - **未解決の物理問題**: Trapezoid フィルタ（AMax）でも同じ劣化が出るか。長いシェーピングは
    広帯域ノイズを平均化するのでノイズ面の罰は短ゲート積分より軽いはずだが、**クリップ/飽和の
    罰はフィルタで救えず同一**のはず。→ UI 露出後に実測で決着（既知ピークの FWHM を
    ChGain 0/6/12 dB で比較）。**FW 開発者への確認事項: ChGain が AMax データパスのどこに
    効くか + FW 内部ビット幅（trapezoid アキュムレータの飽和条件）**。
  - デフォルトは **0 dB** とし、UI tooltip に「分光では 0 dB 推奨」を明記する。

### Phase A — ground truth: DPP_OPEN DevTree ダンプ【✅ 完了 2026-07-16】

- ✅ `docs/devtree_examples/vx2730_dppopen_sn52622.json` にコミット済み（gant、13july
  trigger-logic FW、dcoffset=30.001 は検証テスト中の値）。
- **確定した DPP_OPEN のチャンネルパラメータは 10 個のみ**:
  `chenable / dcoffset / chgain / signaloffset / gainfactor / itlconnect / wavedatasource`
  （writable）+ `adctovolts / chstatus`（RO）+ `handle`（メタ）。
- **含意**:
  1. `SignalOffset` の実在を実機で確認 → 「DC Offset」の命名判断（DevTree 側に残す）の裏付け。
  2. **`channelstriggermask` はこの FW ビルドの DevTree に存在しない**。AMax の
     `ch_trigger_mask` splice（PSD2 共有パス）は現行 config が未設定なので実害なしだが、
     設定すると FELib 拒否の可能性 → Phase B で扱いを確定する。
  3. PSD2 テーブル共有（37 項目）は 10 項目の実態に対して過大 → Phase B の専用テーブルは
     この 10 項目が正。trigger/energy/waveform 系は全てカスタムレジスタ側。
- FW 開発者に見せて「追加で欲しいもの」を選んでもらう入力資料にもなる。

### Phase B — 専用テーブル化【中・必要になったら】

- PSD2 テーブル共有をやめ、ダンプに基づく `AMAX_OPENDPP_PARAMS` を新設。
  DPP_OPEN に存在しない PSD2 項目（gate/CFD/charge/coincidence 系）を構造的に排除。
- 不足フィールド（`signal_offset` / `itl_connect` / `wave_data_source` 等、ダンプで実在確認
  できたもの）を ChannelConfig に追加。reclen 除外は維持。

### Phase D — DevTree 駆動 codegen【将来の柔軟性、開発者の好みに合う】

- `amax_codegen`（RegisterFile → TS/Rust）と同じ発想で、**DevTree ダンプ JSON → OpenDPP
  パラメータテーブル（TS ChannelParamDef + Rust ChannelParamEntry）を生成**する codegen。
- FW 更新で OpenDPP パラメータが増減しても `--devtree` ダンプ → codegen 再実行で自動追従。
  allowedvalues → enum options、range → min/max がそのまま UI メタになる。
- 「レジスタは codegen、OpenDPP も codegen」で設定系の作りが対称になる。

## 3. 関連事項

- **TODO 58 M6（unknown-key 警告、defer 中）**: 本件の発見 2（silent drop）はその実害例。
  Phase B で ChannelConfig にフィールドを足す際、`serde_ignored` の導入を同時に検討する価値あり。
- SET_IN_RUN_PSD2 を AMax が共有している点（`digitizer.rs:1539`）も、専用テーブル化（Phase B）の
  際に AMax 実態に合わせて見直す。

## 4. 完了条件

- [x] Phase 0 (実装): AMax の Settings/Tune Up 両方の Input タブに DC Offset が出る
      （`channel-params.ts` splice、`AMAX_DC_OFFSET`）
- [x] Phase 0 (実機検証): gant 実機で Tune Up Apply → DevTree 直読みで dcoffset/chgain
      反映確認（2026-07-16、SN52622 / 13july FW）
- [x] Phase 0: `amax.offset` を "Offset (Trapezoid)" にリネームして DevTree DCOffset
      （"DC Offset"）と UI 上で区別（ラベル/tooltip、§2 の確定情報に従う）
- [x] Phase 0: ChGain も同時に露出（`vga_gain`、デフォルト 0 dB、「分光では 0 dB 推奨」tooltip 付き）
- [x] Phase A: DevTree ダンプを `docs/devtree_examples/vx2730_dppopen_sn52622.json` にコミット
      （チャンネルパラメータは 10 個のみと確定、§Phase A の含意参照）
- [ ] ChGain × trapezoid の分解能影響を実測（既知ピーク FWHM を 0/6/12 dB 比較）+
      FW 開発者にデータパス位置/内部ビット幅を確認
- [ ] Phase B/D は Phase A の結果と FW 開発者の追加要望を見て判断
