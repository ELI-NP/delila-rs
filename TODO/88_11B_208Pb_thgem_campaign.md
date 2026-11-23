# TODO 88 — 11B_208Pb 実験(ThGEM × 8 + 中性子、2026-11-22〜28)の準備と ThGEM 専用 sink

**Status: 🚧 本番中 (11-21 22:46〜、run 25 まで)。11-23: reader の segfault 3 回(CAEN ライブラリ、無効チャンネルのあるボードだけ)→ 全チャンネル有効・Operator の LINGER=0・guard が無応答の reader で止める・reader のコアダンプ (§5.11、§5.13)。コミット済み 2026-11-23 — 残 = run 17 の ROOT を .delila から作り直す、Recorder の EOS 後の 1,617 イベント、ThGEM の CFD 調整 (TODO 90)**

**改名 (2026-11-20): 実験名 `11B_204Pb` → `11B_208Pb`。** 標的は 208Pb(204Pb は来年の春〜初夏の別の実験なので、名前が残ると混乱する。ユーザー)。
本番のランが 1 つもないうちに変えた(pf-qaq の `/data/11B_204Pb` にあったのは Tune Up の `run0000_*` だけ)。
この文書は 11-17 からの記録も含めて新しい名前に書き換えてある。当時の実際の名前は `11B_204Pb`。
- [x] リポジトリ(Mac): ファイル名 12 個(この TODO、TOML、デジタイザ JSON × 5、対応表、イベント定義、較正表、Grafana のフォルダ、`docs/detector_positions_11B_208Pb.drawio`)と本文。TOML 2 本が読め、参照先が全部あること、`test_thgem_sink` 86 件を確認
- [x] pf-qaq (11-20 16:05〜16:10、DAQ は Configured・レート 0・Tune Up でない状態で停止): `config/` の 9 ファイルを**その場で改名**(Mac から上書きしていない。デジタイザ JSON 5 つは改名前とバイト単位で同じ)、TOML 2 本・対応表・イベント定義・較正表・`start_daq.sh` のコメント・`elogd_thgem.cfg` の中の名前を置換。`/data/11B_208Pb` と `/archive/DELILA_data/11B_208Pb` を作成、旧 2 ディレクトリは削除(中身は Tune Up の `run0000_*` 112 個だけ、ユーザー了承)。archive_sync の cron 行(LOCK・LOG・TAG も `11b208pb`)。eLog: 空だった旧ログブックのフォルダを消して再起動、`/11B_208Pb/` が 200、旧名は 404。Grafana: フォルダを改名、`Rates — 11B_208Pb`(uid `rates-11b-208pb`、5 ボード 96 ch)を作り直し、reload、空になった旧フォルダを API で削除。DAQ を起動(ログ `logs/20261120_160751/`、設定は DAQ_Test のまま)→ Configure(ラン 14 として): 8 コンポーネント Configured、5 台とも PLL locked、Operator が読んだデジタイザ設定は改名前と同じ。変更前の控えは pf-qaq の `~/rename_backup_20261120/`
- [x] side3 (11-20): data_pull の cron 行(SRC・DST・LOCK・LOG・TAG)、`/data/9MV2026/11B_208Pb` を作成、旧フォルダは削除(pf-qaq から引いた同じ `run0000_*` 112 個だけ)。crontab の控えは `~/crontab.bak.20261120`
- 触っていないもの: InfluxDB と MongoDB。11-18〜20 に旧名で入った記録があればそのまま残る(未確認)

**関連:** [73_campaign_2026_11_pfqaq.md](73_campaign_2026_11_pfqaq.md)(11 月キャンペーンの親。これが 2 本目の「ThGEM + 中性子」) /
[79_root_sink_nwall_online.md](79_root_sink_nwall_online.md)(nwall_sink、部品の出どころ) /
[71_generic_event_building.md](71_generic_event_building.md)(イベント定義) /
[70_root_sink_time_offsets.md](70_root_sink_time_offsets.md)(XY 中心の時刻オフセット) /
[72_digitizer_time_offset_calibration.md](72_digitizer_time_offset_calibration.md)(ボード間のずれ) /
[74_grafana_event_rate_monitor.md](74_grafana_event_rate_monitor.md)(対応表の書式) /
[84_high_rate_investigation.md](84_high_rate_investigation.md)(レートの上限)

---

## 1. 決まったこと(ユーザー、2026-11-17)

| 項目 | 内容 |
|---|---|
| 日程 | **2026-11-22〜11-28**。準備に使えるのは 5 日 |
| `experiment_name` | **`11B_208Pb`**(テスト用の名前ではない。ラン番号は 1 から) |
| 検出器 | **ThGEM × 8 台 = 40 ch**(1 台 5 ch: トリガー + XL / XR / YU / YD)。N_Wall の Si を ThGEM で置き換える形。セットアップは一から組み直す(10 月の ThGEM とは全部変わる) |
| ボード番号 | **VX2730 = module 0(マスター)**、ThGEM のボードが 1 から続く(N_Wall と同じ形) |
| デジタイザ | **DPP-PSD**。V1730 が使えればそれ、だめなら V1725(未定) |
| 時刻合わせ | **セットアップ時にパルサーを入れ、ボード間の時間差を最小にする**(N_Wall と同じ、TODO 72 の手順)。ラン中もパルサーを入れるかは未定 |
| レート | N_Wall と同程度。**波形は取らない** |
| スローコントロール | ThGEM チームが用意済み。DELILA 側は**レートと PMT の HV 監視**(SY5527)だけ |
| オンライン表示 | **nwall_sink とは別に、ThGEM 専用の sink を作る**(§3) |
| デジタイザ(同日、§7 の回答) | **V1730 × 3 枚が多分使える**(使えなければ V1725 の PSD) |
| VX2730 | **N_Wall のまま**: ch0 = ビーム 1/100、ch1〜7 = LaBr3、ch8〜31 = 液シン |
| dt ペアの基準 | **RF のバンチ**(ビーム 1/100 から復元) |
| イベントの窓 | **前 500 / 後 1500 ns**(N_Wall と同じ)。もっと短くてもよさそう(ユーザー)。イベント定義はラン開始ごとに読み直すので、実験中に再起動なしで変えられる |
| ROOT の転送 | **side3 へ運ぶ**(N_Wall と同じ `scripts/data_pull.sh`、§5) |
| eLog | ログブック名 = **`11B_208Pb`**(実験名と同じ) **✅ 11-17: 追加して eLog を再起動、`/11B_208Pb/` が開く** |

## 2. ボードとチャンネルの割り当て(案)

> **11-20: 最終の配線表で確定 → §5.8。** ThGEM の位置はこの案のまま。下は当時の案の記録。

- **1 台の ThGEM の 5 ch は同じボードに入れる。** XY は腕どうしの時間差なので、同じボードの中ならボード間のずれと
  ラン開始時の 8 ns の跳び(TODO 77)の影響を受けない。
- 16 ch のボードなら **ThGEM 3 台(ch0〜14)+ ch15 = パルサー**。3 枚で 9 台分(使うのは 8 台、5 ch 空き)。
  最後の ch をパルサーにするのは TODO 72 で決めた方針。
- 同期スタートは N_Wall と同じ数珠つなぎ(VX2730 の TRG-OUT → S-IN)。N_Wall では TRG-OUT = Run で
  V1725 どうしの差が 0.03 ns 以内で一定だった(TODO 77 §6.12)。
- 時間精度: 10 月の XY 像は V1730(500 MS/s)の PSD と CFD で取った。V1725(250 MS/s)の PSD で同じ精度が出るかは
  確かめていない → V1725 になったら、セットアップ時にパルサーで時間の幅を測る。
- PSD1 の既知の設定: `cfd_interpolation_point = 0`(1 だと 2 ns の櫛)、Charge Pedestal = TRUE
  (E = 65535 が負の電荷の印になるのを避ける)。波形を取らないので record length は短く、DIG1 のカップル アービターの件
  (record length が長いと、カップル内で同時のイベントが片方消える)には当たらない。
- 実際の割り当てはセットアップ時にユーザーが決める。sink は役割を対応表から読むので、コードは割り当てに依存しない。

## 3. ThGEM 専用 sink(`thgem_sink`)

### 3.1 方針

- 新しいプログラム `tools/root_sink/thgem/thgem_sink.cxx`。**nwall_sink には手を入れない**(実験ごとに別のプログラム、
  TODO 79 §5.1)。
- **nwall_sink からそのまま使うもの:** 受信・ソート・イベント構築・記録(hits と events の 2 ツリー、書き込み失敗時の処理、
  時刻オフセット、ランごとの来歴)、`BeamTime` と `rfPeriodNs`、RF のバンチ時刻(`bunch_clock.hpp`)、
  `/hits`・`/rf`・`/ls` の表示、keV 表示(`--energy-calib`)。
- **外すもの:** Si 用の `/events`(pid、pid_pair、sector_dE_vs_E、dE_ring_vs_sector、si_phase、ls_dt_si、ls_tof_si、pid_keV)。
- **足すもの:** ThGEM の表示(§3.3)、対応表の ThGEM の役割(§3.2)、8 台分の XY 再構成
  (今の `thgem_reco.hpp` は 2 台に固定。腕の選び方は同じ規則 = 自分のトリガーに最も近い hit、±窓、`nearest_in`)。
