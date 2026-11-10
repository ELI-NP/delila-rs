# TODO 84 — pf-qaq の高レート調査(N_Wall とは別の実験名で)

**Status: ✅ 測定完了 (2026-11-07 11:02〜11:46)、N_Wall に復帰済み(Configured、次のラン 54、本番ファイルはハッシュ一致)、試験のファイルは削除済み。結果は §8。** 残 = §8.3 のユーザー判断と、実験後の修正。

発端: [TODO 73 §5.8](73_campaign_2026_11_pfqaq.md)(ラン 45、46 = Ring のノイズ 6.5 MHz、キュー 104 GB、`nwall_sink` が OOM、再起動で 85 GB 消失)。ユーザーの要望: 高レートで何が起きているか確認、別の実験名、ファイルは作業後に消す、デジタイザは自由に使ってよい、HV は不要(ノイズは HV なしの Si から)。本番の想定は **HV ありで合計せいぜい 1 MHz**(ユーザーの見積もり)。プランは Fable と 2 往復で協議(Fable はコードだけを読み、実機には触っていない)。

## 1. 既に分かったこと(ログとコードから、DAQ には触っていない)

**実測:**
- **メモリ(OOM の表、11-06 19:44:35):** `nwall_sink` 105.8 GB、`recorder` 101.9 GB、`monitor` 34.8 GB、ほかは 5 GB 未満。
- **ボードのレート(ラン 45、3 秒ごと 200 点):** M3 0.48 ± 0.03、M4 3.22 ± 0.27、M5 2.84 ± 0.23、合計 6.54 ± 0.38 MHz。相関は M4-M5 +0.16、M3-M4 −0.04、M3-M5 −0.09。逆相関がなく、合計の揺れは独立な和と同じ → **光リンクの上限で頭打ちという説は支持されない**。
- **ch ごと:** M4 の ch0〜13 は 0.203〜0.242 MHz(平均 0.222)、ch14 は 0.111。M4 の台形 rise 3296 + flat 1200 = 4496 ns の逆数が 222 kHz。M5(V1725S、FW 139.137、同じ台形)は 0.002〜0.498 MHz でばらばら。M3 は 0.015〜0.056。M4 の揃い方は FW の ch ごとの上限に見える(未検証、§3 Step 4)。
- **Recorder の速度:** 開始直後から 180 万 ev/s で一定 → メモリが詰まるにつれ遅くなる説(direct reclaim)は形が合わない。入力なしで約 320 万 ev/s。
- **Monitor の開始の遅れ:** ラン 45 は Start から 13 秒、ラン 46 は 18 秒(179,975 のまま 15 秒止まる)。その後は約 370 万 ev/s で、入力より遅い。34 kHz のラン 48 では遅れなし。原因は未確定。Influx の Monitor `queue_items` は常に 0 で使えない。ラン 45 では 17:27:01 の Monitor のサンプルが欠けている。
- **rollover 警告:** M3 22 万、M4 79 万、M5 89 万行。Start + 34.3 秒に始まりラン中ずっと。btt は `0x8000xxxx`〜`0xFFFFFFFF` を **17.18 秒周期**(2^31 × 8 ns)で繰り返す。
- **タイムスタンプは壊れていない(PHA1):** `recover info` のフッタで、ラン 45 のファイル 0000 が 0〜7.38 s、0030 が 213.8〜221.2 s、0060 が 425.4〜432.7 s。折り返さずに進む。

**コードで確認:**
- `src/recorder/mod.rs:609`: 書き込みスレッドが、フッタの時刻範囲のためだけに全バッチを `EventDataBatch::from_msgpack` でデコード。コメントの "cheap vs I/O" は NVMe では成り立たない可能性が高い。これが律速かは未測定。
- `tools/root_sink/common/eb_core.hpp:229`: Sorter が 100 ms のチャンクごとにバッファ全体を `std::sort`。バッファはレート × (`safe_horizon_ms` + span)。Fable の見積もりで天井は約 110 万 hits/s(未測定)。
- `src/reader/decoder/rollover.rs`: 後ろ向きの `Underflow` で `prev_raw` を更新しない。`RolloverTracker::new(32)` に対し、btt の bit 31 が立ちっぱなしの 31 bit 相当なので、34.36 秒以降は全アグリゲートで警告になる。時刻は EXTRAS + TTT から作るので PHA1 では影響しない。`SwFineTs`(extras option 5)の経路は影響を受ける可能性がある(未確認)。**修正は decoder の規則どおり、仕様書のページと実機確認が要るので今回はしない。**

