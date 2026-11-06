# TODO 83 — エネルギー較正の表示(ADC → keV、1 次式、Monitor と root_sink の表示だけ。係数は外で作る)

**Status: 🚧 Phase 0〜3 実装済み、master に取り込み、pf-qaq に配備 (2026-11-06) — 残 = §10 の実データでの確認、学生の表を置く**

**関連:** [73_campaign_2026_11_pfqaq.md](73_campaign_2026_11_pfqaq.md)(N_Wall、最初の使い道の候補) /
[79_root_sink_nwall_online.md](79_root_sink_nwall_online.md)(nwall_sink の表示) /
[70_root_sink_time_offsets.md](70_root_sink_time_offsets.md)(ファイルの読み直しの先例 = 失敗したら前の表を保持して STALE) /
[docs/plans/energy_calibration_psd.md](../docs/plans/energy_calibration_psd.md)(2026-02 の旧設計書。Phase 4〜5 が未着手。本チケットが置き換える) /
GitHub issue ELI-NP/delila-rs#7(open、「keV, MeV に換算したヒストグラムも必要」、PSD の部分は済み)

---

## 1. 目的と範囲

**ユーザーの指示(2026-11-06):**
- 「表示だけ変えたい。だから 1 次の線形フィットだけに絞る。root_sink と Monitor の両方でできるようにしたい。」
- 「基本的にパラメーターは学生なんかのデータ解析練習としてやってもらってパラメーターだけもらうのでその前提にしたいです。」

**解釈:** フィット(ピーク位置と既知のエネルギーから 1 次式を決めること)は **delila-rs の外**で行う。delila-rs がやるのは、受け取った係数を置き、`E[keV] = kev_per_adc × ADC + offset_kev` を**表示に当てる**ことだけ。

- **やること:** チャンネルごとの 1 次式を 1 つのファイルに置き、**Web Monitor** と **root_sink(nwall_sink、standard)の JSROOT 表示**が同じファイルを読んで keV の軸で表示する。解析する人に渡す「何を・どこから・どう返すか」の文書(§5)。
- **記録は一切変えない。** `.delila`・`.root`(hits / events)はこれまでどおり生の ADC(`u16`)。この機能が触るのは画面のピクセルだけ。
- **係数の値に課す規則は、検証に要るものだけ:** 有限で、傾き > 0。何本のピークで決めたかは問わない。

**範囲外(やらない):**
- **係数を作ること全部**: ピーク検出、最小二乗、Monitor の「フィット結果を較正に追加」、ROOT の較正マクロ、UI からのファイル保存。既存の Gaussian フィット(expand ダイアログ)は残すが、用途は「受け取った係数で線が正しい keV に来るか」の確認。
- 2 次以上の多項式、チャンネルごとの単位の切り替え、MeV 表示(§3.7)。
- 記録ファイルへの較正値の書き込み、sidecar、UserInfo(§3.9)。
- `energy_short`、`psd`、`user_info*` の較正(§3.3)。
- Tune Up 画面(`pages/waveform/`)の 1D ヒストグラム。`app-histogram-chart` を共用している(`waveform.component.ts:348`)が `xAxisLabel` を渡していないので ADC のまま変わらない。keV にするのは別件。
- EB の `chSettings.json` の `p0..p3`(§3.8)。
- `tools/root_sink/thgem/` のオフラインマクロ(表示ではない)。

## 2. 読んで確かめたこと(2026-11-06、コードのみ。実機は触っていない)

| # | 事実 | 根拠 |
|---|---|---|
| F1 | Monitor のバックエンドは生の ADC を 1 ADC = 1 bin で積む(既定 65536 bin、0〜65536)。REST は bin の配列 + `config{num_bins,min_value,max_value}` を返す | `src/monitor/mod.rs:397-410`(`histogram.fill(event.energy as f32)`)、`:108-127`、`:808-832`、`:866-883` |
| F2 | フロントの X 軸セレクタ Channel / keV / MeV は**軸の名前を変えるだけ**。bin 中心 `x = min + (i+0.5)·幅` は ADC のまま、ツールチップは常に `Channel:` | `setup-tab.component.ts:159-170`、`histogram-chart.component.ts:188-191`(x)、`:237`・`:449`(name)、`:442`(tooltip)。**今日の「keV」表示は ADC の値に keV と書いているだけ。本チケットで直す** |
| F3 | ズーム範囲 `ViewCell.xRange`、フィット範囲 `fitRange`、フィット結果(center / sigma / fwhm)、タブの `xView` はすべて ADC の数値で、レイアウトとして保存される | `histogram.types.ts:292-321`、`:238-245`、`monitor.component.ts:416-430`(localStorage + Operator の `PUT /api/monitor/layout`) |
| F4 | レイアウトの保存先は `config/digitizers/monitor_layout.json`(Operator の `config_dir` が `./config/digitizers`) | `src/bin/operator.rs:400`、`src/operator/routes/monitor_layout.rs:29,66` |
| F5 | Gaussian フィットは `hist.config` の `min_value` と bin 幅から x を作って当てはめる。**config を変えれば別の単位でそのまま動く** | `fitting.service.ts:90-199`、`:265-284`(`extractRange`)、`histogram-expand-dialog.component.ts:369-375` |
| F6 | 2D ヒートマップはカテゴリ軸で、ラベルは bin の左端を**整数に丸めた文字列**(`Math.round`) | `heatmap-chart.component.ts:181-190`、`:86`・`:278`(tooltip) |
| F7 | `chSettings.json` の `p0..p3` は EB の構造体にあり、`calibrate_energy` を呼ぶのは自分のテストだけ(`src` と `tools` を grep)。UI は `P0..P3`(大文字)で作る。Rust は `rename="p0"` + `default` なので、UI が作った JSON を Rust が読むと **`p1 = 0.0` になる**(未知キー無視 + 既定値)。今は誰も読まないので実害なし | `src/event_builder/config.rs:36,51-63,74-77`、`web/operator-ui/src/app/models/types.ts:422-425`、`services/event-builder.service.ts:243-246` |
| F8 | root_sink の表示は main スレッドが持ち、Writer / Receiver からは **bounded(1000) の tee** で受ける(落としてよいのは表示だけ、`display_drops` に数える)。記録の構造体 `ScalarHit.energy` は `u16` | `nwall_sink.cxx:1221-1226`、`:1592`、`:1997-2003`、`root_sink.cxx:1829`、`:1958-1960`、`sink_core.hpp:115-121` |
| F9 | nwall_sink の表示で ADC のエネルギーを使うもの: `energy_m<M>`(ch × E、1024 bin **0〜65536**)、`pid`(E vs dE、512 bin **0〜32768**、各イベントの E sector と dE sector の**ADC で最大**のヒット)、`ls_psd`(E_long 0〜65536) | `nwall_sink.cxx:1237-1241`、`:1279-1284`、`:1328-1333` |
| F10 | standard root_sink の表示は組み込み(dt1 / dt2 / channels)+ `--hists` の宣言的定義。変数 `energy` / `energy_short` は hit スコープ、`value_of` が false を返すと fill しない(psd が未定義のときと同じ流儀) | `standard/hist_config.hpp:45-63,115-130`、`root_sink.cxx:1171-1186`、`:2411-2425`(fill は Display スレッド) |
| F11 | ファイルの読み直しの先例: 起動時に読み、ラン開始ごとに読み直し、失敗したら**前の表を保持して STALE**、黙って無効にしない。nwall の対応表は Writer が読み直して `DisplayMsg` で表示へ渡す(記録と表示で別の表を読まないため) | `time_offsets.hpp:369-421`、`nwall_sink.cxx:1863-1880`、TODO 79「BeamTime」 |
| F12 | TOML → プログラム: Monitor は `[network.monitor]` を Rust が読む(`MonitorNetworkConfig`)。root_sink は Rust が読まず、`start_daq.sh` が `[network.root_sink]` のキーを CLI フラグに写す(`rs_add_arg key --flag`) | `src/config/mod.rs:663-697`、`src/bin/monitor.rs:40-72`、`scripts/start_daq.sh:299-301,415-431` |
| F13 | N_Wall の対応表は 8 列目以降を「解析用(位置、較正…)」と予約している。書式は空白区切り・`#` コメント、C++(`nwall_map.hpp`)と Python(`gen_dashboards.py`)に同じ読み方のパーサがある | `config/channel_map_N_Wall.txt:17-19`、`nwall_map.hpp:123-146`、`tools/grafana/gen_dashboards.py:219` |
| F14 | **hits tree(名前 `delila`)の枝:** `module/b`、`channel/b`、`energy/s`(UShort_t、生の ADC)、`energy_short/s`、`timestamp_ns/D`。root_sink・nwall_sink・delila2root の 3 つで同じ名前。events tree は `HitMod[NHits]/b`、`HitCh[NHits]/b`、`HitEnergy[NHits]/s`(同じく生の ADC) | `root_sink.cxx:144,669-673`、`nwall_sink.cxx:112,565-569,591-593`、`delila2root.C:90,166-170`、`docs/root_sink_manual.md:236-244` |
| F15 | PHA1 の `energy` は 15 bit(0〜32767)。PSD の `energy` は 16 bit | `src/reader/decoder/pha1.rs:36`(`ENERGY_MASK = 0x7FFF`)、F9 の `pid` が 0〜32768 なのはこのため |
| F16 | Monitor の View のセルは 1 つの `(sourceId, channelId)` を持ち、重ね描きも合算もない。`sourceId` はそのまま REST の `/api/histograms/{module_id}/{channel_id}` の `module_id` になる。UI は Operator を経由せず `monitorBaseUrl` で Monitor に直接つなぐ | `view-tab.component.ts:518-528`、`histogram.service.ts:27-32,132-137`、`histogram.types.ts:292-294` |
| F17 | issue #7 は open。本文「keV, MeV に換算したヒストグラムも必要。PSD もあったら嬉しい」、コメント「PSDはできた」 | `gh issue view 7` |
| F18 | N_Wall の対応表は 112 ch: VX2730 32(beam 1、LaBr3 7、LS 24)+ Si 77(dE sector 16 + dE ring 45 + E sector 16)+ パルサー 3 | `config/channel_map_N_Wall.txt` |