- イベント定義: トリガー = **どれかの ThGEM のトリガー**(8 個のタグ)。窓は §7 で確認。

### 3.2 対応表

`config/channel_map_11B_208Pb.txt`。1〜4 列目は Grafana と共用(TODO 74 §3.2)、5〜7 列目が sink 用の役割:

| 列 | 値 |
|---|---|
| `det` | `beam` / `LaBr` / `LS` / `pulser`(N_Wall と同じ)+ **`ThGEM`** |
| `kind` | ThGEM なら **`T` / `XL` / `XR` / `YU` / `YD`**、ほかは `-` |
| `index` | ThGEM の番号 **1〜8**、ほかは `-` |

- 読み込みは `thgem_map.hpp`(`nwall_map.hpp` の写しに ThGEM を足したもの)。エラーは行番号つきで集める。
- 5 つの役割のどれかが欠けた ThGEM は警告を出し、その台の XY は作らない(トリガーだけの表示は残す)。

### 3.3 表示(v1 案、§7 で確認)

| フォルダ | ヒストグラム | 内容 |
|---|---|---|
| `/hits` `/rf` `/ls` | nwall_sink と同じ | hitmap、energy_m<M>、tof_rf_m<M>、gamma_since_beam(% 800)、since_beam、ls_psd。LaBr3 チャンネルごとの `gamma_since_beam_ch` は E ≥ `gamma_min_adc` だけを入れる。11-22 にユーザーの指示で 4000 → 100 ADC(run 13 と 14 の間)→ **1000 ADC**(run 14 と 15 の間)。TOML の `[network.root_sink]`。どちらも sink だけを再起動して反映した |
| `/thgem/xy` | `xy_T<n>` | 8 台それぞれの XY。x = t_XR − t_XL、y = t_YU − t_YD [ns]、4 本の腕がそろったイベントだけ。±250 ns、500 ビン(10 月と同じ) |
| `/thgem` | `narms` | ThGEM 番号 × そろった腕の数(0〜4)。台ごとの効率の目安 |
| `/thgem` | `tof` | ThGEM 番号 × (トリガー − バンチ)(0〜T)。RF に対する ThGEM の TOF |
| `/thgem` | `pair_hits` | 8 × 8。同じイベントでどの組が一緒に鳴ったか |
| `/thgem/dt_pair` | `dt_T<j>_vs_T<i>` | **10 月の `dt2_vs_dt1`(TODO 65)の 8 台版。** dt = t(ThGEM のトリガー) − バンチ。2 台とも同じバンチを基準にする(イベントのトリガーが選んだバンチ)。28 組すべて、対応表を読んだ時点で空のまま作る(N_Wall の pid_pair と同じ)。±1000 ns、500 ビン(10 月と同じ) |
| `/thgem/ls_tof` | `ls_tof_m0_ch<C>` | 液シン − (ThGEM が選んだバンチ) − rf_t0、**液シン 1 本ずつの 1D × 24**(ユーザー 2026-11-17: 2D より 1D がよい)。対応表を読んだ時点で作る、タイトルは対応表の名前。N_Wall で Si がしていた役 |
| `/thgem` | `ls_dt` | 液シン − トリガー(2D) |

10 月は基準がビーム × ThGEM のハードの同時計数(ch11)だった。今回はビーム 1/100 から復元したバンチを基準にする。

### 3.4 試験

- 単体テスト(ROOT なし): 対応表の読み込み(ThGEM の役割、欠け、重複)、8 台の XY 再構成。
- 合成データでの通し試験: `make_test_run.C` の 8 台版(VX2730 + ThGEM × 8、既知の位置と TOF)を `test_publisher` で流す。
- **合格の条件:** 受け取った hit = 書いた hit、late 0、display_drops 0、`check_events.C` IDENTICAL、
  XY 像が合成データで置いた位置に出る、dt ペアの相関が入れたとおりに出る。

### 3.5 作業量と日程

| 日 | 作業 |
|---|---|
| 11-17〜18 | 対応表(`thgem_map.hpp` + テスト)、sink の写しと Si 部分の除去、ThGEM 表示 |
| 11-19 | 合成データでの通し試験、`start_daq.sh` へのプログラム名の追加、TOML・イベント定義の下書き |
| 11-20 | pf-qaq に配備(DAQ が止まっているとき)。ボードがつながっていれば Configure |
| 11-21 | 試運転: パルサーでボード間の時刻合わせ、ThGEM の信号、XY 中心 |
| 11-22 | 実験開始 |

作業量はおよそ 3 日。ボードの選定と配線(ユーザー)が 11-20 の配備に間に合うかが一番の不確定要素。

### 3.6 間に合わない場合

nwall_sink に ThGEM 用のイベント定義を渡せば、記録と `/rf`・`/ls` の表示はコード変更なしで動く(足りないのは ThGEM の表示だけ)。
注意: `nwall_map.hpp` は `ThGEM` を知らないので、そのときは対応表の ThGEM の行を 4 列だけにする(役割なし)。

### 3.7 実施記録(2026-11-17、Mac、未コミット)

| もの | 内容 |
|---|---|
| `tools/root_sink/thgem/thgem_map.hpp` | 対応表(`ThGEM` + `T`/`XL`/`XR`/`YU`/`YD` + 番号)。エラーは行番号つき、警告 = トリガーなし・腕なし・ボードをまたぐ。対応表とイベント定義のトリガーの食い違いを調べる `trigger_mismatch` |
| `tools/root_sink/thgem/thgem_planes.hpp` | 1 イベントの ThGEM(何台でも)。腕の選び方は `thgem_reco.hpp` の `nearest_in` をそのまま使う |
| `tools/root_sink/thgem/test_thgem_sink.cpp` | 単体テスト 86 件、全部合格(`-Wall -Wextra` で警告なし) |
| `tools/root_sink/thgem/thgem_sink.cxx` | nwall_sink の写し。Si の表示を外し、§3.3 の表示を追加。`--thgem-delay-ns`、`--arm-window-ns`(既定 1000)。対応表とイベント定義が食い違うと起動時とラン開始ごとに警告 |
| `tools/root_sink/thgem/make_test_run_11b.C` | 合成ラン(8 台、向かい合う組の反応 2 kHz、単独 300 Hz/台、液シン・LaBr3・パルサー、答えが分かっている形) |
| `config/channel_map_11B_208Pb.txt`, `config/event_def_11B_208Pb.json` | **下書き**(§2 の割り当て案)。配線が決まったら両方を直す |
| `scripts/start_daq.sh` | `program = "thgem_sink"` を許可、`thgem_delay_ns`・`arm_window_ns` を渡す |
| `tools/root_sink/thgem/README.md` ほか | 使い方と試験結果 |
| 液シンの TOF(同日、ユーザー) | 2D をやめ、液シン 1 本ずつの 1D × 24(`/thgem/ls_tof/ls_tof_m0_ch<C>`、タイトルは対応表の名前)。通し試験で 24 本を確認 |
| `tools/root_sink/thgem/delayline_offsets.C` | 入口 `delayline_offsets_map(ファイル, 対応表, 出力 json, ラベル)` を追加: 対応表の 5 電極そろった ThGEM 全部、モジュールをまたいでも可。10 月の 2 台用の入口は同じ数値を出す(確認済み) |
| `config/config_11B_208Pb_pfqaq.toml` | **下書き**。pf-qaq の N_Wall の TOML(2026-11-17)から: VX2730 = 0、V1730 × 3 = 1〜3(光リンク番号は仮)、出力 `/data/11B_208Pb`、eLog `11B_208Pb`、sink = thgem_sink。Rust の設定パーサで読めることを確認(merger、ソース 4 つ) |
| `config/digitizers/11B_208Pb_*.json` | VX2730 = pf-qaq の N_Wall の設定のコピー。V1730 × 3 = 10 月の SN66 の ThGEM 用の値を並べ替え(トリガー ← 10 月の ch1、腕 ← ch2/3/5/4)、S-IN スタート、`gpio_mode` = Run、`start_delay` 0、ch15 = パルサー(POSITIVE)、module 3 の ch10〜14 は無効。シリアル番号は空欄 |
| `config/energy_calib_11B_208Pb.txt` | N_Wall の表の module 0(VX2730)の 31 行だけ。Si の行は入れない(module 1〜3 は ThGEM) |

**XY 中心のマクロの確認(合成データ):** ThGEM 3 の XL を +10 ns、ThGEM 7 の YD を −6 ns ずらしたファイルで、ThGEM 3 の中心 x が
ちょうど −10.000 ns、ThGEM 7 の y が +6.000 ns 動き、ほかは不変。出した表を thgem_sink の `--time-calib` に入れて流し直すと、
8 台とも残差 0.000 ns。

**Charge Pedestal = TRUE(2026-11-17 決定):** 10 月の V1730 は FALSE だった。PSD1 では負の電荷が E = 65535 になり、
本物のオーバーレンジと区別できない。TRUE は FPGA が全電荷に一定値を足すだけ(測定はしない、ON/OFF のみ)。QDC のようなペデスタルピークはできない
(ユーザーが 11-17 に VX2730 = PSD2 で確認: 値を設定でき、1000 にすると 1000 足されるだけ)。**V1730 で足される量は未確認**
(CAEN のレジスタ説明では 1024 と記憶、資料では未確認)→ セットアップの最初のランで、ノイズの山の位置と Qshort に足されるかを見る。
3 枚の JSON の defaults を TRUE にした。オフラインでは一定値を引く。上限はその分早く飽和する。

