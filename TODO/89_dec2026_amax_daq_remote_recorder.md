# TODO 89 — 12 月の AMax 実験(Si + LaBr3、VX2730): 読み出しは DAQ ホスト、記録は side3(ネットワーク透過テストを兼ねる)

**Status: 🚧 §3.3 リンク切れの見張り = 実装済み、Mac で E2E 合格 (2026-11-18、未コミット) — 残 = §2.3 中継 Merger、§3.1 / §3.2 別ホストでの起動、§3.4 ROOT 出力、ハード(§4)。日程と DAQ ホストは未定**

**関連:** [80_generic_root_sink.md](80_generic_root_sink.md)(hits / events の 2 ファイル、ROOT の書き込み方) /
[84_high_rate_investigation.md](84_high_rate_investigation.md)(Recorder と ROOT の速度の上限、バックログ自動停止) /
[69_write_failure_autostop.md](69_write_failure_autostop.md)(ディスクの見張り) /
[68_backlog_watermark.md](68_backlog_watermark.md)(drain-first stop) /
[64_amax_opendpp_params.md](64_amax_opendpp_params.md)・[67_amax_channel_pages_clamp.md](67_amax_channel_pages_clamp.md)(AMax) /
[81_delila_zst_export.md](81_delila_zst_export.md)(ホストの外に出すときの圧縮)

---

## 1. 決まったこと(ユーザー、2026-11-18)

| 項目 | 内容 |
|---|---|
| 記録先 | **side3 の `/data`**(ZFS raidz2、20 TB × 6、空き 71 TB、lz4、recordsize 1M、11-15 の scrub は異常なし)。ユーザー: 「Side3 しかない」「ZFS なのもありがたい」 |
| 使わないもの | gant の RAID(空き 6.0 + 11 TB、約 100 MB/s)。pf-qaq の `/archive`(33 TB、11B_208Pb で 11-28 まで使用中) |
| side3 の役目 | 解析用ホストから外して**ホールに入れ**、12 月のデータホストにする。解析のユーザーには自分のラップトップで作業してもらう |
| DAQ ホスト | **今と同じ構成か gant**(未確定)。Reader / Merger / Monitor / Operator はこちら |
| 構成の狙い | **Recorder と ROOT の sink を side3 に置く = ネットワーク透過テストを兼ねる** |
| 直結の線 | gant の SFP+ 10G は 4 口(X520 + 82599ES): **3 口がデジタイザ、空いている 1 口を side3 に直結できる**。side3 に SFP+ の NIC を足す(ロープロファイルのものがある。最悪 ELITPC から普通のカードを借りる)。DAC かファイバー、直結専用のプライベートサブネット |
| 波形 | **残す**(AMax はまだ開発中で信用しきれない) |
| 持ち帰り | 共同研究者は **ROOT ファイルだけ**を持ち帰る → TTree のブランチ設計が必要(§3.4) |
| 欠けたとき | **ラン中に Recorder までの途中でバッチが欠けたら、ランを止めてエラーを出す**(11-18)→ §3.3 で実装 |
| 日程・DAQ ホスト | side3 をホールに入れる日、12 月の実験の日、DAQ ホスト(今の構成か gant)は**どれも未定**(11-18) |

## 2. データ量と速度

### 2.1 見積もり(共同研究者のメール、11-13)

AMax(OpenDPP)形式、800 サンプル、60 時間、60 kcps、多重度 3 × 2 面 = 6、ヒットの 40 % に波形 → **Si で 51.8 TB**(96 % が波形)。
raw と merged の両方を残すと 100 TB を超える。先方への確認事項は §6。

DELILA 側の換算(11-13):

- `.delila` は波形を MessagePack の int 配列で持つので 1 サンプル約 3 B(2 B の 1.5 倍)。
- この見積もりの波形は約 **230 MB/s**、`.delila` の書き込みは ROOT を除いて約 **350 MB/s**。

### 2.2 side3 `/data` の書き込み速度(実測、2026-11-18)

`/dev/shm` に置いた圧縮できないデータを dd で書き込み(`conv=fsync`、プールの使用率 4 %)。

| 条件 | 速度 |
|---|---|
| 1 本(32 GiB) | **886 MB/s** |
| 2 本同時 | 2 × 412 = **824 MB/s**(`zpool iostat` では raidz2 のパリティ込みで約 1.3 GB/s) |

必要量(約 350 MB/s + ROOT)の**約 2 倍**。まだ測っていないもの: プールが埋まったとき(ZFS は 80 % を超えると遅くなる)、書きながら読むとき。

