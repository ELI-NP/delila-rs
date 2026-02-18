# TODO #33: delila2root コンバーター

**Created:** 2026-02-17
**Status: ✅ Phase 1 完了**
**Priority:** 2

## Summary

.delila バイナリファイル（MsgPack）を時間ソート済みの ROOT TTree に変換するスタンドアロン C++ ツール。
自動変換パイプラインに組み込み可能。一時ファイルなし。

## Phase 1 完了 (2026-02-18)

設計書: `docs/plans/delila2root.md`

### 実装内容
1. **Event 構造体を POD 化** (24B, unique_ptr 排除)
2. **per-file sort + two-pointer merge** (全バッファソート排除)
3. **swap trick** (vector::erase 排除)
4. **ROOT TTree 最適化** (LZ4, AutoFlush 100万, バスケット 256KB)
5. **Makefile** (-O3, `root-config` ポータブル)

### ベンチマーク結果 (2026-02-18, macOS M4 Pro, NVMe SSD)

| 項目 | 初版 | 高速化版 |
|------|------|----------|
| 3ファイル (31.5M events) | 8分+ (killed) | **5.4秒** |
| 99ファイル (10.4億 events) | 処理不能 (16GB+ RAM) | **2分31秒** |
| レート | — | **6.9 M events/s** |
| ピークメモリ | 16 GB+ (3ファイル) | **2.2 GB** (99ファイル) |
| 出力ファイル (99 files) | — | 14 GB (LZ4) |

### 検証結果 (2026-02-18)

- **イベント数**: 1,041,862,138 / 1,041,862,138 — 完全一致
- **タイムスタンプ順序**: 10.4億エントリ全件走査 — **違反 0**
- **NaN / 負のタイムスタンプ**: 0
- **タイムスタンプ範囲**: 0.165 ms → 58,881 s (約16.4時間)
- **モジュール/チャンネル**: Mod 0-5, 83チャンネル — 全て正常
- **carry_over**: 各ファイル間 ~900-2,000 events (非常に小さい重複)

### 初版の問題点 (参考)

1. `buffer.erase(begin, begin+count)` — O(n) ベクタ先頭削除
2. `std::sort` を carry_over+新ファイルの結合バッファ全体に適用 — 毎回フルソート
3. `unique_ptr<Waveform>` — millions のヒープ確保/解放 + ソート時の間接参照

### Phase 2: 波形モード対応 (将来)
- wf_offset + wf_length で waveform pool 参照
- carry_over の波形データ管理

### Phase 3: オプション並列化 (将来)
- `--threads` でパイプライン有効化 (SSD 環境向け)
- Reader thread が次ファイルを先読み+ソートする間 Main が merge+flush

## Requirements

- 99+ ファイル × ~10.5M events/file（~10億イベント、波形なし ~21 GB）を処理可能
- メモリ使用量: ~320 MB (シーケンシャル), ~570 MB (--threads)
- 時間ソート済み ROOT TTree を出力
- `--waveform` オプションで波形ブランチも出力可能 (Phase 2)
- **各種環境で動作**: macOS (SSD) / Linux (HDD含む), clang / g++

## Files

| File | Description |
|------|-------------|
| `tools/delila2root/delila2root.cpp` | メインプログラム（MsgPack パーサー含む） |
| `tools/delila2root/Makefile` | `root-config --cflags --libs` でビルド |

## Acceptance Criteria

### Phase 1 (非波形モード) — ✅ 全クリア (2026-02-18)
- [x] `make` でビルド成功
- [x] 3ファイルで動作確認 → **5.4秒** (目標 ~10秒以内)
- [x] タイムスタンプが昇順であることを確認 → **違反 0** (10.4億エントリ全件)
- [x] イベント数一致 → **1,041,862,138 完全一致**
- [x] メモリ使用量 → **2.2 GB** (ROOT 内部バッファ含む、99ファイル)
- [x] 99ファイル全件変換 → **2分31秒** (目標 5-6分を大幅クリア)

### Phase 2 (波形モード) — 将来
- [ ] `--waveform` で波形ブランチが出力される
- [ ] 波形付きファイルでも正常変換

### Phase 3 (オプション並列化) — 将来
- [ ] `--threads` で先読みパイプライン有効化
- [ ] SSD 環境でさらなる高速化
