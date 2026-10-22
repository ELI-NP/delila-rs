# TODO 74 — Grafana をイベントレートモニタとして使えるようにする(pf-qaq)

**Status: 🚧 P1〜P5 実装済み (2026-10-22) — P1 + 匿名 Viewer 化は pf-qaq に配備済み。P2/P5(Operator)は pf-qaq でのビルドまで、有効化は Operator の再起動待ち(データ取得中のためユーザー判断)**

**P2〜P5 実装記録 (2026-10-22):**
- P2/P5: `src/operator/influxdb.rs` — `RateTracker`(実測の経過時間で割る、初回は rate なし、カウンタが戻ったら 0)、
  `RunEventDetector`(初回ポーリングは記録のみ、start / stop / ラン番号の変化)、`run_event,exp=… run=…u,event="start|stop"`
  (実験名が空なら `exp` タグなし)。テスト 18 本、`cargo test --lib` 全通過、clippy 警告 0。
  スキーマは `docs/plans/grafana_monitoring.md` に反映。
- P3/P4: `tools/grafana/gen_dashboards.py`(標準ライブラリのみ、テスト 25 本)。DAQ の TOML と
  `config/channel_map_<実験名>.txt` から `rates_<実験名>.json` を `docker/grafana/provisioning/dashboards/json/generated/`
  (git 管理外)に生成。中身 = ボード × ch の現在レート表(0 Hz は赤、無効 ch は灰)、group ごとの時系列(名前は SQL で付与 →
  改名は過去にもさかのぼる)、止まっている ch の表、ボードごとの合計レート、ランのアノテーション。
  `delila_overview.json` は期間指定に追従する汎用版に作り直し(データ量は累積値でなく毎秒の差分に修正)、
  Fission2026 の Beam Current は `fission2026_beam.json` へ移した(パネルはそのまま)。
  生成した全 29 種類の SQL を pf-qaq の InfluxDB と Grafana(`/api/ds/query`)で確認。
  注意: Grafana の InfluxDB(SQL)でアノテーションを動かすには target に `query` キーが要る(プラグインのソースで確認)。

**UI からの保存 (2026-10-22、ユーザー: admin でも編集を保存できない、毎回説明するのが面倒):**
`provisioning/dashboards/default.yml` の `editable: true` はプロバイダの設定項目ではなく(正しくは `allowUiUpdates`)、
そのため全ダッシュボードが「Provisioned dashboard」として保存を拒否していた。`allowUiUpdates: true` に変更。
ログインした人は UI でそのまま保存できる(匿名は Viewer なので変わらず閲覧のみ)。UI での保存は Grafana の DB に残り、
**JSON ファイル自体が変わったとき(git pull で更新、対応表を変えて再生成)にファイルの内容で上書きされる**。
同じ対応表での再生成は同一ファイルなので上書きされない。生成ダッシュボードも `editable: true` にした。

**フォルダ構成 (2026-10-22、ユーザー: 実験別):** `foldersFromFilesStructure: true` で `json/` のサブディレクトリ =
Grafana のフォルダ。`Common/`(Overview)、`N_Wall/`(SY5527 と MHV-4 の HV)、生成するレート用は
`json/<experiment_name>/rates_<slug>.json`(`.gitignore` の `json/*/rates_*.json`、旧 `json/generated/` は廃止)。
Fission2026 の 4 枚(Beam Current、SY5527 のスロット別 VMon / IMon、N1408 の Si HV)は実験終了のため削除(git 履歴には残る)。

**P1 配備記録 (2026-10-22、ラン中にユーザー了承のうえ実施):** `docker/docker-compose.yml` に
`INFLUXDB3_QUERY_FILE_LIMIT=50000`(このイメージの `serve --help-all` で `--query-file-limit` / 同 env を確認)、
Grafana の匿名ロールを `Viewer` に、admin パスワードを `${GRAFANA_ADMIN_PASSWORD:-delila}` に。pf-qaq で
`docker compose up -d influxdb grafana`(InfluxDB は約 10 秒で healthy)。確認: `system_rate` の全期間
(2026-10-06〜、515,725 行)が 0.5 秒で返る(以前は HTTP 500)、Operator の書き込みは直後から再開、
匿名の `/api/dashboards/db` 書き込みは 403、`canEdit` = false。**残: admin パスワードの変更(ユーザー)。**

**関連:** [73_campaign_2026_11_pfqaq.md](73_campaign_2026_11_pfqaq.md)(親) /
[docs/plans/grafana_monitoring.md](../docs/plans/grafana_monitoring.md)(2026-02 の設計書) /
[src/operator/influxdb.rs](../src/operator/influxdb.rs)(Operator → InfluxDB の書き込み) /
[docker/grafana/provisioning/dashboards/json/](../docker/grafana/provisioning/dashboards/json/) /
[docker/docker-compose.yml](../docker/docker-compose.yml)

