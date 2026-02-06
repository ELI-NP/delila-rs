# PSD1 Waveform Debug Tool

DT5730B (DPP-PSD firmware) デジタイザのwaveformデータをデバッグするためのツール。

## 概要

DAQシステム全体を使わずに、デジタイザに直接接続してRAWデータを取得・解析できる。

## ビルド

リモートマシン（172.18.4.147）でビルド：

```bash
ssh aogaki@172.18.4.147
cd ~/WorkSpace/delila-rs
cargo build --release --bin psd1_waveform_test
```

## 使い方

```
psd1_waveform_test [options]

Options:
  --url <dig1://...>     デジタイザURL (default: dig1://caen.internal/usb?link_num=0)
  --reclen <ns>          Record Length を設定（ナノ秒）
  --events <n>           読み取るイベント数 (default: 10)
  --raw                  RAWデータをhexdump表示
  --decode               PSD1デコーダでデコードして表示
```

### 例

```bash
# 基本情報とタイムボム確認
./target/release/psd1_waveform_test

# RAWデータを3イベント分表示
./target/release/psd1_waveform_test --raw --events 3

# Record Length を 512ns に設定してRAW表示
./target/release/psd1_waveform_test --reclen 512 --raw

# デコードして5イベント表示
./target/release/psd1_waveform_test --decode --events 5
```

## 出力例

### 基本情報
```
--- Timebomb Check ---
  [OK] Timebomb: 28:45 remaining

--- Device Info ---
  modelname       : DT5730B
  serialnum       : 990
  fwtype          : DPP_PSD
  numch           : 8

--- Waveform Settings ---
  Record Length (ns) : 992
  Waveforms Enabled  : TRUE
  Extras Enabled     : TRUE
```

### RAW Hexdump
```
  Board Aggregate Header:
    [0] 0xA0000123 - size=291, magic=0xA (expect 0xA)
    [1] 0x00000003 - ch_mask=0b00000011, board_id=0, fail=0
    [2] 0x00000001 - aggr_counter=1
    [3] 0x12345678 - time_tag=305419896

  Channel Aggregate Header:
    [4] 0x80000040 - ch_size=64, magic=1
    [5] 0xE0000040 - num_samples/8=64 (samples=512)
        DT=1 EQ=1 ET=1 EE=1 ES=1 AP=0 DP1=0 DP2=0 Extra=2
```

## タイムボム制限

**重要**: ライセンスなしのデジタイザは30分でタイムアウトする。

- ツール起動時に自動チェック
- 残り60秒未満で警告
- 0秒で停止、デジタイザの再起動を促す

```
--- Timebomb Check ---
  [!!!] TIMEBOMB EXPIRED!
  [!!!] Please power cycle the digitizer and restart.
```

対処法：デジタイザの電源をOFF→ONして再起動

## PSD1 RAWデータフォーマット

### Board Aggregate Header (4 words)

| Word | Bits | Field |
|------|------|-------|
| 0 | [27:0] | Aggregate size (32-bit words) |
| 0 | [31:28] | 0xA (magic) |
| 1 | [7:0] | Dual channel mask |
| 1 | [26] | Board fail flag |
| 1 | [31:27] | Board ID |
| 2 | [22:0] | Aggregate counter |
| 3 | [31:0] | Time tag |

### Channel Aggregate Header (2 words)

| Word | Bits | Field |
|------|------|-------|
| 0 | [21:0] | Channel aggregate size |
| 0 | [31] | 1 (magic) |
| 1 | [15:0] | NUM_SAMPLES / 8 |
| 1 | [18:16] | DP1 selection |
| 1 | [21:19] | DP2 selection |
| 1 | [23:22] | AP selection |
| 1 | [26:24] | Extra option |
| 1 | [27] | ES (samples enabled) |
| 1 | [28] | EE (extras enabled) |
| 1 | [29] | ET (time tag enabled) |
| 1 | [30] | EQ (charge enabled) |
| 1 | [31] | DT (dual trace enabled) |

### Waveform Data

各32-bitワードに2サンプル：

| Bits | Field |
|------|-------|
| [13:0] | Sample 0 (14-bit) |
| [14] | Digital Probe 1 |
| [15] | Digital Probe 2 |
| [29:16] | Sample 1 (14-bit) |
| [30] | Digital Probe 1 |
| [31] | Digital Probe 2 |

## 関連ドキュメント

- `legacy/UM4380_725-730_DPP_PSD_Registers_rev6.pdf` - CAEN公式マニュアル
- `legacy/DELILA2/lib/digitizer/RefMaterials/PSD1_Data` - データフォーマット仕様
- `TODO/29_psd1_waveform_debug.md` - デバッグ進捗