## 3. 決めたこと(理由と、捨てた案)

### 3.1 係数の置き場所と形式: 空白区切りの表 1 つを、Monitor と root_sink が同じく読む
- **決定:** 実験ごとに 1 ファイル **`config/energy_calib_<exp>.txt`**(例 `config/energy_calib_nwall.txt`)、1 行 = 1 チャンネル `module channel kev_per_adc offset_kev`、`#` から行末はコメント(§4)。キーは `(module, channel)`(= hits tree の `module`/`channel` 枝、= `timeSettings.json` の `module`/`channel`、`mc_key`)。
- **理由(表を選んだ):** 係数は外から来る(§1)。解析した人が返すのは「チャンネルごとに数字 4 つ」で、表ならそのまま貼れる。`channel_map_*.txt` と同じ書式なので読み方・コメントの流儀・エディタの扱いが既に決まっている(F13)。列は 4 つちょうどで、それ以外は要らない(ユーザー、2026-11-06)。diff が 1 行 = 1 ch で読める。
- **捨てた案 — JSON(`timeSettings.json` と同じ流儀、`"unit"`・`"version"`・項目ごとの `"source"`・スキーマつき):** 自己記述で型が厳しい利点はあるが、受け取った数字を項目に書き直す手間(引用符・カンマ・最後の要素のカンマ)が毎回かかり、解析する人に「JSON を書いて」と頼むことになる。由来の欄もスキーマも要らない。単位は keV で固定なので(§3.7)、ファイルに書かせる値がない。将来 MeV を足すなら表示側の話で、ファイルの形式は変わらない。
- **捨てた案 — `chSettings.json` の `p0`/`p1`:** EB の MongoDB 文書(版管理つき、`event_builder_repository.rs`)で、`[module][channel]` の全チャンネル必須の 2 次元配列、多項式、`P0`/`p0` の食い違い(F7)、Monitor も root_sink も読んでいない、N_Wall は Rust EB を使わない。
- **捨てた案 — 対応表 `channel_map_N_Wall.txt` の 8 列目以降(F13):** N_Wall 専用で、Monitor・standard root_sink・ThGEM・ELIADE に対応表はない。較正は実験のあいだに何度も差し替わるので、役割の表と分ける。8 列目予約の文言は「較正は `energy_calib_*.txt`」に直す。
- **所有者:** 実験を回す人(ユーザー)。受け取った行を貼る、または受け取ったファイルをそのまま置く。「一時ファイルに書いて `mv`」を TOML のコメントに書く。
- **見つけ方:** TOML のキー **`energy_calib_file`** を `[network.monitor]` と `[network.root_sink]` の**両方**に同じ値で書く。Monitor は `MonitorNetworkConfig` に `energy_calib_file: Option<String>` を足して読む。root_sink は `start_daq.sh` の `rs_add_arg energy_calib_file --energy-calib` で CLI に写す。
  - 理由: `time_calib_file` が `[network.event_builder]` と `[network.root_sink]` に同名で重複しているのと同じ流儀(F12)。Monitor は root_sink の節を読まないし、Operator は `[network.root_sink]` を一切解釈しない。1 か所にまとめるには Rust 側か start_daq.sh に新しい解決ルールが要り、今の価値に見合わない。
  - 両方が何を読んだかは Monitor の `GET /api/calibration`(§6.1)と root_sink の起動行(§6.3)に実パスが出るので、食い違いは見れば分かる。

### 3.2 係数は delila-rs の外で作る。delila-rs は受け取った表を検証して当てるだけ
- 解析する人に渡す文書は §5(式、どの枝、キー、返し方、較正できなかった ch)。
- delila-rs が表に課す規則は §4 の検証だけ。ピークの本数や方法は問わない(1 点で切片 0 と決めた表も、そのまま受け付ける)。
- **受け取った表の確かめ方は §5.2**: 新しい道具は作らず、Monitor の `Reload calibration` と root_sink の起動行・`/ReloadCalib`・`/EnergyCalib`(どれも読み込み結果を件数と行番号つきのエラーで返す)、それと keV の View で既知の線の位置を見る。

