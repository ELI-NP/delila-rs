# Parameter Validation System — DevTree-Based

## Context

現在パラメータバリデーションが存在しない。UIで1ns刻みの入力が可能だが、ハードウェアは例えば8nsや16nsステップしか受け付けない。`apply_config()`はバリデーションなしで`set_value()`を呼び、FELibがエラーを返しても`warn!`で無視して続行する。さらに、同じFW（PSD1）でもフォームファクタによりパラメータが異なる（DT5730Bには`dt_ext_clock`があるがVX1730Bにはない）。

DevTreeは全メタデータを持っている: datatype, min, max, **increment（ステップ）**, allowed_values, setinrun, expuom, default_value。これを活用してバリデーションを実装する。

## Rounding Strategy: Round-to-Nearest（CoMPASS方式）

```
snapped = round((value - min) / increment) * increment + min
clamped = clamp(snapped, min, max)
```

| パラメータ | increment | 入力 101ns | 結果 |
|-----------|-----------|-----------|------|
| ch_trg_holdoff | 8 ns | 101 | **104** (nearest) |
| ch_pretrg | 8 ns | 41 | **40** (nearest) |
| ch_gate | 2 ns | 101 | **102** (nearest) |
| reclen | 16 ns | 1001 | **1008** (nearest) |

**リジェクトではなく補正＋通知。** CoMPASSと同じ挙動。物理的に方向性のある丸め（gate→ceil、delay→floor）は過剰複雑化（KISS違反）。

---

## Phase 1: Backend — ParamInfo拡張 + snap_to_step()

### 1a. `ParamInfo` にフィールド追加
**File:** `src/reader/caen/handle.rs` (L131-148)

```rust
pub struct ParamInfo {
    // 既存
    pub name: String,
    pub datatype: String,
    pub access_mode: String,
    pub setinrun: bool,
    pub min_value: Option<String>,
    pub max_value: Option<String>,
    pub allowed_values: Vec<String>,
    pub unit: Option<String>,
    // 追加
    pub increment: Option<String>,       // "8", "2", "0.1"
    pub default_value: Option<String>,   // "96"
    pub expuom: Option<i32>,             // -9 = nanoseconds
}
```

`Serialize`/`Deserialize` derive追加（REST APIで必要）。

### 1b. `extract_param_info()` 更新
**File:** `src/reader/caen/handle.rs` (L283-325)

`increment`, `defaultvalue`, `expuom` を DevTree JSONから取得。

### 1c. `validation.rs` 新規モジュール
**File:** `src/reader/caen/validation.rs` (新規)

```rust
/// CoMPASS方式: round-to-nearest + clamp
/// 浮動小数点精度に注意: value/increment で丸め、increment を掛け直す
pub fn snap_to_step(value: f64, min: f64, max: f64, increment: f64) -> f64 {
    if increment <= 0.0 {
        return value.clamp(min, max);
    }
    let steps = ((value - min) / increment).round();
    let snapped = steps * increment + min;
    snapped.clamp(min, max)
}

/// パラメータ値をDevTreeメタデータで検証・補正
pub fn validate_param(value: &str, info: &ParamInfo) -> ValidateResult
```

**浮動小数点精度対策（Geminiレビュー指摘）:**
- 大半のパラメータはinteger step（2, 8, 16）→ 結果を `i64` にキャストして整数文字列で返す
- DC offset等の float step（0.1）→ stepの小数桁数に合わせてフォーマット
- `value / increment` を使う（`value * (1/increment)` は精度が悪い）
- `(value - min)` は min がステップに整列している前提（DevTree実データで確認済み）

```rust
pub struct ValidateResult {
    pub value: String,       // 補正後の値（適切にフォーマット済み）
    pub adjusted: bool,      // 補正されたか
    pub message: Option<String>, // "101 → 104 (step=8)"
}
```

### 1d. テスト（TDD）
- `snap_to_step` の境界値テスト
- 浮動小数点エッジケース: `snap_to_step(50.3, 0.0, 100.0, 0.1)` → `50.3`（dirty floatにならないこと）
- 整数ステップ: `snap_to_step(101.0, 0.0, 524280.0, 8.0)` → `104.0` → 文字列 "104"
- min非ゼロ: `snap_to_step(41.0, 40.0, 2016.0, 8.0)` → `40.0` → 文字列 "40"
- 範囲外クランプ: `snap_to_step(-10.0, 0.0, 100.0, 1.0)` → `0.0`
- 実DevTree JSON (`docs/devtree_examples/`) からパラメータをパースしてバリデーションテスト

---

## Phase 2: Backend — DevTreeキャッシュ + apply_config改善

### 2a. `build_param_cache()` メソッド追加
**File:** `src/reader/caen/handle.rs`

```rust
/// DevTreeを一度だけ取得・パースし、HashMap<String, ParamInfo>に変換
pub fn build_param_cache(&self) -> Result<HashMap<String, ParamInfo>, CaenError>
```

DevTree内のboard-level(`/par/`)とchannel-level(`/ch/0/par/`)を再帰的に収集。同名パラメータは最初のhitを採用（min/max/incrementはチャンネル間で同一）。

### 2b. `DeviceConnection` にキャッシュ保持
**File:** `src/reader/mod.rs`

接続成功時に `build_param_cache()` を呼び、`DeviceConnection` に `param_cache: Option<HashMap<String, ParamInfo>>` として保持。失敗時は `None`（現行動作にフォールバック）。

### 2c. `apply_config_validated()` メソッド追加
**File:** `src/reader/caen/handle.rs`

```rust
pub fn apply_config_validated(
    &self,
    config: &DigitizerConfig,
    param_cache: &HashMap<String, ParamInfo>,
) -> Result<ApplyConfigResult, CaenError>
```

