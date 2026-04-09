# TODO #46: DIG1 Software Fine Timestamp (SW Fine TS)

**Status: PLANNED**
**Priority: Medium**
**Related: GitHub Issue #21 (Fine TS)**
**Created: 2026-04-09**

## 概要

DIG1 (x725/x730) の Fine Timestamp 精度を改善するため、FPGA の整数除算による HW Fine TS に加えて、
SAZC/SBZC 生データからソフトウェアで計算する SW Fine TS モードを実装する。

## 背景

### 問題
- DIG1 の HW Fine TS (10-bit) は FPGA 内部の截断整数除算で計算されるため、丸め誤差が発生
- Fine TS の分布が連続的ではあるが、理想的な一様分布からずれている
- DIG2 はさらに離散的（別問題、本 TODO のスコープ外）

### 原理
DIG1 DPP-PSD/PHA ファームウェアは、CFD (PSD) または RC-CR2 (PHA) 信号のゼロクロス前後のサンプル値を
EXTRAS ワードに出力できる（Extra option 0b101）。ソフトウェアで f64 除算すれば丸め誤差を排除できる。

```
HW Fine TS (現在):  FPGA が整数除算 → 截断誤差あり
SW Fine TS (提案):  SAZC/SBZC を f64 で除算 → 截断誤差なし
```

## 設計

### 1. モード切替

| モード | Extra Option | EXTRAS ワード内容 | 備考 |
|--------|-------------|------------------|------|
| HW Fine TS | 0b010 | ExtendedTS(16) + Flags(6) + FineTS(10) | 現在の実装 |
| SW Fine TS | 0b101 | SAZC(16) + SBZC(16) | 新規実装 |

**FELib 設定値:**
- PSD1 HW: `"EXTRAS_OPT_TT48_FLAGS_FINETT"` (現在)
- PSD1 SW: `"EXTRAS_OPT_SBZC_SAZC"`
- PHA1 HW: `"EXTRAS_OPT_TT48_FINETT"` (現在)
- PHA1 SW: `"EXTRAS_OPT_EBZC_EAZC"`

**注意:** FELib の allowed values index は PSD1 と PHA1 で異なるが、文字列ベースで設定するため問題なし。

### 2. Extended Timestamp の喪失と対策

SW Fine TS モードでは Extended Timestamp (16-bit) が失われる。

- HW モード: 47-bit TTT → ロールオーバー周期 ~70,000 秒
- SW モード: 31-bit TTT → ロールオーバー周期 **~4.29 秒** (2^31 × 2ns)

#### 二段構え方式: Board Aggregate Time Tag (Primary) + Host PC Time (Safety Net)

Board Aggregate Time Tag を主系、Host PC 時刻を安全網として併用する。

**Primary: Board Aggregate Time Tag（Gemini 確認済み）**
- Board Aggregate Time Tag と Event TTT は **同一の FPGA 内グローバルカウンタ** から取得される
- Board Aggregate Time Tag は **32-bit** (bit[31:0] すべて公開)
- Event TTT は **31-bit** (bit[31] は FORMAT ビットとしてハイジャックされている)
- Aggregate は内包するイベントより**常に後**に生成される → `board_time_tag >= max(TTT)` がハードウェアで保証
- 同一クロックドメインなので **USB/光リンクの遅延ジッタの影響を受けない**

**Safety Net: Host PC Time**
- Board Aggregate Time Tag の 32-bit ロールオーバー見逃し（RC2）を自動検出・自動修正
- 復元タイムスタンプと Host PC 時刻の大幅な乖離をサニティチェック
- 異常検出時はログ出力 + Host PC 時刻から正しい rollover_count を再計算

#### アルゴリズム

```rust
struct TimestampTracker {
    prev_board_time_tag: u32,
    board_rollover_count: u64,
    run_start_time: Instant,   // Host PC: Run 開始時刻
}

impl TimestampTracker {
    fn new() -> Self {
        Self {
            prev_board_time_tag: 0,
            board_rollover_count: 0,
            run_start_time: Instant::now(),
        }
    }
```

**Step 1: Board Aggregate Time Tag の 64-bit 拡張 + Host PC サニティチェック**

