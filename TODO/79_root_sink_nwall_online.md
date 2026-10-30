# TODO 79 — root_sink の N_Wall 用オンライン表示(TOF・dE/E 相関)と共通の対応表

**Status: 🚧 nwall_sink v1 実装・合成データで E2E 済み (2026-10-30) — 残 = 同期マーク、pf-qaq への配備(N_Wall の TOML と同時)、実機での確認と較正**

**関連:** [73_campaign_2026_11_pfqaq.md](73_campaign_2026_11_pfqaq.md) §3.1(10 台構成)・§3.3(ビームの位相の復元) /
[71_generic_event_building.md](71_generic_event_building.md)(イベント定義 v1.1: タグの範囲指定、L2) /
[74_grafana_event_rate_monitor.md](74_grafana_event_rate_monitor.md) §3.2(対応表の書式) /
[72_digitizer_time_offset_calibration.md](72_digitizer_time_offset_calibration.md)(ボード間のずれ)

---

## 1. 発端 (2026-10-29)

N_Wall のオンライン表示は root_sink で行い、Monitor は簡単なものだけにする(ユーザー)。見たいもの:

- **中性子の TOF**(見られるなら)
- **dE リング × E リング の相関**
- **dE Sector × E Sector の相関**

対応表(module, ch → リング / Sector)は 1 本にまとめ、Grafana・root_sink・オフライン解析が同じ表を読む(ユーザー賛成)。

## 2. 対応表の拡張

Grafana 用の書式 `module channel name group`(空白区切り、`#` コメント、TODO 74 §3.2)の **5 列目以降**に root_sink 用の列を足す。
Grafana の生成スクリプトは 5 列目以降を読み飛ばすので、そのまま共用できる。

- 案: `module channel name group det kind index`
  - `det` = `dE` / `E` / `LS` / `LaBr` / `beam` / `pulser`
  - `kind` = `ring` / `sector` / `-`
  - `index` = リングまたは Sector の番号。**リングは inner / middle / outer をまとめた通し番号 0〜44**(相関を見やすいため、ユーザー)、Sector は 0〜15
- 置き場所: `config/channel_map_N_Wall.txt`(Grafana と同じファイル)
- root_sink(C++)で読む。行ごとに `#` を落として `std::istringstream`。

## 3. オンラインの表示(案)

| 表示 | 作り方 |
|---|---|
| **全チャンネルの TOF** | 各 ch のヒットの時刻 − 直近のバンチの時刻(周期 T で折り返し)。バンチの時刻は、V1730 ch15 の 1/100 ビームから**周期と位相をその場で当てはめて**復元(TODO 73 §3.3: 実データで T = 199.980 ns、4 µs で R = 0.997)。**液シンに限らず全モジュールの全 ch を作る**(ユーザー。液シンはおまけ扱い)。例: 横軸 TOF × 縦軸 通しの ch 番号の 2 次元 1 枚 + 必要な ch の 1 次元 |
| dE リング × E リング | 同じイベント内の dE リング番号 × E リング番号(2 次元) |
| dE Sector × E Sector | 同じイベント内の dE Sector 番号 × E Sector 番号(2 次元) |
| (あると便利) | dE エネルギー × E エネルギー(粒子識別)、LaBr3 のエネルギー、液シンの PSD × エネルギー |

- **イベントの組み方:** TODO 71 のイベント定義(v1.1)。トリガーは Si の Sector(タグに `[module, 0, 15]` でまとめる)など。
  窓の中の液シン・ビーム・LaBr3 を同じイベントに入れる。
- **N_Wall 用の root_sink の設定:** ThGEM 用(`module = 0` に限定)とは別にし、全ボードを見る。
- **位相の復元のコード**は、オフラインの解析ツールと共用する(`time_offsets.hpp` と同じく単一ヘッダー)。
  ラン 217 の ch11(ビーム × ThGEM)と、本番の 1/100 ビームで検証する。

## 4. 要確認(ユーザー)

回答済み(2026-10-29): リングは通し番号 0〜44 / 液シンはおまけでよいが、**TOF は全チャンネル分作る**。

残り:

1. イベントのトリガー(Si の Sector だけか、液シン単独のイベントも作るか)

## 5. 検討 (2026-10-30)

### 5.1 決まったこと(ユーザー)
- **N_Wall の root_sink は ThGEM とは別のプログラムにする。** 実験ごとに個別に作り、成熟した段階で最大公約数的な部分を
  ライブラリにする。`tools/root_sink/` の構成をそれを見越したものにする(§5.3)。
- **Si の構成が変わった:** dE = リング + Sector、**E = Sector のみ**(E のリングはなし)。→ V1725 は dE リング 3 台
  (ch15 = パルサー)+ dE Sector 1 台 + E Sector 1 台の 5 台。**E 側にはパルサーを入れる空き ch がない。**
