# DELILA DAQ 運用マニュアル

## システム構成

```
┌─────────────────────────────────────┐     ┌──────────────────────────┐
│  Local Mac (このコンピュータ)        │     │  Remote Linux             │
│                                     │     │  172.18.4.147             │
│  Node Agent (port 8090)             │     │                          │
│   ├── merger                        │     │  Node Agent (port 8090)  │
│   ├── recorder                      │     │   └── reader-psd1        │
│   ├── monitor                       │     │       (DT5730B USB)      │
│   ├── reader-psd2 (DT5730S network) │     │                          │
│   └── operator (port 8080)          │     └──────────────────────────┘
│                                     │
│  Web UI: http://localhost:8080      │
│  Monitor: http://localhost:8081     │
└─────────────────────────────────────┘
```

---

## 1. DAQ の起動

### Step 1: リモートマシンの Agent を起動

ターミナルで以下を実行：

```bash
ssh 172.18.4.147
cd ~/WorkSpace/delila-rs
./target/release/node_agent -f config/agent_remote.toml --start-all
```

Agent が常駐し、PSD1 Reader が自動起動する。
正常起動すると以下のようなログが表示される：

```
INFO node_agent: Loaded 1 process config(s) from config/agent_remote.toml
INFO node_agent:   reader-psd1 -> ./target/release/reader [...] (auto_restart=true)
INFO node_agent: Starting all processes...
INFO delila_rs::node_agent::process: Process started name="reader-psd1" pid=12345
INFO node_agent: Node Agent listening on http://0.0.0.0:8090
```

### Step 2: ローカルの Agent を起動

別のターミナルで以下を実行：

```bash
cd ~/WorkSpace/delila-rs
./target/release/node_agent -f config/agent_local.toml --start-all
```

Merger, Recorder, Monitor, PSD2 Reader, Operator が全て自動起動する。

### Step 3: 動作確認

ブラウザで http://localhost:8080 を開く。
全コンポーネントが **Idle** / **Online** と表示されれば成功。

コマンドラインで確認する場合：

```bash
# ローカルの全プロセス状態
curl -s http://localhost:8090/api/status | python3 -m json.tool

# リモートの全プロセス状態
curl -s http://172.18.4.147:8090/api/status | python3 -m json.tool

# DAQ システム状態 (Operator 経由)
curl -s http://localhost:8080/api/status | python3 -m json.tool
```

---

## 2. データ収集の開始と停止