**通し試験(合成データ 2 s、`test_publisher`):** 受け取った 325,522 hit をすべて書き込み、late 0、イベント 8,968
(単独 4,803、反応 4,161)、`check_events.C` IDENTICAL、T = 199.9500 ns を測定。`pair_hits` の対角以外は (1,5)(2,6)(3,7)(4,8)
だけ(偶然の同時が数件)。XY の点は置いた位置から 0.2 ns 以内、腕 4 本そろう割合 84 %(期待 81 %)、`dt_T5_vs_T1` の平均
(25.2, 34.8)・相関 −0.85(4 ns ビン込みの期待 −0.84)。イベント定義を N_Wall のものにすると食い違いの警告が全部出ることも確認。

## 4. 時刻の較正

1. **ボード間:** セットアップ時にパルサーを全ボードの ch15(と VX2730)に入れ、`time_map.C`(TODO 72)→ start_delay を変える →
   次のランでパルサーが変えた量だけ動いたことを確かめる。
2. **XY の中心:** 腕ごとの時刻オフセット(TODO 70)。`delayline_offsets.C` は今 2 台用 → 8 台用にする(対応表から読む)。
   出力 `config/time_settings_11B_208Pb.json`。sink はラン開始ごとに読み直す。
3. **RF:** ビーム 1/100 から T をランごとに測る(nwall_sink と同じ)。

## 5. sink 以外の準備

| 項目 | 内容 |
|---|---|
| TOML | `config/config_11B_208Pb_pfqaq.toml`(N_Wall の TOML から: VX2730 = 0 + ThGEM のボード) |
| 出力先 | `/data/11B_208Pb` にするなら **archive_sync の cron を追加**(11-03 に N_Wall が HDD に同期されていなかった件) **✅ 11-17: `/data/11B_208Pb` と `/archive/DELILA_data/11B_208Pb` を作成、3 本目の cron 行(LOG `~/archive_sync_11b208pb.log`)、手で 1 回 OK、旧 crontab `~/crontab.bak.20261117`** |
| side3 への転送 | side3 の crontab に 2 本目の行: `SRC=daq@192.168.144.142:/archive/DELILA_data/11B_208Pb DST=/data/9MV2026/11B_208Pb`、N_Wall と同じ除外(`--exclude=*.delila --exclude=.*`)、LOCK と LOG は別名。転送先のフォルダ名は案 **✅ 11-17: その行で設定(DST `/data/9MV2026/11B_208Pb`、LOCK `/tmp/data_pull_11b208pb.lock`、LOG `~/data_pull_11b208pb.log`、BWLIMIT 60M)、手で 1 回 OK、旧 crontab `~/crontab.bak.20261117`** |
| デジタイザの設定 | V1730 なら 10 月の `thgem_v1730_psd1_0.json`(ThGEM の信号に合わせたもの)から。V1725 の PSD は新しく作る |
| Monitor | 配置(`monitor_layout.json`)を新しい構成に |
| Grafana | 対応表からレートのダッシュボードを生成(`tools/grafana/gen_dashboards.py`) **✅ 11-17 pf-qaq: 新しいフォルダ `11B_208Pb` に `Rates — 11B_208Pb`(uid `rates-11b-208pb`、80 ch、パネルは beam / LaBr / LS / ThGEM1〜8)。SY5527 の `HV Monitor` も N_Wall フォルダからここへ移した(uid は同じ、未コミット)。N1408 の Si HV は N_Wall に残す。新しいフォルダは自動では読み込まれず、`/api/admin/provisioning/dashboards/reload` で読ませた(`tools/grafana/README.md`)** |
| eLog | ログブック `11B_208Pb`(`docker/elog/elogd_thgem.cfg` に追加、`docker compose restart elog`) |
| `start_daq.sh` | `[network.root_sink] program` の許可リストに `thgem_sink` を足し、TOML のキーを渡す **✅ 11-17: pf-qaq に配置(未コミット)** |
| Operator | **設定ファイルが壊れていたらエラーで止める。** 2026-11-16、`[[network.sources]]` が 1 行だけ残って TOML が読めず、Operator は警告 1 行(読めなかった理由なし)でエミュレータ用の既定値のまま 9090 番に立ち上がった(`src/bin/operator.rs`)。**✅ 2026-11-17 Mac で済(未コミット、pf-qaq 未配備)**: 読めないファイル・無いファイルはほかのコンポーネントと同じくエラーで終了(ファイル名、行番号、足りない項目を表示)、エミュレータへの切り替えは削除。テスト 3 件。`start_daq.sh` は Operator が終了したら15 秒待たずにすぐ `operator.log` の末尾を出す。**11-17 pf-qaq でビルド済み**(`target/release/operator`、次に DAQ を起動したときから有効。動いていた Operator はそのまま) |
| コミット | 未コミットの作業を整理(nwall_sink の % 800 と pid の 8 ADC ビン、Sorter の高速化 `eb_core.hpp` — thgem_sink も使う) |
| シフト手順書 | `docs/shift_manual_nwall.md` をもとに ThGEM 版 |
| HV | PMT(液シン、LaBr3)は今の SY5527 の監視を続ける。ThGEM の HV は ThGEM チーム |


### 5.1 pf-qaq への配備 (2026-11-17 15:20–15:25、DAQ は Idle、再起動なし)

- **thgem_sink**: Mac の作業ツリー(未コミット)の `tools/root_sink` と `tools/delila2root` を `/tmp/thgem_build_20261117` に置いてビルド
  (g++ 13.3、ROOT 6.38.06、`-fopenmp -Wall -Wextra` で警告なし)→ `~/.local/bin/thgem_sink`(新規)。単体テスト 86/86、25/25。
  合成データの通し試験(ポート 15562 / 18092、本番の設定ファイルを使用): 325,522 hit 受信 = 書き込み、イベント 8,968、
  `check_events` IDENTICAL、T = 199.9500 ns、対応表とイベント定義の食い違いの警告なし = Mac と同じ。
- **設定**: `config/` に 11B_208Pb の 8 ファイル(新規のみ、既存は上書きしない)。`config.toml` は N_Wall のまま
  (V1730 がまだ無い。つないだら `ln -sfn config/config_11B_208Pb_pfqaq.toml config.toml` → DAQ 再起動)。
- **VX2730 の設定**: Tune Up 後の `nwall_vx2730_psd2_0.json` と 11B 用のコピーは中身が同じ(違いは `charge_pedestal: 0`、
  `short_charge_pedestal: 0` が明示されたことと、キーの並びだけ)。
- **Operator / start_daq.sh**: `src/bin/operator.rs`、`scripts/start_daq.sh` を置いて Operator だけビルド
  (pf-qaq の git は 9d4d1cf + この 2 ファイルと設定の変更)。壊れた設定・無い設定で終了コード 1 を確認。
- **cron と eLog**: §5 の表の各行。

### 5.2 ボードの確認 (2026-11-18、pf-qaq、DAQ は N_Wall の設定で VX2730 だけ Idle)

ユーザーが V1730 × 3 + V1725 × 1 を A3818 の 0-0、1-0、2-0、2-1 につないだ(VX2730 はそのまま)。
`caen_info`(pf-qaq で `--features "x743 dev-tools"` でビルド)で 1 台ずつ開いて読んだ:

| A3818 link-node | ボード | 割り当て |
|---|---|---|
| 0-0 | VX1730B **SN69**、DPP-PSD、16 ch、500 MS/s | module 1(TOML の下書きどおり) |
| 1-0 | VX1730B **SN59**、DPP-PSD | module 2 |
| 2-0 | VX1730B **SN53**、DPP-PSD | module 3 |
| 2-1 | **V1725S SN517**(CAEN のログの表記。N_Wall の module 5 の JSON のシリアルは 27647 で、同じボードかは未確認) | **開けない**: CAEN -6。`CAENDPPLayer.log` = `CAEN_DGTZ_Reset()` が error -18(タイムアウト)。5 秒後にもう一度試しても同じ。**原因 = クロックケーブルがまだ無い**(ユーザー: PLL が無い、すぐ持ってくる) |

- 3 台の V1730 のシリアル番号を JSON に記入し、TOML の「リンク番号は仮」を実際の対応に書き換えた。
  Mac と pf-qaq の両方を同じ内容にした(上書き前に md5 が一致していることを確認)。
- V1730 の `FPGA_FwVer` と、`ChEnable` などの ch パラメータは `caen_info` が読めない(CAEN -1)。`caen_info` が DIG2 のパラメータ名で読みに行くためと思われる(未確認)。V1725 が開けない件とは別。
- **V1725 の役割(ユーザー、11-18):** 小さな Si 検出器 2 個をビームモニタとしてビーム軸の近くに置く、その読み出し。

### 5.3 11B の設定に切り替え (2026-11-18 12:23、ユーザーの指示)

