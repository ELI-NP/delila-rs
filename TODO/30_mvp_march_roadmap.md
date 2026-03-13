# MVP March 2026 Roadmap

**Created:** 2026-02-10
**Updated:** 2026-03-13
**Status: ✅ MVP 達成**
**Target:** 2026年3月中旬

## 目標

1. **172.18.4.76 で PSD2 + PSD1 + PHA1 DAQ を走らせる**
2. **カウントレート・HV モニタリング（Grafana 別系統）**
3. **簡単にデプロイ・管理・運用できるシステム**
4. **ELOG 電子ログブック統合（Run Stop 時に自動投稿）**

---

## Goal 1: PSD2 + PSD1 + PHA1 全ファームウェア DAQ

### 現状: ✅ 全 FW 実機稼働中
- PSD2 (VX2730 Ethernet): 実機動作済み
- PSD1 (VX1730B 光リンク x5): 実機動作済み
- PHA1 (VX1730B 光リンク): 実機動作済み（2026-03-13 確認）

### タスク

| # | タスク | 優先度 | 依存 | 見積 | 状態 |
|---|--------|--------|------|------|------|
| 1-1 | PHA1 用 JSON コンフィグテンプレート作成 | **高** | なし | 小 | ✅ 完了 (`config/digitizers/pha1_template.json`) |
| 1-2 | PHA1 デジタイザの Settings UI パラメータ確認 | **中** | 1-1 | 小 | ✅ 完了 |
| 1-3 | config_76_production.toml に PHA1 ソース追加 | **高** | ハードウェア確定後 | 小 | ✅ 完了 |
| 1-4 | PHA1 実機接続テスト (76) | **高** | 1-3 + HW | 中 | ✅ 完了 — 実機稼働中 |
| 1-5 | Event Builder オンラインパイプライン統合 | **高** | 下記 Goal 1-EB 参照 | 大 | ⏭️ 3-4月実験では不要。夏以降の実験で必要 |

### Goal 1-EB: Event Builder オンライン統合

現在 Event Builder は standalone offline ツール。3月 MVP ではオンラインパイプライン内で動作させる。

**アーキテクチャ案:**
```
Reader x N → Merger → PUB ──┬── Monitor (SUB, 既存)
                             ├── Recorder (SUB, 既存: raw data 保存)
                             └── EventBuilder (SUB, 新規)
                                  └── PUB → EB-Recorder (SUB, built events 保存)
```

| # | タスク | 優先度 | 見積 |
|---|--------|--------|------|
| EB-1 | Event Builder を ZMQ SUB コンポーネント化 | **高** | 中 | ✅ 完了 (online.rs 既存) |
| EB-2 | component_architecture.md パターンに準拠（CommandLoop + ProcessLoop） | **高** | 中 | ✅ 完了 (online.rs 既存) |
| EB-3 | Operator に EB コンポーネント追加（Configure/Start/Stop 制御） | **高** | 中 | ✅ 完了 (online.rs 既存) |
| EB-4 | EB 出力形式決定（ROOT? MessagePack? 別 PUB?） | **高** | 設計 | ✅ 確定: oxyroot ROOT file-per-batch |
| EB-4.5 | **統一パイプライン: HitSource + pipeline.rs + Offline CLI** | **高** | 大 | ✅ Phase 0-3 完了 ([TODO](event-builder/38_eb_unification_mimalloc.md)) |
| EB-4.6 | **Online EB → 統一パイプライン移行 (ZmqHitSource)** | **中** | 中 | 📋 夏以降の実験で必要 |
| EB-4.7 | **レガシーコード削除 (SliceBuilder, L1Builder)** | **低** | 小 | EB-4.6 完了後 |
| EB-5 | Web UI で EB ステータス表示 | **低** | 小 | EB-4.6 完了後 |
| EB-6 | 実データでの E2E テスト (PSD1+PSD2+PHA1 → EB → 出力検証) | **高** | 中 | ✅ Offline EB で検証済 |

**検討事項:**
- EB は Merger の PUB を SUB する（Monitor/Recorder と同じパターン）
- EB の出力: built events を ROOT ファイルに書き出し？ または別の ZMQ PUB で下流へ?
- Time Slice パラメータ（coincidence window 等）の設定 UI
- EB は TOML の `[network]` セクションに `[network.event_builder]` として追加

