# 10+ デジタイザ対応スケーリング計画

**Created:** 2026-02-03
**Status:** 計画中（叩き台）

---

## 結論: 現アーキテクチャはほぼそのまま使える

プロセス分離型 Reader、Merger の N対1 ZMQ fan-in、Operator の pipeline ordering は全て N台対応済み。
必要なのは **パフォーマンスのボトルネック解消** と **運用面の改善** のみ。

---

## 変更不要（既に動くもの）

| 機能 | 理由 |
|------|------|
| プロセス per Reader | CaenHandle が NOT thread-safe → プロセス分離が正解 |
| ポート割り当て (5555+id, 5560+id) | 10台でも 5555-5564, 5560-5569 で衝突なし |
| Master/Slave 同期 (TrgOut cascade) | `start_all_sequential()` が N台に対応済み |
| Merger single SUB → N Reader PUB | ZMQ が内部で多重化。zero-copy 転送 |
| Operator Vec\<ComponentConfig\> | 上限なし |
| TOML [[network.sources]] | 任意数のソース定義可能 |

---

## Phase A: Quick Wins（アーキテクチャ変更なし）

### A1. Operator ステータスポーリング並列化
- **問題:** `get_all_status()` が for ループで逐次実行（13コンポーネント × 5ms = 65ms）
- **修正:** `futures::future::join_all` で並列化 → 5ms（1 RTT）に短縮
- **影響:** `wait_for_state()` の収束も高速化（Start シーケンス全体が速くなる）
- **ファイル:** `src/operator/client.rs` — `get_all_status()`, `execute_on_all()`, `reset_all()`

<details><summary>修正案コード</summary>

```rust
// get_all_status (L98-105)
pub async fn get_all_status(&self, configs: &[ComponentConfig]) -> Vec<ComponentStatus> {
    let futures: Vec<_> = configs.iter().map(|c| self.get_status(c)).collect();
    futures::future::join_all(futures).await
}

// execute_on_all (L156-168)
pub async fn execute_on_all(
    &self,
    configs: &[ComponentConfig],
    command_fn: impl Fn(&ComponentConfig) -> Command,
) -> Vec<CommandResult> {
    let futures: Vec<_> = configs
        .iter()
        .map(|config| {
            let command = command_fn(config);
            self.execute_command(config, command)
        })
        .collect();
    futures::future::join_all(futures).await
}

// reset_all (L424-431)
pub async fn reset_all(&self, configs: &[ComponentConfig]) -> Vec<CommandResult> {
    let futures: Vec<_> = configs.iter().map(|c| self.reset(c)).collect();
    futures::future::join_all(futures).await
}
```

</details>

### A2. Detect 並列化
- **問題:** `detect_digitizers()` が逐次（USB デジタイザは 1台5秒、10台で50秒）
- **修正:** `send_command` 部分のみ `join_all` で同時発行。レスポンス処理（MongoDB検索・config更新）は逐次のまま（write lock 競合回避）
- **ファイル:** `src/operator/routes/digitizer.rs` — `detect_digitizers()`

<details><summary>修正案コード（detect_digitizers 内の for ループ部分を置き換え）</summary>

```rust
// Step 1: 全 Detect コマンドを並列発行（ボトルネック解消）
let detect_futures: Vec<_> = digitizer_components
    .iter()
    .map(|comp| async {
        let result = state
            .client
            .send_command(&comp.address, &Command::Detect)
            .await;
        (*comp, result)
    })
    .collect();
let detect_results = futures::future::join_all(detect_futures).await;

// Step 2: レスポンスを逐次処理（MongoDB lookup + config write lock）
for (comp, result) in detect_results {
    match result {
        Ok(resp) if resp.success => {
            // ... 既存のレスポンス処理ロジックそのまま ...
        }
        Ok(resp) => {
            errors.push(format!("{}: {}", comp.name, resp.message));
        }
        Err(e) => {
            errors.push(format!("{}: {}", comp.name, e));
        }
    }
}
```

</details>

### A3. 10台用設定テンプレート + 自動生成スクリプト
- **問題:** 10個の `[[network.sources]]` + Merger subscribe リスト手動管理はミスの元
- **修正:** `scripts/gen_config.sh` — CSV入力（id, ip, type, is_master）からTOML生成
- **成果物:** `config/config_10dig_template.toml` + `scripts/gen_config.sh`
- **優先度:** 中 — 2月中に10台運用開始予定。手書き10セクションはミスの元なので早めに用意

<details><summary>修正案（gen_config.sh の入出力イメージ）</summary>

入力 CSV (`config/digitizers.csv`):
```csv
# id, type, url, is_master
0, psd2, dig2://172.18.4.56, true
1, psd2, dig2://172.18.4.57, false
2, pha1, dig2://172.18.4.58, false
```