**訂正:** 「`nwall_sink` は以前の 248 万/s より遅い」は比較が誤り。248 万は TODO 80 の 2 ファイル案(H3)で、`nwall_sink` は 1 ファイル(H1 = 142 万、events なしなら A1 = 321 万)。

## 2. 分離(別の実験名)

実験名 `HiRate_Test`、出力 `/data/HiRate_Test`。

| 項目 | やり方 | 残るもの |
|---|---|---|
| TOML | `config/config_hirate_pfqaq.toml`。`config.toml` のシンボリックリンクは触らず、`./scripts/start_daq.sh config/config_hirate_pfqaq.toml --no-mongo` で引数に渡す | ファイルのみ、作業後に削除 |
| MongoDB | `--no-mongo`。ラン番号は REST で明示 | なし |
| eLog | `[operator.elog]` を書かない | なし |
| InfluxDB | `[operator.influxdb]` を書かない(`channel_rate` は実験名を持たず N_Wall のダッシュボードに混ざる上、消せない) | なし。測定は `/api/status` のポーリング |
| JSON | 現在の pf-qaq のファイルから `config/digitizers/hirate_*.json` にコピー。本番の 6 個は `sha256sum` で前後比較 | 作業後に削除 |
| event def | `config/event_def_hirate.json` = N_Wall のコピーで trigger を `dE_ring` に(E が鳴らないので、events を最悪側で書かせる) | 作業後に削除 |
| channel map、keV の表 | N_Wall のものを読むだけ | なし |
| Monitor のレイアウト | UI で Save しない。前後で `cmp` | なし |
| archive_sync | `/data/N_Wall` と `/data/DELILA_data` しか見ない。ただし `/data` が 80 % を超えると古いファイルを消すので、試験中は 70 % 未満を守る(今 57 %、余裕約 450 GB) | なし |

## 3. 手順(大事な順。各 Step は単独で意味がある)

**Step 0 準備(約 20 分、DAQ に触らない):** TOML、JSON コピー、event def、計測スクリプト、`memguard`、`abort_restore`、ハッシュの基準値。ここまで実機は不変。

**Step 1 ノイズの確認 + 開始の遅れ(約 5 分):** Ring を全部有効にして 60 秒。今のノイズが約 6.5 MHz あるか(ポンプやケーブルで変わるかもしれない)。同時に 200 ms ごとの `/api/status`、`/proc/<pid>/task/*/stat`、`pidstat -t`、`iostat -x`。Monitor の遅れと Recorder のスレッドごとの CPU を初めて見る。

**Step 2 1 MHz の合否(約 45 分、最重要):** M4 だけを N ch 有効にする。各 ch が約 222 kHz で飽和しているなら、N でレートを決められる(ノイズの揺れに強い): 5 ch ≈ 1.1 MHz、7 ch ≈ 1.55 MHz、9 ch ≈ 2.0 MHz。各 10 分。合格 = Recorder / Monitor の `queue_bytes`、`nwall_sink` の `q[sort= work= write= disp=]` が増え続けない。最初に増え出すレートが余裕の限界。増えたら同じ点を `safe_horizon_ms` 200 で再測定(コードを変えずに余裕を増やせる唯一の設定。ボードの順序の乱れは 100 ms 以内と実測済み)。

