# delila2root コンバーター設計書

> **Status: superseded by Rust [src/bin/delila_to_root.rs](../../src/bin/delila_to_root.rs) (binary 名 `delila2root`)** — 本書は C++ 版の設計記録。 C++ 版は wire format 拡張 (`user_info[4]` + Phase 4.5 probe-type + AMax 16-digital-probe) に追従できず TODO 56 (2026-05-15) で `tools/delila2root/` ごと退役した。 Rust 版は schema を `#[serde(default)]` で前方互換、 全 49 branch (波形 vector 含む)、 hadd 後処理で LZ4 圧縮 (`hadd -f404 out.lz4.root out.root`)。 詳細は [TODO/56_delila2root_waveform_support.md](../../TODO/56_delila2root_waveform_support.md) 参照。

**Created:** 2026-02-17
**Updated:** 2026-02-18 (Phase 1 完了、ベンチマーク結果追記)
**TODO:** [TODO/33_delila2root_converter.md](../../TODO/33_delila2root_converter.md)

## Context

.delila バイナリファイル（MsgPack）を時間ソート済みの ROOT TTree に変換するスタンドアロン C++ ツール。
run0018 は 99 ファイル × ~10.5M events/file = ~10億イベント（波形なし、~21 GB）。

### 初版の問題点

初版はスライディングウィンドウ方式で実装したが、**3ファイル (31.5M events) で 8分+、メモリ 16GB** という致命的なパフォーマンス問題が発生。

**根本原因:**
1. `buffer.erase(begin, begin+count)` — O(n) ベクタ先頭削除。10M 要素が毎回メモリコピー
2. `std::sort` を carry_over + 新ファイルの結合バッファ全体に適用 — 毎回 O(n log n) フルソート
3. `unique_ptr<Waveform>` — millions のヒープ確保/解放 + ソート時の間接参照コスト

### データの前提 (重要)

- ファイル内のイベントは **グローバルにはソートされていない**
  - 複数デジタイザのバッチが Merger でインターリーブされて書き込まれる
  - 同一デジタイザ内でも、異なるチャンネル間のソートは保証されない
  - 読み出し境界上のデータもソート保証なし
- **各ファイル読み込み後にフルソートが必須**
- ただし carry_over（前ファイルの残り）はソート済み

### ポータビリティ要件

- macOS (M4 Pro/Max, NVMe SSD) / Linux (HDD含む)
- Apple clang / g++ (C++17)
- ROOT 6.x
- HDD 環境での同時 read/write はシーク増加で劣化するため、I/O パターンはシーケンシャルを基本

---

## アルゴリズム: POD Event + Per-File Sort + Two-Pointer Merge

### 概要

```
Phase 1: Footer 先読み → ファイルを first_event_time 順にソート
Phase 2: ROOT 出力セットアップ (LZ4圧縮, 大バスケット)
Phase 3: シーケンシャル処理 (デフォルト, HDD安全)
  For each file[i]:
    (a) ファイル全体をシーケンシャルリード + MsgPack パース → events vector
    (b) std::sort(events) — POD 24B なので高速
    (c) Two-pointer merge: carry_over × events → TTree::Fill
        threshold (= file[i+1].first_event_time) 以下は直接 Fill
        threshold 以上は次の carry_over へ push
Phase 4: 残りフラッシュ → tree->Write() → Close

オプション (Phase 3 将来): --threads で先読みパイプライン有効化 (SSD向け)
```

### 改善点の比較

| 項目 | 初版 | 高速化版 |
|------|------|----------|
| Event 構造体 | 40B, unique_ptr<Waveform> | 24B, POD |
| ソート対象 | 全バッファ (carry_over + 新ファイル) | 新ファイルのみ |
| ソート回数/ファイル | 1回 (バッファ全体) | 1回 (新ファイルのみ) |
| マージ方式 | なし (全体ソートで統合) | two-pointer merge O(n) |
| バッファ削除 | vector::erase O(n) | swap trick O(1) |
| メモリピーク | 16GB+ (3ファイル) | ~320 MB |
| ROOT 圧縮 | デフォルト (ZLIB) | LZ4 Level 1 (高速) |

---

## データ構造

### Event (非波形モード, Phase 1)

```cpp
struct Event {
    double timestamp_ns;   // 8B — 先頭配置 (ソートキーのキャッシュ効率)
    uint64_t flags;        // 8B
    uint16_t energy;       // 2B
    uint16_t energy_short; // 2B
    uint8_t module;        // 1B
    uint8_t channel;       // 1B
    // padding 2B
    // sizeof = 24 bytes, POD, CopyAssignable, trivially swappable
};
```

**設計判断:**
- `unique_ptr<Waveform>` を排除 → ヒープ確保ゼロ、POD で直接 sort 可能
- `timestamp_ns` を先頭に配置 → ソート時のキャッシュライン先頭にソートキー
- 24B = キャッシュフレンドリー、std::sort のスワップコスト最小
- CopyAssignable → 将来的に並列ソートも適用可能 (__gnu_parallel::sort, std::execution::par)

### Gemini との協議結果

Gemini (gemini-analyze-code, performance focus) との2回の議論で以下が確認された:

