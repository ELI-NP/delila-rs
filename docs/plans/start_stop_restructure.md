# Reader Start/Stop フロー再構成 — 設計書

**日付:** 2026-02-24
**Status:** 計画中 (Gemini レビュー待ち)
**関連:** TODO/41_start_stop_restructure.md

---

## 1. 問題

DIG1 (PSD1/PHA1, DT5730B/VX1730B) のタイムスタンプカウンタが Run 間でリセットされない。

### 1.1 実験結果 (2026-02-24, 172.18.4.76)

DAQ を再起動せずに複数 Run を実行した場合、PSD1 のタイムスタンプは前回 Run の続きから開始:

| Run | PSD2 (VX2730) timestamp_ns | PSD1 (DT5730B) timestamp_ns |
|-----|---------------------------|----------------------------|
| 104 (DAQ 再起動直後) | 0.000 | 0.000 |
| 105 (Stop → Start) | 0.000 | ~7.16s (前回 Run の残留値) |
| 108 (Reset → Configure → Start) | 0.000 | ~26.6s |

### 1.2 試行済みコマンド (すべて効果なし)

| コマンド | 結果 |
|---------|------|
| `/cmd/reset` | タイムスタンプリセットされず |
| `/cmd/disarmacquisition` | タイムスタンプリセットされず |
| `/cmd/cleardata` | タイムスタンプリセットされず |
| S-IN 信号 | タイムスタンプリセットされず |
| `/cmd/armacquisition` (S-IN mode) | タイムスタンプリセットされず |

### 1.3 唯一の解

`CAEN_FELib_Close` + `CAEN_FELib_Open` — DeviceConnection を Drop して再生成するとタイムスタンプが 0 にリセットされる。DAQ プロセス再起動時に毎回タイムスタンプがリセットされるのはこのため。

**Note:** PSD2 (DIG2/VX2730) は `/cmd/swstartacquisition` でタイムスタンプがリセットされるため、この問題は DIG1 固有。

---

## 2. Legacy DELILA C++ の分析

### 2.1 ReaderPSD.cpp フロー

`legacy/DELILA/Components/ReaderMT_PSD/ReaderPSD.cpp`:

```
daq_configure() (line 85):
  ├─ LoadParameters(configFile)
  ├─ OpenDigitizers()         ← CAEN_DGTZ_OpenDigitizer2
  ├─ InitDigitizers()         ← ProgramDigitizer (全パラメータ設定)
  ├─ UseHWFineTS()
  └─ AllocateMemory()

daq_start() (line 153):
  └─ fDigitizer->Start()      ← StartAcquisition (arm + start 一括)

daq_stop() (line 169):
  └─ fDigitizer->Stop()       ← StopAcquisition

daq_unconfigure() (line 143):
  ├─ FreeMemory()
  └─ CloseDigitizers()        ← CAEN_DGTZ_CloseDigitizer
```

**毎 Run サイクル:** `configure(Open+Init) → start → stop → unconfigure(Close)`

Legacy では **Arm は分離されていない**。`StartAcquisition()` が arm+start を一括で行う。DAQ-MW フレームワークでは毎回 configure/unconfigure のサイクルを踏むため、毎 Run で Open/Close が行われ、タイムスタンプは自然にリセットされていた。

### 2.2 現在の Rust 実装の問題

現在の Rust 実装 (`src/reader/mod.rs`) では:
- **Open:** プロセス起動時に `try_connect_raw()` で 1 回のみ
- **Close:** プロセス終了時の Drop でのみ
- **Stop:** `/cmd/disarmacquisition` + drain + `/cmd/cleardata` のみ — **connection を Close しない**

結果として、DIG1 のタイムスタンプカウンタは DAQ プロセス再起動時まで蓄積し続ける。

---

## 3. 新フロー設計

### 3.1 ユーザー指定のフロー

```
直列 Open       → flock で自動直列化済み (CaenHandle::open 内)
並列 Configure  → 全 Reader 並列
全て成功したら
並列 Start      → armacquisition + (SW なら swstartacquisition)
Stop + Close    → 並列で良い
```

