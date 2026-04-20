# DELILA-RS 設定ファイル (TOML) マニュアル

`scripts/start_daq.sh` が読み込む TOML 設定ファイルの全仕様。
ソース: [src/config/mod.rs](../src/config/mod.rs)

```bash
./scripts/start_daq.sh config/config_psd1_test.toml
```

引数を省略すると `config.toml` が使われる。`--no-mongo` で MongoDB 起動をスキップ可能。

---

## 目次

- [1. 全体構造](#1-全体構造)
- [2. `[operator]` — Operator (Web UI) 設定](#2-operator--operator-web-ui-設定)
- [3. `[network]` — ネットワークトポロジ](#3-network--ネットワークトポロジ)
- [4. `[[network.sources]]` — データソース (必須)](#4-networksources--データソース-必須)
- [5. `[network.merger]` — Merger](#5-networkmerger--merger)
- [6. `[network.recorder]` — Recorder](#6-networkrecorder--recorder)
- [7. `[network.monitor]` — Monitor](#7-networkmonitor--monitor)
- [8. `[network.event_builder]` — オンラインイベントビルダー (オプション)](#8-networkevent_builder--オンラインイベントビルダー-オプション)
- [9. `[operator.influxdb]` / `[operator.elog]` — 外部連携](#9-operatorinfluxdb--operatorelog--外部連携)
- [10. `[settings]` — Emulator / バッチ設定](#10-settings--emulator--バッチ設定)
- [11. ポート割当ルール](#11-ポート割当ルール)
- [12. 完全例: ローカル PSD1 1台](#12-完全例-ローカル-psd1-1台)
- [13. 完全例: 分散 (リモート Reader)](#13-完全例-分散-リモート-reader)
- [14. デジタイザ URL リファレンス](#14-デジタイザ-url-リファレンス)

---

## 1. 全体構造

```toml
[operator]       # Web UI / REST API
  [operator.influxdb]   # (optional) Grafana 連携
  [operator.elog]       # (optional) ELOG 連携

[network]
  [[network.sources]]   # 1つ以上。Reader/Emulator ごとに1ブロック
  [network.merger]
  [network.recorder]
  [network.monitor]
  [network.event_builder]   # (optional)

[settings]       # (optional) Emulator / バッチパラメータ
  [settings.file]
```

起動スクリプトは `[[network.sources]]` を列挙して、各ソースに対し emulator か reader を立ち上げる。`host != "localhost"` のソースは自動で起動スキップ (= 分散モード)。

---

## 2. `[operator]` — Operator (Web UI) 設定

| キー | 型 | 既定値 | 説明 |
|------|----|--------|------|
| `experiment_name` | string | `"DefaultExp"` | 実験名。サーバー側で固定 (UI から変更不可)。ROOT ファイル名などに使用 |
| `port` | u16 | `9090` | REST API / Web UI の HTTP ポート。Swagger は `http://localhost:PORT/swagger-ui/` |
| `web_ui_dir` | string | 自動検出 | Angular ビルド済みディレクトリ。省略時 `web/operator-ui/dist/operator-ui/browser/` |
| `configure_timeout_ms` | u64 | `5000` | Configure フェーズのタイムアウト (ms) |
| `arm_timeout_ms` | u64 | `5000` | Arm フェーズのタイムアウト (ms) |
| `start_timeout_ms` | u64 | `5000` | Start フェーズのタイムアウト (ms) |
| `reset_timeout_ms` | u64 | `5000` | Reset フェーズのタイムアウト (ms) |

```toml
[operator]
experiment_name = "PSD1_Test"
port = 9090
configure_timeout_ms = 10000   # 遅い光リンクなら 10s に延長
```

---

## 3. `[network]` — ネットワークトポロジ

| キー | 型 | 既定値 | 説明 |
|------|----|--------|------|
| `cluster_name` | string | `"default"` | クラスタ識別名 (表示用) |
| `port_base_data` | u16 | `7000` | データ (PUB) ソケットのベースポート。`bind` 省略時 `port_base_data + id` に自動割当 |
| `port_base_command` | u16 | `7100` | コマンド (REP) ソケットのベースポート。`command` 省略時 `port_base_command + id` に自動割当 |

子テーブル:
- `[[network.sources]]` — 配列 (1つ以上)
- `[network.merger]` — 1つ
- `[network.recorder]` — 1つ
- `[network.monitor]` — 1つ
- `[network.event_builder]` — オプション (あれば online_event_builder バイナリ起動)

---

## 4. `[[network.sources]]` — データソース (必須)

Reader または Emulator を 1 プロセス起動するごとに 1 ブロック書く。

| キー | 型 | 必須 | 既定値 | 説明 |
|------|----|------|--------|------|
| `id` | u32 | ✅ | — | ソース ID (一意)。自動割当ポートや module_id の基準 |
| `name` | string | | `""` | 表示名 (UI 上の識別用) |
| `type` | enum | | `"emulator"` | `emulator` / `psd1` / `psd2` / `pha1` / `amax` / `x743ci` / `x743std` / `zle` |
| `bind` | string | | `tcp://*:{port_base_data + id}` | データ PUB のバインド。**省略推奨** (11節の自動割当を使う) |
| `command` | string | | `tcp://*:{port_base_command + id}` | コマンド REP のバインド。**省略推奨** (11節の自動割当を使う) |
| `digitizer_url` | string | type≠emulator | — | デジタイザ URL。14節参照 |
| `config_file` | string | type≠emulator | — | デジタイザ設定 JSON パス (相対 or 絶対) |
| `module_id` | u8 | | `id` と同値 | イベントタグ用モジュール ID |
| `time_step_ns` | f64 | | 自動 | ADC サンプル周期 (ns)。PHA1 (250MS/s) は `4.0`、PSD1/PSD2 (500MS/s) は `2.0` |
| `pipeline_order` | u32 | | `1` | Start/Stop の順序。小さいほど上流 |
| `host` | string | | `"localhost"` | Reader が動作するホスト。`localhost` 以外だと start_daq.sh は起動スキップ → リモートで手動起動 |
| `adc_min` | u16 | | `0` | エネルギー下限フィルタ (inclusive)。`< adc_min` のイベントは破棄 |

> **推奨** — `bind` / `command` は書かない。`id` から自動で `tcp://*:{7000+id}` / `tcp://*:{7100+id}` が割り当てられ、ポート衝突を避けやすい。ベースポートを変えたい場合は `[network] port_base_data` / `port_base_command` で一括変更する (11節参照)。

### `type` の意味

| 値 | 対応ハードウェア / ライブラリ |
|----|-------------------------------|
| `emulator` | ダミーデータ生成 (テスト用) |
| `psd1` | CAEN DPP-PSD (旧) 。DT5730/VX1730 等、CAENDigitizer ライブラリ経由 |
| `psd2` | CAEN DPP-PSD (新) 。VX2730 等、dig2 ライブラリ経由 |
| `pha1` | CAEN DPP-PHA。V1725S 等、CAENDigitizer ライブラリ経由 |
| `amax` | DELILA カスタム AMax FW (台形フィルタ MCA)、DPP_OPEN |
| `x743ci` | V1743 Charge Integration モード |
| `x743std` | V1743 Standard waveform モード |
| `zle` | DPP-ZLE (未実装) |

### 最小例 (emulator)

```toml
[[network.sources]]
id = 0
name = "emu-0"
type = "emulator"
```

### 最小例 (PSD1 USB)

```toml
[[network.sources]]
id = 0
name = "psd1-dt5730"
type = "psd1"
digitizer_url = "dig1://caen.internal/usb?link_num=0"
config_file = "config/digitizers/psd1_test.json"
```

---

## 5. `[network.merger]` — Merger

複数ソースのデータを 1 本の PUB に統合するコンポーネント。

| キー | 型 | 必須 | 既定値 | 説明 |
|------|----|------|--------|------|
| `subscribe` | [string] | | 自動生成 | 上流 PUB の connect アドレス配列。空なら全ソースの `host:port_base_data+id` から自動生成 |
| `publish` | string | ✅ | — | 下流への PUB バインド (例: `tcp://*:5557`) |
| `command` | string | | — | コマンド REP バインド |
| `pipeline_order` | u32 | | `2` | Start/Stop 順序 |

推奨: `subscribe` は**省略**して自動解決に任せる。`host` フィールドで分散構成を記述すれば、Merger が正しいホストを自動的に参照する。

```toml
[network.merger]
publish = "tcp://*:5557"
command = "tcp://*:5570"
```

---

## 6. `[network.recorder]` — Recorder

`.delila` 生データバイナリをディスクに書き出す。

| キー | 型 | 必須 | 既定値 | 説明 |
|------|----|------|--------|------|
| `subscribe` | string | ✅ | — | Merger PUB の connect アドレス |
| `command` | string | | — | コマンド REP バインド |
| `output_dir` | string | | `"./data"` | 出力ディレクトリ (自動作成) |
| `max_file_size_mb` | u64 | | `1024` | ファイルローテーション: サイズ上限 (MB) |
| `max_file_duration_sec` | u64 | | `600` | ファイルローテーション: 時間上限 (秒) |
| `pipeline_order` | u32 | | `3` | Start/Stop 順序 |

```toml
[network.recorder]
subscribe = "tcp://localhost:5557"
command = "tcp://*:5580"
output_dir = "./data"
max_file_size_mb = 2048
max_file_duration_sec = 1800
```

---

## 7. `[network.monitor]` — Monitor

ライブヒストグラム / 波形を HTTP で提供する。

| キー | 型 | 必須 | 既定値 | 説明 |
|------|----|------|--------|------|
| `subscribe` | string | ✅ | — | Merger PUB の connect アドレス |
| `command` | string | | — | コマンド REP バインド |
| `http_port` | u16 | | `8081` | HTTP / Web UI ポート |
| `pipeline_order` | u32 | | `3` | Start/Stop 順序 |
| `psd_bins` | u32 | | `200` | PSD 1D ヒストのビン数 |
| `psd_min` | f32 | | `-0.2` | PSD 1D 最小値 |
| `psd_max` | f32 | | `1.2` | PSD 1D 最大値 |
| `psd2d_x_bins` | u32 | | `512` | PSD 2D: X (Energy) ビン数 |
| `psd2d_y_bins` | u32 | | `200` | PSD 2D: Y (PSD) ビン数 |

```toml
[network.monitor]
subscribe = "tcp://localhost:5557"
command = "tcp://*:5590"
http_port = 8081
```

---

## 8. `[network.event_builder]` — オンラインイベントビルダー (オプション)

このセクションがあり、かつ `target/release/online_event_builder` が存在する場合のみ起動される (ROOT 出力は `--features root` ビルドが必要)。

| キー | 型 | 必須 | 既定値 | 説明 |
|------|----|------|--------|------|
| `subscribe` | string | ✅ | — | Merger PUB の connect アドレス |
| `command` | string | | — | コマンド REP バインド |
| `output_dir` | string | | `"./data/events"` | ROOT イベントファイル出力先 |
| `coincidence_window_ns` | f64 | | `500.0` | コインシデンス窓幅 (ns) |
| `slice_duration_ns` | f64 | | `10_000_000.0` | タイムスライス長 (ns) = 10 ms |
| `buffer_delay_ns` | f64 | | `5_000_000.0` | TimeSortBuffer の遅延 (ns) = 5 ms |
| `ch_settings_file` | string | | — | チャンネル設定 JSON (detector type, threshold 等) |
| `time_calib_file` | string | | — | タイムキャリブレーション JSON |
| `pipeline_order` | u32 | | `3` | Start/Stop 順序 |

```toml
[network.event_builder]
subscribe = "tcp://localhost:5557"
command = "tcp://*:5595"
output_dir = "./data/events"
coincidence_window_ns = 500.0
ch_settings_file = "config/chSettings.json"
```

---

## 9. `[operator.influxdb]` / `[operator.elog]` — 外部連携

### InfluxDB (Grafana メトリクス)

| キー | 型 | 必須 | 既定値 | 説明 |
|------|----|------|--------|------|
| `url` | string | ✅ | — | InfluxDB v3 エンドポイント (例: `http://localhost:8181`) |
| `database` | string | | `"delila"` | DB 名 |
| `interval_secs` | u64 | | `2` | ポーリング間隔 (秒) |

```toml
[operator.influxdb]
url = "http://localhost:8181"
database = "delila"
interval_secs = 2
```

### ELOG (電子ログブック)

| キー | 型 | 必須 | 既定値 | 説明 |
|------|----|------|--------|------|
| `url` | string | ✅ | — | ELOG サーバー URL |
| `logbook` | string | ✅ | — | ログブック名 |
| `author` | string | | `"DELILA-DAQ"` | 自動投稿時の Author |

```toml
[operator.elog]
url = "http://localhost:8082"
logbook = "3MV_2026"
```

---

## 10. `[settings]` — Emulator / バッチ設定

主に Emulator のパラメータ。実デジタイザ運用では通常省略。

| キー | 型 | 既定値 | 説明 |
|------|----|--------|------|
| `source` | enum | `"file"` | `"file"` (推奨) / `"mongodb"` (未対応) |

`[settings.file]`:

| キー | 型 | 既定値 | 説明 |
|------|----|--------|------|
| `events_per_batch` | u32 | `100` | 1バッチあたりのイベント数 |
| `batch_interval_ms` | u64 | `100` | バッチ送出間隔 (ms) |
| `num_modules` | u32 | `2` | モジュール数 (Emulator) |
| `channels_per_module` | u32 | `16` | チャンネル数 (Emulator) |
| `enable_waveform` | bool | `false` | 波形生成 (Emulator) |
| `waveform_probes` | u8 | `3` | 波形プローブビットマスク (1=analog1, 2=analog2, 3=both, 63=all) |
| `waveform_samples` | usize | `512` | 波形サンプル数 |

```toml
[settings]
source = "file"

[settings.file]
events_per_batch = 200
batch_interval_ms = 50
enable_waveform = true
waveform_samples = 1024
```

---

## 11. ポート割当ルール

### 自動割当 (推奨)

ソースの `bind` / `command` を省略すると以下で自動割当:

- データ: `tcp://*:{port_base_data + id}` (既定 `7000 + id`)
- コマンド: `tcp://*:{port_base_command + id}` (既定 `7100 + id`)

```toml
[network]
port_base_data = 7000
port_base_command = 7100

[[network.sources]]
id = 0              # データ: 7000, コマンド: 7100
[[network.sources]]
id = 1              # データ: 7001, コマンド: 7101
```

### 既定ポート早見表

| コンポーネント | ポート | 種類 |
|--------------|--------|------|
| Operator HTTP | 9090 | REST/Web UI |
| Monitor HTTP | 8081 | REST/Web UI |
| Source data (自動) | 7000+id | ZMQ PUB |
| Source command (自動) | 7100+id | ZMQ REP |
| Merger publish | 5557 | ZMQ PUB |
| Merger command | 5570 | ZMQ REP |
| Recorder command | 5580 | ZMQ REP |
| Monitor command | 5590 | ZMQ REP |
| EventBuilder command | 5595 | ZMQ REP |
| MongoDB | 27017 | TCP |

> **注意** — ポート衝突は一切の警告なくサイレントに失敗する (ZMQ bind エラーがログに出るだけ)。同一ホストで複数 DAQ を立ち上げる場合は `[operator] port`、`[network.monitor] http_port`、`publish` ポート、`port_base_*` を必ず分ける。

---

## 12. 完全例: ローカル PSD1 1台

`config/config_psd1_test.toml` の構造:

```toml
[operator]
experiment_name = "PSD1_Test"
port = 9090

[network]

[[network.sources]]
id = 0
name = "psd1-dt5730"
type = "psd1"
# bind/command 省略 → data: 7000, command: 7100 に自動割当
digitizer_url = "dig1://caen.internal/usb?link_num=0"
config_file = "config/digitizers/psd1_test.json"
pipeline_order = 1

[network.merger]
publish = "tcp://*:5557"
command = "tcp://*:5570"
pipeline_order = 2

[network.recorder]
subscribe = "tcp://localhost:5557"
command = "tcp://*:5580"
output_dir = "./data"
pipeline_order = 3

[network.monitor]
subscribe = "tcp://localhost:5557"
command = "tcp://*:5590"
http_port = 8081
pipeline_order = 3
```

---

## 13. 完全例: 分散 (リモート Reader)

Reader が別マシンで動作する構成。`host` で明示する。

```toml
[operator]
experiment_name = "PSD1_Distributed"
port = 9090

[network]

[[network.sources]]
id = 0
name = "psd1-dt5730"
type = "psd1"
host = "172.18.4.147"        # ← Reader が動作するリモートホスト
# bind/command 省略 → data: 172.18.4.147:7000, command: 172.18.4.147:7100 に自動解決
digitizer_url = "dig1://caen.internal/usb?link_num=0"
config_file = "config/digitizers/psd1_test.json"
pipeline_order = 1

# subscribe は書かない → host から自動解決される
[network.merger]
publish = "tcp://*:5557"
command = "tcp://*:5570"
pipeline_order = 2

[network.recorder]
subscribe = "tcp://localhost:5557"
output_dir = "./data"

[network.monitor]
subscribe = "tcp://localhost:5557"
http_port = 8081
```

起動手順:

```bash
# ローカル (Operator ホスト)
./scripts/start_daq.sh config/config_psd1_distributed.toml
# → Reader は起動スキップされ、案内が表示される

# リモート (172.18.4.147)
./target/release/reader --config config/config_psd1_distributed.toml --source-id 0
```

---

## 14. デジタイザ URL リファレンス

`digitizer_url` の形式は type によって決まる。

| type | プレフィックス | ライブラリ |
|------|--------------|------------|
| `psd1` / `pha1` | `dig1://` | CAENDigitizer (FELib v1 互換) |
| `psd2` / `amax` | `dig2://` | dig2 (FELib v2) |
| `x743ci` / `x743std` | (不要) | CAENDigitizer。config_file で接続指定 |

### dig1:// (USB)

```
dig1://caen.internal/usb?link_num=<N>
```

- `link_num` — USB デバイス番号 (0 始まり)

### dig1:// (光リンク / A3818)

```
dig1://caen.internal/optical_link?link_num=<N>&conet_node=<M>
```

- `link_num` — A3818 のポート番号 (0–3)
- `conet_node` — デイジーチェーン上のノード番号 (0–7)

### dig2:// (Ethernet)

```
dig2://<IP-or-hostname>
```

例: `dig2://172.18.4.56`

---

## 参考

- 実例: [config/](../config/) — 用途別サンプル多数
- デジタイザ JSON 仕様: [docs/digitizer_system_spec.md](../docs/digitizer_system_spec.md)
- アーキテクチャ全体: [docs/architecture/config_and_deployment.md](../docs/architecture/config_and_deployment.md)
- ソース実装: [src/config/mod.rs](../src/config/mod.rs)
- 起動スクリプト: [scripts/start_daq.sh](../scripts/start_daq.sh)
