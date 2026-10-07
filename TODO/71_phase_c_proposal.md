# TODO 71 Phase C 提案 — タグ付きチャンネル表・AC の印・L2

2026-10-07 調査。ELIFANT-Event は `a74bf8d`(パスは `/Volumes/Ce140/ELIFANT-Event` からの相対)、root_sink は c121318(Phase A)が対象。

## 1. ELIFANT-Event のやり方

### 1.1 チャンネル設定(`JSON/chSettings.json`)

- 形式は `[module][channel]` の入れ子配列で、1 ch = 1 オブジェクト。キーは 20 個で、省略できない(中身は `include/ChSettings.hpp:27-52`、読み込みは `:127-172`)。
- 雛形は `-i` で作る(`:88-125`)。`ACModule`/`ACChannel` の既定は 128(= 無し)。
- 各キーと、どこで使われるか:

| キー | 意味 | 使う場所 |
|---|---|---|
| `Module`, `Channel` | 位置(引くときは配列の添字を使う) | — |
| `IsEventTrigger` | トリガーにする | L1 `src/L1EventBuilder.cpp:231-232`、時刻合わせ `src/TimeAlignment.cpp:308` |
| `ID` | 整数。**小さいほどトリガーとして優先**。時刻合わせのヒストグラムの行番号も兼ねる | L1 `:238, 255-257, 276-278`、`TimeAlignment.cpp:326, 439` |
| `ThresholdADC` | `ChargeLong` がこれ以下のヒットを、読み込み時に捨てる | L1 `:459`、`TimeAlignment.cpp:283` |
| `HasAC`, `ACModule`, `ACChannel` | AC の相手(1 ch に 1 つだけ) | L1 `:307-309` |
| `Tags` | 文字列の配列 | L2 の Counter だけ(`src/L2EventBuilder.cpp:80-95`) |
| `DetectorType` | "AC" / "PMT" / "HPGe" / "Si" | 時刻合わせのリビンだけ(`TimeAlignment.cpp:442-449`) |
| `p0`〜`p3` | 3 次のエネルギー較正 | **EB では使わない**。解析マクロで使う(`macro/reader.cpp:40-45`) |
| `Phi`, `Theta`, `Distance`, `x`, `y`, `z` | 幾何 | どこでも使わない |

p91Zr の実例(m10 c0 の HPGe。整形だけ詰めた):

```json
{"ACChannel": 11, "ACModule": 8, "Channel": 0, "DetectorType": "HPGe", "Distance": 0.0,
 "HasAC": true, "ID": 192, "IsEventTrigger": true, "Module": 10, "Phi": 0.0,
 "Tags": ["HPGe"], "Theta": 0.0, "ThresholdADC": 0,
 "p0": 159.757, "p1": 0.784285, "p2": 1.22218e-05, "p3": 0.0, "x": 0.0, "y": 0.0, "z": 0.0}
```

- **p91Zr の構成**(11 モジュール、208 ch)

| モジュール | 中身 | タグ | トリガー | ID | AC |
|---|---|---|---|---|---|
| m0 | Si E セクター 16 ch | `E_Sector` | ○ | 0-15 | — |
| m4 | Si ΔE セクター 16 ch | `dE_Sector` | ○ | 64-79 | — |
| m1-3, m5-7 | Si リング | 無し | — | | — |
| m8 | AC 32 ch | 無し | — | | — |
| m9 | PMT 32 ch | `PMT` | ○ | 160-191 | 14 ch に付く |
| m10 | HPGe 16 ch | `HPGe` | ○ | 192-207 | 10 ch に付く |

