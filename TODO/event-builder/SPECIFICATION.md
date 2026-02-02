# Event Builder 仕様書

**Version:** 0.4.0
**Date:** 2026-02-02
**Status:** Implementation Phase 7 (Time Slice)

> **Note:** Phase 1-6 では Moving Time Window 方式で実装されたが、
> Phase 7 で本仕様書の Time Slice 方式に移行中。

---

## 1. 概要

### 1.1 目的

核物理実験（DELILA）のDAQシステム向けイベントビルダー。
複数デジタイザからの非同期ヒットデータを時間順に整列し、
コインシデンスウィンドウ内のヒットをイベントとして構築する。

### 1.2 動作モード

| モード | 入力 | 出力 | 用途 | 状態 |
|--------|------|------|------|------|
| オンライン | ZeroMQ (Event Bridge) | ROOT + ヒストグラム | リアルタイム処理 | **Phase 1 (今回)** |
| オフライン | ROOTファイル | ROOTファイル | 過去データの再解析 | 将来拡張 |

### 1.3 要求仕様サマリ

| 項目 | 値 |
|------|-----|
| 処理レート（オンライン） | 2 MHz |
| 許容レイテンシ | 10秒 〜 1分 |
| メモリ使用量上限 | 10 GB |
| コインシデンスウィンドウ | ±500 ns（設定可能） |
| 最大到着遅延 | 1秒未満 |

---

## 2. 入力データ

### 2.1 ヒットデータ構造

ELIFANT-Event (C++) の構造体に基づく：

```cpp
struct HitData {
    unsigned char Mod;      // モジュールID (0-10)
    unsigned char Ch;       // チャンネルID (0-15)
    double FineTS;          // タイムスタンプ [ns]
    uint16_t ChargeLong;    // エネルギー値 (ADC) - 長ゲート
    uint16_t ChargeShort;   // エネルギー値 (ADC) - 短ゲート (PSD用)
};
```

**Rust表現:**

```rust
/// 1ヒットのデータ
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    /// モジュールID (0-10)
    pub module: u8,
    
    /// チャンネルID (0-15)
    pub channel: u8,
    
    /// タイムスタンプ [ns]
    pub timestamp_ns: f64,
    
    /// エネルギー値 - 長ゲート積分 (ADC units)
    pub energy: u16,
    
    /// エネルギー値 - 短ゲート積分 (ADC units)
    /// PSD (Pulse Shape Discrimination) に使用
    pub energy_short: u16,
}
```

**メモリレイアウト（参考）:**

```
フィールド        サイズ    累積
─────────────────────────────
module            1 byte    1
channel           1 byte    2
(padding)         6 bytes   8
timestamp_ns      8 bytes   16
energy            2 bytes   18
energy_short      2 bytes   20
(padding)         4 bytes   24
─────────────────────────────
合計: 24 bytes（アライメント込み）
```

### 2.2 データソース

#### オンライン: Event Bridge (Phase 1)

- **プロトコル:** ZeroMQ SUB/PUB パターン
- **データ形式:** 固定バイナリフォーマット (14 bytes/hit, パディングなし)
- **詳細仕様:** `docs/event_bridge_wire_format.md`
- **デフォルトアドレス:** `tcp://localhost:5600`
- **到着順序:** Merger がタイムソート済み (バッチ内は時間順保証)

#### オフライン: ROOTファイル (✅ 実装済み)

- **TTree構造:**
  - Branch: `Mod` (UChar_t)
  - Branch: `Ch` (UChar_t)
  - Branch: `FineTS` (Double_t) - 単位: **ns** (delila-rs) / **ps** (ELIFANT レガシー)
  - Branch: `ChargeLong` (UShort_t)
  - Branch: `ChargeShort` (UShort_t)

- **典型的なファイル:** 数十秒分のデータ、約4100万ヒット/ファイル

- **タイムスタンプ単位:**
  - **delila-rs 出力:** ns (ナノ秒) — 新規実装の標準
  - **ELIFANT レガシー:** ps (ピコ秒) — 旧システムのデータ
  - 実装では `--timestamp-unit` オプションで指定可能にする予定

