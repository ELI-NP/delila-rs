# TODO 29: PSD1 Waveform Debug

**Status: COMPLETED**
**Created: 2026-02-06**
**Completed: 2026-02-06**

## 問題

PSD1 (DT5730B) デジタイザからのwaveformデータに問題あり：

1. ~~**サンプル数が変わらない** - Record Length を 1024→512 に変更しても 512点のまま~~ → 解決
2. ~~**データが無意味** - 同じ値ばかりが入っている~~ → 解決
3. ~~**パルス位置が異常** - min_pos=394 (waveform末尾)~~ → 解決

## 環境

- Hardware: DT5730B (Serial: 990, DPP-PSD firmware)
- Connection: USB via Linux machine (172.18.4.147)
- License: **なし** - 30分タイムボム制限あり

## デバッグツール

`src/bin/psd1_waveform_test.rs` を作成

### 使い方

```bash
# リモートでビルド
ssh aogaki@172.18.4.147
cd ~/WorkSpace/delila-rs
cargo build --release --bin psd1_waveform_test

# 基本情報表示（タイムボムも確認される）
./target/release/psd1_waveform_test

# RAWデータ確認
./target/release/psd1_waveform_test --raw --events 3

# Record Length変更してテスト
./target/release/psd1_waveform_test --reclen 512 --raw

# デコードテスト
./target/release/psd1_waveform_test --decode --events 5
```

## 確認ポイント

### RAWデータフォーマット（CAEN PSD1）

1. **Board Aggregate Header** (4 words)
   - Word 0: bit[28:31] = 0xA (magic), bit[0:27] = size
   - Word 1: bit[0:7] = channel mask
   - Word 2: aggregate counter
   - Word 3: time tag

2. **Channel Aggregate Header** (2 words)
   - Word 0: bit[0:21] = size, bit[31] = 1
   - Word 1: bit[0:15] = NUM_SAMPLES/8

3. **Event Data**
   - Time tag (if ET=1)
   - Waveform (if ES=1): bit[0:13] = sample, bit[14] = DP1, bit[15] = DP2
   - Extras (if EE=1)
   - Charge (if EQ=1)

## 調査ログ

### 2026-02-06

- [x] 初回RAWデータ取得
- [x] Board/Channel headerの検証
- [x] NUM_SAMPLES/8 の値確認
- [x] 生waveformサンプル値の確認

**根本原因特定: `/par/waveforms = FALSE` だった**

デフォルト設定でwaveformが無効になっていた。`--enable-waveform`オプションで有効化すると正常に取得できた。

#### waveform無効時 (ES=0)
```
Channel Aggregate Header:
  [5] 0x72000000 - num_samples/8=0 (samples=0)
      DT=0 EQ=1 ET=1 EE=1 ES=0 AP=0 DP1=0 DP2=0 Extra=2
```

#### waveform有効時 (ES=1)
```
Channel Aggregate Header:
  [5] 0x7A00003E - num_samples/8=62 (samples=496)
      DT=0 EQ=1 ET=1 EE=1 ES=1 AP=0 DP1=0 DP2=0 Extra=2

Waveform samples:
  S[0]=13267, S[1]=13277, S[2]=13276, ... (正常な値)
```

#### 解決策
設定ファイル (`config/digitizers/psd1_test.json`) で `waveforms: true` を設定する

## タイムボム注意

ライセンスなしデジタイザは30分でタイムアウト。
`/par/timebombdowncounter` で残り時間確認可能。
期限切れの場合はデジタイザを物理的に再起動（電源OFF/ON）。

## 関連ファイル

- `src/bin/psd1_waveform_test.rs` - デバッグツール
- `src/bin/psd1_raw_dump.rs` - RAWバイナリ解析ツール
- `src/reader/decoder/psd1.rs` - PSD1デコーダ
- `docs/psd1_debug_tool.md` - ツール詳細ドキュメント
- `legacy/DELILA2/lib/digitizer/RefMaterials/PSD1_Data` - データフォーマット仕様

## 解決: デコーダのwaveformワード計算バグ修正

### 問題の根本原因

デコーダが waveform データを半分しか読んでいなかった。

**CAEN仕様** (`legacy/.../PSD1_Data` より):
> NUM SAMPLES WAVE/8 corresponds to the number of words to be read in the event related to the waveform /4

つまり: `waveform_words = num_samples_wave * 4`

**旧コード (バグ)**:
```rust
let total_words = ch_header.num_samples_wave as usize * 2;  // 半分しか読まない！
```

**修正後**:
```rust
let total_words = ch_header.num_samples_wave as usize * 4;  // 正しい
```

### 検証結果

`psd1_raw_dump` でバイナリ解析:
```
num_samples/8 = 8 (samples=64)
Waveform words: 32 (ES=1, num_samples_wave=8 * 4)
Event total: 35 words = 140 bytes
Channel data: 5600 words (ch_size - 2)
Events: 160 (ch_data / event_size) ← 割り切れる！
```

修正後の `psd1_waveform_test` 結果:
```
Pulse: min=3755 max=8305 range=4550 min_pos=322
```
- pre-trigger = 320ns = 160サンプル
- min_pos=322 (重複込み) → 実サンプル=161 → トリガー点(160)直後 ✓

### 修正ファイル

1. `src/reader/decoder/psd1.rs`
   - `DualChannelHeader::event_size_words()`: `* 2` → `* 4`
   - `decode_waveform()`: `* 2` → `* 4`

2. `src/reader/decoder/pha1.rs` (同様のバグを修正)
   - `DualChannelHeader::event_size_words()`: `* 2` → `* 4`
   - `decode_waveform()`: `* 2` → `* 4`

### 注意: サンプル重複

現在のデコーダは各サンプルを2回出力している（CAEN style duplication）。
400サンプル → 800出力サンプル。これが必要かどうかは今後検討。
