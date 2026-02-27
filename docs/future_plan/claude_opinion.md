# DELILA-RS 再設計についての考察

**Date:** 2026-02-26
**Context:** 現行設計の実運用経験を踏まえ、ゼロから再設計するなら何を変えるか

---

## 1. Cargo Workspace に分割（最大のインパクト）

現状: **1 crate + 30 binaries**。全部再コンパイルが痛い。

```
delila-rs/
  crates/
    delila-types/       # Hit, EventData, Message, State — 依存ゼロ、変更少
    delila-caen/        # CAEN FFI — unsafe はここだけに隔離
    delila-transport/   # ZMQ wrapper + framing + HWM policy
    delila-reader/      # ReadLoop + DecodeLoop + firmware decoders
    delila-pipeline/    # Merger (thin)
    delila-recorder/    # File I/O + .delila format
    delila-monitor/     # Histogram/Waveform + REST
    delila-eb/          # Event Builder (chunk_builder + pipeline)
    delila-operator/    # Orchestration + Web UI serve
    delila-config/      # TOML/JSON parse + validation
```

**利点:**
- `delila-caen` を触らなければ Reader 以外は再コンパイル不要
- `unsafe` の影響範囲が `delila-caen` crate 内に限定される（`cargo audit` が効く）
- CI で crate ごとの並列テスト
- 各 crate が独立して semver を持てる

C++ で言えば、今は全部 `.cpp` を `g++ -o binary` で一括コンパイルしているのを、`.so` ライブラリに分割する感覚。Rust の workspace はそれをビルドシステムレベルで保証する。

---

## 2. One Binary, Many Nodes（憧れの設計）

各マシンに1つの `delila-rs` バイナリ。config で役割を決め、内部でスレッドとして各コンポーネントを持つ。独立だが協働する。

### デプロイイメージ

```
┌──────────────── Machine A (76, A3818) ──────────────────┐
│                                                          │
│  $ delila-rs --config node_a.toml                        │
│                                                          │
│  ┌─────────┐  ┌─────────┐       ┌─────────┐             │
│  │Reader(0)│  │Reader(1)│  ...  │Reader(5)│  <- threads  │
│  └────┬────┘  └────┬────┘       └────┬────┘             │
│       └────────────┼────────────────┘                    │
│                    v                                      │
│              ┌──────────┐                                │
│              │ DataPort │ -- tcp:5555 ----------------->  │
│              └──────────┘                                │
│              ┌──────────┐                                │
│              │ REST API │ :9090  (自ノードの状態/設定)     │
│              └──────────┘                                │
└──────────────────────────────────────────────────────────┘

┌──────────────── Machine B (147, Storage) ────────────────┐
│                                                          │
│  $ delila-rs --config node_b.toml                        │
│                                                          │
│  ┌──────────┐    ┌──────────┐    ┌──────────┐            │
│  │ Merger   │--->│ Recorder │    │ Monitor  │  <- threads │
│  └──────────┘  | └──────────┘    └──────────┘            │
│   ^ tcp:5555   | ┌──────────┐                            │
│   from node_a  └>│ EB       │                            │
│                   └──────────┘                            │
│  ┌───────────┐                                           │
│  │ Operator  │  <- このノードが coordinator               │
│  │ REST API  │ :9090  (全体制御 + Web UI)                 │
│  └───────────┘                                           │
└──────────────────────────────────────────────────────────┘
```

**単独マシンのとき:**

```
┌──────────────── Standalone (1台で全部) ──────────────────┐
│                                                          │
│  $ delila-rs --config standalone.toml                    │
│                                                          │
│  Reader×1 --ch--> Merger --ch--> Recorder                │
│                          --ch--> Monitor                 │
│                          --ch--> EB                      │
│                                                          │
│  Operator REST API :9090                                 │
│  (全部 in-process、ZMQ なし、シリアライズなし)              │
└──────────────────────────────────────────────────────────┘
```

### Config が全てを決める

```toml
# node_a.toml — デジタイザを持つマシン
[node]
name = "node-a"
role = "source"                    # source | processor | standalone
rest_port = 9090

# このノードが持つデジタイザ
[[digitizers]]
name = "PSD1-990"
firmware = "PSD1"
connection = "usb://990"
config_file = "digitizers/psd1_990.toml"

[[digitizers]]
name = "PSD1-123"
firmware = "PSD1"
connection = "optical://0/0/0"
config_file = "digitizers/psd1_123.toml"

# データ出力先
[output]
destination = "tcp://172.18.4.147:5555"    # 別ノードへ送る

# coordinator の場所（自分が coordinator でなければ）
[coordinator]
url = "http://172.18.4.147:9090"
```

