# TODO 81 — データを DAQ ホストから外に出すときに `.delila.zst` に圧縮する

**Status: 📋 起票 (2026-11-03) — 方針のみ、未着手**

**関連:** [80_generic_root_sink.md](80_generic_root_sink.md) §6(ディスク予算: 書き込み量の 8 割は `.delila`) /
[69_write_failure_autostop.md](69_write_failure_autostop.md)(`scripts/archive_sync.sh`) /
[tools/delila2root/](../tools/delila2root/)(`TDelila.hpp`) / `src/recorder/format.rs`(`DataFileReader`)

---

## 1. 発端 (2026-11-03)

TODO 80 のディスク予算で、書き込む量の 8 割が `.delila`(実データ 26.6 B/hit、ROOT の hits tree は 6.7 B/hit)と分かった。
ユーザー: `.delila` を圧縮すべきか。ただし実験中に CPU を割けるか分からないので、**実験後のデータの移動・保存の段で**やるべき。

## 2. 今の状態(pf-qaq、実測)

- `/archive`(HDD 4 台の ZFS raidz2)は **ZFS の lz4 圧縮が既定で効いている**。`.delila` は 1.85〜2.10 倍に縮んで置かれている
  (ThGEM のラン 208 / 218、見かけ 911〜1074 MB → 実体 492〜544 MB)。`.root` はすでに ZSTD なので 1.04 倍。
  データセット全体では 1.49 倍。→ **pf-qaq の中では、圧縮はもう済んでいる。ZFS の設定は変えない**(ユーザー、zstd への変更は不要と判断)。
- 問題は **pf-qaq の外に出すとき**: ZFS の圧縮はファイルについていかない。外付け HDD や別のサーバーにコピーすると元の大きさに戻る。

## 3. 圧縮方式の比較(2026-11-03、pf-qaq)

試料 = `run0004_0100_N_Wall.delila` の先頭 200 MB(VX2730 PSD2、波形なし)。`head -c | 圧縮 | wc -c`、ファイルは作っていない。DAQ は Configured。

| 方式 | 圧縮率 | 圧縮の速さ(入力) |
|---|---|---|
| ZFS lz4(参考、別のファイル) | 1.85〜2.10 倍 | 透過 |
| `zstd -1`、1 スレッド | 2.43 倍 | 267 MB/s |
| `zstd -3`、1 スレッド | 2.61 倍 | 142 MB/s |
| **`zstd -3 -T8`** | **2.61 倍** | **752 MB/s** |
| `zstd -9 -T8` | 2.81 倍 | 196 MB/s |
| `zstd -19 -T16` | 3.22 倍 | 13 MB/s |
| `gzip -6` | 2.63 倍 | 24 MB/s |
| `pigz -6 -p 8` | 2.64 倍 | 188 MB/s |
| `xz -6 -T8` | 3.64 倍 | 17 MB/s |

- **gzip は使わない**: zstd -3 と同じ縮み方で 6 倍遅い。展開も zstd の方が速い(一般的な性質、未測定)。
- **xz** は一番縮むが、1 TB に 16 時間。使うなら長期の冷蔵保存だけ。
- **`zstd -3 -T8`** なら 1 TB を約 22 分で 2.6 倍に。
- 記録の経路(Recorder)には入れない。CPU は理由ではなく(1 MHz でも `.delila` は 27 MB/s、zstd -3 の 1 コアの 2 割)、
  記録の経路はできるだけ単純にしておくため。

## 4. 方針(案)

- 名前は `run####_####_<exp>.delila.zst`。**元の `.delila` は、圧縮したものを確認するまで消さない。**
- 確認: zstd のフレームのチェックサム(`zstd -t`)+ 展開したものが元と一致(`cmp` または md5)。`archive_sync.sh` の「一致を確かめてから消す」と同じ考え方。
- 読む側: 使う前に展開させるか、ツールが `.zst` をそのまま読めるようにするか(§5 の 2)。

## 5. 決めること

1. **いつ・どこで圧縮するか**: pf-qaq から持ち出す直前に専用スクリプト(例 `scripts/export_runs.sh`: 圧縮 → 確認 → コピー → 先でもう一度確認)か、`/archive` に置いた後の夜間処理か。
2. **読む側の対応**: `.delila.zst` を直接読めるようにするか。対象 = Rust の `DataFileReader`(`recover`、`event_dump`、`event_builder`、`eb_delila_replay`、
   `delila_source_replay`、`delila_merger_replay`、`pha_trap_tune`、`pha_fw_scan`)と C++ の `TDelila.hpp`(`delila2root`、解析マクロ)。
   `TDelila.hpp` は「依存なし」が売りなので、libzstd を必須にしない形(使えるときだけ、または `zstd -dc` をパイプで)が要る。
3. **圧縮の強さ**: 既定 `-3`。持ち出し先の容量が厳しければ `-9`(2.81 倍、8 スレッドで 196 MB/s)。
4. **波形入りの `.delila`** で圧縮率を測り直す(今回は波形なしのみ)。

## 6. 次の作業

- [ ] §5 の 1〜2 をユーザーと決める
- [ ] 波形入りのファイルで §3 を測り直す
- [ ] 実装・確認
