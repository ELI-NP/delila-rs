# データ完全性 & パフォーマンス精査レポート

**作成日:** 2026-01-28
**Status:** Phase A + B + E 全完了・実機検証済み (2026-01-28)

---

## 実装結果サマリー

### Phase A: データ完全性修正 — 実装完了 ✅

| # | 修正 | ファイル | 結果 |
|---|------|---------|------|
| A1 | CLAUDE.md ポリシー更新 | `CLAUDE.md` | 「データ損失は絶対に許容しない」+ try_send retry パターン |
| A2 | Reader: bounded channel (1000) + try_send retry | `src/reader/mod.rs` | unbounded→bounded, TLS crash 回避 |
| A3 | Merger: record_drop() 呼出追加 | `src/merger/mod.rs` | ZMQ送信エラー時にドロップ記録 |
| A4 | DataSink: EOS エラーハンドリング統一 | `src/data_sink/mod.rs` | `let _` → `.is_err()` + break |

**実機検証:** 98 kHz, queue=0, クラッシュなし, Reader-Recorder イベント数一致

### Phase B: パフォーマンス修正 — 実装完了 ✅

| # | 修正 | ファイル | 結果 |
|---|------|---------|------|
| B1 | convert_event: move セマンティクス | `src/reader/mod.rs` | 6 Vec clone 排除 (波形あり時 ~18KB/event 削減) |
| B2 | read_data: shrink_to_fit() | `src/reader/caen/handle.rs` | 1MB→実データサイズに縮小 |
| B3 | Monitor: process_batch/event owned | `src/monitor/mod.rs` | 波形 clone 排除 + HistogramConfig Copy |

**実機検証:** 98.7 kHz, queue=0, データ損失なし, エラー/警告なし

### Phase E: 波形モード堅牢性修正 — 実装完了 ✅

| # | 修正 | ファイル | 結果 |
|---|------|---------|------|
| E1 | Stop ハング修正: try_send retry に shutdown/state チェック | `src/reader/mod.rs` | チャンネル full 時に Stop が完了するように |
| E2 | DecodeLoop サイレントクラッシュ修正: `?` → 明示的 match | `src/reader/mod.rs` | エラーがログに記録されるように |
| E3 | SIGBUS 修正: read_data バッファ 64MB 事前確保 + 再利用 | `src/reader/caen/handle.rs`, `src/reader/mod.rs` | CAEN FELib の bounds check なし問題を回避 |
| E4 | デバッグログ削除 + EVENT COUNT MISMATCH 警告削除 | `src/reader/mod.rs`, `src/reader/decoder/psd2.rs` | プロダクションログのクリーンアップ |

**波形モード実機検証結果:**

| テスト | 条件 | 結果 |
|--------|------|------|
| 1 kHz 波形 | ch16, record_length=4096, Positive | 安定動作, queue=0, ~999 Hz |
| 10 kHz 波形 | ch16, record_length=4096, Positive | 安定動作, queue=0, ~9,800 Hz, Stop 正常 |
| 帯域制限確認 | パルサー > 6 kHz (波形あり) | ~6.6 kHz で頭打ち → **1 GbE 帯域飽和** |

**帯域制限の分析:** 波形あり 1 event ≈ 16.4 KB。6,600 Hz × 16.4 KB ≈ 108 MB/s ≈ 864 Mbps。
VX2730 の `dig2://` 接続は 1 GbE のため、プロトコルオーバーヘッド込みで ~6.6 kHz が理論上限。
波形なしモード (event ~100 bytes) では 98 kHz 出ており、ソフトウェアのボトルネックではない。

### 修正ファイル一覧

