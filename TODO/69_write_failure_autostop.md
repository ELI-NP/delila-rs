# TODO 69 — ディスク書き込み失敗でランを止める (Recorder / root_sink)

**Status: 📋 OPEN (2026-10-06) — Phase 1 はデフォルト ON で決定(§4)、未着手**

**関連:** CLAUDE.md 絶対ルール(落とすくらいなら止める) /
TODO 58 H7(`write_failed` ラッチ、[archive](archive/58_code_review_2026-06-10.md)) /
[68_backlog_watermark.md](68_backlog_watermark.md)(autostop watcher + drain-first stop の先例) /
[scripts/archive_sync.sh](../scripts/archive_sync.sh)(pf-qaq の暫定対策)

---

## 1. 発端 (2026-10-06)

ThGEM 新 PC(pf-qaq)のストレージ構築で、記録先を SSD(`/data`, 3.5 TB)、長期保存を
HDD ZFS(`/archive`)に分けた。SSD は 1 MHz 連続で約 2 日で埋まるため、
「満杯になったら何が起きるか」をコードで確認したところ、**満杯 = ラン継続のままデータ欠損**だった。

## 2. 現状(コード確認済み)

### Recorder

- 書き込み失敗(ENOSPC / I/O error)は `error!` + `write_failed` ラッチ
  ([recorder/mod.rs:1012-1018](../src/recorder/mod.rs#L1012-L1018))。
  close 失敗も同様([:1038-1043](../src/recorder/mod.rs#L1038-L1043), [:1093-1096](../src/recorder/mod.rs#L1093-L1096))。
- ラッチは `effective_state()` を `Error` にし、status に `, WRITE FAILED` を付ける
  ([:563-582](../src/recorder/mod.rs#L563-L582))。Configure / Reset で解除。
- **ランは止まらない。** Operator はこの Error を見て何もしない。止まる経路は opt-in の
  backlog autostop([backlog_watch.rs](../src/operator/backlog_watch.rs))だけで、これは
  `backlog_level >= 2` しか見ていない。
- 失敗後も writer は後続バッチを毎回書こうとする。途中で空きが戻ると書き込みが再開するが、
  失敗した位置に**途中まで書かれたブロック**が残る可能性がある。`.delila` は長さプレフィクス
  付きブロック列なので、その後ろのブロックが読めなくなる恐れあり(**要検証**: `format.rs`
  の `data_blocks()` / `validate` の挙動)。

### root_sink (C++)

- チェックしているのは `TFile::Open` の失敗だけ
  ([root_sink.cxx:533-540](../tools/root_sink/root_sink.cxx#L533-L540))。
- `Fill()`([:600](../tools/root_sink/root_sink.cxx#L600), [:607](../tools/root_sink/root_sink.cxx#L607))、
  `AutoSave`([:617-618](../tools/root_sink/root_sink.cxx#L617-L618))、
  `Write()` / `Close()`([:685-687](../tools/root_sink/root_sink.cxx#L685-L687))の失敗は見ていない。
  ROOT の SysError が stderr に出るだけで動き続ける。
- root_sink は受動的な SUB で、Operator への状態チャンネルを持たない。

### 空き容量の監視

- どこにも無い(statvfs / df 相当の呼び出しゼロ、InfluxDB にもディスク指標なし)。
  `docs/plans/grafana_monitoring.md` に `disk_usage` が「MVP 外の将来追加」として載っているだけ。

## 3. 暫定対策(運用側、2026-10-06 稼働)

pf-qaq の `scripts/archive_sync.sh`(cron 15 分): `/data` が 80 % を超えたら、`/archive` と
バイト一致した古いファイルを 70 % まで自動削除。満杯そのものを起こさないための外側の安全網で、
**書き込み失敗時にランが続く穴は塞いでいない**。他ホスト(side3, eliadeSN0N, .76)には無い。

## 4. 方針案(要決定)

### Phase 1 — 書き込み失敗でランを止める(本件の核心)

- Operator 側 watcher が Recorder の書き込み失敗を検知 → ラン停止。
  backlog_watch の型(status ポーリング、`run_control_lock` 下で check-then-act)をそのまま流用。
- 検知は status 文字列のパースではなく、`ComponentMetrics` に `write_failed: bool` を追加して読む。
- 停止の種類: **drain-first ではなく即時停止**が自然(書けないディスクへ drain しても意味がない)。
  source を先に止めて欠損を最小にする点は drain-first と同じ。
- 停止理由を Mongo の run 記録 / eLog に残す。
- **決定 (2026-10-06, ユーザー): デフォルト ON。** TOML セクション無しで常に動く。
  backlog autostop(TODO 68)が二重 opt-in なのは、水位の閾値がホストの RAM 次第で
  誤発火の余地があるため。書き込み失敗は誤検知の余地が無い事実なので、絶対ルールどおり常に止める。
- **実装時に決めること:**
  1. 無効化の手段(TOML で opt-out)を残すか。残さない方が KISS だが、障害解析で止めたくない場面があるか。
  2. ポーリング間隔(backlog watcher と共用するか)。
  3. 複数 Recorder 構成で 1 台だけ失敗した場合も止めるか(止める、が絶対ルールに沿う)。

### Phase 2 — 満杯になる前に止める(予防)

- Recorder が `output_dir` の空き容量(statvfs)を metrics で報告 → InfluxDB / Grafana。
- TODO 68 と同じ soft / hard 水位: soft で warn、hard で **drain-first stop**(まだ書けるうちに
  止めるので footer まで正常に閉じられる)。空きが hard 未満なら Start を拒否。
- Phase 1 は「失敗してから止める」、Phase 2 は「失敗する前に止める」。両方あるべき。

### Phase 3 — root_sink の書き込み失敗検知

- `AutoSave` / `Write` 後に `TFile::TestBit(TFile::kWriteError)`(または戻り値)を確認し、
  大声で出す(stderr + JSROOT HTTP の状態表示)。
- Operator 連携は root_sink にコマンド経路が無いので別途設計(HTTP status を Operator が読む等)。
  `.delila` が正本なので優先度は Phase 1/2 より下。

## 5. テスト計画

- 単体: Recorder writer に env ゲートの故障注入フック(TODO 68 の `test_write_delay_ms` と同じ流儀、
  例: N バイト書いたら ENOSPC を返す)→ `write_failed` が立つ・metrics に出ることを確認。
- E2E(Linux): 小さい loopback / tmpfs を `output_dir` に → emulator で埋める →
  Operator が poll 間隔内にランを止めること、失敗から停止までの欠損イベント数を報告できることを確認。
- Phase 2: 空き容量を hard 未満にして Start 拒否、Running 中に hard を割って drain-first stop で
  footer 付きの正常ファイルになることを確認。
- §2 の「途中ブロック」問題: 故障注入後に空きを戻して書き込み再開 → `recover` / `validate` で
  どこまで読めるか確認。読めなくなるなら、失敗後は書き込みを再開せず止めるのが正しい。
