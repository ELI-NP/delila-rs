# TODO 59 — ELIADE PHA エネルギー分解能 自動チューン（SW 台形リプレイ）【日本語版】

**Status: 📋 PLANNING (2026-06-16, 改訂 2026-07-13)**
**担当:** Aogaki + Claude
**実験:** ELIADE（8× Clover HPGe アレイ, ELI-NP）
**タイムライン:** ベンチテスト 2026年6月開始 · Ge 分解能チューンアップ 2026年いっぱい · ビーム 2027年1月〜

> 英語版（同僚共有用）: [59_eliade_trap_autotune.md](59_eliade_trap_autotune.md)。内容は同一。

> **改訂 2026-07-13:** es2 実機 + コード検証の壁打ちを反映。主な変更:
> ① Phase 0 を「低オンライン閾値 + `adc_min` ゲート」戦略に書き換え（データ量 64 GB 設計込み）。
> ② **AMax trap コア流用は消滅**（開発者メール無反応 = 提供なしと判断。そもそも tree 内の
> `amax.rs` は decoder のみで SW trap 実装は存在しなかった）→ **スクラッチ実装 + probe-overlay
> 逆工学**方式へ転換。③ 20 µs 波形の実機動作 + DELILA 全経路（decoder/.delila v3/delila2root）の
> 想定を検証済み。④ RCCR2 トリガーのオフライン最適化の可否と限界を §6b に反映。

---

## 0. ELIADE DAQ の前提（1段落）

ELIADE は HPGe クローバー用に **4× CAEN V1725（DPP-PHA, 16ch, 250 MS/s, 各10ch 使用）** と、
ホストあたり **1× V1730（DPP-PSD）** を、最大 **8 ホスト**（1ホスト運用も8ホスト運用も両対応必須）
で走らせる。全デジタイザは **コモン外部クロック**（ドリフトなし＝定数の start-phase オフセットのみ）と、
1枚の SW 制御ボードが起動する **デイジーチェーン RUN 信号** を共有。8 クローバー間の γ-γ coincidence が必須。

本 TODO は **エネルギー分解能の自動チューン（2026年の成果物）だけ**を扱う。残り3ブロックは別管理（§7 に要約）:

1. `start_delay` グローバル自動較正（HW タイムスタンプ原点合わせ）
2. チェーン topology 管理（full vs subset、topology-tag 付き較正テーブル）
3. EB 時間オフセット（PHA↔PSD 定数、first-run データから経験的に決定）
4. **PHA trap 自動チューン ← 本ファイル**

---

## 1. ゴール

per-channel の DPP-PHA 台形フィルタ パラメータ選定を完全自動化し、既知 γ ピークの **FWHM を最小化**する。
**オペレータが手で grid をいじる工程をゼロにする。** オペレータの手作業は「Phase 0 で波形ランを1回録る」
「Phase 3 で実機確認に頷く」だけになる。

### なぜ今これが現実的になったか（核心の気づき）

歴史的な障壁は、総当たりチューンが *オンライン* で **acquisition 時間が律速** だったこと。候補パラメータ
ごとに、安定 FWHM が出るだけのカウントを貯める新規ランが要る。だから「高価な eval 回数を最小化する」
ベイズ最適化が必要に見えた。

**波形オフラインリプレイでこれを丸ごと回避する。** 台形フィルタは「デジタイズ済み preamp 波形」の
*下流* にある。生入力波形を一度録れば、*同じ保存波形* に *任意の* trap パラメータをソフトで
**1 eval ≈ ms** で再適用できる。acquisition がもはや eval 毎ではなくなる。律速が消えると:

- **plain grid search で十分** — BO 不要。
- 最適化は **ノート PC でオフライン**、ハード不要、いつでも。
- 既存の「生 `.delila` 常時保存、オフライン再処理が正道」思想にそのまま乗る（`delila2root`、
  オフライン EB リプレイと同じ）。
- **同一イベント集合で全 grid 点を評価**するため、grid 点間の統計誤差は同相で相関し、序列比較では
  キャンセルされる。オンラインスキャン（各点が別統計・別時間帯 = 温度/HV ドリフト混入）に対する
  本質的優位。