- **切り替える前:** N_Wall の DAQ が VX2730 だけで Idle。キューは Recorder・Merger とも 0、nwall_sink は受信 = 書き込み。
- **`thgem_sink` が後片付けの対象に入っていなかった。** `stop_daq.sh` にも、`start_daq.sh` の「前回のプロセスを片付ける」部分にもなく、再起動すると古い sink が残ってポートがぶつかるところだった → 両方に足した(未コミット)。pf-qaq の 2 ファイルは Mac の変更前と同じ md5 だったので上書き。
- 手順: `./scripts/stop_daq.sh` → `ln -sfn config/config_11B_208Pb_pfqaq.toml config.toml` → `start_daq.sh`(ログ `logs/20261118_122334/`)。
  - 起動したもの: Reader 0〜3、Merger、Recorder、Monitor、thgem_sink(`/data/11B_208Pb`、`ecal=31`)、Operator(:9091、`experiment_name = 11B_208Pb`、次のラン = 1)。
  - V1730 × 3 は起動時に PLL locked。
- **Configure(ラン 1 として)**: 7 コンポーネントとも Configured、4 台とも PLL locked。
  - 拒否されたパラメータは V1730 の `/par/dt_ext_clock` だけ(CAEN -1)。VME ボードではクロックの選択がハード側で決まるため。N_Wall のときから出ている無害なもの。
  - 書き込んだパラメータの数: VX2730 77、module 1・2 は 130、module 3 は 103(ch10〜14 が無効のぶん少ない)。
- **module 4(V1725、Si ビームモニタ)を準備:**
  - TOML の枠は**コメントのまま**。ケーブルが来たら外して、DAQ を再起動して Configure する。
  - `config/digitizers/11B_208Pb_v1725_pha1_4.json` = pf-qaq の N_Wall の `nwall_v1725_pha_5.json`(Si 用に調整済み)をもとにした。変えたのは次のとおり:
    - `gpio_mode` を V1730 と同じ `OUT_PROPAGATION_RUN` に、`start_delay` を 0 に(セットアップ時にパルサーで合わせる)
    - **Si = ch0 と ch2**(ユーザー: ch0・ch1 はカップルで共通のバッファを使うので念のため分ける)。ほかは無効
    - **論理信号のパルサーは入れない**(ユーザー: PHA で論理信号をトリガすることはできるが保証しかねる)。
      **代わりに ch15 = BNC の NIM パルサー(水色のパネル、2 幅)のテールパルス**。ほかのボードと同じ TTL のパルサーで駆動する(ユーザー、11-18)。
      - パルサーを減衰 50 µs・負極性にすれば、ch15 は Si と同じポールゼロ(`trap_pole_zero_ns = 50000`)で済む。閾値は 100。
      - **機種 = BNC PB-5**(ユーザー、11-18。3 幅)。**トリガーから出力までの遅れを設定でき、その値は信用できる**(ユーザー)。最小は 200 か 250 ns。
        - ch15 のパルサーの時刻 = TTL + 設定した遅れ D。`time_map.C` の `time_map_boards` は既定で窓 ±2000 ns、ピークを探すのは ±1000 ns なので、+250 ns でも入る。
        - 前後のランの比較(`time_map_verify`)では D が打ち消し合うので、コードの変更は要らない。
        - 絶対的な時刻差に使うときは D を引く。残るのは PHA の時刻の取り方(テールパルスの立ち上がりに RC-CR2)と、V1730 の CFD との一定の差。気になるなら、module 3 の空き ch(10〜14)に同じ出力を一度入れて測れる。
        - PB-5 の立ち上がり時間は短め(〜100 ns 以下)の方が時刻がよい。
  - 対応表に `4 0 SiMon1`、`4 2 SiMon2`、`4 15 pulser4` を追加。`thgem_map.hpp` の読み込み関数でエラー・警告とも 0。
  - Mac と pf-qaq は同じ内容(上書き前に、自分の追加分以外は md5 が一致することを確認)。
  - Grafana のダッシュボードは module 4 が入ってから作り直す。

### 5.4 A3818 のドライバ修復と module 4 の追加 (2026-11-20)

**配線(ユーザー):** 全デジタイザとパルサーがつながった。
- mod 0〜3: TTL のパルサーを LEMO の T で 4 つに分けたもの(おそらく 1 V 未満)。**mod 0 は ch0**、mod 1〜3 は ch15。
- mod 4: PB-5 のテールパルス(TTL から 250 ns 遅れ)を ch15。

**見つかった問題: 11-19 10:16 の再起動(手動、10:08 に正常停止)でカーネルが 7.0.0-34 → 7.0.0-42 に上がり、A3818 のドライバが読み込まれなかった。**
`/dev/a3818_*` がなく、DIG1 のボードは 1 台も開けない状態だった。ランはしていなかったので失ったデータはない。原因は 2 つ:

1. **DKMS が、今動いているカーネル向けにビルドする。** ドライバの Makefile は `KERNELDIR ?= /lib/modules/$(shell uname -r)/build`、dkms.conf は `MAKE="make"` で上書きしない。
   - 42 用の `.ko` は中身が 34 用だった(11-04 に作られたもの)。11-20 06:20 に自動更新で入った 43 用の `.ko` は中身が 42 用だった。USB ドライバ(CAENUSBdrvB)も同じ作り。
   - 修復(ユーザーが sudo で実行):
     - 両方の dkms.conf を `MAKE="make KERNELDIR=/lib/modules/${kernelver}/build"` に書き換え
     - 42 と 43 で `dkms build --force` → `dkms install --force`
     - `modprobe a3818`
   - 確認: 42 用・43 用とも中身のカーネル版が一致。
2. **udev のルール(`10-CAEN-A3818.rules`、`10-CAEN-USB.rules`)と `/usr/bin/v1718_name.sh` が消えていた。** ドライバの後処理(dkms_post_remove)は、どれか 1 つのカーネルからドライバを外すだけで、全カーネル共通のこのファイルを消す。古いカーネルを片付けたときに消えたと思われる(ルールのフォルダの時刻は 11-08、それより前)。
   - 10-06 の起動時に作られたデバイスは 0666 のまま残っていたので、11-19 の再起動まで気づかなかった。
   - 修復(sudo): DKMS が触らない別名のファイル **`/etc/udev/rules.d/99-delila-caen.rules`** に同じルールを書き、`v1718_name.sh` を戻し、`udevadm trigger --subsystem-match=a3818 --action=add`。
   - 確認: 32 個のデバイスがすべて 0666。

**その後:**
- `caen_info` で 4 台とも開けた。2-1 = **V1725S SN517、DPP-PHA、250 MS/s**。
- TOML の module 4 を有効にして DAQ を起動 → Configure(ラン 1)。8 コンポーネントとも Configured、**5 台とも PLL locked**。
- module 4 の警告 2 種類(CFD の設定レジスタが読めない、`ch_trg_latency` が DevTree にない)は、N_Wall の module 5(同じ V1725 PHA)でも同じものが出ていた → 無害。

**ユーザーの回答(11-20):**
- VX2730 の ch0 のパルサーは、デジタイザの時間オフセットを見るためだけ。**本番では mod 0 の ch0 = ビーム 1/100 に戻す**(設定も負極性・閾値 500 に戻すこと。対応表とイベント定義は今のままで正しい)。
- V1730 の ch15 は 2 Vpp にする。
- 進め方: ユーザーが Tune Up で信号を合わせる → 私が N_Wall(TODO 77)と同じ手順でボードディレイを合わせる。
  ファイルは直接書き換えない。Operator が起動時に設定を読み込んで持っているため。

### 5.4.1 本番のつなぎ方(ユーザー、11-20)と切り替えの点検表

| ボード | 時間オフセットの測定中 | 本番 |
|---|---|---|
| mod 0(VX2730)ch0 | TTL パルサー(正) | **ビーム 1/100**(負極性・閾値 500 に戻す) |
| mod 1〜3(V1730)ch15 | TTL パルサー(正、2 Vpp) | **ビーム 1/1000**(極性は信号に合わせる) |
| mod 4(V1725)ch15 | PB-5(TTL で駆動、250 ns 遅れ) | **PB-5 をビーム 1/1000 で駆動** |

- **本番中もボード間のずれを見続けられる**(TODO 72 で欲しかった形)。
  - 1/1000 の各パルスが 1/100 のどれかと同じバンチなら、`time_map.C` に `pulser_spec = "0:0,1:15,2:15,3:15,4:15"` を渡し、そのまま本番データで測れる。
  - 別々の分周器なら、どのパルスもバンチの格子(T ≈ 200 ns)には乗る。その場合は mod 0 の ch0 から作る RF に対する位相で測る(T の整数倍は決まらない)。
  - **回答(ユーザー、11-20): 分周器 RD2000(2 ch、入出力とも NIM)をカスケードにする。ch0 = 1/100、ch1 = その 1/10 → 1/1000。** 1/1000 の各パルスは 1/100 のどれかと同じバンチなので、本番データから直接測れる。
    - 1/1000 は RD2000 を 1 段多く通るぶん、1/100 より一定量遅れる(RD2000 の内部遅延 + ケーブルの差)。
    - そこで、本番の最初のランで「ビーム由来の信号でのずれ」の基準値を一度測る。以後はその値からの変化(ランごとの跳び、ずれていくこと)を見る。ボード自体のずれは、今の TTL パルサーの測定で決める。
