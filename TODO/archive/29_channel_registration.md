# 29: Channel Registration + チャンネル名サポート

**Status: ✅ COMPLETED**
**Created:** 2026-02-05
**Priority:** 1 (Tune Up Mode の前提 + Monitor 機能改善)

---

## 概要

Monitor のヒストグラム/波形チャンネルを **事前登録** する仕組みを導入。
現在の「データ到着時に遅延生成」方式では空チャンネルが UI に表示されず、
Tune Up 開始直後にチャンネルリストが空になる問題がある。

**主要機能:**
- `RegisterChannels` ZMQ コマンドで Operator → Monitor にチャンネル定義を送信
- Monitor が空のヒストグラムを事前生成、チャンネルリスト API に登録チャンネルを含める
- 個別チャンネル名サポート (`DigitizerConfig.channel_names`)
- 通常 Configure 時と Tune Up 開始時の両方で送信

---

## Problem

### 1. Monitor のチャンネル可視性

現在の `MonitorState` はデータ到着時にヒストグラムを遅延生成する:

```rust
let histogram = self.histograms.entry(key).or_insert_with(|| {
    Histogram1D::new(module_id, channel_id, self.histogram_config)
});
```

→ データが来ていないチャンネルは `GET /api/histograms` や `GET /api/waveforms` に表示されない。
例えば 32ch デジタイザで Ch15 だけデータがない場合、UI でそのチャンネルが不可視になる。

### 2. Tune Up チャンネルリスト

Tune Up モード開始フロー:
1. Idle 状態 → Tune Up Start → Reader + Merger + Monitor が起動
2. フロントエンドが `fetchChannelList()` で `GET /api/waveforms` を呼ぶ
3. Monitor はまだデータを受信していない → **空リストを返す**
4. チャンネルセレクタが空 → 波形もヒストグラムも表示されない

`fetchChannelList()` は周期的に呼ばれないため、一度空が返ると手動 Refresh までチャンネルが表示されない。

---

## 設計判断

1. **ZMQ コマンド方式**: Operator → Monitor 間の通信は既存の ZMQ コマンドパターンに合わせる
2. **通常 Configure + Tune Up 両方**: Configure 後に全デジタイザのチャンネルを登録、Tune Up では対象のみ
3. **個別チャンネル名**: DigitizerConfig に `channel_names` フィールドを追加。未指定は `"{digitizer_name}/Ch{n}"` デフォルト
4. **Clear vs Reset**: Clear はデータのみクリア (registered_channels 保持、空ヒストグラム再生成)。Reset は全クリア
5. **波形の事前生成は不要**: 空波形に意味はない。チャンネルリスト API にのみ登録チャンネルを含める

---

## 実装 Phase

### Phase 1: Common — ChannelRegistration + Command variant

**File:** `src/common/command.rs`

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelRegistration {
    pub module_id: u32,     // = source_id = digitizer_id
    pub channel_id: u32,
    pub name: String,       // 個別チャンネル名 (e.g., "LaBr3-A" or "LaBr3 Array/Ch0")
}