### 3.2 State Machine の変更

現在:
```
Idle → Configured → Armed → Running → Configured (Stop)
```

新規:
```
Idle → Configured → Running → Configured (Stop)
                                           ↓
                        (ReadLoop 内部: connection Close → 次回 Open)
```

- `ComponentState::Armed` は enum に残す (後方互換)
- `Configured → Running` の直接遷移を追加
- Operator は Arm フェーズを使用しない
- ReadLoop 内部で armacquisition を Start 時に呼ぶ

### 3.3 ReadLoop 内部フロー

**Stop 時:**
1. `/cmd/disarmacquisition`
2. Drain remaining data (既存: max 1000 events or 1s)
3. Send Stop signal to decode pipeline
4. `connection = None` — **RAII で CAEN_FELib_Close が呼ばれる**
5. `last_connect_attempt` を過去に設定 (cooldown バイパス)

**次回 Start 時 (state catch-up が自動処理):**
1. `connection.is_none()` → `try_connect_raw()` — **CAEN_FELib_Open** (flock 直列化)
2. `!hw_configured` → `apply_config()` — Configure
3. `!hw_running` → `send_start_acquisition()` — armacquisition + (SW なら swstartacquisition)

### 3.4 FW 別 Start コマンド仕様

| Firmware | Start Source | コマンドシーケンス |
|----------|-------------|------------------|
| DIG1 (PSD1/PHA1) | START_MODE_SW | `/cmd/armacquisition` (arm=start) |
| DIG1 (PSD1/PHA1) | START_MODE_S_IN | `/cmd/armacquisition` (arm のみ、S-IN で start) |
| DIG2 (PSD2) | SWcmd | `/cmd/armacquisition` → `/cmd/swstartacquisition` |
| DIG2 (PSD2) | SIN | `/cmd/armacquisition` (arm のみ、S-IN で start) |

**統合関数:** `send_start_acquisition()` がすべてのケースを処理。

---

## 4. Borrow Checker 対策

### 4.1 問題

ReadLoop の状態同期ブロックは `if let Some(ref mut conn) = connection { ... }` の中にある。
この借用中に `connection = None` (Drop) を呼ぶことはできない。

### 4.2 以前の失敗

`connection.take()` を `if let Some(ref mut conn)` 内で呼ぼうとして失敗。
`needs_close` フラグを使ったが変数シャドウイングで混乱し、ユーザーに却下された。

### 4.3 解決策: `should_close_connection` フラグパターン

```rust
let mut should_close_connection = false;

if let Some(ref mut conn) = connection {
    // ... Configure / Start / Stop 処理 ...

    // Stop 時にフラグを設定
    if stop_needed {
        // ... disarm, drain, stop signal ...
        should_close_connection = true;
    }
}
// ← if let ブロック終了、conn の借用が解放される

if should_close_connection {
    connection = None;  // RAII: Drop → CAEN_FELib_Close
    last_connect_attempt = Instant::now() - RECONNECT_COOLDOWN - Duration::from_millis(1);
}
```

**前回との違い:**
- `let mut should_close_connection = false;` をブロック外で宣言（シャドウイングなし）
- ブロック内で `= true` を代入（`let` 再宣言ではない）
- 単純で読みやすい標準的な Rust パターン

---

## 5. flock 直列化の仕組み

`src/reader/caen/handle.rs:172-215`:

```rust
pub fn open(url: &str) -> Result<Self, CaenError> {
    let _lock = Self::acquire_open_lock();  // flock(LOCK_EX)
    // ... CAEN_FELib_Open ...
}  // _lock drop → flock 解放
```

- `/tmp/delila_caen_open.lock` に対して `flock(LOCK_EX)` を取得
- `CAEN_FELib_Open` 完了後に自動解放 (RAII)
- **全プロセスの Open が直列化される**
- a3818 カーネルドライバが同時 open でクラッシュする問題への対策

新フローでは Stop 後の再接続時にも自動的にこの直列化が効く。追加コード不要。

---

## 6. 各コンポーネントへの影響分析

### 6.1 Reader (src/reader/mod.rs)

