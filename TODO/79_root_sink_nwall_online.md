# TODO 79 — root_sink の N_Wall 用オンライン表示(TOF・dE/E 相関)と共通の対応表

**Status: 📋 起票 (2026-10-29)**

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
  - `index` = リングまたは Sector の番号
- 置き場所: `config/channel_map_N_Wall.txt`(Grafana と同じファイル)
- root_sink(C++)で読む。行ごとに `#` を落として `std::istringstream`。

## 3. オンラインの表示(案)

| 表示 | 作り方 |
|---|---|
| 中性子の TOF | 液シン(VX2730)の時刻 − 直近のバンチの時刻。バンチの時刻は、V1730 ch15 の 1/100 ビームから**周期と位相をその場で当てはめて**復元(TODO 73 §3.3: 実データで T = 199.980 ns、4 µs で R = 0.997)。液シンごと、または全部の和 |
| dE リング × E リング | 同じイベント内の dE リング番号 × E リング番号(2 次元) |
| dE Sector × E Sector | 同じイベント内の dE Sector 番号 × E Sector 番号(2 次元) |
| (あると便利) | dE エネルギー × E エネルギー(粒子識別)、LaBr3 のエネルギー、液シンの PSD × エネルギー |

- **イベントの組み方:** TODO 71 のイベント定義(v1.1)。トリガーは Si の Sector(タグに `[module, 0, 15]` でまとめる)など。
  窓の中の液シン・ビーム・LaBr3 を同じイベントに入れる。
- **N_Wall 用の root_sink の設定:** ThGEM 用(`module = 0` に限定)とは別にし、全ボードを見る。
- **位相の復元のコード**は、オフラインの解析ツールと共用する(`time_offsets.hpp` と同じく単一ヘッダー)。
  ラン 217 の ch11(ビーム × ThGEM)と、本番の 1/100 ビームで検証する。

## 4. 要確認(ユーザー)

1. リングの番号の付け方(15 × 3 の inner / middle / outer をまとめて 0〜44 にするか、`(group, 0〜14)` にするか)
2. イベントのトリガー(Si の Sector だけか、液シン単独のイベントも作るか)
3. TOF は液シンごとに見たいか、全部の和で十分か