### 2.3 システム構成

```
総チャンネル数: 11モジュール × 16チャンネル = 176チャンネル

Module 0:  Ch 0-15
Module 1:  Ch 0-15
...
Module 10: Ch 0-15
```

---

## 3. チャンネル設定

### 3.1 設定ファイル形式 (JSON)

```json
{
  "timestamp_unit": "ns",
  "coincidence_window_ns": 500.0,
  "buffer_delay_ns": 1000000000.0,
  "slice_duration_ns": 10000000.0,
  
  "channels": [
    {
      "module": 0,
      "channel": 0,
      "name": "HPGe_0",
      "detector_type": "HPGe",
      "is_trigger": true,
      "tags": ["HPGe", "Trigger"],
      "ac_pair": null
    },
    {
      "module": 0,
      "channel": 1,
      "name": "AC_0",
      "detector_type": "AC",
      "is_trigger": false,
      "tags": ["AC"],
      "ac_pair": null
    },
    {
      "module": 1,
      "channel": 0,
      "name": "Si_E_0",
      "detector_type": "Si",
      "is_trigger": false,
      "tags": ["Si", "E_Sector"],
      "ac_pair": [0, 1]
    }
  ]
}
```

### 3.2 検出器タイプ

| タイプ | 説明 | 用途 |
|--------|------|------|
| HPGe | 高純度ゲルマニウム | ガンマ線検出、主トリガー |
| Si | シリコン検出器 | 荷電粒子 (E/dE) |
| AC | アクティブシールド | バックグラウンド除去 |
| PMT | 光電子増倍管 | シンチレータ読み出し |

### 3.3 トリガーチャンネル

- JSON設定で `"is_trigger": true` と指定
- 複数トリガーチャンネル可
- トリガーヒットを中心にイベント構築

---

## 4. イベント構築アルゴリズム

### 4.1 処理フロー

```
┌─────────────────────────────────────────────────────────────────┐
│                        Event Builder                            │
│                                                                 │
│  ┌──────────┐    ┌──────────┐    ┌──────────┐    ┌──────────┐ │
│  │  Input   │───▶│  Time    │───▶│ Timeslice│───▶│  Event   │ │
│  │  Buffer  │    │  Sort    │    │ Builder  │    │  Output  │ │
│  └──────────┘    └──────────┘    └──────────┘    └──────────┘ │
│       ▲                               │                        │
│       │                               ▼                        │
│  [到着遅延吸収]              [コインシデンス検出]              │
│   最大1秒                      ±500 ns                         │
└─────────────────────────────────────────────────────────────────┘
```

### 4.2 時間ソート

**目的:** 非同期到着するヒットを時間順に整列

**アルゴリズム:**
1. ヒットをBTreeMap<timestamp, Vec<Hit>>に挿入
2. `current_time - buffer_delay` より古いヒットを取り出し
3. 取り出したヒットは時間順が保証される

**パラメータ:**
- `buffer_delay`: 1秒（到着遅延の最大値 + マージン）

### 4.3 タイムスライス

**目的:** 連続データを独立処理可能な単位に分割

**採用方式: オーバーラップ方式**

```
時間軸 ─────────────────────────────────────────────────────────▶

Slice N:   [========= Core =========][= Overlap =]
                                      ↑
                                 coincidence_window

Slice N+1:                     [= Overlap =][======= Core =======]
```

**境界処理ルール:**
- トリガーがCore領域内 → このスライスで処理
- トリガーがOverlap領域内 → 次スライスで処理（重複防止）

**パラメータ:**
- `slice_duration`: 10 ms（調整可能）
- `overlap`: 500 ns（= coincidence_window）

**設計判断の記録:**

| 方式 | 説明 | 採用 |
|------|------|------|
| **オーバーラップ方式** | Core + Overlap領域で分割、Overlap内トリガーは次スライス | ✓ 採用 |
| 境界またぎマージ | 不完全イベントを次スライスとマージ | 複雑 |
| グローバルソート | タイムスライスなし、全データ順次処理 | 並列化困難 |
| イベント単位持ち越し | 必要ヒットのみ次スライスへ | 複雑 |