- 対応表: mod 1〜3 の ch15 と mod 4 の ch15 は `det = pulser` のままにする。
  - `thgem_map.hpp` は `beam` を 1 本しか許さない(2 本目はエラー)。RF は mod 0 の ch0 からだけ作る。`pulser` は数えるだけで、特別な処理はない。
  - 名前(`pulser1`〜`4`)は、本番に切り替えるときに `beam1k_1` などへ変えてもよい(Grafana の表示名)。
- レート: ビームが 5 MHz なら、1/1000 は各ボード 5 kHz。V1725 の PB-5 も 5 kHz で、50 µs の減衰に対して間隔は平均 200 µs。
- 切り替えるときの確認: **1/100 と 1/1000 は NIM(負)なので、VX2730 の ch0 と V1730 の ch15 は負極性にする**(今の ch15 は TTL 用に正)。PB-5 の外部トリガーが NIM で動くかと、PB-5 の遅れの設定値も確認する。
- **VX2730 の ch0 を戻すときの元の値**(Tune Up の前): override = `trigger_threshold 500`、`cfd_delay_ns 4`、`energy_coarse_gain x4`。極性・弁別方式・gate_pre は既定値(Negative、LeadingEdge、既定)。

### 5.4.2 Tune Up の後の設定(ユーザーが 11-20 14:17〜14:32 に保存、私が確認)

| ボード | 変わったところ | 判断 |
|---|---|---|
| mod 0 ch0 | 正極性、CFD、gate_pre 100(cfd_delay は既定へ) | パルサーの測定用。本番の前に §5.4.1 の元の値へ戻す → **戻ったのは負極性と閾値 500 だけ(11-22 に確認、N_Wall ラン 168 と 11B ラン 12 のスナップショットの比較)。ユーザーの判断(11-22): CFD・cfd_delay 8・gate_pre 100 のまま使う。入力が NIM になったら、ユーザーがあとで調整する** |
| mod 2・3 ch15 | 正、DC 20、2 Vpp、ゲイン 640 fC | OK |
| mod 1 ch15 | 正、DC 20、2 Vpp(既定値として)、ゲイン 160 fC | OK |
| **mod 1 ch0〜14** | **入力レンジが全 ch で 2 Vpp(既定値を変えた)、DC 25 → 10、ThGEM のトリガー ch0/5/10 が正 → 負** | **mod 2・3 と食い違う。意図したものかユーザーに確認中**。パルサーの測定には影響しない |
| mod 4 | ch15 = 正(閾値は既定の 20)、`record_length` 10000 → 100000 | 波形を取らないリストモードでは record length は使われない(UM5678)ので無害。波形を見るために伸ばしたと思われる |

Tune Up で、5 台ともパルサーのチャンネルにイベントが来た(VX2730 ch0、ほかは ch15)。Si の ch0・ch2 は 0(信号はまだ)。

### 5.5 ボードディレイ調整の準備 (2026-11-20、DAQ には触っていない)

- **今のディレイはすべて 0:** VX2730 の `rundelay` 0(マスター、SW スタート、TRG-OUT = Run)、V1730 × 3・V1725 の `start_delay` 0(S-IN スタート、TRG-OUT = Run)。
- **手順(TODO 77 §6 と同じ):**
  1. パルサーを入れたまま、ふつうのランを 1〜2 分。thgem_sink の hits ツリー(`hits_tree = true`)を使う。
  2. 基準 = VX2730 の ch0 のパルサー。各ボードのパルサー(mod 1〜3 の ch15、mod 4 の ch15)の Δt を `time_map.C` で測る。
     パルサーが最後の ch にないので、`pulser_spec = "0:0,1:15,2:15,3:15,4:15"` と明示する。
  3. ディレイを決める。DIG1 の `start_delay` は 16 ns 刻み、VX2730 の `rundelay` は 8 ns 刻み。どちらも、遅らせたボードの時刻が小さくなる。マイナスにはできないので、全体に一定の値を足す。
  4. 次のランで、パルサーが変えた量だけ動いたことを確かめる(`time_map_verify`)。ボード間の差はランごとに 0 / 8 ns の不定があるので、何本かのランで見る。
  5. 刻みより細かい残りは、`time_calib_file` の時刻補正表で合わせる。
- **mod 4 だけの注意:** パルサーの Δt には PB-5 の遅れ 250 ns と、PHA の時刻の取り方(RC-CR2 のゼロクロス、平滑 128 サンプル)による一定の遅れが含まれる。
  250 ns は引けるが、PHA の分は、同じ出力を一度 V1730 の空き ch に入れないと分からない。ランごとの不定を見るだけなら、どちらも打ち消し合う。

### 5.6 ボードディレイの調整 (2026-11-20 14:38〜14:52、DAQ_Test ラン 3〜7、Claude が Operator REST から)

**準備:**
- ラン中の実験名を `DAQ_Test` にするだけでは足りない。thgem_sink は ROOT のファイル名を Operator の `experiment_name`(TOML の値)から取るため、`.root` が `run000N_0000_11B_208Pb.root` になり、本番の同じ番号のランと名前がぶつかる(sink は後から来た方に `_<unix_ns>` を付ける)。
- そこで 11-10 と同じく専用の TOML を使った。**`config/config_daq_test_11b_pfqaq.toml`** = 11B の TOML の写しで、実験名 `DAQ_Test`、出力 `/data/DAQ_Test`、InfluxDB と eLog なし。デジタイザの JSON は 11B のものをそのまま使う。
- `config.toml` をこれに向けて DAQ を再起動した(DAQ_Test の次のラン = 3)。

**パルサー:** 1 kHz の TTL = mod 0 ch0 / mod 1〜3 ch15。PB-5(+250 ns)= mod 4 ch15。dt = t(ch) − t(mod 0 ch0)、`time_map_offsets`(bin 0.25 ns、mode)。ピークの幅は FWHM 0.2〜0.3 ns(mod 4 は 1.4 ns)。

| ラン | 設定 | mod 1 | mod 2 | mod 3 | mod 4(生の値) |
|---|---|---|---|---|---|
| 3(2 分) | 全部 0 | −345.12 | −373.88 | −402.89 | +144.39 |
| 4(2 分) | **VX2730 `rundelay` 408、mod 1 `start_delay` 64、mod 2 32**、mod 3・4 は 0 | −9.12 | −5.88 | −2.89 | +560.60 |
| 5(20 秒) | 同上 | −9.12 | −5.88 | −2.89 | +560.69 |
| 6(20 秒) | 同上 | **−1.12** | **+2.12** | **+5.11** | +552.74 |
| 7(20 秒) | 同上 | −1.12 | +2.12 | +5.11 | +552.71 |

- **マスター(VX2730、SW スタート)の `rundelay` は、マスター自身の取得開始だけを遅らせる。** TRG-OUT の Run は遅れない(遅れるなら dt は変わらないはず)。ラン 6・7 はディレイの設定から見込んだ値(−1.12 / +2.12 / +5.11)とぴったり一致。DIG2 マスターでこれを確かめたのは初めて(TODO 77 では S-IN スレーブでだけ確認していた)。
- **V1730 の `start_delay`(S-IN スレーブ)は設定どおりに効く:** mod 1 と mod 3 の差は 57.77 → −6.23(64 を入れた)、mod 2 と mod 3 の差は 29.01 → −2.99(32 を入れた)。0.01 ns 以内で一致。
- **ランごとの跳び:** VX2730 とスレーブ全員の間が、ランごとに 0 か 8.00 ns 動く(4 本中 2 本ずつ)。**V1730 どうしは 5 本とも 0.1 ns 以内で一定**、mod 4 もスレーブと一緒に動く。N_Wall(TODO 77 §6.7)と同じで、遅延では消せない → 精密な時間は、ランごとにパルサー(本番はビーム 1/1000)で補正する。
- **結果:** スレーブ 3 台どうしは 6.2 ns 以内にそろった(16 ns 刻みではこれ以上は無理)。VX2730 に対しては、−1.1〜+5.1 ns か −9.1〜−2.9 ns のどちらか(ランで変わる)。
- **mod 4(V1725 PHA)は 0 のまま(未決)。** ラン 6・7 の状態で、生の値 552.7 − PB-5 の設定 250 = **+302.7 ns**(ケーブル長の差は含む)。
  - これはボードのずれと、PHA が時刻を決めるまでの遅れ p の和。
    私は「大半は p(RC-CR2 の平滑 128 サンプル = 512 ns)」と推定したが、**ユーザーは否定(11-20): 303 ns は PHA 自体の遅れより明らかに長い**。原因は未解明。
  - 切り分けの案:
    - (a) 波形つきの短いラン(mod 4 だけ)で、トリガーの位置とパルスの立ち上がりの差から p を直接測る。
    - (b) PB-5 の出力を T で V1730 の空き ch(mod 3 ch10)にも入れ、PB-5 の実際の遅れ(設定 250 ns)を V1730 の CFD で測る。
    - 残り = V1725 のボード自体のずれ。
  - **(a) の結果(11-20 15:04〜15:10、DAQ_Test ラン 8・9、mod 4 だけ波形 ON: 7000 ns、pre-trigger 4000 ns、測ったあと設定は元に戻した = バックアップと完全に一致):**

    | | ラン 8(平滑 128 = 512 ns、今の設定) | ラン 9(ch15 だけ平滑 16 = 64 ns) |
    |---|---|---|
    | FW のトリガーの印(digital probe 1)− 入力の 50% の点 | **241.9 ns** | **200.4 ns** |
    | パルサーの時間差(イベントの時刻、両ランとも −8 ns の状態) | 561.32 ns(FWHM 1.6) | 519.13 ns(**FWHM 0.3**) |

    - トリガーの印は 3984 ns(pre-trigger の 16 ns 手前)で一定。入力の 10→50% は 40 ns(PB-5 の立ち上がりは速い)。
    - **イベントの時刻はトリガーの印と一緒に動く**(−42.19 対 −41.5 ns)。
    - したがって 303 ns の内訳 = PHA のトリガーの遅れ 242 ns(平滑 128 の分は約 42 ns、残り約 200 ns は平滑を軽くしても残る)+ **約 61 ns**(V1725 のボードのずれ + PB-5 の 250 ns を超える分 + ケーブル)。
    - 約 200 ns の部分が本当に PHA の遅れなのか、FW が波形とトリガーの印を一定量ずらして記録しているだけなのかは、この測定では区別できない。区別するには (b)(V1730 の CFD で同じパルスを測る)が要る。
    - ch15 のパルサーは、平滑 16 にすると時間の幅が 1.6 → 0.3 ns に良くなる。Si の ch は雑音との兼ね合いで別に決める。
  - ハードの遅延でそろえる必要はない(ハードの同時計数は使っていない)。ソフトの時刻補正表で直せる。
