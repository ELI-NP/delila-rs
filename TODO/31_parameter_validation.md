# TODO #31: Parameter Validation System — DevTree-Based

**Created:** 2026-02-13
**Status: ✅ 完了 (Phase 1-3)**
**Priority:** 2

## Overview

DevTreeメタデータ（min/max/increment/allowed_values）を活用したパラメータバリデーション。
UIの1ns刻み入力をハードウェアのステップ制約（2/8/16ns等）に自動補正する。

## Background

- 現在バリデーションなし — `set_value()`にそのまま渡し、FELibエラーはwarnで無視
- DevTreeは全メタデータを持っている（接続時に取得可能）
- 同じFW（PSD1）でもフォームファクタで利用可能パラメータが異なる
  - DT5730B (Desktop): `dt_ext_clock` あり
  - VX1730B (VME): `dt_ext_clock` なし（2026-02-13 実機で確認済み）

## Design

### Rounding: Round-to-Nearest（CoMPASS方式）

```
snapped = round((value - min) / increment) * increment + min
clamped = clamp(snapped, min, max)
```

リジェクトではなく補正＋通知。Geminiレビューで方向性丸め（gate→ceil等）も検討したが、
CoMPASS互換性 + KISS優先で round-to-nearest に統一。

### Floating Point Precision

- 整数ステップ（2, 8, 16）→ i64キャストで整数文字列出力
- 小数ステップ（0.1 = DC offset）→ stepの小数桁数でフォーマット
- `value / increment` を使用（`value * (1/increment)` は精度劣化）

## Phases

### Phase 1: Backend Foundation
- [x] ← TODO: ParamInfo拡張（increment, default_value, expuom）
- [x] ← TODO: extract_param_info()更新
- [x] ← TODO: validation.rs新規モジュール（snap_to_step, validate_param）
- [x] ← TODO: ユニットテスト

### Phase 2: Backend Integration
- [x] build_param_cache() — DevTree→HashMap<String, ParamInfo>
- [x] DeviceConnection.param_cache
- [x] apply_config_validated() + apply_config_running_validated()
- [x] ApplyConfigResult構造体 (validation.rsに定義済み)
- [x] ReadLoop統合（cache利用時はvalidated版、なければ従来版にフォールバック）
- [ ] REST APIレスポンス拡張（Phase 4と同時に実装予定）

### Phase 3: Frontend
- [x] ChannelParamDef.step フィールド
- [x] channel-params.ts step値ハードコード（全3 FW × 全数値パラメータ）
- [x] on-blur snapping in channel-table (snapValue)
- [x] HTML [step] 属性（ブラウザのスピナーも正しいステップ刻みに）
- [ ] Apply結果通知（Phase 4のREST拡張後に実装予定）

### Phase 4 (将来): Dynamic Schema
- [ ] GET /api/digitizers/:id/schema
- [ ] フロントエンドでDevTree値を動的ロード

## Key Files

| File | Changes |
|------|---------|
| `src/reader/caen/handle.rs` | ParamInfo拡張, build_param_cache, apply_config_validated |
| `src/reader/caen/validation.rs` | **新規**: snap_to_step, validate_param |
| `src/reader/mod.rs` | DeviceConnection.param_cache |
| `src/operator/routes/digitizer.rs` | Apply レスポンス拡張 |
| `web/.../channel-table.component.ts` | step属性, snapValue() |
| `web/.../channel-params.ts` | step値追加 |

## Design Document

Full design: `docs/plans/parameter_validation.md`

## Verification

1. `cargo test` — snap_to_step, DevTree parse, validate_param
2. `cargo clippy -- -D warnings`
3. `ng build`
4. 実機: DT5730B SN990 で pre_trigger=101ns → 104ns 補正確認
5. 実機: VX1730B で dt_ext_clock Skipped 確認