```toml
# node_b.toml — 処理・記録マシン
[node]
name = "node-b"
role = "processor"
rest_port = 9090

# coordinator を兼ねる
[coordinator]
enabled = true

# データ受信
[input]
listen = "tcp://*:5555"

# 内部コンポーネント（持つものだけ書く）
[merger]
enabled = true

[recorder]
enabled = true
output_dir = "./data"

[monitor]
enabled = true
http_port = 8081

[event_builder]
enabled = true
time_calib_file = "config/timeSettings.toml"
ch_settings_file = "config/chSettings.toml"
```

```toml
# standalone.toml — 1台で全部
[node]
name = "standalone"
role = "standalone"
rest_port = 9090

[coordinator]
enabled = true

[[digitizers]]
name = "PSD1-990"
firmware = "PSD1"
connection = "usb://990"
config_file = "digitizers/psd1_990.toml"

# output が無い = 全部 in-process
[merger]
enabled = true

[recorder]
enabled = true
output_dir = "./data"

[monitor]
enabled = true
http_port = 8081
```

**ポイント:** `output.destination` が無ければ in-process チャンネル。あれば ZMQ。同じバイナリ、同じコード。

### ノード間の協調

```
          Coordinator (node-b)
          ┌──────────────────┐
          │   Operator       │
          │                  │
          │ GET /api/status <-------- node-a (定期 heartbeat)
          │                  │
          │ POST /api/start --------> node-a:/api/cmd/start
          │ POST /api/stop  --------> node-a:/api/cmd/stop
          │                  │
          │ Web UI (:9090)   │  <- 人間はここだけ見る
          └──────────────────┘
```

各ノードが **同じ REST API** を持つ。Coordinator は peer ノードの REST API を叩いて制御する。ZMQ REQ/REP コマンドチャンネルは不要になる — **HTTP でコマンドも状態もデータ以外の全てを流す**。

```
各ノード共通 API:
  GET  /api/status          # 自ノードの状態
  GET  /api/metrics         # 自ノードのメトリクス
  POST /api/cmd/configure   # Configure
  POST /api/cmd/arm         # Arm
  POST /api/cmd/start       # Start
  POST /api/cmd/stop        # Stop
  POST /api/cmd/reset       # Reset

Coordinator のみ追加:
  GET  /api/nodes           # 全ノード一覧
  POST /api/run/start       # 全ノード一斉 Start
  POST /api/run/stop        # 全ノード一斉 Stop (pipeline order)
```

### 内部構造（1バイナリの中身）

```rust
struct Node {
    config: NodeConfig,

    // Config に応じて Some/None
    readers: Vec<ReaderHandle>,        // デジタイザごとに1つ
    merger: Option<MergerHandle>,
    recorder: Option<RecorderHandle>,
    monitor: Option<MonitorHandle>,
    event_builder: Option<EbHandle>,
    coordinator: Option<Coordinator>,  // 全体制御
}

impl Node {
    async fn run(config: NodeConfig) -> Result<()> {
        let node = Self::from_config(config)?;
        // 各コンポーネントを spawn
        // channels で接続
        // REST API を起動
        // shutdown signal を待つ
    }
}

fn main() {
    let config = NodeConfig::load("node.toml")?;
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(Node::run(config))
}
```

**バイナリは1つ。** `cargo build --release` で `delila-rs` が1つできる。全マシンに同じバイナリを配る。config だけが違う。

---

## 3. ZMQ PUB/SUB → PUSH/PULL（データパス）

現状の設計負債: **PUB/SUB + HWM=0**。

PUB/SUB は「遅い SUB は勝手にドロップ」が設計思想。それを HWM=0 で無限バッファに変えて「絶対にドロップしない」にしている — ZMQ の設計意図と真逆に使っている。

```
現状:  Reader --PUB/SUB--> Merger --PUB/SUB--> Recorder
                                           \-> Monitor

再設計: Reader --PUSH/PULL--> Merger --PUSH/PULL--> Recorder
                                     \--PUB/SUB --> Monitor (ここだけ)
```

**PUSH/PULL** はバックプレッシャーが組み込まれている。送信側が「相手が詰まったら待つ」。データを絶対に落とさないパスには PUSH/PULL が正しい。

**Monitor だけ PUB/SUB** に残す。Monitor は表示目的 — 遅延しても最新データを見せればよく、落ちても致命的でない。

### Transport 抽象化

同一マシンなら in-process チャンネル、別マシンなら ZMQ:

```rust
enum DataSink<T: Send + Serialize> {
    Channel(crossbeam::Sender<T>),        // 同一プロセス: ゼロコピー
    LocalZmq(ZmqPushSocket),              // ipc:///tmp/delila.sock
    RemoteZmq(ZmqPushSocket),             // tcp://172.18.4.76:5555
}
```

構成パターン:

