# TODO 73 — 2026-11 実験キャンペーン(pf-qaq: Si + 中性子 → ThGEM + 中性子)

**Status: 📋 起票 (2026-10-22) — 計画と準備状況の親チケット。個別の作業は 74〜76**

**関連:** [74_grafana_event_rate_monitor.md](74_grafana_event_rate_monitor.md) /
[75_mesytec_hv_monitor.md](75_mesytec_hv_monitor.md) /
[76_caen_hv_crate_monitor.md](76_caen_hv_crate_monitor.md) /
[config/config_thgem_pfqaq.toml](../config/config_thgem_pfqaq.toml) /
`docs/pfqaq_operations_overview.pdf`(運用の概要、2026-10-07 時点)

---

## 1. 計画(ユーザー、2026-10-22)

| 時期 | 実験 | 検出器 |
|---|---|---|
| **11 月中旬** | 1 本目、`experiment_name` = **`N_Wall`** | **Si + 中性子(液シン 24 台)** のみ |
| その後 | 2 本目以降 | **ThGEM + 中性子** + 細かいもの |

**Si(2026-10-22 回答):** 今はチャンネル 1 つだけ(**module 0 = V1730 SN66 の ch 1**)。11 月中旬までに
**S1 か S7 を 2 枚**にする。読み出しは別に持ってくる **V1725(DPP-PHA、16 ch)× 8 台 = 128 ch**(当初「6 台」と回答 → 同日に 8 台と訂正)


**Si のチャンネル(同):** E と dE の 2 層で、**それぞれ リング 15 × 3(inner / middle / outer)+ セクター 16 = 61 ch**、
**計 122 ch** → V1725 × 8 = 128 ch に収まる(空き 6 ch)。

**時間相関(同):** **Si と中性子の同時計測は要る。** V1725 × 6 と VX2730 のクロック・スタートの結線と設定は
ユーザーが行う。

**root_sink(同):** ThGEM は使わないので、`N_Wall` では**別の root_sink**(ThGEM 用の設定は使わない)。
いずれ本格的なイベントビルダーにしたいが、今は後回し(土台は TODO 71 の汎用層)。

**実験名(同):** `N_Wall` で進める(変わることもある)。

**HV(同):** Si = **Mesytec MHV-4 が 1 台**、中性子 = **CAEN SY5527**(IP 不明のため後回し)。

準備はほぼ終わっている(ユーザー)。残りの柱は次の 2 つ。

- **Grafana をイベントレートモニタとしてちゃんと使う** → [TODO 74](74_grafana_event_rate_monitor.md)
- **HV の読み出し**: Si は Mesytec のモジュール → [TODO 75](75_mesytec_hv_monitor.md)、
  中性子は CAEN の HV クレート → [TODO 76](76_caen_hv_crate_monitor.md)。
  どちらも InfluxDB に書いて Grafana に出す

## 2. 準備状況(2026-10-22 時点)

| 項目 | 状態 |
|---|---|
| DAQ PC | pf-qaq(daq@192.168.144.142、DHCP)。`/data`(NVMe)+ `/archive`(ZFS)+ `archive_sync.sh` 稼働中 ✅ |
| 中性子(液シン 24 台) | VX2730 DPP-PSD = source 1(`PSD2-VX2730-LS`)。eno1 直結(pf-qaq 172.18.4.50/22 ↔ 2730 172.18.4.57)、10-20 に Configure 成功(47 パラメータ、0 エラー)。値は液シン向けの初期値のまま(未チューニング)、32 ch 全部有効。**未コミット** |
| Si | 今は 1 ch(module 0 ch 1)。11 月中旬に **V1725 PHA × 8 台(128 ch、使うのは 122 ch)**で読む。設定ファイル・結線は未着手 |
| ThGEM | V1730 SN66 = source 0、root_sink(`module = 0`)、時刻オフセット(TODO 70) |
| Grafana | データは来ているが、長い期間が表示できない・ダッシュボードが Fission2026 向けのまま → TODO 74 |
| HV(Si / 中性子) | 未実装 → TODO 75 / 76 |
| Monitor の 2D Save Image | 修正して pf-qaq に配備済み(10-20)。**未コミット** |
| リポジトリ | **未 push のコミット 24 個 + 未コミットの作業**(2730 設定、UI 修正、pha_fw_scan、ポスター、運用 PDF)。実験前に片付ける |