### 3.3 Monitor の表示: bin は ADC のまま、`config` の min/max だけを写像する
- **決定:** 変換は**フロントの 1 か所**。1 次式(`kev_per_adc > 0`)は等間隔 bin を等間隔 bin に写すので、ヒストグラムの配列はそのまま、`config.min_value` と `config.max_value` を `kev_per_adc × v + offset_kev` に置き換えた「keV のヒストグラム」を作る(`utils/energy-calib.ts` の純関数 `calibrateHistogram1D` / `calibrateHistogram2D`)。これを `view-tab` と `expand-dialog` がチャートとフィットに渡す。**チャート・`rebin1d/2d`・フィット・ズームは何も知らなくてよい**(F5)。セルは 1 ch なので(F16)、セルごとにその ch の係数を 1 組引くだけ。
- **単位はタブの性質**(今の `xAxisLabel` と同じ場所、`SetupConfig` / `ViewTab`)。**`xAxisLabel === 'keV'` のタブでは、タブとセルの X 方向の数値(`xView.min/max`、`xRange`、`fitRange`、`fitResult`)はすべて keV**。`'Channel'` のタブでは ADC。View を作ったあとに単位は変えない(今も `histogramType` は作成時に固定)。別の単位で見たければ View をもう 1 つ作る。
  - 理由: 変換を往復させない。フィット結果がそのまま keV(「FWHM 3.4 keV」= 受け取った係数の確認に使える)。ズーム範囲が物理量(1100〜1400 keV)なので係数を差し替えても意味が変わらない。ゲインの違うチャンネルを並べた View で「Apply Range to All」をすると、**全セルが同じ keV の窓**になる(これが keV 表示の価値)。
  - 捨てた案: ECharts の軸 formatter だけ(ツールチップ・フィット・ズームが ADC のまま残り、今の罠と同じ)。サーバで keV のヒストグラムを別に積む(メモリ倍、bin 幅の選択が要る、REST の値が ADC か keV か曖昧になる)。チャートが入口と出口で変換する(保存した値の意味が単位に依存する)。
- **2D:** 変換するのは **`energy` の軸だけ**。`energy_short` は短ゲートの電荷で、長ゲートの係数を掛けると数字が合わない(keV と書いた嘘になる)。`psd`・`user_info*` はそのまま。ヒートマップのラベルは丸めを bin 幅に合わせる(幅 ≥ 1 keV なら整数、それ未満なら小数 1〜2 桁)。
- **既定の表示範囲:** keV のタブでは `xView.min/max` が無いときは**そのセルの全範囲(変換後の config)**。`DEFAULT_AXIS_VIEW.energy`(0〜65536、F3)は ADC 用なので keV では使わない。実装メモ: `rebin1d` は今 min/max/bins が揃わないと何もしないので(`histogram-chart.component.ts:96-98`)、bins だけのときはヒストグラムの range で補う。
- **フィット結果の表示:** 文字列に単位を付ける(`Center: 1332.4 ± 0.3 keV`、`FWHM: 3.4 keV`、ADC なら `ADC`)。係数の値が変わったとき、keV タブのその ch の `fitResult` / `fitRange` は消して snackbar で 1 回知らせる(古い係数のフィット曲線が新しいデータからずれて見えるのは罠)。ズーム範囲は keV なので残す。ADC と keV のあいだで単位が変わった ch はズームも消す。**判定は `loaded_at` でなく係数の値で行う**(ラン開始のたびに読み直すので、`loaded_at` で判定するとフィットが毎ラン消える)。
- **レイアウトの保存:** 形は変えない。`xAxisLabel` に `'keV'` が入るだけ。古いレイアウトの `'MeV'` は読み込み時に `'keV'` に写す(`migrateLegacyHistType` と同じ場所、`monitor.component.ts:376-414`)。古い `'keV'` タブは、これまで ADC を keV と書いていたものが、係数のある ch では本物の keV、無い ch では §3.4 の ADC 表示になる(どちらも今より正しい)。`'Channel'` に戻す案は、ユーザーが keV を選んだ意思を捨てるので採らない。

### 3.4 係数のない ch: ADC で表示し、ADC と大きく書く。keV とは絶対に書かない
- **決定:** keV のタブで係数が無いチャンネルは、**ADC のまま**表示し、軸名を `ADC (not calibrated)`、セルのラベルに琥珀色のバッジ `ADC`、ツールチップとフィット文字列も `ADC`。View のツールバーに「較正済み 10 / 12 cells」。Monitor ページの上に 1 行 `Energy calibration: <path> — 96 channels (keV), loaded 2026-11-06 10:21` / `none configured` / `ERROR: ...` / `STALE: ...`(`GET /api/calibration` から)。
- 理由: 係数が 1 つ欠けただけで形とレートが見えなくなるのは、シフトの人にとって最悪。はっきり印をつけた ADC は嘘ではない。混在する View も同じ規則をセルごとに当てる。「解析する人が較正できなかった ch は表から抜く」(§5.1)という約束と対になる: 抜いた ch はそのまま ADC で見え続ける。
- 捨てた案: 空のセルにメッセージ(データが見えない)。全セルを ADC に落とす(較正済みの ch まで巻き添え)。

### 3.5 root_sink の表示: keV で詰める別のヒストグラムを作る。ADC のものは残す。記録経路には触らない
- **変換の方法:** **fill のときに keV に換算して、keV で定義したヒストグラムに詰める。** `SetLimits` で軸を貼り替える案は、1 枚に複数 ch が乗る `energy_m<M>`(ch × E)や `pid`(E vs dE、ch をまたぐ)では成り立たない(行ごとに違う 1 次式を 1 本の軸に貼れない)。
- **nwall_sink(範囲内、N_Wall が使い道):** `/hits/energy_keV_m<M>`(ch × E[keV]、2048 bin)と `/events/pid_keV`(E[keV] vs dE[keV]、512 × 512)を**追加**する。ADC の `energy_m<M>`・`pid`・`ls_psd` はそのまま(データは常にどこかに見える)。**係数の無い ch は keV のヒストグラムには詰めない**(ch ごとの WARN は出さない: 表が空だと 100 行を超えるので、ランの終わりに 1 行で一覧を出し、状態行に件数を出す)。
  - **`pid_keV` の「最大のヒット」:** ADC の `pid` は ADC で最大の E sector・dE sector を選ぶ(F9、ゲインが sector ごとに違えば keV では最大でないことがある)。`pid_keV` は**先に keV に換算し、係数の無い ch を除いてから、keV で最大**を選ぶ。ADC の `pid` は変えない。
  - **keV 軸の範囲:** **対応する ADC ヒストグラムの上限を写像する**: `energy_keV_m<M>` は `kev_per_adc × 65536 + offset_kev`(ADC の `energy_m` が 0〜65536)、`pid_keV` は E 側・dE 側それぞれ `kev_per_adc × 32768 + offset_kev`(ADC の `pid` が 0〜32768、PHA1 は 15 bit、F15)。そのヒストグラムに入る ch の最大を上限、`min(0, 最小の offset_kev)` を下限にする。65535 を一律に使うと `pid_keV` の軸の半分が空になるので使わない。
  - **係数の値が変わったら、keV のヒストグラムだけ作り直す**(Unregister → delete、次の fill で新しい範囲で作る)。範囲が同じでも作り直す(古い係数で詰めた分と混ざるため)。同じ値の読み直しでは何もしない。ADC のヒストグラムはラン境界で消さない設計(root_sink マニュアル「ライブモニタ」)のままで、keV のものだけ表が変わったときに消えるのは、表示専用なので受け入れる。
  - 捨てた案: 役割ごとの固定定数、CLI オプション。定数は実験ごとに違い、オプションは増やさない。
- **standard root_sink(範囲内、最後のフェーズ):** `--hists` に hit スコープの変数 **`energy_kev`** を 1 つ足す。係数の無い ch は `value_of` が false(psd 未定義と同じ、F10)。bin 数と範囲はユーザーが JSON に書く(HPGe なら 0〜3000 keV × 3000 bin のように)。`energy_kev_range` のカットは要望があるまで作らない。ELIADE(`config/histograms_sn01.json`)がそのまま恩恵を受ける。宣言的なので安く、範囲に入れる。
- **共有コード:** `tools/root_sink/common/energy_calib.hpp`(ROOT に依存しない。空白区切りの読み込み(`nwall_map.hpp` と同じ書き方)、検証、`EnergyCalib` 表、`to_kev`、`axis_limits_kev(table, channels, adc_max)`、`ActiveCalib` + `reload_calib`(`reload_offsets` と同じ方針)、起動行と状態行の文字列)。nwall_sink と root_sink の両方が include する。`sink_core.hpp` / `eb_core.hpp` / `event_def.hpp` は変えない。
- **読み直す場所:** **Display(main スレッド)が RunOpen を受けたときに自分で読む**。対応表のように Writer 経由にしないのは、記録経路がこの表を一切使わないから(対応表は BeamTime の記録に要るので Writer に移した、F11)。RunOpen は blocking push なので必ず届く。
- **表示 tee の drop との関係:** 変わらない。keV のヒストグラムは ADC のものと同じチャンクから詰まるので、落とした分は両方から同じように欠ける。記録経路は unbounded のまま。
- **記録経路に触れていないことの確認:** §7。

