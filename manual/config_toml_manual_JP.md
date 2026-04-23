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
| `web_ui_dir` | string | 自動検出 | Angular ビルド済みディレクトリ。省略時 `web/operator-ui/dist/operator-ui/browser/`。**`dist/` はリポジトリにコミット済みなので Node.js なしで動く**（UI を書き換えた開発者は `cd web/operator-ui && npm run build` 後に `dist/` も commit）|
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

**権威ソース:** [legacy/GD9764_FELib_User_Guide.pdf](../legacy/GD9764_FELib_User_Guide.pdf) Rev.2 Chap 6

URL は RFC 3986 形式: `<scheme>://<authority>/<path>?<queries>`。大文字小文字区別なし。
Query は `&` で連結可。

### 14.1 スキーム早見表

| type | プレフィックス | 実装ライブラリ | 対応世代 |
|------|--------------|------------|---------|
| `psd1` / `pha1` | `dig1://` | CAEN Dig1 (FELib v1 compat) | V17xx / VX17xx / DT57xx (FW 1.0) |
| `psd2` / `amax` | `dig2://` | CAEN Dig2 (FELib v2) | V27xx / VX27xx / DT27xx (FW 2.0) |
| `x743ci` / `x743std` | **(URL 不要)** | CAENDigitizer 直呼び | V1743 — `[x743]` セクションで接続指定 |

**重要**: デジタイザごとに**同時 1 接続のみ**。`CAEN_FELib_Open()` を別プロセスから呼ぶと
既存セッションが強制切断される。同一プロセスなら `DeviceAlreadyOpen` が返る。

---

### 14.2 `dig2://` (Digitizer 2.0)

Authority が IP/ホスト名または予約オーソリティ `caen.internal`。

| 接続形態 | URL 例 | 備考 |
|---|---|---|
| **Ethernet (IPv4)** | `dig2://192.0.2.1` | 最も標準的。IP 直指定を推奨 |
| **Ethernet (IPv6)** | `dig2://[2001:db8::1]` | 角括弧必須 |
| **Ethernet (mDNS)** | `dig2://caendgtz-eth-<pid>` | OS 依存（Linux では `.local` が必要）。非推奨、IP 推奨 |
| **USB (short form)** | `dig2://caendgtz-usb-<pid>` | `<pid>` = デジタイザ S/N |
| **USB (`caen.internal`)** | `dig2://caen.internal/usb/<pid>` | 上記の別表記 |
| **OpenARM (組込 ARM)** | `dig2://caen.internal/openarm` | DT27xx 内蔵 ARM から使用。172.17.0.1 を指定せずこれを使う |

実例:
```
dig2://172.18.4.56           # VX2730 on LAN
dig2://caendgtz-usb-52622    # same box via USB 3.0
```

---

### 14.3 `dig1://` (Digitizer 1.0)

`path` が **接続タイプ**を表し、CAEN_DGTZ_ConnectionType enum にマップされる。
Query parameters で接続先を詳細指定。

| path | enum (CAEN_DGTZ_*) | 意味 | Authority |
|---|---|---|---|
| `/usb` | `USB` | USB 2.0 直結（V1720 / V1730 USB ポート等） | `caen.internal` |
| `/optical_link` | `OpticalLink` | A2818 / A3818 PCIe カード + 光リンク (CONET) | `caen.internal` |
| `/usb_a4818` | `USB_A4818` | USB → A4818 → 光リンク → デジタイザ | `caen.internal` |
| `/usb_a4818_v2718` | `USB_A4818_V2718` | A4818 → V2718 VME bridge | `caen.internal` |
| `/usb_a4818_v3718` | `USB_A4818_V3718` | A4818 → V3718 VME bridge | `caen.internal` |
| `/usb_a4818_v4718` | `USB_A4818_V4718` | A4818 → V4718 VME bridge | `caen.internal` |
| `/eth_v4718` | `ETH_V4718` | Ethernet → V4718 VME bridge | **V4718 の IP** |
| `/usb_v4718` | `USB_V4718` | USB → V4718 VME bridge | `caen.internal` |