- AC は 1 対 1 で付く。例: m10 c0-3 → m8 c11-14、m10 c4-9 → m8 c19-24。
- `ThresholdADC` は 208 ch 中 207 ch が 0 で、m9 c18 だけが 20 → 実際にはほとんど使われていない。
- (メモ)ID 順では PMT(160-)が HPGe(192-)より優先される。TODO 71 §3 の例「Si E > Si ΔE > HPGe > PMT」とは、HPGe と PMT の順が逆。
- 窓は `settings.json` の `CoincidenceWindow`(p91Zr は 100 ns、`JSON/settings.json:3`)。トリガーを中心に ±W で、境界を含む(L1 `:250, 271`)。`TimeWindow`(1000)は、時刻合わせのヒストグラムの幅にしか使わない(`TimeAlignment.cpp:88-92`)。
- 表に無い (mod, ch) のヒットは、黙って捨てられる(L1 `:449-457`)。

### 1.2 AC の印(`src/L1EventBuilder.cpp:297-318`)

- イベントを作った後で、そのイベント内の全ヒットを見る。ch が `HasAC` で、**同じイベントの中**に (`ACModule`, `ACChannel`) のヒットがあり、`|t_AC − t_トリガー| < W` なら、`isWithAC = true` にする。
- 時間条件は**トリガー時刻からの距離**で測る(印を付けるヒットからの距離ではない)。
  - イベント内のヒットはもともと |t| ≤ W なので、実質は「相手が同じイベントにいる」という条件になる(ちょうど ±W の相手だけは、`<` なので外れる)。
  - W = 100 ns の例: HPGe が +90 ns、AC が −90 ns(180 ns 離れている)でも印が付く。HPGe が +90 ns、AC が +110 ns(差は 20 ns)だと、AC がイベントの外なので印が付かない。
- 何も捨てず、印を付けるだけ(`RawData_t::isWithAC`、`include/EventData.hpp:36`)。AC のヒット自身もイベントに残る。印の付いたヒットは解析マクロが飛ばす(`macro/reader.cpp:229-232`)。
- ELIFANT はヒットを複数のイベントに重複して入れる。そのため、相手が「隣のイベントに取られる」ことは起きない。root_sink は規則 4 でヒットを重複させないので、ここが違う(§2.4)。

### 1.3 L2(`JSON/L2Settings.json`、`include/L2Conditions.hpp`、`src/L2EventBuilder.cpp`)

- 形式: `Type` が `Counter` / `Flag` / `Accept` のどれかであるオブジェクトを、平らな配列に並べる。p91Zr(`JSON/L2Settings.json:1-38`)は Counter 2 つ、Flag 2 つ、Accept 1 つ:

```json
{"Type": "Counter", "Name": "E_Sector_Counter", "Tags": ["E_Sector"]},
{"Type": "Flag", "Name": "E_More_Than_0", "Monitor": "E_Sector_Counter", "Operator": ">", "Value": 0},
{"Type": "Accept", "Name": "Si_Both", "Monitor": ["E_More_Than_0", "dE_More_Than_0"], "Operator": "AND"}
```

- **Counter:**
  - 挙げたタグのどれか 1 つでも持つ ch(タグは OR)のヒットを数える。ch ごとの真偽表を前もって作る(`L2EventBuilder.cpp:80-95`、`L2Conditions.hpp:25-32`)。
  - ヒットの数を数えるので、同じ ch が 2 回鳴れば 2 になる。
  - AC の印は見ない。指定はタグだけ(ch を直接は書けない)。
- **Flag:** 1 つの Counter を整数と比べる。演算子は `== != < <= > >=`(`L2Conditions.hpp:58-83`)。知らない演算子は stderr に出して false にする。**知らない Counter 名は、何も出さずに false**。
- **Accept:** Flag を `AND` か `OR` で組み合わせる(`L2Conditions.hpp:100-142`)。Accept が複数あれば、その OR をとる(`L2EventBuilder.cpp:341-344`)。NOT や入れ子は無い(`== 0` などで代用する)。
- **出力:** Flag は `名前/O`、Counter は `名前/l` のブランチになる。名前は設定に書いたもの(`L2EventBuilder.cpp:278-285`)。Accept の結果はブランチにならない。解析マクロは名前でブランチを読む(`macro/reader.cpp:201-204`)。
- **採らなかったイベントは書かない**(`:349-351`)。L1 のファイルは残るので、データがすべて消えるわけではない。
  - Accept が 1 つも無いと、1 件も書かれない(既定で全部を通す行がコメントアウトされている、`:346-348`)。ヒットの無いイベントも飛ばす(`:321-323`)。
