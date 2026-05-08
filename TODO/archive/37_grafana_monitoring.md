# Grafana モニタリング (InfluxDB v3 Core + Grafana)

**Status: ✅ COMPLETED**
**Updated:** 2026-03-12
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

## 完了事項

- [x] `Cargo.toml` に `reqwest` 追加
- [x] `src/config/mod.rs` に `InfluxDbConfig` struct 追加
- [x] `src/operator/influxdb.rs` InfluxDB Writer タスク実装
- [x] Operator 配線 (`mod.rs`, `routes/mod.rs`, `bin/operator.rs`)
- [x] `cargo fmt && cargo clippy -- -D warnings && cargo test` パス
- [x] `docker/docker-compose.yml` に influxdb3 + grafana + mongo + mongo-express
- [x] `docker/grafana/provisioning/datasources/influxdb.yml` (Flight SQL)
- [x] `docker/grafana/provisioning/dashboards/default.yml`
- [x] `docker/grafana/provisioning/dashboards/json/delila_overview.json` (3パネル: Channel Rate, Total Rate, Data Rate)
- [x] `docker/grafana/provisioning/dashboards/json/channel_rate.json` (48ch 個別 Stat パネル: 3 module × 16 ch)
- [x] 192.168.147.98 デプロイ + 実機テスト完了（3MV config, PHA1 + PSD1 × 2）

## デプロイ情報

- **ホスト:** 192.168.147.98 (`daq@`)
- **Grafana:** http://192.168.147.98:3000 (admin / delila)
- **InfluxDB v3 Core:** http://192.168.147.98:8181 (認証なし)
- **Docker Compose:** `~/delila-rs/docker/docker-compose.yml`
- 匿名アクセス: Viewer（閲覧のみ）、編集には admin ログイン必要

## ダッシュボード

| ダッシュボード | UID | 内容 |
|---------------|-----|------|
| DELILA DAQ Overview | `delila-overview` | Channel Rate 時系列グラフ + Total Rate + Data Rate |
| Channel Rate | `delila-channel-rate` | 48ch 個別 Stat パネル (3 module × 16 ch)、背景色レート表示 |

## メトリクス（2 Measurements）

| Measurement | 内容 | Tags |
|-------------|------|------|
| `channel_rate` | チャンネル別 count (累積) | module, channel |
| `system_rate` | モジュール別 total_events + bytes (累積) | module |

Grafana 側で `non_negative_derivative(mean(...), 1s)` によりレート (Hz, bytes/s) を算出。