### 3.6 反映のタイミング: 起動時、ラン開始ごと、要求時。ラン中に勝手には変えない
- **Monitor:** 起動時に読む。`on_start`(`src/monitor/mod.rs:1040-1047`、ラン開始で Clear するのと同じ場所)で読み直す。**`POST /api/calibration/reload`** で要求時に読み直す(UI の「Reload calibration」ボタン)。UI は `GET /api/calibration` を 5 秒ごとに取りに行く(小さい JSON)ので、どのブラウザでも同じ状態になる。
  - 要求時の読み直しが要る理由: 受け取った表を置いた**その場で**、ランを始めず(ラン番号を消費せず)に件数・エラー・線の位置を確かめるため(§5.2)。
- **root_sink / nwall_sink:** 起動時、RunOpen、**`/ReloadCalib`**(THttpServer のコマンド。standard の `/ReloadHists` と同じ仕組み、`root_sink.cxx:124-126,1688-1693`。nwall に同じ 10 行を足す)。
- **失敗したら:** 前の表を保持して STALE(F11)。起動時に失敗したら**較正なしで起動**し、ERROR を出し、状態に `ERROR` を残す(`ecal=ERROR`、UI の 1 行)。**`--time-calib` のように exit 2 にはしない**: あれは記録される時刻を変えるファイルで、こちらは画面だけ。画面のために Monitor や記録中の sink を止めるのは優先順位が逆。
- 捨てた案: ファイルの mtime 監視。書きかけを読む(TODO 70 §6 のリスク)し、いつ変わったかが分かりにくい。要求時の reload で足りる。

### 3.7 単位: keV だけ
- 表示は keV のみ。ファイルの列名 `kev_per_adc` / `offset_kev` に単位が入っているので、ファイルに単位の欄は無い。
- Si の dE-E が 0〜20000 keV と読めるのは我慢する。MeV は後から「表示で 1000 で割る」だけなので、ファイルにも Rust にも C++ にも今は入れない。
- 捨てた案: Channel / keV / MeV の 3 択を生かす。MeV の経路(型・ラベル・丸め・ヒートマップのラベル)が倍になるのに、頼まれていない。

### 3.8 既存の keV/MeV セレクタ、旧設計書、issue #7、`p0..p3`
| 対象 | 扱い | 理由 |
|---|---|---|
| セレクタ Channel / keV / MeV(F2) | **ここで直す。** `ADC / keV` の 2 択にし、`XAxisLabel` を `'Channel' \| 'keV'` に絞る。保存済みの `'MeV'` は `'keV'` に写す | 嘘の表示を消すのが本チケットの最低ライン。Phase 1 で、係数が 1 つも無くても「keV と書いて ADC を出す」ことは無くなる(§3.4 の ADC バッジになる) |
| `docs/plans/energy_calibration_psd.md` | **Phase 1 で**冒頭に「較正の部分(§2.1/2.3/3.7/4.3〜4.5/5 Phase 4〜5)は TODO 83 に置き換え。PSD の部分は実装済みの記録として残す」と書き足す。削除しない | 2026-02 の決定(フロントで変換、ファイルに係数)は本チケットでも生きている。多項式・CRUD・自動検出・Monitor 内の係数作成は捨てた(係数は外で作る) |
| issue #7 | Phase 1 が入ったらコメント(本チケットへのリンク、keV は 1 次式・表示のみ・係数は外で作る)、Monitor の Phase が終わったら close。どちらもユーザーに確認してから行う。設計の時点では GitHub に書き込んでいない | — |
| `ChSettings.p0..p3`、`calibrate_energy`、UI の `P0..P3`(F7) | **別件。触らない。** F7 の `p1 = 0.0` は EB 設定 UI の潜在バグなので、Phase 1 のときに、ユーザーに確認してから 1 行の issue を立てる(公開 repo への書き込みなので) | EB の ELIFANT 互換の文書で、表示とは責任が違う。直すなら UI の型と Mongo の既存文書の両方を見る必要があり、本チケットに混ぜると範囲が崩れる |

### 3.9 来歴: 記録ファイルには何も足さない
- **決定:** `.delila`・`.root`・sidecar・UserInfo は**バイト単位で今と同じ**(§7 で確認する)。置いた人が書きたければ `#` コメントに書き、「いつ何を変えたか」は git の履歴で追う。解析する人にコメントを求めない。
- 理由: 研究者が知るべき「変えたもの」は記録された値の変更であって、この機能は値を変えない。TODO 70 で sidecar と UserInfo を付けたのは hits tree の時刻が補正後になったから。同じ基準を当てると、こちらには付けない。
- 将来 events tree に補正後のエネルギーを書く日が来たら(TODO 80 の「events = 補正後 + 補正表同梱」)、そのときに表を同梱する。それはこのチケットの外。

## 4. ファイル形式(`config/energy_calib_<exp>.txt`)

```
# Energy calibration for experiment "N_Wall" — DISPLAY ONLY (Monitor and nwall_sink).
#   E_keV = kev_per_adc * energy + offset_kev      (energy = the raw ADC branch, see TODO 83 §5)
#   module channel kev_per_adc offset_kev      (exactly these four columns)
# One line per channel. A channel not listed is shown in ADC (never labelled keV).
# Re-read at start-up, at every run start and on request (Monitor: Reload calibration; sink: /ReloadCalib).
# A bad file keeps the previous table (STALE): write a temp file and mv it into place.
# .delila and .root keep raw ADC whatever is written here.
#
# mod ch  kev_per_adc  offset_kev       (VALUES BELOW ARE AN EXAMPLE, not a measurement)
0   1     0.22180      3.1
5   0     0.30120     -12.4
5   1     0.29870     -11.9
```

- **行:** `#` から行末はコメント。空行は無視。残りを空白で分け、**4 列ちょうど**: `module channel kev_per_adc offset_kev`。5 列以上は黙って読み飛ばさずエラーにする(CSV の貼り間違いや列のずれを捕まえる)。
- **検証(ファイル全体を拒否、全部集めて行番号つきで報告):** 4 列でない、`module`/`channel` が 0〜255 の整数でない、`kev_per_adc` が数でない・非有限・**≤ 0**、`offset_kev` が数でない・非有限、同じ `(module, channel)` が 2 回(両方の行番号)。
  - 傾き ≤ 0 を行単位でなくファイル単位で弾くのは、`reload_offsets` と同じ「部分的に採用しない」規則に揃えるため(採用した表が何かを一意にする)。
- コメントだけ・空のファイルは **OK で 0 件**(「係数なし」を明示したファイル)。読み込み時に `0 channels` と出す。
- 表に無い `(module, channel)` = 係数なし(§3.4、§3.5)。警告は Monitor では出さない(keV タブのバッジで見える)、sink ではランごとに 1 回。
- 単位・版の欄は無い(§3.7)。形式を変えるときは別の名前のファイルにする。
- 列の順序を取り違えた表(`offset kev_per_adc`)は、傾き ≤ 0 か桁で見つかるものしか検出できない。§5.2 の「既知の線の位置を見る」が最後の砦。