- 知らない `Type` は、stderr に出して先へ進む(`:133-135`)。

### 1.4 ELIFANT の L1/L2 にあって Phase A に無いもの(ELIADE に関係するもの)

- **AC の印と L2** → Phase C の本体。
- **`ThresholdADC`**(読み込み時の閾値)→ 採らない(SPEC v0.7)。p91Zr でも実質は使われていない。
- **`ID` による優先** → Phase A の `priority` で置き換え済み。
- **エネルギー較正 `p0`〜`p3`** → ELIFANT も EB では使わず、マクロで使う。ELIADE の addback や Ge-Ge のゲートには要る → 再構成の層で扱う。
- **幾何**(Phi など)→ ELIFANT では未使用。ELIADE の角度相関やドップラー補正で要るが、解析側の仕事。
- **ELIFANT にも無いが、ELIADE で要りそうなもの:**
  - **addback**(クローバー内の和)
  - **pileup / saturation のフラグ**(root_sink は `EventData.flags` を読み飛ばしている、`tools/root_sink/sink_core.hpp:124-127`)
  - **コアの 2 ゲイン**(SN01 では ch0 と ch1 が同じコアらしい)
  - **複数ホスト**(Phase D)

## 2. root_sink への提案

### 2.1 設定 JSON の拡張(event_def に足す。1 ファイルのまま、`version` は "1.1")

1. **`tags`(チャンネル表):** タグ名 → ch の一覧。
   - `[m, c]` のほかに、`[m, 最初, 最後]`(範囲、両端を含む)も書ける。
   - ELIFANT の「ch ごとの `Tags`」と同じ情報を、逆向きに書いたもの。ELIADE は 1 ホストで 40 ch を超えるので、ch ごとに書くと長くなる。
   - 解決した「ch → タグ」の表は provenance に出す。
2. **トリガー:** `channels` に加えて `tags` も書ける(両方の和集合をとる)。「1 つの ch は 1 つのトリガーにだけ属する」という検査はそのまま。
3. **`anti_compton`:**
   - `window_ns {pre, post}` は、**印を付けるヒットからの相対時刻**。
   - `pairs` は `{name, detector: [タグ], shield: [タグ]}` の配列。多対多で書けるので、4 結晶 × 複数 BGO を 1 行で書ける。
   - 同じペアの detector と shield に同じ ch があればエラー。
4. **`l2`:** `counters` / `flags` / `accept` の 3 つの配列(ELIFANT の平らな配列を、種類ごとに分けただけ)。
   - counter: `{name, tags, ac: "any"(既定) | "clean" | "vetoed"}`。
     - 数えるのは**ヒットの数**(ELIFANT と同じ)。
     - `ac` は、ELIADE で「AC の印が無い Ge の数」を数えるために足す。
   - flag: `{name, counter, op, value}`。op は ELIFANT と同じ 6 つ。
   - accept: `{name, op: "AND" | "OR", flags: [...]}`。全体の採否は、accept の OR(ELIFANT と同じ)。
5. **検査:** Phase A と同じく、**全部のエラーをまとめて**出す。
   - 対象: 知らないタグ・counter・flag を参照している、名前の重複、ブランチ名に使えない名前、知らない op、空のリスト、counter やペアが 64 個を超える(ビット表を使うため)。
   - ELIFANT で黙って false になっていたものも、すべて読み込み時のエラーにする。
   - `_comment` は、どのオブジェクトにも書ける。
   - 古いバイナリに 1.1 のファイルを渡すと、「知らないキー」で読み込みエラーになる → STALE(黙って無視はされない)。
