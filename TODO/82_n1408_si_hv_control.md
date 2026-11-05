# TODO 82 — Si の HV(CAEN N1408): モニタ + ch ごとの ON/OFF と設定(Web、パスフレーズつき)

**Status: 🚧 実装・pf-qaq で実機確認済み、ユーザーがブラウザで操作済み (2026-11-05) — 残 = 本番のパスフレーズ、systemd への登録(sudo、ユーザー)、Si をつないでの確認、本物のランでの拒否の確認、Grafana のパネル(TODO 74 P4)**

**関連:** [73_campaign_2026_11_pfqaq.md](73_campaign_2026_11_pfqaq.md)(親) /
[75_mesytec_hv_monitor.md](75_mesytec_hv_monitor.md)(MHV-4、N_Wall では N1408 に交代) /
[docs/hv_control_best_practices.md](../docs/hv_control_best_practices.md)(2026-11-02 の検討メモ) /
`tpcdaq-rs` SPEC v1.42(同じ規則の先例) / マニュアル UM7352 rev.2 = `legacy/WEB_UMN1408_rev2.pdf`(ローカルのみ)

## 1. 決まったこと(ユーザー、2026-11-05)
- Si の電源は **CAEN N1408**(SN 1592、FW 1.00、4 ch、±800 V、20 µA、USB は FTDI FT232B で固有の番号なし)。
- ON/OFF の状態が読めるので、リモートコントロールをつける。
- 誰でも ON/OFF できるのは怖いので、**tpcdaq-rs と同じパスフレーズ。セキュアである必要はない**(事故防止であって認証ではない)。
- **同日の仕様変更: 電圧や電流のトリップ設定も Web からできるようにする。** → `VSET`、`ISET`、`TRIP` を追加。tpcdaq-rs v1.42 の「ON/OFF だけ」より広い。
- **同日、ランプも追加: `RUP`、`RDW` を 10 V/s まで。** 危険度は `MAXV` より低い(到達する電圧は変わらない)が、打ち間違いを止めるため上限をつける。`MAXV` と `PDWN` はパネルのまま。
- **ラン中は混乱を避けるため、できないようにする。** → ラン中にできるのは OFF だけ(ON と設定変更は拒否)。Tune Up はランに含めない。
- ch の名前は今は `ch0`〜`ch3`。どの検出器かは未定なので 4 ch とも操作可能。使わない ch はスイッチを OFF にしておく。
- ch0 の設定(+、120 V、MAXV 200 V、ISET 2.1 µA、3 V/s)は正しい。ほかはおそらく変更する。
- `PDWN` は全 ch `KILL` のまま(トリップしたら最速で落とす)。

## 2. 実機で測った挙動(2026-11-05、何もつないでいない状態、ユーザーがパネルを操作、ch1 = 20 V)
生ログは pf-qaq の `~/n1408_bench_20261105.log`。

**プロトコル:** マニュアルどおり(FW 1.00 でも。マニュアルは 1.0.1 以上と書いている)。9600 8N1、1 往復約 62 ms。
エラーは `#BD:00,LOC:ERR` / `CMD:ERR` / `PAR:ERR` / `CH:ERR`。形式が違う文字列と、いない番地には返事なし(1 s のタイムアウト)。

