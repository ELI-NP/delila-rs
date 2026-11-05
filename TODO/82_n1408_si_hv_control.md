# TODO 82 — Si の HV(CAEN N1408): モニタ + ch ごとの ON/OFF(Web、パスフレーズつき)

**Status: 🚧 実装・pf-qaq で実機確認済み (2026-11-05) — 残 = ch 名と使う ch(ユーザー)、本番のパスフレーズ、systemd への登録(sudo、ユーザー)、Si をつないでの確認、Grafana のパネル(TODO 74 P4)**

**関連:** [73_campaign_2026_11_pfqaq.md](73_campaign_2026_11_pfqaq.md)(親) /
[75_mesytec_hv_monitor.md](75_mesytec_hv_monitor.md)(MHV-4、N_Wall では N1408 に交代) /
[docs/hv_control_best_practices.md](../docs/hv_control_best_practices.md)(2026-11-02 の検討メモ) /
`tpcdaq-rs` SPEC v1.42(同じ規則の先例) / マニュアル UM7352 rev.2 = `legacy/WEB_UMN1408_rev2.pdf`(ローカルのみ)

## 1. 決まったこと(ユーザー、2026-11-05)
- Si の電源は **CAEN N1408**(SN 1592、FW 1.00、4 ch、±800 V、20 µA、USB は FTDI FT232B で固有の番号なし)。
- ON/OFF の状態が読めるので、リモートコントロールをつける。
- 誰でも ON/OFF できるのは怖いので、**tpcdaq-rs と同じパスフレーズ。セキュアである必要はない**(事故防止であって認証ではない)。

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

## 3. 作ったもの(`tools/hv_monitor/`)
`n1408_control.py`(1 プロセス、DAQ とは独立)、`n1408_control.html`、`n1408_control_config.yaml`、`n1408-control.service`、`test_n1408_control.py`(79 件)。
旧 `hv_monitor.py`(Fission2026 の読み出しだけ)は置き換え。シリアルポートは 1 プロセスしか開けないので、モニタと制御は同じプロセス。

- **Web からできるのは ch ごとの ON と OFF だけ**(tpcdaq-rs v1.42 と同じ)。VSET、ISET、MAXV、ランプ、トリップ、`PDWN`、全 ch の `CH:4` ON は**送れない**: ポートへの書き込みは `send_command` 1 か所で、許可リストの文字列だけ(テストで固定)。
- **最後に決めるのは機器:** REMOTE かつスイッチ HV_EN。LOCAL ならソフトは何もできない。
- **`STAT` の読み返しで判定:** ON = `ON` の bit、OFF = `RDW` か `ON` の bit なし。5 秒以内に確認できなければ「failed」。
- **先に拒否(何も送らない):** LOCAL、スイッチ OFF(`CMD:OK` が返ってしまうので)、KILL、INTERLOCK、キャリブレーションエラー、設定で許可していない ch。
- **状態の語:** OFF / DISABLED / RAMP_UP / ON / RAMP_DOWN / SETTLING / TRIP / KILL / INTERLOCK / UNKNOWN。`ON` の bit があっても VMON が VSET より 2.5 V 以上低ければ SETTLING(OFF の終わりを ON と見せない)。読めないものは UNKNOWN で、0 にしない。
- **ソフトは状態を覚えない:** 表示は毎周期の読み値だけ。接続時に BDNAME と BDSNUM を確かめ、違う機器なら読みも制御もしない(by-id のパスは汎用名なので)。
- **操作権:** 名前 + パスフレーズで取得(tpcdaq-rs と同じ。常に横取りでき、前の持ち主の名前を出す)。10 分操作がなければ失効。パスフレーズはホストの平文ファイルで、リポジトリには入れない。ファイルがなければ読み出しのみ。ログインのたびに読むので再起動は不要。
- **記録:** すべてのコマンドと状態の変化を `~/hv_audit/n1408_audit.jsonl`(append-only)と InfluxDB `n1408_event` に。読み値は `n1408`。パスフレーズは記録しない。
- **応答:** ボタンから結果まで 2〜3 秒(周期の読み出しの切れ目を待つ)。**緊急は前面の KILL。**

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

画面は headless Chrome で確認(操作権ありの状態は記録した応答の再生で)。**ボタンそのものはまだ人が押していない。**

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
ページは http://192.168.144.142:8095/ 。試験の状態: pf-qaq の設定で ch1、ch2 だけ `control: true`(ローカル修正)、試験用のパスフレーズ。

## 7. 残り
1. ch 名と使う ch(ユーザー)→ `n1408_control_config.yaml` に入れてコミット。
2. 本番のパスフレーズ、systemd(ユーザー、§6)。
3. Si をつないでの確認: 極性、VSET、MAXV、ISET、IMON、トリップ。
4. Grafana の N_Wall ダッシュボードに `n1408` のパネル(TODO 74 P4)。
5. tpcdaq-rs の「HV ON は Idle のときだけ」は入れていない(ラン中の ON も通る)。要るかはユーザー判断。