**Step 3 開始の遅れの繰り返し(約 15 分):** Ring 全部で 60 秒 × 3、Step 1 と同じ計測。残る候補: 受信側の停止、ヒストグラム側の停止、プロセス全体の停止。Recorder の半減は H1(CPU のコアあたり速度)か H3(実行待ち)を `pidstat -t` と `/proc/<tid>/schedstat` で分ける。`perf stat` はユーザーの sudo が 1 回要る(任意)。

**Step 4 自動停止の試験(約 15 分):** `backlog_hard_limit_mb = 20000`、`[operator.backlog_autostop]`、`drain_stop_timeout_secs = 1800` で Ring 全部。約 4 分で止まり、書き切ってフッタが付くか。N_Wall で有効にするかの判断材料。

**Step 5 M4 の上限の確認(約 5 分、任意):** JSON コピーで `trap_rise_time_ns` 3296 → 1648 だけ変えて 60 秒。予想 ch0〜13 → 0.351 MHz、M4 → 5.1 MHz、リンク 2 → 8.4 MHz(約 105 MB/s)。ここで M4 が伸びず M5 が落ちて逆相関が出れば、それがリンクの上限。

**Step 6 リプレイ(任意、ビルドが要る):** デジタイザの上限を超えた速度を試すとき。`cargo build --release --features x743,dev-tools --bin delila_merger_replay --target-dir ~/hirate_target`(本番のバイナリに触らない)。`start_daq.sh` は Merger を必ず起動するので使えない: `recorder` → `monitor` → `nwall_sink` → `delila_merger_replay --source-id 0 --input /data/N_Wall/run0045_*.delila --delay-ms N --loop-replay` → `operator`(Mongo なし)。source は `type = "emulator"`、`bind = tcp://*:5557`、`command = tcp://*:7100`、`[network.merger]` なし。`--delay-ms` は 33 ≈ 1 M/s、5 ≈ 6.5 M/s。未確認: Operator が Merger なしで動くか(Fable: 動かなければ `publish` だけ書いて Merger は起動しない)。

ディスクは全部で 200 GB 未満の見込み。

## 4. 安全

- **メモリ:** 試験一式を `systemd-run --user --scope -p MemoryMax=170G` の中で動かす(`memory.max` が効くことを pf-qaq で確認済み、`XDG_RUNTIME_DIR` が要る)。OOM はこのスコープの中だけで起きる → InfluxDB、Grafana、`n1408-control`、`sy5527_monitor` を守る。
- **`memguard`:** 2 秒ごと。`MemAvailable` < 60 GB で `POST /api/stop_drain`、`/data` > 70 % で同じ。PID ファイルで管理し、`pkill -f` は使わない(コマンド行が一致して自分を殺す)。
- **全 Ring のランは 60 秒まで**(Step 4 を除く)。次の Start の前にキューが 0 になるまで待つか、試験データなので sink を kill する。
- **N_Wall のスタックでは絶対にキューが残ったまま再起動しない。** 試験のスタックでは構わない(捨てるデータ)。
- **実機:** HV は触らない。CAEN ツールは Reader が掴んでいるボードに使わない。Reader のログに `Reconnected`、`transient`、Running で 0 イベントのボードが出たら止めて報告。ボードが固まればクレートの電源再投入(ユーザー)。

## 5. 取りやめと復帰(`abort_restore.sh`、何度実行しても同じ)

1. PID ファイルで計測と `memguard` を止める。`pkill -f delila_merger_replay`。
2. `./scripts/stop_daq.sh`(試験のキューは捨てる)。
3. `readlink config.toml` が `config/config_nwall_pfqaq.toml`。本番 JSON 6 個、`monitor_layout.json`、channel map、event def の `sha256sum -c`。
4. `./scripts/start_daq.sh`(N_Wall)。`experiment_name` = N_Wall、9 コンポーネント + `nwall_sink`、`/api/runs/next` = 54。
5. `POST /api/configure`(ハードに本番の設定を入れ直す。始める前の状態 = Configured に戻る)。
6. `/data/N_Wall` のファイル数と最新のファイルが前と同じ。

## 6. 後片付け

