# TODO 91 — Operator の画面から DAQ を再起動する(安全装置つき)

**Status: 🔧 実装済み・Mac で E2E 済み (2026-11-20、コミット済み 2026-11-23 `40d195e`) — 実機(pf-qaq)への配備は 11B_208Pb(11-22〜28)の後か、ユーザーが決める**

**関連:** [85_digitizer_pll_guard.md](85_digitizer_pll_guard.md)(「DAQ を再起動すれば直る」場面) /
[84_high_rate_investigation.md](84_high_rate_investigation.md)(キューの大きさ) /
[68_backlog_watermark.md](68_backlog_watermark.md)(drain-first stop) /
[78_operator_start_failure_recovery.md](78_operator_start_failure_recovery.md)

---

## 1. なぜ要るか

ランの合間に DAQ を再起動すれば直る場面が、このところ何度もあった。

| 日 | 何が起きたか |
|---|---|
| 2026-11-10 | Reader が、開けなかったボードを開き直せないまま固まった。DAQ の再起動で直った(TODO 85) |
| 2026-11-20 | VX2730 のリセットが失敗して `jesd_unlock` になった。直すには DAQ の再起動が要る(TODO 88 §5.7) |
| ほか | FELib の −15(接続が死んでいる)は自動ではつなぎ直さない。DAQ の再起動で直す |

今はそのたびに ssh して `stop_daq.sh` → `start_daq.sh` を打つ必要がある。シフトの人にはこれ自体が危ない。

- **実例: ラン 45・46 のとき、Recorder のキューに溜まっていた 85 GB を、再起動で失った。**
- CLAUDE.md の絶対ルール「データを落とさない」に直接かかわる。

## 2. 危ないところと対策

| 危ないところ | 対策 |
|---|---|
| **キューが残ったまま再起動すると、データが消える** | 条件をすべて満たすときだけ受け付ける(§3.1)。1 つでも欠けたら、理由を表示して断る |
| Operator が自分自身を再起動する作りになる。途中で失敗すると UI が消え、原因が見えなくなる | 再起動は Operator の外のプロセスにやらせる(§3.2)。止める前に設定を読めるか確かめる(preflight)。結果はファイルに残し、戻ってきた Operator が表示する |
| 誤操作 | 確認の手順(`RESTART` と打つ)。続けて押せないように 2 分の間隔。誰がいつ押したかを残す |
| ハードの故障(ケーブルなど)は再起動では直らない | 間隔と回数の表示で、押し続けるループに気づけるようにする。画面に「直らなければハードを確認」と出す |
| 認証がない | 今の Start・Stop と同じ条件(危険が増えるわけではない)。v1 では入れない |

## 3. 設計(案)

### 3.1 受け付ける条件(Operator 側、純粋な関数 + テスト。`run_guard::decide` と同じ形)

- `current_run` がない。状態が Running でも Armed でもない。drain-first stop の途中でない。Tune Up 中でない。
- Recorder と Merger の `queue_bytes` が 0。
- **ROOT の sink のキューが 0。**
  - 今の Operator は sink のキューを見ていない。sink(thgem_sink、nwall_sink、root_sink)はキューの深さをログの 1 行(`q[sort=… work=… write=… disp=…]`)にしか出しておらず、HTTP で見られるのは `/WriteStatus` だけ。
  - そこで、sink に HTTP の項目(例 `/QueueStatus`)を足し、Operator が `[network.root_sink].http_port` から読む。**C++ 側の変更が要る。**
- 断るときは HTTP 409 で理由を返す。例:「Recorder のキューに 12.3 GB 残っています」「ラン 5 が実行中です」。

### 3.2 再起動するプロセス

- **案 A(推奨、KISS):** Operator が `setsid` で切り離した補助スクリプト `scripts/restart_daq.sh` を起動して、すぐ 202 を返す。スクリプトの動き:
  1. preflight: `config.toml` を Operator と同じ方法で読めるか確かめる(TODO 88 で読めない設定はエラーで止まるようにした。その確認だけをするオプションを足す)。sink の実行ファイルがあるかも見る。
     **失敗したら何も止めずに終わり、結果を書く。**
  2. `stop_daq.sh` → `start_daq.sh`(同じ `config.toml`。v1 では設定の切り替えはしない)。
  3. 結果を `logs/last_restart.json` に書く(開始時刻、終了時刻、成否、`start_daq.sh` の要約、失敗なら `operator.log` の末尾)。
  4. 新しい Operator は起動時にこれを読み、画面に一度だけ表示する。
