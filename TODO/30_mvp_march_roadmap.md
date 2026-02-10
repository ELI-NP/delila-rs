# MVP March 2026 Roadmap

**Created:** 2026-02-10
**Status:** 計画中
**Target:** 2026年3月中旬

## 目標

1. **172.18.4.76 で PSD2 + PSD1 + PHA1 DAQ を走らせる**
2. **カウントレート・HV モニタリング（Grafana 別系統）**
3. **簡単にデプロイ・管理・運用できるシステム**

---

## Goal 1: PSD2 + PSD1 + PHA1 全ファームウェア DAQ

### 現状
- PSD2 (VX2730 Ethernet): 実機動作済み
- PSD1 (VX1730B 光リンク x5): 実機動作済み
- PHA1 (VX1730B 光リンク): デコーダ完成・テスト済み。本番構成は未定

### タスク

| # | タスク | 優先度 | 依存 | 見積 |
|---|--------|--------|------|------|
| 1-1 | PHA1 用 JSON コンフィグテンプレート作成 | **高** | なし | 小 |
| 1-2 | PHA1 デジタイザの Settings UI パラメータ確認 | **中** | 1-1 | 小 |
| 1-3 | config_76_production.toml に PHA1 ソース追加 | **高** | ハードウェア確定後 | 小 |
| 1-4 | PHA1 実機接続テスト (76) | **高** | 1-3 + HW | 中 |
| 1-5 | Event Builder オンラインパイプライン統合 | **高** | 下記 Goal 1-EB 参照 | 大 |

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
| EB-1 | Event Builder を ZMQ SUB コンポーネント化 | **高** | 中 |
| EB-2 | component_architecture.md パターンに準拠（CommandLoop + ProcessLoop） | **高** | 中 |
| EB-3 | Operator に EB コンポーネント追加（Configure/Start/Stop 制御） | **高** | 中 |
| EB-4 | EB 出力形式決定（ROOT? MessagePack? 別 PUB?） | **高** | 設計 |
| EB-5 | Web UI で EB ステータス表示 | **低** | 小 |
| EB-6 | 実データでの E2E テスト (PSD1+PSD2+PHA1 → EB → 出力検証) | **高** | 中 |

**検討事項:**
- EB は Merger の PUB を SUB する（Monitor/Recorder と同じパターン）
- EB の出力: built events を ROOT ファイルに書き出し？ または別の ZMQ PUB で下流へ?
- Time Slice パラメータ（coincidence window 等）の設定 UI
- EB は TOML の `[network]` セクションに `[network.event_builder]` として追加

---

## Goal 2: カウントレート・HV モニタリング（Grafana）

### 方針
DAQ Web UI には統合しない。Grafana + Prometheus/InfluxDB で別系統のモニタリングダッシュボードを構築。

### タスク

| # | タスク | 優先度 | 見積 |
|---|--------|--------|------|
| 2-1 | Prometheus exporter 設計（Monitor API → metrics） | **中** | 中 |
| 2-2 | HV exporter 作成（SY5527 → Prometheus/InfluxDB） | **中** | 中 |
| 2-3 | docker-compose に Prometheus + Grafana 追加 | **中** | 小 |
| 2-4 | Grafana ダッシュボード作成（レート、HV 電圧/電流） | **中** | 中 |

**選択肢:**
- **A: Prometheus exporter (推奨)** — Python/Rust で `/metrics` エンドポイント提供。Prometheus が scrape
- **B: InfluxDB + Telegraf** — Telegraf が Monitor API と SY5527 をポーリング → InfluxDB → Grafana

**DAQ カウントレート exporter:**
- Monitor の `GET /api/histograms` から per-channel count を取得
- `GET /api/status` から total_events, event_rate を取得
- Prometheus gauge として公開

**HV exporter:**
- 既存の `tools/hv_calibration/` の CAENHVWrapper バインディングを再利用
- VMon (実測電圧), IMon (実測電流), V0Set (設定電圧), Status を定期読み出し
- Prometheus gauge として公開

---

## Goal 3: デプロイ・管理・運用改善

### タスク

| # | タスク | 優先度 | 参照 | 見積 |
|---|--------|--------|------|------|
| 3-1 | Operator ステータスポーリング並列化 (A1) | **高** | TODO/26 | 小 |
| 3-2 | start_daq.sh 改善: ヘルスチェック + サマリー (C1) | **中** | TODO/26 | 小 |
| 3-3 | タイムアウト TOML 設定化 (C3) | **中** | TODO/26 | 小 |
| 3-4 | 設定自動生成スクリプト (A3) | **中** | TODO/26 | 中 |
| 3-5 | デプロイスクリプト改善 (rsync + build + restart 一発) | **中** | なし | 小 |
| 3-6 | rust-embed でフロントエンド埋め込み（単一バイナリ） | **低** | なし | 中 |
| 3-7 | systemd サービスファイル作成 | **低** | なし | 小 |

### 優先順位
1. A1 (並列化) — 10台で体感速度改善、コード変更小
2. C1 (start_daq.sh) — 運用品質向上
3. C3 (タイムアウト) — 10台 USB で timeout 回避
4. A3 (設定テンプレート) — ハードウェア確定後
5. 3-5 (デプロイ) — 頻繁にデプロイする今こそ
6. 3-6/3-7 — 余裕があれば

---

## 実装スケジュール案

### Phase 1: 基盤整備 (2月中旬)
- [ ] 3-1: ステータス並列化 (A1)
- [ ] 3-2: start_daq.sh 改善 (C1)
- [ ] 3-3: タイムアウト設定化 (C3)
- [ ] 1-1: PHA1 コンフィグテンプレート
- [ ] 1-2: PHA1 Settings UI パラメータ確認

### Phase 2: Event Builder 統合 (2月下旬〜3月上旬)
- [ ] EB-1〜EB-4: Event Builder オンライン化
- [ ] 3-4: 設定自動生成（ハードウェア確定後）

### Phase 3: モニタリング + 実機テスト (3月上旬)
- [ ] 2-1〜2-4: Grafana モニタリング
- [ ] 1-3〜1-4: PHA1 実機テスト
- [ ] EB-6: 全ファームウェア E2E テスト

### Phase 4: 運用安定化 (3月中旬 — MVP)
- [ ] 3-5: デプロイ改善
- [ ] 最終統合テスト（PSD1 + PSD2 + PHA1 + EB + Grafana）

---

## 「やらないこと」(MVP scope外)

| 項目 | 理由 |
|------|------|
| L2 Filter | 3-4月実験では不要 |
| MongoDB 必須化 | --no-mongo で十分動く |
| HV 制御 Web UI | Grafana で読み取り、設定変更は既存 Python ツール |
| 分散 Merger | 10台程度では不要 |
| rust-embed | あれば便利だが MVP ブロッカーではない |