| ファイル | 変更内容 |
|---------|---------|
| `CLAUDE.md` | データ損失ポリシー更新 + try_send retry 推奨パターン |
| `src/reader/mod.rs` | bounded channel, try_send retry, convert_event move |
| `src/reader/caen/handle.rs` | shrink_to_fit() 追加 |
| `src/merger/mod.rs` | record_drop() 有効化 + ZMQ エラー時呼出 |
| `src/data_sink/mod.rs` | EOS 送信エラーハンドリング統一 |
| `src/monitor/mod.rs` | process_event/batch owned, HistogramConfig Copy |
| `src/reader/decoder/psd2.rs` | EVENT COUNT MISMATCH 警告削除 (RAW フォーマット仕様) |
| `src/reader/caen/wrapper.c` | C wrapper (変更なし、参照) |
| `src/bin/caen_info.rs` | read_data API 変更に追従 (64MB バッファ) |
| `tests/felib_integration_test.rs` | read_data API 変更に追従 |
| `config/digitizers/psd2_test.json` | ch16 Positive polarity + dc_offset 追加 |
| `config/digitizers/psd2_waveform.json` | 波形テスト用設定 (record_length=4096, WaveAnalogProbe 設定) |

### 未実施 (Phase C/D — Follow-up)

- C1: Waveform 型統一 (decoder/common)
- C2: Merger/Recorder/Monitor bounded channel
- D: 並列デコード (FuturesOrdered)

---

## 1. CLAUDE.md ポリシー問題

### 現状

CLAUDE.md L127-129 に以下の記述がある:

```
1. **Receiver Task**: ZMQソケットからの受信専用。処理をせずチャンネルに送るだけ。
   - `try_send()` を使用し、チャンネルがfullでもブロックしない
   - ブロック禁止（データ損失よりも受信継続を優先）
```

**問題:** 「データ損失よりも受信継続を優先」はユーザーのポリシー「データ損失は絶対に許容しない」と矛盾する。

### 修正案

```markdown
1. **Receiver Task**: ZMQソケットからの受信専用。処理をせずチャンネルに送るだけ。
   - **データ損失は絶対に許容しない**
   - bounded channel を使用し、メモリ使用量に上限を設ける
   - `try_send()` + retry loop (spawn_blocking 内) で backpressure 対応
   - 注意: `blocking_send()` は macOS で TLS fatal error を起こすため使用禁止
   - 処理が追いつかない場合は処理スレッドを増やす方針
```

推奨パターンの例も更新が必要 (L162-178)。

---

## 2. チャンネル送信パターン精査結果

### 2.1 unbounded_channel の使用箇所 (OOM リスク)

全データパスが `unbounded_channel()` を使用している。
データ損失はないが、consumer が遅い場合にメモリが無制限に増加し OOM クラッシュのリスクがある。

| # | ファイル | 行 | パス種別 | OOM リスク |
|---|---------|-----|---------|-----------|
| 1 | `src/reader/mod.rs` | 762 | DATA (ReadLoop→DecodeLoop) | **高** — 1MB バッファが蓄積 |
| 2 | `src/merger/mod.rs` | 303 | DATA (Receiver→Sender) | 中 |
| 3 | `src/recorder/mod.rs` | 586 | DATA (Receiver→Writer) | **高** — fsync 遅延で蓄積 |
| 4 | `src/monitor/mod.rs` | 655 | CONTROL (HTTP→Histogram) | 低 |
| 5 | `src/monitor/mod.rs` | 656 | DATA (Receiver→Histogram) | 低 |
| 6 | `src/data_sink/mod.rs` | 296 | DATA (Receiver→Processor) | 中 |

**修正案:** bounded channel + backpressure

- **Reader (spawn_blocking 内):** `channel(1000)` + `try_send()` + retry loop (1ms sleep)
  - `blocking_send()` は macOS で TLS fatal error を起こすため使用禁止
- **Merger/Recorder/Monitor (async 内):** `channel(1000)` + `.send().await` (async backpressure)
- 容量 1000 は ~数十秒分のバッファ。波形ありの場合は要調整。

### 2.2 データドロップ違反 (3件)

#### 違反 1 (深刻度: 高) — Merger ZMQ PUB 送信失敗でデータ消失

**ファイル:** `src/merger/mod.rs` L494-501