`rm -rf /data/HiRate_Test`、`hirate_*` の JSON と TOML、計測スクリプト、`~/hirate_target`。結果をこの TODO に書いてから、試験のログの `logs/<dir>` も消す。ラン 45 の 62 GB と `run_inprogress_1793986014.root` はユーザーの判断(Step 6 で使うので、それまでは残す)。

## 8. 結果(2026-11-07、実機、HV なし、Ring のノイズ)

試験一式は `systemd-run --user --scope -p MemoryMax=170G` の中で動かした(全 11 プロセスがスコープ内、`memguard` は一度も発動せず)。Mongo / Influx / eLog には何も書いていない。レートは M4 の有効 ch 数で決めた(今日のノイズは 1 ch あたり 150〜450 kHz で、11-06 の 222 kHz にはそろっていなかった)。events は trigger = `dE_ring` の最悪側。

| Step | ラン | 入力 | Recorder(`.delila`) | ROOT(`nwall_sink`) | Monitor |
|---|---|---|---|---|---|
| 1 | 1 | Ring 全部 6.6 MHz(M3 0.6、M4 2.8、M5 3.3)、ZSTD-5 | 1.9 M ev/s で書き込みスレッド 98 %、キューは増加 | 約 0.5 M hits/s(events あり、ZSTD)、`q[sort]` が増加 | 約 26 秒周期で 8〜16 秒停止 |
| 2a | 2 | M4 5 ch、0.80 MHz、4.5 分 | キュー 0 | 0.83 M/s、キュー 0 | 停止なし |
| **2b** | **3** | **M4 7 ch、1.2〜1.6 MHz、10 分** | **キュー 0、書き込みスレッド 39 %** | **キュー ≈ 0、215 %** | **停止なし** |
| 2d | 6 | M4 10 ch、2.7〜3.1 MHz、3 分 | 97 %、キュー +0.3 GB/分 | 書き込み 1.86 M/s で頭打ち、`q[sort]` 増加、late 0 | 周期的に停止 |
| 2c | 4 | M4 15 ch、5.0〜5.4 MHz、4 分 | キュー +4 GB/分 | **late 11.7 億**(下記) | 約 22 秒周期で 15 秒停止 |
| 4 | 5 | 2c と同じ + 自動停止 20 GB | 21.0 GB で drain-first stop、4.3 分で書き切り、全ソースの EOS で閉じた | | |

### 8.1 分かったこと
- **本番の 1 MHz は合格(圧縮なし)。** 上限は ROOT 約 1.9 MHz(× 1.9)、Recorder 約 2.7 MHz(× 2.7)。M4 だけの、events を最悪側にした条件での値。
- **Reader はボード 1 枚あたり約 5 MHz で追いつかなくなる**(デコード 8 スレッドが 99 %)。そのボードのデータだけが遅れ、`.delila` のファイルの時刻の重なりが 1.1 → 5.3 秒と伸びた。Stop 後の片付けに 12.5 秒。**遅れが `safe_horizon_ms`(1 秒)を超えると `nwall_sink` はヒットを late として捨てる(ROOT のみ、`.delila` は全部ある)。** late はログの数字だけで、Operator からは見えない。
- **Monitor の「遅れて始まる」の正体は周期的な停止**(高レートのときだけ、開始時に限らない)。止まっている間は Monitor のスレッドで動いているのが 1 本だけ(59/59 サンプル)、動いている間は 2 本(31/37)。受信ループ(全バッチのデコード)が CPU を使い続ける間、ヒストグラムのタスクが走らない。tokio のタスク飢餓が最有力(コードでは未確認)。Monitor の RSS も 4 → 17 GB に増える(SUB パイプ)。1.6 MHz では起きない。
- **Recorder は書き込みスレッドの CPU で決まる**(97〜98 %、実行待ち 0、direct reclaim 0)。全バッチのデコード(`recorder/mod.rs:609`)が原因の候補(未証明)。
- **自動停止(TODO 68)は動く。** ただし書き切りに 258 秒かかったので、`drain_stop_timeout_secs` は既定の 60 秒では短い。`nwall_sink` は対象外。
- Operator の `/api/status` はキャッシュしない(毎回各コンポーネントに直接問い合わせる)。Recorder の `events_processed` は開始直後に 5〜15 秒止まって見えるが、ログでは書いている(計測値の問題)。