## 3. 実験ごとの DAQ 設定(案)

1 本目(Si + 中性子)と 2 本目(ThGEM + 中性子)で、使うボード・root_sink・`experiment_name` が変わる。
`config.toml` のシンボリックリンクを切り替える運用(SN01/SN02 と同じ)にして、TOML を実験ごとに分ける案:

- `config/config_nwall_pfqaq.toml`(1 本目、`experiment_name = "N_Wall"`): V1725 PHA × 8 + VX2730。
  root_sink は ThGEM 用とは別の設定(当面はヒットの記録とライブヒストグラム程度、本格 EB は後回し)

### 3.1 `N_Wall` の DAQ 構成(案)

| source | ボード | 接続 | 用途 |
|---|---|---|---|
| 0〜7 | V1725 DPP-PHA × 8(16 ch、4 ns サンプリング、`type = "pha1"`、`time_step_ns = 4.0`) | A3818 の光リンク(4 本)。1 リンクに 2 台ずつデイジーチェーン(`conet_node` 0–1)で帯域を分散 — ユーザーも「それが一番」。Si は低レートなので余裕はある。振り分けは結線に合わせて TOML の `link_num` / `conet_node` を書くだけ(ユーザー: 問題なし) | Si(S1/S7 × 2、E / dE、122 ch) |
| 8 | VX2730 DPP-PSD(今の source 1 を番号替え) | eno1 直結(172.18.4.57) | 中性子(液シン 24 台) |

- V1730 SN66(今の Si 1 ch)を `N_Wall` で使うかは未定(ユーザー)。VX2730 にも空きが 8 ch(ch 24–31)ある。
- 検出器名は実験前には付けない。Grafana は `Mod, Ch` で始め、対応が分かったら付ける(TODO 74 §3.1)。

### 3.2 時間相関の確認(DAQ 側でやること)

結線と設定はユーザー。DAQ 側では次を用意・確認する。

1. スタートの揃え方: DIG1 の V1725 同士(S-IN / 光リンク経由)と DIG2 の VX2730 の両方が、同じ合図で
   タイムスタンプを 0 にするか。過去の記録では、DIG1 の S-IN はレベルトリガーで、ボード間に約 650 ms の
   ずれが出た例がある(Operator の `arm_all_sync` で HW の Armed を待つようにした経緯あり)。
2. ボード間の時刻オフセット: パルサーを全ボードに入れたランで `tools/root_sink/time_map.C`(TODO 72)を回し、
   `timeSettings.json`(TODO 70)を作る。
3. V1725 で波形を録る場合は record length に注意(V1725 は 7168 ns 以上でカップル内の同時イベントが片方消える。
   DIG1 のカップル アービターの件)。
- `config/config_thgem_pfqaq.toml`(2 本目): 今のもの + VX2730

切り替えは Operator の再起動が必要(設定は起動時に読む)。ラン番号は `experiment_name` ごとに続く。

## 4. 要確認(ユーザー)

回答済み(2026-10-22): 日程 = 11 月中旬、`experiment_name` = `N_Wall`(変わることもある)、
Si = V1725 PHA × 6 台(今は module 0 ch 1 の 1 ch)、Si と中性子の時間相関あり(結線・設定はユーザー)、
ThGEM は使わず root_sink は別設定(本格 EB は後回し)、HV = MHV-4 × 1(読み出しのみ)と SY5527(IP 不明)。

回答済み(同日追加): V1725 は **8 台**(128 ch)で Si 122 ch が収まる。光リンクの振り分けは
TOML の番号を合わせるだけなので問題なし。V1730 を使うかは未定。Si は E / dE それぞれ 61 ch(計 122 ch)。

残り:

1. ~~チャンネル対応表と検出器名~~ → **実験前には決まらない**(ユーザー: いつも実験後に解析しながら正解の対応表を作る)。
   Grafana は `Mod, Ch` で始め、名前は後から付ける。やり方は TODO 74 §3.1
2. 2 本目の「細かいもの」の中身 → **完全に未定**(ユーザー)。決まった時点でこの TODO に足す
