# Settings UI v2: カテゴリ再編 + SetInRun 対応

**Created:** 2026-02-04
**Status: ✅ 完了 (Phase 1-6)**
**Priority:** High — MVP 機能改善
**前提:** TODO/19 (Settings UI Phase 6) COMPLETED

---

## 目的

1. **カテゴリ再編:** Board / Frequent / Advanced → 機能別6カテゴリに再編成
2. **SetInRun 対応:** Running 中でも `SetInRun=true` のパラメーターを変更・適用可能にする
3. **パラメーター追加:** `docs/compass_devtree_mapping.md` の全パラメーターをUIに追加

---

## Phase 1: フロントエンド — パラメーター定義リファクタリング

### 1-1. `ChannelParamDef` に `setInRun` プロパティ追加

**ファイル:** `web/operator-ui/src/app/models/types.ts` (L13-28)

```typescript
export interface ChannelParamDef {
  key: string;
  label: string;
  type: 'number' | 'enum' | 'boolean';
  options?: string[];
  unit?: string;
  min?: number;
  max?: number;
  setInRun?: boolean;    // ← NEW: true = Running中でも変更可能
}
```

### 1-2. 6カテゴリのパラメーター定義を作成

**ファイル:** `web/operator-ui/src/app/components/digitizer-settings/digitizer-settings.component.ts`

既存の `PSD2_FREQUENT_PARAMS`, `PSD2_ADVANCED_PARAMS` 等 (L29-99) を以下に再編:

#### Board パラメーター（ボードレベル、channel-table不使用）

| FW | パラメーター |
|----|-------------|
| 共通 | Clock Source, Start Mode, Global Trigger Source, FPIO Type |
| 共通 | Test Pulse Period/Width, Start Delay |
| 共通 | TRG OUT Mode, GPO Mode, SyncOut Signal |
| 共通 | Board Veto Source/Polarity/Width |
| PSD1 | Ext Clock, Output Selection, Extras, Event Aggregation |
| PHA | Ext Clock, Output Selection, Extras, Event Aggregation |

#### Channel パラメーター（カテゴリ × FW で定義）

| カテゴリ | PSD2 | PSD1 | PHA |
|---------|------|------|-----|
| **Input** | Enable, Polarity, DC Offset, VGA Gain, Baseline Avg, Fixed Baseline, Record Length, Pre-trigger, Waveform Downsampling | Enable, Polarity, DC Offset, Input Dynamic, Baseline Mean, Fixed Baseline, Pre-trigger | Enable, Polarity, DC Offset, Coarse Gain, Baseline Mean, Pre-trigger |
| **Trigger** | Discriminator Mode, Threshold, CFD Delay, CFD Fraction, Trigger Holdoff, Smoothing Factor, Time Filter Smoothing, Event Trigger Source, Wave Trigger Source | Discriminator Mode, Threshold, CFD Delay, CFD Fraction, Input Smoothing, Trigger Holdoff, Self Trigger, Global Trigger Gen, Trigger Output Propagate | Threshold, Trigger Holdoff, Fast Discr Smoothing, Input Rise Time, Self Trigger, Global Trigger Gen, Trigger Output Propagate |
| **Energy** | Energy Coarse Gain, Gate Long, Gate Short, Pre-gate, Charge Pedestal, Short Charge Pedestal, Charge Smoothing | Energy Coarse Gain, Gate Long, Gate Short, Pre-gate, Charge Pedestal Enable | Trap Rise Time, Trap Flat Top, Trap Pole Zero, Peaking Time, N Samples Peak, Peak Holdoff, Energy Fine Gain |
| **Coincidence** | Ch Trigger Mask, Coincidence Mask, Anti-coincidence Mask, Coincidence Window, Veto Source (ch), Veto Width (ch), Event Selector | Coincidence Mode, Veto Source, Pileup Rejection | Coincidence Mode, Veto Source |
| **Waveform** | Wave Saving, Analog Probe 0/1, Digital Probe 0/1/2/3 | (Waveforms は Board で管理) | Wave Trigger, Analog/Digital Probes |

**実装方針:**
- FW別カテゴリ定数を定義: `PSD2_INPUT_PARAMS`, `PSD2_TRIGGER_PARAMS`, ... 等
- `getCategoryParams(fw: FirmwareType, category: string): ChannelParamDef[]` 関数
- 各パラメーターに `setInRun` フラグを `docs/compass_devtree_mapping.md` から転記

### 1-3. `ChannelConfig` / `BoardConfig` にフィールド追加