```rust
    /// Board Aggregate Time Tag (32-bit) を 64-bit に拡張
    /// host_now: この Aggregate を受信した Host PC 時刻
    fn update_board_time(&mut self, board_time_tag: u32, host_now: Instant) -> u64 {
        // Primary: 逐次比較による 32-bit ロールオーバー検出
        if board_time_tag < self.prev_board_time_tag {
            self.board_rollover_count += 1;
        }
        self.prev_board_time_tag = board_time_tag;
        let extended = (self.board_rollover_count << 32) | board_time_tag as u64;

        // Safety Net: Host PC 時刻とのサニティチェック
        let board_ns = (extended as f64) * TIME_STEP_NS;
        let host_ns = host_now.duration_since(self.run_start_time).as_nanos() as f64;
        let drift = (host_ns - board_ns).abs();

        const SANITY_THRESHOLD_NS: f64 = 2_000_000_000.0; // 2秒
        if drift > SANITY_THRESHOLD_NS {
            // Board Time Tag ロールオーバー見逃しを検出 → Host PC 時刻で修正
            let rollover_period_ns = (1u64 << 32) as f64 * TIME_STEP_NS;
            let board_tag_ns = (board_time_tag as f64) * TIME_STEP_NS;
            let corrected_rollovers =
                ((host_ns - board_tag_ns) / rollover_period_ns).round() as u64;

            warn!(
                "Board time drift detected: board={:.3}s host={:.3}s → \
                 correcting rollover_count {} -> {}",
                board_ns / 1e9, host_ns / 1e9,
                self.board_rollover_count, corrected_rollovers,
            );
            self.board_rollover_count = corrected_rollovers;
            return (corrected_rollovers << 32) | board_time_tag as u64;
        }

        extended
    }
```

**Step 2: Event TTT の完全復元**

31-bit TTT を 64-bit extended_board_time を基準にして復元する。

```rust
    /// 31-bit Event TTT を 64-bit に復元
    fn reconstruct_ttt(&self, extended_board_time: u64, ttt: u32) -> u64 {
        let ttt = ttt & 0x7FFF_FFFF;

        // Board Aggregate Time Tag の上位ビットを使って TTT の bit[31] 以上を復元
        // イベントは Aggregate より前に発生 → candidate <= extended_board_time
        let candidate = (extended_board_time & !0x7FFF_FFFF) | ttt as u64;

        if candidate > extended_board_time {
            // TTT が board_time_tag より大きい → 前の 31-bit エポックのイベント
            candidate - (1u64 << 31)
        } else {
            candidate
        }
    }
}
```

**二段構え方式の利点:**

| 観点 | Board Aggregate Time Tag (Primary) | Host PC Time (Safety Net) |
|------|-----------------------------------|---------------------------|
| クロック | 同一ドメイン（ジッタなし） | 異なるドメイン（USB 遅延あり） |
| 精度 | 2ns 単位で正確 | ~100ms のジッタ |
| ロールオーバー検出 | 逐次比較（8.59秒周期） | round() で無制限 |
| 弱点 | 8.59秒以上の無イベント | クロックドリフト |
| 役割 | **通常時の高精度復元** | **異常時の自動検出・自動修正** |

通常運用では Board Aggregate Time Tag のみが実質的に使われる。
Host PC Time は drift > 2秒を検出した場合のみ介入し、rollover_count を修正する。
ソフトウェアトリガ注入等の追加対策は不要。

#### レースコンディションと対策

**RC1: Aggregate 内のバッファ滞留 (>2.14秒)**

31-bit エポック間隔は ~4.29秒。イベントから Aggregate 封鎖までの最大許容時間は ~2.14秒。
低レート時に Aggregate がなかなか閉じないと、TTT 復元アルゴリズムが破綻する。

**対策:** Aggregate Timeout を ~100ms に設定する。
これにより、設定イベント数に達しなくても ~100ms で Aggregate が強制送信される。
（現在の実装で既に Aggregate Timeout は設定されているか要確認）

**RC2: 長時間無イベント (>8.59秒)**