- 案 B: 既存の `node_agent`(REST でプロセスを管理する)を常駐させる。部品は揃っているが、運用した実績がなく、常駐が 1 つ増える。

### 3.3 画面

- Control のパネルに「DAQ を再起動」を置く。Idle / Configured / Error / Mixed のときだけ押せる。
- 確認のダイアログ: 条件の一覧(どれも緑であること)+ `RESTART` と打つ欄。
- 押したあと: 「再起動中…」を表示し、Operator が戻るまで status を読み続ける(最大 2 分)。戻ったら結果のバナー。
- 2 分以内に戻らなければ「ssh して `logs/latest/operator.log` を確認」と表示する(UI からはそれ以上できない)。
- UI を変えたら `npm run build` をして、`dist/` も同じコミットに入れる。

### 3.4 記録

- Operator のログに WARN。押した側の IP、条件の値、結果。
- eLog にも 1 件書くか(案)。

## 4. 試験

- 単体: 受け付ける条件の関数(ラン中、Tune Up 中、Recorder や Merger のキュー > 0、sink のキュー > 0、sink に問い合わせできない、をそれぞれ断る)。
- Mac での E2E(エミュレータの設定):
  - ボタン → 再起動 → Operator が戻り、全コンポーネント Idle、結果のバナー。
  - ラン中・キューあり(わざと遅い Recorder)を断る。
  - preflight の失敗(壊れた TOML)では何も止まらない。
- 実機: 11B_208Pb の後、DAQ_Test の設定で一度。

## 5. 未定

- 配備の時期(おすすめ: 11B_208Pb の後)。
- eLog に書くか。
- 設定の切り替え(DAQ_Test ↔ 本番)を画面からできるようにするか。v1 では入れない(間違った設定で起動する危険があるため)。

---

## 6. 実装と試験の記録 (2026-11-20、Mac のみ。実機には触れていない)

案 A(切り離したスクリプト)で実装した。

### 6.1 作ったもの

| 部分 | 内容 | ファイル |
|---|---|---|
| sink のキュー | 3 つの sink に HTTP の Text 項目 `/QueueStatus` を追加。値は `idle sort=0 work=0 write=0 disp=0`(run file を開いているあいだは `writing`)。`/WriteStatus` と同じ場所で更新する。記録の処理には触れていない。読み方は `curl http://<host>:8090/QueueStatus/item.json` の `value`(`h.json` には値が出ない) | `tools/root_sink/{standard/root_sink,nwall/nwall_sink,thgem/thgem_sink}.cxx`、`tools/root_sink/README.md` |
| 受け付ける条件 | 純粋な関数 `daq_restart::refusal_reasons`(理由のリスト、空なら可)。sink の文字列の解釈 `parse_queue_status`(厳密。読めなければ「不明」=断る) | `src/operator/daq_restart.rs`(新規) |
| API | `POST /api/daq/restart`。202 = 受け付けた、409 = 断った(理由つき)、429 = 2 分以内の 2 回目、500 = スクリプトを起動できない。utoipa に登録 | `src/operator/routes/daq.rs`(新規)、`routes/mod.rs` |
| 設定 | `[network.root_sink]` の `program` と `http_port` だけを Rust で読む(ほかのキーは start_daq.sh のもの)。`http_port` がなければ sink の既定 8090、`0` なら HTTP なし → 断る | `src/config/mod.rs`、`src/bin/operator.rs` |
| 設定の確認 | `operator --config X --check-config`: `load_config` だけをして終わる(0 = 使える) | `src/bin/operator.rs` |
| 再起動のスクリプト | 自分を切り離して起動し直す → preflight(Operator のバイナリ、`--check-config`、sink の実行ファイルを start_daq.sh と同じ順で探す)。**失敗なら何も止めずに結果を書いて終わる** → `stop_daq.sh` → `start_daq.sh`(同じ config、Operator が MongoDB なしなら `--no-mongo`)→ 新しい Operator が全コンポーネント online で答えるまで待つ(既定 90 s)→ `logs/last_restart.json`(一時ファイル → rename)。失敗なら `operator.log` の末尾 20 行も入れる。出力は `logs/restart_daq.log` | `scripts/restart_daq.sh`(新規) |
| 結果の表示 | `/api/status` の `last_restart`(終わってから 10 分間)。新しい Operator は起動時に読み、さらに 180 s のあいだ 1 秒ごとに読み直す(スクリプトは新しい Operator が答えてから結果を書くため) | `daq_restart.rs`、`routes/status.rs`、`bin/operator.rs` |
| 画面 | Control パネルの「⋮」メニューに「Restart DAQ (all components)」。確認のダイアログ(条件の一覧 + `RESTART` と打つ欄)。送ったあとは「Restarting the DAQ…」と進行バー、**自分の request id の結果**が `/api/status` に出たら終わり。2 分で戻らなければ「DAQ ホストで `logs/restart_daq.log` と `logs/latest/operator.log` を確認」。結果のバナー(成功は緑、preflight の失敗は「nothing was stopped」、起動の失敗はログの末尾つき)。`dist/` もビルドした | `web/operator-ui/src/app/components/control-panel/{control-panel.component.ts,restart-daq-dialog.component.ts}`、`models/types.ts`、`services/operator.service.ts` |
| 記録 | Operator のログに WARN: 押した側のアドレス(`from=`)、断った理由、受け付けた request id、スクリプトの pid、config | `routes/daq.rs`、`bin/operator.rs`(`into_make_service_with_connect_info`) |

