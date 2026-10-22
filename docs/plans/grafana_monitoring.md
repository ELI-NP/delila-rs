# Grafana モニタリング設計書

**Created:** 2026-02-19
**Status:** 実装準備完了
**Reviewed:** Gemini 協議済み (2026-02-19)

## 目的

実験中のイベントレート・スループットを Grafana ダッシュボードでリアルタイム可視化する。
数秒のビームオン/オフサイクルが識別できる時間分解能（2秒間隔）を確保し、
オペレーターが直感的にシステムの健全性を把握できるようにする。

## アーキテクチャ

Monitor（ZMQ SUB ベースの軽量プロセス）の肥大化を防ぐため、
InfluxDB への書き込み責務は Operator に集約する。

```
Operator (ポーリングループ)
  ├─ 1. Monitor REST API → チャンネル別 total_counts 取得
  ├─ 2. ComponentMetrics → 全体 throughput 取得
  ├─ 3. Instant::elapsed() による実測時間差分から正確な rate を計算
  │
  └─ 4. InfluxDB Writer (reqwest HTTP POST, Line Protocol)
       │
       ▼
  InfluxDB v3 Core (:8181, Docker)
       │ (Flight SQL)
       ▼
  Grafana (:3000, Docker)
```

### 設計判断の根拠

- **Operator 集中型を採用**: Operator は既に Monitor/Recorder のステータスをポーリングしている。
  同じループに InfluxDB 書き込みを追加するのが自然。Monitor に reqwest 依存を追加して肥大化させない。
- **ジッター対策**: `sleep(2s)` + ネットワーク遅延で実際の取得間隔は厳密な2秒にならない。
  固定値 `interval_secs` で割らず、`Instant::elapsed().as_secs_f64()` の実測値を使用する。
- **累積カウント + 実測レート両方保存**: 累積カウントで後からの柔軟な分析を可能にしつつ、
  実測レートで Grafana クエリを `SELECT rate FROM channel_rate` と簡素化する。

## メトリクス定義（Line Protocol）

**実装:** `src/operator/influxdb.rs`（2026-10-22 時点、TODO 74 P2/P5 反映）。
Operator が `[operator.influxdb] interval_secs` ごとに全コンポーネントへ ZMQ GetStatus を投げ、
結果を 1 回の POST（`/api/v3/write_lp`）で書く。タイムスタンプは付けない（InfluxDB の受信時刻）。
整数フィールドは unsigned（`u` 接尾辞）。接尾辞なしの数値は float。

### `system_rate` — ソース（Reader / Emulator）ごと

```
system_rate,module=0 total_events=500000u,event_rate=15000.5,bytes=2000000u
```

| Tag | 説明 |
|-----|------|
| `module` | ソースの source_id |

| Field | 型 | 説明 |
|-------|----|------|
| `total_events` | unsigned | 累積イベント数（`events_processed`） |
| `event_rate` | float | コンポーネント自身が算出したレート (Hz) |
| `bytes` | unsigned | 累積バイト数（`bytes_transferred`） |

### `channel_rate` — (module, channel) ごと

```
channel_rate,module=0,channel=3 count=45234u,rate=1523.5
```

| Tag | 説明 |
|-----|------|
| `module` | ソースの source_id |
| `channel` | チャンネル番号（Reader の `channel_counts` の添字） |

**名前のタグは付けない**（TODO 74 §3.1）。InfluxDB 上の識別は (module, channel) だけにし、
検出器名は表示時（ダッシュボード生成時）に対応表から付ける。名前をタグにすると改名のたびに系列が分かれ、
過去のデータに新しい名前が付かないため。

| Field | 型 | 説明 |
|-------|----|------|
| `count` | unsigned | 累積イベント数 |
| `rate` | float | レート (Hz) = (count − 前回の count) ÷ 前回からの経過時間。経過時間は `Instant` の実測値（設定の interval ではない） |

`rate` の規則:

- その (module, channel) の最初のサンプル → `rate` なし（`count` だけの行）
- count が前回より小さい（新しいラン等でカウンタが戻った）→ `rate=0`、次回はこの count から差分を取る
- 経過時間が 0 以下、または結果が有限でない → `rate` なし
- 行を書くのは count > 0 のチャンネルだけ。前回値の記録は全チャンネルで更新する

### `backlog` — パイプライン（Merger / Recorder / …）ごと（TODO 68）

```
backlog,component=Recorder queue_bytes=0u,queue_bytes_peak=1048576u,queue_items=0u,backlog_level=0u,events_processed=123456u
```

| Tag | 説明 |
|-----|------|
| `component` | コンポーネント名（空白はエスケープ） |

