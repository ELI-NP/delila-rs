# TODO 78 — Operator: スタート失敗からの復帰と「Running なのにイベント 0」の警告

**Status: 📋 起票 (2026-10-29)**

**関連:** [77_multiboard_start_sync.md](77_multiboard_start_sync.md) §4(DIG1 スレーブは S-IN が来なくても Running を報告) /
[73_campaign_2026_11_pfqaq.md](73_campaign_2026_11_pfqaq.md)(N_Wall は 10 台のカスケード) /
`docs/operations_manual.md` §7「Start が完了せず、開始と失敗を繰り返す場合」

---

## 1. 発端 (2026-10-29)

10/22 に S-IN の信号が来ないまま Start したとき、開始と失敗が続き、**知っている人が `stop_daq.sh` で殺すしかなかった**
(ユーザー: 「復帰するパスもあるといい」)。N_Wall はボードが 10 台になるので、詰まったときに誰でも戻れる道が要る。

## 2. 今の挙動(コード確認、2026-10-29)

- `run_start`(`src/operator/routes/status.rs`)は Reset → Configure → Arm(`arm_all_sync`)→ Start(`start_all_sync`)。
- Start の段で、あるグループが Running にならないと、**エラー(408「Start phase failed」)を返して終わる。始まったコンポーネントは
  そのまま**(Recorder / Monitor / Merger と、マスターのデジタイザは Running)。
- 失敗した Reader は目標が Running のままなので、**ハードの Start を 5 秒ごとに再試行し続ける**(`read_loop_dig1.rs` / `read_loop_dig2.rs`)。
- Stop や Reset を送れば止まる(目標が下がる)。UI の Start は毎回 Reset から始まる。つまり道はあるが、
  **何が起きていて何を押せばよいかが画面から分からない**。
- 半分動いている間、マスターはデータを取り、Recorder は書いている可能性がある(要確認: その間のファイルの扱い)。

## 3. やること(案)

| # | 内容 |
|---|---|
| (a) | **Running 中の無イベント警告:** あるソースのイベント数が N 秒増えないとき、ログ・UI・Grafana(アノテーション)に警告。Si は低レートなので N はソースごとに設定(TOML)。「S-IN 待ちのまま Running」(TODO 77 §4)を拾う |
| (b) | **Start 失敗時の巻き戻し:** Start の段で失敗したら、始まったコンポーネントを Stop し、全体を一貫した状態(Configured)に戻す。失敗したボードと最後のエラー(例: `PSD2-…: CAEN -6 swstartacquisition`)をレスポンスに入れ、「S-IN の信号を確認」などの手がかりを出す |
| (c) | **UI:** 失敗の理由とボード名を表示し、Reset → 設定の修正 → Start の流れが画面から分かるようにする(`stop_daq.sh` 不要) |
| (d) | 巻き戻しの間に書かれたファイル(あれば)の扱いを決める(残す / 消さない / ランとして記録しない) |

## 4. 確認の方法(案)

- エミュレータに「Start が必ず失敗する」フックを入れ(TODO 69 の `scripts/write_failure_test.sh` のフックと同じ考え方)、
  E2E: Start 失敗 → 巻き戻し → 全体が Configured → フックを外して Start → 成功。
- 実機: pf-qaq で S-IN を抜いた状態の Start(TODO 77 の T1 と同じ日に)。

## 5. 要確認(ユーザー)

1. 無イベント警告の N(秒)の目安。Si は何秒くらい鳴らないことがあり得るか
2. 警告をどこに出したいか(UI、Grafana、eLog への自動投稿)