6. **読み直し・STALE・provenance:** Phase A の仕組みのまま(UserInfo `eventDef`、`_eventDef.json`)。provenance には、解決後のタグ表・AC のペア・L2 を足す。
7. **データ構造(案):**
   - `EventDef` に、タグ名、ペアごとの detector / shield のビット表(mc_key → uint64)、counter ごとの ch 表を足す。
   - `EventHit` に `with_ac` を足す。
   - `GenericEvent` に、counter の値・flag のビット・accept のビットを足す。
   - `GenericStats` に、AC の印が付いた数、flag / accept が true の数を足す。

### 2.2 例

**ELIADE 風**(数値は仮)の構成:
- 2 クローバー。m0-3 = クローバー 1 の結晶 A-D、m4-7 = クローバー 2。
- 各 V1725 は、ch0 = コア(高ゲイン)、ch1 = コア(低ゲイン)、ch2-9 = セグメント。
- m8 = V1730。ch0-3 が BGO1、ch4-7 が BGO2。

```json
{"version": "1.1",
 "window_ns": {"pre": 2000, "post": 2000},
 "group_ns": 2000,
 "tags": {
   "C1":   [[0, 0, 9], [1, 0, 9], [2, 0, 9], [3, 0, 9]],
   "C2":   [[4, 0, 9], [5, 0, 9], [6, 0, 9], [7, 0, 9]],
   "core": [[0, 0], [1, 0], [2, 0], [3, 0], [4, 0], [5, 0], [6, 0], [7, 0]],
   "seg":  [[0, 2, 9], [1, 2, 9], [2, 2, 9], [3, 2, 9], [4, 2, 9], [5, 2, 9], [6, 2, 9], [7, 2, 9]],
   "BGO1": [[8, 0, 3]],
   "BGO2": [[8, 4, 7]]},
 "triggers": [{"name": "Ge", "tags": ["core"]}],
 "anti_compton": {
   "window_ns": {"pre": 300, "post": 300},
   "pairs": [{"name": "C1", "detector": ["C1"], "shield": ["BGO1"]},
             {"name": "C2", "detector": ["C2"], "shield": ["BGO2"]}]},
 "l2": {
   "counters": [{"name": "nGe",      "tags": ["core"]},
                {"name": "nGeClean", "tags": ["core"], "ac": "clean"},
                {"name": "nSeg",     "tags": ["seg"]},
                {"name": "nBGO",     "tags": ["BGO1", "BGO2"]}],
   "flags":    [{"name": "GeGe", "counter": "nGeClean", "op": ">=", "value": 2}],
   "accept":   [{"name": "GammaGamma", "op": "AND", "flags": ["GeGe"]}]}}
```

- コアをトリガーにすると、クローバー内で散乱した γ(addback の対象)では 2 つのトリガーが 1 つの束にまとまり、1 イベントになる(規則 1)。
- 優先順位を書いていないので、束の最初のコアが基準になる。

**ThGEM:** 今の `config/event_def_thgem.json` を、**一字も変えずに**読める(`tags`・`anti_compton`・`l2` はどれも省略できる)。出力は Phase A と同じで、`HitWithAC`(全部 false)が増えるだけ。L2 を使う場合は、たとえば次のように書く:

```json
"tags": {"arms1": [[0, 2, 5]]},
"l2": {"counters": [{"name": "nArms1", "tags": ["arms1"]}],
       "flags":    [{"name": "Full1", "counter": "nArms1", "op": "==", "value": 4}],
       "accept":   [{"name": "Full", "op": "OR", "flags": ["Full1"]}]}
```

### 2.3 出力ブランチ(辞書不要)