---

## 1. 発端 (2026-10-22)

11 月の実験(TODO 73)では、Grafana をイベントレートモニタとして**ちゃんと**使いたい(ユーザー)。
HV(TODO 75 / 76)も同じ Grafana に載せる。

## 2. 現状(pf-qaq で確認、2026-10-22)

データは流れている: `system_rate`・`channel_rate`(module 0 と 1)・`backlog`・`disk`。
Operator が 2 秒ごとに書いている(`[operator.influxdb] interval_secs = 2`)。問題は次のとおり。

1. **長い期間のクエリが失敗する。** InfluxDB 3 Core は 1 クエリで読む parquet ファイルを
   **432 個まで**に制限している。pf-qaq は 2 週間で 8,990 ファイルになり、
   `SELECT ... FROM system_rate`(全期間)は HTTP 500
   「Query would scan 432 Parquet files, exceeding the file limit」。1 日分なら通る。
   Luna2 のビューアでは `INFLUXDB3_QUERY_FILE_LIMIT=50000` で解決済みだが、
   **リポジトリの compose には入っていない**。
2. **Overview ダッシュボードが Fission2026 向けのまま。** 「Beam Current」=
   `system_rate` の module 2 ÷ 10 が固定で入っている。全クエリが
   `now() - interval '5 minutes'` 固定で、**画面上部の期間指定に追従しない**。
3. **チャンネル別のレートがない。** `channel_rate` には累積の `count` しかない。
   設計書にある `rate` フィールドと `name` タグ(検出器名)は実装されていない。
   チャンネル別のパネルもリポジトリのダッシュボードにはない
   (puppex には手で作ったものがあった。uid `adcwh47`)。
4. **ランの開始・停止が Grafana から見えない**(アノテーションなし)。

## 3. やること(案)

| # | 内容 | 場所 |
|---|---|---|
| P1 | `INFLUXDB3_QUERY_FILE_LIMIT` を compose に入れる(pf-qaq はラン間にコンテナを作り直す) | docker-compose.yml |
| P2 | `channel_rate` に `rate`(実測の経過時間で割る。カウンタが戻ったら 0)を追加。**名前のタグは付けない**(§3.1: InfluxDB 上の識別は module と channel だけにする)。テスト先行 | src/operator/influxdb.rs |
| P3 | ダッシュボードを汎用化: 期間指定に追従、module ごとのレート・総数・データ量・backlog/disk。チャンネル別レート(module 変数 + ch 選択、module×ch のヒートマップ)。Beam Current は Fission2026 専用にして外す | provisioning/dashboards |
| P4 | 実験別のダッシュボード: 「Si + 中性子」「ThGEM + 中性子」。**対応表から生成する**(§3.1)。HV のパネルも同居 | 同上 + 生成スクリプト |
| P5 | ランの開始・停止をアノテーションで表示(Operator から書く) | Operator + ダッシュボード |
| P6(任意) | アラート: Running 中にレートが 0 になった、HV がトリップした | Grafana alerting |

**優先順位(ユーザー了承、2026-10-22):** 11 月中旬の 1 本目(`N_Wall`)までに
P1・P2・P3・P4(「Si + 中性子」)・P5。P6 は後回し。

### 3.1 チャンネル名とダッシュボードの作り方(2026-10-22)

**前提(ユーザー):** チャンネルと検出器の正しい対応は、**実験前には分からない**。いつも実験後に解析しながら
正解の対応表を作っている。これまでの Grafana は `Mod, Ch` の形で表示し、一度回した後に名前を付け替えていた。
名前はあとでユーザーが直すか、ユーザーの対応表をもとに Claude が直す。

**だから要るもの:** 名前を何度変えても壊れず、変えた名前が**過去のデータにもさかのぼって**付くこと。
Grafana の画面で 150 ch(Si 122 + 液シン 24 + …)を手で付け替えるのは現実的でない。

**原則:** InfluxDB には名前を書かない。**データの識別は (module, channel) だけ**にし、名前は表示するときに付ける。
名前をタグとして書くと、改名のたびに系列が分かれ、過去のデータは古い名前のまま残る。

**案:**

| 案 | やり方 | 改名の手間 | 過去への反映 |
|---|---|---|---|
| **A(採用)** | **対応表ファイル(1 枚、§3.2 の空白区切り: `module channel name group`)からダッシュボードを生成するスクリプト。** 名前のない行は `M{m}/Ch{c}`。`group`(例 `dE_inner`、`E_sector`、`LS`)ごとにパネルを分ける。生成した JSON を provisioning に置けば Grafana は再起動なしで読み直す | 対応表を直して生成し直す(ユーザーでも Claude でも) | される(表示時に付けるため) |
| B | 対応表を InfluxDB の小さな表に入れ、ダッシュボードのクエリで JOIN して名前を付ける | 表に 1 行書くだけ | される。ただしクエリが複雑になり、Grafana 上での手直しが難しい |
| C | これまでどおり Grafana の画面で手で付け替える | 150 ch 分の手作業。provisioning で上書きされると消える | 設定次第 |

