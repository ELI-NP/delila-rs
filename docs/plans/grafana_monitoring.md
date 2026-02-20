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

### Measurement 1: `channel_rate` — チャンネル別イベントレート

```
channel_rate,module=0,channel=3,name=det03 counts=45234i,rate=1523.5 <ns_timestamp>
```

| Tag | 説明 |
|-----|------|
| `module` | デジタイザ module_id |
| `channel` | チャンネル番号 |
| `name` | チャンネル名（Channel Registration で登録した名前、なければ `m{module}_ch{channel}`） |

| Field | 型 | 説明 |
|-------|----|------|
| `counts` | integer | 累積イベント数（`total_counts`） |
| `rate` | float | `Instant::elapsed()` 実測値に基づくレート (Hz)。delta < 0（Clear/Reset時）は 0.0 |

**Cardinality**: ~200 series (module x channel) — InfluxDB v3 にとっては問題なし。

### Measurement 2: `system_rate` — システム全体

```
system_rate total_events=500000i,event_rate=15000.5,bytes=2000000i,data_rate=800000.0 <ns_timestamp>
```

| Field | 型 | 説明 |
|-------|----|------|
| `total_events` | integer | 全チャンネル合計イベント数 |
| `event_rate` | float | Monitor 算出の全体レート (Hz) |
| `bytes` | integer | Recorder の bytes_transferred |
| `data_rate` | float | Recorder の data_rate (bytes/s) |

### Measurement 3: `run_info` — ラン情報（状態変化時のみ書き込み）

```
run_info,state=Running run_number=42i,comment="production" <ns_timestamp>
```

| Tag | 説明 |
|-----|------|
| `state` | ComponentState (Idle, Running, etc.) |

| Field | 型 | 説明 |
|-------|----|------|
| `run_number` | integer | ラン番号 |
| `comment` | string | ランコメント |

### 将来追加（MVP 外）

| Measurement | 内容 | 備考 |
|-------------|------|------|
| `busy_status` | デジタイザ Busy カウンタ | Reader から FELib 経由で取得。未実装 |
| `event_builder` | ビルド済みイベント数 | EB オンライン化後 |
| `hv_monitor` | HV 電圧/電流/Status | Python SY5527 exporter |
| `disk_usage` | ディスク使用量 | Python スクリプト |

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

### ダッシュボード (`docker/grafana/dashboards/delila_overview.json`)

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
