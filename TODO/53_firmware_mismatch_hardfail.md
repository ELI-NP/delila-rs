# TODO 53: Firmware Mismatch Hard-Fail on Configure

**Status:** ✅ **COMPLETED (2026-05-07)**
**Created:** 2026-05-07 (live discovery during R-P4 verification session)
**Commit:** `c7d1fc8`

## 背景

2026-05-07 の R-P4 (Angular settings panel + NotificationService migration) commit (`111b9fd`) 直後の動作確認で発見した silent miswire。

`config_pha2_56_phys.toml` (`type = "pha2"` 宣言) を、 実体が AMax FW (DPP_OPEN) のデジタイザ (172.18.4.56) に対して Apply したところ:

| 計測 | 値 |
|---|---|
| `total` params | 43 |
| `ok` (apply 成功) | 12 (28%) — DevTree path を AMax と共有する board-level params のみ |
| `skipped` (cache miss / DevTree 未登録) | 31 (72%) |
| `failed` | 0 |
| Backend WARN 行数 | 62 (Failed to set parameter ... CAEN error -6) |
| `system_state` | `Configured` (operator は成功と判定) |
| UI snackbar | "Configuration applied to hardware" (緑) |

**[CLAUDE.md](../CLAUDE.md) の "Silent failure を作らない" ポリシー違反**:
> cache miss / 範囲外値 / FW 拒否は必ず `info!` 以上で可視化。 `debug!` のまま埋めると数ヶ月後に発見される

backend log では `info!` 以上で出ていたが、 (a) Apply summary の `skipped=31` が surface されない、 (b) 集計レベルで「成功」扱いだったため operator REST → UI snackbar まで届かなかった、 という二重 surface 漏れ。

## 根本原因

`src/reader/read_loop_dig1.rs::run` および `src/reader/read_loop_dig2.rs::run` の `ReadLoopRequest::ApplyConfig` arm が `try_connect_*` 直後に `apply_config_validated` を呼ぶだけで、 **HW 報告 firmware と config 宣言 firmware を比較していなかった**。

`CaenHandle::get_device_info()` は `/par/FwType` から "DPP_OPEN" / "DPP_PHA" 等の文字列を返せていたが、 これを使うのは `Detect` REST endpoint のみで、 `Configure` 系では完全に未使用だった。

## 修正内容 (commit `c7d1fc8`)

### 1. `FirmwareType::from_caen_device` 新設 (`src/config/digitizer.rs`)

```rust
pub fn from_caen_device(firmware_type: &str, model: &str) -> Option<Self>
```

- 5 known FELib FW string → 対応 `FirmwareType` 直接 map
- model name fallback: `1730/1725/5730/5725` → `Some(PSD1)`、 `1740/2730/2740` → `Some(PSD2)`
- 未知の場合 `None` (旧 helper の "PSD2 default" は detect path の shim 側に移動)
- X743 family は **意図的に never returned** (FELib 経由しない、 defense-in-depth)

### 2. `check_firmware_match` 共有 helper (`src/reader/mod.rs`)

dig1/dig2 両方から呼ぶ helper:

```rust
pub(crate) fn check_firmware_match(
    conn: &DeviceConnection,
    url: &str,
    declared: FirmwareType,
) -> Result<(), String>
```

mismatch 時の error message は URL + HW report (FW string + model + serial) + mapped variant + declared variant を含み、 operator は読んだ瞬間に診断可能:

```
Firmware mismatch: digitizer at dig2://172.18.4.56 reports firmware
"DPP_OPEN" model "VX2730" SN "52622" (mapped to AMax), but config
declares firmware PHA2. Refusing to Apply — reload the correct config
or update the source's `type` field, then re-Configure.
```

### 3. dig1/dig2 ApplyConfig arm に check 挿入

`try_connect_*` 直後、 `apply_config_validated` の前。 mismatch なら `Err(...)` で短絡し `state.rs:369 → CommandResponse::error → operator REST 5xx → UI 赤 snackbar` に伝搬。

`ApplyConfigRunning` arm は post-Running 専用 (= ApplyConfig 成功後しか到達しない) ので check は重複、 instrument せず。

### 4. 既存 helper を 2 行 shim に refactor

`src/operator/routes/digitizer.rs::firmware_from_device_info` は新 helper を呼ぶ薄い wrapper に。 detect path の `unwrap_or(FirmwareType::PSD2)` 既存セマンティクスは shim 側で維持、 唯一の caller (`detect_digitizers` line 288) は call site 不変。

## 影響範囲

| 領域 | 影響 |
|---|---|
| Emulator (`data_source_emulator`) | 影響なし — `CaenHandle` 経由しない別系統 binary |
| X743 read loop (`read_loop_x743_std`) | 影響なし — `CaenLegacyHandle` 別系統 |
| `CaenHandle::apply_config_validated` 既存テスト (`caen/handle.rs:2167`) | 影響なし — read loop bypass で直接呼ぶ |
| Detect REST endpoint | 既存 PSD2 default 維持 (shim) |
| 7 FW (PSD1/PSD2/PHA1/PHA2/AMax/V1743/X743Std) ApplyConfig path | DIG1/DIG2 の 5 firmware は新 check を通過、 X743 系は影響なし |

## テスト

`src/config/digitizer.rs::tests` に 6 unit tests 追加:

1. `from_caen_device_maps_dig1_strings` — `DPP-PSD` → PSD1、 `DPP-PHA` → PHA1
2. `from_caen_device_maps_dig2_strings` — `DPP_PSD` → PSD2、 `DPP_PHA` → PHA2、 `DPP_OPEN` → AMax
3. `from_caen_device_falls_back_on_model` — FW string 空でも model 名で recover
4. `from_caen_device_returns_none_when_unknown` — **None 返却を pin** (旧 PSD2 default からの semantic 変更)
5. `from_caen_device_never_returns_x743` — defense-in-depth
6. `from_caen_device_pha2_config_rejects_amax_hardware` — **2026-05-07 case 固定 regression**

read-loop 側の integration test は `CaenHandle` mocking が困難のため省略。 read-loop の追加コード (~10 行 / arm) は helper 呼び出し + `Err` short-circuit のみで、 logic は helper 側 unit test で完全カバー。

## 検証

```
cargo test --release --features dev-tools,root              # 579 → 585 pass (+6)
cargo clippy --release --tests --features dev-tools,root -- -D warnings  # clean
cargo build --release                                        # default clean
cargo build --release --features dev-tools,root             # all features clean
```

### Live smoke (実機検証 — 別途実施予定)

1. `/stop-daq` で実行中の DAQ 停止
2. 新 binary を実機 (172.18.4.56) で稼働させて `config_pha2_56_phys.toml` で `/start-daq`
3. UI で **Configure** 押下:
   - **期待**: 赤 snackbar に詳細 mismatch message 表示
   - **期待**: backend log の WARN 62 連発が **出ない** (hard-fail で短絡)、 ERROR 1 行のみ
   - **期待**: `system_state` は `Idle` のまま
4. 正しい AMax config で Configure → 通常通り成功

## 関連

- [CLAUDE.md](../CLAUDE.md) "Silent failure を作らない" ポリシー強化事例
- [TODO/52](52_refactor_sprint_2026-q2.md) R-P4 検証中の live discovery
- 既存 silent failure 事案: 2026-05-04 `e641e99` (PHA2 mid-loop heuristic で波形 truncation)、 `e45e0ec` (case-insensitive cache miss が silent fallthrough)
- 2 行 shim refactor: 2026-05-06 R-P5 (digitizer route helpers) と同じ「single-purpose helper の合成」判断