Web UI (http://localhost:8080) から操作するか、コマンドラインで：

```bash
# Detect → Configure → Arm → Start
curl -X POST http://localhost:8080/api/run/start

# Stop
curl -X POST http://localhost:8080/api/stop

# Reset (Idle に戻す)
curl -X POST http://localhost:8080/api/reset
```

---

## 3. DAQ の停止

### 方法 A: Agent の Ctrl+C

各ターミナルで **Ctrl+C** を押す。
Agent が全子プロセスを停止してから終了する。

### 方法 B: API 経由

```bash
# ローカルの全プロセス停止
curl -X POST http://localhost:8090/api/stop-all

# リモートの全プロセス停止
curl -X POST http://172.18.4.147:8090/api/stop-all
```

---

## 4. 個別プロセスの操作

```bash
# プロセス一覧と状態確認
curl -s http://localhost:8090/api/status

# 特定プロセスの停止
curl -X POST http://localhost:8090/api/processes/reader-psd2/stop

# 特定プロセスの起動
curl -X POST http://localhost:8090/api/processes/reader-psd2/start

# 特定プロセスの再起動
curl -X POST http://localhost:8090/api/processes/reader-psd2/restart

# プロセスのログ確認 (最新50行)
curl -s http://localhost:8090/api/processes/reader-psd2/logs?tail=50

# リモートの PSD1 Reader のログ
curl -s http://172.18.4.147:8090/api/processes/reader-psd1/logs?tail=50
```

---

## 5. トラブルシューティング

### デジタイザを物理的に再起動した場合

Reader プロセスが死ぬが、Agent の **auto_restart** が自動で再起動する。
数秒待って Agent の状態を確認：

```bash
curl -s http://172.18.4.147:8090/api/status
```

`restart_count` が増えて `state: "running"` になっていれば OK。
Reader 再起動後は Web UI で **Reset → Detect → Configure → Start** が必要。

### プロセスが起動しない場合

ログを確認：

```bash
# ローカルの operator のログ
curl -s http://localhost:8090/api/processes/operator/logs?tail=100

# リモートの reader のログ
curl -s http://172.18.4.147:8090/api/processes/reader-psd1/logs?tail=100
```

### Agent 自体が起動しない場合

直接ログファイルを確認：

```bash
# ローカル
cat /tmp/node_agent.log

# リモート
ssh 172.18.4.147 cat /tmp/node_agent.log
```

### ポートが使用中の場合

前回の Agent が残っている可能性がある：

```bash
# ローカル
pkill -f node_agent

# リモート
ssh 172.18.4.147 pkill -f node_agent
```

### Settings にデジタイザが表示されない場合

各デジタイザの JSON 設定ファイル内の `digitizer_id` が重複していないか確認する。
`digitizer_id` は TOML の `[[network.sources]]` の `id` と一致させる必要がある。
重複があると後からロードされた方で上書きされ、片方しか表示されない。

```
config/digitizers/psd1_test.json  → "digitizer_id": 0  (= TOML の source id 0)
config/digitizers/psd2_56.json   → "digitizer_id": 1  (= TOML の source id 1)
```

---

## 6. 設定ファイル一覧

| ファイル | 説明 |
|---------|------|
| `config/config_combined.toml` | DAQ トポロジ定義 (PSD1+PSD2) |
| `config/agent_local.toml` | ローカル Agent 設定 |
| `config/agent_remote.toml` | リモート Agent 設定 |
| `config/digitizers/psd1_test.json` | PSD1 デジタイザパラメータ |
| `config/digitizers/psd2_56.json` | PSD2 デジタイザパラメータ |

---

## 7. Agent API リファレンス

| Method | Path | 説明 |
|--------|------|------|
| GET | `/api/status` | 全プロセス状態 |
| GET | `/api/processes/:name` | 単一プロセス状態 |
| POST | `/api/processes/:name/start` | 起動 |
| POST | `/api/processes/:name/stop` | 停止 |
| POST | `/api/processes/:name/restart` | 再起動 |
| GET | `/api/processes/:name/logs?tail=N` | ログ取得 |
| POST | `/api/start-all` | 全プロセス起動 |
| POST | `/api/stop-all` | 全プロセス停止 |

---

## 8. DAQ 設定ファイル (TOML) 文法リファレンス

DAQ トポロジ設定ファイル（例: `config/config_combined.toml`）の全フィールドを説明する。

### 全体構造

```toml
[operator]
experiment_name = "DELILA_Combined"   # 実験名（省略時: "DefaultExp"）

[network]
cluster_name = "default"              # クラスタ名（省略可）

[[network.sources]]                   # データソース（複数指定可）
# ...

[network.merger]                      # Merger 設定
# ...

[network.recorder]                    # Recorder 設定
# ...

[network.monitor]                     # Monitor 設定
# ...
```

### `[operator]` セクション

| フィールド | 型 | 必須 | デフォルト | 説明 |
|-----------|------|------|-----------|------|
| `experiment_name` | string | No | `"DefaultExp"` | 実験名。ファイル名やログに使用される |

### `[[network.sources]]` セクション（データソース）

`[[...]]` は TOML の配列テーブル。複数のデータソースを定義できる。

| フィールド | 型 | 必須 | デフォルト | 説明 |
|-----------|------|------|-----------|------|
| `id` | integer | **Yes** | — | ユニーク ID。デジタイザ JSON の `digitizer_id` と一致させること |
| `name` | string | No | `""` | 人間可読な名前（例: `"psd1-dt5730"`） |
| `type` | string | No | `"emulator"` | ソースタイプ。下表参照 |
| `bind` | string | **Yes** | — | ZMQ データ送信アドレス（例: `"tcp://*:5555"`） |
| `command` | string | No | — | ZMQ コマンド受信アドレス（例: `"tcp://*:5560"`） |
| `config_file` | string | No | — | デジタイザ設定 JSON のパス（例: `"config/digitizers/psd1_test.json"`） |
| `digitizer_url` | string | No | — | デジタイザ接続 URL。PSD2 は必須（例: `"dig2://172.18.4.56"`） |
| `module_id` | integer | No | — | イベントタグ用モジュール ID |
| `time_step_ns` | float | No | — | ADC 時間ステップ (ns)。500 MHz なら `2.0` |
| `pipeline_order` | integer | No | `1` | Start/Stop の順序。小さい方が先に起動 |
| `is_master` | bool | No | `false` | マルチデジタイザ同期でのマスターフラグ |
| `host` | string | No | `"localhost"` | Reader が動いているマシンのホスト名/IP。リモートの場合は要指定 |

#### `type` に指定可能な値

| 値 | 説明 |
|---|------|
| `emulator` | テスト用ダミーデータ生成器 |
| `psd1` | CAEN DPP-PSD ファームウェア (CAEN ライブラリ経由、USB/光リンク) |
| `psd2` | CAEN DPP-PSD2 ファームウェア (dig2 ライブラリ経由、ネットワーク) |
| `pha1` | CAEN DPP-PHA ファームウェア (CAEN ライブラリ経由) |
| `zle` | CAEN DPP-ZLE ファームウェア（将来） |
| `amax` | DELILA AMax ファームウェア（カスタム DPP_OPEN） |

#### `host` フィールドの重要性

Reader がリモートマシンで動く場合、Merger がデータを正しく subscribe するために `host` を指定する必要がある。
Merger の `subscribe` リストを空にすると、各ソースの `bind` アドレスの `*` を `host` で置換して自動解決する。

```toml
# リモート Reader の例
[[network.sources]]
id = 0
type = "psd1"
bind = "tcp://*:5555"           # Reader 側は * でバインド
host = "172.18.4.147"           # Merger は tcp://172.18.4.147:5555 に接続
```

### `[network.merger]` セクション

| フィールド | 型 | 必須 | デフォルト | 説明 |
|-----------|------|------|-----------|------|
| `subscribe` | string[] | No | 自動解決 | 上流ソースの ZMQ アドレスリスト。空の場合、ソースの `bind`+`host` から自動生成 |
| `publish` | string | **Yes** | — | 下流への ZMQ パブリッシュアドレス（例: `"tcp://*:5557"`） |
| `command` | string | No | — | ZMQ コマンド受信アドレス（例: `"tcp://*:5570"`） |
| `pipeline_order` | integer | No | `2` | Start/Stop の順序 |

#### `subscribe` の自動解決

`subscribe` を省略または空配列 `[]` にすると、全ソースの `bind` アドレスの `*` を各ソースの `host` で置換して自動解決する。
リモートソースがある場合はこの自動解決を使うのが推奨。

```toml
# 方法 1: 自動解決（推奨 — リモートソースに対応）
[network.merger]
publish = "tcp://*:5557"
command = "tcp://*:5570"

# 方法 2: 手動指定（全ソースがローカルの場合のみ）
[network.merger]
subscribe = ["tcp://localhost:5555", "tcp://localhost:5556"]
publish = "tcp://*:5557"
command = "tcp://*:5570"
```

### `[network.recorder]` セクション

| フィールド | 型 | 必須 | デフォルト | 説明 |
|-----------|------|------|-----------|------|
| `subscribe` | string | **Yes** | — | Merger からの ZMQ サブスクライブアドレス |
| `command` | string | No | — | ZMQ コマンド受信アドレス |
| `output_dir` | string | No | `"./data"` | データファイル出力ディレクトリ |
| `max_file_size_mb` | integer | No | `1024` | 最大ファイルサイズ (MB)。超過で自動ローテーション |
| `max_file_duration_sec` | integer | No | `600` | 最大ファイル時間 (秒)。超過で自動ローテーション |
| `pipeline_order` | integer | No | `3` | Start/Stop の順序 |

### `[network.monitor]` セクション

| フィールド | 型 | 必須 | デフォルト | 説明 |
|-----------|------|------|-----------|------|
| `subscribe` | string | **Yes** | — | Merger からの ZMQ サブスクライブアドレス |
| `command` | string | No | — | ZMQ コマンド受信アドレス |
| `http_port` | integer | No | `8081` | Monitor Web UI のポート番号 |
| `pipeline_order` | integer | No | `3` | Start/Stop の順序 |

### 完全な設定例

```toml
# PSD1 (リモート USB) + PSD2 (ローカル ネットワーク) の構成例

[operator]
experiment_name = "DELILA_Combined"

[network]

# Source 0: PSD1 (DT5730B, リモート Linux で USB 接続)
[[network.sources]]
id = 0
name = "psd1-dt5730"
type = "psd1"
host = "172.18.4.147"
bind = "tcp://*:5555"
command = "tcp://*:5560"
digitizer_url = "dig1://caen.internal/usb?link_num=0"
config_file = "config/digitizers/psd1_test.json"
pipeline_order = 1

# Source 1: PSD2 (DT5730S, ローカルからネットワーク接続)
[[network.sources]]
id = 1
name = "psd2-dt5730s"
type = "psd2"
bind = "tcp://*:5556"
command = "tcp://*:5561"
digitizer_url = "dig2://172.18.4.56"
config_file = "config/digitizers/psd2_56.json"
pipeline_order = 1

# Merger (subscribe 自動解決)
[network.merger]
publish = "tcp://*:5557"
command = "tcp://*:5570"
pipeline_order = 2

# Recorder
[network.recorder]
subscribe = "tcp://localhost:5557"
command = "tcp://*:5580"
output_dir = "./data"
pipeline_order = 3

# Monitor
[network.monitor]
subscribe = "tcp://localhost:5557"
command = "tcp://*:5590"
http_port = 8081
pipeline_order = 3
```

---

## 9. Node Agent 設定ファイル (TOML) 文法リファレンス

Node Agent 設定ファイル（例: `config/agent_local.toml`）の全フィールドを説明する。

### 全体構造

```toml
[agent]
name = "local-mac"
port = 8090

[[process]]
name = "merger"
command = "./target/release/merger"
args = ["--config", "config/config_combined.toml"]
auto_restart = true
```

### `[agent]` セクション

| フィールド | 型 | 必須 | デフォルト | 説明 |
|-----------|------|------|-----------|------|
| `name` | string | No | `"node"` | エージェント名（ログ・識別用） |
| `port` | integer | No | `8090` | Agent REST API のポート番号 |
| `log_buffer_lines` | integer | No | `1000` | 各プロセスのログリングバッファサイズ（行数） |

### `[[process]]` セクション（管理対象プロセス）

`[[...]]` は TOML の配列テーブル。管理する各プロセスを定義する。

| フィールド | 型 | 必須 | デフォルト | 説明 |
|-----------|------|------|-----------|------|
| `name` | string | **Yes** | — | プロセス名（API パスで使用: `/api/processes/:name`） |
| `command` | string | **Yes** | — | 実行コマンドのパス |
| `args` | string[] | No | `[]` | コマンド引数 |
| `working_dir` | string | No | Agent のカレントディレクトリ | 作業ディレクトリ |
| `auto_restart` | bool | No | `false` | 異常終了時の自動再起動 |
| `restart_delay_secs` | integer | No | `3` | 再起動までの待機秒数 |
| `env` | table | No | `{}` | 環境変数（キー = 値のテーブル） |

### 設定例

```toml
# ローカルマシン: 全コンポーネント管理
[agent]
name = "local-mac"
port = 8090
log_buffer_lines = 1000

[[process]]
name = "merger"
command = "./target/release/merger"
args = ["--config", "config/config_combined.toml"]
auto_restart = true
restart_delay_secs = 3

[[process]]
name = "recorder"
command = "./target/release/recorder"
args = ["--config", "config/config_combined.toml"]
auto_restart = true

[[process]]
name = "reader-psd2"
command = "./target/release/reader"
args = ["--config", "config/config_combined.toml", "--source-id", "1"]
auto_restart = true
restart_delay_secs = 5

# 環境変数を設定する例
# [process.env]
# RUST_LOG = "debug"
```

```toml
# リモートマシン: PSD1 Reader のみ
[agent]
name = "reader-linux"
port = 8090

[[process]]
name = "reader-psd1"
command = "./target/release/reader"
args = ["--config", "config/config_combined.toml", "--source-id", "0"]
auto_restart = true
restart_delay_secs = 5
```

---

## クイックリファレンス

```bash
# === 起動 ===
# 1. リモート (別ターミナルまたはSSH)
ssh 172.18.4.147 "cd ~/WorkSpace/delila-rs && ./target/release/node_agent -f config/agent_remote.toml --start-all"

# 2. ローカル
cd ~/WorkSpace/delila-rs
./target/release/node_agent -f config/agent_local.toml --start-all

# === 状態確認 ===
curl -s http://localhost:8080/api/status | python3 -m json.tool

# === 停止 ===
# 各ターミナルで Ctrl+C、または:
curl -X POST http://localhost:8090/api/stop-all
curl -X POST http://172.18.4.147:8090/api/stop-all
```
