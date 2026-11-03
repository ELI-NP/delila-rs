# TODO 80 — 汎用 root_sink(全チャンネルの hits + イベントビルダーの events)

**Status: 📋 設計中 (2026-11-03) — ROOT の並列書き込みを調査・実測済み(Mac + pf-qaq)、方式 = TTree の 2 ファイル。残 = 遅着ヒットの扱い、Stop 直前の端、追いつけないときの決まり、実装**

**関連:** [79_root_sink_nwall_online.md](79_root_sink_nwall_online.md)(実験ごとの sink) /
[71_generic_event_building.md](71_generic_event_building.md)(イベント定義) /
[70_root_sink_time_offsets.md](70_root_sink_time_offsets.md)(時刻オフセット) /
[69_write_failure_autostop.md](69_write_failure_autostop.md)(書き込み失敗) /
[68_backlog_watermark.md](68_backlog_watermark.md)(バックログ水位) /
[event-builder/66_cpp_event_builder.md](event-builder/66_cpp_event_builder.md)(今の 5 スレッド構成) /
**調査報告 [docs/root_parallel_write_research.md](../docs/root_parallel_write_research.md)** /
ベンチマーク [tools/root_sink/bench/](../tools/root_sink/bench/)

---

## 1. 発端 (2026-11-02, pf-qaq)

実験名を `N_Wall` に変えたあとも `[network.root_sink]` が ThGEM のまま `module = 0` だった。つながっているのは
VX2730(source 1)だけなので、root_sink はヒットを**全部「対象外のボード」として捨て**、ラン 2・3 の `.root` は
約 16 KB の空ファイルになった。**警告は出ていない**(状態行の `other_module=` が増えるだけ)。`.delila` は無事。
ユーザーが `module = 1` に直して解決。

原因は今の作り: 受信 → `--module` で絞る → 時刻補正 → 全ボードを 1 本の時刻順に並べる(1 s 以上遅れたヒットは
**全 tree から**落とす)→ イベントビルダー → 1 本の書き込みスレッドが hits tree と events tree を同じファイルに。
hits tree はこの流れの副産物なので、絞り込み・補正・遅着落ちの影響をそのまま受ける。

## 2. 決定事項(ユーザー、2026-11-03)

| 項目 | 決定 |
|---|---|
| hits tree | **全ボード・全チャンネル、落とさない**。設定がずれていても中身が必ずある |
| events tree | イベントビルダーが作ったイベント |
| ファイル | 並列で速度が出るなら 1 ファイル、出ないなら 2 ファイル → **調査の結果 2 ファイル**(§4) |
| hits の時刻 | **生のまま(補正しない)**。補正表を間違えても壊れるのは作り直せる events だけ、という理由で決定。補正は解析時 |
| events の時刻 | **補正後**(「すでに変形されたデータ」)。**使った補正表をファイルに同梱**。ブランチ名でどちらの時刻か分かるようにする(例: `timestamp_ns` / `timestamp_ns_aligned`)。同梱の表をかける小さなヘルパーを配る |
| ソート | hits tree は**可能ならソート** |
| 目標レート | **合計 1 MHz**(今後の実験も考えて) |
| 現実の上限 | **ディスク(HDD)**。ユーザーの経験では約 3 MHz で HDD への書き込みが張り付いた。SSD に書いても長い実験では結局 HDD に移すので、どこかで頭打ちになる。複数 HDD の RAID は時間もお金もない → **HDD の限界に合わせる** |

## 3. 形(案)

受信の直後で 2 本に分ける。

- **hits 経路**: 絞らない・補正しない・落とさない → `run####_N_<exp>_hits.root`(仮)。実質「その場で動く delila2root」で、
  ラン後の変換が要らなくなる。**hits tree の件数 = `.delila` の件数**を毎ラン確かめられる(ずれたらバグ)。
- **イベントビルダー経路**: 同期しているボードだけ(`--module` の意味は「記録するボード」から
  「イベントビルダーに入れるボード」に変わる)→ 時刻補正 → 並べ替え → イベントビルダー → `_events.root`。
  遅着で落ちたヒットも hits tree と `.delila` には残るので、「損失」ではなく「イベントにできなかった」になる(数えて出す)。
- 実験専用の表示(ThGEM の XY、N_Wall の TOF)は持たない。JSON で定義する汎用ヒストグラムは持ってよい。
  2 本に分ける作りは、あとから root_sink / nwall_sink にも入れられる。

## 4. ROOT の並列書き込み(調査の要約、詳細は調査報告)

- **TFile に並列書き込みは無い**(6.40 でも)。6.38 の `ROOT::Experimental::RFile` は Put/Get だけ。
  1 つの TFile の 2 つの tree を 2 スレッドから同時に書く公式の方法も無い。
