# TODO 86 — ランのタイマーを Operator に持たせる

**Status: ✅ 実装・pf-qaq に配備・実機で確認 (2026-11-10、cc5b974)。** ユーザーが `DAQ_Test` のラン 1 で試した(§8)。表示はときどき 1 秒引っかかるが OK(ユーザー)。残 = eLog の `[stop: timer …]`(DAQ_Test は eLog なしで未確認)。

どのページ・どの PC から見ても同じ残り時間が出て、ブラウザがなくても時間が来たら止まるようにする。

## 1. 発端と今の作り

- ユーザー(2026-11-10): 「タブを変えるとタイマーがどっかにいっちゃう」。シフトマニュアル(`docs/shift_manual_nwall.md`)では、シフトの人にタイマーを使わせたい(ランは標準 2 時間、計画で変わる)。
- 今の作り(2026-11-10 にコードで確認):
  - `web/operator-ui/src/app/services/timer.service.ts`: `providedIn: 'root'`。残り秒数を `setInterval` で **1 秒ごとに 1 引く**。
  - 表示は Control のページ(`control-panel.component.ts`)だけ。ヘッダ(`app.ts`)には出ない。
  - 時間切れの処理(アラームのダイアログと `operator.stop()`)は、Control Panel のコンストラクタが `timer.onTimerComplete` に入れる関数。Start が成功した後に、ブラウザが `timer.startTimer()` を呼ぶ。
  - サーバ(Operator)はタイマーのことを何も知らない。
- そのため起きること:
  1. Control 以外のページでは残り時間が見えない(ユーザーが見た症状)。
  2. **ブラウザのタブを隠すと遅れる。** Chrome は、5 分以上隠れたページのタイマーを 1 分に 1 回しか動かさない(intensive throttling)。1 秒に 1 引く作りなので、2 時間のタイマーが何十時間にもなる。macOS と Windows では、ほかのウィンドウに完全に隠れたウィンドウも同じ扱い。
  3. 再読み込み、ブラウザを閉じる、PC のスリープ: タイマーが消え、**ランは止まらない**。
  4. タイマーがあるのは Start を押したブラウザだけ。別の PC やシフト交代後の人には見えない。
- 当面の運用(この TODO が終わるまで、シフトマニュアルに書く):
  - Control のページを専用の、見えているウィンドウにしておく。Monitor と Grafana は別のウィンドウで開く。
  - タイマー中はそのページを再読み込みしない。PC をスリープさせない。
  - 時間が来ても止まっていなければ Stop を押す。
  - 交代のときは終了予定時刻を eLog に書く。

## 2. 方針(案): タイマーは Operator が持ち、画面は見せるだけ

### 2.1 Operator

- **Start で受け取る。** `/api/run/start` の要求に、省略できる `timer` を足す(`minutes` と `auto_stop`)。
  - 省略すれば今と同じ動き。REST でランを回すスクリプト(`scripts/time_jump_scan.sh` など)はそのまま動く。
  - 今の要求の型 `ConfigureRequest` は `/api/configure` と共用なので、`run_start` 用の型を分けて、`ConfigureRequest` を flatten で含める。
- **ランの情報に持たせる。** `CurrentRunInfo` に `timer: Option<RunTimer>`(長さ、auto_stop)を足す。
  - 起点は今の `start_time` と同じく、Start が成功した時点。Configure などにかかった 20 秒ほどはランの時間に入れない。
  - 時間切れの判定は単調時計(`Instant`)で行う。NTP で時刻が飛んでも、ランが伸び縮みしないようにする。
  - `/api/status` の `run_info.timer` で、`duration_secs`、`auto_stop`、`remaining_ms`(サーバで計算、auto_stop なしのときは超過分が負)、`ends_at`(表示用の UNIX ms)を返す。
- **見張りのタスク** `operator::run_timer::run_watcher`(`digitizer_guard::run_watcher` と同じ形)。
  - 1 秒ごとに `current_run` を見る。止めるかどうかは純粋関数 `decide(timer, now) -> 何もしない | 期限切れ(知らせるだけ) | 止める` にして、ここをテストする。
  - 止めるとき: `run_control_lock` を取り、**同じラン番号でまだ Running か確かめてから**止める(TODO 58 H14 の check-then-act)。その間に人が止めて次のランを始めていたら、何もしない。
  - 止め方は Stop ボタンと同じ(`/api/stop` の中身を `plain_stop(state, reason)` に切り出して共用)。理由は `timer (120 min)`。Tune Up 中は何もしない(既存の見張りと同じ)。
- **ラン中に変える**(§4 の 3 で決定): `PUT /api/run/timer` で、長さの変更・取り消し・auto_stop の切り替え。Running のときだけ受け付ける(それ以外は 409)。変更はランのメモ(notes)に自動で残す(例: 「timer 120 → 150 min」)。
- **記録する。** ラン文書(MongoDB)に、予定の長さと auto_stop を開始時に書く。タイマーで止まったときは、既存の `stop_reason` に理由が入り、eLog の Run Summary のコメントにも既存の経路で `[stop: timer (120 min)]` が付く。
- `current_run`(とタイマー)は Operator のメモリにしかなく、Operator を再起動すると消える。**扱わない**: Operator の再起動は DAQ の再起動と同じで、ラン中にはしない(ユーザー 2026-11-10)。