---

## 2. パイプライン全体像

```
Phase 0  録る（一度、実機で）   : 通常 Run + waveforms_enabled, probe=Input, FW energy 同録,
                                  低オンライン閾値 + adc_min ゲート, 既知 γ 源, 20 µs 窓。
Phase 1  SW-trap validation   : FW 自身の param で同一波形をリプレイし、FW energy と
                                  PER-EVENT で比較 + probe2 台形トレースとサンプル単位照合。
Phase 2  オフライン最適化       : 同じ保存波形上で param を grid 掃き、ch ごとに
                                  FWHM(keV) 最小化。純 CPU。
Phase 3  実機 verify          : 最適 param を FW に焼き、短ランで実 FWHM が SW 予測と
                                  一致するか確認。config に確定。
```

Phase 1-2 は **1個のオフラインツール** に収まる: `.delila`（波形 + FW energy）を読み、per-channel
config patch を吐く新 bin `pha_trap_tune`（`dev-tools` feature、`delila2root` と同族のオフライン系）。

---

## 3. Phase 0 — 録る（後から直せない制約）

### 3.1 実機・実装の事前検証（2026-07-13 完了）

- **20 µs 窓は実機動作確認済み**: es2（172.18.4.132, V1725 SN217, PHA）の Tune Up で
  record_length 20000 ns = 5000 サンプル @4 ns の波形取得を確認。
- **DELILA 全経路にサンプル数の暗黙上限なし**（コード検証済み）:
  - PHA1 デコーダ: `num_samples_wave: u16` × 8 サンプル/単位 → **上限 ~52万サンプル**
    （`src/reader/decoder/psd1_pha1_common.rs:118`）
  - `.delila` v3: 「固定長」は**フィールド数固定**の意味で、波形配列は可変長 `Vec<i16>`
    （`src/recorder/format.rs:30`）
  - `delila2root`: `std::vector` ブランチ、固定サンプル数の仮定なし
- **イベントサイズ ≈ 20 kB**（5000 サンプル時）: analog probe 10 kB + digital probe 10 kB。
  **PHA1 の digital probe は無効化不可能** — 波形ワードに bit15=Tn(D0, Trigger 固定) /
  bit14=DP(D1, 選択制) が常時埋込まれ、デコーダも無条件に u8/sample 非パックで展開する
  （`src/reader/decoder/pha1.rs:134-168`）。UI に "None" が無いのは FW 仕様通り。
  → ただし §6b の通り **D0=Trigger は trigger エミュレータの ground truth** なので、
  このキャンペーンでは digital probe は「無駄」ではなく録る価値がある。

### 3.2 ラン種別は 2 つ（capture と validation を分ける）

| ラン | vtrace | dtrace | レート | 用途 |
|------|--------|--------|--------|------|
| **capture run**（Phase 0 本体） | probe_0 = **Input** 単独 | D0=Trigger(固定), D1 任意 | フル 250 MS/s (4 ns/sample) | Phase 2 最適化スキャン用データ |
| **validation run**（Phase 1 用） | **dual trace: Input + Trapezoid** | D1 = **Peaking** | dual trace で実効半減（要実機確認: interleave → 8 ns/sample 相当） | SW trap の FW 照合 |

validation run は FW 自身の台形トレース（probe2）+ energy 抽出窓（D1=Peaking）+ FW energy を
per-event で持つので、SW 実装の逆工学顕微鏡になる（§4）。半レートで検証した実装をフルレートに
適用する際、**rise/flat のサンプル数換算（4 vs 8 ns/sample）を混同しないこと**。

### 3.3 閾値と adc_min — ノイズはディスクの手前で捨てる

**戦略: オンライン閾値（RCCR2 threshold）を思い切り下げ、`adc_min` でノイズをディスク手前で落とす。**