32-bit board_time_tag のロールオーバーは ~8.59秒。非常に低レート（バックグラウンドのみ）の場合、
2つの Aggregate 間隔が 8.59秒を超えるとロールオーバーを見逃す。

**対策: Host PC Time Safety Net が自動的に検出・修正する。**
`update_board_time()` 内のサニティチェックで drift > 2秒を検出すると、
Host PC 時刻から正しい `rollover_count` を再計算して自動修正する。
warn ログで通知されるが、データ処理は中断しない。
実運用では核物理実験で 8.59秒間イベントゼロは極めて稀（ソースランでも通常 >10 Hz）。

### 3. SAZC/SBZC のエンコーディング（実機検証済み）

**PSD1 と PHA1 でエンコーディングが異なる。**

#### PSD1: 14-bit unsigned ADC 値（ゼロ = ADC midpoint 8192）

実測: upper=8200〜11000, lower=5000〜8100（両方正、8192 付近を挟む）

```rust
const ADC_MIDPOINT: f64 = 8192.0;

let before_zc = ((extras_word >> 16) & 0xFFFF) as f64; // bits[31:16], > 8192
let after_zc = (extras_word & 0xFFFF) as f64;          // bits[15:0],  < 8192
let denom = before_zc - after_zc;

let fine_fraction = if denom.abs() > f64::EPSILON {
    ((ADC_MIDPOINT - after_zc) / denom).clamp(0.0, 1.0)
} else {
    0.0
};
let fine_ns = fine_fraction * config.time_step_ns;
```

#### PHA1: 符号付き値（ゼロ中心）

RC-CR2 フィルタ出力は直接ゼロを跨ぐ。値が非常に小さい (0〜数 LSB)。

```rust
let before_zc = ((extras_word >> 16) & 0xFFFF) as u16 as i16 as f64; // 正
let after_zc = (extras_word & 0xFFFF) as u16 as i16 as f64;          // 負
let denom = before_zc - after_zc;

let fine_fraction = if denom.abs() > f64::EPSILON {
    (before_zc / denom).clamp(0.0, 1.0)
} else {
    0.0
};
let fine_ns = fine_fraction * config.time_step_ns;
```

#### 共通: ビット配置

| bits | 内容 | PSD1 での典型値 | PHA1 での典型値 |
|------|------|----------------|----------------|
| [31:16] | Before ZC | 8200〜11000 (unsigned) | 0〜+3 (signed) |
| [15:0] | After ZC | 5000〜8100 (unsigned) | -1〜-4 (signed) |

### 4. Interpolation Points レジスタ

CFD settings register (0x1n3C) bits[11:10] はゼロクロスの何番目のサンプルを使うかを制御。
HW Fine TS でも SW Fine TS でもこの設定が Fine TS の品質に直結する。
SW Fine TS では SAZC/SBZC として出力されるサンプルペアの選択に影響する。

| 値 | bits[11:10] | サンプル位置 | 特徴 |
|----|------------|-----------|------|
| 0 (デフォルト) | 00 | 1st before/after | 最高分解能、小信号ではノイズに敏感 |
| 1 | 01 | 2nd before/after | より安定、非線形性の影響あり |
| 2 | 10 | 3rd before/after | |
| 3 | 11 | 4th before/after | 最も安定、分解能は低下 |

#### FELib DevTree にパラメータが存在しない

DevTree には `ch_cfd_delay`, `ch_cfd_fraction`, `ch_cfd_smoothexp` の 3 つのみ。
Interpolation Points に対応する FELib パラメータは**存在しない**。
→ **レジスタ直接書き込み** (`set_user_register`) で設定する。

#### レジスタアドレス

```
CFD Settings Register: 0x1n3C (n = チャンネル番号)
  bits[7:0]   = CFD Delay (既に ch_cfd_delay で設定)
  bits[9:8]   = CFD Fraction (既に ch_cfd_fraction で設定)
  bits[11:10] = Interpolation Points ← これを追加
  bits[31:12] = Reserved

Broadcast: 0x803C (全チャンネル一括)
Channel 0: 0x103C, Channel 1: 0x113C, ..., Channel N: 0x1(N)3C
```