## 5. 受け渡し(解析する人に伝えること、受け取った表の確かめ方)

### 5.1 解析する人へ(README の 1 節にそのまま載せる文面の下書き)
1. **求めるもの:** チャンネルごとに 1 次式 `E_keV = kev_per_adc × energy + offset_kev` の 2 つの係数。方法(何本の線で決めるか、フィットの道具)は任せる。
2. **`energy` とは:** hits tree `delila` の枝 **`energy`**(`UShort_t`、生の ADC)。root_sink / nwall_sink が書いた `runNNNN_*.root`(`hits_tree = true` のとき)でも、`.delila` から `delila2root` で作った `.root` でも、同じ名前・同じ値。events tree なら `HitEnergy[NHits]` が同じ量。PHA(V1725 の Si)は台形の高さで 0〜32767、PSD(VX2730 の LaBr3 / LS)は長ゲートの電荷で 0〜65535。**`energy_short` ではない。** `--time-calib` の有無はエネルギーに関係ない。
3. **チャンネルの指定:** 枝 `module` と `channel` の値そのもの(`module` は TOML の `[[network.sources]]` の `module_id`、無ければ `id`。N_Wall のラン 1〜4 では VX2730 が module 1、ラン 5 以降は module 0 — `config_nwall_pfqaq.toml:12-13`。較正に使ったランの `.root` で見ること)。
4. **返すもの:** 1 ch につき 1 行、`module channel kev_per_adc offset_kev` の 4 列だけ。それ以外は要らない。
5. **較正できなかった ch:** **行を書かない**(0 や NaN を書かない。傾き 0 はファイルごと拒否される)。書かなかった ch は画面に ADC のまま「ADC」と印がついて出る。
6. **較正を変えたいとき:** 新しい行を渡す。古い行と入れ替えるのは DAQ 側。

### 5.2 受け取った表の確かめ方(新しい道具は無し)
1. `energy_calib_<exp>.txt` を一時ファイルに書いて `mv`。
2. Monitor の **Reload calibration** → 1 行の状態表示に `N channels, loaded ...` が出ればパースは通った。`STALE: <行番号: 理由>` なら直す(前の表のまま)。root_sink 側は `/ReloadCalib` → `/EnergyCalib` と状態行 `ecal=N`、起動していれば起動行。
3. keV の View で既知の線(γ なら ⁶⁰Co など、Si なら α など、手元の線源)にフィット → center が期待の keV に来るか、FWHM が妥当か。nwall_sink なら `/events/pid_keV` のバナナが sector をまたいで重なるか。
4. 列の取り違えや別のランの係数は、ここで初めて見つかる。

## 6. プログラムごとの振る舞い

### 6.1 Monitor バックエンド(Rust、`src/monitor/energy_calib.rs` 新規)
- TOML `[network.monitor] energy_calib_file = "config/energy_calib_nwall.txt"`(`MonitorNetworkConfig`、省略可)。相対パスは Monitor の作業ディレクトリから(他のファイルと同じ)。
- 状態 `ActiveCalib { table, source_path, loaded_at, status: Ok|Stale|Error|None, errors }` を `Arc<RwLock<_>>` で REST と `on_start` が共有する。ヒストグラムのタスク(`MonitorState`)には**一切渡さない**(F1 の fill は変えない)。CLAUDE.md の「タスク間で Mutex を共有してブロックしない」はデータ経路の話で、これは数秒に 1 回読む設定値(Operator の `monitor_layout: RwLock<Value>` と同じ)。
- **REST(Monitor に置く。UI は Operator を経由しない、F16)**
  - `GET /api/calibration` → `{"configured":true,"path":"...","status":"ok","unit":"keV","loaded_at":"2026-11-06T10:21:03Z","entries":[{"module":5,"channel":0,"kev_per_adc":0.3012,"offset_kev":-12.4}],"errors":[]}`。未設定なら `{"configured":false,"status":"none","entries":[],...}`。STALE なら `status:"stale"` + 直近の `errors`(行番号つき)+ 保持している表。コメントは返さない(表示に要らない)。
  - `POST /api/calibration/reload` → 読み直して同じ形を返す(失敗は 200 で `status:"stale"|"error"` を返す。設定の誤りは HTTP エラーではなく状態)。未設定なら 404。
  - 既存の `GET /api/histograms*` は**変えない**(値は ADC のまま)。Monitor の REST には Swagger が無いので載せない。
- **ログ:** 起動時 `energy calibration: <realpath> (N channels, keV)` / `energy calibration: none (histograms in ADC)` / `ERROR energy calibration: <path>: <errors> — starting WITHOUT calibration`。読み直し成功で `info!`、失敗で `error!`(`STALE, previous table kept`)。表に無い ch の警告はバックエンドでは出さない(どの ch が表示されるかを知らない)。
- `.unwrap()` なし。ファイルが無い・読めない・不正はすべて `errors` に文章で。

### 6.2 Monitor フロントエンド(Angular)
- `HistogramService` に `calibration` シグナル(`GET /api/calibration` を 5 秒ごと)と `reloadCalibration()`。
- `utils/energy-calib.ts`(純関数、spec つき): `LinearCalib {kevPerAdc, offsetKev}`、`toKev`、`calibrateHistogram1D(hist, calib)`、`calibrateHistogram2D(hist, xCalib|null, yCalib|null)`、`lookup(calibration, module, channel)`。`kevPerAdc <= 0` や非有限は「係数なし」と同じに扱い `console.error`(バックエンドが弾くので通常来ない)。
- `view-tab` と `expand-dialog`: タブが keV で、そのセルの ch に係数があれば、チャートとフィットに渡す前に `calibrateHistogram1D/2D` を通す。2D は `xAxis === 'energy'` / `yAxis === 'energy'` の軸だけ。
- `histogram-chart`: 入力 `unit: 'ADC' | 'keV'`(既定 `'ADC'`、Tune Up 画面はこの既定で不変)と `calibrated: boolean`。軸名・ツールチップ・フィット文字列に単位。keV タブで `calibrated=false` なら軸名 `ADC (not calibrated)`。
- `heatmap-chart`: ラベルの丸めを bin 幅から決める(`formatAxisValue`)。
- `setup-tab`: セレクタを `ADC` / `keV` の 2 択(名前は `Unit`)。energy の 1D と、energy の軸を持つ 2D で出す(PSD・UserInfo では出さない)。Quick Create の行にも出す(既定は ADC)。keV のとき Range の Min/Max は空欄可(= 各セルの全範囲)、単位の表示 `[keV]`。単位を変えると入力済みの範囲は消す(bin 数は残す)。
- `view-tab` ツールバー: 「較正済み n / m」、keV タブで係数が 0 件なら `ADC` バッジが全セルに出る。
- Monitor ページ: 状態 1 行 + 「Reload calibration」ボタン + 係数が変わったときの snackbar(keV タブのフィットを消した旨)。
- 保存形式の変更なし。`'MeV'` → `'keV'` の移行は `applyState`。
- `npm run build` して `web/operator-ui/dist/` を同じコミットに入れる。