- 測定の JSON は pf-qaq の `/tmp/tm_daqtest/run{3..7}_offsets.json`。`config.toml` はまだ DAQ_Test の TOML を指している。

### 5.7 V1725 の平滑を 1 に、VX2730 の jesd_unlock (2026-11-20 15:07〜)

- **平滑:** ユーザー: 平滑 128 はかけすぎ、1 サンプル(なし)でよいはず、Si は雑音を見てから決める。
  - mod 4 の `fast_discr_smoothing` の既定値を RCCR2_SMTH_1 にした(REST で保存、ほかの値は変更なし)。
  - **ラン 10(10 秒)で V1725 が鳴りっぱなしになった:** ch15 が 313 万、Si の ch0・ch2 が各 160 万(10 秒間)、`.delila` 146 MB。閾値 20 は平滑 128 用の値で、平滑なしでは雑音を拾う。
  - ユーザーが案 1(平滑 1 のまま、信号のない Si の ch を無効、ch15 の閾値を Tune Up で上げる)で Tune Up する。
- **VX2730 の jesd_unlock:** ラン 10 を始めるときの Reset で、VX2730 のリセットが CAEN −6 で失敗した(このセッションの 9 回中 1 回だけ。15:08:17)。
  - その 0.5 秒後から `jesd_unlock`(errorflags 0x20)。ラン 10 では VX2730 の hit が 0。
  - 15:09:32 のリセットでも直らず。ping は正常。
  - 直し方(TODO 85 と同じ): DAQ を再起動してボードを開き直す → 残るなら VX2730 の電源の入れ直し。リセットが失敗した原因は不明。

### 5.8 最終の配線表 (2026-11-20 18:10〜18:30、ユーザー:「これが最終版のはず」)

ユーザーの配線表(Google スプレッドシート。URL はリポジトリに書かない)に合わせた。
名前は表のまま。表の 77 件と対応表を機械的に照合し、すべて一致(表にないのは mod 4 の ch15 = PB-5 だけ)。

| ボード | 表 | 前の案からの変化 |
|---|---|---|
| mod 0 (VX2730) | ch0 ビーム 1/100、ch1〜6 Gamma 1〜6、ch7 Gamma 0(中央)、ch8〜19 NWB12〜01、ch20〜31 NWA12〜01 | 名前だけ(LaBr1〜7 → Gamma、LS01〜24 → NWB/NWA)。ch と役割は同じ |
| mod 1〜3 (V1730) | D1〜D8: T・XL・XR・YU・YD(mod 1・2 は ch0〜14、mod 3 は ch0〜9)、ch15 ビーム 1/1000 | 名前だけ(T1_trig → D1-T など、pulser1〜3 → beam1k_1〜3)。**トリガーの ch は案と同じ** |
| mod 4 (V1725) | **ch4 Si-L、ch8 Si-R** | **Si が ch0・ch2 から ch4・ch8 へ**(別のカップル) |

- **対応表** `config/channel_map_11B_208Pb.txt`: 書き直し(ヘッダーの DRAFT を外した)。役割の語(`LaBr`、`LS`)は sink のキーワードなのでそのまま(LaBr = γ 線検出器、LS = Neutron Wall とヘッダーに注記)。
  - mod 1〜3 の ch15 は `det = pulser` のまま(`beam` は 1 本だけ、§5.4.1)、グループ `beam1k` で Grafana に出す。
- **イベント定義** `config/event_def_11B_208Pb.json`: トリガー・窓・まとめる幅は変えていない。タグとカウンタの名前を `LS`/`nLS` → `NW`/`nNW`、`LaBr`/`nLaBr` → `Gamma`/`nGamma`(events ツリーの枝の名前が変わる)。1 枚だけのイベントも書く(ユーザー、11-20: 較正で要る)。
- **確認:** Mac でビルドした thgem_sink に新しい 2 ファイルを読ませた: 役割のある ch 76(beam 1、LaBr 7、LS 24、pulser 4、ThGEM 40)、ThGEM 8 台とも 5 本そろう、トリガーの食い違いの警告なし。
- **V1725 (mod 4):** pf-qaq の Operator REST で、上書きの `0`・`2`(`enabled` だけ)を `4`・`8` へ移し、保存・読み直しで一致。ch15(PB-5、正)は変えていない。名前の説明文も直した。次の Configure で入る。控え: pf-qaq `/tmp/v1725_4_before_final.json`。
- **TOML 2 本**(11B、DAQ_Test): コメントだけ(ボードの説明、Si-L ch4 / Si-R ch8、ch15 = ビーム 1/1000)。
- **pf-qaq:** 対応表・イベント定義・TOML 2 本を送った(上書き前は Mac の旧版とバイト単位で同じ。控え `~/final_cabling_backup_20261120/`)。sink は次のランの開始で読み直す。
- **Grafana:** `gen_dashboards.py --config config/config_11B_208Pb_pfqaq.toml` で作り直して reload。96 ch、名前つき 78、パネルは beam / Gamma / NWB / NWA / D1〜D8 / beam1k / SiMon。生成 JSON はリポジトリにも取り込んだ。
- **デジタイザ JSON 5 つ:** Mac のリポジトリを pf-qaq の版(ユーザーの Tune Up の保存分 + 今の V1725)にそろえた。
- **未確認(ユーザーに質問):** 表は mod 4 の ch15 が「-」。PB-5 を本番でも ch15 に入れるか(今は残してある)。γ 線検出器は表の凡例が CeBr3、図 `docs/detector_positions_11B_208Pb.drawio` は LaBr3。
- `config/energy_calib_11B_208Pb.txt` は N_Wall の係数のまま(コメントの名前も LaBr1〜7)。γ 線の較正をし直すときに作り直す。

### 5.9 V1730 の集約待ちで .root からヒットが抜けた (DAQ_Test ラン 16、2026-11-21、Cf 線源での試験)

- **見つけたもの:** thgem_sink の警告「180 hit(s) arrived more than 1000 ms behind the newest hit and are MISSING from every tree of the .root file (the .delila has them)」。ラン 16 は 2 億 8662 万ヒット。
- **.delila で到着順に測った(`lag_check`、ファイル 0004〜0006 の 1176 秒):**
  - V1730 のヒットはふだんでも 100〜300 ms 遅れて届く(すぐに届く VX2730 が基準)。
  - **遅れが大きいのは mod3 の 2 本だけ:** ch4(D7-YD、780 Hz)が最大 1.22 秒(1 秒超えは 254 個)、ch1(D7-XL、1950 Hz)が最大 0.61 秒。どちらも相方の T が 0 Hz。mod3 には ch6(D8-XL)が 20.8 kHz で鳴っていた。
  - mod2 の ch14(1280 Hz、相方 0 Hz)は 0.3 秒以内。mod1・mod2 の低い計数率の ch(0.3 Hz など)も遅れていない。
- **原因(UM4380 rev6 p.11「Number of Events per Aggregate」、p.14「Forced Data Flush」0x8040):** カップルの集約は `eventaggr`(1023)個たまるか、フラッシュされるまで読めない。ライブラリの autoflush(`/par/autoflush_enable` TRUE)は、ボードに送るものがないときにしか効かないと見られる。忙しいボード(mod3)では、ゆっくりたまるカップルが満杯まで待たされる。見込みの 1023/780 Hz = 1.31 秒、1023/1950 Hz = 0.52 秒が実測と合う。
  - **本番での危険:** 腕が鳴るボードでは、低い計数率の T の ch(例 10 Hz)が最大で約 100 秒待たされ、.root のイベントから抜ける。
  - `eventaggr` を小さくすると待ちは縮むが、カップルのメモリ(≒ eventaggr × 最大 1024 集約)も小さくなる。1 なら 1024 イベントしか貯められない。