生成ルール:
- `bind = "tcp://*:$((5555+id))"`, `command = "tcp://*:$((5560+id))"`
- `merger.subscribe` = 全ソースの bind アドレス一覧を自動生成
- `pipeline_order` = is_master が true なら 1（最初に Start）、false なら 2
- Merger/Recorder/Monitor は固定テンプレート

```bash
#!/bin/bash
# scripts/gen_config.sh - Generate config.toml from digitizers.csv
CSV="${1:-config/digitizers.csv}"
OUTPUT="${2:-config.toml}"
SUBSCRIBE=""
# ... CSV をパースして [[network.sources]] を生成 ...
# ... SUBSCRIBE リストを自動構築 ...
# ... Merger/Recorder/Monitor は固定テンプレートを出力 ...
```

</details>

### ~~A4. Reader バッファサイズをファームウェア別に設定可能化~~ → 「やらないこと」に移動

---

## Phase B: 堅牢性（監視強化）

### ~~B1. Merger unbounded → bounded channel~~ → 「やらないこと」に移動
### ~~B2. Recorder unbounded → bounded channel~~ → 「やらないこと」に移動

### B3. キュー深度をメトリクスに追加
- **現状:** `ComponentMetrics` に `queue_size`/`queue_max` フィールドが既にあるが、両方ハードコード `0`
- **修正:** `received - sent - dropped` で算出するだけ。新規カウンタ不要
- **UI でキュー状況が見える** → 異常な滞留を早期検知
- **ファイル:** `src/merger/mod.rs`, `src/recorder/mod.rs`

<details><summary>修正案コード</summary>

```rust
// src/merger/mod.rs — get_metrics() (L254-265)
fn get_metrics(&self) -> Option<crate::common::ComponentMetrics> {
    let stats = self.ext_state.get_stats();
    let queue_depth = stats.received_batches
        .saturating_sub(stats.sent_batches)
        .saturating_sub(stats.dropped_batches);
    Some(crate::common::ComponentMetrics {
        events_processed: stats.sent_batches,
        bytes_transferred: 0,
        queue_size: queue_depth,
        queue_max: 0, // unbounded — 上限なし
        event_rate: 0.0,
        data_rate: 0.0,
    })
}

// src/recorder/mod.rs — get_metrics() (L519-531) も同様
// received_events - written_events - dropped_batches で算出
```

</details>

---

## Phase C: 運用改善

### C1. start_daq.sh 改善
- Reader 間の `sleep 0.3` 削除（独立プロセス、順序依存なし。ZMQ は再接続を自動でやる）
- 各コンポーネント起動後の `sleep 0.3` も削除（Merger/Recorder/Monitor 間に順序依存なし）
- 起動後ヘルスチェック追加（Operator の `/api/status` を数秒ポーリング → 全コンポーネント Idle 確認）
- 起動完了時にサマリーテーブル表示（source_id, type, port, PID）

<details><summary>修正案（差分イメージ）</summary>

```bash
# Reader ループ: sleep 0.3 削除
for id in $SOURCE_IDS; do
    # ... 既存の emulator/reader 起動ロジック ...
    PIDS["$id"]=$!
    # sleep 0.3 削除
done

# Merger/Recorder/Monitor/Operator: sleep 削除、一気に起動
$BINARY_DIR/merger --config "$CONFIG_FILE" > "$LOG_DIR/merger.log" 2>&1 &
$BINARY_DIR/recorder --config "$CONFIG_FILE" > "$LOG_DIR/recorder.log" 2>&1 &
$BINARY_DIR/monitor --config "$CONFIG_FILE" > "$LOG_DIR/monitor.log" 2>&1 &
# ... operator 起動 ...

# ヘルスチェック（Operator が最後に起動→全コンポーネント Idle 確認）
echo "Waiting for all components..."
for i in $(seq 1 30); do
    STATUS=$(curl -s http://localhost:8080/api/status 2>/dev/null)
    if echo "$STATUS" | grep -q '"all_idle":true'; then
        echo -e "${GREEN}All components ready${NC}"
        break
    fi
    sleep 0.5
done

# サマリーテーブル
printf "%-4s %-10s %-8s %-6s %s\n" "ID" "Type" "Port" "PID" "Status"
for id in $SOURCE_IDS; do
    printf "%-4s %-10s %-8s %-6s %s\n" "$id" "$(get_source_type $id)" "$((5560+id))" "${PIDS[$id]}" "OK"
done
```

</details>

### C2. ComponentStatus にコンポーネント種別・source_id 追加
- **現状:** `ComponentConfig` には `source_id` があるが、`ComponentStatus`（API レスポンス）にはない
- **修正:** `ComponentStatus` に `source_id` と `component_type` を追加。`get_status()` で `ComponentConfig` から転写
- **ファイル:** `src/operator/mod.rs`, `src/operator/client.rs`

<details><summary>修正案コード</summary>