| 確かめたこと | 結果 |
|---|---|
| LOCAL で `SET ON` / `OFF` | `LOC:ERR`、何も起きない。**LOCAL はパネルの物理的な鍵になる** |
| REMOTE、スイッチ OFF で `SET ON` | **`CMD:OK` が返るのに何も起きない**(`STAT` = DIS、0 V)。**`CMD:OK` は成功の証拠にならない** |
| そのあとスイッチを HV_EN に | 上がらない(さっきの ON は記憶されない) |
| REMOTE、HV_EN で `SET ON` | `ON` → `ON\|RUP` → `ON`。5 V/s で約 6 秒、19.98 V。`RUP` は 17 V あたりで先に消える |
| `SET OFF` | `ON\|RDW` → **`ON`(`RDW` なし、3〜4 秒)** → 0。`ON` の bit は VMON が約 0.15 V になるまで消えない。`PDWN=KILL` でもランプダウンする(`PDWN` はトリップのときだけ) |
| ON 中の ON、OFF 中の OFF | `CMD:OK`、何も起きない |
| ランプの途中で反転 | 素直に向きが変わる |
| ON 中にスイッチ OFF | `ON\|RDW\|DIS` で**ランプダウン**。HV_EN に戻しても**上がらない** |
| ON 中にスイッチ KILL | `KILL`(bit 11)、ON の bit はすぐ消える。約 4 秒で 0.3 V。HV_EN に戻しても**上がらない** |
| ON 中に REMOTE → LOCAL → REMOTE | 落ちない、VMON も動かない。LOCAL の間は `DIS` が立たない |
| LOCAL でスイッチ OFF → HV_EN | **そのまま上がる**(LOCAL ではスイッチが ON/OFF)。切り替えただけでは上がらない |
| ON 中に USB を抜き差し | **HV は落ちない**。再接続で `REMOTE`、`ON`、19.98 V |
| ON 中にクレートの電源を入れ直し(2 回、切っていたのは 6 s と 12 s) | **OFF のまま上がらない**。REMOTE と設定値は残る。USB も一度消える(FTDI はモジュール側の電源) |

**未確認:** トリップ(負荷なしでは起こせない)、インターロックと REMOTE KILL の入力、負荷をつけたときの IMON とランプ。

### 2.1 設定の書き込み(同日、OFF でスイッチも OFF の ch3 と、30 V 以下の ch1。すべて元に戻した)
| 送った値 | 返事 | 読み返し |
|---|---|---|
| **VSET 81(ch3 の MAXV は 80)** | **`CMD:OK`** | **081.00。MAXV を超える VSET を機器は拒否しない** |
| VSET 801 / −5 / `abc` / 空 | `VAL:ERR` | 変わらない |
| VSET 61.234 / 61.235 / 80.01 | `CMD:OK` | 061.24 / 061.24 / 080.02(0.02 V 刻み) |
| ISET 21 / 21.001 / −1 | `CMD:OK` / `VAL:ERR` / `VAL:ERR` | 上限は 21.000 µA(マニュアルの仕様は 20 µA) |
| ISET 9.1234 | `CMD:OK` | 09.123 |
| **TRIP 1000** | **`CMD:OK`** | **1000.0 = 過電流を無期限に許す(p.14)** |
| TRIP 1000.1 / −1 | `VAL:ERR` | 変わらない |
| TRIP 0.55 / 0 / 999.9 | `CMD:OK` | 0000.6 / 0000.0 / 0999.9 |
| ON 中に VSET 25 → 30 → 22 | `CMD:OK` | `ON\|RUP` / `ON\|RDW` で動く。`RUP` と `RDW` は目標の少し手前で消える |
| ON 中に ISET 5、TRIP 5 | `CMD:OK` | 状態は変わらない |
| ON 中に ISET 0(負荷なし、IMON 0) | `CMD:OK` | トリップしない(負荷があれば別) |

| RUP / RDW 1、3、5、100 | `CMD:OK` | 001、003、005、100(`RUPDEC` = 0、整数) |
| RUP / RDW 0、101、−1、`abc` | `VAL:ERR` | 変わらない |
| **RUP / RDW 5.5** | **`CMD:OK`** | **005(黙って切り捨て)** |
| ON 中に RUP 10、VSET 0 → 25 V | `CMD:OK` | ランプの bit 約 2.3 秒(10 V/s に合う) |
| **ランプの途中で RUP 1 → 10、RDW 10 → 1** | `CMD:OK` | **その場で効く。電圧は飛ばない** |

VMON の読みには 1 秒弱の遅れがあるので、ランプの速さは bit の時間で見た。

**MAXV を超える VSET で ON にしたとき、出力が本当に MAXV で止まるかは未確認**(`STAT` の bit 6 `MAXV` はあるので、たぶん止まる)。

## 3. 作ったもの(`tools/hv_monitor/`)
`n1408_control.py`(1 プロセス、DAQ とは独立)、`n1408_control.html`、`n1408_control_config.yaml`、`n1408-control.service`、`test_n1408_control.py`(79 件)。
旧 `hv_monitor.py`(Fission2026 の読み出しだけ)は置き換え。シリアルポートは 1 プロセスしか開けないので、モニタと制御は同じプロセス。