### 6.2 断る条件(どれか 1 つで 409)

- ランが進行中(`current_run` がある)。Tune Up 中。run-control のロックが取れない(start、stop、drain-first stop の途中)。
- Armed か Running のコンポーネントがある。
- Merger か Recorder が答えない、metrics がない、`queue_bytes` > 0。
- sink が設定されていて、問い合わせできない・読めない(TODO 91 より前の sink もここに入る)、`writing`、sort + work + write > 0。
- 次は断らない: Monitor のキュー、sink の `disp`(どちらも表示用で、落としてよい経路)。設定に `[network.root_sink]` がなければ sink は見ない。Reader が offline(= Degraded)でも断らない(再起動で直したい場面そのもの)。
- 受け付けたあとは、終わるまで Configure・Arm・Start・`/api/run/start`・Tune Up の開始も 409 で断る(確認と停止のあいだに何も始まらないように。結果が出ないままでも 180 s で解除)。

### 6.3 試験

単体(Rust、TDD):
- `daq_restart` 13 件: 静かな DAQ は可、ラン・Tune Up・コマンド実行中、Armed/Running、Recorder/Merger のキュー > 0、答えない/metrics なし、Monitor と sources のキューは断らない、sink の `writing`/キュー > 0(`disp` だけは可)、sink に問い合わせできない、文字列の厳密な解釈、sink の節なし/HTTP なし/応答なし、2 分の間隔、結果の新しさ、結果ファイルの読み書きと壊れたファイル。
- handler 4 件(偽のコンポーネント、`routes/status.rs`): ラン中は 409、キュー 5 MB は 409、sink が答えないと 409、受け付け → 実行中は configure が 409 → preflight 失敗の結果が `/api/status` に出る → 2 回目は 429。
- 設定 1 件、`load_config` 1 件(sink の port: 指定、既定 8090、0)。
- 結果: `cargo fmt --check` 差分なし、`cargo clippy --tests -- -D warnings` と `--all-targets` 警告 0、`cargo test` 901 passed / 0 failed / 25 ignored(lib 889、うちこの TODO で 18 件、bin operator 4、うち 1 件)。

UI: Karma 156/156(この TODO で 7 件: 押せる状態、キューが残る条件の表示、自分の request id の結果で待ちが終わる・古い結果では終わらない、409 では待たない、preflight 失敗のバナー、ログの末尾、ダイアログは `RESTART` を打つまで押せない)。`npm run build` 済み(予算の警告は既存の waveform のもの)。