```rust
match socket.send(msg).await {
    Ok(()) => {
        ext_state.atomic_stats.record_sent();
    }
    Err(e) => {
        warn!(error = %e, "Failed to send message");
        // ← メッセージは消失。record_drop() は定義されているが呼ばれていない
    }
}
```

ZMQ PUB 送信エラー時にメッセージが消失する。`record_drop()` メソッドが定義 (L136-137) されているが `#[allow(dead_code)]` で未使用。

**修正案:** リトライ or バッファリング。ZMQ PUB は subscriber がいない場合に失敗しうるが、
メインデータパイプラインなので最低限 `record_drop()` を呼び、オペレータに通知すべき。

#### 違反 2 (深刻度: 中) — Event Bridge ZMQ PUB 送信失敗でデータ消失

**ファイル:** `src/bin/event_bridge.rs` L132-134

```rust
if let Err(e) = pub_socket.send(zmq_msg).await {
    warn!(error = %e, "Failed to send message");
    // ← メッセージは消失
}
```

Merger と同じパターン。

**修正案:** 同上。

#### 違反 3 (深刻度: 中) — DataSink EOS シグナル消失

**ファイル:** `src/data_sink/mod.rs` L436

```rust
let _ = tx.send(ProcessorMessage::Eos { source_id });
// ← `let _` で EOS 送信エラーを無視。Data の場合 (L428) は .is_err() でチェックしている。
```

EOS (End-of-Stream) は Processor にストリーム終了を通知する重要な制御信号。
`Data` 送信 (L428) は `.is_err()` で break するが、`EOS` は `let _` で無視。

**修正案:** Data と同じく `.is_err()` チェックで break。

### 2.3 正常なパターン

以下は問題なし:

- **Reader ZMQ PUB:** `?` 演算子でエラー伝播 (L700)
- **Recorder channel:** `.is_err()` + break (L745, L752)
- **watch channel:** `let _` は idiomatic (最新値のみ保持)
- **oneshot channel:** `let _` は idiomatic (受信側切断は正常)
- **shutdown broadcast:** `let _` は正常

---

## 3. 不要な Clone/コピー精査結果

### 3.1 (深刻度: 高) Reader `convert_event()` — 6 Vec clone

**ファイル:** `src/reader/mod.rs` L364-379, L689

```rust
// L364: 参照で受け取る → clone 必須
fn convert_event(event: &EventData) -> CommonEventData {
    CommonWaveform {
        analog_probe1: wf.analog_probe1.clone(),  // Vec<i16>, 1-8 KB
        analog_probe2: wf.analog_probe2.clone(),  // Vec<i16>, 1-8 KB
        digital_probe1: wf.digital_probe1.clone(), // Vec<u8>, 64-512 B
        digital_probe2: wf.digital_probe2.clone(), // Vec<u8>
        digital_probe3: wf.digital_probe3.clone(), // Vec<u8>
        digital_probe4: wf.digital_probe4.clone(), // Vec<u8>
    }
}

// L689: 参照イテレータ → events は消費されない
for event in &events {
    batch.push(Self::convert_event(event));
}
```

**影響:** 波形あり 100kHz × ~18 KB/event = **1.8 GB/sec の不要メモリコピー**

C++ 比喩: `std::vector` を毎回コピーしている。`std::move()` 相当の最適化が可能。

**修正案:**

```rust
// Step 1: 消費イテレータに変更
let n_events = events.len();
for event in events {  // events は consumed (所有権移動)
    batch.push(Self::convert_event(event));
}

// Step 2: シグネチャ変更 (owned)
fn convert_event(event: EventData) -> CommonEventData {
    if let Some(wf) = event.waveform {
        CommonWaveform {
            analog_probe1: wf.analog_probe1,   // move, ゼロコスト
            analog_probe2: wf.analog_probe2,   // move
            digital_probe1: wf.digital_probe1, // move
            // ... 以下同様
        }
    }
}
```

### 3.2 (深刻度: 中) Reader `read_data()` — 毎回 1MB Vec 確保

**ファイル:** `src/reader/caen/handle.rs` L618

