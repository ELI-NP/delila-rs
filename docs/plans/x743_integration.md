# x743 (SAMLONG) デジタイザ統合プラン

**作成日:** 2026-02-19
**ステータス:** 計画中
**優先度:** MVP 後（3月中旬以降）

---

## 1. 概要

CAEN x743 シリーズ（V1743, VX1743, DT5743）を delila-rs に統合する。
x743 は SAMLONG スイッチドキャパシタ方式の波形デジタイザで、既存の FADC ベース（x730/x725）とは根本的に異なるアーキテクチャを持つ。

### x743 の主要特性

| 項目 | x743 | 既存 (x730/x725) |
|------|-------|-------------------|
| ADC 方式 | スイッチドキャパシタ (SAMLONG) | Flash ADC |
| 分解能 | 12 bit | 14 bit |
| サンプリング | 3.2 / 1.6 / 0.8 / 0.4 GHz | 250 / 500 MS/s |
| チャンネル構造 | 2ch/group (グループベース) | 個別チャンネル |
| DPP FW | なし (Normal + Charge Mode) | PSD1/PSD2/PHA1 |
| ライブラリ | CAENDigitizer Library (旧) | FELib (新) |
| 最大レート | ~7 kHz (デッドタイム 125μs) | 数百 kHz〜 |
| キャリブレーション | SAMLONG 補正必須 | 不要 |

### 2つの取得モード

| モード | API値 | 内容 | 出力データ |
|--------|-------|------|-----------|
| **STANDARD** | `AcquisitionMode_STANDARD (0)` | デジタルオシロスコープ | フル波形 (ADC サンプル配列) |
| **DPP_CI** | `AcquisitionMode_DPP_CI (1)` | 電荷積分 | Charge (pC), Peak, Baseline, TDC |

- STANDARD モード: トリガー検出・エネルギー計算・タイミングはすべて**ソフトウェア処理**
- DPP_CI モード: charge/peak/baseline/TDC は FW 計算済み、波形データなし

---

## 2. 設計方針

### 2.1 FirmwareType 拡張

```rust
pub enum FirmwareType {
    PSD1,
    PSD2,
    PHA1,
    AMax,
    X743CI,   // x743 Charge Integration モード
    X743Std,  // x743 Standard (波形) モード
}
```

**理由:** DPP_CI と STANDARD ではデータ構造・デコード処理・パイプライン動作が大きく異なるため、
FirmwareType レベルで分離する。`is_dig1()` 等のメソッドに `is_legacy_api()` を追加。

```rust
impl FirmwareType {
    /// CAENDigitizer Library を使用するか (FELib ではなく)
    pub fn is_legacy_api(&self) -> bool {
        matches!(self, Self::X743CI | Self::X743Std)
    }

    /// グループベースのチャンネル構造か
    pub fn is_group_based(&self) -> bool {
        matches!(self, Self::X743CI | Self::X743Std)
    }
}
```

### 2.2 FFI 戦略: bindgen による自動生成

| 方式 | 採用 | 理由 |
|------|------|------|
| bindgen 自動生成 | **採用** | `CAEN_DGTZ_X743_EVENT_t` 等の複雑な構造体を手書きするとメモリレイアウトのバグリスク大 |
| 手書き FFI | 不採用 | 関数 ~30 + 構造体/enum 多数、保守コスト高 |

```
src/reader/caen_legacy/
├── mod.rs          # re-exports, safe wrapper API
├── ffi.rs          # bindgen 生成コード (build.rs で自動生成)
├── handle.rs       # X743Handle (RAII wrapper)
├── error.rs        # CAENDigitizer エラーコード変換
└── types.rs        # Rust-friendly 型定義
```