**ファイル:** `web/operator-ui/src/app/models/types.ts` (L120-144)

新規パラメーターに対応するフィールドを追加。
`extra?: Record<string, unknown>` を活用するか、明示的フィールドにするか設計判断が必要。

**判断:** FW共通パラメーターは明示的フィールド、FW固有は `extra` に格納する既存方針を維持。

---

## Phase 2: フロントエンド — タブ UI 再構成

### 2-1. 6タブ構成に変更

**ファイル:** `digitizer-settings.component.ts` テンプレート (L203-345)

```
┌──────────────────────────────────────────────────────────────┐
│ Digitizer: [LaBr3-001 (PSD2) ▼]  Name: [___]  [Detect] [Apply] [Save] │
├──────────────────────────────────────────────────────────────┤
│ [Board] [Input] [Trigger] [Energy] [Coincidence] [Waveform]  │
├──────────────────────────────────────────────────────────────┤
│                                                               │
│              ← 選択中タブのコンテンツ →                       │
│                                                               │
└──────────────────────────────────────────────────────────────┘
```

- **Board タブ:** フォームグリッド（現行と同構造、パラメーター追加）
- **Input〜Waveform タブ:** 各々 `<app-channel-table>` を使用
- パラメーターが0個のカテゴリタブは非表示（例: PSD1 で Waveform Probes がない場合）

### 2-2. Board タブのパラメーター追加

現行: Start Source, Global Trigger Source, Test Pulse Period/Width, Record Length, Waveforms, Probes
追加: Clock Source, Output Clock, SyncOut, Start Delay, TRG OUT Mode, GPO Mode, FPIO Type, Board Veto Source/Polarity/Width

FW依存セクションの条件付き表示を維持。

---

## Phase 3: バックエンド — SetInRun 対応

### 3-1. 状態マシン変更: Running から ApplyDigitizerConfig を許可

**ファイル:** `src/common/command.rs` (L77-83)

```rust
// Before:
ComponentState::Running => vec!["Stop", "GetStatus"],

// After:
ComponentState::Running => vec!["Stop", "GetStatus", "ApplyDigitizerConfig"],
```

### 3-2. 状態マシンハンドラ: Running時の制約追加

**ファイル:** `src/common/state.rs` (L325-356)

```rust
Command::ApplyDigitizerConfig(config) => {
    let current = self.state();
    match current {
        ComponentState::Idle | ComponentState::Configured => {
            // 既存ロジック: 全パラメーター適用
            match self.on_apply_digitizer_config(&config) { ... }
        }
        ComponentState::Running => {
            // NEW: SetInRun パラメーターのみ適用
            match self.on_apply_digitizer_config_running(&config) { ... }
        }
        _ => {
            CommandResponse::error(current, "ApplyDigitizerConfig not available")
        }
    }
}
```

### 3-3. Reader: `on_apply_digitizer_config_running()` の実装

**ファイル:** `src/reader/mod.rs` (または `src/reader/caen/`)

新しいトレイトメソッド:
```rust
fn on_apply_digitizer_config_running(&mut self, config: &DigitizerConfig) -> Result<usize, String> {
    // SetInRun=true のパラメーターのみ FELib に書き込む
    // 適用したパラメーター数を返す
}
```

**SetInRun パラメーターリスト:**
SetInRun=true のパラメーターをRust側に定義する。`docs/compass_devtree_mapping.md` の SetInRun 列を参照。

```rust
// PSD2 の SetInRun=true パラメーター
const PSD2_SET_IN_RUN_PARAMS: &[&str] = &[
    "chenable", "chpretriggert", "absolutebaseline", "dcoffset",
    "chgain", "triggerthr", "smoothingfactor", "chargesmoothing",
    "timefiltersmoothing", "longchargeintegratorpedestal",
    "shortchargeintegratorpedestal", "channelvetosource", "adcvetowidth",
    "channelstriggermask", "coincidencemask", "anticoincidencemask",
    "coincidencelengtht", "eventselector", "eventtriggersource",
    "wavetriggersource", "wavesaving", "waveanalogprobe0",
    "waveanalogprobe1", "wavedigitalprobe0", "wavedigitalprobe1",
    "wavedigitalprobe2", "wavedigitalprobe3",
    // Board
    "testpulseperiod", "testpulsewidth", "syncoutmode",
    "boardvetosource", "boardvetopolarity", "boardvetowidth",
];
```

### 3-4. Rust DigitizerConfig にフィールド追加

**ファイル:** `src/config/digitizer.rs`