- **TTree + `EnableImplicitMT`**: `Fill()` は 1 スレッドのまま、並列になるのはブランチごとの圧縮だけ。
  5 ブランチの hits tree では**ほぼ効かない**(最大 +15 %)。
- **`TBufferMerger`**: TTree を本当に並列で書ける唯一の公式の仕組み(8 スレッドでほぼ線形)。順序は自前で
  そろえれば保てる。ただし**途中保存(AutoSave)ができず、異常終了でラン全体を失う**。
- **RNTuple**: 本格的に並列で書ける(`RNTupleParallelWriter` は 6.38 で正式 API)。ただし
  `TTree::Draw` / `SetBranchAddress` / `TChain` が使えない(RDataFrame 一本化、開発者が明言)、
  **異常終了するとファイルが丸ごと読めなかった**(実測)、6.36(side3)で 6.38 以降のファイルを読める保証が無い。
- **ソートしても圧縮率・速度は変わらない** → ソートは解析の都合だけで決めてよい。
- **結論: TTree、`_hits.root` と `_events.root` の 2 ファイル、それぞれ書き込みスレッド 1 本、ZSTD-5、IMT なし。**
  1 MHz を大きく超える実験が来たら TBufferMerger か RNTuple を選べる余地だけ残す。

## 5. 実測

### 5.1 合成データ(`tools/root_sink/bench/run_suite.sh`)

2026-11-03。合成ヒット(ただの乱数、1 MHz 相当、10 ボード × 15 ch、5 ブランチ = 生 14 B/hit)を
メモリ上に作ってから書く = ROOT の直列化・圧縮・書き込みだけの上限値。ファイルはページキャッシュに収まるので
**ディスクの速さは測っていない**。生の CSV = `tools/root_sink/bench/results_*.csv`。

| # | 書き方 | **pf-qaq** Xeon w7-3555, ROOT 6.38.06, 50 M hits | コア | Mac M4 Pro, ROOT 6.40.04, 30 M hits |
|---|---|---|---|---|
| A1 | TTree、1 スレッド、ZSTD-5 | **3.21 M hits/s**(8.0 B/hit) | 1.0 | 4.95 |
| A2 | TTree、1 スレッド、ZSTD-1 | 9.33(8.7 B/hit) | 1.0 | 13.7 |
| A6 | TTree、1 スレッド、圧縮なし | 12.2 | 0.9 | 18.9 |
| B3 | TTree + `EnableImplicitMT(16)` | 3.12 ← **効かない** | 1.2 | 4.66(8) |
| C5 | TBufferMerger 16 スレッド、順序そろえ | 23.8 | 10.0 | 24.2(8) |
| D1 | RNTuple、1 スレッド、ZSTD-5 | **8.82**(7.2 B/hit) | 1.0 | 12.6 |
| D3 | RNTuple + IMT 4 | 25.8 | 3.7 | 44.9 |
| D4 | RNTuple + IMT 16 | 29.0 | 5.4 | 48.2(8) |
| E5 | RNTupleParallelWriter 16、順序そろえ | **63.0** | 11.9 | 78.3(8) |
| G1 | events(可変長配列)TTree | 2.45(11.9 B/hit) | 1.0 | 3.21 |
| G3 | events RNTuple(`std::vector`) | 5.11(7.8 B/hit) | 1.0 | 7.41 |
| H1 | 1 ファイルに hits と events を 1 スレッドで | 1.42 | 1.0 | 1.92 |
| H3 | **2 ファイル・2 スレッド** | **2.48**(1.75 倍) | 1.7 | 3.15 |

- Xeon の 1 コアは Mac の 0.57〜0.80 倍(TTree の A1 で 0.65)。
- **採用案(H3)で、hits と events を両方書いて 2.48 M hits/s = 1 MHz 目標の約 2.5 倍。** 遅い方は events(G1)。
- RNTuple は 1 スレッドで TTree の 2.7 倍、並列で 20 倍まで伸びるが、§4 の理由(解析の互換・異常終了)で今は採らない。
  **どちらにしても 1 MHz ではディスクが先に限界に来る**(§6)。
- ソート済み(A1/D1)とボード順(F1/F2)で、圧縮率・速度とも差なし。

### 5.2 実データ: pf-qaq ラン 4(2026-11-02〜03、今の root_sink)

`10MBq Co60 source for time alignment`、VX2730(PSD2、液シン)1 台、49,875 s(13.9 h)。
root_sink は 10-07 ビルド(書き込みスレッド 1 本、hits tree のみ実質稼働 — イベント定義のトリガーが module 0 を指していて events は 0)。

