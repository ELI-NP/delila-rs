# Event Bridge ワイヤフォーマット仕様書

**Version:** 1.0.0
**Date:** 2026-01-27
**Status:** ⚠️ **DEPRECATED (2026-05-19)** — see [EB SPEC v0.5.1 § 2.4](../TODO/event-builder/SPECIFICATION.md)

> ## なぜ deprecated か
>
> 本ドキュメントは当初「delila-rs (Rust) → 別リポジトリの C++ Event Builder」を
> 結ぶための 14 B 固定バイナリ wire format を定義していました。SPEC v0.3 以降で
> C++ EB 経路は撤回され、Event Builder は **Rust 側で完結**するようになり、
> Online EB は **Merger PUB に直接 subscribe**（MessagePack `EventDataBatch`）
> する形に移行しました（[`ZmqHitSource`](../src/event_builder/source.rs) +
> [`EventBuilderPipeline`](../src/event_builder/pipeline.rs)）。
>
> 既存の `event_bridge` バイナリ (`src/bin/event_bridge.rs`) は当面残しますが、
> 新規連携は MessagePack 経由を推奨します。本ドキュメントは Phase 5 完了時に
> 削除予定。

---

## 1. 概要

本ドキュメントは、delila-rs パイプライン (Rust) と C++ Event Builder 間の
ZeroMQ 通信プロトコルを定義する。

### 1.1 アーキテクチャ

```
delila-rs パイプライン (Rust)              C++ (別リポジトリ)
┌────────┐   ┌────────┐   ┌────────┐
│ Reader │──▶│ Merger │──▶│Recorder│
└────────┘   └───┬────┘   └────────┘
                 │ PUB (MessagePack)
                 ├──────────▶ Monitor
                 │
                 ▼
            ┌──────────┐   PUB (本仕様)   ┌───────────────┐
            │  Event   │────────────────▶│ Event Builder │
            │  Bridge  │  固定バイナリ     │ (C++)         │
            │  (Rust)  │                  └───────────────┘
            └──────────┘
```

Event Bridge は Merger の MessagePack 出力を受信し、
本仕様の固定バイナリフォーマットに変換して PUB する。

### 1.2 トランスポート

| 項目 | 値 |
|------|-----|
| プロトコル | ZeroMQ PUB/SUB |
| デフォルトアドレス | `tcp://*:5600` (Bridge PUB) |
| バイトオーダー | Little-Endian |
| エンコーディング | 固定長バイナリ (パディングなし) |

---

## 2. メッセージフォーマット

### 2.1 メッセージヘッダ

全メッセージは共通の5バイトヘッダで始まる:

```
Offset  Size  Type    Field       Description
──────────────────────────────────────────────
0       1     u8      msg_type    メッセージ種別
1       4     u32     n_hits      ヒット数 (Data) / 0 (制御)
──────────────────────────────────────────────
Total: 5 bytes
```

### 2.2 メッセージ種別

| msg_type | 名称 | ペイロード | 説明 |
|----------|------|-----------|------|
| `0x01` | Data | Header + Hit[N] | ヒットデータバッチ |
| `0x02` | EndOfStream | Header のみ (n_hits=0) | DAQ 停止通知 |
| `0x03` | Heartbeat | Header のみ (n_hits=0) | 生存確認 |

### 2.3 Hit 構造体

Data メッセージ (msg_type=0x01) のペイロード。ヘッダ直後に N 個連続する。

```
Offset  Size  Type    Field           Description
──────────────────────────────────────────────────────
0       1     u8      module          モジュールID (0-255)
1       1     u8      channel         チャンネルID (0-255)
2       2     u16     energy          エネルギー (長ゲート, ADC)
4       2     u16     energy_short    エネルギー (短ゲート, ADC)
6       8     f64     timestamp_ns    タイムスタンプ [ns] (IEEE 754)
──────────────────────────────────────────────────────
Total: 14 bytes per hit
```

### 2.4 メッセージサイズ

```
Data メッセージ:     5 + 14 × N  bytes
制御メッセージ:      5            bytes

例: 1000 ヒットバッチ = 5 + 14,000 = 14,005 bytes
```

---

## 3. 除外フィールド

Rust 側の `EventData` 構造体から以下のフィールドは**転送しない**:

| フィールド | 型 | 除外理由 |
|-----------|-----|---------|
| `flags` | u64 | Event Builder で不要 (デコーダ内部フラグ) |
| `waveform` | Option\<Waveform\> | 波形データは通常 None。必要時は別チャンネルで |