UI で追加した新パラメーターに対応するフィールドを `ChannelConfig` / `BoardConfig` に追加。
FELib apply ロジック (`apply_channel_params`, `apply_board_params`) に新パラメーターを追加。

---

## Phase 4: フロントエンド — SetInRun UI 対応

### 4-1. システム状態の取得

**ファイル:** `digitizer-settings.component.ts`

Operator API (`GET /api/status`) からシステム状態を取得し、コンポーネントに注入。

```typescript
systemState = signal<string>('Idle');  // 'Idle' | 'Running' | etc.
isRunning = computed(() => this.systemState() === 'Running');
```

定期ポーリング (既存の StatusService を利用) または WebSocket で状態を監視。

### 4-2. channel-table に disabled 制御を追加

**ファイル:** `channel-table.component.ts` (L70-170 テンプレート)

```typescript
// 新 Input
@Input() disabled = false;        // 全体無効化
@Input() disabledKeys: string[] = []; // 特定キーのみ無効化

isDisabled(key: string): boolean {
  return this.disabled || this.disabledKeys.includes(key);
}
```

テンプレート側: `[disabled]="isDisabled(param.key)"` を各 input/select に追加。
無効化されたセルはグレーアウト表示（CSS: `opacity: 0.5; pointer-events: none;`）。

### 4-3. digitizer-settings でカテゴリ毎に disabled キーを計算

```typescript
disabledKeys = computed(() => {
  if (!this.isRunning()) return [];
  // SetInRun=false のパラメーターキーをリストアップ
  return this.allParams()
    .filter(p => !p.setInRun)
    .map(p => p.key);
});
```

### 4-4. Board タブの SetInRun 対応

Board タブのフォームフィールドにも同様の disabled 制御を追加。
Running 中は `setInRun=false` のフィールドをグレーアウト。

### 4-5. Apply ボタンの状態表示

Running 中の Apply ボタン:
- ラベルを「Apply (Runtime)」に変更
- ツールチップ: "SetInRun パラメーターのみ適用されます"
- 適用後のレスポンスで適用されたパラメーター数を表示

---

## Phase 5: 結合テスト

### 5-1. フロントエンドテスト
- [ ] 6タブ表示: 各FW (PSD2/PSD1/PHA) で正しいパラメーターが表示される
- [ ] 空カテゴリの非表示確認
- [ ] Idle時: 全パラメーター編集可能
- [ ] Running時: SetInRun=false がグレーアウト、SetInRun=true が編集可能
- [ ] Apply → パラメーター適用 → 成功表示

### 5-2. バックエンドテスト
- [ ] Idle/Configured → ApplyDigitizerConfig: 全パラメーター適用（既存動作維持）
- [ ] Running → ApplyDigitizerConfig: SetInRun パラメーターのみ適用
- [ ] Running → 非SetInRun パラメーター変更 → スキップ（エラーではない）
- [ ] 適用パラメーター数が正しく返される

### 5-3. 実機テスト
- [ ] VX2730 (PSD2): Running 中に Threshold, DC Offset 変更 → 即反映
- [ ] DT5730B (PSD1): Running 中に Threshold 変更 → 即反映

---

## 変更ファイル一覧

| Phase | Action | File | Description |
|-------|--------|------|-------------|
| 1 | Modify | `web/.../models/types.ts` | `ChannelParamDef` に `setInRun` 追加、Config フィールド追加 |
| 1 | Modify | `web/.../digitizer-settings/digitizer-settings.component.ts` | パラメーター定義を6カテゴリに再編 |
| 2 | Modify | `web/.../digitizer-settings/digitizer-settings.component.ts` | テンプレート: 6タブ構成 |
| 2 | Modify | `web/.../digitizer-settings/digitizer-settings.component.ts` | Board タブにパラメーター追加 |
| 3 | Modify | `src/common/command.rs` | Running 状態の valid_commands に ApplyDigitizerConfig 追加 |
| 3 | Modify | `src/common/state.rs` | Running 時の ApplyDigitizerConfig ハンドラ分岐 |
| 3 | Modify | `src/reader/mod.rs` (or caen/) | `on_apply_digitizer_config_running()` 実装 |
| 3 | Modify | `src/config/digitizer.rs` | 新パラメーターフィールド追加 |
| 4 | Modify | `web/.../channel-table/channel-table.component.ts` | disabled 制御追加 |
| 4 | Modify | `web/.../digitizer-settings/digitizer-settings.component.ts` | システム状態取得、disabledKeys 計算 |