- **閾値を下げる理由**: ①録れたデータは「オンライントリガーが発火した」条件付き分布なので、
  §6b のトリガー最適化で試す候補閾値の**最小値よりさらに下げて**おかないと、オフラインスキャンが
  「見えていないイベント」に対して盲目になる（superset 化）。②低エネルギー効率の評価にも同じ
  データが使える。
- **`adc_min`（reader 側、per-board）**: 低閾値化で増えるノイズトリガーは FW energy が小さいので
  `adc_min` の energy floor がディスク到達前に捨てる。総量とレートの両方を制御。
- **per-board 制約への対処 — ゲイン整列**: 個別結晶 ch とサム ch でゲインが数倍違うため、
  同一 board 内で `adc_min` を機能させるには FW energy の座標を揃える必要がある:
  1. **×4 の粗い段差は `coarse_gain`**（X1/X4 = 入力ダイナミックレンジ 2/0.5 Vpp）で吸収。
     これは**アナログのレバー**で、ADC レンジ利用率そのものを改善する（es2 実測で生波形振幅が
     14-bit の ~12% しか使っておらず、1 LSB ≈ 0.7 keV 相当 = 量子化が FWHM ~2 keV に対して
     無視できない水準だった。個別 ch は X4 側へ振って振幅を稼ぐこと）。
  2. **残りの端数は `energy_fine_gain`**（×1.0–10.0）で FW energy の座標だけ揃える。
     **fine gain はデジタル倍率であり、オフライン解析（生波形から再計算）には一切影響しない** —
     用途は adc_min の座標合わせのみ。注意 2 点: min 1.0 なので**上に揃える**方向のみ /
     15-bit energy（32767）の頭打ちに注意（sum peak 2.5 MeV まで見る場合の headroom を確認）。
- **⁶⁰Co のゲート位置**: 1332 keV のコンプトン端は **1118 keV < 1173 keV ピーク**。よって
  `adc_min` を 1173 の少し下に置くと**残るのはほぼ光電ピーク 2 本だけ**になり、イベント数が
  1/10〜1/20 に落ちる。閾値は per-channel の実効値になるよう fine gain 込みで逆算するか、
  保守的に低め（全 ch 共通で余裕を持たせ、データ量 2〜3 割増を許容）に置く。

### 3.4 統計とデータ量の設計

- ゲート後 **10万イベント/ch** → ピークあたり ~5万カウント → FWHM フィット精度 ~0.6%。
  grid 点の序列比較には過剰なほど十分（同一イベント集合による誤差相関も効く）。
- 20 kB/event × 10万 = **2 GB/ch → 32ch 全録で 64 GB**。全結晶の個別最適化データが
  この予算で揃う（「代表 ch だけ録る」妥協は不要）。
- スループット: ソース測定はレートが低く、ゲート後はさらに 1/10 なので Recorder 書込は
  問題にならない。`adc_min` ゲートは**帯域対策ではなく総量対策**（ゲート無しでピーク 5万
  カウントを貯めると全イベント 100〜200万発/ch = TB 級になる）。

### 3.5 後からオフラインで取り戻せない制約（従来からの原則）

1. **波形窓長 ≥ テストする最長シェーピング。** rise ~8 µs まで掃くなら窓は
   baseline + 完全な台形応答 ≈ `2·(rise+flat) + 減衰` を含むこと。**20 µs 窓 + pre-trigger
   数 µs** が現行案。窓が短いと長シェーピングの eval が silent に誤る。
2. **pre-trigger baseline を十分に**（baseline restorer の平均に使う。2–4 µs 目安）。
3. **クリップ禁止**: coarse_gain 変更後、最高エネルギー（sum peak 含む）が ADC レンジ内に
   収まることを確認。
4. **decimation は使わない**（フィルタ応答の等価性が崩れる）。

出力: channel-group ごとに、`{生入力波形, FW energy, FW 使用 param}` を per-event で持つ
`.delila` ファイル（validation run は + 台形 probe + Peaking bit）。

---

## 4. Phase 1 — SW-trap validation（厳密にやるべき部分）

### 4.1 実装方針（改訂 2026-07-13: AMax コア流用は消滅、スクラッチ実装へ）

