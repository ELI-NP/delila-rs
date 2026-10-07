# TODO 70 — root_sink に (module, channel) 単位の時刻オフセット

**Status: ✅ Phase 1 実装・pf-qaq 配備済み (2026-10-07) — 残 = 検出器が動いているランで XY 像の中心を実機確認 / Phase 2(§7)は未着手**

**関連:** [event-builder/SPECIFICATION.md](event-builder/SPECIFICATION.md) §4.3(timeSettings.json) /
[event-builder/66_cpp_event_builder.md](event-builder/66_cpp_event_builder.md)(C++ EB = root_sink 系譜) /
[69_write_failure_autostop.md](69_write_failure_autostop.md) /
`src/event_builder/time_offsets.rs`(Rust 側の実装) / `docs/plans/rollover_tracker_design.md`(レイヤ原則)

---

## 1. 発端 (2026-10-06)

ThGEM のセットアップを pf-qaq(実験場、192.168.144.142)へ移したら、root_sink の XY 像が中心からずれた。
side3 run 45 / 47 と pf-qaq run 56 を、同じ方法でオフラインで再構成して比べた
(トリガーごとに ±1000 ns 内で最も近い XL/XR/YU/YD を拾い、1/99 % 点の中点を取る)。

| ThGEM1 | side3 run 45/47 | pf-qaq run 56 |
|---|---|---|
| 像の中心 (x, y) | (+0.5..+1.1, +1.6..+2.5) ns | **(−17.5, −29.8) ns** |
| 像の幅 | ≈ 411 × 414 ns | ≈ 421 × 398 ns(ほぼ同じ → 平行移動のみ) |
| 腕 − トリガー XL / XR / Y下 / Y上 | 88 / 88 / 89 / 90 ns | **141 / 123 / 146 / 119 ns** |

- XL は XR より約 18 ns、Y下は Y上より約 27〜31 ns 遅れて届いている。同軸ケーブル(約 5 ns/m)で約 3.5 m と約 6 m に相当する。
- ケーブル長が変わったと判断した。ThGEM2 の中心は (−2.5, −15) ns(side3 では一度も動いていなかった)。

**ユーザー判断:** ソフトでオフセットを補正する。理由は、**マルチデジタイザ環境でボード間の相関を見るのに必ず必要**になるから。

**確定した方針(2026-10-06 ユーザー回答):**
- **今回はオフセットだけ(Phase 1)**。`M:C` 形式で対象を指定する改修とハードウェアのクロック同期は、Phase 2(§7)として記述のみ。
- **`.root` の hits ツリーには補正後の時刻**を書く。`.delila` は常に生の時刻。使ったオフセットは `.root` 内と sidecar に記録する。

## 2. 形式と約束事(既存の Rust EB に合わせる)

- **木構造の形式 `timeSettings.json`**(`src/event_builder/time_offsets.rs`、`schemas/timeSettings.schema.json`):
  ```json
  {"version":"1.0","entries":[{"module":0,"channel":1,"ref":null,"offset_ns":0.0},
                              {"module":0,"channel":2,"ref":[0,1],"offset_ns":8.7}]}
  ```
  絶対オフセット = root からその ch までの経路上の `offset_ns` の合計(**root 自身の値も足す**。Rust テストの `root_offset_is_applied` と同じ)。
- **旧形式**(`event_builder time-calib` の出力、`src/event_builder/config.rs:126-153`):
  `{"ref_module":0,"ref_channel":0,"offsets":{"00_02":8.7}}`。これも受け付ける。
  - 形式は、`entries` があれば木構造、`offsets` があれば旧形式と決める。両方ある・どちらも無いは error。
  - Rust のように「木構造で読めなければ黙って旧形式を試す」ことはしない。木構造の本当のエラーが隠れるため。