---

## 実装順序

```
Phase 1 (パラメーター定義) ← 最初にやる。以降の全フェーズの基盤
    ↓
Phase 2 (タブ UI 再構成) ← Phase 1 完了後すぐ。UI で動作確認可能になる
    ↓
Phase 3 (バックエンド SetInRun) ← UI と並行可能だがテストに Phase 2 が必要
    ↓
Phase 4 (フロントエンド SetInRun) ← Phase 2 + 3 の両方が必要
    ↓
Phase 5 (結合テスト) ← 全フェーズ完了後
```

---

## 設計判断

1. **6カテゴリ = CoMPASS 準拠:** Board / Input / Trigger / Energy / Coincidence / Waveform。物理屋に馴染みやすい機能別分類。
2. **SetInRun は UI + Backend 両方で制御:** UIで防止 + Backend でフィルタリングの二重保護。
3. **新コマンドは作らない:** 既存の `ApplyDigitizerConfig` を Running 状態にも拡張。KISS 原則。
4. **SetInRun=false のパラメーターは静かにスキップ:** Running 中に全設定を Apply しても、SetInRun=false はエラーではなくスキップ。適用数をレスポンスで返す。
5. **パラメーターリストは静的定義:** `compass_devtree_mapping.md` の情報をTypeScript/Rustに転記。DevTree動的UIは将来。

### 既知の制限 (Phase 1-4)

- **Board タブの SetInRun disabled 制御が未実装:** Channel タブは `disabledKeys` で Running 中の非 SetInRun パラメーターをグレーアウトするが、Board タブのフォームフィールド (`start_source`, `global_trigger_source`, `clock_source` 等) には `[disabled]` バインディングがない。Backend 側は正しくフィルタリングするため実害はないが、UI の一貫性として改善の余地あり。

---

## Phase 6: PSD1/PHA1 時間単位変換 (ns ↔ samples)

**Status:** ✅ 完了
**前提:** Phase 1-4 完了

### 背景

- PSD2: FELib DevTree が `t` suffix パラメーター (ns 単位) を提供 → 変換不要
- PSD1/PHA1: DevTree は samples 単位 (`ch_gate`, `ch_cfd_delay` 等)
- **CoMPASS は PSD1/PHA1 でも ns 表示** → ユーザーは ns で値を入力・確認することに慣れている
- サンプリングレート: 500 MS/s → 1 sample = 2 ns (`time_step_ns = 2.0`)

### 方針: Option 1 — Backend conversion at apply time

Config は常に ns で保存。Backend の `to_caen_parameters()` で PSD1/PHA1 の時間系パラメーターのみ
`ns ÷ time_step_ns` → samples に変換して DevTree に送信する。

**理由:**
- CoMPASS と同じ表示 (ns) でユーザー体験が統一される
- Config ファイルの値が物理量として人間に読みやすい
- `_ns` suffix の config key 命名が正確になる（全FWでns）
- 将来別サンプリングレートのデジタイザを追加しても config 変更不要

### 6-1. Frontend: PSD1/PHA1 パラメーター定義を ns に統一

**ファイル:** `web/.../digitizer-settings/digitizer-settings.component.ts`

変更対象パラメーター（PSD1/PHA1 の `unit: 'samples'` → `unit: 'ns'`、min/max を ×2）:

| Config Key | PSD1 現在 | PSD1 変更後 | PHA1 現在 | PHA1 変更後 |
|---|---|---|---|---|
| `pre_trigger` → `pre_trigger_ns` | 40-2016 samples | 80-4032 ns | 64-2000 samples | 128-4000 ns |
| `cfd_delay_ns` | 0-510 samples | 0-1020 ns | — | — |
| `trigger_holdoff` → `trigger_holdoff_ns` | 0-524280 samples | 0-1048560 ns | 8-8184 samples | 16-16368 ns |
| `gate_long_ns` | 4-32766 samples | 8-65532 ns | — | — |
| `gate_short_ns` | 2-2046 samples | 4-4092 ns | — | — |
| `gate_pre_ns` | 0-510 samples | 0-1020 ns | — | — |
| `input_rise_time` → `input_rise_time_ns` | — | — | 16-2040 samples | 32-4080 ns |
| `trap_rise_time` → `trap_rise_time_ns` | — | — | 8-32760 samples | 16-65520 ns |
| `trap_flat_top` → `trap_flat_top_ns` | — | — | 8-8184 samples | 16-16368 ns |
| `trap_pole_zero` → `trap_pole_zero_ns` | — | — | 8-524280 samples | 16-1048560 ns |
| `peak_holdoff` → `peak_holdoff_ns` | — | — | 8-8184 samples | 16-16368 ns |

