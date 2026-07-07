# DELILA DAQ 運用マニュアル

DAQ の起動・データ収集・停止・トラブルシューティングを説明する。
設定ファイル (TOML) の文法は
[設定ファイルマニュアル](../manual/config_toml_manual_JP.md) を参照。

English version: [operations_manual_en.md](operations_manual_en.md)

---

## システム構成

DAQ は 1 本の ZMQ パイプラインで、`scripts/start_daq.sh` が 1 つの config
から全コンポーネントを起動する。

```
  Reader(s) ──ZMQ──> Merger ──ZMQ──┬──> Recorder  (.delila / ROOT)
  (decode)          (sort/EB)      └──> Monitor    (histograms / waveforms)

  Operator (REST + Web UI, port 9090) が全体を制御
```

| サービス | URL / ポート |
|---------|-------------|
| Operator REST / Swagger UI | http://localhost:9090/swagger-ui/ |
| Monitor Web UI | http://localhost:8081/ |
| Mongo Express (run 履歴) | http://localhost:8082/ |

> **制御は必ず Operator REST API（Web UI）経由で行う。** 直接 ZMQ コマンドは
> 使わない。`scripts/daq_ctl.sh`（`controller` バイナリ）は低レベルの開発用途。

Reader をリモートマシンで動かす分散構成は「6. 分散構成」を参照。

---

## 1. ビルド

```bash
# 本番サービスバイナリ一式（reader / merger / recorder / monitor / operator …）
cargo build --release --bins

# ROOT 出力・Event Builder は root feature が必要
cargo build --release --features root --bin event_builder
```

---

## 2. DAQ の起動

config を 1 つ指定して起動する。

```bash
./scripts/start_daq.sh config/config_psd1_test.toml       # --no-mongo で MongoDB をスキップ
```

`start_daq.sh` は次を行う：

1. 前回の残プロセスを `pkill`
2. （必要なら）MongoDB / Docker 起動
3. config の `[[network.sources]]` に対応する reader を起動
4. merger → recorder → monitor →（有効なら）online_event_builder → operator を起動
5. Operator が `/api/status` に応答するまで待機

正常起動すると末尾に Web UI の URL が表示される。

---

## 3. 動作確認

ブラウザで http://localhost:9090 を開く。全コンポーネントが **Idle** /
**Online** と表示されれば成功。

コマンドラインで確認する場合：

```bash
curl -s http://localhost:9090/api/status | python3 -m json.tool
```

---

## 4. データ収集の開始と停止

Web UI から操作するのが基本。コマンドラインの場合は Operator REST を使う。

状態機械：
`Idle → Configure → Configured → Arm → Armed → Start → Running → Stop → Configured`

```bash
# 個別遷移
curl -X POST http://localhost:9090/api/configure       # Idle → Configured
curl -X POST http://localhost:9090/api/arm             # Configured → Armed
curl -X POST http://localhost:9090/api/start           # Armed → Running
curl -X POST http://localhost:9090/api/stop            # Running → Configured
curl -X POST http://localhost:9090/api/reset           # Any → Idle

# 一括（Detect → Configure → Arm → Start をまとめて）
curl -X POST http://localhost:9090/api/run/start
```

Run 番号の付与・履歴は `/api/runs*`（Web UI の Runs ページ）で管理する。

---

## 5. DAQ の停止

```bash
./scripts/stop_daq.sh
```

全コンポーネントを停止する。個別のコンポーネントログは `logs/latest/*.log`
（`./logs/latest/` は最新ランへのシンボリックリンク）。

---

## 6. 分散構成（リモート Reader）

Reader を別マシン（例: デジタイザが USB 接続された Linux）で動かす場合、
`[[network.sources]]` の `host` にそのマシンの IP を指定する。Merger は
`subscribe` を空にすると `bind`+`host` から接続先を自動解決する（詳細は
[設定ファイルマニュアル](../manual/config_toml_manual_JP.md)）。

リモート Reader は SSH 経由で起動できる：

```bash
./scripts/start_remote_reader.sh config/config_psd1_test.toml
```

その後ローカルで `start_daq.sh` を実行すると、リモート reader を除く
merger / recorder / monitor / operator が起動する。

---

## 7. トラブルシューティング

### デジタイザを物理的に再起動した場合

Reader が接続エラーになる。DIG1（USB/光リンク）は **Close+Open でタイムスタンプ
がリセットされる**ため、通信エラーで安易に再接続してはならない。再起動後は
DAQ を停止 → `start_daq.sh` で繋ぎ直し、Web UI で **Reset → Configure → Start**。

### プロセスが起動しない・応答しない場合

ログを確認：

```bash
tail -100 logs/latest/operator.log
tail -100 logs/latest/reader_0.log      # source id ごとに reader_<id>.log
tail -f  logs/latest/*.log
```

### ポートが使用中の場合

前回のプロセスが残っている可能性がある。`start_daq.sh` は起動時に自動 `pkill`
するが、手動で止める場合：

```bash
./scripts/stop_daq.sh
# それでも残る場合
pkill -f target/release/operator
```

### Settings にデジタイザが表示されない場合

各デジタイザ JSON の `digitizer_id` が重複していないか確認する。`digitizer_id`
は TOML の `[[network.sources]]` の `id` と一致させる必要がある。重複があると
後からロードされた方で上書きされ、片方しか表示されない。

```
config/digitizers/psd1_test.json → "digitizer_id": 0   (= TOML の source id 0)
config/digitizers/psd2_56.json   → "digitizer_id": 1   (= TOML の source id 1)
```

### events が 0 のまま増えない場合

ソフトの前に物理を疑う。NIM クレート OFF（Configure 成功だが events=0）、
デジタイザ FW ハング（CAEN -6 が連続、電源サイクル要）、VME OFF（Configure
不可）などが典型。PHA FW は historically wedge しやすく、Start 後に ADC
スペクトラムを確認し、異常ならクレート電源リセットが SOP。

---

## 8. 設定ファイル一覧

| ファイル | 説明 |
|---------|------|
| `config/config_*.toml` | DAQ トポロジ定義（起動時に指定） |
| `config/digitizers/*.json` | デジタイザごとのパラメータ |

- TOML 文法の全リファレンス → [設定ファイルマニュアル](../manual/config_toml_manual_JP.md)
- パラメータ名の対応 → [CoMPASS ↔ DevTree mapping](compass_devtree_mapping.md)

---

## クイックリファレンス

```bash
# === ビルド ===
cargo build --release --bins

# === 起動 ===
./scripts/start_daq.sh config/config_psd1_test.toml

# === 状態確認 ===
curl -s http://localhost:9090/api/status | python3 -m json.tool

# === Run 制御（基本は Web UI http://localhost:9090）===
curl -X POST http://localhost:9090/api/run/start     # 一括開始
curl -X POST http://localhost:9090/api/stop          # 停止

# === 全停止 ===
./scripts/stop_daq.sh
```