- **符号: `aligned_ts = raw_ts − offset_ns`(正 = その ch は遅れて届く)**。
  - コードで確認済み: `hit.rs:92`、`slice_builder.rs:103`、`pipeline.rs:341`、`time_calibrator.rs` のテスト「Ch1 +15 ns late → offset +15」。
  - **SPECIFICATION.md:269-270 の符号の説明はコードと逆**なので直す(本 TODO の作業に含める)。`hit.rs:67` という参照も古い(現在は `:92`)。
- キーは `(module<<8)|channel`(Rust の `make_channel_key`、`hit.rs:98` と同じ)。
- **レイヤ:** 時刻補正は decoder 層では行わない(`rollover_tracker_design.md` §「レイヤ責任の原則」)。
  - root_sink は Rust EB と同じく「入口で一度だけ補正し、ラン中は固定」(SPEC §4.3.1)。
  - ケーブル遅延は**データの時間軸の補正**で、mm 較正や −70 ns の TOF 表示オフセットのような**解析定数とは別物**。`eb_core.hpp:388-391` のコメントにこの区別を明記する。
  - トリガーや LaBr3 の ch のオフセットを動かすと TOF が変わるので、マクロ側の定数は再導出が必要になる。

## 3. 実装

### 3.1 新ヘッダ `tools/root_sink/time_offsets.hpp`(ROOT にも ZMQ にも依存しない)
- `mc_key(m, c)`、`OffsetTable`(解決済みの絶対オフセットの map、形式、root 数、最大深さ、警告、`spread_ns()`)。
- `parse_time_offsets(text)`: **全エラーを集めて**返す。
  - **木構造で error:**
    - 重複 (m, c)(両方の位置を報告)
    - ref の参照先が無い
    - 循環・自己参照
    - module/channel が 0..255 の範囲外、または整数でない
    - `ref` が 2 要素の配列でない
    - `offset_ns` が有限の数でない
    - `ref` キーが無い、未知のキー(`"Ref"` のようなタイプミスを捕まえる)
    - `version` が無い
  - **木構造で warn:** root が 2 個以上(「時刻の基準が分かれている」)、深さ > 5、version が "1.0" 以外。
  - **木構造で許可するキー:** `_comment`(文字列または文字列の配列)と `$schema`。
  - **旧形式で error:**
    - 不正なキー(`"O_3"`、`"0-3"`、`"0_3_1"`、`"0_256"`)。Rust の「警告して無視」より厳しくする。TODO 58 L6 の「黙って 0 になる」罠を避けるため。
    - 正規化すると重複するキー(`"00_02"` と `"0_2"`)。
- `apply_time_offsets(hits, table, reported, on_missing)`: ヒットごとに map を 1 回引く。
  - 表に無い ch は生の時刻のまま通し、`reported` を使ってランごとに 1 回だけ `on_missing` を呼ぶ。
- `ActiveOffsets`(現在有効な表 + 原文 + パス + 読み込み時刻 + stale フラグ)と `reload_offsets()`:
  - 成功したら新しい表を採用し、stale を解除する。
  - 読めない・解釈できない・解決できない・オフセットの差 ≥ `safe_horizon` のいずれかなら、**前の表と原文を維持して stale にし、大声で知らせる**。黙って補正を 0 に戻すことはしない。
- provenance JSON:
  - `applied`、`convention`(文言で明記)、`source_path`、形式、`loaded_at`、`status`(ok / stale)、`reload_error`、root 数、深さ、警告
  - `resolved_abs_ns`(`%.17g` で書き、値が往復しても変わらないようにする)
  - `source_text`(エスケープした文字列として埋め込む)
  - オフセット無しのときは `{"applied":false,...}`。

### 3.2 `eb_core.hpp`
- `struct RunProvenance {std::string time_offsets_meta, time_offsets_source;}` を `SorterMsg` と `WorkMsg` に追加する(RunOpen のときだけ使う)。`DisplayMsg` は変えない(届くヒットは補正済み)。
- `BuiltEvent` のコメントと 388-391 のコメントを「補正後の時間軸の ns」に直す。

