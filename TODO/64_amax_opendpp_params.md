# TODO 64 — AMax UI に OpenDPP（標準 DevTree）パラメータ設定を追加

**Status: 📋 PLANNING (2026-07-13)**
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

### Phase A — ground truth: DPP_OPEN DevTree ダンプ【小・Phase 0 と並行可】

- gant の AMax 実機に `caen_info <url> --devtree` → `docs/devtree_examples/vx2730_dppopen_snXXXX.json`
  として保存。公開パラメータ + allowedvalues + range が確定する。
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

- [ ] Phase 0: AMax の Settings/Tune Up 両方の Input タブに DC Offset が出て、Apply で
      実機に反映される（gant 実機で確認）
- [ ] Phase 0: `amax.offset`（trapezoid オフセット）との UI 上の区別（ラベル/tooltip、§2 の確定
      情報に従う）
- [ ] Phase 0: ChGain も同時に露出（デフォルト 0 dB、「分光では 0 dB 推奨」tooltip 付き）
- [ ] Phase A: DevTree ダンプを `docs/devtree_examples/` にコミット
- [ ] ChGain × trapezoid の分解能影響を実測（既知ピーク FWHM を 0/6/12 dB 比較）+
      FW 開発者にデータパス位置/内部ビット幅を確認
- [ ] Phase B/D は Phase A の結果と FW 開発者の追加要望を見て判断