- **対策(Mac で実装・テスト済み、未配備):** Reader が Running 中に 100 ms ごとに `/par/manual_flush` = 1 を書く(`src/reader/read_loop_dig1.rs` `Dig1Flusher`、`devtree::par::MANUAL_FLUSH`)。集約の大きさは変えない。失敗は警告(10 秒に 1 回まで)。テスト 3 件(間隔、警告の間引き、PSD1・PHA1 の DevTree に WRITE_ONLY・setinrun true で実在)。904 件すべて通過、clippy 警告 0。
  - 配備: `read_loop_dig1.rs` と `devtree_paths.rs` を pf-qaq へ(どちらも pf-qaq の版 = HEAD、確認済み)→ reader をビルド → DAQ 再起動 → DAQ_Test のランで `lag_check` を取り直し、V1730 の遅れが全 ch で約 0.3 秒以内、sink の `late=0` を確認する。
- 道具: `lag_check.cpp`、`lag_ch.cpp`(TDelila.hpp だけで動く、pf-qaq `/tmp`)。
- **ラン 17(11-21 17:15〜18:44)でさらに大きく:** 波形 2 本表示 × ノイズで 670 MB/s、mod2・mod3 は全 ch がボードごと 1.1〜1.3 s 遅れ(読み出しが追いつかない)、ゆっくりしたカップルは最大 3 s。sink の .root から 482 万ヒットが抜けた(.delila は完全)。ディスクの残りで自動停止(drain-first、5 秒、最小の空き 0.79 GB)。N_Wall の archive_sync が直後に 84 GB 空けた。
- **配備(2026-11-21 21:00、ユーザーの了承、DAQ はランなし・キュー 0 で再起動):** Reader の定期フラッシュ(ビルドは別の target ディレクトリ、旧 reader は `~/reader.20261110.bak`)、両 TOML の `safe_horizon_ms = 5000` と `disk_soft_free_mb = 300000`、ThGEM の閾値 T 250・腕 400 を保存(TODO 90 §4.3)、`/data/DAQ_Test` の archive_sync の cron 行(控え `~/crontab.bak.20261121`)。確認のラン 20(DAQ_Test、3 分): フラッシュの失敗 0、sink は 231 万ヒットを全部書いた、V1730 の遅れ最大 148 ms。
  - 教訓: `stop_daq.sh` は `target/release/<名前>` をコマンドラインに含むプロセスを止める。同じ文字列を含む ssh のコマンドは自分ごと止められる → 手順はスクリプトファイルにして呼ぶ。

### 5.10 本番の設定に切り替え (2026-11-21 22:46、ユーザーの指示)

- `config.toml` → `config/config_11B_208Pb_pfqaq.toml`(DAQ_Test との違いは実験名、InfluxDB、eLog、保存先 `/data/11B_208Pb` だけ)、DAQ を再起動、Configure(ラン 1): 5 台とも PLL locked、`adc_min = 3500` が V1730 の 3 台で有効、InfluxDB 書き込み開始。次のラン = 1。
- ThGEM の閾値は全 40 ch = 100(TODO 90 §4.3)、V1730 の `adc_min = 3500`(モーターのノイズをデータから落とす)。
- **転送の間隔を 5 分に(ユーザー):** pf-qaq の archive_sync の 11B の行と、side3 の data_pull の 11B の行を `*/15` → `*/5`(控え pf-qaq `~/crontab.bak.20261121b`、side3 `~/crontab.bak.20261121`)。ロックは両方とも以前から `/tmp` のファイル + `flock -n`(前の回が動いていれば何もしない。プロセスが終われば自動で外れ、kill や再起動でも残らない)。書きかけの .root は `run_inprogress_*.root` で、archive_sync が飛ばす → ランの後に転送。
- 行き先: pf-qaq `/data/11B_208Pb/`(.delila と .root)→ `/archive/DELILA_data/11B_208Pb/`(5 分ごと)→ side3 `/data/9MV2026/11B_208Pb/`(.root のみ、5 分ごと、BWLIMIT 60M)。

### 5.11 run 17 で reader が 2 台落ち、Operator が止まった (2026-11-23、時刻は現地 EET)

run 17(PuBeNi calibration、11-22 22:01 開始)。DAQ は 11-22 20:36 に再起動したもの。

- **01:06:01 reader 4(PHA1 V1725 SiMon)、06:07:37 reader 3(PSD1 V1730 ThGEM7-8)が segfault**。どちらも `libCAENDPPDigitizer.so` の中(ライブラリ先頭からの位置 0x26eac と 0x26242、アドレス 0x10 への書き込み、スレッド名 tokio-runtime-w = 読み出しループの spawn_blocking)。ログには何も残らずに止まる。pf-qaq の kernel ログ(10-06 以降)で reader の segfault はこの 2 件が初めて。N_Wall 以降に増えたものは 11-21 の manual_flush(100 ms ごと、読み出しループと同じスレッドで順に呼ぶ)。原因は未確定。A3818 の RX timeout は一晩中 1 時間に 700 回前後、06 時以降は 2,500 回前後。
- **01:06:51 から Operator が "Too many open files"**(fd 上限 1024、1014 個がソケット)。落ちた reader に問い合わせるたびにソケットが漏れる(上限を上げた後は 1 秒に約 100 個)。UI も REST も応答せず、**digitizer guard も run を止めなかった**(reader の消失は Error 扱いになっていない)。run 17 は 4 台→3 台のまま 08:08 まで記録。
- thgem_sink: いつからか全ヒットが `late`(2.77e9)になり、ROOT へは 1.09e9 ヒットで止まった。RSS 43 GB、入力 0 でも CPU 500%。run 17 の ROOT は `run_inprogress_1795377715.root`(9.0 GB)のまま残る。**.delila は 96 ファイルとも正常に閉じた**。
- 復旧(ユーザーの指示、Claude が実施): `prlimit --nofile=1048576` で Operator を生き返らせる → 08:08:31 `POST /api/stop`(run 17 は status=Error で閉じ、eLog 投稿)→ stop_daq.sh → start_daq.sh(08:10、logs/20261123_081012)→ Configure(ラン 18): 5 台とも PLL locked、警告は前夜と同じ種類だけ。クレートの電源リセットは不要だった。
- 残り: ① Operator のソケット漏れ ② guard が reader の消失で止めない ③ sink の late の連鎖、run 17 の ROOT は .delila から作り直し ④ CAEN ライブラリ内 segfault の原因(manual_flush を疑う)。

### 5.12 mod0: ガンマと beam を CFD に、LS(ch8〜31)は昨日の値へ (2026-11-23)

- ユーザーが run 19 の前に mod0 ch0〜7(beam + Gamma)を CFD にした(cfd_delay 28〜34、閾値 12〜13、ch0 は 100)。そのとき既定値も変わり、LS の ch8〜31 に `gate_pre_ns` 20 → 128 と pedestal 0 が入った(run 19 では LS も CFD、run 20 で LeadingEdge に戻っていた)。
- ユーザーの依頼で、ch8〜31 を run 17 のスナップショット(run 16・18 とも同じ)へ戻した。既定値を run 17 のものに戻し、ch0〜7 には今の値(gate_pre 128、pedestal 0 を含む)を個別の上書きとして持たせた。確認: ch0〜7 は変更前と同じ、ch8〜31 は run 17 と同じ、board は不変。`POST /api/digitizers/0/apply`(Configured、run 20 と 21 の間)で 122 パラメーターを送り、ok=122・failed=0。pedestal の FW 既定値は 0(DevTree)なので、未設定に戻しても値は昨日と同じ。控え: pf-qaq `/tmp/restore_ls_20261123/`(file_before_restore.json、current_op.json、snaps.json、new_cfg.json)。

### 5.13 run 21 で reader 3 が再び segfault、無効チャンネルとの対応 (2026-11-23)

- run 21(BG run with beam tuning、11:01 開始)。**12:08:51 reader 3 が segfault**。06:07 と同じ命令(0x26242)。Operator は fd 1024 に張り付き、UI と REST が止まった(§5.11 と同じ)。
- **クラッシュ箇所**: `libCAENDPPDigitizer.so`(シンボルなし)のイベントデコードのループ。32 bit ワードを読み、bit31(カップル内の偶数/奇数チャンネル)で分岐して、チャンネルごとの配列へ 48 バイトのレコードを書くところ。配列のポインタが NULL で、そのチャンネルの最初のイベントを書こうとした(書き込み先 0x10)。0x26eac(PHA)も同じ形。Reader は RAW エンドポイント(DATA、SIZE のみ)で読み、デコードは自前の Rust デコーダが行う。それでも Dig1 プラグインは内部でバッファをデコードしているらしい。
- **無効チャンネルとの対応**: 落ちたのは無効チャンネルのあるボードだけ。mod3(ch10〜14 無効)が 2 回、mod4(ch4/8/15 以外は無効)が 1 回。全チャンネル有効の mod1 と mod2 は一度も落ちていない(この DAQ セッションの RX timeout は mod2 が 864 回で最多、mod3 246、mod1 142、mod4 48)。チャンネルの有効/無効は run 1 から変わっておらず、manual_flush 入りの reader(11-21 20:58 ビルド)で run 1〜16 も落ちなかった。まれに起きることで、きっかけは未確定。落ちる直前の 12 秒間、reader 3 に -6 以外の出力はなかった。
- **対策(ユーザー承認)**: mod3 の ch10〜14 と mod4 の未使用 13 ch を enabled にし、閾値 16383(最大、入力は何もつながっていない)にした。使っているチャンネル、board、既定値は変えていない。`POST /api/digitizers/{3,4}/apply`(Configured、run 22 の前)。mod3 は 81 パラメーター、mod4 は 61 で、どちらも failed=0。mod4 の `eventaggr` 1023 → 511 の丸めは前からある。これでも落ちたら、次に疑うのは manual_flush。控え: pf-qaq `/tmp/enable_unused_20261123/`。
  - **閾値 16383 では鳴りやまなかった(ユーザー、mod3 の Tune Up)。4000 なら問題ない。** DevTree の上限が「鳴らない値」ではない(推測: FW の中でベースライン + 閾値が 14 bit からあふれる。未確認)。ユーザーが mod3 の ch10〜14 を 4000 にした(13:10 保存)。mod4 の未使用 13 ch も、mod3 の Tune Up が終わってから 4000 にした(13:13、全コンポーネント Idle、61 パラメーター、failed=0。ch4/8/15 は 100 のまま)。