### 3.3 `root_sink.cxx`
- `Options.time_calib`、`--time-calib FILE`(`event_builder build --time-calib` と同名)。usage と parse_args に追加する。
- **起動時:** 読み込みに失敗したら全エラーを出して `return 2`。
  - オフセットの差が `margin_ns` 以上なら WARNING。
  - 起動行の例: `time offsets: <realpath> (tree, 11 entries, 1 root(s), depth 2, spread 26.7 ns) — aligned_ts = raw_ts - offset_ns; re-read at every run start`
  - 指定が無いときは `time offsets: none (timestamp_ns recorded RAW)` と出す。
- **Receiver(`1145-1253` 付近)**
  - `run_state.on_data` の RunOpen ラムダ(`1161` 付近)で、ファイルを読み直して固定する。`toff_reported` を空にし、ランごとにオフセット一覧の行を出す(stale なら STALE と付ける)。`m.prov` に provenance を詰める。
  - `keep_module` の直後(`1175` 付近)、display への写し(`1178`)と Sorter への push(`1183`)より前で `apply_time_offsets` を呼ぶ。
    - こうすると並べ替え・hits ツリー・統合イベント・matcher・ヒストグラムがすべて補正後の時間軸で動く。
    - Sorter は `timestamp_ns` で並べ、`core_range` や H10 の判定も同じ値で `lower_bound` するので、**Sorter より前で補正することが必須**。
  - 表に無い ch の WARNING はランごとに 1 回だけ出し、`counters.toff_unmapped` に数える。
- **Sorter の RunOpen(`1292-1301`):** `m.prov = std::move(in.prov)`。Workers は WorkMsg をそのまま渡すので変更不要。
- **`Recorder::open_run`**
  - `TNamed "timeSettings"` を `delila`・`built` 両ツリーの UserInfo に付ける。UserInfo はツリーの一部なので、AutoSave や ROOT の自動分割で生まれる各パートにも載る。ファイルのキーにすると最後のパートにしか載らない。
  - オフセット無しのときも `applied:false` を付ける。
  - 補正ありのときは、hits ツリーの title を `DELILA scalar events (timestamp_ns ALIGNED: raw - timeSettings offset; see UserInfo)` にする。
- **Writer の RunClose:** `<stem>_timeSettings.json` を書く。中身は **RunOpen 時に固定した原文**で、そのまま Rust EB にも渡せる。
  - `_hists.json` が finalize 時にファイルを読み直すのとは意図的に違う。
  - `copy_hists_sidecar` の共通部分を `write_text_sidecar` として切り出す。
- **ステータス行:** `| toff=off|11|11 STALE|11 (2 unmapped)`。

### 3.4 設定・スクリプト・ツール
- **`scripts/start_daq.sh`:** `rs_add_arg time_calib_file --time-calib`(TOML キーは Rust EB の `[network.event_builder].time_calib_file` と同じ名前)。
- **`tools/root_sink/delayline_offsets.C`**(新規。session scratchpad の `xyoff.C` を発展させる)
  - 引数: `delayline_offsets(pattern, xy1="1,2,3,5,4", xy2="6,7,8,9,10", out_json="", label="", module=0, win=1000)`
  - トリガーごとに ±win 内で最も近い腕のヒットを拾う(同距離なら後ろ側、`eb_core` の `nearest_in` と同じ規則)。4 本そろったものだけ使う。
  - 表示する x・y 自体の中心(1/99 % 点の中点)を cx、cy とし、オフセットを `XR=+cx/2, XL=−cx/2, YU=+cy/2, YD=−cy/2` とする。
  - この決め方の理由(マクロのヘッダコメントに書く):
    - 腕ごとの中点の差は、差そのものの中点と一致しない。表示する x・y の中心から決めれば、像の中心がぴったり 0 になる。
    - トリガーと LaBr3 を 0 のままにするので、TOF・`dt_trig`・dt1/dt2 は変わらない。
    - 2 本に半分ずつ振り分けるので、腕の時刻の和(dt_XL + dt_XR)も変わらない。
    - YU と YD を入れ替えて使っても、cy の符号が反転するだけで同じ値が出る。
  - 出力: 木構造の JSON。
    - root は ThGEM1 のトリガー。ThGEM2 のトリガーは `[m, trig1]` に 0 で参照させる。
    - 入力に出てきたそれ以外の ch もすべて 0 で登録する。こうしておくと、root_sink の「表に無い」警告が「較正後に新しい ch が現れた」ことを意味するようになる。
  - 補正済みの `.root`(UserInfo が `applied:true`)を入れたときは「残差」と表示する(冪等性のチェック)。
  - 統合イベントの像の中心を出す関数 `delayline_centres_built()` も同梱する。