1. **POD 32B 以下なら std::sort 直接がインデックスソートより速い** (M4 Max)
   - インデックスソートはランダムアクセスパターンを生む
   - POD の直接スワップはシーケンシャルアクセスでハードウェアプリフェッチが効く
2. **Two-pointer merge で直接 TTree::Fill** が最適
   - inplace_merge + flush + erase の3ステップを1パスに統合
3. **unique_ptr 排除が最大の改善** — ヒープ確保 millions 回を排除
4. **LZ4 Level 1** は ZLIB より大幅に高速（ROOT のデフォルト）

---

## 実装詳細

### Phase 1: 非波形モード高速化

#### 1. Event 構造体を POD に変更
- `unique_ptr<Waveform>` と Waveform 構造体を削除（非波形モード）
- `timestamp_ns` を先頭に移動
- sizeof(Event) = 24 bytes

#### 2. read_events_from_file → ファイル単位読み込み+ソート
- 現行関数を修正して POD Event のベクタを返す
- 読み込み後に `std::sort(events.begin(), events.end(), cmp)` を適用
- MsgPackParser は波形データを `skip_value()` でスキップ

#### 3. メインループを Two-Pointer Merge に全面置換

```cpp
void merge_and_flush(std::vector<Event>& carry_over,
                     const std::vector<Event>& file_events,
                     double safe_threshold,
                     TTree* tree, /* branch vars */) {
    auto it_c = carry_over.begin();
    auto it_n = file_events.begin();
    std::vector<Event> next_carry;

    while (it_c != carry_over.end() || it_n != file_events.end()) {
        const Event* ev;
        if (it_c != carry_over.end() &&
            (it_n == file_events.end() ||
             it_c->timestamp_ns <= it_n->timestamp_ns)) {
            ev = &(*it_c++);
        } else {
            ev = &(*it_n++);
        }

        if (ev->timestamp_ns < safe_threshold) {
            // 直接 TTree::Fill — 中間バッファなし
            br_mod = ev->module;
            br_ch = ev->channel;
            br_energy = ev->energy;
            br_eshort = ev->energy_short;
            br_timestamp = ev->timestamp_ns;
            br_flags = ev->flags;
            tree->Fill();
            total_written++;
        } else {
            next_carry.push_back(*ev);
        }
    }
    carry_over = std::move(next_carry);
}
```

**ポイント:**
- carry_over (ソート済み) と file_events (ソート済み) を同時走査
- threshold 以下は即 `TTree::Fill()` — 中間バッファに溜めない
- threshold 以上は `next_carry` に push → 次イテレーションの carry_over
- `buffer.erase()` 完全排除 — `carry_over = std::move(next_carry)` のみ

#### 4. ROOT TTree 最適化

```cpp
fout->SetCompressionAlgorithm(ROOT::kLZ4);
fout->SetCompressionLevel(1);       // 高速圧縮
tree->SetAutoFlush(1000000);        // 100万エントリごとにフラッシュ
// 各 Branch にバスケットサイズ指定 (デフォルト 32KB → 256KB)
tree->Branch("Timestamp", &br_timestamp, "Timestamp/D", 256000);
```

#### 5. Makefile (ポータブル)

```makefile
CXX      = $(shell root-config --cxx)   # ROOT が使うコンパイラに合わせる
CXXFLAGS = -O3 -Wall -Wextra $(shell root-config --cflags)
LDFLAGS  = $(shell root-config --libs)
# 環境依存の追加最適化 (手動で有効化):
# CXXFLAGS += -march=native -flto
# LDFLAGS  += -flto
```

### Phase 2: 波形モード対応 (将来)

Event 構造体に波形参照を追加:
```cpp
struct Event {
    double timestamp_ns;
    uint64_t flags;
    uint32_t wf_offset;    // waveform pool 内のオフセット (0 = 波形なし)
    uint16_t wf_length;    // 波形サンプル数
    uint16_t energy;
    uint16_t energy_short;
    uint8_t module;
    uint8_t channel;
    // sizeof = 32 bytes, まだ POD
};
```

波形データは `std::vector<int16_t> analog_pool` 等のフラットバッファで管理。
carry_over に波形付きイベントを移す際は、carry_over 用プールにデータをコピーして offset を書き換える。

### Phase 3: オプション並列化 (将来)

```
./delila2root --threads -o out.root data/*.delila
```

- Reader thread が file[i+1] を read+parse+sort する間、Main thread が file[i] を merge+flush
- ダブルバッファ: `std::thread` + `mutex` + `condition_variable` で handoff
- HDD 環境では非推奨（同時 read/write でシーク増加）
- デフォルト OFF

---

## メモリ予算 (非波形モード)

| コンポーネント | 見積もり | 実測 (99 files) |
|---|---|---|
| ファイルイベント: 10.5M × 24B | ~250 MB | ~250 MB |
| carry_over (通常 ~1-2K events) | ~48 MB (見積もり過大) | ~48 KB |
| MsgPack ブロックバッファ (再利用) | ~1 MB | ~1 MB |
| ROOT バスケット + インデックス + 圧縮バッファ | ~20 MB | **~1.9 GB** |
| **合計ピーク (シーケンシャル)** | **~320 MB** | **2.2 GB** |
| **合計ピーク (--threads)** | **~570 MB** | (未測定) |