// Command enum に追加:
RegisterChannels(Vec<ChannelRegistration>),
```

### Phase 2: Config — 個別チャンネル名サポート

**Files:** `src/config/mod.rs`, `web/.../models/types.ts`

Rust:
```rust
/// Optional per-channel names (key = channel index, value = name)
/// Channels without entries default to "{digitizer_name}/Ch{n}"
#[serde(default, skip_serializing_if = "Option::is_none")]
pub channel_names: Option<HashMap<u32, String>>,
```

TypeScript:
```typescript
channel_names?: Record<number, string>;
```

JSON config 例:
```json
{
  "digitizer_id": 0,
  "name": "LaBr3 Array",
  "num_channels": 32,
  "channel_names": {
    "0": "LaBr3-A",
    "1": "LaBr3-B"
  }
}
```

### Phase 3: Monitor — RegisterChannels handling

**File:** `src/monitor/mod.rs`

- `MonitorState` に `registered_channels: Vec<ChannelRegistration>` 追加
- `HistogramMessage::RegisterChannels(Vec<ChannelRegistration>)` variant 追加
- histogram_task:
  - `registered_channels` を上書き保存
  - 各チャンネルに空の `Histogram1D` を `entry().or_insert_with()` で生成
- `ListWaveforms`: registered_channels のキーも union で返す
- `GetListSummary`: registered_channels のヒストグラムを含める (total_counts: 0)
- REST レスポンス型に `name: Option<String>` フィールド追加
- `Clear` 時: データクリア + 空ヒストグラム再生成 (registered_channels 保持)
- `Reset` 時: registered_channels もクリア
- Command handler: `RegisterChannels` を histogram_tx に転送

### Phase 4: Operator — RegisterChannels 送信

**Files:** `src/operator/routes/status.rs`, `src/operator/routes/tuneup.rs`

ヘルパー関数:
```rust
fn build_channel_registrations(
    configs: &HashMap<u32, DigitizerConfig>,
    filter_id: Option<u32>,
) -> Vec<ChannelRegistration>
```

- 通常 Configure: `configure_all_sync` 成功後、全デジタイザのチャンネルを Monitor に送信
- Tune Up Start: configure 成功後、対象デジタイザのチャンネルのみ送信
- Monitor address は `state.components` から `!is_digitizer && name.contains("Monitor")` で特定

### Phase 5: Frontend — チャンネル名表示 + empty チャンネル対応

**Files:** `web/.../histogram.types.ts`, `web/.../waveform.component.ts`

- `WaveformChannelInfo`, `ChannelSummary` に `name?: string` 追加
- チャンネルセレクタに名前表示: `{{ ch.name ?? ('Src' + ch.module_id + '/Ch' + ch.channel_id) }}`
- `fetchChannelList()` を低頻度ポーリング追加 (5秒間隔)

---

## 修正ファイル一覧

| File | Changes |
|------|---------|
| `src/common/command.rs` | `ChannelRegistration` struct, `Command::RegisterChannels` |
| `src/config/mod.rs` | DigitizerConfig に `channel_names` 追加 |
| `src/monitor/mod.rs` | MonitorState, HistogramMessage, histogram_task, REST responses, command handler |
| `src/operator/routes/status.rs` | Configure 後 RegisterChannels 送信 |
| `src/operator/routes/tuneup.rs` | TuneUp start 後 RegisterChannels 送信 |
| `web/.../models/types.ts` | DigitizerConfig channel_names |
| `web/.../models/histogram.types.ts` | name フィールド |
| `web/.../pages/waveform/waveform.component.ts` | fetchChannelList 周期化 + 名前表示 |

---

## データフロー図

```
                    Configure / TuneUp Start
                           │
                           ▼
                    ┌──────────────┐
                    │   Operator   │
                    └──────┬───────┘
                           │ ZMQ: Command::RegisterChannels
                           │     [{module_id, channel_id, name}, ...]
                           ▼
                    ┌──────────────┐
                    │   Monitor    │
                    │              │
                    │  histogram_task:
                    │  - Store registered_channels
                    │  - Pre-create empty Histogram1D
                    │              │
                    │  REST APIs:  │
                    │  GET /api/histograms → 空チャンネルも含む
                    │  GET /api/waveforms  → 登録チャンネルも含む
                    └──────────────┘
                           │
                    ┌──────┴───────┐
                    │   Frontend   │
                    │              │
                    │  全チャンネル表示
                    │  (空 = 0 counts)
                    │  チャンネル名表示
                    └──────────────┘
```

---

## Verification

1. `cargo clippy -- -D warnings` + `cargo test`
2. `ng build`
3. 手動テスト:
   - Configure 後 Monitor ページで全チャンネル表示 (0 counts 含む)
   - Tune Up 開始 → 即座にチャンネル選択可能
   - チャンネル名が正しく表示 (custom name or default "{name}/Ch{n}")
   - Clear 後もチャンネルリストが維持される
   - Reset 後はチャンネルリストもクリア