**旧方針の経緯**: AMax カスタム FW が台形 MCA であり、開発者から trap ロジック（コード）を
入手できる前提で「AMax コア流用」としていた。**メール照会に無反応 = 丁重な辞退と判断**。
またコード検証の結果、tree 内の `src/reader/decoder/amax.rs` は **FW 出力の decoder であって
SW 台形の実装は元々存在しない**ことを確認（950 行、probe レーン/user word の展開のみ）。

**失ったものは骨組みだけで、答えではない**:

- 再帰そのもの（§4.3）は Jordanov-Knoll + UM4380 で完全に公知。~30 行。
- FWHM を決める段 2/4（baseline restorer / energy 抽出窓）は、AMax コードがあっても
  「**725 PHA と要確認**」だった — ターゲットは CAEN PHA の実装であり、AMax コードは
  所詮「別 FW の参考実装」。検証作業は元々丸ごと残っていた。

**新方針: probe-overlay 逆工学。** FW の挙動そのものを毎イベント観測できる材料が揃っている:

| ground truth | 得られるもの | 検証できる段 |
|---|---|---|
| probe2 = Trapezoid トレース | FW 台形の**サンプル単位の実波形**（固定小数の丸め込み） | 段 2+3（どのサンプルからズレるかで局所化） |
| D1 = Peaking ビット | FW の energy 抽出窓の位置・幅 | 段 4 |
| FW energy（毎イベント） | 最終出力 | 全段通し（per-event 残差） |

VHDL を読むより**出力を全段 observable にして突き合わせる方が確実**（読み間違いのリスクも無い）。

**予想される反復ポイント**: BLR の holdoff/freeze 挙動、trap 出力の固定小数スケーリング。
最初から一発で合うとは思わないこと。工数感: 再帰+枠組み ~半日、BLR と段 4 の詰め
（probe 重ね合わせの反復）が本体で数日。

⚠️ **「線型 1:1 energy 対応」は分解能の根拠としては不十分**（原則として維持）。線型対応は
centroid/gain が合うことしか意味しない。**FWHM はフィルタのノイズ伝達特性**（台形の重み、
baseline restorer、rise/flat-top の正確なサンプル数、固定小数の丸め、energy 抽出窓）で決まる。
gain 定数倍は無視して安全（分解能は相対量、毎 eval ピークを再フィットして keV 再較正する）。

### 4.2 validation 基準

捕捉波形を **FW が使った正確な param** で SW trap に通し:

- per-event 残差 `SW_energy − FW_energy`; その std が **≪ ピーク FWHM（理想は ±1 LSB）** を要求。
  **線型フィットの R² ではなく per-event 残差を使う。** per-event 一致が FWHM 再現を保証する; R² は
  しない。
- **SW 台形トレース vs デコード済み `analog_probe2`** をサンプル単位で重ねる。
- **SW の energy 抽出窓 vs D1=Peaking ビット**の位置一致。
- 決定的チェック（FW 動作点）: **`FWHM_SW ≈ FWHM_FW`**。centroid ではなく FWHM が一致して初めて
  最適化に使える。

これはまさに CLAUDE.md ドクトリン（「spec ページ参照 + 実機検証; 分解能を silent に間違えない」—
e641e99 ヒューリスティック、e45e0ec silent-cache 事案）。**Phase 1 が通るまで最適化に進まない。**

### 4.3 台形数式の錨（Jordanov-Knoll; UM4380 準拠）

```
l = k + m                                  # k = rise, m = flat-top （サンプル単位; 725 = 4 ns/sample）
d[n] = v[n] - v[n-k] - v[n-l] + v[n-k-l]
p[n] = p[n-1] + d[n]
r[n] = p[n] + M·d[n]                        # M = pole-zero（減衰 τ をサンプルで）
s[n] = s[n-1] + r[n]
energy ∝ s を flat-top（peaking）窓で平均
```

**pole-zero M は測る、探索しない:** 捕捉波形の preamp 立下りを指数フィット → τ → M、per channel。
（これは CoMPASS の手作業 PZ ステップの自動化にもなる — 単体でも嬉しい副産物。）