- **`compare_hits.C`:** 4 番目の引数でオフセットファイルを受け取り、生の側(A)に同じ補正を掛けてからビット一致を比べる。
  - 引数なしで比べると、オフセットを掛けた ch だけ tsxor が不一致になる(件数や和は一致する)。この挙動をドキュメントに書く。
- **`config/time_settings_thgem.json`**(新規): run 56 にマクロを掛けた結果をコミットする。
  - 腕ごとの中点から見た目安: ch2 +8.70 / ch3 −8.70 / ch4 +13.35 / ch5 −13.35(ref `[0,1]`)、ch7 +1.50 / ch8 −1.50 / ch9 −5.90 / ch10 +5.90(ref `[0,6]`)。
  - オフセットの差は約 27 ns で、`safe_horizon` 1000 ms や margin 20 ms よりはるかに小さい。
- **`config/config_thgem_pfqaq.toml`:** `time_calib_file = "config/time_settings_thgem.json"` を追加し、コメントを付ける。
  - 内容: 移設でケーブルが変わったこと、符号、作り直し方、次のラン開始から有効になること、一時ファイルに書いてから `mv` で置き換えること。
- **`schemas/timeSettings.schema.json`:** `_comment` を許可し、root_sink でも使うことを明記する。
- **ドキュメント:**
  - `tools/root_sink/README.md`: 使い方、start_daq の対応表、出力、設計メモ、テスト。
  - `docs/root_sink_manual.md`: 「時刻オフセット」の節、TTree の時刻が補正後であること、`_timeSettings.json`、トラブルシュート(読み直し失敗で STALE、表に無い ch、XY が中心に無い)。

## 4. テスト(TDD、既存の CHECK 方式)

### `test_sink_core.cpp`

| テスト | 内容 → 期待値 |
|---|---|
| 符号 | オフセット {(0,1):0, (0,2):+30, (0,3):−5}、時刻 1000/1030/995 → すべて 1000.0 |
| 木構造の解決 | Rust の `single_root_resolves` のフィクスチャを、子が先に来る順で与える → (5,0)=59.05, (0,0)=46.75, (9,1)=0.05、root 1、深さ 2 |
| root 自身のオフセット | root 100、子 5 → 100, 105 |
| 木構造の error | 重複、参照先なし、2 個の循環、自己参照、範囲外、型違い、`ref` 欠落、`"Ref"`、未知のトップキー、version 欠落、壊れた JSON → それぞれ error。悪いエントリが 2 つなら error も 2 つ(全部集める) |
| 木構造の warn | root 2 個、深さ 7、version "2.0" |
| 旧形式 | 正常系、`"O_3"`、`"00_02"` と `"0_2"` の重複、`"0_256"`、`entries` と `offsets` の両方がある |
| モジュール別 | (0,2):10 と (1,2):−20 → それぞれ 990 / 1020、(2,2) は生のまま + 報告 |
| ランごとに 1 回の報告 | 同じランで 2 回目は報告しない、`clear()` 後は再び報告する |
| 読み直しの方針 | 正常 A → 不正(A を維持、stale)→ 読めない → 正常 B(stale 解除)→ 差が大きすぎる(error) |
| provenance | JSON を書いて読み直し、引用符・改行・バックスラッシュを含む原文が往復で変わらないこと |
| run 56 相当 | 合成イベント(腕の遅延は §1 の値)で、生では (−17.4, −26.7) / (−3.0, −11.8)、補正後は 0 ± 1e-9 |