| Field | 型 | 説明 |
|-------|----|------|
| `queue_bytes` | unsigned | 現在のバックログ（バイト） |
| `queue_bytes_peak` | unsigned | ラン開始以降の `queue_bytes` の最大値 |
| `queue_items` | unsigned | キューの件数（`queue_size`） |
| `backlog_level` | unsigned | 0 = OK、1 = soft 超え、2 = hard 超え |
| `events_processed` | unsigned | 累積処理イベント数 |

### `disk` — ファイルを書くコンポーネントごと（TODO 69）

`disk_free_bytes` を報告するコンポーネント（Recorder 等）だけ。

```
disk,component=Recorder free_bytes=500000000000u,disk_level=0u,write_failed=false,unwritten_events=0u
```

| Field | 型 | 説明 |
|-------|----|------|
| `free_bytes` | unsigned | 出力先ファイルシステムの空き容量 |
| `disk_level` | unsigned | 0 = OK、1 = soft 未満、2 = hard 未満 |
| `write_failed` | boolean | 書き込み失敗でこのランの書き込みを止めた（Configure/Reset まで保持） |
| `unwritten_events` | unsigned | 書き込み失敗後に受け取り、ディスクに載っていないイベント数 |

### `run_event` — ランの開始・停止（TODO 74 P5、Grafana アノテーション用）

```
run_event,exp=N_Wall run=53u,event="start"
run_event,exp=N_Wall run=53u,event="stop"
```

| Tag | 説明 |
|-----|------|
| `exp` | Operator の `experiment_name`（`/api/status` と同じ値）。`,` `=` 空白はエスケープ |

| Field | 型 | 説明 |
|-------|----|------|
| `run` | unsigned | ラン番号 |
| `event` | string | `"start"` または `"stop"` |

判定（同じポーリングループ内）:

- 「ラン中」= オンラインで Running のソースが 1 つ以上ある。ラン番号はそのソースの `run_number`
  （ソース間で食い違えば最大値を使い warn）
- Operator 起動後の最初のポーリングは状態を覚えるだけで何も書かない（ラン中に Operator を再起動しても
  偽の start を出さない）
- ラン外 → ラン中: start。ラン中 → ラン外: 前回のラン番号で stop。
  ラン中のままラン番号が変わった: stop(旧) → start(新)
- 時刻はポーリング周期の分解能（既定 2 秒）。Running のソースが全部オフラインになった場合も「ラン外」として stop が出る

### 将来追加（MVP 外）

| Measurement | 内容 | 備考 |
|-------------|------|------|
| `busy_status` | デジタイザ Busy カウンタ | Reader から FELib 経由で取得。未実装 |
| `event_builder` | ビルド済みイベント数 | EB オンライン化後 |
| `hv_monitor` | HV 電圧/電流/Status | Python SY5527 exporter |

## データフロー詳細

### InfluxDB Writer（Operator 内バックグラウンドタスク）

```rust
// 疑似コード
let client = reqwest::Client::new();
let mut prev_counts: HashMap<(u32, u32), u64> = HashMap::new();
let mut last_instant = Instant::now();

loop {
    sleep(Duration::from_secs(interval_secs)).await;
    let elapsed = last_instant.elapsed().as_secs_f64();
    last_instant = Instant::now();

    // 1. Monitor API からチャンネル別 counts 取得
    let resp = client.get(monitor_url + "/api/histograms").await?;
    let histogram_list: HistogramListResponse = resp.json().await?;

    // 2. 実測 elapsed を使って正確なレート計算
    let mut lines = String::new();
    for ch in histogram_list.channels {
        let key = (ch.module_id, ch.channel_id);
        let prev = prev_counts.get(&key).copied().unwrap_or(0);
        let delta = ch.total_counts as i64 - prev as i64;
        let rate = if delta > 0 { delta as f64 / elapsed } else { 0.0 };

        // Line Protocol 生成
        writeln!(lines,
            "channel_rate,module={},channel={},name={} counts={}i,rate={:.2} {}",
            ch.module_id, ch.channel_id, ch.name,
            ch.total_counts, rate, timestamp_ns
        );
        prev_counts.insert(key, ch.total_counts);
    }

    // 3. 全体メトリクス
    // Recorder の ComponentMetrics から bytes, data_rate
    // Monitor の event_rate, total_events

    // 4. HTTP POST to InfluxDB
    client
        .post(format!("{}/api/v2/write?bucket={}&precision=ns", influxdb_url, database))
        .body(lines)
        .send().await?;
}
```

### エラーハンドリング