### 4.4 モジュール構造 — 段分解 + per-stage 覗き穴

**「Phase 1 がコケた時にどの段がズレてるか局所化できるよう、SW trap を分離可能で覗ける段に切る」**。
モノリシックに書くとデバッグ地獄。各段は中間トレース（素朴に `Vec<f64>` を返すだけで十分、KISS）を
出せるようにする。PHA energy 処理の段分解:

| 段 | 内容 | FWHM に効く? | 実装方針（改訂） |
|----|------|:---:|----------|
| 1 | Input（probe_0=Input 生波形） | — | そのまま |
| 2 | Baseline 計算/restorer（pre-pulse 平均を減算） | **効く** | スクラッチ実装、**probe2 重ねで BLR 挙動を逆工学** |
| 3 | 台形 recursion（d,p,r,s + PZ M） | 効く（シェーピング） | スクラッチ実装（公知の 30 行） |
| 4 | Energy 抽出（peaking 位置で取得、Npk 平均） | **効く**（ノイズ平均） | スクラッチ実装、**D1=Peaking で窓を直接観測** |
| 5 | gain 正規化 → LSB | — | **無視（線型倍, Aogaki 正しい）** |

FWHM を決めるのは台形そのものより **段 2・4（baseline + energy 窓）**。段分解のおかげで
「台形は probe2 と一致、でも energy が合わない → 段 4 の窓だ」式の局所化ができる。

---

## 5. Phase 2 — オフライン最適化（grid で十分）

### 5.1 物理 prior で探索空間を削る

- **Pole-zero M:** 波形から測定（§4.3）→ 固定、探索しない。
- **Trigger threshold:** 強いピークの分解能にはほぼ効かない → 適切に固定（§6b で別途最適化）。
- **Baseline-mean / peak-mean 窓:** 平均を増やすとノイズ減（逓減）vs pile-up → 妥当な最大に設定、
  必要なら粗くチェック。
- **実効探索 ≈ rise × flat-top（+ peaking 位置）: 2–3 次元。** 古典的「分解能 vs シェーピング時間」
  U 字カーブ。

### 5.2 オプティマイザ

オフラインで ms/eval なので、fine grid でも瞬殺:

- coarse grid 例 `rise ∈ {0.5,1,2,3,4,6,8} µs × flat-top ∈ {0.5,1,1.5,2} µs` ≈ 28 点、
  **<1 秒/ch**。
- 最良点周辺を fine grid で詰める。
- **ch は独立** → 各 ch を自分の波形から並列に最適化。
- **BO 不要**（元の動機=オンライン acquisition コストが消えた）。

### 5.3 スキャン戦略 — 深掘り 2ch → 狭域展開（2026-07-13 追加）

仮説（Aogaki）: **同型クローバー結晶なら rise/flat の最適点は近い**（U 字カーブは最適近傍で平坦）。
これを検証可能な形で使う:

1. **代表 ~2ch で全域の深い grid**（rise × flat × 検証用に PZ 近傍も）→ 最適領域を確定
2. **残り全 ch は最適領域まわりの狭い grid のみ**（CPU 時間節約）
3. PZ は全 ch とも波形から個別測定（preamp 個体差、§4.3）
4. 狭域スキャンで最適点が領域の端に張り付く ch があれば、それ自体が診断情報
   （その結晶/preamp は何かおかしい）→ その ch だけ全域スキャンに戻す

**注意 — trap と trigger の交差項**: DPP-PHA の energy は「トリガー時刻から Peaking Time 後」の
台形をサンプルするため、**トリガージッタ/ウォークは flat-top 上のサンプル位置の揺れになる**。
flat-top を詰めるとトリガージッタが分解能の尻尾に出る（flat-top 幅 × ジッタの結合）。
同一データで §6b のトリガースキャンも回せるので、この結合まで観測可能 — flat-top の最終値は
trigger 側の結果を見てから確定する。

### 5.4 FWHM メトリック — 「ピーク移動」の罠を避ける