```rust
let mut data = vec![0u8; buffer_size];  // 毎回 1MB 確保
data.truncate(size);  // len は縮小するが capacity は 1MB のまま
```

**影響:** 100kHz でこの関数が呼ばれるたびに 1MB の heap allocation が発生。
さらに `truncate()` は `capacity` を解放しないので、channel に 1MB の Vec が蓄積。

**修正案 (2段階):**

```rust
// Fix A: shrink_to_fit() 追加 (最小変更)
data.truncate(size);
data.shrink_to_fit();  // 1MB → 実データサイズに縮小

// Fix B: バッファ再利用 (より効率的)
// read_loop 内で事前確保し、出力は data[..size].to_vec() でコピー
let mut buffer = vec![0u8; config.buffer_size];
loop {
    buffer.resize(config.buffer_size, 0);
    // FFI 呼び出し...
    let output = buffer[..size].to_vec();  // 実データサイズのみ確保
}
```

### 3.3 (深刻度: 中) Monitor — 波形を毎イベント clone

**ファイル:** `src/monitor/mod.rs` L233, L240-242

```rust
// L233: 最新波形を保存する際に毎回 clone
waveform: wf.clone(),  // 2-18 KB/event

// L240: バッチを参照で受け取る → clone 必須
pub fn process_batch(&mut self, batch: &EventDataBatch) {
    for event in &batch.events {
        self.process_event(event);
    }
}
```

**修正案:** `process_batch()` を `batch: EventDataBatch` (owned) に変更し、
最後のイベントの波形だけ move。

### 3.4 (深刻度: 中) Merger — `Bytes::copy_from_slice`

**ファイル:** `src/merger/mod.rs` L437

```rust
let raw_bytes: Bytes = Bytes::copy_from_slice(&data);
```

ZMQ メッセージ全体をコピー。`tmq::Message` の内部バッファを `Bytes` に変換する際の制約。
現状の設計 (zero-copy forwarding) は既に最適。tmq API の制限。

### 3.5 (深刻度: 中) 重複 Waveform 型

**ファイル:**
- `src/reader/decoder/common.rs` — `decoder::Waveform`
- `src/common/mod.rs` — `common::Waveform`

ほぼ同一の構造体が2つ存在し、`convert_event()` という変換関数が必要になっている。
型を統一すれば `convert_event()` 自体が不要になる。

**修正案:** decoder が `common::Waveform` を直接使用。
または `From<decoder::Waveform> for common::Waveform` を move ベースで実装。

### 3.6 (深刻度: 低) Monitor — ヒストグラムスナップショット clone

**ファイル:** `src/monitor/mod.rs` L272

```rust
histograms: self.histograms.clone(),  // 32ch × 65536 bins × 8B = 16 MB
```

HTTP API レスポンス用。データパスではなく HTTP クエリパスなので影響は限定的。

---

## 4. 修正優先度

### Phase A — データ完全性 (最優先)

| # | 修正内容 | ファイル | 影響 |
|---|---------|---------|------|
| A1 | CLAUDE.md ポリシー更新 | `CLAUDE.md` | ドキュメント |
| A2 | Reader: unbounded → bounded channel + try_send retry | `src/reader/mod.rs` | OOM 防止 |
| A3 | Merger: ZMQ 送信エラー時の record_drop() 呼出 | `src/merger/mod.rs` | 可視化 |
| A4 | DataSink: EOS 送信エラーハンドリング統一 | `src/data_sink/mod.rs` | バグ修正 |

### Phase B — パフォーマンス (高優先)

| # | 修正内容 | ファイル | 削減量 |
|---|---------|---------|--------|
| B1 | convert_event: &EventData → EventData (move) | `src/reader/mod.rs` | ~1.8 GB/sec |
| B2 | read_data: shrink_to_fit() 追加 | `src/reader/caen/handle.rs` | チャンネル内メモリ |
| B3 | Monitor: process_batch を owned に変更 | `src/monitor/mod.rs` | 波形 clone 排除 |

### Phase C — 構造改善 (中優先)