### 2.2 UI

- **`TimerService` は表示だけにする。** 1 引く方式をやめる。
  - 残り時間 = 前回の状態取得で受け取った `remaining_ms` −(今の `performance.now()` − 受け取った時刻)。
  - こうすると PC の時計がずれていても関係ない。隠れたタブで遅れても、次の状態取得(1 秒ごと)で正しい値に戻る。
- **ヘッダ(`app.ts` の toolbar)に出す。** 今の「Run: N」の隣に「1:23:45 left」。auto_stop なしで時間が過ぎたら「+3:12 over」。全ページで見える。
- **Control Panel:**
  - 今の入力(Run with timer、Duration (min)、Auto-stop on expire)は、Start の要求と一緒に送る。
  - ラン中のバー表示はサーバの値から出す。
  - ラン中の変更: 「+30 min」「Set」「Cancel timer」のボタン。
- **知らせ方:** 開いている Operator UI のページは、どれも時間切れを見たら 1 回ダイアログと音を出す(閉じるのはページごと)。
  - 注意: 音はブラウザの自動再生の制限を受ける。読み込み後に一度もクリックしていないページは鳴らないことがある。ダイアログは出る。
- **ブラウザからの `operator.stop()` はやめる。** 止めるのはサーバだけ。二重の Stop を防ぐ。

### 2.3 変わらないこと

- 手動の Stop と、自動停止(backlog、ディスク、書き込み失敗、PLL)が先に来れば、そちらで終わる。ランが終われば、終わり方に関係なくタイマーも消える。
- タイマーで止まるのは普通の終わり方なので、赤い自動停止の枠(`auto_stop`)は出さない。

## 3. 作業の順(TDD)

1. Rust: `decide()` のテスト。期限前、ちょうど、期限後、auto_stop なしは 1 回だけ知らせる、ラン番号が違えば何もしない。
2. Rust: 要求の互換性(`timer` なし、あり、不正な値: 0 分、上限超え)のテスト。`/api/status` の JSON のテスト。止めた理由がラン文書と eLog のコメントに入るテスト。
3. Rust: 見張りがロックを取り、ランが変わっていたら止めないテスト(既存の `run_start_reaches_running_with_healthy_fakes` の偽物を使う)。
4. Rust: `PUT /api/run/timer`。Running 以外は 409、範囲、notes への記録。
5. UI: `TimerService` の残り時間の計算(偽の時計)、ヘッダが全ページに出ること、Start の要求に timer が入ること、時間切れの知らせが 1 回だけのこと。`npm run build` して `dist/` も同じコミットに入れる。
6. Mac で通しの確認(エミュレータ): 2 分のタイマーで始めて、次の 4 つで止まり方と表示を確かめる。
   - タブを隠す
   - 再読み込みする
   - ブラウザを閉じる
   - 2 つ目のブラウザで開く(同じ残り時間か)
7. `cargo fmt && cargo clippy --tests -- -D warnings && cargo test`、UI のテスト。

## 4. 決定(ユーザー 2026-11-10、すべて推奨どおり)

1. **時間切れの止め方: 普通の Stop。** 手で Stop ボタンを押したのと同じ(Stop 直後のテールの扱いも同じ)。
2. **長さの上限: 24 時間まで**(今の UI は 180 分。夜通しのラン、例えばラン 79 の約 13 時間もタイマーにできるように)。
3. **ラン中の延長・短縮・取り消し: 入れる。** 変更はランの notes に自動で残す。
4. **auto_stop なし(知らせるだけ): 残す。** ヘッダに超過時間を出す。
5. **時間切れの知らせ: 開いている全ページで出す。** シフトの人が Monitor を見ていても気づけるように。

## 5. 配備(pf-qaq、ランの合間)

- 変わるのは Operator のバイナリと UI の `dist/` だけ。入れるにはランの合間に DAQ を再起動する(Operator の再起動は DAQ の再起動と同じ扱い)。
- 配備したらシフトマニュアルの注意書き(§1 の当面の運用)を外す。

## 6. 実装 (2026-11-10)

**計画から変えた点:**
- **ランごとに時計を持つ(`RunClock`)。** タイマーの有無にかかわらず、Start が成功した時点の単調時計を、すべてのランで持つ。こうすると、ラン中の後付け(Set timer)、延長、取り消しが同じ扱いで済む。古い経路の Start(`/api/start`)でも時計を始める。
- **予定の長さはラン文書の欄でなく、ランのメモ(notes)に残す。** 「Run timer: 120 min, stops the run」「Run timer changed: … -> …」「Run timer cancelled (was …)」。MongoDB の文書の形は変えない。止めた理由は既存の `stop_reason`(`timer (120 min)`)で、eLog の Run Summary のコメントにも付く。