- **ヒット単位:** `HitWithAC[NHits]/O` を**常に**出す(AC を定義していない設定では、全部 false)。設定によって木の形が変わらないので、TChain やマクロが壊れない。
- **L2:** 設定に書いた名前を、そのままブランチ名にする(ELIFANT と同じ。`events->Draw("HitEnergy", "GeGe")` と書ける)。
  - counter は `名前/i`、flag は `名前/O`、accept は `名前/O`。全体の採否は `L2Accept/O`(accept が 1 つ以上あるとき)。
  - 名前は C の識別子に限る。固定のブランチ名(`EventID`, `TriggerTime`, …, `L2Accept`)とぶつかったらエラー。
  - 名前付きのブランチは**ランごとに固定**される(定義はラン開始時に凍結する)。1 ラン 1 ファイルなので問題は無い。定義の違うランを TChain でつなぐときだけ注意が要る(provenance を見れば分かる)。
  - 別の案: 固定名の配列(`L2Counters[NL2Counters]/i` + 名前は UserInfo に置く)。木の形は固定になるが、`L2Counters[2]` と書くのは読みにくく、設定を変えると番号がずれる → 推奨しない。
- **ランのまとめと状態行:** AC の印が付いたヒットの数(ペアごと)と、flag / accept ごとの true のイベント数を出す。

### 2.4 どこで動くか、AC の窓の意味

- **Sorter:** 変更は無い。持ち越す範囲を広げるだけ(下記)。
- **Worker:** どちらも、チャンクと def だけで決まる純関数 → Worker の数に依存しない。総当たりの参照実装も、そのまま広げられる。
  1. **AC:** 振り分け(規則 4)の**前に**、チャンクの時刻順のヒット全体で計算する。
     - ペアごとに、shield のヒットの時刻の列を作る。
     - detector の各ヒットについて、`[t − pre_ac, t + post_ac]` に shield のヒットがあるかを二分探索で調べ、結果を `EventHit.with_ac` に入れる。
  2. **L2:** `build_generic_events` の後で、イベントごとに `apply_l2(def, GenericEvent&)` を呼ぶ。counters → flags → accept の順に、イベントのヒットだけを見て決める(GenericEvent の純関数)。
- **AC の窓は、印を付けるヒット自身からの相対時刻で測り、イベントとは独立に決める**(推奨)。
  - 相手の BGO ヒットが、規則 4 で**隣のイベントに入っても、どのイベントにも入らなくても**、印は付く。
  - 理由 1: 物理的な意味(結晶と BGO のプロンプトな同時計数)にそのまま一致する。
  - 理由 2: 結果がトリガーの設定や窓の位置に左右されない。ELIFANT 式だと、窓の端にある Ge は相手を取り逃がし、180 ns 離れた組にも印が付く(§1.2)。
  - 理由 3: 規則 4(ヒットを重複させない)と両立する。
  - ELIFANT 式(「同じイベント内に相手がいる」)を規則 4 の下で使うと、隣のイベントに BGO を取られた Ge が素通りしてしまう → 採らない。
- **チャンクの約束の変更:** 窓の端にあるヒットの AC を決めるには、さらに外側の `pre_ac` / `post_ac` までのヒットが要る。
  - `ev_hi = D − pre − post − post_ac`(上側は、揃ったヒットだけを使う)
  - `lookback_ns() = G + 2·pre + post + pre_ac + post_ac`
  - AC の無い設定では、今と同じ値になる。上の ELIADE の例では 8.6 µs で、safe horizon(50 ms)より十分小さい。

### 2.5 L2 はイベントを捨てるか、印だけか

- **推奨: 印だけ。** すべてのイベントを書き、`L2Accept` などのブランチで選ぶ(`events->Draw(..., "L2Accept")`)。
- 理由:
  - 「取りこぼしは無い方が断然よい」。
  - ニアオンラインモニタとして、条件を後から変えて見直せる。
  - Phase A の検査(イベント内のヒット + 窓外のヒット = core のヒット数)が、そのまま成り立つ。
  - ELIADE の初期は低レートなので、容量は問題にならない。
- 代償: Accept の割合が小さく、レートが高い場合(例: シングルが Ge-Ge の 100 倍)には、ファイルが大きくなる。
  - 必要になったら、捨てずに「採ったイベントだけの 2 本目の木」を足す(元の木は全部のまま)。
  - 捨てる方式も、捨てた数を数えて検査に 3 つ目の項を足せば作れる。ただし今は作らない。

