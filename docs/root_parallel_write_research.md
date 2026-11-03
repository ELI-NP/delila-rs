# ROOT 6.36–6.40 の並列書き込み能力 調査報告 — delila-rs 次期 root_sink 設計向け

関連: [TODO 80](../TODO/80_generic_root_sink.md)(汎用 root_sink の設計)。調査・Mac での実測 = Fable(サブエージェント)、DAQ ホスト pf-qaq での実測 = §7。

- 作成日: 2026-11-03
- 調査環境: macOS (Apple M4 Pro, 10P+4E = 14 cores), ROOT 6.40.04 (/opt/ROOT, features: imt root7)。実機・ラボホストには一切接続していない。
- ベンチマーク一式: [`tools/root_sink/bench/`](../tools/root_sink/bench/) (`bench.cxx`, `hits_gen.hpp`, `run_suite.sh`, `crash_test.cxx`, `verify_read.C`, `README.md`)。生の結果 CSV = `results_m4pro_root6.40.04.csv`(本文 §3)、`results_xeon_w7-3555_root6.38.06.csv`(§7)
- 参照した ROOT ソース: GitHub root-project/root の 6.40 / 6.36 patches ブランチ(本文の行番号はそのファイル)
- タグ: 【資料】= 一次資料(リファレンス/リリースノート/ソース/開発者フォーラム投稿)で確認した事実、【実測】= 本調査で測定した値、【判断】= 私の解釈・推奨

---

## 0. 直接回答(要約)

**Q. 「最近の ROOT は TFile で並列書き込みできる」は本当か? TTree にも効くのか、RNTuple だけか?**

- 【資料】**TFile 自体に並列書き込み機能は存在しない。** TFile/TTree は "conditionally thread safe"(同一インスタンスを複数スレッドで共有しない限り安全)で、同一 TFile へ複数スレッドが同時に書く仕組みは 6.40 でもない。6.38 で入った `ROOT::Experimental::RFile` は「Put/Get だけの最小 API」で並列性とは無関係。TFile の async/prefetch は読み出し側の機能。
- 【資料】【実測】**TTree の書き込み並列化は `EnableImplicitMT` によるバスケット圧縮の並列化のみ**で、粒度は「ブランチ単位」。`TTree::Fill()` は呼び出しスレッドで逐次実行され、圧縮タスクは同じ `Fill()` 呼び出し内で `Wait()` される。5 ブランチの hits tree では**実測ゲインはほぼゼロ(1.03–1.24 コア、最大 +15 %)**。
- 【資料】【実測】**TTree を本当に並列化する唯一の公式機構は `ROOT::TBufferMerger`**(スレッド毎に TMemFile 上の TTree を Fill → 1 ファイルへマージ)。実測で 8 スレッド 25 M hits/s とリニアに伸びる。ただし**順序はマージ順(= 各スレッドの `Write()` 呼び出し順)**で、自前で直列化しないと時刻順は崩れる(直列化版は 0 inversions で同速度を確認)。
- 【資料】【実測】**本格的な並列書き込みは RNTuple 専用**: (a) `RNTupleWriter` + IMT = ページ圧縮の並列化(順序保持、実測 4 スレッドで 45 M hits/s)、(b) `RNTupleParallelWriter`(6.38 で stable API)= 複数 fill context が独立クラスタを追記(実測 8 スレッド 80 M hits/s、順序はコンテキスト内のみ保証、staged cluster で全体順序を制御可能)。
- 【資料】**同一 TFile 内の 2 本の TTree(または TTree + RNTuple)を 2 スレッドで同時に書く公式機構はない。** RNTupleParallelWriter::Append のリファレンスも「fill 中は下位ファイルに他からアクセスしてはならない」と明記。

**Q. 1 MHz sorted hits は達成できるか?**

- 【実測】M4 Pro 1 スレッドで TTree(ZSTD-5) **4.9 M hits/s**、RNTuple(ZSTD-5, IMT 無し) **12.6 M hits/s**。データは全てメモリ上なので「ROOT の直列化+圧縮+書き込みだけ」の上限値。
- 【判断】Xeon w7-3555 の 1 コアは M4 P-core より遅い(目安 0.5–0.7 倍)と見込んでも、**TTree 単一 Writer スレッドで 2.5–3.5 M hits/s ≒ 1 MHz に対して 2.5–3.5 倍の余裕**。events tree を**別ファイル・別スレッド**にすれば hits 側の余裕はそのまま。現行 delila2root の 485 k hits/s は「.delila デコード + 17 ブランチ(波形ブランチ含む)」を含む数字で、純粋な TTree 書き込み速度ではない。

---

## 1. 機構インベントリ【資料】

### 1.1 TFile レベル