| 項目 | 変更 |
|------|------|
| `send_arm_command()` | **削除** → `send_start_acquisition()` に統合 |
| `send_start_command()` | **削除** → `send_start_acquisition()` に統合 |
| `read_loop_raw()` Arm ブロック | **削除** — Start ブロックに統合 |
| `read_loop_raw()` Stop ブロック | **変更** — `should_close_connection = true` 追加 |
| `read_loop_raw()` Reset ブロック | **変更** — 切断予定なら skip、代わりに切断 |
| `read_loop_opendpp()` | 上記と同一の変更 |
| `DeviceConnection` struct | **変更なし** |
| `try_connect_raw()` | **変更なし** |
| `try_connect_opendpp()` | **変更なし** |

### 6.2 Operator (src/operator/routes/status.rs)

| 項目 | 変更 |
|------|------|
| `run_start()` | Phase 2 (Arm) 削除 |
| `start()` | Auto-arm ブロック削除 |
| `stop()` | **変更なし** |
| `configure()` | **変更なし** |

### 6.3 State Machine (src/common/command.rs)

| 項目 | 変更 |
|------|------|
| `can_transition_to()` | `(Configured, Running)` 追加 |
| `valid_commands()` | Configured に "Start" 追加 |

### 6.4 Tuneup (src/operator/routes/tuneup.rs)

| 項目 | 変更 |
|------|------|
| `tuneup_start()` | `arm_all_sync` 削除 |
| `tuneup_apply()` | `arm_all_sync` 削除 |

### 6.5 影響なし

- Merger, Recorder, Monitor, Emulator — Arm は no-op だったため影響なし
- `client.rs` の `arm_all()` / `arm_all_sync()` — 残す (将来の手動 arm 用)
- Angular UI — "Armed" 表示は DAQ state ではなく waveform trigger のみ
- DecodeLoop — 変更なし
- CLAUDE.md State Machine 記載 — 更新必要

### 6.6 Operator コメント更新

- `src/operator/routes/status.rs` のコメント "backend does Reset → Configure → Arm → Start" を更新
- `src/operator/services/operator.service.ts` (Angular) のコメント更新

---

## 7. リスク分析

### 7.1 リスク: Close/Open に時間がかかる

**影響:** Start の遅延 (数百ms〜数秒)
**対策:** flock が直列化を保証。各 Reader は順番に Open。Close は RAII で Stop 時に即座実行。
**受容:** Legacy でも毎 Run で Close/Open していた。許容範囲。

### 7.2 リスク: Close 中にドライバがハング

**影響:** ReadLoop がブロック
**対策:** `spawn_blocking` で動作するため tokio ランタイムには影響しない。ハングした場合はプロセス再起動。
**受容:** Legacy でも同じリスク。a3818 v1.6.12 で安定性向上済み。

### 7.3 リスク: Reconnect cooldown バイパスの副作用

**影響:** 意図しないタイミングで即座再接続
**対策:** `should_close_connection` が true の場合のみバイパス。通常のエラー切断では RECONNECT_COOLDOWN (1s) が有効。

### 7.4 リスク: DIG2 (PSD2) で不要な Close/Open

**影響:** PSD2 は `/cmd/swstartacquisition` でタイムスタンプがリセットされるため Close/Open 不要
**対策:** 全 FW 統一で Close/Open する。PSD2 でも害はない (少しの遅延のみ)。
**受容:** コードの単純さを優先。FW ごとの分岐は避ける。

---

## 8. 検証計画

### 8.1 ユニットテスト

1. `Configured.can_transition_to(Running)` == true
2. `handle_command(Start)` from Configured → Running 成功
3. `valid_commands(Configured)` に "Start" を含む

### 8.2 ビルド検証

```bash
cargo fmt && cargo clippy -- -D warnings && cargo test
```

### 8.3 ハードウェア検証

1. DT5730B (PSD1) で Start → Stop → Start
2. 2回目の Run のタイムスタンプが 0.000 付近から開始することを `event_dump` で確認
3. VX2730 (PSD2) でも同様に動作確認
4. Tune Up Apply サイクル (Stop → Apply → Start) が正常動作
