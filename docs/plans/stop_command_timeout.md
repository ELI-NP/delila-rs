# 高データレート時の Stop コマンドタイムアウトを修正

**Status: ✅ COMPLETED** (2026-02-16)

## Context

**Tune Up モード**で高データレート時に Stop コマンドがUIでタイムアウトエラーになる問題。

### 根本原因: Reader の decode_loop が tokio runtime を独占

```
Reader プロセス内:
  command_task (tokio::spawn) — ZMQ REP で Stop コマンドを受信
  decode_loop  (tokio::spawn) — デコード + シリアライズ + ZMQ PUB 送信
  ReadLoop     (spawn_blocking) — CAEN FFI blocking read ← 問題なし
```

`decode_loop` ([src/reader/mod.rs:1283-1453](src/reader/mod.rs#L1283-L1453)) は `tokio::select!` 内で:
1. `rx.recv().await` → 高データレートではチャンネルに常にデータ → **即 Ready（yield なし）**
2. `decoder.decode_into()` → **CPU-bound（yield なし）**
3. `msg.to_msgpack()` → **CPU-bound（yield なし）**
4. `data_socket.send(zmq_msg).await` → HWM=0 で無制限バッファ → **即 Ready（yield なし）**
5. ループ先頭に戻る

**全ステップが即 Ready の場合、ループは一度も yield せずに回り続ける。** tokio worker thread が decode_loop に独占され、同じ runtime 上の command_task がスケジュールされない → ZMQ REP がレスポンスを返せない → Operator 側で5秒タイムアウト。

Tune Up モードでは Recorder は Running ではないため、Recorder のディスクI/Oは無関係。

### Gemini レビュー結果

- 根本原因の分析に同意
- `block_in_place()` を高頻度で呼ぶのは runtime churn のリスクあり
- 専用 thread への分離が推奨パターン
- Recorder の writer_task も同様の問題を持つ（通常Run時に影響する可能性）→ 別途対応

---

## 修正方針

### 変更1 (主因修正): decode_loop に yield_now() 追加
**File:** [src/reader/mod.rs](src/reader/mod.rs#L1283-L1453)

最もシンプルで KISS な修正。`tokio::task::yield_now().await` を追加して、他のタスク（command_task）にスケジュールの機会を与える。

```rust
// decode_loop の select! ループ内、データ処理後に追加
output = rx.recv() => {
    match output {
        Some(ReadLoopOutput::Raw(raw_data)) => {
            // ... decode, serialize, send ...

            // 高データレート時に command_task がスケジュールされるよう yield
            tokio::task::yield_now().await;
        }
        // ...
    }
}
```

**なぜ yield_now() で十分か:**
- `yield_now()` は現在のタスクをタスクキューの末尾に戻す（コスト: ナノ秒）
- 他のタスク（command_task）が待機中なら即座にスケジュールされる
- CPU-intensive な decode/serialize 自体は数マイクロ秒 — `spawn_blocking` に移すオーバーヘッドの方が大きい
- 1イベントごとの yield は過剰 → **バッチ処理後に1回** で十分

### 変更2 (副次修正): Recorder writer_task を専用 std::thread に分離
**File:** [src/recorder/mod.rs](src/recorder/mod.rs)

通常 Run モードでの同様の問題を防止。Tune Up とは別の問題だが、同じ原因パターン。

#### 2a. チャンネルを `std::sync::mpsc` に変更

```rust
// Before: tokio::sync::mpsc (async)
let (writer_tx, writer_rx) = mpsc::unbounded_channel::<WriterCommand>();

// After: std::sync::mpsc (blocking)
let (writer_tx, writer_rx) = std::sync::mpsc::channel::<WriterCommand>();
```

`std::sync::mpsc::Sender::send()` は non-blocking — async コードからも問題なし。

#### 2b. writer_task を blocking 関数に変換

`tokio::select!` → `recv_timeout()` + state ポーリング。`state_rx.borrow()` は任意のスレッドから安全に呼べる（Send + Sync）。

```rust
fn writer_task_blocking(
    rx: std::sync::mpsc::Receiver<WriterCommand>,
    config: RecorderConfig,
    stats: Arc<AtomicStats>,
    state_rx: watch::Receiver<ComponentState>,
) {
    let mut writer = FileWriter::new(config, stats);
    let mut eos_received = false;
    let mut last_state = ComponentState::Idle;

    loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(cmd) => { /* 既存のコマンド処理ロジックをそのまま移植 */ }
            Err(RecvTimeoutError::Timeout) => {
                // state_rx.changed() の代替: 100ms毎にstate確認
                let current = *state_rx.borrow();
                if current != last_state {
                    last_state = current;
                    // ファイルクローズ等の既存ロジック
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}
```

#### 2c. spawn を std::thread に変更

```rust
// Before:
let writer_handle = tokio::spawn(async move { Self::writer_task(...).await });

// After:
let writer_handle = std::thread::Builder::new()
    .name("recorder-writer".to_string())
    .spawn(move || Self::writer_task_blocking(...))
    .expect("Failed to spawn writer thread");
```

#### 2d. RecorderCommandExt の writer_tx 型変更

`mpsc::UnboundedSender<WriterCommand>` → `std::sync::mpsc::Sender<WriterCommand>`

### 変更3 (副次修正): Reader の Stop信号送信を保証
**File:** [src/reader/mod.rs](src/reader/mod.rs#L772)

```rust
// Before: try_send — 満杯で黙って失敗
let _ = tx.try_send(ReadLoopOutput::Stop);

// After: 最大3秒リトライ
let stop_deadline = Instant::now() + Duration::from_secs(3);
let mut stop_signal = ReadLoopOutput::Stop;
loop {
    match tx.try_send(stop_signal) {
        Ok(()) => { info!("Stop signal sent"); break; }
        Err(TrySendError::Full(returned)) => {
            if Instant::now() > stop_deadline {
                error!("Failed to send Stop: channel full for 3s");
                break;
            }
            stop_signal = returned;
            std::thread::sleep(Duration::from_millis(10));
        }
        Err(TrySendError::Closed(_)) => { break; }
    }
}
```

### 変更4 (副次修正): Reader ドレインループに上限追加
**File:** [src/reader/mod.rs](src/reader/mod.rs#L764-L768)

```rust
let drain_start = Instant::now();
const MAX_DRAIN_EVENTS: u64 = 1000;
const MAX_DRAIN_TIME: Duration = Duration::from_secs(1);
while let Ok(Some(raw)) = conn.endpoint.read_data(100, &mut read_buffer) {
    drained += 1;
    let decoder_raw = decoder::RawData::from(raw);
    let _ = tx.try_send(ReadLoopOutput::Raw(decoder_raw));
    if drained >= MAX_DRAIN_EVENTS || drain_start.elapsed() > MAX_DRAIN_TIME {
        warn!(drained, "Drain limit reached");
        break;
    }
}
```

---

## Critical Files

| File | Changes | 重要度 |
|------|---------|--------|
| [src/reader/mod.rs](src/reader/mod.rs) | `yield_now()` + Stop信号保証 + ドレイン制限 | **主因修正 (Tune Up)** |
| [src/recorder/mod.rs](src/recorder/mod.rs) | writer_task を専用 std::thread に分離 | 予防修正 (通常Run) |

## Verification

1. `cargo clippy -- -D warnings`
2. `cargo test`
3. 実機テスト — Tune Up モード (DT5730B SN:990):
   - 高データレート（self-trigger、閾値低め）で Tune Up Start → Stop を繰り返す
   - Stop がタイムアウトせずに完了することを確認
4. 実機テスト — 通常 Run:
   - 高データレートで Run → Stop を繰り返す
   - Recorder のファイルが正常にクローズされることを確認