- **Web からできるのは ch ごとの ON、OFF と `VSET`、`ISET`、`TRIP`、`RUP`、`RDW` の変更だけ。** `MAXV`、`PDWN`、全 ch の `CH:4` は**送れない**: ポートへの書き込みは `send_command` 1 か所で、許可リストと決まった形の文字列だけ(テストで固定)。`MAXV` を残したのは、電圧を変えるのと同じ場所で上限を外せないようにするため。パネルなら LOCAL に切り替えて変更でき、HV は落ちない。
- **設定の拒否(クランプしない):** `VSET` > `MAXV`(読み値)、`ISET` > 機器の上限、`TRIP` > 999.9 s、`RUP` / `RDW` が 1〜`ramp_max_v_per_s`(10)の外、負の値、機器が保持しない桁、ON 中に今の電流以下の `ISET`、**操作者が見た値と今の値が違うとき(誰かが変えた)**。同じ値なら何も送らない。送ったあとは読み返して比べる。
- **ラン中は OFF だけ:** Operator の `/api/status` に毎回聞く(`status: running` か `system_state: Running`)。Tune Up はランではない。Operator が答えないときは止めずに通し、結果とログに「DAQ の状態が不明」と残す(DAQ を立てていないときも HV は触れるように)。
- **最後に決めるのは機器:** REMOTE かつスイッチ HV_EN。LOCAL ならソフトは何もできない。
- **`STAT` の読み返しで判定:** ON = `ON` の bit、OFF = `RDW` か `ON` の bit なし。5 秒以内に確認できなければ「failed」。
- **先に拒否(何も送らない):** LOCAL、スイッチ OFF(`CMD:OK` が返ってしまうので)、KILL、INTERLOCK、キャリブレーションエラー、設定で許可していない ch。
- **状態の語:** OFF / DISABLED / RAMP_UP / ON / RAMP_DOWN / SETTLING / TRIP / KILL / INTERLOCK / UNKNOWN。`ON` の bit があっても VMON が VSET より 2.5 V 以上低ければ SETTLING(OFF の終わりを ON と見せない)。読めないものは UNKNOWN で、0 にしない。
- **ソフトは状態を覚えない:** 表示は毎周期の読み値だけ。接続時に BDNAME と BDSNUM を確かめ、違う機器なら読みも制御もしない(by-id のパスは汎用名なので)。
- **操作権:** 名前 + パスフレーズで取得(tpcdaq-rs と同じ。常に横取りでき、前の持ち主の名前を出す)。10 分操作がなければ失効。パスフレーズはホストの平文ファイルで、リポジトリには入れない。ファイルがなければ読み出しのみ。ログインのたびに読むので再起動は不要。
- **記録:** すべてのコマンドと状態の変化を `~/hv_audit/n1408_audit.jsonl`(append-only)と InfluxDB `n1408_event` に。読み値は `n1408`。パスフレーズは記録しない。
- **応答:** ボタンから結果まで 2〜3 秒(周期の読み出しの切れ目を待つ)。**緊急は前面の KILL。**
- **同時に変えるときの順番:** ページは `RUP`、`RDW`、`ISET`、`TRIP`、最後に `VSET` の順で送る(電圧が新しい速度と制限の下で動くように)。途中で失敗したらそこで止める。パネルで 10 V/s を超えるランプにした ch は表で赤く出す(下げるのは Web からできる)。
- **`VSET` の 1 回の変化量に上限はない。** 確認画面に今の値と新しい値を出し、上限は `MAXV`。**`MAXV` を各 ch の運用値の少し上に絞っておくのが一番効く。**

## 4. 実機での確認(2026-11-05 15:53、pf-qaq、REST 経由、何もつないでいない)
| 操作 | 結果 |
|---|---|
| 間違ったパスフレーズ / トークンなし | 403 / 401、何も送らない。拒否はログに残る |
| ch0 ON(設定で不許可)/ ch2 ON(スイッチ OFF) | 409、`SET` は送らない |
| ch1 OFF(すでに OFF)/ ON 中の ON | 200 `already`、`SET` は送らない |
| ch1 ON | 200、OFF → RAMP_UP → ON(19.98 V) |
| ch1 OFF | 200、ON → RAMP_DOWN → SETTLING → OFF |
| ランプアップ中に ALL OFF | 200、許可した ch を順に OFF、ch ごとの結果 |
| 操作権を手放したあと | 401 |
| ログ / InfluxDB | 13 行 / `n1408`、`n1408_event` に入っている |