---

## Goal 2: カウントレート・HV モニタリング + 通知

### 方針
DAQ Web UI には統合しない。**InfluxDB v3 Core + Grafana** で別系統のモニタリングダッシュボードを構築。
通知は **Webhook（Telegram or Discord — 実装時に決定）** で実現。
インフラは **Docker (docker-compose)** で管理。

### アーキテクチャ
```
Rust (Operator/Monitor)  ──→  InfluxDB v3 Core  ──→  Grafana  ──→  Webhook (Alert)
                          ──→  Webhook (直接 POST: Run start/stop, エラー)
Python (HV tools)         ──→  InfluxDB v3 Core  ──→  Grafana  ──→  Webhook (Alert)
```

### タスク

| # | タスク | 優先度 | 見積 | 状態 |
|---|--------|--------|------|------|
| 2-1 | docker-compose に InfluxDB v3 Core + Grafana 追加 | **高** | 小 | ✅ 完了 |
| 2-2 | Rust → InfluxDB 書き込み（Operator/Monitor からレート・イベント数を push） | **高** | 中 | ✅ 完了 |
| 2-3 | Python HV exporter → InfluxDB 書き込み（SY5527 VMon/IMon/Status） | **中** | 中 | 将来タスク |
| 2-4 | Grafana ダッシュボード作成（レート、HV 電圧/電流） | **中** | 中 | ✅ 完了 (DAQ Overview + Channel Rate) |
| 2-5 | Webhook 通知統合（Run start/stop を Rust から直接送信） | **中** | 小 | 将来タスク |
| 2-6 | Grafana → Webhook アラート設定（レート低下、HV トリップ等） | **中** | 小 | 将来タスク |

### 技術選定の理由
- **InfluxDB v3 Core** (not Prometheus): Push 型で DAQ からの直接書き込みに適合。InfluxQL サポートあり（v1 経験を活用）。Telegraf 不要
- **Webhook 通知 (Telegram or Discord)**: API がシンプル（HTTP POST 1発）。Grafana Alert にも built-in 対応。薄い抽象層で切り替え可能
- **Docker**: InfluxDB + Grafana のインフラをコードで管理。172.18.4.76 にそのままデプロイ可能

---

## Goal 3: デプロイ・管理・運用改善

### タスク

| # | タスク | 優先度 | 参照 | 見積 | 状態 |
|---|--------|--------|------|------|------|
| 3-1 | Operator ステータスポーリング並列化 (A1) | **高** | TODO/26 | 小 | ✅ 完了 (`client.rs`: `join_all` 化) |
| 3-2 | start_daq.sh 改善: ヘルスチェック + サマリー (C1) | **中** | TODO/26 | 小 | ✅ 完了 (ヘルスチェック + サマリーテーブル + Operator API 状態表示) |
| 3-3 | タイムアウト TOML 設定化 (C3) | **中** | TODO/26 | 小 | ✅ 完了 (`[operator]` に `configure/arm/start/reset_timeout_ms`) |
| 3-4 | 設定自動生成スクリプト (A3) | **中** | TODO/26 | 中 | |
| 3-5 | デプロイスクリプト改善 (rsync + build + restart 一発) | **中** | なし | 小 | |
| 3-6 | rust-embed でフロントエンド埋め込み（単一バイナリ） | **低** | なし | 中 | |
| 3-7 | systemd サービスファイル作成 | **低** | なし | 小 | |

### 優先順位
1. A1 (並列化) — 10台で体感速度改善、コード変更小
2. C1 (start_daq.sh) — 運用品質向上
3. C3 (タイムアウト) — 10台 USB で timeout 回避
4. A3 (設定テンプレート) — ハードウェア確定後
5. 3-5 (デプロイ) — 頻繁にデプロイする今こそ
6. 3-6/3-7 — 余裕があれば

---

## 実装スケジュール案

