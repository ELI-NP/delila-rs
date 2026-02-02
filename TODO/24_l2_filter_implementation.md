# L2 Filter 実装計画

**Created:** 2026-02-02
**Status:** 計画中
**Design Doc:** (この文書で設計も兼ねる)

---

## 概要

Event Builder の L2 (Level 2) フィルタを実装する。
L1 (Event Building) で構築されたイベントに対してフィルタリングを行い、
解析に必要なイベントのみを選別する。

**設計方針:**
- **Time Slice 方式 (SliceBuilder) を標準採用** — オフライン・オンライン両方で使用
- Moving Time Window 方式 (L1Builder) は **deprecated** (シングルスレッドでスケールしない)
- ELIFANT-Event 互換の JSON 設定フォーマット
- KISS 原則: 最小限の構造で柔軟なフィルタリング

**スレッド設定:**
- rayon デフォルト: CPU コア数を自動検出
- 環境変数: `RAYON_NUM_THREADS=N` で制限可能
- 将来: 設定ファイルで `num_threads` 指定可能に

---

## ELIFANT-Event L2Settings.json 形式

```json
[
    {
        "Type": "Counter",
        "Name": "E_Sector_Counter",
        "Tags": ["E_Sector"]
    },
    {
        "Type": "Flag",
        "Name": "dE_More_Than_0",
        "Monitor": "dE_Sector_Counter",
        "Operator": ">",
        "Value": 0
    },
    {
        "Type": "Accept",
        "Name": "Si_Both",
        "Monitor": ["E_More_Than_0", "dE_More_Than_0"],
        "Operator": "AND"
    }
]
```

**3段階フィルタリング:**
1. **Counter**: タグまたはディテクタタイプでヒット数をカウント
2. **Flag**: カウンタに対する条件式 (>, <, ==, >=, <=, !=)
3. **Accept**: 複数フラグの論理演算 (AND, OR, NOT)

---

## 既存コードとの統合

### ChSettings (config.rs)

```rust
pub struct ChSettings {
    pub detector_type: String,  // "HPGe", "Si", "AC", "PMT", etc.
    pub tags: Vec<String>,      // ユーザー定義タグ ["E_Sector", "dE_Sector"]
    // ...
}
```

- `detector_type` と `tags` を L2 フィルタで活用
- `tags` はカウンタの `Tags` フィールドに対応

### BuiltEvent (built_event.rs)

```rust
pub struct BuiltEvent {
    pub event_id: u64,
    pub trigger_time: f64,
    pub trigger_module: u8,
    pub trigger_channel: u8,
    pub hits: Vec<EventHit>,
}
```

- L2 フィルタは `BuiltEvent` を入力として受け取る
- フィルタ結果は `bool` (accept/reject)

---

## 実装設計

### 新規ファイル

| ファイル | 目的 |
|---------|------|
| `src/event_builder/l2_filter.rs` | L2Filter 構造体、フィルタリングロジック |
| `src/event_builder/l2_config.rs` | L2Settings JSON パーサー |

### データ構造

```rust
/// L2 フィルタ設定要素
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "Type")]
pub enum L2Element {
    /// ヒット数カウンタ
    Counter {
        #[serde(rename = "Name")]
        name: String,
        #[serde(rename = "Tags")]
        tags: Vec<String>,
    },
    /// 条件フラグ
    Flag {
        #[serde(rename = "Name")]
        name: String,
        #[serde(rename = "Monitor")]
        monitor: String,
        #[serde(rename = "Operator")]
        operator: String,
        #[serde(rename = "Value")]
        value: i32,
    },
    /// 受理条件
    Accept {
        #[serde(rename = "Name")]
        name: String,
        #[serde(rename = "Monitor")]
        monitor: Vec<String>,
        #[serde(rename = "Operator")]
        operator: String,
    },
}

/// L2 フィルタ評価結果
pub struct L2Result {
    /// Accept 判定結果
    pub accepted: bool,
    /// Counter 値 (Multiplicity)
    pub multiplicities: HashMap<String, u32>,
    /// Flag 値
    pub flags: HashMap<String, bool>,
}

/// L2 フィルタ
pub struct L2Filter {
    /// チャンネル設定マップ
    channel_map: HashMap<(u8, u8), ChSettings>,
    /// フィルタ要素 (評価順)
    elements: Vec<L2Element>,
    /// カウンタ名 -> インデックス
    counter_indices: HashMap<String, usize>,
    /// フラグ名 -> インデックス
    flag_indices: HashMap<String, usize>,
    /// Accept 条件名
    accept_name: Option<String>,
}

impl L2Filter {
    /// イベントを評価 (Multiplicity + Flag + Accept)
    pub fn evaluate(&self, event: &BuiltEvent) -> L2Result;

    /// Accept 判定のみ
    pub fn accept(&self, event: &BuiltEvent) -> bool;

    /// Multiplicity のみ計算
    pub fn compute_multiplicities(&self, event: &BuiltEvent) -> HashMap<String, u32>;

    /// バッチフィルタリング (並列処理可能)
    pub fn filter_events(&self, events: Vec<BuiltEvent>) -> Vec<(BuiltEvent, L2Result)>;
}
```

### 演算子

