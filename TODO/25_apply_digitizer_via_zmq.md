# Apply Digitizer Config via ZMQ (Phase 6 拡張)

**Created:** 2026-02-03
**Status:** IMPLEMENTATION PLANNED
**Priority:** High — Settings UI E2E の最終ピース
**Parent:** `TODO/19_settings_ui.md`

---

## 問題

UI の Apply ボタンは Operator メモリのみ更新。Reader はファイルから設定を読むため、
ハードウェア反映には **Save → Stop → Reset → Configure** の 4 ステップが必要。
Running 中の設定変更は不可能。

## 解決策

新コマンド `ApplyDigitizerConfig(DigitizerConfig)` を追加。

```
[Angular UI] ── Apply ──► POST /api/digitizers/{id}/apply
                                │
                                ▼
                          [Operator]
                           ├── 1. メモリ更新
                           ├── 2. ディスク保存 (digitizer_{id}.json)
                           └── 3. ZMQ 送信 ──► [Reader CMD handler]
                                                    │
                                                    ▼ (channel delegation)
                                              [read_loop]
                                                    │
                                              CaenHandle::apply_config()
                                                    │
                                                    ▼
                                              [Hardware Applied]
```

- State machine 状態変更なし（Idle/Configured から実行可能）
- Running/Armed からは拒否（データ取得中のパラメータ変更は危険）
- `UpdateEmulatorConfig` パターンに準拠

---

## 設計判断

1. **DetectRequest → ReadLoopRequest enum に統合** — チャネル乱立を防ぐ
2. **Idle/Configured のみ許可** — Running 中の HW 書き込みは CAEN FELib 仕様上も危険
3. **エンドポイントは `/api/digitizers/{id}/apply`** — PUT (メモリのみ) と分離
4. **ディスク保存は best-effort** — 保存失敗でも HW 適用は続行
5. **タイムアウト 10 秒** — USB 経由の DT5730B は遅いため Detect (5s) より長めに設定

---

## 実装手順

### Step 1: Command enum に ApplyDigitizerConfig 追加

**ファイル:** `src/common/command.rs`

```rust
pub enum Command {
    // ... existing ...
    /// Apply digitizer configuration to hardware (Reader-only)
    ApplyDigitizerConfig(crate::config::digitizer::DigitizerConfig),
}
```

変更点:
- Command enum に variant 追加
- `Display` impl に arm 追加: `ApplyDigitizerConfig(id={})`
- `valid_commands()`: Idle に `"ApplyDigitizerConfig"` 追加、Configured にも追加

### Step 2: CommandHandlerExt trait + handle_command() 拡張

**ファイル:** `src/common/state.rs`

trait に default method 追加:
```rust
fn on_apply_digitizer_config(
    &mut self,
    _config: &crate::config::digitizer::DigitizerConfig,
) -> Result<usize, String> {
    Err("ApplyDigitizerConfig not supported".to_string())
}
```

`handle_command()` に新 match arm:
- Idle/Configured のみ許可（他は error response）
- `ext.on_apply_digitizer_config(config)` 呼び出し
- 成功: `CommandResponse::success().with_data(json!({"params_applied": count}))`
- 状態変更なし

### Step 3: Reader — DetectRequest → ReadLoopRequest 統合

**ファイル:** `src/reader/mod.rs`

```rust
enum ReadLoopRequest {
    Detect {
        response_tx: std::sync::mpsc::SyncSender<Result<serde_json::Value, String>>,
    },
    ApplyConfig {
        config: crate::config::digitizer::DigitizerConfig,
        response_tx: std::sync::mpsc::SyncSender<Result<usize, String>>,
    },
}
```

変更箇所:
- `DetectRequest` struct → `ReadLoopRequest` enum に置換
- `ReaderCommandExt`: `detect_tx` → `request_tx: Sender<ReadLoopRequest>`
- `on_detect()`: `ReadLoopRequest::Detect` 使用
- `on_apply_digitizer_config()` 実装: `ReadLoopRequest::ApplyConfig` 送信、timeout 10s
- `read_loop_raw` / `read_loop_opendpp`:
  - パラメータ名 `detect_rx` → `request_rx`
  - `try_recv()` で match、`ApplyConfig` 時は `handle.apply_config(&config)` 呼び出し
- `Reader::run()`: チャネル名変更 `(request_tx, request_rx)`

### Step 4: Operator — 新 REST エンドポイント

**ファイル:** `src/operator/routes/digitizer.rs`

`POST /api/digitizers/{id}/apply` ハンドラ:
1. path ID と config.digitizer_id の一致確認
2. `state.digitizer_configs.write().insert(id, config.clone())`
3. `config_dir/digitizer_{id}.json` に書き出し (best-effort)
4. `state.components` から `is_digitizer && source_id == Some(id)` の Reader を検索
5. `client.send_command(address, Command::ApplyDigitizerConfig(config))` 送信
6. レスポンス返却

**ファイル:** `src/operator/routes/mod.rs`
- import に `apply_digitizer_config` 追加
- route: `.route("/api/digitizers/:id/apply", post(apply_digitizer_config))`
- OpenAPI paths に登録

### Step 5: Frontend — サービス + コンポーネント修正

**ファイル:** `web/operator-ui/src/app/services/digitizer.service.ts`

新メソッド:
```typescript
async applyToHardware(config: DigitizerConfig): Promise<ApiResponse> {
  return await firstValueFrom(
    this.http.post<ApiResponse>(`${this.apiUrl}/${config.digitizer_id}/apply`, config)
  );
}
```

**ファイル:** `web/operator-ui/src/app/components/digitizer-settings/digitizer-settings.component.ts`

`applyConfig()` メソッドを修正:
- `updateDigitizer()` (PUT, メモリのみ) → `applyToHardware()` (POST, メモリ+ディスク+HW) に変更
- スナックバーで結果表示（成功: `"Applied N parameters"` / 失敗: エラーメッセージ）

---

## 変更ファイル一覧

| Action | File | Description |
|--------|------|-------------|
| Modify | `src/common/command.rs` | `ApplyDigitizerConfig` variant 追加 |
| Modify | `src/common/state.rs` | trait method + handle_command dispatch |
| Modify | `src/reader/mod.rs` | `ReadLoopRequest` enum + `on_apply_digitizer_config` 実装 |
| Modify | `src/operator/routes/digitizer.rs` | `POST /api/digitizers/{id}/apply` エンドポイント |
| Modify | `src/operator/routes/mod.rs` | ルート登録 + OpenAPI |
| Modify | `web/.../services/digitizer.service.ts` | `applyToHardware()` メソッド追加 |
| Modify | `web/.../digitizer-settings/digitizer-settings.component.ts` | `applyConfig()` 修正 |

---

## 検証手順

1. `cargo build --release`
2. DAQ 起動 (`./scripts/start_daq.sh config/config_psd2_test.toml`)
3. Detect → S/N: 52622 確認
4. UI で ch8 を Enable に変更
5. Apply クリック → Reader ログ: `"Digitizer config applied to hardware"` + `params_applied > 0`
6. `config/digitizers/digitizer_0.json` に ch8 override 書き込み確認
7. Configure → Arm → Start → データ取得確認（設定が壊れていないこと）
8. Idle 状態から Apply → 同じく成功すること

---

## 将来拡張

- Running 中の `setinrun=true` パラメータのみ部分適用（Phase 7 以降）
- MongoDB 保存 (バージョニング) との統合
- Apply 結果の詳細レポート（パラメータ毎の成功/失敗）