- **イベントのトリガー = Si の Sector。** dE vs E の 2 次元(粒子弁別)が最重要で、dE と E の両方がないイベントは意味がない。
- **表示は並べ替えたあとの hit と組んだイベントを流す**(N_Wall の root_sink で。ThGEM 用は今のまま)。了承済み。
- 飛行距離は未定(固定されていない)。Si の対応表(リング・Sector の結線)は、事前に分かる見込みは薄い。

### 5.2 TOF の方針(案)
- **液シン 24 + γ(LaBr3)7 + ビームパルス 1 を VX2730 にまとめる**(ユーザーの案、32 ch がちょうど埋まる)。
  → ビームに対する TOF は 1 台の中で閉じる: ボード間のずれも、ランごとの 8 ns の跳び(TODO 77)も入らない。
  γ フラッシュで液シンごとの t0(ケーブル)を合わせる(時刻オフセットのファイル、定数)。
- **Si はバンチを選ぶだけ:** dE/E のイベントでは、Si の時刻から「どのバンチで起きた反応か」を決め(バンチ間隔 200 ns
  なので ±100 ns まで許される)、液シンの TOF = t(液シン) − t(そのバンチ)。精度は VX2730 だけで決まり、Si 側の
  8 ns の跳びやケーブルの差は効かない。**200 ns を超える遅い中性子でも折り返さない**(シングルの TOF は 200 ns で折り返す)。
- バンチの時刻: VX2730 のビームの hit(1/100 なら 20 µs ごと)のうち一番近いもの + 周期の整数倍。周期はランごとに
  ビームの hit から求める(ラン 217: T = 199.980 ns)。
- Si ↔ VX2730 のずれは ±50 ns 程度で分かれば足りる: Si と同時の即発 γ(LaBr3 / 液シン)か、§5.4 の同期マークで測る。

### 5.3 `tools/root_sink/` の構成(**2026-10-30 実施、7d33b8b**)
```
tools/root_sink/
  common/    実験に依らない部分(ライブラリの候補): sink_core / eb_core / event_def / time_offsets / time_map の各ヘッダー、
             その単体テスト、test_publisher.cxx、time_map.C、compare_hits.C、check_events.C
  standard/  root_sink(名前はそのまま。ThGEM と ELIADE が使う): root_sink.cxx、hist_config.hpp、histograms.json、test_hist_config.cpp
  thgem/     ThGEM のオフライン解析: thgem_reco.hpp、thgem_events.C、delayline_offsets.C、test_thgem_reco.cpp
  nwall/     N_Wall 用 nwall_sink(これから)
```
- **今の `root_sink` は ELIADE も使っているので、名前は変えない**(ユーザー選択)。移動だけで中身は変えていない。
  テストの件数は分割の前後で同じ(754 = 647 + 107、257 = 232 + 25、244)。root_sink・test_publisher・全マクロ(delila2root.C を含む)が新しい場所からコンパイルできることを確認。
- `sink_core.hpp` には standard の Δt / XY の照合がまだ入っている(ライブラリにするときに standard/ へ移す)。
- 受信・並べ替え・イベント組み・記録(Recorder、書き込み失敗の処理、来歴)は、nwall_sink では standard からの写しで始める
  (当面は重複。N_Wall が回ったあと、ライブラリにする最初の候補)。
- 起動: `[network.root_sink]` にプログラムを選ぶキーを足し、`start_daq.sh` が選ぶ(nwall_sink を作るときに)。

### 5.4 同期マーク(コモントリガー、案)
- VX2730 の **Test Pulse**(周期 8 ns 刻み・最大約 34 s、TRG-OUT / GPIO に NIM/TTL で出せる、DevTree で確認)を 1 Hz で出し、
  FAN-OUT で 5 台の V1725 の TRG-IN へ。V1725 は `trg_ext_enable = TRUE` で全 ch が同じクロックで 1 件ずつ記録する
  (DT5730B の DevTree にある。**VME の V1725 で受け付けるかは実機で確認**)。VX2730 自身は 1 ch のトリガー元に TRGIN を足す。
- データの中で見つけるのは簡単: 1 台の全 ch が同じ時刻、エネルギーはほぼ 0、ちょうど 1 秒おき。
- 取りこぼしへの影響: 1 ch あたり 1 秒に 1 回 × 不感時間(µs)≈ 10⁻⁵。
- 注意: (i) そのままだと E Sector が全部鳴った偽の dE∧E イベントになる → N_Wall の root_sink でイベントを組む前に取り除き、
  別に記録する。(ii) Reader の `adc_min` が 0 より大きいと、エネルギーほぼ 0 のマークは Reader で落ちる。
- 使い道: Si ボードのランごとのずれ(8 ns の跳び)、**どのボードが跳んだかの特定**(TODO 77 で保留した問い)。
  §5.2 の方式なら物理の結果には要らないので、保険・診断の位置づけ。