### 2.3 直結の線に乗る量(見積もり)

Merger の PUB を side3 の 2 つ(Recorder と ROOT の sink)が購読すると、同じデータが線を 2 回通る。
1 つあたり約 350 MB/s なら合計約 0.7 GB/s ≈ **5.6 Gb/s**。10G には収まるが、余裕は半分くらい。
iperf3 で実際の帯域を測ってから決める(§5 の 1)。

**対策 = side3 に中継の Merger を置く(ユーザーの案、11-18)。線を通るのは 1 回分(約 2.8 Gb/s)。**

```
DAQ ホスト: Reader × n → Merger ─┬→ Monitor
                                 └→ (10G 直結、1 回だけ) → side3: 中継 Merger ─┬→ Recorder
                                                                              └→ ROOT の sink
```

- **別の splitter コンポーネントは作らない。** Merger はもともと「SUB で受けたバイト列を、中身を見ずに PUB へ流す」だけ
  (ヘッダーだけ読む zero-copy、[src/merger/mod.rs](../src/merger/mod.rs) の冒頭)。splitter を作っても同じコードの写しになる。
  中継の Merger が転送するのは連番の付いた元のバッチそのままなので、Recorder の見張り(§3.3)は線の区間も含めて端から端まで効く。
- **今のままでは動かない。** 設定にも Operator にも `[network.merger]` は 1 つしかない。
  Operator が中継の Merger に指令を出さないと、中継は Running にならない。Running でない Merger は受けたものをすべて捨てる(Stop テールの扱い)。
- やること:
  - 中継の Merger を設定に書けるようにする(例: `[network.relay]`、`subscribe` = DAQ ホストの Merger、`pipeline_order` は Merger と Recorder の間)。
  - Operator のコンポーネント一覧に名前を付けて加える(例 `Merger (side3)`)。
  - side3 での起動(§3.2)。
  - Rust の変更は設定と Operator だけ。Merger 本体は変えない。

### 2.4 ROOT の波形圧縮(TODO 80 と合流)

| データ | 方式 | 縮み | 速度(1 スレッド) |
|---|---|---|---|
| gant run20171(ラボの AMax、2048 サンプル、雑音 2.5 LSB) | 一次差分 → zigzag → 下位・上位バイトを別ブランチ、ZSTD 1、1 MB basket | **3.91 倍**(理論上限 4.15 倍) | 301 MB/s |
| 同上 | 生のまま ZSTD 1 | 1.56 倍 | — |
| N_Wall ラン 122(ビーム中の実 Si、V1725、8 ns) | 同じ zigzag lo/hi | E sector 7.39 倍 / dE sector 6.34 倍 / dE ring 3.91 倍 | — |

**3 倍は安全**、それより上は検出器ごとの雑音次第。VX2730 は 2 ns サンプリングで条件が違うので要再測定。
波形 1 本ずつ符号化するので、どのエントリーも単独で復号できる。230 MB/s を 300 MB/s/スレッドで圧縮するので**並列化が必要**(nwall_sink は今は圧縮なしで書いている)。

## 3. DELILA 側でやること

### 3.1 Recorder を side3 で動かす

- TOML(案): `[network.recorder]` を `subscribe = "tcp://<gant の直結 IP>:5557"`、`command = "tcp://<side3 の直結 IP>:5580"` にする。
  Merger の PUB は `tcp://*:5557` で bind しているので、直結の口からも届く。