### `test_eb_core.cpp`

| テスト | 内容 → 期待値 |
|---|---|
| 補正後の時刻で並べ替え | ch2 を +30 補正。生では ch2 が後に来るが、補正後は (ch2 @ T−10, ch1 @ T) の順で各 1 回、単調に並ぶ。統合イベントで `dt_xl1 == −10`(生なら +20) |
| オフセットの差が大きすぎる | 差 20000 > horizon 5000 → `late_hits() > 0`。`reload_offsets` が差を `safe_horizon` 未満に制限する根拠 |

ビルドと実行: `g++ -std=c++17 -O0 -g test_sink_core.cpp -o /tmp/ts && /tmp/ts`、`g++ -std=c++17 -O0 -g -pthread test_eb_core.cpp -o /tmp/teb && /tmp/teb`

### E2E(Mac、ROOT 6.40 `/opt/ROOT`)

run 56 の delila2root ツリー(生の時刻、到着順)を `hadd` でまとめ、`test_publisher` で root_sink に流す。
root_sink は ThGEM と同じフラグで動かし、ポートは `--zmq tcp://127.0.0.1:5560 --http-port 8091` にする。

| # | 確認すること |
|---|---|
| E0 | オフセット無し: `compare_hits` で IDENTICAL、UserInfo は `applied:false`、統合イベントの像の中心は (−17.4, −26.7, −3.0, −11.8) |
| E1 | オフセットがすべて 0: IDENTICAL(x − 0.0 はビット単位で同じ) |
| E2 | 腕ごとの中点から作った目安値: 4 番目の引数付き `compare_hits` で IDENTICAL、取りこぼし 0、像の中心 \|·\| ≤ 3 ns |
| E3 | マクロの出力: 像の中心 \|·\| < 0.5 ns、出力にマクロを掛けた残差 < 0.5 ns |
| E4 | UserInfo が両ツリーにあり、sidecar が元ファイルとバイト単位で一致し、title に ALIGNED と入っている |
| E5 | root_sink を止めずにラン 56→57→58 を流す: 57 で編集が反映され、58 で不正なファイルにすると ERROR・STALE・前の原文になる |
| E6 | 不正・読めない・差が大きすぎる(2e9 ns)ファイルでは起動しない(exit 2、全エラーを表示) |
| E7 | ch0 が無いファイル: m0c0 の WARNING がランごとに 1 回だけ出る |
| E8 | 自動分割のしきい値を小さくしたビルド(`-DROOTSINK_TEST_MAX_TREE_SIZE=300000`): 各パートに UserInfo、sidecar は 1 つ |
| E9 | 起動時に表示する表が `eb_offsets` の出力と一致する(`config/time_settings_thgem.json` と、モジュールが複数ある `tools/p91zr_test/timeSettings.json` の両方) |

## 5. デプロイ(pf-qaq)と確認

1. pull し、`source ~/root/bin/thisroot.sh` してから単体テストを通し、README §Build に従ってビルドする。
2. run 56 にマクロを掛けて `config/time_settings_thgem.json` を作る。`eb_offsets` で突き合わせてからコミットする。
3. 新しいバイナリを別ポートで空起動して、起動行を確認する。
   `--zmq tcp://127.0.0.1:5999 --http-port 0 --out-dir <scratch>`
4. **ランとランの間に限り**(DAQ が待機中で、ログに "finalized" が出ている)入れ替える。
   - `cp root_sink root_sink.prev` してから、`install` → `mv` で置き換える(差し替えを一瞬で済ませ、動いている root_sink に影響させないため)。
   - TOML にキーを足し、`stop_daq.sh` → `start_daq.sh` で起動し直す。
5. 最初のランで次を確認する。
   - ログに起動行とランごとのオフセット行があり、ERROR が無い。
   - JSROOT `:8090` の xy1・xy2 が中心にある。
   - sidecar と UserInfo がある。
   - マクロの残差が 1 ns 未満。
   - 同じランの `.delila` を delila2root に掛けると生の (−17, −27) のまま(`.delila` に手を加えていない証拠)。