#### 実装方法: Read-Modify-Write

他のビット（Delay, Fraction）を壊さないよう、現在のレジスタ値を読んでから bits[11:10] だけ変更する。

```rust
// apply_config 内（ch_extras_opt 設定の後）
if let Some(interp_point) = config.channel_defaults.cfd_interpolation_point {
    for ch in 0..config.num_channels {
        let addr = 0x103C + (ch as u32) * 0x0100;
        // Read current value to preserve other bits
        let current = handle.get_user_register(addr)?;
        let new_val = (current & !0x0C00) | ((interp_point as u32 & 0x3) << 10);
        handle.set_user_register(addr, new_val)?;
        debug!(ch = ch, addr = format!("0x{:04X}", addr),
               value = interp_point, "Set CFD interpolation point");
    }
}
```

#### Config フィールド

`ChannelConfig` に追加:

```rust
/// CFD interpolation point for Fine TS calculation.
/// Controls which sample pair (before/after zero crossing) is used.
/// 0 = 1st (closest, highest resolution), 1 = 2nd, 2 = 3rd, 3 = 4th (most stable).
/// Applies to both HW and SW Fine TS modes. DIG1 (PSD1/PHA1) only.
/// NOTE: No FELib DevTree parameter exists — set via direct register write (0x1n3C bits[11:10]).
#[serde(skip_serializing_if = "Option::is_none")]
pub cfd_interpolation_point: Option<u8>,
```

TOML 設定例:
```toml
[digitizer.channel_defaults]
cfd_delay_ns = 6
cfd_fraction = "CFD_FRACTLIST_50"
cfd_interpolation_point = 0   # 0=1st, 1=2nd, 2=3rd, 3=4th
```

#### UI 定義

```typescript
{
    key: 'cfd_interpolation_point',
    label: 'CFD Interpolation Point',
    type: 'select',
    options: [
        { value: 0, label: '1st sample (highest resolution)' },
        { value: 1, label: '2nd sample' },
        { value: 2, label: '3rd sample' },
        { value: 3, label: '4th sample (most stable)' },
    ],
    default: 0,
    firmware: ['PSD1', 'PHA1'],
    tooltip: 'Which sample pair around CFD zero crossing to use for Fine TS interpolation',
}
```

### 5. LED モードとの互換性

#### 結論: LED モードでも SW Fine TS は動作する（条件付き）

**FPGA 内部の動作（Gemini 確認済み）：**

FPGA では LED フィルタと CFD フィルタが**常に並列動作**している。
`0x1n80` bit[6] (Discrimination Mode) は、トリガ判定に使うフィルタ出力を選択する MUX を切り替えるだけ。
Fine TS の計算ロジック（ゼロクロス内挿器）は **CFD データパスに固定配線** されている。

```
入力信号 ──→ [LED フィルタ] ──→ MUX ──→ トリガ判定
          ├→ [CFD フィルタ] ──→ MUX      (bit[6] で選択)
          └→ [CFD フィルタ] ──→ [ゼロクロス内挿器] ──→ SAZC/SBZC, Fine TS
                                    ↑ 常に動作（Discrimination Mode に依存しない）
```

したがって:
1. LED モードでイベントがトリガされると、FPGA は CFD パイプラインの出力を参照して
   LED トリガ近傍の CFD ゼロクロスを検出し、SAZC/SBZC を出力する
2. Extra option 0b010 (HW Fine TS) も LED モードで同様に CFD ベースで計算されている
3. **SW Fine TS と HW Fine TS の動作条件は同一**

#### ただし CFD パラメータの設定が必須

LED モードでは通常 CFD パラメータ（Delay, Fraction）を設定しない。
しかし CFD フィルタは常に動作しているため、パラメータが不適切だとゼロクロスが:
- 発生しない（SAZC/SBZC がゼロ）
- トリガ点と乖離する（タイミングが無意味）
- スロープが緩い（量子化が悪化）

**対策:**
- SW Fine TS 選択時、CFD パラメータ（delay, fraction）が設定されていない場合は **warn ログ** を出す
- LED モードでの SW Fine TS を禁止するのではなく、CFD パラメータ設定を促す
- PHA1 は RC-CR2 が常に動作するため、この問題はない