**注:** ROOT の内部メモリ使用量が見積もりを大幅に超えた。10億エントリの TTree では、バスケットバッファ、エントリインデックス (TTreeIndex)、LZ4 圧縮ワークバッファの合計が ~1.9 GB に達する。これは ROOT の仕様上避けられない。初版の 16 GB+ と比較すれば大幅な改善。

## パフォーマンス見積もり vs 実測

### 見積もり

| 操作 | 1ファイルあたり | SSD | HDD |
|---|---|---|---|
| Read + Parse (210 MB) | シーケンシャルリード | ~0.5s | ~1.5s |
| std::sort (10.5M × 24B POD) | CPU バウンド | ~0.5s | ~0.5s |
| Two-pointer merge + Fill | TTree::Fill 律速 | ~1.5s | ~1.5s |
| **実効** | | **~2.5s** | **~3.5s** |

| 構成 | SSD | HDD |
|---|---|---|
| シーケンシャル (99 files) | ~4 min | ~6 min |
| --threads (99 files) | ~3 min | 非推奨 |
| + tree->Write | +30s | +30s |

### 実測結果 (2026-02-18, macOS M4 Pro, NVMe SSD)

| テスト | ファイル数 | イベント数 | 時間 | レート | ピーク RSS |
|---|---|---|---|---|---|
| 3ファイル | 3 | 31,740,984 | **5.4 s** | 5.9 M/s | 636 MB |
| 99ファイル | 99 | 1,041,862,138 | **151 s (2m31s)** | 6.9 M/s | 2.2 GB |

**1ファイルあたり実効: ~1.5 s** (見積もり ~2.5 s より高速)

### 検証結果 (10.4億エントリ全件走査, 53秒)

| 検証項目 | 結果 |
|---|---|
| イベント数一致 | 1,041,862,138 / 1,041,862,138 — **OK** |
| タイムスタンプ昇順 | **違反 0** |
| NaN タイムスタンプ | 0 |
| 負のタイムスタンプ | 0 |
| タイムスタンプ範囲 | 0.165 ms → 58,881 s (約16.4時間) |
| モジュール/チャンネル | Mod 0-5, 83 ch — 全て正常 |
| carry_over | 各ファイル間 ~900-2,000 events |
| 出力ファイル | 14 GB (LZ4圧縮) |

### 初版との比較

| 項目 | 初版 | 高速化版 | 改善率 |
|---|---|---|---|
| 3ファイル | 8分+ (killed, 未完了) | 5.4秒 | **>90x** |
| 99ファイル | 処理不能 | 2分31秒 | **∞** |
| ピークメモリ | 16 GB+ (3ファイルで) | 2.2 GB (99ファイル) | **>7x** |
| レート | — | 6.9 M events/s | — |

---

## ファイル構成

| File | Description |
|------|-------------|
| `tools/delila2root/delila2root.cpp` | メインプログラム（MsgPack パーサー含む） |
| `tools/delila2root/Makefile` | `root-config` ベースのビルド |

## ROOT TTree ブランチ

### スカラーブランチ（常時出力）
```
Mod        : UChar_t   (u8)
Ch         : UChar_t   (u8)
Energy     : UShort_t  (u16)
EnergyShort: UShort_t  (u16)
Timestamp  : Double_t  (f64, nanoseconds)
Flags      : ULong64_t (u64)
```

### 波形ブランチ（`--waveform` オプション時のみ, Phase 2）
```
AP1        : vector<Short_t>   (analog probe 1)
AP2        : vector<Short_t>   (analog probe 2)
DP1        : vector<UChar_t>   (digital probe 1)
DP2        : vector<UChar_t>   (digital probe 2)
DP3        : vector<UChar_t>   (digital probe 3)
DP4        : vector<UChar_t>   (digital probe 4)
```

## 使い方

```bash
cd tools/delila2root && make
# スカラーのみ（高速）
./delila2root -o output.root data/run0018_*.delila
# 波形付き (Phase 2)
./delila2root -o output.root --waveform data/run0018_*.delila
# SSD 環境でパイプライン有効化 (Phase 3)
./delila2root --threads -o output.root data/run0018_*.delila
```

## Verification — ✅ Phase 1 全項目クリア (2026-02-18)

| # | 検証項目 | 結果 |
|---|---|---|
| 1 | `make` でビルド成功 | ✅ (Apple clang, ROOT 6.36, -O3) |
| 2 | 3ファイルテスト | ✅ 5.4秒, 31,740,984 events |
| 3 | タイムスタンプ昇順 | ✅ 10.4億エントリ全件走査, 違反 0 |
| 4 | イベント数一致 | ✅ 1,041,862,138 / 1,041,862,138 |
| 5 | メモリ監視 | ✅ 2.2 GB (ROOT内部バッファ含む) |
| 6 | 99ファイル全件ベンチマーク | ✅ 151秒 (2分31秒), 6.9 M events/s |