| 演算子 | 説明 |
|--------|------|
| `>` | より大きい |
| `<` | より小さい |
| `>=` | 以上 |
| `<=` | 以下 |
| `==` | 等しい |
| `!=` | 等しくない |
| `AND` | 論理積 |
| `OR` | 論理和 |
| `NOT` | 論理否定 (単一モニター) |

---

## 実装フェーズ

### Phase 1: L2 Config パーサー (0.5日)

**ファイル:** `src/event_builder/l2_config.rs`

**タスク:**
- [ ] L2Element enum (Counter, Flag, Accept)
- [ ] L2Settings JSON パーサー
- [ ] バリデーション (名前重複チェック、参照整合性)
- [ ] 単体テスト (5 tests)

### Phase 2: L2Filter 構造体 (1日)

**ファイル:** `src/event_builder/l2_filter.rs`

**タスク:**
- [ ] L2Filter 構造体
- [ ] Counter 評価 (タグマッチング)
- [ ] Flag 評価 (比較演算)
- [ ] Accept 評価 (論理演算)
- [ ] `accept()` メソッド
- [ ] 単体テスト (10 tests)

### Phase 3: 統合 (0.5日)

**タスク:**
- [ ] mod.rs に L2Filter を export
- [ ] CLI (`event_builder build`) に `--l2-config` オプション追加
- [ ] SliceBuilder との統合テスト
- [ ] `--num-threads N` オプション追加 (rayon スレッド数指定)

### Phase 4: オンラインモード対応 (将来)

**OnlineSliceBuffer:**
- ストリーミング入力対応
- スライス収集 → 並列処理 → 出力の非同期パイプライン
- bounded channel でバックプレッシャー制御

---

## CLI 設計

```bash
# L2 フィルタ適用
event_builder build \
    -i input.root \
    -o events.root \
    --trigger 0:0 \
    --l2-config l2Settings.json \
    --num-threads 8              # オプション: rayon スレッド数

# スレッド数を環境変数で指定
RAYON_NUM_THREADS=4 event_builder build \
    -i input.root \
    -o events.root \
    --trigger 0:0
```

## ROOT 出力形式

L2 フィルタ適用時、Counter で定義した Multiplicity がブランチとして自動生成される。

### 出力 TTree 構造

```
events TTree:
  - event_id: u64
  - trigger_time: f64
  - trigger_module: u8
  - trigger_channel: u8
  - nhits: u32             // 全ヒット数
  - hits: Vec<EventHit>

  // L2 Counter から自動生成
  - HPGe_mult: u32         // Counter "HPGe_Count" の値
  - Si_E_mult: u32         // Counter "Si_E_Count" の値
  - Si_dE_mult: u32        // Counter "Si_dE_Count" の値
  ...
```

### 設計方針

- **BuiltEvent は変更しない** — L1 出力はシンプルに保つ
- **Multiplicity は L2Filter が計算** — Counter 設定から動的に決定
- **ROOT 出力時に付加** — `write_events_with_l2()` 関数を追加

### ~~compare サブコマンド~~ (削除)

~~Time Slice 方式と Moving Time Window 方式の結果を比較する。~~

**理由:** L1Builder (Moving Time Window) は deprecated。Time Slice のみを使用する。

---

## 使用例

### l2Settings.json

```json
[
    {
        "Type": "Counter",
        "Name": "HPGe_Count",
        "Tags": ["HPGe"]
    },
    {
        "Type": "Counter",
        "Name": "Si_Count",
        "Tags": ["Si_E", "Si_dE"]
    },
    {
        "Type": "Flag",
        "Name": "Has_HPGe",
        "Monitor": "HPGe_Count",
        "Operator": ">",
        "Value": 0
    },
    {
        "Type": "Flag",
        "Name": "Has_Si",
        "Monitor": "Si_Count",
        "Operator": ">=",
        "Value": 2
    },
    {
        "Type": "Accept",
        "Name": "Gamma_Si_Coincidence",
        "Monitor": ["Has_HPGe", "Has_Si"],
        "Operator": "AND"
    }
]
```

### chSettings.json (タグ設定)

```json
[
    [
        {
            "ID": 0,
            "Module": 0,
            "Channel": 0,
            "DetectorType": "HPGe",
            "Tags": ["HPGe", "Clover1"],
            ...
        },
        {
            "ID": 1,
            "Module": 0,
            "Channel": 1,
            "DetectorType": "Si",
            "Tags": ["Si_E", "Telescope1"],
            ...
        }
    ]
]
```

---

## テスト計画

### 単体テスト

1. **Counter テスト**
   - 単一タグマッチング
   - 複数タグマッチング (OR)
   - タグなしヒット

2. **Flag テスト**
   - 各演算子 (>, <, ==, >=, <=, !=)
   - 境界値テスト

3. **Accept テスト**
   - AND 演算
   - OR 演算
   - 単一フラグ

4. **統合テスト**
   - 複雑なフィルタチェーン
   - 空のイベント
   - ゼロヒットイベント

### パフォーマンステスト

- スレッド数による処理時間変化を計測
- 大規模データでのスケーラビリティ検証

---

## 依存関係

```toml
# 追加なし (既存の依存関係で対応可能)
# serde, serde_json, rayon は既存
```

---

## 参照

- ELIFANT-Event L2Settings: `legacy/ELIFANT-Event/JSON/L2Settings.json`
- ChSettings 定義: `src/event_builder/config.rs`
- Event Builder 実装: `TODO/23_event_builder_implementation.md`