**Operator(Rust):**
- `src/operator/run_timer.rs`(新規): `RunTimerRequest`(1〜1440 分、`auto_stop` の既定は true)、`RunClock`、`RunTimerStatus`、`TimerExpiredNotice`、純粋関数 `decide()`、`begin_run()`、`add_note()`、`poll_once()`、`run_watcher()`(1 秒ごと、Tune Up 中は何もしない)。
- 時間切れの処理: `run_control_lock` を取り、同じランで同じタイマーのままか確かめてから、`expired` を立て、知らせを置き、auto_stop なら止める。
- `routes/status.rs`:
  - `stop` の中身を `plain_stop(state, reason)` に切り出し、タイマーと共用する。
  - 終わりの後処理で時計も消す。
  - `run_start` は `RunStartRequest`(`ConfigureRequest` を flatten + `timer`)を受け取る。範囲外は何も触る前に 400 で断る。
  - `/api/status` に `run_info.timer` と `timer_expired` を足した。
- `routes/run.rs`: `PUT /api/run/timer`(設定と変更。もう走った時間より短ければ 400、ランがなければ 409)と `DELETE /api/run/timer`。
- `bin/operator.rs`: 見張りを起動。OpenAPI にも登録した。
- テスト +14(判定 8、偽のコンポーネントでの振る舞い 6)。ライブラリ 857 件すべて通過、clippy(テスト込み)警告 0。

**UI:**
- `TimerService`: 1 引く方式を廃止した。残り時間 = サーバの `remaining_ms` − 状態を受け取ってからの経過。
  - アラームは `timer_expired` の新しいものにだけ鳴らす。開いた時点ですでにあった知らせには鳴らさない。
  - 最初のクリックで音の準備をする(自動再生の制限への対策)。
- `app.ts`:
  - ヘッダに「1:23:45 left」と、超過時は赤で「+3:12 over」を出す。
  - アラームのダイアログもここで開く(どのページでも出る)。
- Control Panel:
  - Start でタイマーを送る。ブラウザからの Stop はやめた。
  - ラン中は「+30 min」「Cancel timer」、タイマーのないランには「Set timer」。上限は 1440 分。
- アラームのダイアログ: ラン番号、何分のタイマーか、止めたか続いているかを出す。
- テスト +13(サービス 7、Control Panel 4、App 2)、全 133 件通過。`dist/` はビルド済み。

**Mac での通しの確認(エミュレータ 1 台、MongoDB なし、ブラウザなし、`config_emulator_local_test.toml` のポートだけ変えた写し):**
- ラン 1(1 分、自動停止): Start から 60.3 秒で Operator が自分で止めた。`timer_expired` が出て、メモに「Run timer: 1 min, stops the run」が残った。
- ラン 2(知らせだけ): `PUT` で 3 分に延ばし、`DELETE` で取り消した。メモに 3 行残った。ランは続き、手で Stop した。
- 両ランとも `.delila` ができた。

**シフトマニュアル:** §1 の 4(上限 1440 分、ブラウザを閉じても止まる、全ページの上に残り時間)、§3(+30 min、Cancel timer、Set timer)、§4 の 1(時間切れのダイアログ)を新しい動きに合わせた。§1 の「当面の運用」の注意書きは入れていない(配備で不要になるため)。

## 7. 配備 (2026-11-10、ユーザー「全部やっちゃってください」)

- コミット cc5b974(タイマー)、07bf788(シフトマニュアル)、push 済み。
- pf-qaq: 16:33 に Idle(次はラン 80、書き残し 0、Tune Up なし)を確認 → c044f82 系から 07bf788 に早送り、ビルド 46 秒・警告 0 → 16:34:39 に DAQ を再起動(`logs/20261110_163440`、強制終了なし)。
- 確認:
  - Operator のログに「Run timer watcher started」。
  - 6 台とも 1 回目でつながり、`PLL locked`。
  - 配信中の `main.js` が新しいビルドと同じ(md5 一致、ヘッダのタイマーのコード入り)。
  - `PUT /api/run/timer` はランがないので 409(設計どおり)。
- ランは始めていない(状態は Idle、次はラン 80。Start が Configure もする)。

## 8. 実機での確認 (2026-11-10、ユーザー、`config/config_daq_test_pfqaq.toml`)

- 16:43 に DAQ_Test の設定で起動した(実験名 DAQ_Test、保存先 `/data/DAQ_Test`、eLog と InfluxDB なし)。
- ラン 1: 14:44:39 UTC に「Run timer: 1 min, stops the run」で開始。60.8 秒後の 14:45:40 に Operator が止めた(「Run timer: the time is up — stopping the run like the Stop button」)。ラン記録は Completed、60 秒。ファイルは `run0001_0000_DAQ_Test.delila` / `.root`。
- ユーザー: 「問い合わせのタイミングで 1 秒ごとの表示がちょっとアレなときもありますが OK」。原因は、表示の 1 秒ごとの更新と 1 秒ごとの状態の問い合わせが独立していること。直すなら表示の更新を 0.25 秒ごとにする(今はしない)。
- 16:52 にユーザーが N_Wall の設定(`config.toml`)で DAQ を起動し直した。`/data/DAQ_Test` は残っている(テストのデータ、消してよい)。