将来 flags が必要になった場合、Hit 構造体を拡張して msg_type を新設する
(後方互換性のため既存の 0x01 は変更しない)。

---

## 4. C++ 受信実装例

### 4.1 ヘッダとヒット構造体

```cpp
#include <cstdint>
#include <zmq.hpp>
#include <vector>

// パディングなしのパック構造体
#pragma pack(push, 1)
struct BridgeHeader {
    uint8_t  msg_type;
    uint32_t n_hits;
};

struct Hit {
    uint8_t  module;
    uint8_t  channel;
    uint16_t energy;
    uint16_t energy_short;
    double   timestamp_ns;
};
#pragma pack(pop)

static_assert(sizeof(BridgeHeader) == 5, "Header must be 5 bytes");
static_assert(sizeof(Hit) == 14, "Hit must be 14 bytes");
```

### 4.2 受信ループ

```cpp
zmq::context_t ctx;
zmq::socket_t sub(ctx, zmq::socket_type::sub);
sub.connect("tcp://localhost:5600");
sub.set(zmq::sockopt::subscribe, "");  // 全メッセージ受信

while (true) {
    zmq::message_t msg;
    auto result = sub.recv(msg, zmq::recv_flags::none);
    if (!result) break;

    if (msg.size() < sizeof(BridgeHeader)) continue;

    auto* header = static_cast<const BridgeHeader*>(msg.data());

    switch (header->msg_type) {
    case 0x01: {  // Data
        uint32_t n = header->n_hits;
        size_t expected = sizeof(BridgeHeader) + sizeof(Hit) * n;
        if (msg.size() < expected) {
            // サイズ不整合 — スキップ
            break;
        }
        auto* hits = reinterpret_cast<const Hit*>(
            static_cast<const char*>(msg.data()) + sizeof(BridgeHeader)
        );
        // hits[0..n-1] を処理
        for (uint32_t i = 0; i < n; ++i) {
            process_hit(hits[i]);
        }
        break;
    }
    case 0x02:  // EndOfStream
        flush_and_close();
        break;
    case 0x03:  // Heartbeat
        // 生存確認 — 必要に応じて処理
        break;
    }
}
```

### 4.3 バイトオーダーに関する注意

本フォーマットは **Little-Endian** 固定。
x86/x86_64 および ARM (little-endian mode) では変換不要。
Big-Endian アーキテクチャでは各フィールドのバイトスワップが必要
（DELILA で使用する環境は全て LE のため、実質的に問題にならない）。

---

## 5. Rust 送信実装例

### 5.1 エンコード関数

```rust
use crate::common::EventData;

const MSG_DATA: u8 = 0x01;
const MSG_EOS: u8 = 0x02;
const MSG_HEARTBEAT: u8 = 0x03;
const HIT_SIZE: usize = 14;

/// EventData バッチを固定バイナリフォーマットにエンコード
pub fn encode_data(events: &[EventData]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(5 + events.len() * HIT_SIZE);

    // Header
    buf.push(MSG_DATA);
    buf.extend_from_slice(&(events.len() as u32).to_le_bytes());

    // Hits
    for ev in events {
        buf.push(ev.module);
        buf.push(ev.channel);
        buf.extend_from_slice(&ev.energy.to_le_bytes());
        buf.extend_from_slice(&ev.energy_short.to_le_bytes());
        buf.extend_from_slice(&ev.timestamp_ns.to_le_bytes());
    }

    buf
}

/// 制御メッセージ (EOS/Heartbeat) をエンコード
pub fn encode_control(msg_type: u8) -> Vec<u8> {
    let mut buf = Vec::with_capacity(5);
    buf.push(msg_type);
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf
}
```

---

## 6. パフォーマンス見積もり

| 条件 | 値 |
|------|-----|
| 入力レート | 2 MHz (2,000,000 hits/sec) |
| バッチサイズ (典型) | ~1000 hits |
| メッセージサイズ (典型) | ~14 KB |
| メッセージレート | ~2000 msg/sec |
| スループット | ~28 MB/sec |

ZeroMQ の TCP スループットは数 GB/sec のため、ボトルネックにならない。
エンコード処理は memcpy 相当のため、CPU 負荷も無視できる。

---

## 変更履歴

| 日付 | バージョン | 変更内容 |
|------|-----------|----------|
| 2026-01-27 | 1.0.0 | 初版作成 |