**build.rs:**
```rust
#[cfg(feature = "x743")]
fn generate_caen_digitizer_bindings() {
    let bindings = bindgen::Builder::default()
        .header("/usr/include/CAENDigitizer.h")
        .allowlist_function("CAEN_DGTZ_.*")
        .allowlist_type("CAEN_DGTZ_.*")
        .allowlist_var("CAEN_DGTZ_.*")
        .generate()
        .expect("Unable to generate CAENDigitizer bindings");
    // ...
}
```

### 2.3 コンパイル戦略: Feature Flag `x743`

```toml
[features]
default = []
root = ["oxyroot"]
x743 = ["bindgen"]  # CAENDigitizer Library が必要
```

- **macOS 開発:** `cargo build` (デフォルト) — x743 コード除外、ビルド通る
- **Linux 実機:** `cargo build --features x743` — CAENDigitizer Library リンク
- CI: macOS ではデフォルト、Linux でも `x743` feature はオプション（実機ライブラリ不要の場合）

条件コンパイル:
```rust
#[cfg(feature = "x743")]
pub mod caen_legacy;

// DecoderKind enum
pub enum DecoderKind {
    Psd2(Psd2Decoder),
    Psd1(Psd1Decoder),
    Pha1(Pha1Decoder),
    AMax(AMaxDecoder),
    #[cfg(feature = "x743")]
    X743CI(X743CIDecoder),
    #[cfg(feature = "x743")]
    X743Std(X743StdDecoder),
}
```

### 2.4 Read Loop 設計: read_loop 内でデコードまで完了

**既存パターン (FELib):**
```
read_loop (spawn_blocking) → [RawData via mpsc] → decode_loop (async) → [EventData via ZMQ]
```

**x743 パターン:**
```
x743_read_loop (spawn_blocking) → [EventData via mpsc] → publish_loop (async) → [EventData via ZMQ]
```

**理由:**
- `CAEN_DGTZ_DecodeEvent(handle, ...)` が **handle を引数に取る** — デコードは handle 所有者（read_loop）内で完結する必要がある
- **CAENDigitizer Library はスレッドセーフ性が保証されていない** (2012年設計、FELib以前の旧API)。`DecodeEvent` が handle 経由で内部状態やハードウェアにアクセスしている可能性があり、別スレッドから `ReadData` と `DecodeEvent` を同時に呼ぶと割り込み競合・未定義動作のリスクがある
- read_loop 内で `ReadData` → `GetNumEvents` → `DecodeEvent` → `DaqEvent 変換` → `FreeEvent` を一括処理
- x743 は ~7 kHz と低レートなので、read_loop 内でのデコード負荷は問題にならない

```rust
// x743 read loop (概念コード)
fn x743_read_loop(handle: &X743Handle, tx: Sender<Vec<EventData>>) {
    let mut buffer = handle.alloc_readout_buffer();
    loop {
        let size = handle.read_data(&mut buffer)?;
        let num_events = handle.get_num_events(&buffer, size)?;
        let mut events = Vec::with_capacity(num_events * max_channels);

        for i in 0..num_events {
            let (info, ptr) = handle.get_event_info(&buffer, size, i)?;
            let raw_event = handle.decode_event(ptr)?; // CAEN_DGTZ_X743_EVENT_t
            // Group → Channel 変換 + DaqEvent 生成
            for group in 0..num_groups {
                if !raw_event.gr_present[group] { continue; }
                for ch_in_group in 0..2 {
                    let channel = group as u8 * 2 + ch_in_group as u8;
                    let event = convert_to_daq_event(module_id, channel, &raw_event, group, ch_in_group);
                    events.push(event);
                }
            }
            handle.free_event(raw_event); // RAII: Drop で自動解放
        }
        tx.send(events)?;
    }
}
```

### 2.5 イベントデータマッピング

#### DPP_CI モード → EventData

