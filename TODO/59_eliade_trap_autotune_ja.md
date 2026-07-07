# TODO 59 — ELIADE PHA エネルギー分解能 自動チューン（SW 台形リプレイ）【日本語版】

**Status: 📋 PLANNING (2026-06-16)**
**担当:** Aogaki + Claude
**実験:** ELIADE（8× Clover HPGe アレイ, ELI-NP）
**タイムライン:** ベンチテスト 2026年6月開始 · Ge 分解能チューンアップ 2026年いっぱい · ビーム 2027年1月〜

> 英語版（同僚共有用）: [59_eliade_trap_autotune.md](59_eliade_trap_autotune.md)。内容は同一。

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

---

## 2. パイプライン全体像

```
Phase 0  録る（一度、実機で）   : Tuneup mode, probe1=Input, 波形ON + FW energy ON,
                                  既知 γ 源, 十分に長い波形窓。
Phase 1  SW-trap validation   : FW 自身の param で同一波形をリプレイし、FW energy と
                                  PER-EVENT で比較。これが信頼の錨。
Phase 2  オフライン最適化       : 同じ保存波形上で param を grid 掃き、ch ごとに
                                  FWHM(keV) 最小化。純 CPU。
Phase 3  実機 verify          : 最適 param を FW に焼き、短ランで実 FWHM が SW 予測と
                                  一致するか確認。config に確定。
```

Phase 1-2 は **1個のオフラインツール** に収まる: `.delila`（波形 + FW energy）を読み、per-channel
config patch を吐く新 bin `pha_trap_tune`。

---

## 3. Phase 0 — 録る（後から直せない制約）

確認済み（Aogaki）: デジタイザは Tuneup mode で **波形 + FW 計算 energy を同イベントで両方吐ける**。
**`analog_probe1 = Input`** に設定し probe1 に生 ADC preamp 信号を載せる（これが SW-trap 入力）。
**`analog_probe2 = Trapezoid`** で FW 自身の中間台形トレースが得られ、Phase 1 のサンプル単位照合に使う。

**後からオフラインで取り戻せない、録る時に決める制約:**

1. **波形窓長 ≥ テストする最長シェーピング。** rise を ~8 µs までテストするなら、窓は
   baseline + 完全な台形応答 ≈ `2·(rise+flat) + 減衰` を含む必要がある。250 MS/s（4 ns/sample）で
   8 µs rise は長い記録が要る。録る前に **テストする最大 rise** を決め、そこから窓長を決定。
   窓が短いと長シェーピングの eval が無効（応答が truncation）→ FWHM が silent に誤る。
2. **pre-trigger baseline を十分に。** baseline restorer は pre-pulse サンプルを平均するので、
   窓に十分入れる。
3. **ピーク統計。** 安定 Gaussian FWHM フィットには **ピーク内 ~5k–10k カウント/ch**。
   源: ⁶⁰Co（1173/1332 keV）or ¹⁵²Eu（多線）。十分なイベントを録る。

出力: channel-group ごとに、`{生入力波形, FW energy, FW 使用 param, 台形 probe}` を per-event で
持つ `.delila` ファイル1個。

---

## 4. Phase 1 — SW-trap validation（厳密にやるべき部分）

### 4.1 流用方針

AMax カスタム FW 自体が台形フィルタ MCA で、その trap ロジックは手元にある。開発者が
**AMax-FW energy ↔ PHA-FW energy が線型に 1:1** であることを確認済み。なので **AMax 台形コアを
SW-trap の骨格として流用**する。

⚠️ **「線型 1:1 energy 対応」は分解能の根拠としては不十分。** 線型対応は2つのブラックボックス FW の
*centroid/gain* が合うことしか意味しない。**FWHM はフィルタのノイズ伝達特性**（台形の重み、
baseline restorer、rise/flat-top の正確なサンプル数、固定小数の丸め、energy 抽出窓）で決まる。
centroid が線型に相関しても FWHM は違いうる。AMax と PHA の **gain 定数倍は無視して安全**（分解能は
相対量、毎 eval ピークを再フィットして keV 再較正する）。しかし周辺の段は無視できない。