**採用理由:**
- CBM/FLESで実績あり
- 実装がシンプル
- 重複なし保証が明確

### 4.4 コインシデンス検出

**アルゴリズム:**

```
for each hit in sorted_hits:
    if hit is not trigger:
        continue
    
    if hit.timestamp >= slice.core_end:
        continue  // Overlap領域のトリガーは次スライスで処理
    
    event = new Event(trigger_time = hit.timestamp)
    
    // 後方検索: より早い時刻のヒットを収集
    for prev_hit in hits_before(hit):
        time_diff = hit.timestamp - prev_hit.timestamp
        if time_diff > coincidence_window:
            break  // ウィンドウ外
        
        if prev_hit is trigger:
            // 先行トリガーあり → このトリガーはスキップ
            discard event
            goto next_hit
        
        event.add(prev_hit)
    
    // 前方検索: より遅い時刻のヒットを収集  
    for next_hit in hits_after(hit):
        time_diff = next_hit.timestamp - hit.timestamp
        if time_diff > coincidence_window:
            break  // ウィンドウ外
        
        if next_hit is trigger:
            break  // 後続トリガーで停止（そのヒットは後続イベントへ）
        
        event.add(next_hit)
    
    output(event)

next_hit:
```

### 4.5 ダブルカウント防止

**採用方式: 時間優先方式**

```
時間軸 ──────────────────────────────────────────▶
              T1        T2
              ●─────────●
              │◀─window─▶│
              
T1がイベント構築 ✓
T2はスキップ（T1が先行するため）
```

**ロジック:**
- 後方検索で先行トリガーを発見 → 現在のトリガーをスキップ
- 前方検索で後続トリガーを発見 → 検索停止（後続ヒットは次イベントへ）

**設計判断の記録:**

| 方式 | 説明 | 採用 |
|------|------|------|
| **時間優先** | 時刻が早いトリガーを採用 | ✓ 採用 |
| ID優先 | IDが大きい/小さい方を採用（ELIFANT-Event現行） | 物理的意味なし |
| エネルギー優先 | エネルギーが高い方を採用 | 常に正しいとは限らない |
| 重複許容 | 両方構築、後処理で除去 | データ量増加 |

**採用理由:**
- 物理的に自然（先に起きた事象を採用）
- 実装がシンプル
- 恣意性がない

### 4.6 AC（アクティブシールド）判定

**目的:** HPGeヒットがACと同時計数した場合にフラグを設定

**アルゴリズム:**
1. チャンネル設定から `ac_pair` を取得
2. イベント内でペアチャンネルのヒットを検索
3. 時間差がコインシデンスウィンドウ内なら `with_ac = true`

---

## 5. 出力データ

### 5.1 構築済みイベント構造

```rust
/// 構築済みイベント
#[derive(Debug, Clone)]
pub struct BuiltEvent {
    /// イベント通し番号
    pub event_id: u64,
    
    /// トリガー時刻 [ns]
    pub trigger_time: f64,
    
    /// トリガーチャンネル
    pub trigger_module: u8,
    pub trigger_channel: u8,
    
    /// イベント内ヒット
    pub hits: Vec<EventHit>,
}

/// イベント内の1ヒット
#[derive(Debug, Clone)]
pub struct EventHit {
    pub module: u8,
    pub channel: u8,
    pub energy: u16,
    pub energy_short: u16,
    
    /// トリガー基準の相対時刻 [ns]
    pub relative_time: f64,
    
    /// ACとの同時計数フラグ
    pub with_ac: bool,
}
```

### 5.2 出力方式

#### システムアーキテクチャ