- **Stop が半分しか届かなかった**: fd 上限を上げた後の `POST /api/stop`(12:49:19)は mod0 と mod1 にだけ届いた。残りは "Zmq error: Too many open files" で、こちらは **ZMQ コンテキストのソケット上限(1023)** なので prlimit では解けない。漏れの原因(見込み): `src/operator/client.rs` は問い合わせごとに REQ ソケットを作り、タイムアウトで捨てる。LINGER が既定の −1 なので、配達できない要求を抱えたソケットがコンテキストに残り続ける。直すには LINGER=0 にする(未実装)。
- reader 2 と 4 は SIGTERM で止めた(12:50:07)。Recorder は最初の EOS(source 1)で run 21 のファイルを閉じ、その後の入力は 0 だった。そのため 12:49:19〜12:50:07 の mod2/mod4 は記録されていない(mod0 と mod1 はもう止まっていた)。ファイルを閉じた後に受け取った 1,617 イベントは written にも dropped にも入っていない(要調査: silent)。その手前までは、run 21 の .delila に mod2/mod4 も最後のファイルまで入っている。mod3 は 12:08 以降が欠け。
- stop_daq.sh → start_daq.sh(12:54、logs/20261123_125415)→ Configure(run 22): 5 台とも PLL locked。start_daq.sh は Operator の fd 上限をハードリミット(1048576)にするよう変更した(UI は生き残る。ZMQ の上限は残る)。
- sink: late=0 で連鎖は起きていない。SIGTERM で 580,099,848 ヒット(Recorder の受信数と同じ)を書いて閉じ、名前は仮の `run_inprogress_1795424507.root` のまま残った。ROOT で開くと kRecovered=0、`delila` ツリーは 580,099,848 エントリー。ユーザーの依頼で `run0021_0000_11B_208Pb.root` に名前を変え、`run0021_0000_11B_208Pb_eventDef.json` を `config/event_def_11B_208Pb.json` から作った(11-20 から変わっておらず、run 20 の付属ファイルと同じ)。finalize がするのは、この名前の変更と付属ファイルの作成だけ。run 21 の Mongo 記録は status=error。
- sink の events が run 16〜21 で 0 件(run 21 は 1 件)なのは、ThGEM が稼働していなかったため(ユーザー)。
- **LINGER=0 の修正(ユーザー承認)**: `src/operator/client.rs` の REQ ソケットに `.set_linger(0)` を入れた。テスト `timed_out_request_to_absent_component_does_not_linger` は、誰も聞いていないアドレスへの要求が 3 回タイムアウトした後、ZMQ コンテキストを 5 秒以内に破棄できることを確かめる(`zmq_ctx_term` は、残ったソケットを永久に待つ)。修正前は失敗、修正後は 0.31 s で通る。Mac では fmt、clippy `--tests -D warnings`、全テストが通った。pf-qaq には client.rs だけを入れた(Mac のツリーは TODO 91 などで先に進んでいるため)。pf-qaq の Operator バイナリは 11-17 のビルドで、その後に変わったソースは devtree_paths.rs(定数を一つ追加)と read_loop_dig1.rs だけ。控え: `~/client.rs.bak.20261123`、`~/operator.bak.20261117`。**13:14 の DAQ 再起動で入った**(logs/20261123_131452、ユーザーの依頼。Configure(run 22)で 5 台とも PLL locked、Operator の fd は 37)。pf-qaq でのビルド(13:10、Operator のみ、Linux でもテストは通った)には `CAEN_PREFIX=/opt/delila-caen` と `BINDGEN_EXTRA_CLANG_ARGS="-I/usr/lib/gcc/x86_64-linux-gnu/13/include"` が要る(libclang 18 の組み込みヘッダーが入っていない。build.rs が走り直すと bindgen が `stdarg.h` で止まる)。FELib のバインディングは 11-17 と同じ(sha 76e2f383…)。
- **コアダンプ(ユーザー承認)**: start_daq.sh は reader だけを `ulimit -c unlimited` のサブシェルから exec で起動する(`$!` は reader の PID のまま。ほかのコンポーネントは 0 のまま。Recorder のコアはキューの 40 GB を抱えうるため)。apport は、パッケージに入っていないプログラムのコアを `/var/lib/apport/coredump/` に残す(所有者 daq、0400。わざと落ちるプログラムで確かめた)。run 25 の reader 5 台には `prlimit --core=unlimited` で付けた。次に落ちたら gdb の backtrace で、呼び出し元が `read_data` か manual_flush(`set_value`)かを見る。
- **guard(ユーザー承認)**: `digitizer_guard.rs` に `Fault::ReaderOffline` を足した。Running 中に source の返事がないまま `OFFLINE_CONFIRM` = 30 s 続いたら、drain-first stop をかける(ほかの故障の確認時間は 10 s のまま。返事 1 回分のタイムアウトが 5 s あるため長めにした)。ポーリングでどのコンポーネントも返事をしないときは数えない(Operator 側の問題で、Stop も届かない)。drain-first stop は、死んだ reader への Stop が失敗しても続き、Merger/Recorder/Monitor の排出を待ってからランを Error として閉じる(pf-qaq の status.rs で確認)。テストを 4 件足した(合わせて 11 件)。digitizer_guard.rs は変更前は pf-qaq と同じだったので、そのまま入れた。控え: `~/digitizer_guard.rs.bak.20261123`、`~/operator.bak.20261123_1310`。
- 本当の直し方は「無効チャンネルのイベントは何もしない」確認。CAEN の digiTES(CAEN がくれたアプリケーション)は ch enable を配列に持ち、false なら何もしない(ユーザー)。CAEN に報告しても「その方法でやってくれ」と言われるだけ(ユーザー)。ただし今回その確認が要る場所は CAEN の FELib Dig1 プラグインの中: reader → libCAEN_FELib → libCAEN_Dig1 → libCAENDPPDigitizer(これを NEEDED にしているのは libCAEN_Dig1 だけで、reader のバイナリは必要としていない)。delila-rs は GetDPPEvents/MallocDPPEvents を呼ばない(CAENDigitizer を使うのは V1743 の経路だけ)。RAW のバイト列を受け取るだけで、チャンネルごとの配列を持たない。全チャンネル有効は、digiTES の配列がいつも全部 true なのと同じ効果。報告するかはユーザー判断。報告するなら: libCAENDPPDigitizer.so.0.0.0(2025-08-26)、libCAEN_Dig1.so.0.0.0、FELib 1.3.2、オフセット 0x26242(PSD)/0x26eac(PHA)、NULL+0x10 への書き込み、無効チャンネルのあるボードでだけ起きる。
- `/` の使用量 1.3 TB(空き 2.1 TB)。ほぼ全部が `logs/20261122_101611`(1.14 TB、RUST_LOG のフィルタを既定にする前の rollover 警告の洪水)。もう増えていない。削るかはユーザー判断。
- 残り: ① Recorder が EOS の後に受け取った 1,617 イベントの計上 ② run 17 の ROOT を .delila から作り直す。

## 6. 初日の手順(案)

1. 全ボード Configure、PLL locked(TODO 85)、同期スタートの確認
2. パルサーでボード間の時刻合わせ(§4.1)
3. Grafana でチャンネルごとのレート、`narms` で ThGEM ごとの 4 本そろう割合
4. 線源またはビームで `delayline_offsets.C` → 時刻オフセット表 → XY 中心が 0 に
5. ビーム 1/100 で T が測れていること、`tof`・`dt_pair` にピーク
6. 物理ラン

## 7. 要確認(ユーザー) — 2026-11-17 回答済み

1. デジタイザ → V1730 × 3 枚が多分使える
2. VX2730 → N_Wall のまま
3. dt ペアの基準 → RF。組の選び方 → **28 組すべて**(ユーザー: 向かい合わない組でも運動量の保存で角度が決まるので、
   どの組が要るかは何とも言えない)。28 組でも TH2D 500 × 500 で約 56 MB
4. イベントの窓 → 前 500 / 後 1500 ns(もっと短くてもよさそう)
5. side3 への転送 → 運ぶ
6. eLog → `11B_208Pb`
