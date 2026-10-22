# TODO 76 — 中性子検出器の HV(CAEN HV クレート)を読み出して Grafana に出す

**Status: 📋 起票 (2026-10-22) — 型番は SY5527 と確定。IP が不明のため後回し(ユーザー)**

**2026-10-22 回答:** クレートは **SY5527**。既存の `sy5527_monitor.py` がそのまま対象になる
(汎用化は必須ではなくなった)。IP が分かるまで着手しない。

**関連:** [73_campaign_2026_11_pfqaq.md](73_campaign_2026_11_pfqaq.md)(親) /
[74_grafana_event_rate_monitor.md](74_grafana_event_rate_monitor.md) /
[tools/hv_monitor/sy5527_monitor.py](../tools/hv_monitor/sy5527_monitor.py) /
`docker/grafana/provisioning/dashboards/json/hv_{monitor,vmon,imon}.json`(SY5527 用の 3 枚)

---

## 1. 発端 (2026-10-22)

中性子検出器(液シン 24 台、VX2730)の HV を **CAEN の HV クレート**から読み出し、
InfluxDB 経由で Grafana に出したい(ユーザー)。

## 2. 今あるもの

- `tools/hv_monitor/sy5527_monitor.py`: CAENHVWrapper を Python の ctypes で呼ぶ読み出し専用モニタ。
  `CAENHV_InitSystem`(TCP/IP)→ スロットごとに `VMon` / `VSet` / `IMon` / `Status` / `Pw` /
  チャンネル名を読み、`sy5527_hv` テーブルに書く。Fission2026(puppex)で稼働実績あり。
- **システム種別が SY5527 に固定**(`SY5527 = 3`)。CAENHVWrapper は SY1527 / SY2527 / SY4527 /
  SY5527 / R6060 / N1470 などに対応しているので、種別を設定ファイルで選べるようにすれば他の型番にも使える。
- ダッシュボード 3 枚(`sy5527-hv-monitor` / `-vmon` / `-imon`)は pf-qaq の Grafana にもう入っている。

## 3. やること(案)

1. IP・スロット構成を確定する(§4)。
2. CAENHVWrapper を pf-qaq に入れる(CAEN の配布物。sudo が要る。リポジトリには入れない —
   CAEN の著作物は入れない方針)。
3. pf-qaq 用の設定(IP、スロット、ライブラリの場所)を作る。型番が SY5527 なので、
   システム種別の汎用化は後でよい(他の型番が来たときに行う)。
4. **ログイン情報を設定ファイルから外す**(§5)。環境変数か、リポジトリ外のファイルから読む。
5. 実機で確認 → launcher に登録して常駐させる。

## 4. 要確認(ユーザー)

1. ~~クレートの型番~~ → SY5527(回答済み)。中性子用のボードのスロット・チャンネル数
2. **ネットワーク**: クレートの IP(不明、後回し)。実験場のネットワーク上か、pf-qaq に直結か
   (直結なら eno1 は VX2730 で使用中なので、スイッチを挟むか別のポートが要る)
3. 読み出しだけでよいか(電圧の設定はクレート側で行う前提)
4. 他のソフト(CAEN の GECO など)も同時に接続するか

## 5. セキュリティ(既存の問題、2026-10-22 発見)

公開リポジトリの `tools/hv_monitor/sy5527_config.yaml` に、**SY5527 クレートのログインパスワードが平文でコミットされている**
(Fission2026 のクレート用)。対処の候補:

- 設定ファイルからパスワードを外し、環境変数またはリポジトリ外のファイルから読むようにする(この TODO で実施)
- 該当クレートのパスワードを変更する(履歴に残っているため、ファイルを消しただけでは消えない)
- 履歴からの削除(force push が要るので、やるかどうかはユーザー判断)