### Phase 1: 基盤整備 (2月中旬) — ✅ 完了
- [x] 3-1: ステータス並列化 (A1) — `get_all_status`/`execute_on_all` を `join_all` で並列化
- [x] 3-2: start_daq.sh 改善 (C1) — ヘルスチェック、PID サマリーテーブル、Operator API 状態表示
- [x] 3-3: タイムアウト設定化 (C3) — `[operator]` セクションに 4 種タイムアウト追加
- [x] 1-1: PHA1 コンフィグテンプレート — `config/digitizers/pha1_template.json`
- [x] 1-2: PHA1 Settings UI パラメータ確認

**発見事項:** DIG1 (PSD1/PHA1) の時間パラメータは ns→samples 変換不要。DevTree が ns を直接受け付ける (`expuom: -9`)。テスト修正済み。

### Phase 2: Event Builder 統合 (2月下旬〜3月上旬) — ✅ 完了 (Offline EB で十分)
- [x] EB-4.5: 統一パイプライン Phase 0-3 (source.rs, pipeline.rs, Offline CLI rewrite)
- [x] EB-6: Offline EB で全 FW データ検証済
- EB-4.6 (Online 移行): 3-4月実験では不要、夏以降の実験で実装予定

### Phase 3: モニタリング + ELOG + 実機テスト (3月上旬〜中旬) — ✅ 完了
- [x] 2-1: Docker インフラ整備（InfluxDB v3 Core + Grafana + ELOG の docker-compose）
- [x] 4-1: ELOG Docker セットアップ
- [x] 4-2〜4-3: ELOG Rust クライアント + TOML 設定
- [x] 4-4: Run Stop 自動投稿
- [x] 2-2: Rust → InfluxDB メトリクス書き込み
- [x] 2-4: Grafana ダッシュボード (DAQ Overview + Channel Rate)
- [x] 1-3〜1-4: PHA1 実機テスト — 稼働中

### Phase 4: 運用安定化 (3月中旬 — MVP) — ✅ 達成
- [x] 全 FW DAQ 稼働中 (PSD2 + PSD1 + PHA1)
- [x] Grafana モニタリング稼働中
- [x] ELOG 自動投稿稼働中

---

## Goal 4: ELOG 電子ログブック統合

### 方針
PSI ELOG (https://elog.psi.ch/elog/) を Docker でセルフホスト。
Run Stop 時にデジタイザ設定 + 統計情報を自動投稿する。

### アーキテクチャ
```
Run Stop (Operator)
  → 統計情報収集（Duration, Event数, エラー数）
  → デジタイザ設定サマリ生成
  → ELOG HTTP POST（reqwest）
```

### タスク

| # | タスク | 優先度 | 見積 | 状態 |
|---|--------|--------|------|------|
| 4-1 | docker-compose に ELOG サーバー追加 | **高** | 小 | ✅ 完了 |
| 4-2 | ELOG Rust クライアントモジュール実装（`reqwest` HTTP POST） | **高** | 中 | ✅ 完了 |
| 4-3 | `[elog]` セクションを TOML 設定に追加 | **高** | 小 | ✅ 完了 |
| 4-4 | Run Stop 時の自動投稿（統計情報 + デジタイザ設定） | **高** | 中 | ✅ 完了 |
| 4-5 | Operator UI に手動 ELOG 投稿ボタン（任意コメント付き） | **低** | 中 | 将来タスク |

### 設定イメージ (`config.toml`)
```toml
[elog]
enabled = true
url = "http://localhost:8080"
logbook = "DELILA"
author = "DELILA-DAQ"
# write_password = ""
```

### 投稿内容（Run Stop 時）
```
Run #42 stopped

Duration: 01:23:45
Total events: 1,234,567
Event rate (avg): 278 evt/s

Digitizer config:
  - dig0 (VX2730, PSD2): 64ch, 500ns gate
  - dig1 (VX1730B, PSD1): 16ch x 5 boards
  - dig2 (VX1730B, PHA1): 16ch
```

---

## 「やらないこと」(MVP scope外)

| 項目 | 理由 |
|------|------|
| L2 Filter | 3-4月実験では不要 |
| MongoDB 必須化 | --no-mongo で十分動く |
| HV 制御 Web UI | Grafana で読み取り、設定変更は既存 Python ツール |
| 分散 Merger | 10台程度では不要 |
| rust-embed | あれば便利だが MVP ブロッカーではない |