```rust
// Configure フェーズでのバリデーション
if config.fine_ts_mode == FineTsMode::Software && config.firmware == FirmwareType::PSD1 {
    if config.channel_defaults.discriminator_mode == Some("DISCR_MODE_LED".to_string()) {
        warn!(
            "SW Fine TS in LED mode: CFD filter still computes Fine TS, \
             but CFD parameters (delay, fraction) must be properly configured. \
             Verify ch_cfd_delay and ch_cfd_fraction are set."
        );
    }
}
```

### 6. 性能影響

- 7M events/10s (700 kHz) に対して、f64 除算は ~10ns/event
- 追加 CPU 負荷: ~7ms/秒 (**< 0.1%**)
- 完全に無視できるレベル

## 実装計画

### Step 1: Config 拡張
**ファイル:** `src/config/digitizer.rs`

```rust
/// Fine Timestamp 計算モード
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FineTsMode {
    /// FPGA 内蔵の 10-bit Fine TS を使用 (Extra option 0b010)
    Hardware,
    /// SAZC/SBZC からソフトウェアで計算 (Extra option 0b101)
    Software,
}

impl Default for FineTsMode {
    fn default() -> Self {
        FineTsMode::Hardware // 後方互換性
    }
}
```

`BoardConfig` に追加:
```rust
pub fine_ts_mode: Option<FineTsMode>,  // DIG1 のみ有効
```

### Step 2: レジスタ設定変更
**ファイル:** `src/reader/caen/handle.rs` (apply_config 内)

現在の `ch_extras_opt` 強制設定を `fine_ts_mode` に基づいて分岐:

```rust
if config.firmware.is_dig1() {
    let extras_value = match (config.firmware, config.fine_ts_mode) {
        (FirmwareType::PSD1, FineTsMode::Hardware) => "EXTRAS_OPT_TT48_FLAGS_FINETT",
        (FirmwareType::PSD1, FineTsMode::Software) => "EXTRAS_OPT_SBZC_SAZC",
        (FirmwareType::PHA1, FineTsMode::Hardware) => "EXTRAS_OPT_TT48_FINETT",
        (FirmwareType::PHA1, FineTsMode::Software) => "EXTRAS_OPT_EBZC_EAZC",
        _ => unreachable!(),
    };
    // ... per-channel 設定
}
```

### Step 3: TimestampTracker 実装
**ファイル:** `src/reader/decoder/psd1.rs`, `src/reader/decoder/pha1.rs`

Board Aggregate Time Tag + Host PC Time 二段構えの TTT 復元ロジック。
アルゴリズム詳細はセクション 2 を参照。`update_board_time()` に `Instant` を渡すため、
ReadLoop から Aggregate 受信時刻を `RawData` 経由で伝搬する必要がある。

```rust
pub struct RawData {
    pub data: Vec<u8>,
    pub n_events: u32,
    pub host_receive_time: Option<Instant>,  // Aggregate 受信時の Host PC 時刻
}
```

### Step 4: デコーダ拡張
**ファイル:** `src/reader/decoder/psd1.rs`, `src/reader/decoder/pha1.rs`

`decode_extras_word()` に新しい match arm を追加:

```rust
/// EXTRAS ワードのデコード結果
enum ExtrasResult {
    /// HW Fine TS (option 0b010)
    HwFineTs { extended_time: u16, fine_time: u16, flags: u32 },
    /// SW Fine TS: SAZC/SBZC (option 0b101)
    SwFineTs { sazc: i16, sbzc: i16 },
    /// その他
    Other { extended_time: u16 },
}

fn decode_extras_word(word: u32, extra_option: u8) -> ExtrasResult {
    match extra_option {
        2 => {
            let extended_time = ((word >> 16) & 0xFFFF) as u16;
            let flags = (word >> 10) & 0x3F;
            let fine_time = (word & 0x3FF) as u16;
            ExtrasResult::HwFineTs { extended_time, fine_time, flags }
        }
        5 => {
            // PSD1: SAZC[31:16], SBZC[15:0]
            // PHA1: EBZC[31:16], EAZC[15:0] (同一フォーマット)
            let sazc = ((word >> 16) & 0xFFFF) as u16 as i16;
            let sbzc = (word & 0xFFFF) as u16 as i16;
            ExtrasResult::SwFineTs { sazc, sbzc }
        }
        _ => {
            let extended_time = ((word >> 16) & 0xFFFF) as u16;
            ExtrasResult::Other { extended_time }
        }
    }
}
```