```rust
// src/operator/mod.rs — ComponentStatus に追加
pub struct ComponentStatus {
    pub name: String,
    pub address: String,
    pub state: ComponentState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<u32>,        // 追加
    #[serde(skip_serializing_if = "Option::is_none")]
    pub component_type: Option<String>, // 追加 ("reader", "emulator", "merger", etc.)
    // ... 既存フィールド ...
}

// src/operator/client.rs — get_status() 内で転写
ComponentStatus {
    name: config.name.clone(),
    address: config.address.clone(),
    source_id: config.source_id,                    // 追加
    component_type: Some(component_type(config)),    // 追加
    // ...
}
```

</details>

### C3. Operator タイムアウトを TOML から設定可能に
- **現状:** `OperatorConfig` に `configure_timeout_ms` / `arm_timeout_ms` / `start_timeout_ms` がデフォルト 5000ms で存在するが、TOML `[operator]` セクションからは読めない（`experiment_name` のみ）
- **問題:** USB デジタイザ 10台の Configure は 5000ms を超過する可能性
- **修正:** `src/config/mod.rs` の `[operator]` パーサーにタイムアウトフィールドを追加し、`OperatorConfig` 構築時に反映
- **ファイル:** `src/config/mod.rs`, `src/bin/operator.rs`

<details><summary>修正案コード</summary>

```toml
# config.toml — [operator] セクションに追加（省略時はデフォルト 5000ms）
[operator]
experiment_name = "PSD2_10dig"
configure_timeout_ms = 15000  # USB 10台: 余裕を持たせる
arm_timeout_ms = 5000
start_timeout_ms = 10000
```

```rust
// src/config/mod.rs — OperatorTomlConfig（TOML パーサー側）に追加
#[serde(default)]
pub configure_timeout_ms: Option<u64>,
#[serde(default)]
pub arm_timeout_ms: Option<u64>,
#[serde(default)]
pub start_timeout_ms: Option<u64>,

// src/bin/operator.rs — OperatorConfig 構築時に TOML 値を反映
if let Some(ms) = toml_config.configure_timeout_ms {
    operator_config.configure_timeout_ms = ms;
}
// arm, start も同様
```

</details>

---

## やらないこと（KISS）

| 候補 | 理由 |
|------|------|
| 単一プロセス内マルチ Reader | CaenHandle が NOT thread-safe。プロセス分離が正しい |
| サービスマネージャ / watchdog | start_daq.sh + Operator status で十分 |
| 動的 Reader 追加 | ビームタイム中にハードウェア変更はしない |
| 分散 Merger / ロードバランシング | 10台 × 10kHz = 1.6 MB/s（波形なし）。単一 Merger で余裕 |
| ZMQ → gRPC/QUIC | 既存パターンが機能している。変更リスク大・価値ゼロ |
| Reader バッファサイズ設定化 | 10台×64MB=640MB。サーバメモリ(16-32GB)の2-4%で問題なし。KISS優先 |
| bounded channel 化 (B1/B2) | bounded にすると上流でデータロスが発生する。DAQでデータロスは許容不可。unbounded のまま B3 で監視 |

---

## データレート見積もり（10台同時）

| シナリオ | レート | Merger 負荷 | Recorder I/O | 判定 |
|----------|--------|-------------|-------------|------|
| 10×10kHz 波形なし | 100k evt/s | ~1.6 MB/s | HDD で余裕 | ✅ |
| 10×10kHz 波形あり | 100k evt/s | ~400 MB/s | NVMe 必須 | ⚠️ |
| 10×1kHz 波形あり | 10k evt/s | ~40 MB/s | SSD で余裕 | ✅ |

---

## テスト戦略（実機10台なしで検証）

1. **エミュレータ 10台テスト** — config.toml に emulator×10 を定義、フルパイプライン検証
2. **既存 2台実機で回帰テスト** — Phase A の並列化で既存動作が壊れないことを確認
3. **bounded channel ストレステスト** — emulator×10 を最大速度で回し、OOM しないことを確認
4. **起動時間測定** — 10 プロセス起動→全 Running 到達までの時間

---

## 実装順序

| 順番 | タスク | ファイル |
|------|--------|----------|
| A1 | get_all_status 並列化 | `src/operator/client.rs` |
| A2 | detect 並列化 | `src/operator/routes/digitizer.rs` |
| A3 | 設定テンプレート + gen_config.sh | `config/`, `scripts/` |
| B3 | キュー深度メトリクス | `src/merger/mod.rs`, `src/recorder/mod.rs` |
| C1 | start_daq.sh 改善 | `scripts/start_daq.sh` |
| C2 | ComponentStatus 拡張 | `src/operator/mod.rs`, `src/operator/client.rs` |
| C3 | タイムアウト設定化 | `src/config/mod.rs`, `src/bin/operator.rs` |