```
delila-rs パイプライン (Rust)              C++ Event Builder (別リポジトリ)

┌────────┐   ┌────────┐   ┌────────┐
│ Reader │──▶│ Merger │──▶│Recorder│
└────────┘   └───┬────┘   └────────┘
                 │ PUB (MessagePack)
                 ├──────────▶ Monitor
                 │
                 ▼
            ┌──────────┐  PUB (固定バイナリ)  ┌───────────────────┐
            │  Event   │────────────────────▶│  Event Builder    │
            │  Bridge  │  14 bytes/hit       │  (C++)            │
            │  (Rust)  │                     │  ├─ Time Sort     │
            └──────────┘                     │  ├─ Coincidence   │
                                             │  ├─ ROOT Writer   │
                                             │  └─ THttpServer   │
                                             └───────────────────┘
```

**設計方針:**
- Event Bridge (Rust) が Merger の MessagePack を固定バイナリに変換
- イベント構築・ROOT出力・ヒストグラム表示は全て C++ で実装
- Merger のゼロコピー設計を維持（Bridge は独立プロセス）
- ワイヤフォーマット仕様: `docs/event_bridge_wire_format.md`
- Bridge 実装計画: `TODO/event-builder/IMPLEMENTATION.md`

**Event Bridge の入力:**
- Merger PUB (MessagePack: `Message::Data(EventDataBatch)`)
- デフォルト: `tcp://localhost:5556`

**Event Bridge の出力:**
- 固定バイナリフォーマット (PUB/SUB)
- デフォルト: `tcp://*:5600`
- フォーマット詳細: `docs/event_bridge_wire_format.md`

#### ROOT出力（C++ Event Builder 内）

**責務:**
- Event Bridge の PUB から固定バイナリを受信
- コインシデンス判定後の BuiltEvent を ROOT ファイルに書き込み

**TTree構造:**

```cpp
TTree* tree = new TTree("Events", "Built Events");

ULong64_t event_id;
Double_t  trigger_time;
UChar_t   trigger_mod;
UChar_t   trigger_ch;
UInt_t    n_hits;

// 可変長配列（最大ヒット数を想定）
static const int MAX_HITS = 256;
UChar_t   hit_mod[MAX_HITS];
UChar_t   hit_ch[MAX_HITS];
UShort_t  hit_energy[MAX_HITS];
UShort_t  hit_energy_short[MAX_HITS];
Double_t  hit_time[MAX_HITS];
Bool_t    hit_with_ac[MAX_HITS];

tree->Branch("EventID",      &event_id,     "EventID/l");
tree->Branch("TriggerTime",  &trigger_time, "TriggerTime/D");
tree->Branch("TriggerMod",   &trigger_mod,  "TriggerMod/b");
tree->Branch("TriggerCh",    &trigger_ch,   "TriggerCh/b");
tree->Branch("NHits",        &n_hits,       "NHits/i");
tree->Branch("HitMod",       hit_mod,       "HitMod[NHits]/b");
tree->Branch("HitCh",        hit_ch,        "HitCh[NHits]/b");
tree->Branch("HitEnergy",    hit_energy,    "HitEnergy[NHits]/s");
tree->Branch("HitEnergyShort", hit_energy_short, "HitEnergyShort[NHits]/s");
tree->Branch("HitTime",      hit_time,      "HitTime[NHits]/D");
tree->Branch("HitWithAC",    hit_with_ac,   "HitWithAC[NHits]/O");
```

#### ヒストグラム表示（C++ Event Builder 内）

**責務:**
- BuiltEvent からヒストグラムを更新
- THttpServerでWeb公開

**ヒストグラム種類:**
- エネルギースペクトル（チャンネル別）
- 時間分布（相対時刻）
- イベントレート（時系列）
- ヒットマルチプリシティ分布

**THttpServer設定:**

```cpp
THttpServer* server = new THttpServer("http:8082");
server->Register("/Histograms", histDir);
```

---

## 6. 性能要件

### 6.1 処理レート

| 条件 | 要求 |
|------|------|
| オンライン入力レート | 2 MHz (2,000,000 hits/sec) |
| イベント構築レート | TBD（トリガーレートに依存） |

### 6.2 レイテンシ

```
入力 ──────────────────────────────────────────▶ 出力
      │                                      │
      │◀── buffer_delay ──▶│◀─ processing ─▶│
      │      (1 sec)       │    (< 1 sec)    │
      │                                      │
      │◀────── 合計: 10秒 〜 1分 許容 ───────▶│
```