### 4.2 validation 基準

捕捉波形を **FW が使った正確な param** で SW trap に通し:

- per-event 残差 `SW_energy − FW_energy`; その std が **≪ ピーク FWHM（理想は ±1 LSB）** を要求。
  **線型フィットの R² ではなく per-event 残差を使う。** per-event 一致が FWHM 再現を保証する; R² は
  しない。
- **SW 台形トレース vs デコード済み `analog_probe2`** をサンプル単位で重ねる（FW が中間台形を出すので
  強い ground truth）。
- 決定的チェック（FW 動作点）: **`FWHM_SW ≈ FWHM_FW`**。centroid ではなく FWHM が一致して初めて
  最適化に使える。

これはまさに CLAUDE.md ドクトリン（「spec ページ参照 + 実機検証; 分解能を silent に間違えない」—
e641e99 ヒューリスティック、e45e0ec silent-cache 事案）。**Phase 1 が通るまで最適化に進まない。**

### 4.3 台形数式の錨（Jordanov-Knoll; UM4380 + AMax 実装と一致）

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

### 4.4 モジュール構造 — *これが「(ii)」の実体*

「(ii)」は **重い API 設計ではなく**、**別トピックでもない**。単に **「Phase 1 がコケた時にどの段が
ズレてるか局所化できるよう、SW trap を分離可能で覗ける段に切る」** だけ。モノリシックに書くと
デバッグ地獄。PHA energy 処理は次のように段分解できる:

| 段 | 内容 | FWHM に効く? | 流用方針 |
|----|------|:---:|----------|
| 1 | Input（probe1=Input 生波形） | — | そのまま |
| 2 | Baseline 計算/restorer（pre-pulse 平均を減算） | **効く** | AMax にあるが **725 PHA と要確認** |
| 3 | 台形 recursion（d,p,r,s + PZ M） | 効く（シェーピング） | **AMax コア流用 ✓** |
| 4 | Energy 抽出（peaking 位置で取得、Npk 平均） | **効く**（ノイズ平均） | AMax にあるが **725 PHA と要確認** |
| 5 | gain 正規化 → LSB | — | **無視（線型倍, Aogaki 正しい）** |

FWHM を決めるのは台形そのものより **段 2・4（baseline + energy 窓）**。AMax の段 2/4 が 725 PHA と
一致するなら、AMax trap を丸ごと突っ込んで Phase 1 が一発で通る — ベストケースでは「(ii)」は ~ゼロ工数。
違う場合（baseline ドループ、peak 位置）、段分解のおかげで該当段だけ特定して直せる
（「台形は probe2 と一致、でも energy が合わない → 段 4 の窓だ」）。

**実務順:** AMax trap を丸ごと突っ込む → Phase 1 → 通れば即 Phase 2、ダメなら問題の1段だけ修正。
唯一の「設計」要件は per-stage 中間トレースを覗けるようにしておくこと。

---

## 5. Phase 2 — オフライン最適化（grid で十分）

### 5.1 物理 prior で探索空間を削る

- **Pole-zero M:** 波形から測定（§4.3）→ 固定、探索しない。
- **Trigger threshold:** 強いピークの分解能にはほぼ効かない → 適切に固定。
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
- **BO 不要**（元の動機=オンライン acquisition コストが消えた）。何かの理由で次元が高いままなら
  その時だけ検討。

### 5.3 FWHM メトリック — 「ピーク移動」の罠を避ける

rise を変えると **gain が変わり、peak centroid が移動**する。フィッタは毎 eval **ピークを自由に
発見**（Gaussian + 線形背景）し、centroid を既知エネルギー（例 1332 keV）に対応させ、**keV の FWHM**
を報告すること。*固定* ch 窓で FWHM を最小化すると間違える。（内側ループは robust な half-max 幅推定で
よい; 最終報告は full Gaussian フィット。）