#### Query parameters

| キー | 用途 | 対応 path |
|---|---|---|
| `link_num=<N>` | A3818/A4818/USB のリンク番号。A4818/USB V4718 では PID | `/optical_link`, `/usb`, `/usb_a4818*`, `/usb_v4718` |
| `conet_node=<N>` | CONET デイジーチェーンのノード番号 (0–7) | `/optical_link`, A4818 bridge 系 |
| `vme_base_address=<addr>` | VME ベースアドレス (0x 形式 OK) | VME bridge 経由時 (V17xx VME モジュール) |

#### URL 例

```
# USB ダイレクト（DT5730B, V1720 USB ポート等）
dig1://caen.internal/usb?link_num=0

# A3818 PCIe 光リンク、リンク 0、デイジーチェーンの 0 ノード目
dig1://caen.internal/optical_link?link_num=0&conet_node=0

# A3818 リンク 2 → V3718 VME bridge → VME アドレス 0x32100000 の V1730
dig1://caen.internal/optical_link?link_num=2&vme_base_address=0x32100000

# A4818 USB bridge 経由で光リンク先のデジタイザ
dig1://caen.internal/usb_a4818?link_num=<A4818_PID>&conet_node=0

# V4718 Ethernet bridge (IP 10.1.2.3) 経由で VME V1730
dig1://10.1.2.3/eth_v4718?vme_base_address=0x00100000

# V4718 USB bridge 経由
dig1://caen.internal/usb_v4718?link_num=<V4718_PID>&vme_base_address=0x00100000
```

---

### 14.4 接続方式早見チャート

| ハードウェア | 推奨 URL |
|---|---|
| **DT5730B (USB)** | `dig1://caen.internal/usb?link_num=0` |
| **V1730 (光リンク + A3818 PCIe)** | `dig1://caen.internal/optical_link?link_num=<port>&conet_node=<node>` |
| **VX1730B (光リンク + A3818)** | 同上 |
| **V1730 VME モジュール (A3818 光 → V3718 VME)** | `dig1://caen.internal/optical_link?link_num=<port>&vme_base_address=<addr>` |
| **VX2730 (Ethernet)** | `dig2://<IP>` |
| **VX2730 (USB 3.0)** | `dig2://caendgtz-usb-<SN>` |
| **DT2730 OpenARM 組込** | `dig2://caen.internal/openarm` |
| **V1743** (DPP-CI/Standard) | `digitizer_url` 不使用 — `[x743]` セクションで `link_num` / `conet_node` / `connection_type` 指定 |

### 14.5 注意事項

- **1 接続/デジタイザ**: 既に接続済みのデジタイザに別プロセスから `CAEN_FELib_Open()` すると既存側が切断される。これが不意に発生すると DAQ が Error 状態になる
- **`caen.internal` は予約オーソリティ**: 実際の IP やホスト名ではない。USB / 光 / VME bridge 系の「非ネットワーク」接続時のプレースホルダー
- **A3818 ドライバ**: Linux ではカーネルモジュール (`a3818.ko`) と `/etc/udev/rules.d/` ルールが必要。DELILA では `v1.6.12-delila1` パッチ版を使用（→ `docs/a3818_driver_analysis.md`）
- **Ethernet デジタイザの検索**: mDNS (`caendgtz-eth-<pid>`) は OS 依存で動かないことがある。**IP 直指定が最も確実**
- **大文字小文字**: URL は区別なし（`DIG2://` も `dig2://` も同じ）

---

## 参考

- 実例: [config/](../config/) — 用途別サンプル多数
- デジタイザ JSON 仕様: [docs/digitizer_system_spec.md](../docs/digitizer_system_spec.md)
- アーキテクチャ全体: [docs/architecture/config_and_deployment.md](../docs/architecture/config_and_deployment.md)
- ソース実装: [src/config/mod.rs](../src/config/mod.rs)
- 起動スクリプト: [scripts/start_daq.sh](../scripts/start_daq.sh)