- InfluxDB 接続失敗: warn ログ、次のサイクルでリトライ。DAQ は止めない
- Monitor 接続失敗: warn ログ、スキップ
- `[influxdb] enabled = false`: タスク自体を起動しない
- バッファリング: MVP では不要。warn + 次サイクルリトライで十分

## TOML 設定

```toml
[influxdb]
enabled = true
url = "http://localhost:8181"
database = "delila"
interval_secs = 2
```

## Docker 構成

`docker/docker-compose.yml` に追加:

```yaml
  influxdb3:
    image: influxdb:3-core
    container_name: delila_influxdb3
    restart: unless-stopped
    ports:
      - "8181:8181"
    volumes:
      - ./data/influxdb3:/var/lib/influxdb3
    command:
      - "serve"
      - "--node-id"
      - "delila"
      - "--object-store"
      - "file"
      - "--data-dir"
      - "/var/lib/influxdb3"

  grafana:
    image: grafana/grafana:latest
    container_name: delila_grafana
    restart: unless-stopped
    ports:
      - "3000:3000"
    environment:
      GF_INSTALL_PLUGINS: "influxdata-flightsql-datasource"
      GF_SECURITY_ADMIN_USER: admin
      GF_SECURITY_ADMIN_PASSWORD: delila
      GF_AUTH_ANONYMOUS_ENABLED: "true"
      GF_AUTH_ANONYMOUS_ORG_ROLE: Viewer
    volumes:
      - ./data/grafana:/var/lib/grafana
      - ./grafana/provisioning:/etc/grafana/provisioning
      - ./grafana/dashboards:/var/lib/grafana/dashboards
    depends_on:
      - influxdb3
```

注: InfluxDB v3 Core は v2 とは異なり、Org/Bucket/Token モデルを持たない。
認証は MVP では不要（ローカルネットワーク + Docker 内部通信）。

## Grafana Provisioning

### データソース — Flight SQL

InfluxDB v3 Core は SQL (DataFusion) が第一級市民。Flux は v3 では非推奨。

`docker/grafana/provisioning/datasources/influxdb.yml`:

```yaml
apiVersion: 1
datasources:
  - name: InfluxDB
    type: influxdata-flightsql-datasource
    access: proxy
    url: http://influxdb3:8181
    jsonData:
      database: delila
```

注: Flight SQL プラグインの正確な設定は実装時に検証する。
InfluxQL 互換モードの方が簡単な場合はそちらを使用。

### ダッシュボード (`docker/grafana/provisioning/dashboards/json/Common/delila_overview.json`)

JSON provisioning で自動構成。パネル構成:

| # | パネル | タイプ | クエリ |
|---|--------|--------|--------|
| 1 | **チャンネル別イベントレート** | Time Series | `SELECT rate FROM channel_rate WHERE ...` module+channel でグループ |
| 2 | **全体イベントレート** | Time Series + Stat | `SELECT event_rate FROM system_rate` |
| 3 | **データスループット** | Time Series | `SELECT data_rate FROM system_rate` (MB/s 表示) |
| 4 | **ラン情報** | Annotation | `run_info` の state 変化をアノテーション表示 |
| 5 | **チャンネル別累積カウント** | Table / Bar | `SELECT counts FROM channel_rate` の最新値 |

## 実装ステップ

| # | タスク | 変更箇所 | 見積 |
|---|--------|----------|------|
| 1 | docker-compose に InfluxDB v3 Core + Grafana 追加 | `docker/docker-compose.yml` | 小 |
| 2 | `[influxdb]` TOML 設定追加 | `src/config/mod.rs` | 小 |
| 3 | InfluxDB Writer タスク実装 | `src/operator/influxdb.rs` (新規) | 中 |
| 4 | Operator 起動時に Writer タスク spawn | `src/operator/mod.rs` | 小 |
| 5 | Grafana データソース provisioning (Flight SQL) | `docker/grafana/provisioning/` | 小 |
| 6 | Grafana ダッシュボード JSON 作成 | `docker/grafana/dashboards/` | 中 |
| 7 | 動作確認 + Write API エンドポイント検証 | — | 中 |

### Post-MVP

- リテンションポリシー: InfluxDB v3 Core のデータ保持期間設定（v2 とは方式が異なる。要調査）
- 認証: API Token による InfluxDB アクセス制御
- バッファリング: 一時的な接続障害時のデータ保持（tokio mpsc チャンネル）

## ポート一覧（更新）

| サービス | ポート |
|----------|--------|
| Operator | 9090 |
| Monitor | 8081 |
| MongoDB | 27017 |
| mongo-express | 8083 |
| **InfluxDB v3 Core** | **8181** |
| **Grafana** | **3000** |