案 A の補足:

- 最初は対応表が空でも動く(全部 `M{m}/Ch{c}`、group はボード単位)。1 回目のランの後、分かった分だけ書き足せばよい。
- 同じ CSV を後で Monitor(8081)の表示名や解析側の対応表にも使えば、出所が 1 つにまとまる(今回の範囲外)。
- 全体を一目で見るのは module × ch のヒートマップ(レート)。0 Hz のチャンネル(つながっていない・死んでいる)が
  すぐ分かる。個別の時系列は group ごとのパネル。

### 3.2 対応表ファイルの形式(ユーザー指定、2026-10-22)

**CSV ではなく空白区切り**(ユーザー: C++ で読みやすく、解析にもそのまま流用できる)。

```text
# module channel name group        ← '#' から行末まではコメント、空行は無視
0  0  dE_Ri00  dE_inner
0  1  dE_Ri01  dE_inner
8  5  LS06     LS
```

- 1 行 1 チャンネル。列は `module channel name group` の順。区切りは空白(スペース・タブ、いくつでも)。
- `name` と `group` に空白は使えない(`_` を使う)。
- **5 列目以降は生成スクリプトが無視する** → 解析側で使う列(位置、較正係数など)を後ろに足してもダッシュボードは壊れない。
- 表に無い (module, channel) は `M{m}/Ch{c}`、group はボード単位(`M{m}`)。
- 置き場所(案): `config/channel_map_<experiment>.txt`(例 `config/channel_map_N_Wall.txt`)。実験ごとに 1 枚。
- C++ での読み方(解析側): `std::getline` で 1 行読み、`#` 以降を落としてから `std::istringstream >> module >> channel >> name >> group`。

### 3.3 見る人と場所(ユーザー、2026-10-22)

**両方。** 基本は実験エリアの隣のユーザールーム(pf-qaq と同じネットワーク)。オフィスや自宅からも VPN で見る。

ここから出てくる課題(pf-qaq で確認済み):

1. **匿名アクセスが Admin 権限になっている**(`GF_AUTH_ANONYMOUS_ORG_ROLE: Admin`、compose と pf-qaq の実機とも)。
   ユーザールームでも自宅でも、見ている人が誰でもダッシュボードやデータソースを変更・削除できる。
   → **匿名は Viewer(閲覧のみ)にし、編集は admin でログインして行う**(**決定**、ユーザー 2026-10-22:
   「ログインしていないなら閲覧だけ」)。admin のパスワードは
   公開リポジトリの compose に `delila` と書かれているので、pf-qaq では `docker/.env`(git 管理外)で別のものにする。
   → P1 と一緒に 1 本目までにやる(compose の作り直し 1 回で済む)。
2. **pf-qaq のアドレスは DHCP**(192.168.144.142、リース 4 時間)。変わるとブックマークが切れる。
   → **固定割り当ては難しい**(ユーザー)。pf-qaq は**電源を入れっぱなしにする**ので、DHCP の更新で今のアドレスを
   使い続ける見込み。停電や長時間の停止のあとはアドレスが変わりうるので、その時は pf-qaq の画面で
   `ip -br addr show eno2` を見て、ブックマークと Claude の接続先(メモリ)を直す。
3. 見る人が増えると InfluxDB へのクエリも増える。自動更新は 5〜10 秒、既定の表示期間は短め(例: 30 分)にする。

## 4. 要確認(ユーザー)

1. ~~優先順位~~ → 既定でよい(回答済み)
2. ~~チャンネル名の扱い~~ → 案 A、対応表は空白区切り(回答済み、§3.2)
3. ~~見る人と場所~~ → ユーザールーム + オフィス・自宅(回答済み、§3.3)
4. ~~匿名アクセス~~ → Viewer(閲覧のみ)に決定(§3.3-1)
5. ~~IP の固定割り当て~~ → 難しい。つけっぱなしで今のアドレスを使い続ける(§3.3-2)

**要確認は以上ですべて回答済み(2026-10-22)。**

## 5. 注意

- InfluxDB 3 Core にはコンパクションがない(Enterprise のみ)。ファイル数は増え続けるので、
  上限を上げると長期間のクエリは遅くなる(Luna2: 17k ファイルの全期間カウントで約 4.5 秒)。
  保持期間を決めるかどうかも検討する。