### 6.3 nwall_sink
- `--energy-calib FILE`(TOML `energy_calib_file`、`start_daq.sh` の `rs_add_arg`)。usage に 1 行。
- 起動: 読めたら `nwall_sink: energy calibration <realpath> (N channels, keV; axes: energy_keV_m0 0..14500, pid_keV E 0..9900 dE 0..9700, ...) — display only, re-read at every run start and /ReloadCalib`。失敗なら `ERROR ... — display in ADC only` で起動は続ける。指定なしなら `energy calibration: none`。
- Display: `/hits/energy_keV_m<M>`、`/events/pid_keV` を最初のヒットで、表から範囲を決めて作る(§3.5)。RunOpen と `/ReloadCalib` で読み直し、係数が変わったら keV のヒストグラムを作り直す。失敗は前の表を保持して STALE。
- 表に無い ch のヒット: keV には詰めない。RunClose で 1 行 `energy calibration: no coefficients for 79 channel(s) with hits, ADC histograms only: m0c0 m0c1 ...`。
- 状態行(10 秒ごと): `| ecal=off|96|96 STALE|96 (3 unmapped)`。JSROOT に文字列項目 `/EnergyCalib`(`/WriteStatus` と同じ作り): パス、件数、読み込み時刻、STALE/ERROR とその行番号、keV 軸の範囲。
- `events` tree・hits tree・`BeamTime`・provenance・sidecar: **変更なし**。

### 6.4 root_sink(standard)
- 同じ `--energy-calib FILE`、同じ起動行・状態行・`/EnergyCalib`・`/ReloadCalib`。
- `hist_config.hpp`: `Var::EnergyKeV`(名前 `energy_kev`、hit スコープ)。`value_of` は Display が持つ表を引く(表の参照を `fill_hit_hists` に渡す。`LiveHist` は変えない)。係数なしは false。
- `--energy-calib` なしで `energy_kev` を使う定義は、起動時に WARN(「この変数は 1 件も fill されない」。`warn_dropped_specs` と同じ流儀、`root_sink.cxx:1465`)。
- `_hists.json` sidecar の仕組みは変更なし(定義ファイルの写しなので、`energy_kev` を使っていればそれが写る。較正ファイルは写さない、§3.9)。

## 7. 記録経路に触れていないことの確認方法
1. **コードの境界:** `energy_calib.hpp` を include するのは `nwall_sink.cxx` と `root_sink.cxx` の Display の部分と `hist_config.hpp` だけ。`eb_core.hpp`・`event_def.hpp`・`sink_core.hpp`・`time_offsets.hpp`・`Recorder`・`Writer`・`Sorter` の diff が無いことをレビューで見る(`git diff --stat`)。Rust 側は `src/monitor/energy_calib.rs` と `src/config/mod.rs`、REST だけ。`src/reader`・`src/recorder`・`src/merger`・decoder・`src/common/mod.rs` の diff は無い。
2. **E2E(Mac、`test_publisher` で同じ `.delila` を 2 回流す):** `--energy-calib` あり / なしで、hits tree は `compare_hits.C` で IDENTICAL、events は `check_events.C` で IDENTICAL、sidecar と UserInfo は `diff` でバイト一致。Monitor は REST の `GET /api/histograms/0/1` の JSON が較正あり / なしで一致(bin も config も)。
3. **実機(§10):** 同じランの `.delila` を `delila2root` に掛けた energy の分布が、root_sink の ADC ヒストグラムと一致し、keV のヒストグラムは `kev_per_adc` 倍に伸びているだけ。

## 8. テスト(TDD、先に書く)

### 8.1 Rust(`src/monitor/energy_calib.rs`、`cargo test`、clippy `--tests` も通す)
| テスト | 内容 → 期待 |
|---|---|
| 正常 | §4 の例 → 3 件、`lookup(5,0) = (0.3012, -12.4)`、`lookup(1,1) = None` |
| コメント・空行 | 行末 `#`、行頭 `#`、空行、タブ区切り → 同じ結果。コメントだけのファイル → OK、0 件 |
| 列数 | 3 列・5 列の行 → error(行番号つき)。`#` でコメントにした行は数えない。CRLF の行は OK |
| 値 | `kev_per_adc` 0 / 負 / `nan` / `inf` / `abc` → error。`offset_kev` 欠落(3 列)/ `inf` → error。module 256 / −1 / 1.5 → error |
| 重複 | 同じ (m,c) 2 回 → error(両方の行番号を報告) |
| 全部集める | 悪い行 2 つ → error 2 つ、どちらにも行番号 |
| 読み直し | 正常 A → 不正(A 保持、stale、errors)→ 読めない(A 保持)→ 正常 B(採用、stale 解除) |
| 起動時失敗 | 無いパス → `status: Error`、表は空、Monitor は起動する(`MonitorState` のテストが影響を受けない) |
| REST の形 | JSON の形、`configured:false` のとき `entries` は空、`POST reload` が 404(未設定)、壊れたファイルは 200 で `stale` |
| `on_start` | 一時ファイルを書き換えて Start を 2 回 → 2 回目に新しい表 |
| 不変 | 較正を読んだ状態で `process_event` → `Histogram1D` の `config` と bin は従来のテストと同じ値(F1 を守る) |
| 設定 | TOML の `energy_calib_file` 省略で `None`、相対パスがそのまま |

### 8.2 C++(`tools/root_sink/common/test_energy_calib.cpp`、`CHECK` 方式、`g++ -std=c++17`)
- 8.1 の parse / reload のケースと同じもの(**同じ表のテキストを使う**: 固定ファイルを `tools/root_sink/common/fixtures/energy_calib/*.txt` に置き、Rust は `include_str!`、C++ はパスで読む。TODO 70 E9 と同じく両言語の解釈が一致することを 1 つのテストで見る)。
- `to_kev(100; 0.5, -10) == 40`。
- `axis_limits_kev(table, channels, adc_max)`: `adc_max` 32768 と 65536 で上限が `kev_per_adc × adc_max + offset_kev` の最大、下限が `min(0, 最小 offset)`、対象 ch に係数が 1 つも無ければ「範囲なし」。
- nwall Display(ROOT なしで試せるように、fill する値を返す関数に分ける): 係数ありの ch と無しの ch のチャンク → keV の値は係数ありだけ、`unmapped` の報告はランごとに 1 回、`clear()` で再び報告。
- **`pid_keV` の最大の選び方:** E sector に 2 ヒット、ADC では ch A が大きいが keV では ch B が大きい(ゲイン差)→ ADC の `pid` は A、`pid_keV` は B。係数の無い ch は候補から外れる。dE 側も同じ。
- hist_config: `energy_kev` が `Var::EnergyKeV` / Hit スコープになる、`x: energy_kev, y: channel` の 2D が通る、係数なしで `value_of` false、`energy_kev` と coinc 変数の混在は error(既存規則)。
- 範囲の再計算: 表 A → 表 B(上限が変わる)→ 「作り直しが必要」の判定が true、同じ表なら false。

### 8.3 Angular(karma、`utils/energy-calib.spec.ts` ほか)
- `calibrateHistogram1D`: config の min/max が写像され、bins と counts は同一参照のまま、`null` で恒等。`kevPerAdc <= 0` は恒等 + `console.error`。
- `calibrateHistogram2D`: energy の軸だけ変わる(x のみ、y のみ、両方、どちらも無し)。
- フィット: `generateGaussian` で ADC center 500 / sigma 20 を作り、係数 (0.5, −10) で変換したヒストグラムに `fitGaussian` → center 240、sigma 10(既存 spec の道具を使う)。
- `histogram-chart`: keV タブ + 係数あり → 軸名とツールチップに `keV`。keV タブ + 係数なし → `ADC (not calibrated)`、**文字列 `keV` を含まない**(本チケットの核心のテスト)。ADC タブ・入力省略 → `Channel`/`ADC`(Tune Up 画面の不変を固定)。
- 移行: `'MeV'` → `'keV'`、`'Channel'` と `'keV'` は不変。
- 係数変更: 値が変わると keV タブの `fitResult`/`fitRange` が消え、`xRange` は残る。単位が変わる ch は `xRange` も消える。同じ値の読み直し(`loaded_at` だけ違う)では何も変わらない。ADC タブは何も変わらない。