画面は headless Chrome で確認(操作権ありの状態は記録した応答の再生で)。

**設定変更の確認(同日 16:30、REST 経由):**
| 操作 | 結果 |
|---|---|
| ch1 VSET 151(MAXV 150)、ch3 VSET 81(MAXV 80) | 409、送らない |
| TRIP 1000、ISET 21.5、桁が多い値、負の値 | 409、送らない |
| 見た値が違う(19 と言って実際は 20) | 409「someone changed it」 |
| MAXV、RUP | 409「cannot be changed from here」 |
| 同じ値 | 200 `already`、送らない |
| ISET 9.1 → 5 → 9.1、TRIP 10 → 5 → 10、ch3 VSET 60 → 61 → 60 | 200、読み返し一致 |
| ch1 ON のまま VSET 20 → 25 → 22 → 20 | 200、RAMP_UP など、VMON が追う |
| ON 中に ISET 0 | 409「the channel would trip」 |
| 終了後 | **全 ch の VSET、ISET、TRIP が開始前と一致** |

**ランプの確認(同日 17 時ごろ、REST 経由):** RUP 11、RDW 100、RUP 0、RUP 5.5、見た値の違いは 409 で送らない。RUP 5 → 10 → 1 → 5、RDW 5 → 10 → 5 は読み返し一致。終了後、全 ch の設定が開始前と一致。

**ユーザーの操作(同日 16:48〜16:51、ログに記録):** ブラウザで操作権を取得、ch0 ON、ch1 VSET 20 → 40、ch1 ON、両方 OFF。全部成功。ch1 の VSET は 40 V のまま残してある。

単体テスト 136 件(記録した応答の再生)。pf-qaq で 6 並列 × 2 回すべて通過。**本物のランでの拒否はまだ試していない**(Operator の応答の形はコードで確認、ラン番号を消費するのでユーザーの最初のランで見る)。

## 5. 運用の注意
- **LOCAL でスイッチを OFF → HV_EN と動かすと HV が上がる。** REMOTE なら上がらない。
- **クレートの停電で HV は OFF になり、戻っても上がらない。** ログに `-ON` が残るが、Si を見ている人が気づく必要がある。
- ON で上がる先はパネルの VSET。ボタンの確認画面に VSET、極性、MAXV を出す。
- 全 ch `PDWN = KILL`: トリップしたら最速で落ちる。変えるならパネル(LOCAL)で。

## 6. 配備(pf-qaq)
```sh
# 常駐(sudo、ユーザー)。先に手で動かしている分を止める
pkill -f n1408_control.py
sudo cp ~/delila-rs/tools/hv_monitor/n1408-control.service /etc/systemd/system/
sudo systemctl daemon-reload && sudo systemctl enable --now n1408-control
# MHV-4 を使わないなら(5 分おきにポートのエラーが出るだけで害はない)
sudo systemctl disable --now mhv4-monitor
# パスフレーズ(再起動不要)
printf '%s\n' '<パスフレーズ>' > ~/.config/delila/n1408_passphrase
```
ページは http://192.168.144.142:8095/ 。今は手で起動(nohup)、試験用のパスフレーズ。

## 7. 残り
1. どの ch がどの検出器かが決まったら `n1408_control_config.yaml` の名前を変える(ユーザーの判断待ち)。
2. 本番のパスフレーズ、systemd(ユーザー、§6)。
3. Si をつないでの確認: 極性、VSET、MAXV、ISET、IMON、トリップ。
4. Grafana の N_Wall ダッシュボードに `n1408` のパネル(TODO 74 P4)。
5. 最初の本物のランで、ON と設定変更が拒否され OFF が通ることを確かめる。
6. MAXV を超える VSET での出力(§2.1 の未確認)。今は超える値を Web から送れないので急がない。