### 2.6 テスト計画

**単体テスト**(既存の 2 本に足す):
- `test_sink_core`(設定):
  - タグの範囲指定と解決、トリガーの `tags`。
  - エラー: 知らないタグ・counter・flag、名前の重複、固定名との衝突、識別子でない名前、同じペアの detector と shield の重なり、負の AC 窓、op の誤り、空の accept。
  - 1.0 のファイルがそのまま読めること、lookback に AC が入ること、provenance。
- `test_eb_core`(AC):
  - 相手がちょうど窓の端にいる(両端を含む)。
  - 相手が隣のイベントにいる / どのイベントにもいない。
  - 相手が持ち越しの範囲にしかいない(チャンク境界)、相手が core_end のすぐ手前にいる。
  - shield 自身には印が付かない。1 つの ch が 2 つのペアに入っている。
- `test_eb_core`(L2): 同じ ch が 2 回鳴れば 2 と数える、`ac` の 3 種類、6 つの op、AND / OR、accept の OR、L2 の無い設定。
- **総当たりの参照実装**に AC と L2 を足し、ランダムな 200 通りで一致させる。わざとバグを入れて捕まるか確かめる(post_ac 分の ev_hi の削り忘れ、lookback の不足、「同じイベント内だけ」で探す実装)。

**E2E:**
- **ThGEM run 56 / run0023 の再生:** 今の設定のまま、events が Phase A の出力と一致すること(`HitWithAC` を除く)。
- **`check_events.C` を広げる:** hits ツリーから AC と L2 を作り直して比べる(配備後の確認にも使う)。
- **SN01 V1725(パルサー):**
  - パルサーを 3 ch 以上に分配したランで、1 ch を仮の「shield」に指定する → detector の印が 100 %、shield の ch を外せば 0 % になること。
  - counter の値が、いつも分配した ch の数になること。
  - **要るデータ:** 分配したパルサーのラン(数分)。
- **SN01 の実 HPGe(2026-10-01 の ⁶⁰Co、ch0-9):** コアをトリガー、ch2-9 を `seg` にして、`nSeg` の分布を見る(コードの確認用。AC は無い)。
- **AC の物理的な検証:** クローバーと BGO(V1730)を同時に取ったラン(⁶⁰Co か ¹³⁷Cs)が要るが、まだ無い。確認すること:
  - AC で抑えたスペクトルで peak-to-total が良くなる。
  - 印の付く割合。
  - Ge−BGO の Δt 分布(AC の窓を決めるのにも使う)。

### 2.7 ユーザーへの質問

1. チャンネル表は「タグ → ch の一覧」(提案、短く書ける)でよいか。それとも ELIFANT と同じ「ch ごとに tags」にするか。
2. AC の窓は「印を付けるヒットからの相対時刻で、イベントとは独立」(提案)でよいか。
3. ELIADE の BGO は、クローバーごとに何 ch あり、どのボードに入るか(クローバー単位で共有か、結晶ごとか)。
4. AC の印は、セグメントにも付けるか、コアだけか(`detector` のタグで選べる。例の既定をどちらにするか)。
5. AC の窓の初期値は、Ge−BGO の Δt 分布を見てから決める、でよいか。
6. counter は、ヒットの数(ELIFANT と同じ、提案)か、鳴った ch の数か。
7. L2 は印だけで、イベントは捨てない(提案)、でよいか。
8. L2 のブランチ名は、設定の名前そのまま(提案)か、頭に `L2_` を付けるか。
9. ELIFANT の `ThresholdADC` に当たるもの(数えるときだけのエネルギーの下限で、捨てはしない)を counter に付けたいか(提案: 付けない)。
10. コアの 2 ゲイン(ch0 / ch1)は、どちらか片方だけをトリガーにする、でよいか(両方にすると、毎回 NTriggers = 2 になる)。
11. エネルギー較正(ELIFANT の p0〜p3)と pileup のフラグは、Phase C の外(再構成の層、または別の TODO)でよいか。