| EventData フィールド | x743 データソース | 備考 |
|---------------------|------------------|------|
| `module` | `digitizer_id` | 設定ファイルから |
| `channel` | `group * 2 + ch_in_group` | 物理チャンネル番号に変換 |
| `timestamp_ns` | `TDC * 5.0` | 5ns 刻み。精度改善は Phase 3 |
| `energy` | `Charge` | 積分電荷 (16bit) |
| `energy_short` | `0` | DPP_CI では short gate なし |
| `fine_time` / `flags` | `Peak \| (Baseline << 16)` | Peak 下位16bit, Baseline 上位16bit |
| `waveform` | `None` | DPP_CI では波形なし |

#### STANDARD モード → EventData (Phase 3)

| EventData フィールド | x743 データソース | 備考 |
|---------------------|------------------|------|
| `timestamp_ns` | ソフトウェア CFD 計算 | `TDC*5 + fine_time` |
| `energy` | ソフトウェアゲート積分 | WaveDemo の WDWaveformProcess.c 参照 |
| `energy_short` | ショートゲート積分 | PSD 計算用 |
| `waveform` | `DataChannel[0..1][]` | `Vec<i16>` に変換 |

### 2.6 SAMLONGキャリブレーション

**タイミング:**
1. **Open 直後:** `LoadSAMCorrectionData()` — EEPROM からキャリブデータをロード（数秒かかる）
2. **Configure 時:** `SetSAMCorrectionLevel(ALL)` — 補正レベルを設定
3. **オプション:** 長時間運用では Start 前に毎回キャリブレーション実行（温度変化対策）

**設定:**
```toml
[board]
sam_correction_level = "all"      # "all", "pedestal_only", "inl", "disabled"
sam_sampling_frequency = "3.2ghz" # "3.2ghz", "1.6ghz", "800mhz", "400mhz"
acquisition_mode = "charge"       # "charge" (DPP_CI), "waveform" (STANDARD)
```

### 2.7 DigitizerConfig 拡張

```rust
pub struct DigitizerConfig {
    // ... 既存フィールド ...

    /// x743 固有設定 (feature = "x743")
    #[cfg(feature = "x743")]
    pub x743: Option<X743Config>,
}

#[cfg(feature = "x743")]
pub struct X743Config {
    pub sam_correction_level: SamCorrectionLevel,
    pub sam_sampling_frequency: SamFrequency,
    pub acquisition_mode: X743AcquisitionMode,
    pub post_trigger_size: u8,  // 1-255, SAMLONG write clock 単位
}
```

**注:** `#[cfg(feature)]` を DigitizerConfig に入れると TOML deserialization が複雑になるため、
実際には `Option<X743Config>` として常に定義し、`x743` feature 無効時は `None` を強制する方が実用的。

### 2.8 フロントエンド (Angular) 設定 UI

**案: チャンネルテーブル + グループ設定セクション**

```
┌─ x743 Board Settings ─────────────────────────────┐
│ Acquisition Mode: [Charge ▼]                       │
│ Sampling Frequency: [3.2 GHz ▼]                    │
│ SAM Correction: [Full (ALL) ▼]                     │
│ Post-Trigger Size: [20]                            │
│ Trigger Logic: [OR ▼]  Majority Level: [1]        │
├─ Group Settings ───────────────────────────────────┤
│ Group │ Pair Logic │ Coincidence Window │          │
│ 0     │ [OR ▼]     │ [15] ns            │          │
│ 1     │ [OR ▼]     │ [15] ns            │          │
│ ...                                                │
├─ Channel Settings ─────────────────────────────────┤
│ Ch │ Enable │ DC Offset │ Threshold │ Polarity │   │
│ 0  │ [✓]    │ [0.0] V   │ [0.05] V  │ [Pos ▼]  │   │
│ 1  │ [✓]    │ [0.0] V   │ [0.05] V  │ [Pos ▼]  │   │
│ ...                                                │
└────────────────────────────────────────────────────┘
```

---

## 3. ファイル構成