- Operator は `command` の文字列にそのままつなぐ(書き換えるのは `tcp://*:` を localhost にするところだけ)。**Rust の変更は要らないはず。まだ試していない。**
- ディスクの見張り(TODO 69)は Recorder が自分で `statvfs` を測り、status で返す([src/bin/recorder.rs:71](../src/bin/recorder.rs#L71)、[src/common/disk_space.rs](../src/common/disk_space.rs))。**別ホストでもそのまま効く。**
- Operator は Recorder の `output_dir` を読まない(grep で確認、11-18)。
- バックログ自動停止(TODO 68 / 84)は Recorder のキューを見る。`backlog_hard_limit_mb` は side3 の RAM(62 GB)に合わせて決め直す(pf-qaq は 251 GB で 40000)。
- 両ホストは**同じコミットのバイナリ**で動かす(MessagePack の形式を合わせるため)。side3 には x743 のビルドがすでにある。DAQ ホストが gant なら、side3 に AMax のビルドは要らない。

### 3.2 起動の仕方

今の `start_daq.sh` で別ホストを飛ばせるのはソースだけで、Recorder と ROOT の sink はいつもローカルで起動する。
また ROOT の sink には `--operator http://localhost:<port>` を渡すので、side3 で動かすと実験名が取れない。
C++ 側は任意の `http://` URL を受け付けるので、変更はスクリプトだけで済む。

- **案 A(推奨、KISS):** `[network.recorder]` と `[network.root_sink]` に `host` キーを足す。`start_daq.sh` はソースと同じ書き方で、別ホストのものを飛ばす。side3 では同じ TOML で「この 2 つだけ起動する」モードを使い、`--operator` には DAQ ホストの Operator の URL を渡す。
- 案 B: 既存の `node_agent`(REST でプロセスを管理する)を side3 に置き、Operator から起動する。部品は揃っているが、運用した実績がない。

### 3.3 リンク切れを見えるようにする — ✅ 実装済み (2026-11-18、未コミット)

**問題:** ZMQ の PUB は、購読者がつながっていない間のメッセージを捨てる。TCP の再送で済む短い乱れは問題ない。
しかし線が抜けたり NIC が落ちたりすると、その間に Recorder に届かなかった分は消える。それなのに誰も気づかなかった。
Merger は「ソース → Merger」の連番を見ていたが、Recorder は「Merger → Recorder」の連番を見ていなかった。

**作ったもの:**

| 場所 | 内容 |
|---|---|
| [src/common/sequence.rs](../src/common/sequence.rs) | Merger の連番チェック(`SourceStats` / `SeqEvent`)をそのまま共通に移した。Merger は `pub use` で使い続ける |
| Recorder の受信タスク | ソースごとの連番を受信タスクだけが持つ(Mutex なし)。Running に入るたびに消す(ソースは Start で 0 から振り直す)。欠けたら `lost_batches` に足し、ERROR のログ(`Sequence gap at the Recorder`)を出す。届いた分はそのまま書く。大きく戻ったら(100 より前)ソースの再起動として warn だけ |
| `ComponentMetrics.lost_batches` | Recorder が報告する。Start で 0 に戻る。status の文字列に `LOST n batches (sequence gaps)` |
| run guard(TODO 69) | `lost_batches > 0` → **drain-first stop**(届いた分は良いデータなので書き切る)。優先順 = 書き込み失敗 > 欠け > ディスク残量 |
| `AutoStopCause::DataLoss` + UI | バナー「Run N stopped: data lost before the Recorder」+ 線を確かめるよう案内。「files are complete」とは言わない |
| ラン記録 | `finish_run_bookkeeping` が**どの止め方でも**(guard、ふつうの Stop、drain-stop)欠けを見る。欠けがあればラン = Error、MongoDB と eLog の停止理由に「n batches lost before the Recorder (sequence gaps)」 |

**検証:**

- テスト: 連番の判定、本物の受信タスクに inproc の PUB から 0, 1, 3, 4 を流して 1 欠け・4 本とも書き込みへ(チェックを外すと失敗するのを確認)、guard の判定と優先順、停止理由の文言、UI のバナー。lib 871 本、clippy `--all-targets -D warnings`、UI の control-panel 10 本。
- Mac の E2E(emulator → Merger → **TCP 中継 = 線** → Recorder、Operator):
  - 線を切らない対照ラン: 欠け 0、ランは続く。
  - ラン中に中継を 2 秒落とす: Recorder が `missing_batches=105` を出す → 0.5 秒後に guard が drain-first stop → バナー `data_loss` / `stopped`。
  - どちらの `.delila` も `recover validate` で正常(ラン 2 = 届いた 275 ブロック、550,000 イベント)。
  - スクリプトは scratchpad の `link_loss_e2e.sh`(リポジトリには入れていない)。

**見えない場合(既知の限界):**

- ランの最初のバッチより前の欠け: 最初に届いた連番を起点にするため。
  - 「最初が 0 でなければ欠け」とはしなかった。前のランのバッチが Merger のチャンネルに残っていて Start 直後に届くと、誤って止まるため。
  - 線が Start の時点で切れているときは、Recorder に何も届かないことで分かる。ただし今は警告が出ない → 後で「Recorder にイベントが来ない」警告を考える(TODO 78 のソース用と同じ形)。
- 最後のバッチより後(EOS の直前)の欠け: 後ろに比べる相手がない。
- 副作用(意図どおり): Reader のデコード失敗で連番が飛んだ場合(`parallel_decode.rs` の Skip)も、これからはランが止まる。pf-qaq の最近のラン 12 本の Merger ログに gap は 0 件で、ふだんのランが誤って止まることはないと判断した。
- 実機の直結でケーブルを抜く試験は、線ができてから一回だけ行う(§5)。

### 3.4 ROOT の出力(TODO 80 と合流)

- ファイルを分ける: **軽い hits のファイル**(全員が持ち帰る)と **waves のファイル**。
- 波形は §2.4 の zigzag lo/hi + ZSTD 1 で書き、複数のスレッドで圧縮する。side3(8 コア 16 スレッド)では Recorder と同じホストで動く。
- ブランチの中身は共同研究者と決める。

## 4. ハードウェアとホストの準備

| 項目 | 内容 |
|---|---|
| side3 の NIC | SFP+ のカードを付け、gant の空き口と直結する。アドレスは例として `10.10.0.1/30`(gant)と `10.10.0.2/30`(side3)。MTU 9000 は iperf3 の結果を見て決める |
| side3 の BIOS | F14 のまま。**移動の前に更新する**。10 月の MCE による再起動と、11-13 の deferred hardware error が残っている |
| side3 の解析データ | `/home/daq/Analysis`(623 GB、`/home` が 11-18 に 100 % になった)と `/data/9MV2026`(2.9 TB)。**DELILA 側からは触らない**。扱いはユーザーと解析担当が決める |
| side3 の移動に伴う変更 | pf-qaq から N_Wall / 11B_208Pb の ROOT を引く cron(`scripts/data_pull.sh`)、side3 のアドレス(今は 192.168.147.183)を参照しているもの、Grafana と eLog のリンク |
| gant | 空きの SFP+ の口を確認する(読み出しだけ、DAQ には触らない) |
| 2 つ目のコピー | ユーザー: EOS に置ければよいが、そもそもアクセスできるか? → 11-18 に読み取りだけで調べた(下) |

**EOS への道(2026-11-18、読み取りだけ):**

- **ELI-Storage**(172.18.6.248、Mac から ssh 可): 1 コア / 3 GB の小さな VM。
  - EOS を `eosxd`(FUSE、autofs)でマウントしている。サーバは `eos.eli-np.ro`、認証は unix。
  - `/eos/users` は 1.7 PB 中 1.4 PB 空き。`/eos/users/gded` はこの VM のユーザー(gded)の持ち物で書ける。
- **今の side3**(192.168.147.183)からは、どちらにも届かない。
  - `eos.eli-np.ro` は名前が引けない。1094 番(xrootd)は閉じているか届かない。
  - ELI-Storage の 22 番にも届かない。
  - ホールに移った後のネットワークは分からない。
- したがって 2 つ目のコピーを EOS に置くには、IT に次のどちらかを頼む必要がある:
  - side3 から EOS への経路と、side3 用の EOS クライアントの認証
  - ELI-Storage 経由でコピーする場合: 1 コアの VM を通すので速度は要確認。60 時間で数十 TB を運ぶ
- 頼めるまでは、side3 の raidz2(2 台壊れても読める)が唯一のコピー。

## 5. 進め方(案)

1. 直結の線を張り、iperf3 で 10G 出るか測る。
2. gant のエミュレータ → side3 の中継 Merger(§2.3)→ Recorder で記録し、`.delila` の件数と中身をローカル記録と比べる。
3. ROOT の sink を side3 で動かす(§3.2 の起動、§3.4 の出力)。
4. リンクを切る試験を一回行い、§3.3 の見張りが欠けを出すか確かめる。
5. AMax の実機で通す。

## 6. 未定・確認待ち

**ユーザー:**

- side3 をホールに入れる日と 12 月の実験の日程(11-18: 未定)。これで決まるもの:
  - 11B_208Pb の ROOT を side3 に引くのをいつ止めるか
  - BIOS 更新の時期
- DAQ ホストは今の構成か gant か(11-18: 未定)。
- ~~Running 中に Merger → Recorder の欠けが出たら、ランを止めるか~~ → **止めてエラー**(11-18、§3.3 で実装)。
- 2 つ目のコピー: EOS にするなら IT に経路と認証を頼む(§4)。

**共同研究者:**

- 60 kcps はイベントレートか、チャンネルトリガーの和か(和なら ×6 で二重に数えている)。
- 多重度 2.6(前回の実験の平均)は両面を含んでいるか。
- AMax の FW は一部のヒットにだけ波形を付けられるか(6 月の FW では常に付いていた)。
- 必要なサンプル数。
- 前回の実験の波形ファイル(実際の Si の圧縮率を測るため)。