各パラメータに対して:
1. キャッシュから `ParamInfo` を引く
2. `validate_param()` で補正
3. `set_value()` で適用
4. 結果を `ParamApplyResult` として記録

### 2d. `ApplyConfigResult` 構造体
**File:** `src/reader/caen/validation.rs`

```rust
pub struct ApplyConfigResult {
    pub total: usize,
    pub ok: usize,
    pub adjusted: usize,     // 補正された数
    pub failed: usize,
    pub details: Vec<ParamApplyResult>,
}

pub struct ParamApplyResult {
    pub path: String,
    pub original_value: String,
    pub applied_value: String,
    pub status: ParamApplyStatus,  // Ok | Adjusted | Failed | Skipped
    pub message: Option<String>,
}
```

### 2e. Reader → Operator レスポンス拡張
**File:** `src/reader/mod.rs`, `src/operator/routes/digitizer.rs`

Apply APIのレスポンスに `ApplyConfigResult` を含める。フロントエンドが補正内容を表示可能に。

---

## Phase 3: Frontend — step属性 + on-blurスナップ

### 3a. `ChannelParamDef` に `step` フィールド追加
**File:** `web/operator-ui/src/app/components/channel-table/channel-table.component.ts`

```typescript
export interface ChannelParamDef {
  // 既存: key, label, type, options?, unit?, min?, max?, setInRun?
  step?: number;  // 追加: increment for snapping
}
```

### 3b. `channel-params.ts` に step値をハードコード
**File:** `web/operator-ui/src/app/models/channel-params.ts`

DevTreeから確認済みの値を設定:

```typescript
// PSD1 例
{ key: 'pre_trigger_ns', ..., min: 40, max: 2016, step: 8 },
{ key: 'gate_long_ns', ..., min: 4, max: 32766, step: 2 },
{ key: 'gate_short_ns', ..., min: 2, max: 2046, step: 2 },
{ key: 'gate_pre_ns', ..., min: 0, max: 510, step: 2 },
{ key: 'cfd_delay_ns', ..., min: 0, max: 510, step: 2 },
{ key: 'trigger_holdoff_ns', ..., min: 0, max: 524280, step: 8 },
{ key: 'dc_offset', ..., min: 0, max: 100, step: 0.1 },
```

### 3c. テンプレートに step + on-blur snapping
**File:** `web/operator-ui/src/app/components/channel-table/channel-table.component.ts`

```html
<input type="number" [step]="param.step ?? 'any'" (blur)="snapValue($event, param)">
```

```typescript
snapValue(event: Event, param: ChannelParamDef): void {
  if (!param.step || param.min == null) return;
  const input = event.target as HTMLInputElement;
  const value = Number(input.value);
  const snapped = Math.round((value - param.min) / param.step) * param.step + param.min;
  const clamped = Math.min(Math.max(snapped, param.min), param.max ?? Infinity);
  if (clamped !== value) input.value = String(clamped);
}
```

### 3d. Apply結果の通知表示
**File:** `web/operator-ui/src/app/components/digitizer-settings/digitizer-settings.component.ts`

Applyレスポンスに`adjusted > 0`があれば、どのパラメータが補正されたかスナックバーで通知。

---

## Phase 4 (将来): REST Schema Endpoint

**優先度低 — MVP後に実装**

- `GET /api/digitizers/:id/schema` — DevTreeメタデータをフロントエンドに公開
- `DigitizerService.loadSchema(id)` — ハードコード値を実機値で上書き
- フォームファクタ差異の自動対応（VMEにdt_ext_clockがないことを自動検出）

---

## Critical Files

| File | Changes |
|------|---------|
| `src/reader/caen/handle.rs` | ParamInfo拡張, extract_param_info更新, build_param_cache, apply_config_validated |
| `src/reader/caen/validation.rs` | **新規**: snap_to_step, validate_param, ApplyConfigResult |
| `src/reader/caen/mod.rs` | validation module追加 |
| `src/reader/mod.rs` | DeviceConnection.param_cache, ApplyConfig handler更新 |
| `src/operator/routes/digitizer.rs` | Apply レスポンス拡張 |
| `web/.../channel-table.component.ts` | ChannelParamDef.step, snapValue() |
| `web/.../channel-params.ts` | 全パラメータにstep値追加 |
| `web/.../digitizer-settings.component.ts` | 補正通知表示 |

## Gemini Review Summary

Geminiに最終レビューを依頼し、以下の指摘を受けて対応済み:

1. **浮動小数点精度** — `3 * 0.1 ≠ 0.3` 問題。整数ステップは `i64` キャスト、小数ステップは桁数制御で対処 → Phase 1c に反映
2. **Round-to-nearest vs 方向性丸め** — gate→ceil, delay→floor が理論的には安全だが、CoMPASS互換性とKISSを優先。FWが内部で安全制約を持っている → 現行方針維持
3. **Frontend/Backend乖離** — ハードコードstep値がDevTree実値と異なるリスク。Backend が authority なので問題なし → Phase 4 で動的schema対応予定
4. **Graceful degradation** — DevTree取得失敗時の無検証フォールバック。現行動作と同じなので許容 → 対応済み

## Verification

1. `cargo test` — snap_to_step境界値、浮動小数点エッジケース、DevTree JSONパース、validate_param
2. `cargo clippy -- -D warnings`
3. `ng build` (web/operator-ui/)
4. 実機テスト: DT5730B SN990でpre_trigger=101ns入力 → 104nsに補正されることを確認
5. 実機テスト: VX1730Bでdt_ext_clock設定がSkippedになることを確認（Phase 2完了後）