### 8.2 実施した変更(ユーザーの依頼、未コミット)
- **`nwall_sink --compression N`**(既定 505 = ZSTD-5、0 = 圧縮なし)、TOML の `compression` → `start_daq.sh` が渡す。**N_Wall の TOML に `compression = 0`**(最終保存先が ZFS で圧縮されるため)。pf-qaq に配備・N_Wall で有効(起動ログ "ROOT compression 0 (uncompressed)")。旧版は `~/.local/bin/nwall_sink.before_compression`。単体テスト 67 本通過。

### 8.3 ユーザーの判断(2026-11-07)と対応
- **自動停止を N_Wall で有効化**(「そのレベルならどうせ手で止めていた」): `[operator.backlog_autostop]`、Recorder の `backlog_hard_limit_mb = 40000`、`drain_stop_timeout_secs = 1800`。**UI にはっきり出すこと**(ユーザー)→ Operator が自動停止の通知(原因 = レート過多 / ディスク不足 / 書き込み失敗、段階 = 書き込み中 / 完了 / 不完全、ラン番号、理由)を持ち `/api/status` の `auto_stop` で返す。次のランの Start で消える。UI は制御パネルに赤い枠で「Run N stopped automatically: data rate too high」、書き込み中は「do not stop or restart the DAQ」、完了後は「the files are complete」と次にすることを出す(`src/operator/auto_stop.rs`、`control-panel.component.ts`)。run guard(ディスク不足、書き込み失敗)も同じ枠に出る。テスト: Rust 3 + 状態取得 1、UI 5(サービス 1、制御パネル 4、制御パネルのテストはこれが最初)。
- **コミットと push**: 22c3052〜9315924。pf-qaq のローカル変更(ThGEM 用の設定など)は `~/delila-rs-local/pfqaq_local_changes_20261107.patch` に保存してから作業ツリーを origin に合わせた。ThGEM の Monitor レイアウトは `config/monitor_layouts/monitor_layout_thgem.json`(戻すときは `config/digitizers/monitor_layout.json` へコピー)。
- **ラン 45 の 62 GB と `run_inprogress_1793986014.root` は残す**(「ゴミも立派なデータ」、ディスクは余っている)。
- **Monitor の停止は Fable に修正を依頼**(実機は触らせない、配備と実機確認はメインのセッション)。

