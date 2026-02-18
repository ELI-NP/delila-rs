# Issue #3: Tune Up時にソフトウェアトリガーを強制する

**GitHub Issue:** #3
**Status: 📋 計画中**
**Updated:** 2026-02-18

## 目的

Tune Upモードでは外部トリガー(SINlevel等)を待たず、ソフトウェアトリガーで即座にデータ取得を開始する。
ユーザーが本番用にSIN/LVDS等を設定していても、Tune Up中は自動的にSWトリガーに切り替わる。

## 設計

### 方針: in-memoryのconfigは不変、Readerに送るcloneのみ上書き

```
state.digitizer_configs  →  clone()  →  force_software_trigger()  →  Readerに送信
       ↑
  元のまま保持（SINlevel等）
```

Tune Up終了 → 通常Run開始時は `state.digitizer_configs` から元の設定がそのまま使われる。

### 変更ファイル

1. `src/config/digitizer.rs` — `force_software_trigger()` メソッド追加
2. `src/operator/routes/tuneup.rs` — `tuneup_start()` と `tuneup_apply()` で呼び出し

### DevTree パラメータ値

| FirmwareType | DevTree Path | SW Trigger Value |
|-------------|-------------|------------------|
| PSD1 / PHA1 | `/par/startmode` | `START_MODE_SW` |
| PSD2 / AMax | `/par/startsource` | `SWcmd` |

### 実装詳細

**Step 1:** `DigitizerConfig` に `force_software_trigger()` メソッド追加

```rust
pub fn force_software_trigger(&mut self) {
    let sw_value = match self.firmware {
        FirmwareType::PSD1 | FirmwareType::PHA1 => "START_MODE_SW",
        FirmwareType::PSD2 | FirmwareType::AMax => "SWcmd",
    };
    self.board.start_source = Some(sw_value.to_string());
    if let Some(ref mut sync) = self.sync {
        sync.start_source = Some(sw_value.to_string());
    }
}
```

注意: `SyncConfig.start_source` は `BoardConfig.start_source` より優先される（`to_caen_parameters()` L877-890）ため、両方の上書きが必要。

**Step 2:** `tuneup_start()` (~L163) でclone後に適用

**Step 3:** `tuneup_apply()` (~L402) でも同様に適用（ユーザーconfigはL358で既に保存済み）

**Step 4:** ユニットテスト (PSD1, PSD2, sync有/無)

## テスト

- [ ] `cargo test` — ユニットテスト
- [ ] `cargo clippy -- -D warnings`
- [ ] 実機: DT5730B (PSD1) で `START_MODE_S_IN` 設定後 Tune Up → 即座にデータ取得開始

## コスト見積もり

- 追加行数: 25-35行
- 変更ファイル: 2
- リスク: 非常に低