## 9. フェーズ(早く価値が出る順)

| Phase | 内容 | 出るもの |
|---|---|---|
| **0** | 形式(§4)、例ファイル(例と明記)、Rust と C++ のパーサ + 読み直し方針 + テスト(8.1、8.2 の parse)、fixture、§5.1 の文面を `tools/root_sink/README.md` と `docs/root_sink_manual.md` に | ファイルが読める、解析する人に渡す文書がある |
| **1** | **Monitor**: バックエンド(TOML、`GET/POST /api/calibration`、`on_start`)、フロント(2 択セレクタ、1 か所の変換、ADC バッジ、単位つきツールチップ / フィット、ヒートマップのラベル、状態 1 行、Reload、`MeV` 移行)、`dist/` 再ビルド。旧設計書に置き換えの注記、F7 の issue | **「keV と書いて ADC を出す」罠が消える。** 受け取った表で keV 表示 |
| **2** | **nwall_sink**: `--energy-calib`、`energy_keV_m<M>`、`pid_keV`(keV で最大を選ぶ)、RunOpen / `/ReloadCalib`、`/EnergyCalib`、状態行、E2E(§7-2) | N_Wall の dE-E が keV で見える |
| **3** | **standard root_sink**: `energy_kev` 変数、ELIADE の `histograms_sn01.json` に例を 1 つ | HPGe を keV で |

- N_Wall の開始に間に合わせる必要は無い(ユーザー: あれば嬉しい)。Phase 1 だけでも独立して価値がある。
- 各 Phase の終わりに `cargo fmt && cargo clippy --tests -- -D warnings && cargo test`、C++ の単体テスト、`npm run build`。

## 10. 実機・実データでの確認(メインセッションとユーザーが行う。サブエージェントは実機を触らない)

**pf-qaq への配備(2026-11-06 10:24 EET、ユーザー承認、Idle、Tune Up なし):** `git merge --ff-only origin/master`(db13b73、ローカルの設定変更と重なりなし)→ `cargo build --release --bins --features x743`(45 s、警告 0)→ `/tmp` で `nwall_sink`、`root_sink` をビルド(警告なし)→ 入れ替える前に pf-qaq の上で単体テスト(94、67、121)と合成データの確認(別ポート、`compare_hits` と `check_events` IDENTICAL、`energy_keV_m5` 3,940、`pid_keV` 3,939)→ `nwall_sink` と `monitor` だけ SIGTERM で止め、`/proc` から取った元の引数と環境で起動(ログは同じファイルに追記)。Reader、Merger、Recorder、Operator の PID は変わらず、Operator は Idle のまま。古いバイナリは `~/.local/bin/nwall_sink.before_ecal`、`root_sink.before_ecal`。UI は `dist/` をファイルで配信するので Operator の再起動は不要。

確認: `GET :8081/api/calibration` = `configured:false`、`:8090/EnergyCalib` = `none`、Monitor のページに `Energy calibration: none configured`、保存済みの 10 タブはすべて ADC。再起動で表示のヒストグラムは 0 に戻った。**受け口(同日 10:27 EET、188c067):** 空の表 `config/energy_calib_nwall.txt` を追加し、`config_nwall_pfqaq.toml` の 2 か所で `energy_calib_file` を有効にして、`monitor` と `nwall_sink` をもう一度再起動した(Monitor と sink がパスを読むのは起動時)。**これ以降は、表を `mv` で置いて Reload を押すだけで、再起動は要らない。** Idle の本番プロセスで確認: 2 行の表 → Monitor と sink とも `2 channels`、壊れた表 → `stale` と行番号で前の表を保持、元に戻すと `0 channels`。最後は空の表に戻してある。学生の表を置くと、その `config/energy_calib_nwall.txt` は pf-qaq のローカル変更になる。

| # | いつ | 確認 |
|---|---|---|
| V1 | Phase 1 後、pf-qaq(または手元のエミュレータ) | 較正ファイルなしで Monitor を起動 → keV の View の全セルに `ADC` バッジ、軸名に `keV` が**無い**。状態 1 行が `none configured`。Tune Up 画面のヒストグラムは今までどおり |
| V2 | 同 | 2 ch だけ書いた表を置いて `Reload calibration` → その 2 セルだけ keV、残りは `ADC`。ツールチップとフィットの単位 |
| V3 | 同 | 手元にある既知の線(ELIADE の ⁶⁰Co、N_Wall で使う線源)でフィット → center が期待の keV に、FWHM が keV で妥当(SN01 の 3.4〜4.8 keV @1332 の記録と比べられる) |
| V4 | 同 | 表を壊して Reload → `STALE` と行番号、表示は前のまま。直して Reload → 戻る。ランを 1 本回して `on_start` で読み直されることをログで見る |
| V5 | 同 | `GET /api/histograms/<m>/<c>` の JSON が較正の前後で同じ(§7-2) |
| V6 | Phase 2 後、pf-qaq、ランとランの間に差し替え | `nwall_sink` の起動行に実パスと keV 軸の範囲。JSROOT に `/hits/energy_keV_m5` と `/events/pid_keV`。ADC のヒストグラムは今までどおり。状態行 `ecal=`。パルサーの ch が `unmapped` として 1 回だけ出る |
| V7 | 同 | そのランの `.delila` を `delila2root` に掛け、`compare_hits.C` IDENTICAL、`check_events.C` IDENTICAL、`_timeSettings.json` が前のランと同じ形(§7-3) |
| V8 | 同 | `/ReloadCalib` で範囲が変わる編集 → keV のものだけ作り直されてログに出る、ADC のものは積算のまま。`pid_keV` のバナナが sector をまたいで重なる |
| V9 | Phase 3 後、ELIADE(SN01 など、ELIADE チームの都合に合わせて) | `histograms_sn01.json` に `energy_kev` の 1 枚を足して ⁶⁰Co の 2 本が 1173 / 1332 keV に |

## 11. リスク
- **keV 軸の範囲を表から導くので、1 ch の係数が桁違いだと軸が伸びる**(nwall_sink)。起動行と `/EnergyCalib` に範囲を出すので気づける。JSROOT でズームは可能。
- **列の取り違え**(§4 末尾)は検証で全部は捕まらない。§5.2 の 3 番(既知の線)を必ずやる。
- **ヒートマップのカテゴリ軸**(F6)は値ではなくラベルなので、keV のラベルの丸めを間違えると隣の bin と同じ文字列になる。bin 幅から桁数を決めるテストを 8.3 に入れた。
- **古い `'keV'` のレイアウト**が、係数のある ch で突然 keV の軸になる(今まで ADC だった)。これは意図した修正だが、ユーザーには Phase 1 の配備時に一言伝える。
- **フィット結果の消去**(係数変更時)は親切のつもりが驚きになりうる。snackbar で理由を出す。嫌なら消さない方針に戻せる(1 か所)。
- **Monitor の `RwLock`** は REST と `on_start` の間だけで、ヒストグラムのタスクには触れない。レビューで `histogram_task` に lock が入っていないことを見る。
- **Tune Up 画面**は `histogram-chart` を共有している(`waveform.component.ts:348`、`xAxisLabel` は渡していない)。新しい入力の既定値を `unit='ADC'` にすれば今までどおり。8.3 と V1 で固定する。

## 12. ユーザーの回答(2026-11-06)