### 8.4 Monitor の修正(ac86dcc、2026-11-07 14:09 EET pf-qaq 配備)
- **原因**(Fable、再現と tokio 1.49 のソースで確認): tmq の `next()` はデータが溜まっている間 ZMQ_EVENTS から即 Ready を返し、受信タスクが一度も譲らない。ヒストグラムタスクの起床はそのワーカーの LIFO スロット(他のワーカーが盗めない)に入る。そのワーカーが I/O とタイマーのドライバを持っていたので、HTTP とコマンドも止まった。
- **修正**: 受信は専用 OS スレッド(ブロッキングの zmq::Socket、RCVHWM=0、RCVTIMEO 100 ms、ヘッダだけ見る)。デコードはヒストグラムタスク側(1 バッチごとに yield)。追いつかないときは生フレームをデコード前に捨て、`dropped_batches` に数えて 5 秒ごとに warn。`channel_capacity` の既定は 1000 → 64(表示の遅れを 1 秒未満に)。CLAUDE.md の例外の記述も bounded(64) に直した(ユーザー「おまかせ」)。
- **Mac での数字**(Fable、入力 1320 万イベント/秒): 修正前は最初の 4 秒処理 0、HTTP 最大 4.2 秒、メモリ 8 秒で 15 GB。修正後は毎秒 560 万で一定、HTTP 12 ms、1.3 GB のまま。master で fmt・clippy・815 テスト通過。
- **配備**: ラン 58 停止後、Recorder のキュー 0・nwall_sink のラン 58 クローズを確認してから再起動。起動ログ "Monitor receive thread started"。この再起動で TOML のセクター名の訂正(mod1 = E、mod2 = dE)も Operator に入った。
- **戻し方**: `~/delila-rs-local/switch_daq_bins.sh old`(404cc57 の 13 本、`bin_404cc57_before_monitor_fix/`)→ ランの合間に再起動。`new` で修正版(`bin_ac86dcc_monitor_fix/`)に戻る。リビルドすると修正版に戻るので、恒久的に戻すなら `git revert --no-commit ac86dcc`。
- **pf-qaq の高レートでの確認はまだ**(2.8 / 5 MHz で processed が進む、捨てた数が数えられる、RSS が平ら、HTTP が応答する)。
- **ラン 58(14:07、Ring 各 220 万/秒、合計 650 万/秒、約 1 分でユーザーが手で停止)**: Recorder は 140 万/秒、停止後に書き切った(dropped なし、.delila 4 ファイル)。nwall_sink の `late` はラン 57 終了時 751 → 2,091,045(ほぼ全部ラン 58。1 秒より遅れたヒットを ROOT から落とす、§7 の可視化の件。うち約 8.4 万はラン 58 の finalize の後に来た)。ROOT は 1.84 億ヒットで約 1 % 欠け、.delila は完全。旧 Monitor はこのランで止まっていた(event_rate 0)。

## 7. 測定の後で決めること(今はコードを変えない)

- ~~Recorder の全バッチのデコードをやめる~~ → **2026-11-10 実施(ユーザー「やめちゃいましょう。データがあればいい」)、下の §7.1**。Sorter を差分マージにする、`safe_horizon_ms` を短くする、Monitor の受信で SUB パイプが増える件(HWM = 0 で、表示専用なのにメモリを 35 GB 使った)。
- TODO 73 §5.8 の未決定(自動停止、再起動でキューを消さない、キュー中の Start とイベント数の記録)。
- rollover トラッカーのビット幅(仕様書のページ、`caen_simple_test`、`SwFineTs` の経路の確認が先)。

### 7.1 Recorder の全バッチのデコードをやめた (2026-11-10)

- 用途を確かめたら 1 つだけだった: `.delila` のフッタに「最初と最後のイベントの時刻」を書くため(`src/recorder/mod.rs`、書き込みスレッドで毎バッチ全デコード)。ファイルへはデコードせずにそのまま書き、イベント数もデータの先頭の数バイトから取っている。フッタの時刻を使っていたのは `recover info` の表示だけで、`delila2root` はフッタのイベント数を照合に使うだけ。
- 変更: Recorder はデコードしない(フッタの時刻は「記録なし」のまま)。`FileFooter::has_time_range()`、`DataFileReader::scan_time_range()`(データを読んで最小と最大)、`batch_time_range()`(バッチ内は時刻順でないので最初と最後ではなく最小と最大)を足した。`recover info` はフッタに時刻がなければデータから出す(「read from the data」と表示)。`recover recover` もバッチの最初と最後でなく最小と最大で組み立てる。
- テスト +3(format 2、Recorder 1)、全 873 件通過、clippy 警告 0。Mac で新しい Recorder のファイル(エミュレータ 5 秒、8700 イベント)に `recover info` → フッタ完全、時刻はデータから表示。
- 配備: 6a33fb3(pf-qaq は 9d4d1cf)。19:12 EET、Idle(次はラン 83)、書き残し 0、メモリ上の設定とファイルが一致することを確かめてから DAQ を再起動(`logs/20261110_191205`、強制終了なし)。6 台とも 1 回目でつながり、全台 PLL OK。
- **上限がどれだけ上がったかは未測定。** pf-qaq の DAQ_Test(nwall_sink なし)で 3〜5 MHz を流せば、書き込みのスレッドの CPU と待ち行列で確かめられる。