| # | 修正内容 | ファイル | 効果 |
|---|---------|---------|------|
| C1 | Waveform 型統一 | decoder/common.rs, common/mod.rs | 変換層排除 |
| C2 | Merger/Recorder/Monitor: bounded channel | 各 mod.rs | OOM 防止 |

### Phase D — 並列化 (Follow-up)

処理が追いつかない場合:
- `FuturesOrdered` + `spawn_blocking` でバッチ単位の並列デコード
- DecoderKind は `Clone` 可能 → worker ごとにクローン
- 順序保証は FuturesOrdered で維持

---

## 5. 検証方法

1. `cargo test` — 既存テスト全パス
2. `cargo clippy` — 警告なし
3. 実機テスト: 100kHz パルサーで Running → メモリが bounded であること確認
4. Stop → Reset → 再 Start でメモリが増加し続けないこと確認
5. 長時間ラン (30 分) でデータ欠落がないことを確認

---

## 6. 注記

### blocking_send() の macOS TLS 問題

`tokio::sync::mpsc::Sender::blocking_send()` は `spawn_blocking` コンテキスト内で macOS の
global allocator TLS デストラクタと衝突し、以下の fatal error を引き起こす:

```
fatal runtime error: the global allocator may not use TLS with destructors, aborting
```

対策: `try_send()` + `std::thread::sleep(1ms)` のリトライループを使用する。

### ZMQ PUB/SUB の特性

ZMQ PUB ソケットは subscriber がいない場合にメッセージを破棄する
(これは ZMQ の設計上の仕様)。この「ドロップ」は ZMQ プロトコルレベルのものであり、
アプリケーションレベルで制御できない。
上記違反1,2 はソケットエラー時の話であり、別の問題。

### CAEN FELib ReadData のバッファ境界チェック欠如

`CAEN_FELib_ReadData()` は渡されたバッファのサイズを一切チェックしない。
高レート + 波形データの場合、1 回の ReadData で数十 MB のデータが返されることがあり、
バッファが小さいと書き込みが境界を超えて **SIGBUS (EXC_BAD_ACCESS)** が発生する。

```
Exception: EXC_BAD_ACCESS (SIGBUS), KERN_PROTECTION_FAILURE
Thread: tokio-runtime-worker
Stack: caen_read_data_raw → CAEN_FELib_ReadData → insert_raw_data → memmove → CRASH
```

対策: `read_data()` の API を変更し、事前確保した大きなバッファ (`64 MB`) を再利用する。
C++ のイディオムでは `static char buf[HUGE]` + reuse に相当。

```rust
// ReadLoop で一度だけ確保
let mut read_buffer: Vec<u8> = vec![0u8; config.buffer_size]; // 64 MB

// 毎回のリードで再利用
match endpoint.read_data(timeout, &mut read_buffer) {
    Ok(Some(raw)) => { /* raw.data は read_buffer[..size].to_vec() */ }
}
```

### DecodeLoop の `?` 演算子によるサイレント終了

`tokio::select!` 内で `?` 演算子を使用すると、エラー発生時に async 関数全体が
即座に終了する。`let _ = decode_handle.await` でエラーを無視していたため、
DecodeLoop が静かに死んでもログに何も残らなかった。

対策: `?` を全て明示的な `match` / `if let Err(e)` に置換し、エラーをログに記録。
`run()` 内のタスク完了ハンドリングも `match` で結果を検査するように変更。

### 波形モードの 1 GbE 帯域制限

VX2730 の `dig2://` 接続は 1 Gbps Ethernet。波形データ付きイベントは ~16.4 KB/event のため、
理論スループット上限は:

```
1,000,000,000 bps / 8 / 16,400 bytes ≈ 7,622 events/sec (理論値)
プロトコルオーバーヘッド込み: ~6,600 events/sec (実測値)
```

これはハードウェア制約であり、ソフトウェアで回避できない。
波形なしモード (~100 bytes/event) では 98 kHz まで確認済みで、ソフトウェアのボトルネックではない。
光リンク (CONET) や record_length 短縮で緩和可能。
