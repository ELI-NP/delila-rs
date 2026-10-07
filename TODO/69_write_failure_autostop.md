# TODO 69 — ディスク書き込み失敗でランを止める (Recorder / root_sink)

**Status: ✅ Phase 1〜3 実装・E2E 合格 (2026-10-07)** — Phase 1+2 = 18357a9(Rust)、Phase 3 = root_sink。実装の記録は §6

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

**✅ 実装済み (2026-10-07)** — `tools/root_sink/root_sink.cxx` の `Recorder`。
詳細は `tools/root_sink/README.md`「Write failures」/ `docs/root_sink_manual.md`「書き込み失敗」。

- 検知: 全 `TTree::Fill` の戻り値(< 0)+ 現在のファイル(`GetCurrentFile()`)の `kWriteError` /
  `IsZombie` を `TFile::Open` 後・チャンクごと・`AutoSave`・`Write`・`Close` 後に確認。
  ディスク満杯の初発は short write(errno は古いまま、ファイルは writable のまま)なので
  ERROR 行に errno は出さず、直前の ROOT の行 + `MB free`(statvfs)で原因を示す。
- 最初の失敗で ERROR 1 行 + ラン単位のラッチ。以後そのランのファイルには Fill/AutoSave しない
  (受信・ソート・EB・モニタは継続、書かなかった hits/events を数える)。ラン終了時は
  `TTree::Write` なしで Close → 通常名にリネーム(消さない)、まとめ行に `INCOMPLETE`。
  開けば ROOT の自動復旧で最後の AutoSave 時点のツリーが読める。
- 表示: status 行 `| WRITE FAILED: run file INCOMPLETE, unwritten hits=N events=M`、
  JSROOT のテキスト項目 `/WriteStatus`(`OK` / `WRITE FAILED ...`、`/WriteStatus/item.json`)。
  次のランがファイルを開くとリセット。`TFile::Open` 失敗も同じ扱い(`NOT recorded`)。
- **Operator 連携はしない(決定)**: `.delila` が正本で、ランを止めるのは Phase 1 の watcher。
- E2E(macOS、8 MB HFS+ の hdiutil イメージ): 2.5 M ヒットを 50 kHz で流し 1,081,018 ヒットで満杯 →
  ERROR 1 行(ROOT のエラーはラン全体で 4 行。失敗後も Fill し続ける単体プローブでは約 12,500 行)、
  status 行の unwritten が増加、モニタ継続、finalize 正常。残したファイルは 1,018,685 ヒット / 14,857 イベント(最後の AutoSave)を
  全エントリ読める。空き 20 KB でのラン開始 → 最初の AutoSave で検知。SIGTERM → provisional 保持 +
  INCOMPLETE 行。空きを戻した次のランは `OK`。読み取り専用ディレクトリ → `TFile::Open` で検知、
  全 146,375 ヒットを unwritten に計上。健全ラン(pf-qaq run 56 リプレイ)は旧ビルドと hits IDENTICAL・
  events digest 一致・sidecar 一致。単体テスト 754 / 257 / 244 全パス。
- 限界: ランの最初の AutoSave(既定 30 秒)より前に失敗するとツリーは空で復旧される
  (basket はファイルにあるがツリーから参照されない)。

## 5. テスト計画

- 単体: Recorder writer に env ゲートの故障注入フック(TODO 68 の `test_write_delay_ms` と同じ流儀、
  例: N バイト書いたら ENOSPC を返す)→ `write_failed` が立つ・metrics に出ることを確認。
- E2E(Linux): 小さい loopback / tmpfs を `output_dir` に → emulator で埋める →
  Operator が poll 間隔内にランを止めること、失敗から停止までの欠損イベント数を報告できることを確認。
- Phase 2: 空き容量を hard 未満にして Start 拒否、Running 中に hard を割って drain-first stop で
  footer 付きの正常ファイルになることを確認。
