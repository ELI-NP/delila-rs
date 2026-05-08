# TODO 53: Firmware Mismatch Hard-Fail on Configure

**Status:** ✅ **COMPLETED (2026-05-07)**
**Created:** 2026-05-07 (live discovery during R-P4 verification session)
**Commits:** `c7d1fc8` (initial: explicit Apply arm) → `91e3505` (live-smoke follow-up: Configure auto-load arm) → `6911651` (UI: surface backend message + Material 3 snackbar token)

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

## 修正内容 (3 commits, 1 セッション)

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

### 3. dig1/dig2 ApplyConfig arm に check 挿入 (`c7d1fc8`)

`try_connect_*` 直後、 `apply_config_validated` の前。 mismatch なら `Err(...)` で短絡し `state.rs:369 → CommandResponse::error → operator REST 5xx → UI 赤 snackbar` に伝搬。

`ApplyConfigRunning` arm は post-Running 専用 (= ApplyConfig 成功後しか到達しない) ので check は重複、 instrument せず。

### 4. 既存 helper を 2 行 shim に refactor (`c7d1fc8`)

`src/operator/routes/digitizer.rs::firmware_from_device_info` は新 helper を呼ぶ薄い wrapper に。 detect path の `unwrap_or(FirmwareType::PSD2)` 既存セマンティクスは shim 側で維持、 唯一の caller (`detect_digitizers` line 288) は call site 不変。

### 5. Configure auto-load path も同 check で gate (`91e3505`、 live-smoke driven)

ライブ確認で発覚: `c7d1fc8` は **explicit Apply** arm (UI "Apply" ボタン → `/api/digitizers/<id>/apply`) しか守っていなかった。 一方 `read_loop_dig1.rs:123-143` / `read_loop_dig2.rs:129-159` の **Configure 状態遷移ハンドラ** は独自に `config.config_file` を読み込んで `conn.handle.apply_config(&dig_config)` を直接呼ぶ auto-load path を持っており、 ここが完全に無防備だった。 `start_daq → /api/configure` がこの auto-load path を発火させるので、 ApplyDigitizerConfig が explicit Apply arm に到達する前に silent miswire が再現する。

修正: dig1/dig2 両方の Configure arm で `conn.handle.apply_config(...)` を `match check_firmware_match(...)` で包む。 mismatch なら ① 詳細 ERROR を 1 行 log、 ② `conn.auto_config_failed = true` を立てて Arm command を blocking (既存の apply_config-failed branch と同じ semantics)、 ③ 早期 return。

ライブ検証 (PHA2 config → AMax HW、 同一 binary):

| | `c7d1fc8` のみ | `91e3505` 適用後 |
|---|---|---|
| WARN 行 | 62 | 0 |
| "Failed to set parameter" | 31 | 0 |
| ERROR | 0 | 1 (full mismatch detail) |
| INFO "Configuration applied" | applied=12 errors=31 | 出ない (短絡) |

これで **explicit Apply** と **Configure auto-load** の両 path が gate された。

### 6. UI: backend の rich diagnostic を surface + Material 3 snackbar token (`6911651`、 live-smoke 派生)

ライブで `91e3505` を確認した際、 詳細 mismatch メッセージが UI に届かず "Failed to apply configuration" の汎用文だけが灰色 snackbar で出る 2 件のバグが発覚:

1. **HttpErrorResponse handling**: backend は `HTTP 500 + {success:false, message:"Reader rejected config: Firmware mismatch: ..."}` を返すが、 Angular の `HttpClient` は `HttpErrorResponse` を投げ、 これは `instanceof Error` ではない → `digitizer-settings.component.ts::applyConfig()` の catch が fallback branch に落ちて生メッセージが捨てられていた。 Pre-R-P4 から潜在 (`MatSnackBar.open(...)` 旧コードも同形) だが、 これまで意味のあるエラー body を返す backend が無かったため masked。 修正: `err.error?.message ?? err.message` で body を取り出す (既に `pages/waveform.component.ts:1188` と `pages/monitor.component.ts:321` が使っている同パターン)。 加えて success path 側で `result.success` を分岐し、 `HTTP 200 + {success:false}` も `notify.error()` 経路に流す。

2. **Material 3 snackbar token**: `NotificationService` の `panelClass` (`.snackbar-{success,error,warning,info}`) は `--mdc-snackbar-container-color` (Material 2 legacy token) を設定していたが、 Material 3 の snackbar surface は `--mat-snack-bar-container-color` を読む → class は当たっているのに塗られず、 重大度に関わらず default gray (`#2f3033`)。 修正: 両 token を同時に設定 (M3 主、 MDC は埋め込みウィジェット / 将来 BC 用)、 supporting-text-color も同様。

ライブ確認 (localhost, PHA2 config → AMax HW): Apply ボタン押下で **赤 snackbar** が bottom-center に出て、 完全な "Reader rejected config: Firmware mismatch: digitizer at dig2://172.18.4.56 reports firmware 'DPP_OPEN' model 'VX2730' SN '52622' (mapped to AMax), but config declares firmware PHA2. Refusing to Apply — reload the correct config or update the source's `type` field, then re-Configure." + Dismiss action 表示。 `ng test --watch=false` 68/68 pass、 `ng build` clean、 `dist/` 再ビルド済 (CLAUDE.md "Frontend Deployment Policy" 遵守)。

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

### Live smoke (実機検証 — 完了 2026-05-07 localhost)

`91e3505` + `6911651` を入れた binary を localhost で `config_pha2_56_phys.toml` (PHA2 宣言) を AMax-FW デジタイザ (172.18.4.56, DPP_OPEN, VX2730 SN:52622) に対して `/start-daq` → UI で Apply 確認:

- ✅ 赤 snackbar が bottom-center に full diagnostic ("Reader rejected config: Firmware mismatch: ... mapped to AMax ... declares firmware PHA2 ...") + Dismiss action 付きで表示
- ✅ backend log の WARN 62 行 → 0 行、 "Failed to set parameter" 31 行 → 0 行、 ERROR 1 行 (full mismatch detail) のみ
- ✅ `system_state` は遷移せず (`auto_config_failed` で Arm が blocking)
- ✅ 正しい AMax config に差し替えて再 Configure → 通常通り成功 (regression なし)

## 関連

- [CLAUDE.md](../CLAUDE.md) "Silent failure を作らない" ポリシー強化事例
- [TODO/52](52_refactor_sprint_2026-q2.md) R-P4 検証中の live discovery
- 既存 silent failure 事案: 2026-05-04 `e641e99` (PHA2 mid-loop heuristic で波形 truncation)、 `e45e0ec` (case-insensitive cache miss が silent fallthrough)
- 2 行 shim refactor: 2026-05-06 R-P5 (digitizer route helpers) と同じ「single-purpose helper の合成」判断