SW Fine TS のタイムスタンプ計算:

```rust
fn calculate_timestamp_sw(
    config: &Psd1Config,
    full_ttt: u64,  // TimestampTracker で復元済みの 64-bit TTT
    sazc: i16,
    sbzc: i16,
) -> f64 {
    let coarse_ns = (full_ttt as f64) * config.time_step_ns;

    let sbzc_f = sbzc as f64;
    let sazc_f = sazc as f64;
    let denom = sbzc_f - sazc_f;
    let fine_ns = if denom.abs() > f64::EPSILON {
        (sbzc_f / denom).clamp(0.0, 1.0) * config.time_step_ns
    } else {
        0.0
    };

    coarse_ns + fine_ns
}
```

### Step 5: フロントエンド UI
**ファイル:** `web/operator-ui/src/app/models/channel-params.ts` (または board-params)

DIG1 デジタイザの設定ページに Board レベルのトグルを追加:
```typescript
{
    key: 'fine_ts_mode',
    label: 'Fine TS Mode',
    type: 'select',
    options: [
        { value: 'hardware', label: 'HW (FPGA)' },
        { value: 'software', label: 'SW (SAZC/SBZC)' },
    ],
    default: 'hardware',
    firmware: ['PSD1', 'PHA1'],  // DIG1 のみ表示
    tooltip: 'Software mode uses raw CFD zero-crossing samples for higher precision Fine TS',
}
```

### Step 6: テスト

#### 6a. ユニットテスト（オフライン）
- `TimestampTracker::update_board_time()`: 32-bit ロールオーバー検出 + Host PC サニティチェック
- `TimestampTracker::reconstruct_ttt()`: 31-bit TTT → 64-bit 復元（境界ケース含む）
- `calculate_timestamp_sw()`: SAZC/SBZC → fine_ns 変換
  - 正パルス/負パルス
  - ゼロクロスなし (denom ≈ 0)
  - 境界値 (fraction = 0.0, 1.0)
- **PSD1/PHA1 ビット位置テスト**: Mock バイナリで PSD1 レイアウトと PHA1 レイアウトそれぞれ検証

#### 6b. 統合テスト（オフライン）
- HW/SW モード切替が正しくレジスタに反映されること
- PSD1 と PHA1 で異なる FELib 文字列が適用されること

#### 6c. 実機テスト: ビット位置検証（COMPLETED 2026-04-09）

**テスト bin:** `src/bin/fine_ts_verify.rs`
**対象:** VX1730B SN:69 (172.18.4.76, DPP-PSD FW, optical link0 conet2) ch8 パルサー入力

**結果サマリ:**

| Phase | extra_option | Events | 結果 |
|-------|-------------|--------|------|
| Phase 1 (HW) | 2 (0b010) | 2097 | Fine TS 一様分布、min=1 max=1023 mean=514.8 |
| Phase 2 (SW) | 5 (0b101) | 2097 | SAZC/SBZC 取得成功 |

**重要な発見:**

1. **SAZC/SBZC は i16（2の補数）ではなく、14-bit unsigned ADC 値**
   - ゼロクロスの「ゼロ」= ADC 中心値 8192
   - 実測値: upper=8200〜11000, lower=5000〜8100（両方正の値）
   - Fine TS 計算: `fraction = (8192 - lower) / (upper - lower)` or `(8192 - before) / (after - before)`

2. **PSD1 のビット配置（実測確認）:**
   - `bits[31:16]` (upper): 常に lower より大きい値 = **Before Zero Crossing**
   - `bits[15:0]` (lower): baseline に近い値 = **After Zero Crossing**
   - → **Interpretation B (upper=Before, lower=After) が正解**
   - → **PSD1 ドキュメント (UM4380) の記述 "upper=After, lower=Before" は誤り**
   - → 実際は PHA1 ドキュメントの記述と同一配置