```
src/reader/
├── mod.rs                    # Reader 本体 (FirmwareType 分岐追加)
├── caen/                     # 既存 FELib FFI (変更なし)
│   ├── mod.rs
│   ├── handle.rs
│   ├── ffi.rs
│   ├── error.rs
│   └── validation.rs
├── caen_legacy/              # 新設: CAENDigitizer Library FFI
│   ├── mod.rs                # safe wrapper API
│   ├── ffi.rs                # bindgen 生成
│   ├── handle.rs             # X743Handle (RAII)
│   ├── error.rs              # エラーコード変換
│   └── types.rs              # Rust-friendly 型
├── decoder/
│   ├── mod.rs                # DecoderKind に X743CI/X743Std 追加
│   ├── common.rs             # 変更なし
│   ├── psd1.rs               # 変更なし
│   ├── psd2.rs               # 変更なし
│   ├── pha1.rs               # 変更なし
│   ├── amax.rs               # 変更なし
│   ├── x743_ci.rs            # 新設: DPP_CI デコーダ
│   └── x743_std.rs           # 新設: STANDARD デコーダ (Phase 3)
└── x743_read_loop.rs         # 新設: x743 専用 read loop

src/config/
└── digitizer.rs              # FirmwareType 拡張 + X743Config 追加
```

---

## 4. フェーズ計画

### Phase 1: FFI 基盤 + 接続確認 (1-2 週間)

**目標:** x743 とハードウェアレベルで通信できる状態

**タスク:**
1. `caen_legacy/ffi.rs` — bindgen で CAENDigitizer Library バインディング生成
2. `caen_legacy/handle.rs` — X743Handle (Open/Close/Reset の RAII ラッパー)
3. `caen_legacy/error.rs` — エラーコード → Result 変換
4. `FirmwareType` に `X743CI`, `X743Std` 追加
5. `Cargo.toml` に `x743` feature flag 追加
6. 接続テスト: Open → GetInfo → Reset → Close

**成果物:** `cargo build --features x743` が通り、実機で Open/Close できる

**依存:** Linux 実機 (172.18.4.147)、libCAENDigitizer.so インストール済み

### Phase 2: DPP_CI モード実装 (2-3 週間)

**目標:** Charge Integration モードで data 取得 → 既存パイプライン (Merger/Recorder/Monitor) に乗せる

**タスク:**
1. `caen_legacy/handle.rs` — Configure (SAM パラメータ一式)、Start/Stop、ReadData
2. `x743_read_loop.rs` — read + decode + 変換ループ
3. `decoder/x743_ci.rs` — `CAEN_DGTZ_X743_EVENT_t` → `EventData` 変換
4. `src/config/digitizer.rs` — `X743Config` 追加
5. TOML 設定ファイル例: `config/config_x743_test.toml`
6. Reader の `mod.rs` で FirmwareType 分岐追加
7. 統合テスト: x743 → Merger → Recorder でファイル記録

**成果物:** DPP_CI モードでの End-to-End データ取得

**依存:** Phase 1 完了、x743 実機

### Phase 3: STANDARD モード + 波形処理 (3-4 週間)

**目標:** フル波形取得 + ソフトウェアベースの信号処理

**タスク:**
1. `decoder/x743_std.rs` — 波形デコーダ (ADC サンプル → WaveformData)
2. 信号処理モジュール:
   - ベースライン計算 (移動平均)
   - LED / CFD ディスクリミネータ (WaveDemo 参照)
   - ゲート積分 (エネルギー計算)
3. Monitor 波形表示対応 (12bit, group→channel 変換)
4. Tune Up 対応 (x743 用パラメータ調整)

**成果物:** 波形取得 + ソフトウェア解析パイプライン

**依存:** Phase 2 完了

### Phase 4: フロントエンド + 運用最適化 (2 週間)

**タスク:**
1. Angular Settings UI: x743 専用設定コンポーネント
2. Monitor: 12bit ヒストグラム表示対応
3. 温度ドリフト対策: 定期キャリブレーション機能
4. ドキュメント更新