```
パターンA: 小規模（1台で完結）
  全部 in-process channel。ZMQ なし。transport = "inprocess"

パターンB: 現在の76（1台だがプロセス分離が必要なら）
  ipc:// で高速ローカル通信

パターンC: 分散（複数マシン）
  Machine A (Reader×6) --tcp:PUSH--> Machine B (Merger + Recorder + ...)

パターンD: 大規模分散
  Machine A (Reader×6) --tcp--> Machine C (Merger + Recorder + EB)
  Machine B (Reader×4) --tcp--> Machine C
```

---

## 4. コマンドチャンネル: ZMQ REQ/REP → HTTP REST

ノード間のコマンドは HTTP で統一。各ノードが同じ REST API を持つので、Coordinator は peer の HTTP エンドポイントを叩くだけ。

利点:
- curl でデバッグ可能
- Swagger/OpenAPI で型安全
- ZMQ REQ/REP の「送信→応答待ち」デッドロックリスクが消える
- コマンド頻度は低い（1 Hz 以下）ので HTTP のオーバーヘッドは無問題

---

## 5. Event Builder をパイプラインのファーストクラス市民に

現状: EB は optional feature、後付け。

再設計では:

```
Reader --> Merger --> [Event Builder] --> Writer
                             |
                      [Raw Recorder]  (常に並行保存)
```

EB がパイプラインの中央に位置する。Raw データの保存は EB を通さない別パスで常に保証。

EB を通さない「raw mode」も config 一行で切り替え可能:
```toml
[pipeline]
event_builder = true   # false なら Reader -> Merger -> Writer 直結
```

---

## 変えないもの

| 要素 | 理由 |
|------|------|
| **tokio async runtime** | Rust のエコシステム標準。代替なし |
| **ZMQ (データパス)** | PUSH/PULL に変えるが、ZMQ 自体は正しい選択 |
| **State Machine (5-state)** | CAEN ハードウェアの Arm/Start 分離が必要 |
| **Lock-free task separation** | component_architecture.md のパターンは正しい |
| **MessagePack** (分散時) | 高速・コンパクト |
| **oxyroot** (ROOT 出力) | 物理屋の標準ツールチェーン。代替不可 |
| **HWM=0 policy** | データを落とさない原則は絶対 |
| **crossbeam チャンネル** | DecodeLoop の並列化パターンは正しい |
| **.delila raw format** | クラッシュ耐性 + オフライン再処理 |
| **MongoDB** | Docker で動かしていて困っていない。SQLite にする理由がない |
| **Angular frontend** | 動いている。書き直しは大工事で見合わない |

---

## 現状との差分まとめ

| 観点 | 現状 | 再設計 |
|------|------|--------|
| バイナリ | 30個 | **1個** (`delila-rs`) |
| プロセス | コンポーネントごとに別プロセス | config に応じて **1プロセス内にスレッド** |
| コマンド | ZMQ REQ/REP + JSON | **HTTP REST**（全ノード共通 API） |
| データ (重要パス) | ZMQ PUB/SUB + HWM=0 | in-process channel **or** ZMQ PUSH/PULL |
| データ (Monitor) | ZMQ PUB/SUB + HWM=0 | ZMQ PUB/SUB + HWM=最新N個 |
| デプロイ | rsync + 各バイナリ起動スクリプト | rsync 1バイナリ + config |
| Crate 構成 | 単一 crate | **Cargo Workspace** (10 crates) |

---

## 優先度（現実的に手を付けるなら）

| 優先度 | 変更 | 効果 | コスト |
|--------|------|------|--------|
| **1** | Workspace 分割 | コンパイル速度、保守性 | 中（リファクタリング） |
| **2** | 1バイナリ + config 駆動 | 運用シンプル化、性能向上 | 大（次プロジェクト向き） |
| **3** | PUB/SUB → PUSH/PULL | HWM=0 workaround 不要 | 小（ソケット変更のみ） |
| **4** | Config 統一 (TOML) | UX 向上 | 中 |

---

## 正直な結論

現在の設計は「C++ DAQ を Rust で書き直す」という出発点から始まっているので、C++ 時代の DELILA2 の設計思想（別プロセス + ZMQ メッセージング）がそのまま残っている。Rust の強みを活かすなら、「同一マシンなら1プロセスで、型システムで安全性を保証する」方向にシフトすべき。

ただし — **今の設計は動いている**。6台のデジタイザで 3.8M events/s を落とさずに処理できている。再設計は「次のプロジェクト」でやるべきもので、今のコードベースを急いで書き直す必要はない。

核心部分（decoders, chunk_builder, state machine, component architecture パターン）はそのまま再利用できる。**crate 分割をまずやれば、その crate 群を新しい Node 構造に組み込む形で段階的に進められる。**

困っていないものは直さない。物理屋のための道具であって、ソフトウェア工学の教科書ではない。