rise を変えると **gain が変わり、peak centroid が移動**する。フィッタは毎 eval **ピークを自由に
発見**（Gaussian + 線形背景）し、centroid を既知エネルギー（例 1332 keV）に対応させ、**keV の FWHM**
を報告すること。*固定* ch 窓で FWHM を最小化すると間違える。（内側ループは robust な half-max 幅推定で
よい; 最終報告は full Gaussian フィット。）

### 5.5 出力

per channel: 最適 `{rise, flat-top, peaking, PZ M}` + FWHM カーブ（同僚が U 字を sanity-check 用）、
既存の `start_delay` 方式の per-channel config パスで適用できる **config patch** として吐く。

---

## 6. Phase 3 — 実機 verify

最適 param を FW に焼き、短ランで **実** FWHM を測り、SW 予測と一致を確認。一致 → config 確定。
不一致 → SW モデルのギャップ; 段分解（§4.4）がどこを見るか教える。これで spec-ref + 実機検証ループを閉じる。

---

## 6b. Phase 5 — Trigger & Timing 最適化（効率 AND timing 両方）

**同じオフラインリプレイ harness を流用**。トリガ経路（RCCR2 = RC-CR² + smoothing → LED/CFD 弁別;
DELILA param では `threshold`/`input_rise_time`/`fast_disc_smooth`）は台形と同じく入力波形の決定的
関数。config 配管は一部既存（V1743 作業の `ttf_smoothing`, `trigger_edge`, `trigger_threshold_v`）。

### 6b.0 トリガー最適化の原理的限界（2026-07-13 明文化）

台形との決定的な違い: **トリガーは「どのイベントがデータに存在するか」を決める側**。録れたデータは
オンライントリガーの条件付き分布なので:

**オフラインでできる（強力）:**
- RCCR2 エミュレータで threshold × input_rise_time × fast_disc_smooth をブン回す
- トリガータイミングのジッタ/ウォーク評価（ソフト CFD を基準時計に）
- ノイズ交差レート: 録った波形の **pre-trigger 区間（純ノイズ）**への RCCR2 適用で、閾値ごとの
  上向き交差レートを直接数える（トリガーバイアスを受けない）

**オフラインだけでは閉じない:**
- オンライン閾値**以下**のイベントの絶対効率（データに無いものは測れない）
- 実レートでの retrigger / pileup guard / dead time の絡み

**バイアス対策は Phase 0 で織込み済み**（§3.3）: capture run のオンライン閾値を候補最小値より
下げて superset 化。ノイズ増は adc_min が受け止める。

**実装順: 台形 → RCCR2。** probe-overlay 検証枠組み（§4.1）を確立してから同じ構造で横展開する。
RCCR2 の ground truth は **D0 = Trigger（PHA1 で固定・全イベントに常時刻印、§3.1）** — FW の
発火サンプル位置との一致で SW エミュレータを validate する。

### 6b.1 構造 — なぜ両目的が1枠に収まるか（パラメータ軸で分離）

- **閾値** → 純粋に *効率/誤トリガ率* の軸。timing には**効かない**（CFD は振幅独立）。誤トリガ予算が
  許す限り下げる：`threshold = k·σ_TTF`, k ≈ 4–5。
- **smoothing + trigger rise** → *共有* ノブ。増やすと σ_TTF↓（閾値↓＝低E効率↑）だがエッジが鈍る
  （timing↓）。**これが A↔B の緊張**。
- **CFD params（delay/fraction/smoothing）** → timing 専用軸。

なので曖昧なトレードオフではなく **制約付き最適化**：

> **閾値を最小化（低E効率を最大化）。ただし timing 分解能 ≤ coincidence window 予算 かつ
> 誤トリガ率 ≤ 予算。**

window 予算は **run 種別ごとの物理判断**（2026 エネルギーチューンアップ = timing 緩い → 閾値を思い切り
下げる; 2027 ビーム coincidence = timing 厳しい → smoothing 控えめ）。ツールは **Pareto front
（効率床 vs timing σ, smoothing でパラメトライズ）** と各 ch の制約付き最適解の両方を出す。