---

## 5. リスクと対策

| リスク | 影響度 | 対策 |
|--------|--------|------|
| **CAENDigitizer Library の Linux 限定** | 中 | `#[cfg(feature = "x743")]` で macOS ビルドに影響なし |
| **bindgen 生成の構造体サイズ不整合** | 高 | 実機で `sizeof()` を C 側と比較検証。テスト追加 |
| **DPP_CI モードのデータ構造が不明瞭** | 中 | WaveDemo は STANDARD のみ。実機で DPP_CI の出力を確認必要 |
| **TDC 5ns 精度が Event Builder に不十分** | 低 | x743 は低レートなので時間窓が広い。PosEdgeTimeStamp も検討 |
| **SAMLONG ペデスタル変動** | 中 | 定期的な SW trigger でベースラインモニタリング（診断用） |
| **handle の thread safety** | 中 | read_loop 内でデコードまで完結。handle 共有不要 |
| **libCAENDigitizer と FELib のシンボル衝突** | 低 | 別プロセス（Reader は 1 プロセス = 1 デジタイザ）で回避済み |

---

## 6. 既存コードへの影響

| ファイル/モジュール | 変更内容 | 影響度 |
|---------------------|----------|--------|
| `src/config/digitizer.rs` | `FirmwareType` に 2 variant 追加、`X743Config` 追加 | 中 |
| `src/reader/mod.rs` | x743 用 read loop 起動分岐 | 小 |
| `src/reader/decoder/mod.rs` | `DecoderKind` に 2 variant 追加 | 小 |
| `Cargo.toml` | `x743` feature + `bindgen` 依存追加 | 小 |
| `build.rs` | bindgen コード生成追加 | 小 |
| Merger / Recorder / Monitor | **変更なし** (EventData 共通形式) | なし |
| Angular フロントエンド | x743 Settings UI 追加 (Phase 4) | 中 |

---

## 7. テスト戦略

### ユニットテスト (CI 対応)
- デコーダ変換ロジック: 既知の `X743_EVENT_t` バイナリ → `EventData` 検証
- 設定パース: TOML → `X743Config` のシリアライズ/デシリアライズ
- グループ↔チャンネル変換の正確性

### 統合テスト (実機のみ)
- Open → Configure → Calibrate → Start → Read → Stop → Close
- DPP_CI: Charge/Peak/Baseline/TDC の妥当性確認
- STANDARD: 波形データの完全性
- End-to-End: x743 → Merger → Recorder → ファイル検証

### テストデータ
- WaveDemo で STANDARD モードの raw binary を保存 → テストリソースとして使用
- DPP_CI モードの出力は実機で取得し保存

---

## 8. リファレンス

| ドキュメント | パス |
|-------------|------|
| CAENDigitizer Library マニュアル | `legacy/UM1935_CAENDigitizer_U_&_R_Manual_rev17.pdf` |
| WaveDemo x743 ソースコード | `legacy/caenwavedemo_x743-1.2.1/` |
| TDigiTES ソースコード (CAENDigitizer使用例) | `legacy/TDigiTES/` |
| x743 Specific Functions | マニュアル Chapter 5 (pp.53-60) |
| Acquisition Example | マニュアル pp.49 |

---

## 9. 判断保留事項

1. **DPP_CI モードで PosEdgeTimeStamp/NegEdgeTimeStamp が使えるか？** — 実機テストで確認
2. **DPP_CI モードで energy_short 相当のデータが取れるか？** — Charge Mode のデータ構造要確認
3. **x743 ボードマニュアル** — SAMLONG の詳細仕様（dead time, trigger management）の追加資料が必要な可能性
4. **DigitizerConfig に `#[cfg(feature)]` を入れるか、常に `Option` で持つか** — TOML互換性の観点から `Option` 推奨
