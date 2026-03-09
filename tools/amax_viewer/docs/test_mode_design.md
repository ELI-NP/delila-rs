# AMax Viewer テストモード設計書

## 概要

`-t` フラグで起動した場合、デジタイザ内蔵テストパルスを使用するテストモードで動作する。
実検出器なしでFWパラメータ調整・データパイプライン検証が可能。

## CLI引数

`clap` (Derive API) を導入する。現在CLI引数は一切ないが、将来 `--ip`, `--ch` 等の追加が予想されるため。

```rust
use clap::Parser;

#[derive(Parser, Debug)]
#[command(about = "AMax Viewer - Firmware Development Tool")]
struct Args {
    /// Start in Test Pulse mode
    #[arg(short = 't', long)]
    test_pulse: bool,
}
```

`Cargo.toml` に `clap = { version = "4", features = ["derive"] }` を追加。

## テストパルス設定

### CAEN FELib パラメータ

| パラメータ | DevTree パス | テストモードデフォルト | 単位 |
|-----------|-------------|---------------------|------|
| Period | `/par/TestPulsePeriod` | 1000000 (1kHz) | ns |
| Width | `/par/TestPulseWidth` | 10 | ns |
| Low Level | `/par/TestPulseLowLevel` | 1000 | ADC count |
| High Level | `/par/TestPulseHighLevel` | 3000 | ADC count |

全パラメータ SetInRun=true（アクイジション中に変更可能）。

### トリガーソース変更

テストモード有効時:
```rust
handle.set_value("/par/GlobalTriggerSource", "TestPulse")?;
handle.set_value("/par/AcqTriggerSource", "GlobalTriggerSource")?;
```

テストモード無効時: 保存しておいた元の値に復元する。

## GUI変更

### テストモード表示

テストモード有効時、サイドパネル最上部にオレンジ色の警告バナーを表示:

```rust
if is_test_mode {
    ui.label(
        egui::RichText::new("TEST PULSE MODE")
            .color(egui::Color32::from_rgb(255, 100, 0))
            .heading()
            .strong()
    );
}
```

理由: テストパルスのデータを実データと誤認するリスクを排除する。

### テストパルスパラメータUI

テストモード有効時のみ、Parameters セクション内に `CollapsingHeader` で表示:

```
▼ Test Pulse
  Period:     [1000000] ns  (→ 1.0 kHz)
  Width:      [500000]  ns
  Low Level:  [1000]    ADC
  High Level: [3000]    ADC
```

- DragValue で値変更可能
- テストパルスパラメータは SetInRun=true なので、変更即時反映（Restart不要）
- 変更時は `handle.set_value()` を取得スレッドに指示

### ランタイムトグル

`-t` フラグは初期状態の設定のみ。GUI上にチェックボックスを設置し、ランタイムで切り替え可能にする。

切り替え時のフロー:
1. Stop acquisition (`disarmacquisition`)
2. トリガーソース切り替え（TestPulse ↔ 元の値）
3. Restart acquisition (`armacquisition` → `swstartacquisition`)

## トリガーソースの保存・復元

### 保存タイミング
テストモード有効化時（初回接続時 or ランタイムトグル時）に現在値を読み取り保存:
```rust
let original_gts = handle.get_value("/par/GlobalTriggerSource")?;
let original_ats = handle.get_value("/par/AcqTriggerSource")?;
```

### 復元タイミング
- テストモード無効化時
- アプリ終了時（`on_exit` / acquisition thread shutdown）

理由: テストモードのまま終了→次回実データ取得時にトリガーが来ない、という事故を防ぐ。

## WaveDataSource

テストモード時に `ADC_DATA` を強制しない。デフォルト値は変更せず、ユーザーが自由に選択可能とする。

理由: FW開発者はテストパルスを入力しつつ TRAPEZOID や BASELINE の波形を確認したいケースがある。

## 実装スコープ

### Phase 1（最小実装） — **COMPLETED** (2026-03-09)
- [x] `clap` 導入、`-t` / `--test-pulse` フラグ追加
- [x] テストモード時にテストパルスパラメータをデフォルト値で設定 (1kHz, Width=10ns, ADC 1000-3000)
- [x] トリガーソースを TestPulse に変更 (AcqTriggerSource のみ — OpenDPP FW は GlobalTriggerSource 非対応)
- [x] GUI にテストモードバナー表示 (オレンジ色 "TEST PULSE MODE")
- [x] ウィンドウタイトルに [TEST PULSE] 付加
- [x] 終了時にトリガーソース復元 + TestPulsePeriod=0 で無効化

### Phase 2（UX改善） — **COMPLETED** (2026-03-09)
- [x] テストパルスパラメータのGUI調整（CollapsingHeader + DragValue、周波数表示付き）
- [x] SetInRun で即時反映（パラメータ変更は Restart 不要、dirty フラグで取得スレッドが検知）
- [x] ランタイムトグル（チェックボックスで切り替え、Stop→トリガー変更→Restart を自動実行）

## 参考

- 既存テスト実装: `src/bin/amax_testpulse_test.rs`
- DevTree テストパルスパラメータ: `docs/devtree_examples/vx2730_psd2_sn52622.json`
- CAEN FELib テストパルスドキュメント: "Period of the Test Pulse, a programmable square wave that can be used as an internal periodic trigger"