3. **PHA1 の実測結果（V1725 SN:208, DPP-PHA FW）:**
   - PHA1 の EBZC/EAZC は **ゼロ中心の符号付き値**（8192 オフセットなし）
   - upper: 0〜2, lower: -1〜-4（非常に小さい値）
   - PSD1 とは異なるエンコーディング

4. **PSD1/PHA1 エンコーディングの違い:**

   | | bits[31:16] | bits[15:0] | 値の型 | ゼロ点 |
   |---|---|---|---|---|
   | **PSD1** | Before ZC | After ZC | 14-bit unsigned | 8192 (ADC midpoint) |
   | **PHA1** | Before ZC | After ZC | signed | 0 |

   **ビット配置は同一**（PSD1 doc の記述が誤り）。エンコーディング（符号/オフセット）が異なる。

#### 6d. 実機テスト: SW Fine TS 品質評価（TODO）
- ランダムソース（放射性崩壊）で HW vs SW Fine TS の分布比較
- Fine TS 分布の一様性を評価（DNL プロット）
- Interpolation Points (00/01/10/11) の効果比較

## 追加の注意事項（Gemini レビューで発見）

### A. PSD1/PHA1 ビット配置とエンコーディング（実機検証済み 2026-04-09）

EXTRAS ワード option 0b101 のビット配置は **PSD1/PHA1 で同一**:

| | bits[31:16] (upper) | bits[15:0] (lower) |
|---|---|---|
| **PSD1** | Before ZC | After ZC |
| **PHA1** | Before ZC | After ZC |

**CAEN PSD1 ドキュメント (UM4380) の記述 "upper=SAZC(After), lower=SBZC(Before)" は誤り。**
実機検証で upper が常に大きい値（= Before ZC）であることを確認。

ただし **エンコーディングが異なる**:

| FW | 値の型 | ゼロ点 | Fine TS 計算 |
|---|---|---|---|
| **PSD1** | 14-bit unsigned | 8192 (ADC midpoint) | `(8192 - lower) / (upper - lower)` |
| **PHA1** | signed (ゼロ中心) | 0 | `upper / (upper - lower)` |

デコーダの実装:

```rust
const ADC_MIDPOINT: f64 = 8192.0; // 14-bit ADC center

let upper = ((word >> 16) & 0xFFFF) as f64; // Before ZC (both FW)
let lower = (word & 0xFFFF) as f64;         // After ZC (both FW)

let fine_fraction = match firmware {
    FirmwareType::PSD1 => {
        // 14-bit unsigned, zero = 8192
        let denom = upper - lower;
        if denom.abs() > f64::EPSILON {
            ((ADC_MIDPOINT - lower) / denom).clamp(0.0, 1.0)
        } else { 0.0 }
    }
    FirmwareType::PHA1 => {
        // Signed, zero-centered (interpret as i16)
        let u = upper as u16 as i16 as f64;
        let l = lower as u16 as i16 as f64;
        let denom = u - l;
        if denom.abs() > f64::EPSILON {
            (u / denom).clamp(0.0, 1.0)
        } else { 0.0 }
    }
    _ => 0.0,
};
let fine_ns = fine_fraction * config.time_step_ns;
```

### B. Flags 喪失の詳細影響

SW Fine TS (option 0b101) では EXTRAS ワードの 6-bit Flags が失われる。
ただし **Pileup フラグは Charge ワードの bit[15] から来るため喪失しない**。

| フラグ | 喪失? | 影響 | 代替手段 |
|--------|-------|------|----------|
| Pileup | **保持** | なし | Charge word bit[15] |
| Trigger Lost | 喪失 | イベント単位の死時間補正不可 | Board 統計カウンタ（マクロ的） |
| Over-range | 喪失 | エネルギーキャリブレーションに影響 | Energy == MAX_ENERGY で検出 |
| 1024 Trigger | 喪失 | 統計カウンタのみ | Board 統計カウンタ |
| N Lost Trigger | 喪失 | 統計カウンタのみ | Board 統計カウンタ |