6. **戻し方:** `root_sink.prev` に戻すか、TOML のキーをコメントアウトする(キーが無ければ今と同じ出力になる)。

## 6. リスク

- **補正できるのは一定の遅延だけ。** 独立に起動したボード同士(`START_MODE_SW`)は時計がずれていくので、表では直せない。ハードウェアの同期が前提(Phase 2)。
- **補正で到着順の乱れが増える。** 増える量はオフセットの差の分。差 ≥ `safe_horizon` は error、差 ≥ `margin` は warning にする。
- **hits ツリーが生の時刻でなくなる。** 同じランでも root_sink の `.root` と delila2root の `.root` で `timestamp_ns` が違う。title、UserInfo、sidecar、ドキュメントで明示する。run ≤ 56 の `.root` は生の時刻のまま。
- **補正後の時刻が負になることがある**(ラン開始直後の raw ≈ 0 から引くため)。パイプラインは問題ないが、ts ≥ 0 を前提にしたオフライン解析のマクロには注意。
- **読み直しの瞬間にファイルが書きかけ**だと解釈に失敗し、STALE(前の表を維持)になる。安全側だが、「一時ファイルに書いてから `mv`」をドキュメントに書く。
- **共有 JSON パーサの制限:** 重複キーは後勝ち、末尾のゴミは検出しない、`\u` は `?` になる。Phase 1 では許容し、ヘッダは変えない。
- **既存の不整合(今回は直さない):** `_hists.json` は RunOpen ではなく finalize 時にコピーしている。

## 7. Phase 2(マルチデジタイザ、記述のみ)

- **`M:C` 形式の指定:** `--xy*-ch`、`--gamma-ch`、`--thgem*-ch`、`BuilderConfig`、hist_config に module のカットを足す。`parse_ch_list` に `M:` を付けられるようにし、キーを (module, channel) にする。
  - `--module` は複数指定できるようにするか、廃止する。
  - `compare_hits` もモジュールを見るようにする。
  - Sorter と matcher の基準時刻(`max_ts_`)は全ボード共通なので、ボード同士の同期が前提になる。
- **ハードウェアの同期:**
  - TRG-OUT→S-IN と CLK-OUT→CLK-IN のデイジーチェーンにする。
  - スレーブは `START_MODE_S_IN` + `OUT_PROPAGATION_SYNCIN`、マスターは `START_MODE_SW` + `OUT_PROPAGATION_RUN`。
  - 例: `config_sn01_3xv1725.toml:41-49`、`config_3MV_20260312.toml`、`docs/psd1_decoder_spec.md:585-599`。
  - ボード間の定数オフセットは、較正ラン(`event_builder time-calib`)で測ってこの表に入れる。
- **その他:**
  - delila2root に `--time-calib` を足す(同じ `time_offsets.hpp` を使う)。オフラインでも補正後のツリーを作れるようにする。
  - 表に無い ch の一覧を RunClose で provenance に載せる。
  - JSROOT に、今使っているオフセットの表を `TNamed` で表示する。
  - 前のランと比べて変わったオフセットを、ランごとのログに差分として出す。

## 8. 実装結果 (2026-10-07)

### 実装したもの
- `tools/root_sink/time_offsets.hpp`(新規): 解釈・検証・解決(木構造 + 旧形式)、`apply_time_offsets`、`reload_offsets`(失敗時は前の表を維持して STALE)、provenance JSON。空ファイルは「empty」と明示する。
- `eb_core.hpp`: `RunProvenance` を `SorterMsg` / `WorkMsg` に追加。時間軸のコメントを更新。
- `root_sink.cxx`
  - `--time-calib`、起動時の読み込み(失敗なら exit 2)、ランごとの読み直し(Receiver の RunOpen)。
  - `keep_module` の直後で補正する。
  - 両ツリーの UserInfo に `TNamed "timeSettings"`、補正時は hits ツリーのタイトルに ALIGNED。
  - `_timeSettings.json` sidecar(`write_text_sidecar` を共通化)、状態行の `toff=`。