sink: Mac(ROOT 6.40.04、Homebrew の libomp・libzmq)で 3 つとも `-Wall -Wextra` 警告 0 でビルド。root_sink を起動して `/QueueStatus/item.json` が `idle sort=0 work=0 write=0 disp=0` を返すことを確認。

E2E(`scripts/restart_daq_test.sh`、エミュレータ 1 台 + Merger + Recorder + Monitor + root_sink、port 9097 / 6855〜6890 / 18086 / 18094、出力は scratchpad):

| | 内容 | 結果 |
|---|---|---|
| (b) | ラン 1 の実行中に再起動 | 409「run 1 is active … emu-0 (Running) … the ROOT sink still has a run file open」(sort=1 が出た回もある)。何も止まらない |
| (d) | 壊れた TOML(`[[network.sources]]` に id なし)でスクリプトを直接実行 | `ok=false stage=preflight`、Operator の pid は変わらず、全コンポーネント online のまま |
| (a) | 再起動 | 202 → Operator の pid が変わる(最終回 89555 → 89894。同じ版で 2 回合格)→ 新しい Operator の `/api/status` に同じ request id で `ok=true`「4 components online, state Idle」、sink は `idle sort=0 work=0 write=0 disp=0` |
| (c) | sink を止めてから再起動 | 409「the ROOT sink's queue cannot be read (…connection refused)」 |

途中で見つけて直したこと:
1. 最初はスクリプトが Operator の子のままで、`stop_daq.sh` が古い Operator を止めなかった(macOS の `pkill` は自分の祖先を対象から外す)。新しい Operator は port が使われていて起動できず、それでも古い Operator が答えるので「成功」と書かれていた。→ スクリプトが自分を切り離して起動し直す(`setsid`、なければ `nohup`。Operator 側でも `process_group(0)`)。
2. スクリプトは新しい Operator が答えてから結果を書くので、起動時に 1 回読むだけでは前回の結果しか見えなかった。→ 起動後 180 s は読み直す。

### 6.4 計画にはなかった判断(メインのセッション・ユーザーが確認)

- 画面で押せる状態に **Degraded も入れた**(計画は Idle / Configured / Error / Mixed)。Reader が落ちた場面が主な用途のため。Merger・Recorder が落ちているときはキューが分からないので、サーバーが断る。
- Monitor のキューと sink の `disp` は条件に入れていない(表示用、CLAUDE.md の例外)。
- 2 分の間隔は 409 ではなく 429 にした。
- 「一度だけ表示」は、サーバーは 10 分間出し続け、各ページで閉じられる形にした。そのため、再起動のあと 10 分以内に `start_daq.sh` で起動し直しても、前の結果が出る。
- 受け付けたあとは Start などを断る(`pending`)。
- eLog には書いていない(§5 の未定のまま)。

### 6.5 残り

- **実機での確認と配備**(11B_208Pb の後か、ユーザーが決める)。pf-qaq では **thgem_sink を作り直すまで、sink が `[network.root_sink]` にある設定では必ず断られる**(古い sink には `/QueueStatus` がない → 「cannot be read」)。安全側だが、配備のときは sink も同時に更新すること。
- Linux での `setsid` の経路は未試験(Mac には `setsid` がなく `nohup` の経路を通った)。Linux の `pkill` は祖先を外さないが、切り離してあるのでどちらでも動くはず。
- E2E は root_sink だけ。nwall_sink と thgem_sink はビルドだけ(変更は 3 つとも同じ)。
- 「キューが残っている」の E2E は、sink が答えない場合とラン中の sink(`writing`)だけ。Recorder・Merger のキュー > 0 は単体と handler の試験(わざと遅い Recorder の E2E はしていない)。
- 画面は Karma と API の E2E だけで、ブラウザで実際には押していない。
- シフトマニュアル(`docs/shift_manual_nwall.md`)への追記は未定。
- Mac の `logs/`(git の対象外)に E2E のログ、`logs/last_restart.json`、`logs/restart_daq.log` が残っている。`logs/latest` は E2E の回を指している。