### 6b.2 メトリック A — 低エネルギー効率 / 誤トリガ率

{smoothing, rise} ごと: 図 of merit = **trigger-filter S/N =（気にする最小エネルギーパルスの TTF 後
振幅）/ σ_TTF**。最大化 → smoothing 選定 → 閾値を `k·σ_TTF` に → 低E効率が従う。誤トリガ率 =
baseline 上の閾値交差/単位時間、**holdoff も再現**（決定的）。閾値いじりを ch ごとの計算最適点に変える。

### 6b.3 メトリック B — timing 分解能（パルサーを基準に）

Ge エミュレータパルサーは**トリガー出力**（エミュレーション信号より 200ns 先行するやつ）を既に持つ。
これを時刻基準に：

- エミュレーション信号 → Ge ch; パルサートリガー出力 → 参照 ch。
- オフライン: Ge ch に CFD をリプレイ、`Δt = t_CFD(Ge) − t(ref)` を per-event でヒストグラム
  → **σ_t = timing 分解能**。
- 定数 **200ns オフセットは無関係 — 効くのは spread（jitter）だけ**（jitter 測定で 200ns は相殺）。
- 注意: 測定 `σ = √(σ_Ge² + σ_ref² + σ_pulser²)`。参照 ch は高 S/N（きれいな fast トリガパルス）にし、
  パルサー jitter ≪ デジタイザ分解能を確認 → σ_ref / σ_pulser を無視/bound できるように。さもないと
  Ge ch でなく参照を測ることになる。

CFD {delay, fraction, smoothing} + 共有 smoothing をオフラインで掃く → timing 分解能の曲面。

### 6b.4 録り方の追加（Phase 0 for trigger）

- **ノイズラン** — random/software/pulser トリガ, 純 baseline → σ_TTF, 誤トリガ率。
  （§6b.0 の pre-trigger 区間解析で一部代替可能だが、専用ノイズランがあれば統計が締まる。）
- **ソースラン** — γ 源 → 実パルスの効率 + energy 分布（**§3 の capture run をそのまま流用** —
  低閾値 superset 化済みなので候補閾値の全域をカバー）。
- **timing ラン** — エミュレーション→Ge ch + パルサートリガー→ref ch, 両波形記録 → CFD jitter。

### 6b.5 validation（台形と同じ信頼パターン）

- SW トリガが **D0 = FW Trigger digital probe と同じサンプルで発火**（PHA1 では D0 固定 =
  全イベント無料で ground truth 付き）。
- SW CFD タイムスタンプが **FW の fine（CFD 内挿）タイムスタンプと per-event 一致** — FW は内挿
  タイムスタンプを既に出すので強い錨。

### 6b.6 おまけの診断価値

σ_TTF vs 期待 ENC で、**チューン可能な電子ノイズ（→ smoothing が効く）** か **外来ピックアップ/
グラウンドループ/マイクロフォニクス（→ パラメータでは直らない、まずハード）** かを切り分け。σ_TTF ≫ ENC
なら、チューンをやめてハードを追え。「閾値を下げるとノイズだらけ」の不満に直接答える — 直しがそもそも
パラメータ領域にあるのかを教えてくれる。

---

## 7. 他 ELIADE ブロックとの関係（本ファイル対象外）

- **`start_delay` 自動較正:** コモンクロック ⇒ ドリフトゼロ、定数オフセットのみ ⇒ 1ショット解析解:
  `StartDelay_b = TS_b − min_b(TS_b)`（クロック単位, 全て ≥ 0; 測った TS がチェーン順を教えるので
  配線順を知る必要なし）。`start_delay` は既に per-board config フィールド
  （`src/config/digitizer.rs:801`, 0–4080 ns, `/par/start_delay` 書込）。8 ホスト集約 coordinator が
  必要（全 40 ボードをまたぐグローバル）。
- **Topology 管理:** full vs subset は **topology-tag 付き較正テーブル**で、手維持の双子 config では
  ない。誤テーブル適用をガード。物理チェーンが保たれた subset は full テーブルを流用; start パスを
  変える subset（例: 上流チェーンなしの末尾 V1730）だけ独自較正が要る。