| # | 問い | 回答 |
|---|---|---|
| Q1 | 解析する人に頼む形 | **4 列の表だけ。「それ以外は不要」** → 5 列目以降の読み飛ばしと、行末コメントのお願いをやめた(§4、§5.1)。`#` コメントは、置く人のために読み飛ばしたまま |
| Q2 | 単位 | **keV。「今までもそうしてきたので」**(§3.7) |

§3 のそれ以外の決定(係数なし ch は ADC と明記して表示、反映は起動時 + ラン開始 + 要求時、standard root_sink の `energy_kev` も範囲内、保存済み keV/MeV View は本物の keV として読む、nwall の keV 軸は ADC 軸の上限を写像、記録ファイルには何も足さない、`P0/p0` は別 issue)は、提示して異論なし。

## 13. 実装の結果(2026-11-06、Mac、実機なし)

**単体テスト:** Rust 808(`src/monitor/energy_calib.rs` と REST、`on_start`)、Angular 100(`utils/energy-calib.spec.ts`、`histogram-chart.component.spec.ts`)、C++ `test_energy_calib` 94、`test_hist_config` 121、`test_nwall` 67、`test_sink_core` 647、`test_eb_core` 232。パーサは Rust と C++ で同じフィクスチャ(`tools/root_sink/common/fixtures/energy_calib/`)を読む。clippy `--tests` と eslint は警告なし。

**Monitor(エミュレータ + Merger + Recorder + Monitor + Operator、REST でラン、ヘッドレス Chrome):**
- keV のタブで係数のある ch は keV の軸、ない ch は `ADC` バッジと `ADC (not calibrated)`。ゲインの違う 2 ch がそれぞれの軸になる。
- 拡大ダイアログのフィットが keV で出る(ADC 500 のピークが 239.9 keV、係数 0.5 / −10)。
- 2D は `Energy [keV]` と `Energy [ADC, not calibrated]`。PSD のタブに keV は出ない。保存済みの `MeV` は keV として読む。
- 壊れた表 → `STALE` と行番号、表示は前のまま。別の表 → 単位が変わった ch のズームとフィットが消える。ラン開始で読み直す(ログ)。

**nwall_sink / root_sink(`make_test_run.C` の 273,824 hits を `test_publisher` で再生):**
- 較正あり / なしで `compare_hits.C` IDENTICAL、`check_events.C` IDENTICAL、サイドカーはバイト一致。
- `energy_keV_m5` 3,940 件(0〜21,614 keV)、`pid_keV` 3,939 件 = ADC の `pid` と同じ件数。`e_m5c0_kev` は `e_m5c0_adc` と同じ 250 件。
- 同じ値の `/ReloadCalib` はヒストグラムを保つ。値が変わると keV だけゼロから、ADC は積算のまま。壊れた表は `STALE`。

**設計から変えたところ:** `version` / `loaded_at` でなく係数の値で変更を判定(ラン開始でフィットが消えないように)。Quick Create と energy 軸の 2D にも単位の選択を追加。係数が変わったら範囲が同じでも keV のヒストグラムを作り直す。ch ごとの WARN はやめてランの終わりに 1 行。

**実データでは未確認**(§10、V1〜V8 はこれから)。

## 14. Monitor の Gaussian フィットの修正(2026-11-06、ユーザー依頼)

**直したこと(`fitting.service.ts`):**
- **誤差が常に 0 だった。** `result.parameterError` を配列として読んでいたが、`ml-levenberg-marquardt` は残差の 1 個の数値を返すので `[0, 0, ...]` に落ちていた(e8ff0db から)。Jacobian から共分散を計算するように変えた。フィットは重みなしの最小二乗なので `Cov = J⁺ V (J⁺)ᵀ`、`V` = 期待カウント。`(JᵀWJ)⁻¹` は重みつきフィットの式で、この推定量の誤差にならない。Area の誤差は A と σ の相関を含める。
- **Area が keV 軸でずれていた(本チケットで持ち込んだ不具合)。** `A·σ·√2π` は bin 幅 1 が前提で、keV タブでは `kev_per_adc` 倍になっていた。bin 幅で割り、どの軸でもカウントにした。
- **低カウントでノイズのスパイクに収束していた(もとから)。** 初期値の幅を 1 ビンのノイズから決めていた。生と平滑化(範囲の 1/20)の 2 つから始めて、ポアソン尤度の良いほうを採る。候補の選択に重みなしの残差を使うと、背の高いスパイクが選ばれる。
- **分解能を追加:** `Resolution = FWHM / Center`(%、誤差なし、ユーザー指定)。Center が 0 以下なら `---`。表示はすべて小数 2 桁、誤差が計算できないときは `---`。

**測定(2026-11-06、Mac、ヘッドレス Chrome、1 ADC ビン、乱数で作った Poisson ヒストグラム各 40〜150 個、範囲はピーク 1 個):**

| A(カウント/ビン) | σ | bg | 成功 修正前→後 | σ 中央値 修正前→後 | center の散らばり / 誤差 | σ の散らばり / 誤差 | Area 中央値(真値) |
|---|---|---|---|---|---|---|---|
| 500 | 10 | 20 | 150/150 → 150/150 | 10.00 → 10.00 | 0.125 / 0.131 | 0.120 / 0.130 | 12550 (12533) |
| 100 | 20 | 5 | 300/300 → 40/40 | 19.98 → 19.84 | 0.465 / 0.413 | 0.304 / 0.405 | 5012 (5013) |
| 30 | 40 | 1 | 147/150 → 150/150 相当 | 39.56 → 39.97 | 1.133 / 1.090 | 0.765 / 1.108 | 2993 (3008) |
| 12 | 40 | 1 | **105/150** → 40/40 | **18.05** → 40.69 | 1.675 / 1.814 | 1.753 / 1.861 | **221 → 1200** (1203) |
| 3 | 40 | 0.2 | — → 40/40 | — → 38.74 | 2.941 / 3.449 | 3.418 / 3.568 | 297 (301) |
| 200(範囲全体) | 5 | 1 | — → 40/40 | — → 5.01 | 0.129 / 0.126 | 0.093 / 0.109 | 2511 (2507) |

(修正前の列の誤差は常に 0 なので比べていない。修正後の 100/20/5 と 12/40/1 は最終版の 40 個、500/10/20 と 30/40/1 はスタートが 1 本の段階の値で、この範囲では同じ解になる。)

**既知の制限:**
- **background が厳密に 0 のとき、σ と Area の誤差は約 3 割小さく出る**(A=100 σ=20: σ の散らばり 0.537 に対して誤差 0.391、Area 146 に対して 91。center は正しい)。0 での打ち切りでフィットが非線形になるため。線形の誤差伝播では出ない。
- 重みなしの推定量なので、center の誤差は `σ/√N` の 1.4〜1.6 倍になる。これは誤差の計算でなくフィットの性質。重みつきにすれば `σ/√N` に近づくが、低カウントで偏るので変えていない。
- 所要時間: 2,000 ビンの範囲で 243 ms、20,000 ビンで 2.9 s、65,536 ビン全部で 10.5 s(この間 UI は止まる)。ピーク 1 個に範囲を絞って押す使い方(ユーザー)なら問題ない。

## 15. 仮定(コードを読んだだけで、実行していないもの)
- F7 の「UI が作った chSettings を Rust が読むと `p1 = 0.0`」は serde の既定(未知キー無視、`default`)からの推論。実行では確かめていない。
- §4 の例の数値は形式を示すためのもので、どの検出器の測定値でもない。
- N_Wall で較正に使う線源はユーザーが決める(チケットでは特定していない)。