- §2 の「途中ブロック」問題: 故障注入後に空きを戻して書き込み再開 → `recover` / `validate` で
  どこまで読めるか確認。読めなくなるなら、失敗後は書き込みを再開せず止めるのが正しい。

## 6. Phase 1 + 2 の実装 (2026-10-07、18357a9)

§4 の「実装時に決めること」への答え: ① opt-out は作らない(誤検知の余地が無く、障害解析でもランを続ける意味が無い)、
② ポーリングは 2 秒固定、backlog watcher とは別タスク(backlog は opt-in のまま触らない)、
③ 複数の書き手のどれか 1 つでも失敗したら止める。

**Recorder** (`src/recorder/mod.rs`):
- バッチごとに flush する。`current_file_size` は「カーネルに渡った完全な単位(ヘッダと丸ごとのブロック)」だけを数える。
- 書き込み失敗時(`abandon_after_failure`): 途中まで書かれたブロックを切り落とし、フッタを付けて閉じる → **正常なファイル**として読める。
  そのランではそれ以上書かず(次のファイルも開かない)、届いたバッチは `unwritten_events` に数える。
  フッタも書けなければ最後の完全なブロックで切って残す(`recover` で全部読める)。ヘッダすら書けなかったファイルは消す。
- §2 の「途中ブロック」問題は実在した: BufWriter は失敗したブロックの長さ 4 バイトをバッファに残し、空きが戻ると次のブロックの前に書き出す → そこから後ろが読めなくなる。上記の切り落としと「失敗後は書かない」で解消。
- 空き容量: `output_dir` を statvfs(rustix、unsafe なし)。TOML `disk_soft_free_mb`(既定 20 GiB)/ `disk_hard_free_mb`(既定 2 GiB)、0 で無効。
- metrics: `write_failed`、`unwritten_events`、`disk_free_bytes`、`disk_level`。状態表示に `WRITE FAILED (N events not written)` / `DISK LOW`。InfluxDB に `disk` 測定。
- テスト用フック `DELILA_TEST_FAIL_WRITE_AT_BYTES`(ブロックの半分まで書いて満杯のふり、起動時に警告)。

**Operator** (`src/operator/run_guard.rs`、常時動作):
- `write_failed` → 即時停止(`failure_stop`: ソースから `stop_all`、ラン履歴は Error、停止理由に最終的な未書き込みイベント数)。
- `disk_level == 2` → drain-first 停止(TODO 68 の仕組み、ラン履歴は Completed + 理由)。
- Start 拒否(HTTP 507): 空きが hard 未満のとき(`/api/start`・`/api/run/start`)、書き込み失敗のラッチが残っているとき(`/api/start` のみ。`/api/run/start` は Reset/Configure で解除するので通す)。

**読み手の既存バグ(同時に修正):** フッタの無いファイル(クラッシュ・上記の切り落とし)でも末尾 64 バイトをフッタ扱いし、最後のブロックを読み落としていた。
Rust の `DataFileReader` と C++ の `TDelila.hpp` の両方。旧 delila2root はフッタを外したファイルで 27,100 / 27,200 イベント、修正後 27,200。

**E2E** (`scripts/write_failure_test.sh`、macOS):

| モード | 内容 | 結果 |
|---|---|---|
| hook | 5 MB で満杯のふり | 失敗から 1.7 秒で停止。送信 372,000 = 書けた 200,000 + 未書き込み 172,000。ファイルは正常(200,000) |
| disk | 32 MB のイメージで本物の ENOSPC | 途中ブロック 48 KB を切り落とし、正常なファイル(1,310,000)。未書き込み 150,000 |
| lowdisk | 64 MB のイメージ、soft 40 / hard 24 MB | hard を割って drain-first 停止、送信 1,656,000 を全部書いた正常なファイル。次の Start は 507 |

単体: Recorder の失敗経路 3 本 + 読み手 2 本 + `disk_space` 4 本 + `run_guard` 7 本(切り落としと読み手の修正は、外すと落ちることを確認)。全体 771 テスト。