- バッファ遅延: 1秒（到着遅延吸収）
- 処理時間: 入力レートに追従できること
- 許容範囲が広いため、バッファを大きく取れる

### 6.3 メモリ使用量

**見積もり:**

```
Hit構造体サイズ: 24バイト（アライメント込み）

バッファ遅延中のヒット数:
  2 MHz × 1秒 = 2,000,000 hits

メモリ使用量:
  2,000,000 × 24 bytes = 48 MB

安全マージン (10倍):
  480 MB

上限 10 GB に対して十分な余裕あり
```

---

## 7. エラー処理

### 7.1 データ異常

| 異常 | 検出方法 | 対処 |
|------|----------|------|
| タイムスタンプ逆転 | 前回より小さい値 | 警告ログ、スキップ |
| 無効なモジュール/チャンネル | 範囲外の値 | 警告ログ、スキップ |
| タイムスタンプギャップ | 大きな時間ジャンプ | 情報ログ、継続 |

### 7.2 システム異常

| 異常 | 検出方法 | 対処 |
|------|----------|------|
| メモリ不足 | バッファサイズ監視 | バックプレッシャー発行 |
| 入力停止 | タイムアウト | フラッシュ処理 |
| 出力エラー | I/O エラー | リトライ、ログ |
| C++プロセス異常 | ZeroMQ接続監視 | 再接続、警告 |

---

## 8. 設定パラメータ一覧

| パラメータ | 型 | デフォルト | 説明 |
|------------|-----|-----------|------|
| `timestamp_unit` | string | "ns" | タイムスタンプ単位 ("ns" or "ps") |
| `coincidence_window_ns` | f64 | 500.0 | コインシデンスウィンドウ [ns] |
| `buffer_delay_ns` | f64 | 1e9 | バッファ遅延 [ns] (1秒) |
| `slice_duration_ns` | f64 | 1e7 | タイムスライス長 [ns] (10ms) |
| `max_buffer_hits` | usize | 10_000_000 | 最大バッファヒット数 |
| `zmq_output_endpoint` | string | "tcp://*:5555" | ZeroMQ出力エンドポイント |

---

## 9. 将来の拡張

### 9.1 L2フィルタリング

- Counter/Flag/Acceptance による条件フィルタ
- 設計は `12_event_builder_design.md` に記載済み
- Phase 2 以降で実装

### 9.2 並列処理

- タイムスライス単位での並列イベント構築
- Rayon による自動並列化
- 性能が不足した場合に検討

---

## 10. 用語集

| 用語 | 説明 |
|------|------|
| Hit | デジタイザからの1検出信号 |
| Event | コインシデンスウィンドウ内のヒット集合 |
| Trigger | イベント構築の基準となるヒット |
| Timeslice | 独立処理可能な時間区間 |
| Core | タイムスライスの主要部分（トリガー処理対象） |
| Overlap | タイムスライスの重複部分（境界イベント用） |
| AC | Active Shield（アクティブシールド） |
| Coincidence | 時間的に近接した複数ヒットの同時検出 |
| PSD | Pulse Shape Discrimination（波形弁別） |

---

## 変更履歴

| 日付 | バージョン | 変更内容 |
|------|-----------|----------|
| 2025-01-27 | 0.1.0 | 初版作成 |
| 2025-01-27 | 0.2.0 | energy_short追加、ROOT/ヒストグラム方式決定、設計判断記録追加 |
| 2026-01-27 | 0.3.0 | アーキテクチャ決定: Event Bridge (Rust) 経由の固定バイナリ方式採用。オンラインモード優先。C++ Event Builder は全て C++ 実装 (別リポジトリ)。ワイヤフォーマット仕様を `docs/event_bridge_wire_format.md` に分離 |
| 2026-02-02 | 0.4.0 | **Rust 実装 Phase 7**: Time Slice 方式への移行開始。Phase 1-6 では Moving Time Window で実装完了済み |
