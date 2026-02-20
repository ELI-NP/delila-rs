# Grafana モニタリング (InfluxDB v3 Core + Grafana)

**Status: 📋 計画完了・実装待ち**
**Updated:** 2026-02-19
**Design Doc:** [docs/plans/grafana_monitoring.md](../docs/plans/grafana_monitoring.md)（Gemini レビュー済み）

## 目的

実験中のチャンネル別イベントレート・スループットを Grafana ダッシュボードでリアルタイム可視化する。
2秒間隔でビームオン/オフサイクルが識別できる時間分解能を確保。

## アーキテクチャ

Operator にバックグラウンドタスク（初）を追加。
Monitor HTTP API (`GET /api/histograms`) をポーリングし、差分レートを計算して
InfluxDB v3 Core に Line Protocol で書き込む。

```
Operator → Monitor HTTP → rate計算 → InfluxDB v3 Core (:8181) → Grafana (:3000)
```

## 実装ステップ

### Step 1: 依存関係 + 設定
- [ ] `Cargo.toml` に `reqwest = { version = "0.12", features = ["json"] }` 追加
- [ ] `src/config/mod.rs` に `InfluxDbConfig` struct 追加（enabled, url, database, interval_secs）
- [ ] `Config` struct に `pub influxdb: Option<InfluxDbConfig>` 追加
- [ ] Config テスト追加

### Step 2: InfluxDB Writer タスク
- [ ] `src/operator/influxdb.rs` 新規作成（~250行）
  - Monitor レスポンス型定義（HistogramListResponse, ChannelSummary）
  - InfluxDbWriter struct（reqwest::Client, prev_counts, last_instant）
  - `pub async fn run_writer(config, monitor_url, app_state)` メインループ
  - `write_cycle()`: Monitor HTTP → delta/rate計算 → Line Protocol → POST
  - `Instant::elapsed()` で実測間隔（ジッター補正）
  - delta < 0（Clear/Reset時）→ rate = 0.0
  - Recorder メトリクス: `app_state.client.get_status()` → bytes/data_rate
  - `run_info` 状態変化検出（CurrentRunInfo の有無）
  - エラー: warn + skip（DAQ を止めない）
- [ ] ユニットテスト: Line Protocol 生成、レート計算、tag エスケープ

### Step 3: Operator 配線
- [ ] `src/operator/mod.rs` に `pub mod influxdb;` 追加
- [ ] `src/operator/routes/mod.rs`: `build()` → `(Router, Arc<AppState>)` に変更
- [ ] `src/bin/operator.rs`: load_config() で influxdb_config 取得 + tokio::spawn

### Step 4: ビルド確認
- [ ] `cargo fmt && cargo clippy -- -D warnings && cargo test`

### Step 5: Docker
- [ ] `docker/docker-compose.yml` に influxdb3 + grafana 追加
- [ ] `docker/grafana/provisioning/datasources/influxdb.yml` (Flight SQL)
- [ ] `docker/grafana/provisioning/dashboards/default.yml`
- [ ] `docker/grafana/dashboards/delila_overview.json` (5パネル)

### Step 6: 統合テスト
- [ ] Docker 起動 → Operator → InfluxDB 書き込み確認
- [ ] Grafana ダッシュボード表示確認
- [ ] 172.18.4.147 デプロイ後の実機テスト

## メトリクス（3 Measurements）

| Measurement | 内容 | 書き込み頻度 |
|-------------|------|-------------|
| `channel_rate` | チャンネル別 counts + rate | 2秒 |
| `system_rate` | 全体 total_events + event_rate + bytes + data_rate | 2秒 |
| `run_info` | state + run_number + comment | 状態変化時のみ |

## 変更ファイル一覧

| ファイル | 操作 | 行数 |
|---------|------|------|
| `Cargo.toml` | 編集 | +1 |
| `src/config/mod.rs` | 編集 | +40 |
| `src/operator/influxdb.rs` | **新規** | ~250 |
| `src/operator/mod.rs` | 編集 | +2 |
| `src/operator/routes/mod.rs` | 編集 | ~5行変更 |
| `src/bin/operator.rs` | 編集 | ~20行変更 |
| `docker/docker-compose.yml` | 編集 | +30 |
| `docker/grafana/provisioning/datasources/influxdb.yml` | **新規** | ~8 |
| `docker/grafana/provisioning/dashboards/default.yml` | **新規** | ~10 |
| `docker/grafana/dashboards/delila_overview.json` | **新規** | ~300 |

## コスト見積もり

- Rust コード: ~300行（influxdb.rs + config + 配線）
- Docker/Grafana: ~350行（compose + provisioning + dashboard JSON）
- リスク: 中（InfluxDB v3 Core の Write API/Flight SQL は実装時に検証要）