| 項目 | 値 |
|---|---|
| ヒット数(Operator の総数) | 16,513,200,555 |
| root_sink が書いたヒット数 | **16,513,200,555(完全一致)** |
| 平均レート | 331 kHz(ラン中ずっと 330〜331 kHz) |
| 遅着 `late` / 表示の取りこぼし | 0 / 0 |
| 内部の待ち行列(sort / work / write) | 最後まで 0〜6 |
| `.delila` | 439.7 GB(410 ファイル)= **26.6 B/hit** |
| hits tree の `.root`(5 ブランチ、ZSTD-5) | 111.1 GB(1 ファイル)= **6.7 B/hit** |

→ 今の構成で 1 MHz 目標の 1/3 を 14 時間、損失・滞留なしで書けた。実データの hits tree は合成データ(約 8 B/hit)より少し小さい。

参考: 同じ N_Wall のラン 3(67.0 M hits)を `delila2root`(35 ブランチ、波形のブランチ込み)で変換すると
4.26 GB = **63.6 B/hit で、`.delila`(26.0 B/hit)の 2.4 倍**。オフライン変換は元より重い。

## 6. ディスク予算(1 ヒットあたりのバイト数から)

実データの値(§5.2)= `.delila` 26.6 B/hit、hits tree 6.7 B/hit。events は合成データで「イベントに入ったヒット 1 個あたり」
約 12 B(TTree)。イベントに入るヒットの割合を f とすると、**1 ヒットあたり合計 ≈ 33 + 12 f バイト**。

| 合計レート | 書き込み量 | 1 日 | 15 時間のラン |
|---|---|---|---|
| 331 kHz(ラン 4 の実績、events なし) | 11 MB/s | 0.95 TB | 0.6 TB |
| **1 MHz** | 33〜45 MB/s | 2.9〜3.9 TB | 1.8〜2.4 TB |
| 3 MHz | 100〜135 MB/s | 8.6〜11.7 TB | 5.4〜7.3 TB |

- 【判断】HDD 1 台の連続書き込みは条件次第で百数十〜二百数十 MB/s。記録・root_sink の 2 ファイル・`/archive` への同期が
  同時に走ることを考えると、**「3 MHz あたりで張り付く」というユーザーの経験と合う**。1 MHz ならその 1/3 程度。
- **全体の 8 割は `.delila`**(26.6 B/hit、hits tree の 4 倍)。ディスクが本当に限界になったときに一番効くのは
  `.delila` を小さくすること(例: ブロック単位の圧縮)で、root_sink の工夫ではない(案として記録、未決定)。
- pf-qaq: `/data` = NVMe 3.5 TB(ラン 4 の後で空き 2.7 TB)、`/archive` = HDD 4 台の ZFS raidz2。
  1 MHz の長い実験なら `/data` は 1〜2 日分で、`archive_sync.sh`(TODO 69)が HDD に移す速さが実質の上限になる。

## 7. 残っている設計

1. **遅着ヒット**: 「ソートする」と「落とさない」の両立 = 大きく遅れたヒットを捨てずに、順番が乱れた位置のまま書いて
   数える(hits 経路)。イベントビルダー経路は今どおり捨てて数える(損失ではない)。
2. **Stop 直前の端**: hits の件数を `.delila` と一致させるため、Stop 直後のテールを recorder と同じように扱う
   (CLAUDE.md の Stop テール例外)。今どうなっているかを実装前に確認。
3. **追いつけないとき**: 落とさない以上、遅れると root_sink のメモリが増える。`.delila` には影響しない作りだが、
   「遅れが続いたら大きく警告、最後は root_sink 自身が止まる」決まりが要る(TODO 68 の水位と同じ考え方)。
4. **events tree の形**: 可変長配列(leaflist)は hits より 35 % 遅く 43 % 大きい(合成データ)。`std::vector` との比較。
5. **ファイル名・ブランチ名**、補正表を同梱する場所(UserInfo + `_timeSettings.json` の今の方式を引き継ぐか)。
6. 設定のずれを黙らせない: root_sink の `module` が `[[network.sources]]` に無ければ `start_daq.sh` が警告、
   ラン終了時に「書いたヒット 0・捨てたヒット N」なら WARN(今の root_sink / nwall_sink にも入れる)。

## 8. 次の作業

- [ ] §7 の 1〜3 をユーザーと決める
- [ ] 実装(`tools/root_sink/generic/` を想定。`common/` の Sorter・イベントビルダー・時刻オフセットを再利用)
- [ ] 実データで検証: hits 件数 = `.delila` 件数、events = 今の root_sink の events と一致(`check_events`)
- [ ] 書き込みスレッドの CPU 使用率を実ランで測る(`top -H`)
