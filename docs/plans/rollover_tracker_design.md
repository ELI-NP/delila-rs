# Rollover Tracker 統一設計（V1743 Step 3 + SW Fine TS 移行）

**ステータス:** 設計確定（2026-04-22）、実装未着手
**関連 TODO:** [TODO/47_v1743_standard_mode_redesign.md](../../TODO/47_v1743_standard_mode_redesign.md) Step 3
**レビュー:** Gemini 2.5 Pro 協議済（2026-04-22）
**目的:** V1743 TDC (40-bit) と PSD1/PHA1 SW Fine TS (32-bit) で共用できる頑健なロールオーバー拡張器

---

## 1. 背景

### 1.1 対象デジタイザとカウンタ

| ファミリ | カウンタ | ビット幅 | tick | rollover 周期 | 現状 |
|---|---|---|---|---|---|
| V1743 | `DataGroup[g].TDC` | 40-bit | 5 ns | ~91 分 | **ロールオーバー追跡なし**（[mod.rs:2457](../../src/reader/mod.rs#L2457)で生値キャスト） |
| PSD1/PHA1 SW Fine TS | Board Aggregate TTT | 32-bit | 2 ns (@ 500 MS/s) | ~8.6 s | [TimestampTracker (common.rs:48)](../../src/reader/decoder/common.rs#L48) で追跡中 |
| PSD1 without SW Fine TS | per-channel BTT | (FW 依存) | — | — | `scan_aggregate_headers()` in Dispatcher（別レイヤ、今回スコープ外） |

### 1.2 既存 `TimestampTracker` の問題点

- **32-bit ハードコード** (`1u64 << 32`)
- **f64 混入**: `time_step_ns: f64` を内部状態に保持。104 日で 1 ns 精度を失う、非決定性
- **`Instant` 依存**: host PC 時刻による safety net。V1743 91 分周期では hw drift で誤発火しうる
- **Single-board 前提**: V1743 の 4 groups を扱えない

### 1.3 プロジェクト制約

- 絶対ルール「データを落とさない」— silent rollover バグは致命的
- Merger / Event Builder は timestamp で sort する。monotonicity がソースで崩れるとソート破綻
- PSD1/PHA1 は本番稼働中 — regression させられない

---

## 2. 設計方針

### 2.1 コア原則（Gemini レビューで確立）

1. **内部は u64 ticks のみ**。ns 変換は ZMQ 送出直前の 1 回だけ
2. **マスキングを API 境界で強制** — `mask = (1 << bits) - 1` を `new()` で precompute、`extend()` 先頭で必ず適用
3. **`Instant` 依存を排除** — host-time safety net は捨てる。代わりに modulo 演算（TCP シーケンス番号式）で "進行方向" を判定
4. **per-group 4 tracker for V1743** — 単一 tracker は FIFO 読み出し順序逆転時に double-rollover バグを起こす
5. **Late arrival → epoch-1 で再構成** — out-of-order と判定したイベントは前 epoch の絶対 tick で返す

### 2.2 run_id は **不要**

Gemini は防御的に `EventData.run_id: u32` 追加を提案したが、本プロジェクトの既存アーキテクチャを分析した結果、**不要**と判断：

- `ReadLoopOutput::Start` / `Stop` は単一 mpsc チャンネル (FIFO 保証)
- DecodeLoop は [mod.rs:2874-2896](../../src/reader/mod.rs#L2874-L2896) で `Start` 信号受信時に `decoder.reset_for_new_run()` を実行
- `Stop` → 前イベント全処理 → EOS → `Start` → tracker reset の順序が構造的に保証されている

→ wire format (BSON/MessagePack/.delila) への変更は一切なし

---

## 3. API

### 3.1 公開インタフェース

```rust
// src/reader/decoder/rollover.rs (新規ファイル)

use thiserror::Error;

#[derive(Debug, Error)]
pub enum RolloverError {
    #[error("out-of-order event from epoch before tracker start (raw={raw:#x})")]
    Underflow { raw: u64 },
}

pub struct RolloverTracker {
    mask: u64,
    bits: u8,
    prev_raw: u64,
    rollover_count: u64,
    // 注: NO f64, NO Instant, NO sanity_threshold.
}

impl RolloverTracker {
    pub fn new(bits: u8) -> Self {
        assert!(bits >= 2 && bits <= 63, "bits must be in [2, 63]");
        Self {
            mask: (1u64 << bits) - 1,
            bits,
            prev_raw: 0,
            rollover_count: 0,
        }
    }

    /// raw counter 値（bits より上位は無視される）から絶対 tick を返す。
    /// rollover_count が 0 の状態で "前 epoch から遅延到着" が来た場合のみ Err。
    pub fn extend(&mut self, raw_tick: u64) -> Result<u64, RolloverError> {
        let raw = raw_tick & self.mask;
        let diff = raw.wrapping_sub(self.prev_raw) & self.mask;
        let half_period = 1u64 << (self.bits - 1);

        let absolute_tick = if diff > half_period {
            // 後ろに戻っている → 前 epoch の late arrival
            if self.rollover_count == 0 {
                return Err(RolloverError::Underflow { raw });
            }
            raw | ((self.rollover_count - 1) << self.bits)
        } else {
            // 通常進行 or 境界越え
            if raw < self.prev_raw {
                self.rollover_count += 1;
            }
            self.prev_raw = raw;
            raw | (self.rollover_count << self.bits)
        };
        Ok(absolute_tick)
    }

    /// Run start で呼ぶ（状態を初期化、新しい Run の最初のイベントに備える）
    pub fn reset(&mut self) {
        self.prev_raw = 0;
        self.rollover_count = 0;
    }

    /// 同一クロックの狭いサブカウンタ（例: 32-bit BTT に対する 31-bit Event TTT）を
    /// 直近の extended 値から再構成する。
    /// sub_bits <= bits でなければならない。
    pub fn reconstruct_subcounter(&self, extended: u64, sub_raw: u64, sub_bits: u8) -> u64 {
        assert!(sub_bits <= self.bits);
        let sub_mask = (1u64 << sub_bits) - 1;
        let sub = sub_raw & sub_mask;
        let hi = extended & !sub_mask;
        let candidate = hi | sub;
        // extended より未来ならそれは誤り → sub_bits 相当だけ戻す
        if candidate > extended {
            candidate.wrapping_sub(1u64 << sub_bits)
        } else {
            candidate
        }
    }
}
```

### 3.2 ns 変換は呼び出し側

```rust
// V1743
let tdc_tick = trackers[g].extend(group.TDC)?;
let tdc_ns = (tdc_tick as f64) * 5.0;

// PSD1 SW Fine TS
let ttt_tick = tracker.extend(header.board_time_tag as u64)?;
let full_ttt = tracker.reconstruct_subcounter(ttt_tick, trigger_time_tag as u64, 31);
let timestamp_ns = (full_ttt as f64) * 2.0 + frac * 2.0;
```

---

## 4. サブタスク

| # | タスク | 依存 | 成功条件 | 備考 |
|---|---|---|---|---|
| **3-1** | `RolloverTracker` 実装 | なし | `cargo clippy -- -D warnings` OK | 独立作業、既存コード不変 |
| **3-2** | CI テスト行列実装（§5） | 3-1 | 全テスト緑、proptest で invariant 確認 | 独立作業 |
| **3-3** | PSD1/PHA1 shadow-mode 並走 | 3-1, 3-2 | 本番 2h run で divergence=0 (tracing::error! ログ) | 旧 `TimestampTracker` と新 `RolloverTracker` 両方で計算、差分検出 |
| **3-4** | V1743 per-group 4 tracker 組み込み | 3-1, 3-2 | `x743_std_event_to_event_data` で `trackers[g].extend(group.TDC)` | 4 groups 独立 |
| **3-5** | V1743 decoder に `reset_for_new_run()` 経路追加 | 3-4 | ReadLoopOutput::Start で 4 tracker 全 reset | 既存 PSD1/PHA1 と同じ形 |
| **3-6** | 旧 `TimestampTracker` 削除 | 3-3 が安定後 | PSD1/PHA1 が `RolloverTracker` 単独で動作 | shadow 期間中 divergence 0 確認してから |
| **3-7** | 実機検証 — V1743 2h run + bits=16 擬似 wrap | 3-4 〜 3-6 | timestamp 単調、wrap ログ出現、EB ソート OK | 172.18.4.147 SN:25 |

### 実装順序

1. **3-1 + 3-2 を先に完成（独立、TDD）** → user review
2. **3-3 (shadow)** を本番に deploy、PSD1/PHA1 で divergence=0 を確認
3. **3-4, 3-5** で V1743 組み込み（3-3 が問題なければ同じパターンで安全）
4. **3-6** 旧 tracker 削除
5. **3-7** 実機 2h 検証

---

## 5. CI テスト行列

### 5.1 決定論的単体テスト（Gemini 推奨 8 ケース）

| # | 入力 (raw ticks) | bits | 期待 (absolute ticks) | 意図 |
|---|---|---|---|---|
| T1 | `[10, 20, 30]` | 40 | `[10, 20, 30]` | monotonic advance |
| T2 | `[MAX-5, MAX, 5, 10]` | 40 | `[MAX-5, MAX, 2^40+5, 2^40+10]` | clean rollover (MAX = 2^40 - 1) |
| T3 | `[10, 8, 12]` | 40 | `[10, 8, 12]` | micro-jitter (後退反映、rollover しない) |
| T4 | `[MAX-2, 2, MAX-1, 5]` | 40 | `[MAX-2, 2^40+2, MAX-1, 2^40+5]` | **rollover with late arrival** — 3番目は epoch 0! |
| T5 | `[5, MAX-5]` (gap > half period) | 40 | `[5, MAX-5]` | massive gap は out-of-order ではなく low-rate leap |
| T6 | `[5, (1<<42)+10]` | 40 | `[5, 10]` | upper bits は silently masked |
| T7 | bits=16 `[5]` then Err on `[5]` after reset from count=0 | 16 | `Err(Underflow)` for out-of-order at count=0 | initial Underflow 検出 |
| T8 | `reset()` → 直前状態を完全クリア | 40 | `[MAX-5, 5] → reset → [10, 20]` → `[10, 20]` | Run start 後は新規 |

### 5.2 proptest (property-based)

```rust
proptest! {
    #[test]
    fn extend_is_monotonic_for_forward_sequences(
        bits in 8u8..=48,
        // 前進のみの tick 列を生成
        seq in prop::collection::vec(0u64..1000, 1..100),
    ) {
        let mut tracker = RolloverTracker::new(bits);
        let mask = (1u64 << bits) - 1;
        let mut expected = 0u64;
        let mut last_abs = 0u64;
        for step in seq {
            expected = expected.wrapping_add(step);
            let raw = expected & mask;
            let abs = tracker.extend(raw).unwrap();
            prop_assert!(abs >= last_abs);
            last_abs = abs;
        }
    }

    #[test]
    fn reconstruct_subcounter_never_exceeds_extended(
        bits in 8u8..=48,
        sub_bits in 8u8..=32,
        extended in 0u64..1_000_000,
        sub_raw in 0u64..1_000_000,
    ) {
        prop_assume!(sub_bits <= bits);
        let tracker = RolloverTracker::new(bits);
        let result = tracker.reconstruct_subcounter(extended, sub_raw, sub_bits);
        prop_assert!(result <= extended);
    }
}
```

### 5.3 Shadow mode 比較テスト（3-3）

- PSD1/PHA1 decoder で新旧両方の tracker を通す
- 結果を f64 ns に変換後、**完全一致**を assert
- 最初のイベントから 1000 イベントまで毎回チェック、以後は 100 イベント毎にサンプリング
- 不一致検出時: `tracing::error!` + diagnostic ログ（prev_raw, new raw, rollover_count 両方）

### 5.4 実機 accelerated test

- `bits=16` の synthetic stream を実機ではなくローカルで生成
- 1 MHz 相当の入力 → 65 ms で wrap → 10,000 wrap = 10 分で検証
- 全イベントの timestamp が単調増加することを assert

---

## 6. 残るリスク・論点

### 6.1 V1743 per-group 順序逆転の頻度

Gemini の懸念「group 0→1→2→3 の readout 順序が境界付近で逆転する」は理論可能だが、実頻度不明。

**初期実装では per-group 4 tracker**で defensive に作り、運用ログで「group 間の TDC 不一致頻度」を観測。もし実質一致ならばシンプル化の余地あり。ただし定常運用では 4 tracker で性能問題ない（メモリ数百バイト）。

### 6.2 gap > half period の低 rate 問題

modulo 演算の前提は「イベント間隔 < 半周期」。V1743 の 45 分ギャップを超えると誤判定の可能性。

- 通常運用では 45 分間 0 イベントはあり得ない（pulser/source 常時）
- ただし **Tune Up モードで一時停止 → 再開** パターンは要注意
- **対策**: Tune Up の Pause/Resume で tracker reset を検討（要仕様確認）

### 6.2.5 ★ 旧 TimestampTracker の host-time safety net の問題点（2026-04-23）

Step 3-3 のユニットテストで、旧 `TimestampTracker` と新 `RolloverTracker` が
**合成データに対して divergence する**ことが判明。原因は旧実装の `Instant`-based
safety net（drift > 2s で rollover_count を host 時刻から再推定）。

#### 合成テストでの発火

- テストでは `Instant::now()` がほぼゼロ時間しか経過しない
- 一方 `btt = 0xFFFF_FF00` は `× 2 ns ≈ 8.6 秒` の board time を主張
- 差分 8.6s > 2s → safety 発火 → `rollover_count` を 0 に「補正」
- 直後の真のラップを見逃す

#### 本番運用での発火（より重要）

合成テストは artifact だが、**本番でも safety net は確実に発火する**：

- 典型的な水晶発振器精度: 50〜100 ppm
- 100 ppm = 1 秒あたり 100 μs のずれ、**1 時間あたり 0.36 秒**
- **約 5.5 時間連続ランで drift が 2 秒閾値を超える**
- それ以降、safety net は発火し続ける

#### 発火時の具体的副作用

長時間ランで sequential 検出が**正しく** rollover_count を追跡している場合でも、
safety net は起動する。補正結果は：

| 条件 | 補正値 | 結果 |
|---|---|---|
| drift が rollover 周期中央付近 | `== sequential_count` | 無害 |
| drift が rollover 周期境界付近 | `== sequential_count ± 1` | **timestamp が 8.59 秒ジャンプ** |

→ EB sort や physics 解析で不定期な 8.59s ジャンプとして顕在化するリスク。

#### 新実装との trade-off

- **旧 (safety net あり)**: sparse input で missed rollover 復旧可能。が、長時間ランで drift 誤発火・off-by-one timestamp ジャンプ
- **新 (safety net なし)**: 4.3 秒以上イベント断絶 + ラップ跨ぎで missed rollover。本番 DAQ の >kHz レートでは発生せず

本番 DAQ では後者のケースはほぼゼロ、前者は時間経過で必ず起きる → **新実装に統一する方が運用リスク小**。
Step 3-6 で safety net 含めて旧実装を削除する根拠。

#### レイヤ責任の原則（2026-04-23 確立）

旧実装の safety net は「decoder 内部で host wall-clock を参照して補正」だった。
これはレイヤ違反。**クロック同期・時刻補正は decoder レイヤで扱わない**：

- 物理（ハードウェア）: クロック共有（PSD Master, SinFanout, daisy chain）
- 物理（トリガ）: 共通トリガ信号（低レート実験ではパルサー external trigger）
- ソフト（上位）: Online Event Builder による cross-module 時刻推論
- ソフト（decoder）: **自モジュール内** coarse counter 拡張のみ（RolloverTracker の責務）

低レート実験（例: 1 Hz）で RolloverTracker の sparse-wrap 問題が気になる場合の対処は
**common trigger 入力または online EB** で解決する。decoder 内部で `Instant` を引かない。
詳細: `memory/layering_principle_clock_sync.md`

### 6.3 Late arrival 後のソート

`extend()` が late arrival に対して `epoch-1` の tick を返す → 出力 timestamp が後退する可能性がある。

- 本プロジェクトの Merger / EB は **既に非単調を許容**（reorder buffer + 最終 sort 前提）なので OK
- PSD1 DecodeLoop の ReorderBuffer（[mod.rs, decode_loop_parallelization](../../TODO/archive/40_decode_loop_parallelization.md)）との整合性は要検証：ReorderBuffer は sequence_number ベースなので timestamp 後退は問題ない

### 6.4 Tune Up モードの影響

Tune Up モードは Run を跨がないが、Apply 時にパイプラインを Stop→Start する（memory: "Tune Up Apply スペクトラム混在修正"）。この際に tracker が正しく reset されることを確認。

---

## 7. 関連ドキュメント

- [TODO/47_v1743_standard_mode_redesign.md](../../TODO/47_v1743_standard_mode_redesign.md) — 親 TODO
- [docs/plans/x743_standard_mode_design.md](x743_standard_mode_design.md) — Standard mode 設計全体
- [src/reader/decoder/common.rs](../../src/reader/decoder/common.rs) — 既存 TimestampTracker
- [src/reader/decoder/psd1.rs](../../src/reader/decoder/psd1.rs) — SW Fine TS 使用例
- [src/reader/mod.rs::x743_std_event_to_event_data](../../src/reader/mod.rs) — V1743 decoder (rollover 組み込み先)
- Gemini 2.5 Pro 協議ログ (2026-04-22) — API 形状・modulo 演算・per-group 判断の根拠
