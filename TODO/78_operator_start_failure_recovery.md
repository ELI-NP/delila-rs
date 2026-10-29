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
- 10/22 のログ(`logs/20261022_170246/operator.log`)では、17:08:19〜17:09:16 に run/start(Reset から始まる一連)が約 9 秒おきに
  8 回届き、Stop は届いていない。UI に自動の再試行はなく、Mixed では Start も押せないので、**この 8 回の出どころは不明**
  (別のタブや Swagger など。ユーザーに確認)。

## 3. やること(案)

| # | 優先 | 内容 |
|---|---|---|
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
2. 10/22 の 8 回の run/start は、手で Start を押したものか(UI では Mixed で Start も無効のはず)