**対策:**
- Over-range: `energy == 0x7FFF` (15-bit max) or `energy == 0xFFFF` (16-bit max) でソフトウェア検出
- Trigger Lost: `remap_dig1_flags()` で SW mode 時はフラグ 0 を設定、統計的死時間補正に限定
- UI に「SW Fine TS では一部フラグが利用不可」の注記を表示

### C. `fine_time: u16` フィールドの扱い

`EventData.fine_time` は現在 ROOT 出力には直接使われていない
（ROOT は `timestamp_ns` → ピコ秒変換で `FineTS` ブランチを生成）。

SW Fine TS モードでは:
- `timestamp_ns` に SW 計算結果を格納（下流は自然に動作）
- `fine_time` には **10-bit 等価値** を格納（互換性のため）:
  ```rust
  let fine_time = (fine_fraction * 1024.0).clamp(0.0, 1023.0) as u16;
  ```
- Monitor の Fine TS ヒストグラムで HW vs SW の分布品質を直接比較可能

### D. x725 vs x730 クロック差

| モデル | サンプリング | time_step_ns | TTT ロールオーバー |
|--------|------------|-------------|-----------------|
| x725 | 250 MSa/s | 4 ns | ~8.59秒 (31-bit) |
| x730 | 500 MSa/s | 2 ns | ~4.29秒 (31-bit) |

`time_step_ns` は `Psd1Config` / `Pha1Config` から動的取得する（ハードコード禁止）。
TimestampTracker のロールオーバー計算にも `time_step_ns` を使用すること。

### E. データ版管理・Run メタデータ

SW Fine TS モードで取得されたデータを delila2root 等のオフラインツールが正しく
解釈するため、**Run メタデータに Fine TS モードを記録** する必要がある。

記録先候補:
- Recorder の Run ヘッダ（JSON メタデータ）
- ファイル名に suffix（例: `run0001_swfts.delila`）— 侵襲的、非推奨
- 既存の Run 設定 JSON ダンプ（`config/` に保存される run config のコピー）

最低限、以下を含める:
```json
{
  "fine_ts_mode": "software",
  "firmware": "PSD1",
  "cfd_interpolation_point": 0
}
```

## 一般注意事項

- **SW Fine TS は DIG1 専用**。DIG2 には適用不可（FW が SAZC/SBZC を出力しない）
- DIG2 の Fine TS 改善は別途 DNL 補正で対応（将来 TODO）
- LED モードでは CFD パラメータの設定が必須（セクション 5 参照）
- Aggregate Timeout を ~100ms に設定すること（RC1 対策）
- RC2（長時間無イベント）は Host PC Time Safety Net が自動検出・修正するため追加対策不要

## 参考文献

- CAEN UM4380: 725 and 730 DPP-PSD Register Description rev.6
  - CFD settings (p.14): bits[11:10] interpolation points
  - DPP Algorithm Control (p.27): bit[6] discrimination mode (LED/CFD)
  - DPP Algorithm Control 2 (p.29): bits[10:8] extras word options
- CAEN UM5678: 725-730 DPP-PHA Register Description
- `legacy/DELILA2/lib/digitizer/RefMaterials/PSD1_Data` (line 76-78: SAZC/SBZC format)
- `legacy/DELILA2/lib/digitizer/RefMaterials/PHA1_Data` (line 75-77: EBZC/EAZC format)
- `docs/devtree_examples/dt5730b_psd1_sn990.json` (ch_extras_opt allowed values)
- `docs/devtree_examples/dt5730b_pha1_sn990.json` (ch_extras_opt allowed values)
- Gemini Pro 分析 (2026-04-09):
  - FPGA 内部では LED/CFD フィルタが常に並列動作、Fine TS は常に CFD ベース
  - Board Aggregate Time Tag は Event TTT と同一クロック、32-bit (TTT は 31-bit)
  - board_time_tag >= max(TTT) はハードウェアで保証（因果律）
  - i16 符号解釈、性能評価 (< 0.1% CPU)
  - RC1: Aggregate 滞留対策、RC2: 長時間無イベント対策