- **EB 時間オフセット:** **`start_delay` とは設計上分離**（HW 原点合わせ vs ソフト event-build
  オフセット）。PHA↔PSD 定数（トリガ点定義/フィルタ遅延の違い）は **first-run データから経験的に**
  導出 — これには **first run に共通アンカー**（例: パルサーのエミュレーション信号を PHA 1ch + PSD 1ch
  に fan-in）が要る。EB が合わせる基準を持つために。

---

## 8. 次の具体ステップ

- [ ] **Phase 0 capture 仕様の確定:** 最大 rise 決定 → 窓長（現行案 20 µs）+ pre-trigger; 源選定
      （⁶⁰Co/¹⁵²Eu）; per-channel の adc_min 逆算（coarse_gain/fine_gain 整列込み）。
- [ ] **dual trace の実効サンプリング確認**（interleave で 8 ns/sample 相当かを実機で確認）。
- [ ] **coarse_gain 見直し**: 個別 ch の ADC レンジ利用率を上げる（現状 ~12%、X4 側へ）。
      クリップ確認（sum peak 含む）。
- [ ] **validation run + capture run を各1本録る**（es2 で 1 結晶から始め、①〜④を一周通す）。
- [ ] **`pha_trap_tune` bin スケルトン:** `.delila`（波形 + FW energy + FW param）を読む
      （`dev-tools` feature、オフライン系）。
- [ ] **段分離 SW trap をスクラッチ実装**（§4.4）: 公知の再帰 + per-stage トレース。
      AMax コアは流用しない（存在しない）。
- [ ] **Phase 1 validation ハーネス:** per-event 残差 + probe2 重ね + D1=Peaking 窓照合 +
      FWHM_SW vs FWHM_FW。
- [ ] **Phase 2 grid search + free-peak Gaussian フィット** → per-channel config patch
      （深掘り 2ch → 狭域展開、§5.3）。
- [ ] **Phase 3 実機 verify** ループ + 確定。
- [ ] **（将来）reader に digital-probe 省略オプション**: EventData から digital probe を落として
      イベントサイズ半減（trigger 検証が終わった後の本番向け、数行 + config flag）。
- [ ] **Phase 5 capture:** ノイズラン（random トリガ）+ timing ラン（エミュレーション→Ge,
      パルサートリガー→ref）。
- [ ] **SW RCCR2**（RC-CR² + smoothing）+ LED/CFD 弁別, 段分離; D0=Trigger probe + FW CFD
      タイムスタンプで validate（台形の検証枠組みを横展開）。
- [ ] **メトリック A**（S/N → 最小閾値）+ **メトリック B**（CFD jitter vs パルサー基準）。
- [ ] **制約付き最適化:** timing ≤ window 予算 & 誤トリガ率 ≤ 予算 で閾値最小化;
      Pareto front + per-channel pick を出力。

---

## 参照

- `legacy/UM4380_725-730_DPP_PSD_Registers_rev6.pdf` — レジスタマップ; Start Delay step = 16/32 ns
- `src/reader/decoder/pha1.rs` — PHA1 波形デコード（probe_0=Input, probe_1=Trapezoid;
  D0=Trigger 固定 bit15 / D1 選択制 bit14; digital probe は u8/sample 非パック展開）
- `src/reader/decoder/psd1_pha1_common.rs:118` — `num_samples_wave: u16`（×8 = 上限 ~52万サンプル）
- `src/recorder/format.rs` — `.delila` v3（フィールド数固定・波形配列は可変長）
- `tools/delila2root/` — C++ 変換（vector ブランチ、固定サンプル数の仮定なし）
- `src/reader/decoder/amax.rs` — AMax FW 出力の **decoder**（SW trap 実装は含まない —
  2026-07-13 に流用方針を撤回）
- `src/config/digitizer.rs:801` — `start_delay` per-board config フィールド
- CLAUDE.md — 「Decoder hot-path heuristic policy」+「silent failure 禁止」ドクトリン（e641e99, e45e0ec）