### 5.5 回答 (2026-10-30)
1. **VX2730 = 液シン 24 + LaBr3 7 + ビーム 1 で確定。V1730 は N_Wall で使わない。** → N_Wall は 6 台(VX2730 + V1725 × 5)。
2. **トリガー = E Sector だけ、dE は必須の条件(L2)。** E が鳴れば高い確率で dE も鳴るので問題なし。
3. 同期マーク: ロジック信号用の FAN-OUT があるはずなのでできる。ただしランコントロール(スタート信号の分配)にも要るので、
   数をユーザーが確認する。
4. 構成とプログラム名は OK。**ただし今の `root_sink` は ELIADE(`config_sn01_*.toml`、`config_sn02_*.toml`、
   `config_sn05_*.toml`)も使っている**(2026-10-30 に判明)→ 名前を `thgem_sink` に変えると ELIADE の全ノードの起動が
   変わる。§5.3 を見直してユーザーに確認中。

### 5.6 VX2730 のチャンネル (2026-10-30、ユーザー)
- **ビーム 0、γ(LaBr3)1〜7、液シン 8〜31**(変わるかもしれない)→ nwall_sink は役割を**対応表から読む**(チャンネル番号に決め打ちしない)。
- モジュール番号の案(異議なし): 0 = VX2730、1 = dE Sector、2〜4 = dE リング(inner / middle / outer)、5 = E Sector。

## 6. 実装 v1 (2026-10-30)

| もの | 場所 |
|---|---|
| nwall_sink 本体(受信・並べ替え・イベント組み・記録は standard の写し、表示は N_Wall 用) | `tools/root_sink/nwall/nwall_sink.cxx` |
| 対応表の読み込み(Grafana の 4 列 + `det kind index`、行番号つきで全エラー列挙、ラン開始ごとに読み直し・失敗時は前の表を保持) | `nwall/nwall_map.hpp` |
| バンチの時計(一番近いビームの hit + 周期の整数倍、周期はランごとに測る、格子外の間隔とビーム停止を除外) | `nwall/bunch_clock.hpp` |
| 単体テスト 56 本 | `nwall/test_nwall.cpp` |
| 合成データ(6 台、ビーム 1/100、dE/E、中性子と即発 γ、液シン単独、LaBr3、パルサー) | `nwall/make_test_run.C` |
| イベント定義(E Sector トリガー、L2 で dE 必須を印) | `config/event_def_nwall.json` |
| 対応表(暫定の結線で記入、112 ch) | `config/channel_map_N_Wall.txt` |
| 起動: `[network.root_sink] program = "nwall_sink"`(既定は root_sink)、`channel_map` などのキー | `scripts/start_daq.sh`、`stop_daq.sh` |

- **表示の経路:** Writer から、並べ替えた core の hit と組んだイベントを有界のキューで表示へ(落としてよいのは表示だけ、数える)。
  root_sink の「受け取り順の分岐」は nwall_sink には無いので、表示用に落とす箇所は 1 つのまま。
- **表示:** `/hits`(hitmap、ボードごとのエネルギー)、`/rf`(ボードごとの RF TOF)、`/ls`(液シンの PSD)、
  `/events`(pid = dE vs E、dE Sector × E Sector、dE リング × dE Sector、液シン − Si、si_phase、Si が選んだバンチからの液シンの TOF)。
- **較正:** `si_delay_ns` = si_phase のピーク。`rf_t0_ns` = γ フラッシュを L/c に置くずれ(チャンネルごとの差は時刻オフセットへ)。

### E2E(2026-10-30、Mac、合成データ 2 秒・273,824 hit)
- 全 hit を記録、イベント 3,939(全部 dE あり)、**check_events.C で IDENTICAL**。
- 周期を 199.9500 ns と測定(合成データの真値。名目は 199.98)。si_phase のピーク 15 ns(合成の Si 遅れ)。
  LaBr3 は 3 ns、γ フラッシュは 5.0 ns(1.5 m の L/c)、中性子は 40〜160 ns に。dE Sector × E Sector は 99.95 % が対角。
- **途中で見つけた不具合:** バンチの時計がビームの hit を 8192 個(約 160 ms)しか覚えず、200 ms のチャンクの前半で
  バンチが引けなかった(RF TOF が 54 % しか埋まらない)→ 時間で忘れる方式(処理中のチャンクの 10 ms 前まで保持)に直し、
  テストを追加。直したあとは全 hit に RF TOF。
- 同じデータを 2 ラン続けて流しても同じ結果(ランごとに時計がリセットされる)。SIGTERM で正常終了。

### 残り
1. 同期マーク(§5.4): FAN-OUT の数の確認待ち。やるなら、イベントを組む前にマークを除く処理を足す。
2. N_Wall 用の TOML(`config/config_nwall_pfqaq.toml`、6 台)と pf-qaq への配備(nwall_sink のビルドは
   `tools/root_sink/nwall/README.md`)。
3. 実機での確認: パルサーやビームでの表示、`si_delay_ns` と `rf_t0_ns` の較正、低レートの DIG1 ボードの遅れ(`late=`)。