- PSD1/PHA1 の config key を `_ns` suffix に統一（`pre_trigger` → `pre_trigger_ns` 等）
- PSD2 の既存 `_ns` key はそのまま

### 6-2. Frontend: ChannelConfig 型統一

**ファイル:** `web/.../models/types.ts`

- 重複フィールドを統一: `pre_trigger` + `pre_trigger_ns` → `pre_trigger_ns` のみ
- 同様に `trigger_holdoff` + `trigger_holdoff_ns` → `trigger_holdoff_ns` のみ
- PHA1 固有パラメーターに `_ns` suffix 追加

### 6-3. Backend: `to_caen_parameters()` に ns→samples 変換追加

**ファイル:** `src/config/digitizer.rs`

```rust
impl DigitizerConfig {
    /// Convert ns config values to samples for PSD1/PHA1 DevTree
    fn ns_to_samples(ns_value: f64, time_step_ns: f64) -> u64 {
        (ns_value / time_step_ns).round() as u64
    }

    pub fn to_caen_parameters(&self) -> Vec<(String, String)> {
        // ... existing logic ...
        // For PSD1/PHA1 time params:
        // config stores ns → convert to samples before writing to DevTree
        match self.firmware {
            FirmwareType::PSD1 | FirmwareType::PHA1 => {
                // gate_long_ns (ns) → ch_gate (samples)
                let samples = Self::ns_to_samples(gate_long_ns, 2.0);
                params.push((path, samples.to_string()));
            }
            FirmwareType::PSD2 => {
                // Already ns, DevTree has `t` suffix → pass through
                params.push((path, gate_long_ns.to_string()));
            }
        }
    }
}
```

変換対象パラメーター (PSD1):
- `gate_long_ns` → `ch_gate` (÷2)
- `gate_short_ns` → `ch_gateshort` (÷2)
- `gate_pre_ns` → `ch_gatepre` (÷2)
- `cfd_delay_ns` → `ch_cfd_delay` (÷2)
- `trigger_holdoff_ns` → `ch_trg_holdoff` (÷2)
- `pre_trigger_ns` → `ch_pretrg` (÷2)

変換対象パラメーター (PHA1):
- `trigger_holdoff_ns` → `ch_trg_holdoff` (÷2)
- `pre_trigger_ns` → `ch_pretrg` (÷2)
- `input_rise_time_ns` → `ch_rccr2_rise` (÷2)
- `trap_rise_time_ns` → `ch_trap_trise` (÷2)
- `trap_flat_top_ns` → `ch_trap_tflat` (÷2)
- `trap_pole_zero_ns` → `ch_tdecay` (÷2)
- `peak_holdoff_ns` → `ch_peak_holdoff` (÷2)

Board-level (PSD1/PHA1 共通):
- `record_length` (Board) → `reclen` (÷2)
- `start_delay` (Board, extra) → `start_delay` (÷2)
- `coinc_trgout` (Board, extra) → `coinc_trgout` (÷2)

### 6-4. Backend: `time_step_ns` の取得

`DeviceInfo.sampling_rate_sps` (500000000) から算出:
```rust
let time_step_ns = 1_000_000_000.0 / sampling_rate_sps as f64; // = 2.0
```

現状は `time_step_ns: 2.0` がデコーダ側でハードコードされている。
`to_caen_parameters()` でも同じ値を使用。将来的に `DeviceInfo` から動的取得に変更可能。

### 6-5. 既存 config ファイルのマイグレーション

PSD1/PHA1 の既存 config JSON で samples 値が入っているファイルを ns に変換する必要がある。
- 手動変換: 各値 × 2
- または初回ロード時に自動マイグレーション（config version フィールドで判定）

---

## 変更ファイル一覧 (Phase 6 追加)

| Phase | Action | File | Description |
|-------|--------|------|-------------|
| 6 | Modify | `web/.../digitizer-settings/digitizer-settings.component.ts` | PSD1/PHA1 パラメーターを ns 表示に変更 |
| 6 | Modify | `web/.../models/types.ts` | ChannelConfig フィールド統一 (_ns suffix) |
| 6 | Modify | `src/config/digitizer.rs` | `to_caen_parameters()` に ns→samples 変換追加 |
| 6 | Modify | PSD1/PHA1 config JSON files | 既存 samples 値を ns に変換 |
