# TODO 78 — Operator: スタート失敗からの復帰と「Running なのにイベント 0」の警告

**Status: 📋 起票 (2026-10-29)**

**関連:** [77_multiboard_start_sync.md](77_multiboard_start_sync.md) §4(DIG1 スレーブは S-IN が来なくても Running を報告) /
[73_campaign_2026_11_pfqaq.md](73_campaign_2026_11_pfqaq.md)(N_Wall は 10 台のカスケード) /
`docs/operations_manual.md` §7「Start が完了せず、開始と失敗を繰り返す場合」

---

## 1. 発端 (2026-10-29)

10/22 に S-IN の信号が来ないまま Start したとき、開始と失敗が続き、**知っている人が `stop_daq.sh` で殺すしかなかった**
(ユーザー: 「復帰するパスもあるといい」)。N_Wall はボードが 10 台になるので、詰まったときに誰でも戻れる道が要る。

## 2. 今の挙動(コードとログで確認、2026-10-29)

- `run_start`(`src/operator/routes/status.rs`)は Reset → Configure → Arm(`arm_all_sync`)→ Start(`start_all_sync`)。
- Start の段で、あるグループが Running にならないと、**エラー(408「Start phase failed」)を返して終わる。始まったコンポーネントは
  そのまま**(Recorder / Monitor / Merger と、マスターのデジタイザは Running)。
- 失敗した Reader は目標が Running のままなので、**ハードの Start を 5 秒ごとに再試行し続ける**(`read_loop_dig1.rs` / `read_loop_dig2.rs`)。
- **このときシステムの状態は「Mixed」**(`SystemState::from_components`、`src/operator/mod.rs`: 状態がそろっていなければ Mixed)。
  **UI は Mixed で Start・Stop・Reset・Configure をすべて無効にする**(`getButtonStates`、`web/operator-ui/src/app/models/types.ts`:
  「Transitional states - all disabled」、`canStart()` も Idle / Configured / Armed のみ)。
  → **ユーザーの「Stop も Reset もできなかった」はこれ**。サーバーの API(`/api/stop`、`/api/reset`)なら効くはずだが、画面からは押せない。
  オフラインのコンポーネントがある「Degraded」でも同じく全部無効。
- 10/22 のログ(`logs/20261022_170246/operator.log`、時刻は UTC = 現地 −3 h)では、17:08:19〜17:09:16 に run/start(Reset から
  始まる一連)が約 9 秒おきに 8 回届き、Stop は届いていない。ユーザーは Start を押していない(グレーアウトしていた)。
- **8 回の出どころ = ブラウザ(Chrome 系)の自動の再送**(2026-10-29 判明)。
  - 失敗すると、run/start は **HTTP 408(Request Timeout)** を返す。
  - Chromium は、使い回した keep-alive の接続で 408 を受けると「サーバーが空き接続を閉じた」とみなし、
    **同じリクエスト(POST でも)を黙って送り直す**(2014 年から。[codereview 303443011](https://codereview.chromium.org/303443011)、bug 377581)。
    UI は 1 秒ごとに状態を問い合わせるので、使い回せる接続が常にある。
  - ログの証拠: 毎回「Starting group order=1」の約 5.08 秒後(Start の待ちが切れた直後)、**約 0.08 秒で次の Reset が来る**。
    人の操作ではこの間隔にならない。UI のコードに再試行はない(`operator.service.ts` / `control-panel.component.ts`)。
  - UI にはエラーが返らない(ブラウザが送り直すので)。**エラーの通知も出ず、画面は状態が行ったり来たりするだけ**。
  - **オペレーターのログにも WARN / ERROR が 1 行もない**。Start 失敗の 408 はログを書かずに返している(silent failure、CLAUDE.md 違反)。
- **Force Reset が効かなかった理由**(同じログ、14:09:12):
  - Force Reset は Configure / Arm の数秒間(状態が Configured / Armed)だけ押せる。ユーザーはそこで押した。
  - Reset は `run_control_lock` で run/start と順番待ちになる(TODO 58 H14)。7 回目の run/start が 408 で終わった直後、
    **Force Reset(12.645)が実行され、その 5 ms 後(12.650)にブラウザが送り直した 8 回目の run/start が Reset → Configure → Arm → Start をやり直した**。
  - つまり **Force Reset は効いていたが、5 ms で上書きされた**。8 回目の途中でユーザーが `stop_daq.sh` で止めた(ログはそこで終わる)。
- 408 を返す箇所は 6 つ(`src/operator/routes/status.rs`): `configure` の Reset、`start` の Start、`run_start` の Reset / Configure / Arm / Start。
  どれも同じく自動で送り直されうる。

## 3. やること(案)

| # | 優先 | 内容 |
|---|---|---|
| (e) | **最優先** | **408 をやめる + 失敗をログに出す(数行):** 6 か所の `StatusCode::REQUEST_TIMEOUT` を 504(Gateway Timeout: 下流のコンポーネントが時間内に応じない)にする。ブラウザは 5xx の POST を送り直さない。各失敗の return の前に `tracing::error!` を入れる。テスト: 失敗時のステータスが 408 でないこと。**これだけで「無限に開始と失敗を繰り返す」は止まり、UI にエラーの通知が出る** |
| (b) | **高** | **Start 失敗時の巻き戻し(サーバー):** Start の段で失敗したら、始まったコンポーネントを Stop し、全体を一貫した状態(Configured)に戻す。失敗したボードと最後のエラー(例: `PSD2-…: CAEN -6 swstartacquisition`)をレスポンスに入れ、「S-IN の信号を確認」などの手がかりを出す |
| (c) | **高** | **UI: Mixed / Degraded でも Stop と Reset を押せるようにする**(今は全部無効)。失敗の理由とボード名を表示し、Reset → 設定の修正 → Start の流れが画面から分かるようにする(`stop_daq.sh` 不要) |
| (a) | 中 | **Running 中の無イベント警告:** あるソースのイベント数が N 秒増えないとき警告。Si は kHz 以上来るはずなので **N ≈ 1 秒**(ユーザー)。Grafana のレートでも分かるが、**属人的にならないよう、ユーザー(実験者)に見える形で出す**(UI の表示が第一候補) |
| (d) | 低 | 巻き戻しの間に書かれたファイル(あれば)の扱いを決める(残す / ランとして記録しない) |

## 4. 確認の方法(案)

- エミュレータに「Start が必ず失敗する」フックを入れ(TODO 69 の `scripts/write_failure_test.sh` のフックと同じ考え方)、
  E2E: Start 失敗 → 巻き戻し → 全体が Configured → フックを外して Start → 成功。
- 実機: pf-qaq で S-IN を抜いた状態の Start(TODO 77 の T1 と同じ日に)。

## 5. 要確認(ユーザー)

1. ~~無イベント警告の N~~ → 約 1 秒(kHz 以上来るはず)。警告は Grafana でも見られるが、実験者に見える形にする(回答済み)
2. ~~10/22 の 8 回の run/start は、手で Start を押したものか~~ → 押していない(グレーアウト)。Force Reset は押した。
   出どころはブラウザの 408 自動再送(§2)
