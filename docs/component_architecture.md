# Component Architecture - Lock-Free Task Separation

**全コンポーネントは以下のアーキテクチャに従うこと。違反は許容しない。**

## 概要図

```
┌─────────────────────────────────────────────────────────────────┐
│                        Component                                 │
│                                                                  │
│  ┌──────────┐   mpsc    ┌──────────┐   mpsc    ┌──────────┐    │
│  │ Receiver │ ────────► │  Main    │ ────────► │ Sender   │    │
│  │ (ZMQ)    │  channel  │  Logic   │  channel  │ (ZMQ/IO) │    │
│  └──────────┘           └──────────┘           └──────────┘    │
│       │                      │                      │           │
│       │ 高速                 │ 処理                 │ 遅い可能性│
│       │ ブロック禁止         │ ソート等             │ fsync等   │
│       ▼                      ▼                      ▼           │
│  ┌──────────┐           ┌──────────┐                           │
│  │ Command  │◄─────────►│ State    │ (watch channel)           │
│  │ (ZMQ REP)│           │ (shared) │                           │
│  └──────────┘           └──────────┘                           │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

## 必須ルール

1. **Receiver Task**: ZMQソケットからの受信専用。処理をせずチャンネルに送るだけ。
   - **データ損失は絶対に許容しない**
   - bounded channel を使用し、メモリ使用量に上限を設ける
   - `try_send()` + retry loop (spawn_blocking 内) で backpressure 対応
   - 注意: `blocking_send()` は macOS で TLS fatal error を起こすため使用禁止
   - 処理が追いつかない場合は処理スレッドを増やす方針

2. **Main Logic Task**: データ処理（ソート、集計等）
   - 重い処理はここで行う
   - 入力・出力ともにmpscチャンネル経由

3. **Sender/Writer Task**: ZMQ送信またはファイル書き込み
   - fsync等の遅い操作はここで吸収
   - 上流をブロックしない

4. **Command Task**: 既存の`run_command_task()`を使用
   - 状態変更は`watch::Sender`経由で通知
   - 統計取得は`Arc<AtomicU64>`等のlock-free構造を使用

## 禁止事項

```rust
// ❌ 禁止: 受信ループ内でMutexロック
msg = socket.next() => {
    let mut state = self.state.lock().unwrap();  // ブロック！
    state.process(msg);
}

// ❌ 禁止: 書き込み完了を待ってから次の受信
for msg in receiver {
    file.write_all(&msg)?;
    file.sync_data()?;  // fsyncが受信をブロック！
}
```

## 推奨パターン

```rust
// ✅ 推奨: タスク分離 + bounded チャンネル (データ損失なし)
let (tx, rx) = mpsc::channel(1000);

// Receiver task (spawn_blocking 内: try_send + retry でデータ損失なし)
tokio::task::spawn_blocking(move || {
    while let Some(msg) = source.read() {
        // NOTE: blocking_send() は macOS で TLS fatal error を起こす。
        // try_send() + retry loop を使用すること。
        let mut pending = msg;
        loop {
            match tx.try_send(pending) {
                Ok(()) => break,
                Err(mpsc::error::TrySendError::Full(v)) => {
                    pending = v;
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(mpsc::error::TrySendError::Closed(_)) => return,
            }
        }
    }
});

// Writer task
tokio::spawn(async move {
    while let Some(msg) = rx.recv().await {
        file.write_all(&msg)?;
        // fsyncはここでブロックしてもReceiverに影響なし
    }
});
```

## 統計・状態共有

```rust
// ✅ Lock-free counters for hot path
struct Stats {
    received: AtomicU64,
    sent: AtomicU64,
}

// ✅ watch channel for state broadcast
let (state_tx, state_rx) = watch::channel(ComponentState::Idle);

// ✅ DashMap for per-key stats (lock per entry, not global)
let per_source: DashMap<u32, SourceStats> = DashMap::new();
```

## 参照実装

- **Merger** (`src/merger/mod.rs`): Receiver/Senderタスク分離の例
- **Recorder** (`src/recorder/mod.rs`): Receiver/Sorter/Writerの3タスク分離の例