- `delayline_offsets.C`(新規): オフセットの測定、JSON 出力、残差表示、`delayline_centres_built()`。
- `compare_hits.C`: 4 番目の引数でオフセットを受け取り、生の側を補正してから照合する。
- `scripts/start_daq.sh`: `time_calib_file` → `--time-calib`。
- `config/time_settings_thgem.json`(run 56 にマクロを掛けた結果)と `config_thgem_pfqaq.toml` の `time_calib_file`。
  - ThGEM1: ch2 +8.752 / ch3 −8.752 / ch4 +14.900 / ch5 −14.900
  - ThGEM2: ch7 +1.250 / ch8 −1.250 / ch9 −7.576 / ch10 +7.576(トリガー・LaBr3 は 0)
- `schemas/timeSettings.schema.json`: `_comment` を許可。
- `SPECIFICATION.md` §4.3.1 の符号の説明を修正(`hit.rs:92` に合わせた)。
- README と `docs/root_sink_manual.md` に、時刻オフセットの節とトラブルシュートを追加。

### テスト
- 単体テスト: `test_sink_core` 223 → **307**(+84)、`test_eb_core` 171 → **179**(+8)。全件合格。Mac(ROOT 6.40)と pf-qaq(ROOT 6.38)の両方で確認。
- E2E(Mac、run 56 の再生、全ケース合格)

| # | 結果 |
|---|---|
| E0 | オフセット無し: IDENTICAL、UserInfo `applied:false`、統合イベントの像の中心 (−17.504, −29.801) / (−2.500, −15.152) = マクロの測定値と一致 |
| E1 | オフセットがすべて 0: IDENTICAL |
| E2 | 腕ごとの目安値: IDENTICAL(補正後に照合)、像の中心 (−0.1, −3.1) / (+0.5, −3.4) |
| E3 | **マクロの値: IDENTICAL、像の中心 (0.000, 0.000) / (0.000, 0.000)**。4 本そろったイベント数(14,660 / 15,751)と統合イベント数(33,463)は補正前と同じ |
| E4 | UserInfo が両ツリーにあり、title に ALIGNED、sidecar は元ファイルとバイト単位で一致 |
| E5 | 1 つの root_sink でラン 56→57→58: 57 で編集が反映、58 で不正なファイルにすると ERROR・STALE・前の表(sidecar も前の表) |
| E6 | 不正・読めない・差 2e9 ns のファイルでは起動しない(exit 2、エラー内容を表示) |
| E7 | ch0 が無いファイル: WARNING がランごとに 1 回(2 ランで 2 回) |
| E8 | 自動分割のしきい値を小さくしたビルド: 6 パートすべてに UserInfo、sidecar は 1 つ |
| E9 | 解決結果が Rust `eb_offsets` と完全一致(本ファイル 11 行、`tools/p91zr_test/timeSettings.json` 208 行) |

### 配備(pf-qaq、2026-10-07 08:14)
- ランとランの間に `~/.local/bin/root_sink` を入れ替えた(旧版は `root_sink.prev`)。`stop_daq.sh` → `start_daq.sh` で起動し直し、`--time-calib` が渡ることを確認した。
- 送ったのは、pf-qaq 側が直前のコミットと同一だったファイルだけ。pf-qaq の `thgem_v1730_psd1_0.json` と `monitor_layout.json` は、UI で調整されたと思われる差分があるので触っていない。
- **ラン 58(3 分)**
  - sidecar が設定ファイルと一致。
  - `.delila` を delila2root にかけ、同じ補正を掛けて照合すると IDENTICAL。
  - `late=0`、`toff=11`、title に ALIGNED、UserInfo `applied:true`。
  - ただし**イベントは 44 件・統合イベントは 0 件**(検出器が動いていなかった)。**XY 像の中心の実機確認は、次に検出器が動いているランで行う**。
  - 確認方法: `delayline_centres_built()`。補正済みの `.root` にマクロを掛けて残差が 1 ns 未満であることも見る。