| 機構 | 版 | 状態 | 内容 | 並列書き込み? |
|---|---|---|---|---|
| `TFile` | — | stable | "Conditionally thread safe: you can use conditionally safe objects concurrently by multiple threads as long as the threads do not share the same objects"、例として `TTree, TDirectory, TFile, TH*` が挙げられる ([ROOT manual: Multi-threading](https://root.cern/manual/multi_threading/)) | **否**(別インスタンスなら別スレッドで可 = 1 スレッド 1 ファイル) |
| `TFile::AsyncOpen` / `TFilePrefetch` / `TTreeCache` | 5.x〜 | stable | 読み出し側のプリフェッチ | 読み出し専用 → 対象外 |
| `TParallelMergingFile` + `parallelMergeServer` | 5.34 | stable (古い) | 複数**プロセス**がソケット経由でサーバに送り、サーバがマージ。TBufferMerger の前身 ([TBufferMerger doc](https://root.cern/doc/v640/classROOT_1_1TBufferMerger.html) "Its purpose is similar to TParallelMergingFile") | プロセス並列。DAQ 内 1 プロセスには不向き |
| `ROOT::TBufferMerger` | 6.10 (Experimental) → `ROOT` 名前空間 | stable。6.32 で内部キュー廃止 | §1.3 | **TTree の唯一のスレッド並列手段** |
| `ROOT::Experimental::RFile` | 6.38 | Experimental | "a minimal and modern interface to ROOT files with explicit ownership of objects"([6.38 release notes §RFile](https://github.com/root-project/root/blob/v6-38-00-patches/README/ReleaseNotes/v638/index.md))。リファレンス([RFile v640](https://root.cern/doc/v640/classROOT_1_1Experimental_1_1RFile.html))は "is not a 1:1 replacement" for TFile、Put/Get のみ。スレッド安全性・並列性の記述なし | **否** |

【判断】「TFile に並列性が入った」という話の出所は、(i) 6.38 リリースノートの RFile、(ii) TBufferMerger("writing data in parallel from multiple threads, while writing to a single output file")、(iii) RNTupleParallelWriter のどれかの混同と思われる。TFile そのものの書き込みモデルは変わっていない。

### 1.2 TTree + `ROOT::EnableImplicitMT()`

ROOT manual は IMT が効く操作として "TTree::Fill(): to fill and compress the branches of a tree, possibly flushing their content to disk" を挙げる。実装(`tree/tree/src/TTree.cxx`, v6-40-00-patches)は次の通り:

- `TTree::Fill()`(L4656–4721): ブランチを**逐次ループ**して `branch->FillImpl(useIMT ? &imtHelper : nullptr)`。バスケットが満杯になったブランチだけ `TBranch::WriteBasketImpl` が `imtHelper->Run(doUpdates)`(= 圧縮+書き込みを TBB タスク化、`TBranch.cxx` L3288)。ループ後に **`imtHelper.Wait()`** — つまり並列区間は**1 回の `Fill()` 呼び出しの内側**に閉じる。
- `TTree::FlushBasketsImpl()`(L5163 コメント "this will launch multiple TBB tasks in parallel via TThreadExecutor to do this operation; one per basket compression"、実装 L5212–5257): `ROOT::TThreadExecutor pool; pool.Foreach(mapFunction, nb)` — **ブランチ 1 本につき 1 タスク**。
- 条件: `ROOT::IsImplicitMTEnabled() && fIMTEnabled`。`fIMTEnabled` は TTree 構築時に `ROOT::IsImplicitMTEnabled()` からコピーされる(L825/906)→ **`EnableImplicitMT()` は TTree 生成前に呼ぶ**。`TTree::SetImplicitMT(bool)` で個別に切れる。

【判断】したがって並列度の上限は「同時に flush されるブランチ数」= hits tree なら 5、しかも `timestamp_ns/D` が 8/14 バイト = 57 % を占めるので理論上限は約 1.75 倍、実測は 1.03–1.24 倍(§3)。`Fill()` 自体(ユーザデータ → バスケットバッファへの直列化)は常に呼び出しスレッド 1 本。

### 1.3 `ROOT::TBufferMerger`

- 仕組み(`io/io/src/TBufferMerger.cxx`, `TBufferMergerFile.cxx` 6.40): `GetFile()` がスレッド毎の `TBufferMergerFile`(TMemFile 派生)を返す。ワーカーが `file->Write()` を呼ぶと `TMemFile::Write(name, opt | kOnlyPrepStep)`(**圧縮はワーカースレッドで**、コメント "Make sure the compression of the basket is done in the unlocked thread")→ `fMerger.Merge(this)` = `std::lock_guard(fMergeMutex)` 下で `TFileMerger::PartialMerge(kAll|kIncremental|kDelayWrite|kKeepCompression)` → `ResetAfterMerge`。
- 6.32 で**内部キューと専用マージスレッドは廃止**(`GetQueueSize/GetBuffered/GetAutoSave` は `_R__DEPRECATED_LATER("The queuing mechanism in TBufferMerger was removed in ROOT v6.32")`)。つまり**マージはワーカースレッド上で、mutex で直列化**されて実行される。
- 出力 TTree のキー(ヘッダ)は **デストラクタ**で `out->Write("",TObject::kOverwrite)` される(コメント "Merge does not write the target objects ... we need to write them now")。出力 TFile へのアクセサはなく、**途中で AutoSave 相当を行う公式手段がない**。
- マージは再圧縮しない: `TFileMerger.cxx` L894 `if (fFastMethod && ((type&kKeepCompression) || !fCompressionChange)) info.fOptions.Append(" fast");` → `kKeepCompression` 付きなので自動的に "fast"(バスケットのバイトコピー)。`SetMergeOptions("fast")` を明示しても同じ(§3 C6 = C2)。
- 順序保証の記述は**なし**(リファレンスにも、ソースにも)。構造上、出力順 = `Merge()` の実行順 = 各スレッドの `Write()` 到着順。
- 注意事項: "After the creation of this file, the user must reset the kMustCleanup bit on any objects attached to it and take care of their deletion"(リファレンス)。`TTree::ChangeFile`(MaxTreeSize 超過)との組み合わせは [issue #6523](https://github.com/root-project/root/issues/6523)(6.24 で修正、開発者 amadio は "disable file switching by setting the maximum TTree size very high" を推奨)。
- 異なる 2 本の TTree を運べるか: 各 TBufferMergerFile に複数 TTree を置けば TFileMerger が名前毎にマージするので**形式上は可能**だが、RDataFrame Snapshot MT(唯一の大規模ユーザ)は 1 本のみで使っている。【判断】未踏の使い方になるので推奨しない。
- 開発者の見解(root-forum [Thread-safe TTree::Fill](https://root-forum.cern.ch/t/thread-safe-ttree-fill/40113), eguiraud 2020-06-28): "there is no chance to get a thread-safe TTree::Fill I'm afraid, the internals are just not designed for it"、"RDataFrame's Snapshot uses TBufferMerger to write thread-local TTrees to the same on-disk TTree concurrently from multiple threads"、"if TBufferMerger was not ready for production, RDataFrame wouldn't be either"。

### 1.4 RNTuple

| 項目 | 版 | 根拠 |
|---|---|---|
| オンディスク形式 1.0 確定 | **6.34** | 6.34 release notes: "The first version of the `RNTuple` on-disk binary format is finalized. Future versions of ROOT will be able to read back RNTuple data written as of this release. ... this version breaks compatibility with experimental RNTuple data written with releases up to v6.34. Please also note that the RNTuple API is not yet moving out of `ROOT::Experimental`." |
| コア API が `ROOT::` へ(Experimental 卒業) | **6.36** | 6.36 release notes §RNTuple: `RNTupleReader`, `RNTupleWriter`, `RNTupleModel` ほか多数が `ROOT::Experimental` → `ROOT`。`RNTupleProcessor` は Experimental のまま |
| `RNTupleParallelWriter` 初出 | 6.32 | 6.32 release notes: "A new `RNTupleParallelWriter` class has been added, providing (initial) support for parallel writing of RNTuples." |
| staged cluster(順序制御) | 6.34 | 6.34 notes: "Support for cluster staging in the `RNTupleParallelWriter`. Cluster staging enables users to enforce a certain ordering" |
| `RNTupleParallelWriter` / `RNTupleFillContext` が stable API | **6.38** | 6.38 notes: "The parallel writer is now part of the public, stable API. The `RNTupleParallelWriter` and the closely related `RNTupleFillContext` moved from the `ROOT::Experimental` to the `ROOT` namespace." 同じ版で "[ntuple] Allow usage of IMT with `RNTupleParallelWriter`"(#18398) |
| 6.40 | 6.40 | `RRawPtrWriteEntry` が stable (`ROOT::Detail`)、SoA / attributes は Experimental。並列書き込みの仕様変更なし |
| バイナリ仕様版 | 6.36 = 1.0.0.3 / 6.38 = 1.0.1.1 / 6.40 = 1.0.2.1 | `tree/ntuple/doc/BinaryFormatSpecification.md` 各ブランチ先頭行 |

**(a) `RNTupleWriter` + IMT(順序保持の並列圧縮)**
- `RNTupleWriter.cxx` L43–47: `IsImplicitMTEnabled() && options.GetUseImplicitMT() != kOff` なら `RNTupleImtTaskScheduler`(= `ROOT::TTaskGroup` ラッパ)を `RPageSinkBuf` に設定。`RPageSinkBuf.cxx` L186–226: ページ単位の seal(圧縮)を `fTaskScheduler->AddTask(...)` でタスク化。バッファ済み未圧縮バイトが `ApproxZippedClusterSize` を超えたら呼び出しスレッドで直接圧縮(メモリ抑制ヒューリスティック)。
- `RNTupleWriteOptions.hxx` L147–155: "For the (sequential) RNTupleWriter, [EImplicitMT::kDefault] translates to EImplicitMT::kOn" — **`EnableImplicitMT()` を呼ぶだけで自動的に有効**。`SetUseImplicitMT(kOff)` で切る。
- 既定値(同ヘッダ L197–208): compression = `kUseGeneralPurpose`(= ZSTD-5 = 505)、ApproxZippedClusterSize 128 MiB、MaxUnzippedPageSize 1 MiB、buffered write ON、page checksums ON。
- 順序: fill は単一スレッド、ページ/クラスタのコミットは fill 順 → **エントリ順は保持される**(§3 実測 inversions = 0)。

**(b) `RNTupleParallelWriter`**
- リファレンス([v640](https://root.cern/doc/v640/classROOT_1_1RNTupleParallelWriter.html)): "Each fill context prepares independent clusters that are appended to the common RNTuple with internal synchronization." / 順序: "Entries from one context are totally ordered as they were filled" が、コンテキスト間の順序はなく、"two consecutive entries in one fill context can end up separated in the final RNTuple"。
- ソース(`RNTupleParallelWriter.cxx` L212–230): `CreateFillContext()` 毎に `RPageSinkBuf(RPageSynchronizingSink(*fSink, fSinkMutex))` — 共通シンクへのコミットだけ mutex、圧縮はコンテキスト側。IMT はオプション `kOn` を明示したときだけ(`RNTupleWriteOptions.hxx` L153 "For the RNTupleParalellWriter, the implementation defaults to EImplicitMT::kOff in order to avoid interference")。
- `Recreate`/`Append` は `GetUseBufferedWrite()` 必須("parallel writing requires buffering")。`Append(TDirectory&)` の注意: "The caller must ensure that the underlying file is not be accessed while data is filled into any created context." "there is no synchronization with other writers or clients writing to the same file"。
- staged cluster: `RNTupleFillContext::EnableStagedClusterCommitting()` + `FlushCluster()`(圧縮・ページ書き込みまで) + `CommitStagedClusters()`(論理コミットのみ)。コミット順 = ファイル内順なので、**コミットだけをブロック順に直列化すれば全体の時刻順を保てる**(§3 E4/E5 で 0 inversions を確認)。
- 制約: "the parallel writer does not (yet) support incremental updates of the underlying model"(late model extension 不可)。

**(c) 互換性ルール**(`BinaryFormatSpecification.md` §Versioning Notes / §Feature Flags)
- "Except for the epoch, the versioning is for reporting only. Readers should use the feature flag in the header to determine whether they support reading the file." / "Readers should gracefully abort reading when they encounter unknown bits set."
- 6.36 時点で feature flag は未定義。6.40 (1.0.2.1) で初めて bit 0 "Nested Deferred Columns" が定義されたが、これは**異種エンコーディングのコレクションをマージした場合にのみ**立つ(6.40 ソース中で `SetFeature()` を呼ぶ箇所は `RNTupleDescriptorBuilder::SetFeature` の定義のみで、通常の writer 経路には呼び出しがない)。6.36 の reader(`RNTupleSerialize.cxx` `CheckFeatureFlags`)は未知ビットがあれば "unsupported format feature" で失敗し、anchor の Minor/Patch 版は検査しない。
- 【判断】我々のようなスカラ/`std::vector` だけの RNTuple を 6.38/6.40 で書けば、feature flag は 0 のままで **6.36.08 でも読める見込みが高い**。ただし一次資料に「6.40 で書いた 1.0.2.1 を 6.36 が読める」と明記した文は見つからなかったので、§5.4 の実機確認項目に入れる。

### 1.5 同一 TFile に 2 本(TTree×2、TTree+RNTuple)を 2 スレッドで同時書き込み

- 【資料】TFile は conditionally thread safe(§1.1)。`RNTupleWriter::Append` は "the TDirectory object must not be moved, destroyed or replaced during that time"、`RNTupleParallelWriter::Append` は上記の通り「fill 中にファイルへ他からアクセス禁止」。TBufferMerger の出力 TFile も非公開。
- 【判断】**公式にサポートされた手段はない。** 「1 ファイルに 2 本」を選ぶなら、1 スレッドが両方を順番に Fill する(§3 H1/H2)か、TBufferMerger の各 memfile に 2 本置く未踏の使い方しかない。「2 ファイル・2 スレッド」なら各ファイルが独立インスタンスなので完全にサポート範囲内。

---

## 2. 順序保持(時刻順 hits tree の要件)

| 機構 | 単一ストリームの順序 | 備考 |
|---|---|---|
| TTree 単一 Writer(IMT 有無とも) | **保持** | Fill 順 = エントリ順 |
| TBufferMerger | **非保持**(デフォルト) | 【実測】4–8 スレッドで 15–20 inversions(ブロック境界ごと)。ブロック単位でワーカーの `Write()` をチケットで直列化すると 0 inversions、速度低下なし(C4/C5/C7) |
| RNTupleWriter(+IMT) | **保持** | 【実測】inversions 0 |
| RNTupleParallelWriter | **非保持**(デフォルト) | 【実測】15–18 inversions。staged cluster + 順序コミットで 0(E4–E7) |
| RDataFrame Snapshot(MT) | 非保持 | `kEntriesReshuffled` ビットが立つ(RDFSnapshotHelpers.cxx) |

【判断】「並列 + 時刻順」は TBufferMerger でも RNTupleParallelWriter でも**ユーザ側のチケット直列化**で実現できる(どちらも測定済み)。ただし前者は「ブロック境界 = flush 境界」になるため、ブロックを小さくしすぎるとバスケットが小さくなる(C7: 250 k hits/ブロックでも速度・サイズに影響なし)。

---

## 3. ベンチマーク【実測】

条件: M4 Pro、ROOT 6.40.04、`-O2`、APFS 内蔵 SSD。合成 hits 30 M 件(1 MHz 相当 Poisson、10 module × 15 ch、不均一レート、ピーク+連続のエネルギー、2 ns + 10 bit fine timestamp)。データは生成後メモリ常駐、計測区間はファイル open → close。`cores` = CPU 時間 / 壁時計。`ratio` = 生データ(14 B/hit)/ファイルサイズ。`inv` = 読み戻した timestamp の隣接逆転数(-1 = 未確認)。生 CSV: `tools/root_sink/bench/results_m4pro_root6.40.04.csv`。

### 3.1 TTree 1 スレッド(hits tree, 5 branches)

| # | 設定 | wall s | cores | **M hits/s** | file MB | MB/s | ratio | inv |
|---|---|---|---|---|---|---|---|---|
| A1 | ZSTD-5 (505) 既定 basket/autoflush | 6.07 | 1.00 | **4.95** | 245 | 40 | 1.71 | — |
| A2 | ZSTD-1 (501) | 2.19 | 0.99 | **13.7** | 262 | 119 | 1.61 | — |
| A3 | ZSTD-9 (509) | 23.2 | 1.00 | 1.29 | 232 | 10 | 1.81 | — |
| A4 | LZ4-4 (404) | 6.25 | 1.00 | 4.80 | 338 | 54 | 1.24 | — |
| A5 | ZLIB-1 (101, TTree 既定) | 5.69 | 1.00 | 5.27 | 274 | 48 | 1.53 | — |
| A6 | 無圧縮 | 1.59 | 0.98 | 18.9 | 421 | 265 | 1.00 | — |
| A7 | 505, basket 256 kB | 6.57 | 1.00 | 4.57 | 246 | 37 | 1.71 | — |
| A8 | 505, basket 1 MB | 6.20 | 1.00 | 4.84 | 245 | 40 | 1.71 | — |
| A9 | 505, AutoFlush −100 MB | 6.91 | 1.00 | 4.34 | 244 | 35 | 1.72 | — |
| A10 | 505, AutoFlush 1 M entries | 6.10 | 0.99 | 4.92 | 245 | 40 | 1.71 | — |

- 無圧縮でも 18.9 M/s → `Fill()` の直列化コスト自体が 1 hit ≈ 53 ns。ZSTD-5 の圧縮が 1 hit ≈ 150 ns。
- LZ4 は ZSTD-5 より速くなく、ratio は大幅に悪い。ZSTD-1 は 2.8 倍速く、ファイルは +7 %。
- basket/AutoFlush は速度に効かない(圧縮支配)。

### 3.2 TTree + `EnableImplicitMT`

| # | 設定 | wall s | cores | M hits/s | 対 A1 |
|---|---|---|---|---|---|
| B1 | 505, IMT 2 | 6.30 | 1.03 | 4.76 | 0.96× |
| B2 | 505, IMT 4 | 6.37 | 1.08 | 4.71 | 0.95× |
| B3 | 505, IMT 8 | 6.43 | 1.17 | 4.66 | 0.94× |
| B4 | 505, IMT 4 + basket 1 MB | 5.26 | 1.24 | 5.70 | 1.15× |
| B5 | 509, IMT 4 | 23.9 | 1.05 | 1.25 | 0.97× |
| F3 | 505, IMT 4, board order | 6.00 | 1.07 | 5.00 | — |

**IMT は TTree 書き込みをほぼ速くしない**(§1.2 の構造通り)。delila2root が `EnableImplicitMT()` を呼んでいるのも効果は僅少と考えられる(そちらは 17 ブランチなので多少マシな可能性はある)。

### 3.3 `TBufferMerger`(N スレッド → 1 ファイル 1 TTree)

| # | 設定 | wall s | cores | **M hits/s** | ratio | inv |
|---|---|---|---|---|---|---|
| C1 | 505, 2 thr, 1 M/block | 4.14 | 1.98 | 7.25 | 1.73 | 15 |
| C2 | 505, 4 thr | 2.27 | 3.68 | 13.2 | 1.73 | 16 |
| C3 | 505, 8 thr | 1.20 | 7.08 | **25.1** | 1.73 | 15 |
| C4 | 505, 4 thr, **ordered** | 2.29 | 3.67 | 13.1 | 1.73 | **0** |
| C5 | 505, 8 thr, ordered | 1.24 | 6.88 | 24.2 | 1.73 | 0 |
| C6 | 505, 4 thr, `SetMergeOptions("fast")` | 2.29 | 3.68 | 13.1 | 1.73 | 20 |
| C7 | 505, 4 thr, 250 k/block, ordered | 2.16 | 3.87 | 13.9 | 1.73 | 0 |
| C8 | 509, 8 thr, ordered | 1.75 | 7.00 | 17.2 | 1.80 | 0 |

ほぼリニアにスケール、マージ(mutex 区間)はボトルネックになっていない(8 スレッドでも 7.1 コア有効)。順序直列化のコストはゼロ。ファイルサイズは単一 TTree と同等。

### 3.4 RNTuple 単一 writer

| # | 設定 | wall s | cores | **M hits/s** | file MB | ratio | inv |
|---|---|---|---|---|---|---|---|
| D1 | 505, IMT off | 2.37 | 1.00 | **12.6** | 216 | 1.95 | 0 |
| D2 | 505, IMT 2 | 1.23 | 1.96 | 24.3 | 216 | 1.95 | — |
| D3 | 505, IMT 4 | 0.67 | 3.76 | **44.9** | 216 | 1.95 | 0 |
| D4 | 505, IMT 8 | 0.62 | 4.25 | 48.2 | 216 | 1.95 | — |
| D5 | 無圧縮 | 0.71 | 0.90 | 42.0 | 420 | 1.00 | — |
| D6 | LZ4-4 | 4.97 | 1.00 | 6.03 | 252 | 1.67 | — |
| D7 | ZLIB-1 | 4.11 | 1.00 | 7.29 | 225 | 1.86 | — |
| D8 | ZSTD-9 | 16.7 | 1.00 | 1.80 | 211 | 1.99 | — |
| D9 | ZSTD-9, IMT 4 | 5.98 | 3.95 | 5.01 | 211 | 1.99 | — |
| D10 | 505, cluster 32 MiB | 2.30 | 1.00 | 13.1 | 216 | 1.95 | — |
| D11 | 505, buffered write off | 2.31 | 1.00 | 13.0 | 216 | 1.95 | — |

- 1 スレッドでも TTree の **2.5 倍**(直列化が軽い: 無圧縮 42 M/s vs TTree 18.9 M/s)、ファイルは **12 % 小さい**(列指向 + split encoding)。
- IMT は 4 スレッドまでほぼリニア、8 では頭打ち(fill スレッド 1 本の直列化が律速)。順序は保持。

### 3.5 `RNTupleParallelWriter`

| # | 設定 | wall s | cores | **M hits/s** | inv |
|---|---|---|---|---|---|
| E1 | 505, 2 ctx | 1.17 | 1.98 | 25.6 | 15 |
| E2 | 505, 4 ctx | 0.66 | 3.74 | 45.4 | 15 |
| E3 | 505, 8 ctx | 0.37 | 7.32 | **80.8** | 18 |
| E4 | 505, 4 ctx, ordered (staged) | 0.68 | 3.71 | 44.1 | **0** |
| E5 | 505, 8 ctx, ordered | 0.38 | 7.11 | 78.3 | 0 |
| E6 | 505, 4 ctx + IMT 4, ordered | 0.54 | 4.81 | 56.1 | 0 |
| E7 | 509, 8 ctx, ordered | 3.81 | 7.41 | 7.88 | 0 |

### 3.6 ソート済み vs ボード順(module 毎 10 ms バッチ、モジュール間未ソート)

| # | 設定 | M hits/s | ratio | inv |
|---|---|---|---|---|
| A1 / F1 | TTree 505 sorted / board | 4.95 / 4.90 | 1.71 / 1.70 | 0 / 27000 |
| D1 / F2 | RNTuple 505 sorted / board | 12.6 / 15.5 | 1.95 / 1.96 | 0 / 27000 |
| E4 / F4 | RNTuple-par 4 ordered sorted / board | 44.1 / 52.8 | 1.94 / 1.95 | 0 / 27000 |

**圧縮率は順序にほぼ無関係**(ZSTD は double timestamp の差分構造を利用しない)。速度も同等以上。→ ソートは「解析の都合」で決めるべきで、I/O 性能の理由にはならない。

### 3.7 events tree レイアウト(1 event = 1–64 hits の可変長配列、平均 ~5、≈200 k events/s)

| # | 設定 | wall s | cores | M hits/s 相当 | k events/s | file MB | ratio |
|---|---|---|---|---|---|---|---|
| G1 | TTree 505, `HitMod[NHits]/b` 等 | 9.35 | 1.00 | 3.21 | 640 | 351 | 1.54 |
| G2 | TTree 505, IMT 4 | 8.60 | 1.15 | 3.49 | 700 | 351 | 1.54 |
| G3 | RNTuple 505, `std::vector<T>` | 4.05 | 1.00 | 7.41 | 1480 | 235 | 2.30 |
| G4 | RNTuple 505, IMT 4 | 1.21 | 3.82 | 24.8 | 4950 | 235 | 2.30 |

TTree の可変長配列は hits tree より 35 % 遅く、ファイルも 43 % 大きい(leaflist 配列はエントリ毎のオフセット/長さオーバーヘッド)。RNTuple は 1.5 倍小さい。

### 3.8 1 ファイル(両 tree を 1 スレッド)vs 2 ファイル(2 スレッド)

| # | 設定 | wall s | cores | M hits/s(hits+events 両方書いて) | file MB |
|---|---|---|---|---|---|
| H1 | ttree2: 1 ファイル、1 スレッドが hits と events を交互に Fill | 15.66 | 1.00 | 1.92 | 596 |
| H2 | 同上 + IMT 4 | 14.95 | 1.12 | 2.01 | 596 |
| H3 | twofile: hits → A、events → B を 2 スレッド同時 | **9.52** | 1.69 | **3.15** | 597 |
| H4 | 同上 + IMT 4 | 8.87 | 1.95 | 3.38 | 597 |

H1 = A1 + G1 の和(6.07 + 9.35 = 15.4 s)にほぼ等しい → 1 ファイル 1 スレッドでは **2 本分のコストが単純加算**される。2 ファイル 2 スレッド(H3)は遅い方(events, 9.35 s)の時間で終わり、**1.64 倍**速い。IMT を足しても +7 % しか変わらない。

### 3.9 クラッシュ耐性(途中で `_exit(1)`、5 M hits)

`crash_test.cxx`: 5.5 M hits を書き、ファイルを閉じずに `_exit(1)`。その後別プロセスで開く。

| 書き手 | 途中保存 | クラッシュ後のファイル | 読めたか |
|---|---|---|---|
| TTree(ZSTD-5) | `AutoSave("SaveSelf")` を 1 M entries 毎 | 46.0 MB | **読める: 5,000,000 entries**(最後の AutoSave まで、0.5 M 件損失) |
| RNTupleWriter | `CommitCluster(true)`(cluster group コミット)を 1 M 毎 | 33.6 MB(ページ/クラスタは書かれている) | **読めない**: `expected RNTuple named 'delila' but instead found ''`(anchor/footer は `CommitDataset()` = デストラクタでしか書かれない) |
| RNTupleWriter | なし | 0 B | 読めない |

【資料】RNTupleWriter リファレンス: "Model updating fails once the dataset is committed with `CommitDataset()`"、`CommitDataset()` は "Automatically called by the destructor"。途中で anchor を書き直す API(TTree の AutoSave に相当するもの)は 6.40 の公開 API に見当たらない。
【判断】**オンライン DAQ の sink としては、現時点で RNTuple は「プロセスが正常終了しない限り run 全体を失う」**。TBufferMerger も同じ性質(§1.3、出力 TTree キーはデストラクタで書く)。TTree 単一 Writer + 定期 AutoSave だけが部分回復できる。.delila を正とする前提なら許容できなくはないが、root_sink の TODO 69(書き込み失敗ラッチ + 最後の AutoSave からの回復)の設計思想とは相容れない。

### 3.10 読み戻し確認(`verify_read.C`)

2 M hits の TTree ファイルと RNTuple ファイル(どちらも bench 出力)に対して ROOT 6.40.04 で:

| 確認 | 結果 |
|---|---|
| `TTree::Draw("energy>>h","module==3","goff")` | 205,686 / 2,000,000 選択、0.06 s |
| `RDataFrame("delila", rntuple.root).Filter("module==3").Histo1D("energy")` | 同じ 205,686 件、0.44 s(JIT 込み) |
| `RDataFrame(RNTuple).Snapshot("delila","rntuple_as_ttree.root")` → `TTree::Draw` | 2,000,000 entries を 0.85 s で TTree 化、Draw 可。Snapshot は "The default Snapshot output data format is TTree, but the input data format is RNTuple ... this current default behaviour might change in the future" と警告 |
| `RNTupleReader::PrintInfo` | 5 fields(`std::uint8_t ×2, std::uint16_t ×2, double`)、2,000,000 entries |

→ RNTuple を選んでも RDataFrame 経由と TTree への変換は機能する(ただし変換は読み出し一回分のコスト、1 ファイル数百 GB では非現実的)。

---

## 4. 読み手(研究者)側の互換性【資料】

| 観点 | TTree | RNTuple |
|---|---|---|
| `TTree::Draw` / `Scan` / `SetBranchAddress` マクロ / `TChain` | そのまま | **不可**。開発者(StephanH, root-forum 2026-03-06 [How to explore RNTuples in the interpreter](https://root-forum.cern.ch/t/how-to-explore-rntuples-in-the-interpreter/64737)): "it made sense to put the reading/drawing/scanning functionality only in one place"(= RDataFrame)。別スレッド([Plotting RNTuple same as TTree](https://root-forum.cern.ch/t/plotting-rntuple-same-as-ttree/63670))でも「`TTree::Draw` を RNTuple に再実装しないことを意識的に決めた」。`RNTupleReader::Show()/PrintInfo()`、`RNTupleInspector` が代替 |
| RDataFrame | 可 | 可(6.36 でデータソースが `ROOT::RDF` に正式化、同一コードで TTree/RNTuple 両対応、"Rosetta stone" 節に `tree->Draw("x","y>2")` → `df.Filter("y > 2").Histo1D("x")` 等の対応表) |
| TBrowser | 可 | 6.38 で "the classic browser now supports RNTuple ... including a new treemap visualization of field sizes"(6.38 release announcement) |
| JSROOT / THttpServer | 可 | JSROOT **7.10.0** で RNTuple 対応、7.11 で型拡充、7.12 で draw expression 編集(`changes.md`)。ROOT 6.40 同梱の JSROOT 版は要確認 |
| hadd / マージ | 可 | 6.34 以降 `RNTupleMerger`・hadd 対応、6.36 で `rntuple.MergingMode` 等のオプション |
| uproot (Python) | 可 | **読み: 可**(5.6.x 以降、"Inspecting the contents of an RNTuple is done in the same way")、**書き: 5.7.0 から RNTuple が既定**。ただし公式ドキュメントは "There are still significant work required to achieve feature-parity with TTrees" ([uproot basic guide](https://uproot.readthedocs.io/en/latest/basic.html), [releases](https://github.com/scikit-hep/uproot5/releases) 最新 5.7.7, 2026-10-06) |
| TTree に戻す逃げ道 | — | `RDataFrame(...).Snapshot("tree","f.root")`(出力既定は TTree)。§3.10 で実測 |
| 6.36 で 6.38/6.40 の RNTuple を読む | — | §1.4(c)。仕様上は feature flag 0 なら可。**実機確認必須** |

【判断】研究者が `TTree::Draw` 一発・既存 C++ マクロ(`SetBranchAddress`)・TChain に依存している現状では、**RNTuple への全面移行は解析側の再教育コストが大きい**。RDataFrame で書き直せる人には問題ないが、「とりあえず Draw」が失われる。

---

## 5. 推奨【判断】

### 5.1 結論

1. **フォーマット: TTree を維持**(hits tree・events tree とも)。理由: (i) 1 MHz 目標に対して TTree 単一スレッドで十分な余裕(§5.2)、(ii) 解析側互換(§4)、(iii) side3 の 6.36 を含む全ホストで確実に読める、(iv) クラッシュ時の `AutoSave` による部分回復が確立(§3.9: TTree は最後の AutoSave まで読めた、RNTuple は `CommitCluster` していても anchor 未書き込みで読めなかった)。RNTuple の速度(1 スレッドで 2.5 倍、IMT で 9 倍)とサイズ(−12 %)は魅力だが、今の要求(1 MHz)では決め手にならない。
2. **ファイル構成: 2 ファイル(`_hits.root` / `_events.root`)、各 1 Writer スレッド。** 同一 TFile に 2 本を並列に書く公式機構はなく(§1.5)、1 スレッドで 2 本書くとコストが単純加算される(H1)。2 ファイルなら各 Writer が独立に AutoSave・MaxTreeSize・失敗ラッチを持てて、現行 `Recorder` クラスをほぼそのまま 2 インスタンス化できる。
3. **圧縮: ZSTD-5 (505) を既定のまま。** 余裕が足りないホストでは ZSTD-1〜3 に落とす(A2: 2.8 倍速、+7 % サイズ)。LZ4 は採用理由なし(遅くて大きい)。
4. **`EnableImplicitMT` は hits/events Writer には不要**(効果 ≤ 15 %、TBB スレッドプールと ZMQ/Sorter スレッドの干渉リスクの方が大きい)。
5. **予備(1 MHz を大きく超える将来実験向け)**: hits tree に `TBufferMerger` + チケット順序直列化(C4/C5 の方式)を使えば TTree のまま 8 スレッドで 20 M hits/s 超。ただし AutoSave 不可(§1.3)なので、クラッシュ時は run 全体を失う。もう一段上なら RNTuple(IMT 付き単一 writer で順序保持のまま 40 M/s)。この 2 つは「実験毎に選べるオプション」として設計に穴を開けておく程度に留める。

### 5.2 1 MHz sorted の達成可否

- 【実測】M4 Pro 1 コア: TTree-505 で 4.95 M hits/s(events 付きでも 3.2 M hits/s 相当)。
- 【判断】Xeon w7-3555(Sapphire Rapids 世代、~4.8 GHz boost)の 1 コア性能を M4 P-core の 0.5–0.7 倍と見込むと **2.5–3.5 M hits/s**。目標 1 MHz に対し 2.5 倍以上の余裕。Sorter/Receiver の処理と ZMQ 受信は別スレッドなので Writer の余裕には影響しない(ただしコア数は消費する)。
- ディスク: 1 MHz × 14 B ≈ 14 MB/s 生、ZSTD-5 後 ≈ 8 MB/s、15 h で ≈ 450 GB(hits のみ)。NVMe には余裕。
- **実機で確認するまで「達成可能」とは断言しない**(§5.4)。

### 5.3 リスク一覧

| リスク | 影響 | 対策 |
|---|---|---|
| Xeon の 1 コア性能が見込みより低い | 余裕が 2 倍を切る | §5.4 で実測。ZSTD-3/1 への切替で 1.5–2.8 倍稼げる |
| hits の瞬間レートが平均の数倍(バースト) | Writer 手前のキューが膨らむ | 無制限キュー(CLAUDE.md 方針)+ バックログ監視・警告。Writer 平均速度 > ピーク平均なら吸収可 |
| 将来ブランチ追加(数スカラー) | 比例して遅く(1 branch ≈ +10–20 %) | TTree のまま許容。events tree の配列ブランチは重いので最小限に |
| 1 ファイル 2 本に固執した場合 | 直列加算で余裕半減、または未踏の TBufferMerger 2-tree 運用 | 2 ファイル採用 |
| TBufferMerger 採用時の AutoSave 不可 | クラッシュで run ファイル全損 | 採用しない(または .delila を正とする前提で許容) |
| RNTuple 採用時の 6.36 互換・`TTree::Draw` 喪失・クラッシュ回復 | 解析側混乱、側3 で読めない可能性 | 採用しない。将来採用時は §5.4 の互換試験を先に |
| `EnableImplicitMT` の TBB プールと自前スレッドの干渉 | デッドロック・性能低下(TTree.cxx L5163 の警告) | Writer では IMT を使わない |

### 5.4 Linux DAQ ホスト(Xeon w7-3555, ROOT 6.38.06)で確認すること

1. `tools/root_sink/bench/run_suite.sh 50000000 ./out_xeon`(`THREADS_MAX=16`)を実行し、A1/A2/A5/G1/H1/H3 の M hits/s を本表と比較。**A1 が 2.5 M hits/s 以上**なら本推奨のまま。2 M を切るなら ZSTD-3 を既定に。
2. ROOT 6.38 では `RNTupleParallelWriter` が `ROOT::` にあるので bench はそのままビルドできるはず(6.36 では `ROOT::Experimental::` のため bench は 6.38 以上専用)。
3. 互換試験(将来 RNTuple を検討する場合のみ): 6.38 で `./bench rntuple --n 2000000 --keep --out rn638.root` → side3 の 6.36.08 で `RNTupleReader::Open` と `RDataFrame` 読み出し。
4. 実データ系: 現行 root_sink を `--hits-tree` のみで走らせ、Writer スレッドの CPU 使用率(`top -H`)を 1 MHz 相当の入力で確認。Writer が 100 % 近いなら Receiver/Sorter 側のボトルネックも疑う(本ベンチは ROOT 部分のみ)。
5. NVMe の持続書き込み(`fio` 等)が 50 MB/s 以上あること(ZSTD-1 でも 20 MB/s 程度)。

---

## 6. 引用一覧

一次資料(リリースノート・ソース): すべて `github.com/root-project/root` の各 patches ブランチ、取得日 2026-11-03
- `README/ReleaseNotes/v632/index.md`(RNTupleParallelWriter 初出)、`v634`(§RNTuple Libraries: フォーマット 1.0 確定、staged cluster)、`v636`(§RNTuple: Experimental 卒業、RDF RNTuple ソース)、`v638`(§RNTuple: parallel writer stable、§RFile、Snapshot 既定 505)、`v640`(§RNTuple: RRawPtrWriteEntry stable、SoA/attributes experimental)
- `tree/tree/src/TTree.cxx`(v6-40-00-patches)L4656–4721 `TTree::Fill`、L5155–5257 `TTree::FlushBasketsImpl`、L7217– `TTree::Merge(TCollection*, TFileMergeInfo*)`; `tree/tree/src/TBranch.cxx` L855 `FillImpl`、L3222–3290 `WriteBasketImpl`
- `io/io/src/TBufferMerger.cxx`, `io/io/src/TBufferMergerFile.cxx`, `io/io/inc/ROOT/TBufferMerger.hxx`(6.32 キュー廃止の deprecation 文言)
- `tree/ntuple/src/RNTupleWriter.cxx` L43–47、`tree/ntuple/src/RNTupleParallelWriter.cxx` L170–230、`tree/ntuple/src/RPageSinkBuf.cxx` L186–226、`tree/ntuple/inc/ROOT/RNTupleWriteOptions.hxx` L147–208、`tree/ntuple/inc/ROOT/RNTupleFillContext.hxx`(staged committing)、`tree/ntuple/doc/BinaryFormatSpecification.md`(6.36: 1.0.0.3 / 6.38: 1.0.1.1 / 6.40: 1.0.2.1)、`tree/ntuple/doc/README.md`("For plotting and scanning through entries, use ROOT::RDataFrame")
- `tree/dataframe/src/RDFSnapshotHelpers.cxx`(v6-40-00-patches)L651–700: Snapshot MT の TBufferMerger 使用、`kEntriesReshuffled`、`SetImplicitMT(false)`
- ROOT 6.36 `tree/ntuple/src/RNTupleSerialize.cxx` L1812–1846 `CheckFeatureFlags`

リファレンスガイド / マニュアル
- https://root.cern/manual/multi_threading/ (thread safety 分類、IMT と TTree::Fill)
- https://root.cern/doc/v640/classROOT_1_1TBufferMerger.html
- https://root.cern/doc/v640/classROOT_1_1RNTupleParallelWriter.html
- https://root.cern/doc/v640/classROOT_1_1RNTupleWriter.html
- https://root.cern/doc/v640/classROOT_1_1Experimental_1_1RFile.html
- https://root.cern/doc/v640/classROOT_1_1RDataFrame.html (§Rosetta stone)

開発者フォーラム / issue
- https://root-forum.cern.ch/t/thread-safe-ttree-fill/40113 (eguiraud 2020-06-28)
- https://root-forum.cern.ch/t/how-to-explore-rntuples-in-the-interpreter/64737 (StephanH 2026-03-05/06)
- https://root-forum.cern.ch/t/plotting-rntuple-same-as-ttree/63670
- https://github.com/root-project/root/issues/6523 (TBufferMerger × MaxTreeSize)

論文
- J. Hahnfeld, J. Blomer, T. Kollegger, "Parallel Writing of Nested Data in Columnar Formats", Euro-Par 2024 (arXiv:2410.14239) — RNTupleParallelWriter の設計(fill context / 独立クラスタ / 単一 writer のボトルネック除去)、"perfect scalability only limited by storage bandwidth"。TBufferMerger のキュー廃止もこの準備作業として言及

外部ツール
- JSROOT `changes.md` (https://github.com/root-project/jsroot/blob/master/changes.md): 7.10.0 "RNTuple support"
- uproot: https://uproot.readthedocs.io/en/latest/basic.html, https://github.com/scikit-hep/uproot5/releases

---

## 7. 追記: DAQ ホスト pf-qaq での実測(2026-11-03)【実測】

§5.4 の 1 を実施(メインセッション)。pf-qaq = Intel Xeon w7-3555(28 コア / 56 スレッド)、251 GB RAM、ROOT 6.38.06(imt, root7)、
出力先はシステム NVMe(記録用の `/data` とは別)。`THREADS_MAX=16 nice -n 10 ./run_suite.sh 50000000 ./out_xeon`、所要 12 分、
DAQ は Configured(ラン間)。6.38.06 で bench はそのままビルドできた(`RNTupleParallelWriter` は `ROOT::`)。
生の CSV: `tools/root_sink/bench/results_xeon_w7-3555_root6.38.06.csv`。

| # | 設定 | M hits/s | cores | B/hit | Mac 比 |
|---|---|---|---|---|---|
| A1 | TTree 1 スレッド ZSTD-5 | **3.21** | 0.99 | 8.0 | 0.65 |
| A2 | TTree ZSTD-1 | 9.33 | 0.97 | 8.7 | 0.68 |
| A6 | TTree 圧縮なし | 12.24 | 0.93 | 14.0 | 0.65 |
| B3 | TTree + IMT 16 | 3.12 | 1.18 | 8.0 | 0.67 |
| B4 | TTree + IMT 4、basket 1 MB | 3.57 | 1.31 | 8.0 | 0.63 |
| C4 / C5 | TBufferMerger 4 / 16 スレッド、順序そろえ(inv 0) | 10.9 / 23.8 | 3.6 / 10.0 | 8.0 | 0.83 / 0.98 |
| D1 | RNTuple 1 スレッド ZSTD-5 | **8.82** | 1.00 | 7.2 | 0.70 |
| D3 / D4 | RNTuple + IMT 4 / 16 | 25.8 / 29.0 | 3.7 / 5.4 | 7.2 | 0.57 / 0.60 |
| E4 / E5 | RNTupleParallelWriter 4 / 16、順序そろえ(inv 0) | 27.8 / **63.0** | 3.7 / 11.9 | 7.2 | 0.63 / 0.80 |
| F1 / F2 | ボード順 TTree / RNTuple | 3.10 / 9.18 | 1.0 | 8.1 / 7.1 | — |
| G1 / G3 | events TTree / RNTuple | 2.45 / 5.11 | 1.0 | 11.9 / 7.8 | 0.76 / 0.69 |
| H1 | 1 ファイル 1 スレッドで両 tree | 1.42 | 0.99 | — | 0.74 |
| H3 | **2 ファイル 2 スレッド** | **2.48** | 1.73 | — | 0.79 |

- §5.2 の見込み(1 コア = M4 の 0.5〜0.7 倍)どおり: TTree A1 で 0.65 倍。**A1 = 3.21 M hits/s は §5.4 の基準(2.5 M 以上)を満たす → 推奨(TTree、2 ファイル、ZSTD-5、IMT なし)のまま。**
- 採用案 H3(hits + events を両方)で 2.48 M hits/s = 1 MHz の約 2.5 倍。
- 16 スレッドは Mac(8 スレッド)より伸びる(TBufferMerger 23.8、RNTupleParallelWriter 63.0)。RNTuple は IMT だと 4 スレッド付近で頭打ち(D3→D4 で +12 %)、ParallelWriter は 16 でも伸びる。
- 実データとの比較: 同じ pf-qaq の N_Wall ラン 4(VX2730 1 台、331 kHz × 13.9 h、16,513,200,555 hits)を今の root_sink(TTree 1 Writer、ZSTD-5)が**取りこぼし・滞留なし**で書き、hits tree は **6.7 B/hit**(合成データの 8.0 より小さい)。`.delila` は 26.6 B/hit。詳細と、ディスクが先に限界に来る見積もりは [TODO 80](../TODO/80_generic_root_sink.md) §5.2・§6。
