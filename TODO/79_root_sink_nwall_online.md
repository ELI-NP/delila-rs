# TODO 79 — root_sink の N_Wall 用オンライン表示(TOF・dE/E 相関)と共通の対応表

**Status: 📋 検討中 (2026-10-30: 前提・トリガー・TOF の方針を議論。実装は未着手)**

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

### 5.3 `tools/root_sink/` の構成(案)
```
tools/root_sink/
  common/   実験に依らない部分 = 今すでにそうなっているもの(移動のみ、中身は変えない)
            sink_core.hpp(ThGEM の照合は thgem/ へ)、eb_core.hpp、event_def.hpp、time_offsets.hpp、time_map.hpp、
            単体テスト、test_publisher.cxx、time_map.C、compare_hits.C
  thgem/    ThGEM 実験 = 今の root_sink(本体、hist_config.hpp、照合、thgem_reco.hpp、thgem_events.C、delayline_offsets.C)
  nwall/    N_Wall 実験(新規): 本体は ThGEM の本体の写しから始め、表示を N_Wall 用にする
```
- 受信・並べ替え・イベント組み・記録(Recorder、書き込み失敗の処理、来歴)は、当面 thgem と nwall で重複する。
  N_Wall が回ったあと、ライブラリにする最初の候補。
- 起動: `[network.root_sink]` にどのプログラムを使うかのキーを足し、`start_daq.sh` が選ぶ。

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

### 5.5 残る問い
1. VX2730 = 液シン 24 + LaBr3 7 + ビーム 1 で確定か(V1730 は N_Wall で使わない?)
2. トリガー: E Sector だけ(dE は L2 で必須)か、dE Sector もトリガーにするか
3. 同期マーク(§5.4)をやるか(FAN-OUT があるか)
4. `tools/root_sink/` の構成(§5.3)とプログラム名(例: `thgem_sink` / `nwall_sink`)