### 5.4 出力

per channel: 最適 `{rise, flat-top, peaking, PZ M}` + FWHM カーブ（同僚が U 字を sanity-check 用）、
既存の `start_delay` 方式の per-channel config パスで適用できる **config patch** として吐く。

---

## 6. Phase 3 — 実機 verify

最適 param を FW に焼き、短ランで **実** FWHM を測り、SW 予測と一致を確認。一致 → config 確定。
不一致 → SW モデルのギャップ; 段分解（§4.4）がどこを見るか教える。これで spec-ref + 実機検証ループを閉じる。

---

## 6b. Phase 5 — Trigger & Timing 最適化（効率 AND timing 両方）

**同じオフラインリプレイ harness を流用**。トリガ経路（TTF = RC-CR² + smoothing → LED/CFD 弁別）は
台形と同じく入力波形の決定的関数。config 配管は一部既存（V1743 作業の `ttf_smoothing`,
`trigger_edge`, `trigger_threshold_v`）。

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

- **ノイズラン** — random/software/pulser トリガ, 純 baseline → σ_TTF, 誤トリガ率。これがないと誤
  トリガが測れない（ソーストリガ波形は選択バイアス — 低閾値が生むノイズイベントが入ってない）。
- **ソースラン** — γ 源 → 実パルスの効率 + energy 分布（エネルギーチューンの録りを流用）。
- **timing ラン** — エミュレーション→Ge ch + パルサートリガー→ref ch, 両波形記録 → CFD jitter。

### 6b.5 validation（台形と同じ信頼パターン）

- SW トリガが **FW Trigger digital probe と同じサンプルで発火**。
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

- [ ] **Phase 0 capture 仕様:** テストする最大 rise を決定 → 窓長 + pre-trigger; 源選定（⁶⁰Co/¹⁵²Eu）;
      カウント/ピーク設定。validation 用 `.delila` を1本録る。
- [ ] **`pha_trap_tune` bin スケルトン:** `.delila`（波形 + FW energy + FW param）を読む。
- [ ] **段分離 SW trap**（§4.4）: AMax コアを投入、per-stage トレースを出せるように。
- [ ] **Phase 1 validation ハーネス:** per-event 残差 + probe2 重ね + FWHM_SW vs FWHM_FW。
- [ ] **Phase 2 grid search + free-peak Gaussian フィット** → per-channel config patch。
- [ ] **Phase 3 実機 verify** ループ + 確定。
- [ ] **Phase 5 capture:** ノイズラン（random トリガ）+ timing ラン（エミュレーション→Ge,
      パルサートリガー→ref）。
- [ ] **SW TTF**（RC-CR² + smoothing）+ LED/CFD 弁別, 段分離; FW Trigger probe + FW CFD
      タイムスタンプで validate。
- [ ] **メトリック A**（S/N → 最小閾値）+ **メトリック B**（CFD jitter vs パルサー基準）。
- [ ] **制約付き最適化:** timing ≤ window 予算 & 誤トリガ率 ≤ 予算 で閾値最小化;
      Pareto front + per-channel pick を出力。

---

## 参照

- `legacy/UM4380_725-730_DPP_PSD_Registers_rev6.pdf` — レジスタマップ; Start Delay step = 16/32 ns
- `src/reader/decoder/amax.rs` — AMax 台形 MCA（SW trap コアの流用元）
- `src/reader/decoder/pha1.rs` — PHA1 波形デコード（`analog_probe1` = Input, `analog_probe2` = Trapezoid）
- `src/config/digitizer.rs:801` — `start_delay` per-board config フィールド
- CLAUDE.md — 「Decoder hot-path heuristic policy」+「silent failure 禁止」ドクトリン（e641e99, e45e0ec）
