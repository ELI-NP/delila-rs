# デジタイザ設定 UI (Phase 6)

**Created:** 2026-01-26
**Status:** VERIFICATION IN PROGRESS (2026-02-03)
**Priority:** High — MVP Post-MVP 機能

---

## 目的

Web UI からデジタイザの設定を閲覧・編集できるようにする。
PSD2 と PSD1 の両方に対応。チャンネル数はハードウェアから自動検出。

---

## デジタイザ自動検出 + DB設定復元

### フロー

```
[Settings UI] ─── "Detect" ボタン押下
    │
    │  POST /api/digitizers/detect
    ▼
[Operator] ─── Reader に Detect コマンド送信
    │
    │  ZMQ CMD (Detect)
    ▼
[Reader] ─── FELib 接続 → get_device_info() → 切断
    │
    │  ZMQ REP (DeviceInfo)
    ▼
[Operator]
    │
    ▼
MongoDB 検索: serial_number で過去の設定を検索
    ├─ Found  → 保存済み設定をロード（num_channels も DB の値を使用）
    └─ Not Found → デフォルト設定を新規作成
                   num_channels はハードウェアから取得
    │
    ▼
UI に表示 → 編集 → Apply → Configure で適用
```

**重要:**
- Operator はデジタイザに直接接続しない。Reader プロセス経由で通信する。
- **Detect は Configure とは独立したステップ。** Settings UI の "Detect" ボタンで Reader がハードウェアに一時接続して DeviceInfo を取得し、切断する。
- Configure は Detect で取得した情報と編集済み設定を使って実行する。
- **Emulator ソースは Settings UI に表示しない。** デジタイザ設定は実機のみ。

### MongoDB スキーマ変更

```javascript
// digitizer_configs に追加するフィールド
{
  serial_number: "52622",    // NEW: ハードウェアのシリアル番号
  model: "VX2730",           // NEW: モデル名
  // ... 既存フィールド ...
}
```

**検索:** `serial_number` でユニーク検索。同一デジタイザの設定を自動復元。

### API 追加

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/digitizers/by-serial/:serial` | シリアル番号で設定検索 |
| POST | `/api/digitizers/detect` | Reader 経由でハードウェアに一時接続し DeviceInfo を取得。DB に serial_number で検索し、過去の設定があればロード、なければデフォルト新規作成。 |

---

## UI設計

### レイアウト概要

```
┌─────────────────────────────────────────────────────────┐
│ Digitizer: [VX2730-001 (PSD2) ▼]          [Reset] [Apply] │
├─────────────────────────────────────────────────────────┤
│  [ Board ]  [ Frequent ]  [ Advanced ]                    │
├─────────────────────────────────────────────────────────┤
│                                                           │
│              ← 選択中タブのコンテンツ →                    │
│                                                           │
└─────────────────────────────────────────────────────────┘
```

### 3タブ構成

| Tab | 内容 | 表示形式 |
|-----|------|----------|
| **Board** | ボード全体設定 | フォームカード |
| **Frequent** | よく使うチャンネルパラメータ | チャンネルテーブル (横=ch, 縦=param) |
| **Advanced** | あまり使わないチャンネルパラメータ | チャンネルテーブル (横=ch, 縦=param) |

### Tab 1: Board Settings

フォーム形式（現行UIベース）:

```
┌─────────────────────────────────────────────────────┐
│ Board Settings                                       │
│                                                      │
│   Start Source:         [SWcmd          ▼]           │
│   Global Trigger:       [ITLA           ▼]           │
│   Test Pulse Period:    [100000         ] ns          │
│   Test Pulse Width:     [1000           ] ns          │
│   Record Length:        [1024           ] ns          │
│   Waveforms:            [ON  ▼]                      │
│                                                      │
│ ── Waveform Probes (Waveforms=ON のとき表示) ──────  │
│   Analog Probe 1:       [ADCInput       ▼]           │
│   Analog Probe 2:       [CFDOutput      ▼]           │
│   Digital Probe 1:      [LongGate       ▼]           │
│   Digital Probe 2:      [ShortGate      ▼]           │
│                                                      │
│ ── PSD1 only ──────────────────────────────          │
│   Start Mode:           [START_MODE_SW  ▼]           │
│   Extras:               [TRUE           ▼]           │
└─────────────────────────────────────────────────────┘
```

- FW依存パラメータは該当FWのときだけ表示
- Waveform Probes セクションは **Waveforms=ON のときのみ表示**
- Probe 設定はボード単位（全チャンネル共通）で Board タブに配置

### Tab 2: Frequent (よく使うチャンネルパラメータ)

```
┌─────────────────────────────────────────────────────────────────────────┐
│  Frequent Channel Parameters                             ← scroll →    │
│ ┌────────────┬───────┬───────┬───────┬───────┬───────┬───────┬─────┐  │
│ │            │  All  │ Ch 0  │ Ch 1  │ Ch 2  │ Ch 3  │ Ch 4  │ ... │  │
│ ├────────────┼───────┼───────┼───────┼───────┼───────┼───────┼─────┤  │
│ │ Enable     │  [✓]  │  [ ]  │  [ ]  │  [ ]  │  [ ]  │  [✓]  │     │  │
│ │ DC Offset %│  50   │  50   │  50   │  50   │  50   │  50   │     │  │
│ │ Polarity   │ [Neg▼]│ [Neg▼]│ [Neg▼]│ [Neg▼]│ [Neg▼]│ [Neg▼]│     │  │
│ │ Threshold  │  1000 │  1000 │  1000 │  1000 │  1000 │  1000 │     │  │
│ │ Gate Long  │  400  │  400  │  400  │  400  │  400  │  400  │     │  │
│ │ Gate Short │  100  │  100  │  100  │  100  │  100  │  100  │     │  │
│ │ Evt Trig   │ [Glb▼]│ [Glb▼]│ [Glb▼]│ [Glb▼]│ [Glb▼]│ [Self▼│     │  │
│ └────────────┴───────┴───────┴───────┴───────┴───────┴───────┴─────┘  │
│                                                                        │
│  All 列: 値を変更すると全チャンネルに一括反映                          │
│  Ch 列: 個別に変更可能。All と異なる値はハイライト表示                 │
└─────────────────────────────────────────────────────────────────────────┘
```

- **All 列**: 値を変更すると全チャンネルに一括適用（`channel_defaults` に対応）
- **Ch 列**: 各チャンネル個別に編集可能。All と異なる値はハイライト表示
- 横スクロールで全チャンネル表示（チャンネル数はハードウェアから自動検出）
- パラメータ名列と All 列は sticky 固定（スクロールしても左端に残る）

### Tab 3: Advanced (あまり使わないチャンネルパラメータ)

同じテーブル形式。パラメータが異なるだけ。

---

## パラメータ分類

### PSD2 (VX2730)

**Board:**
| Parameter | DevTree Path | Type | setinrun | 条件 |
|-----------|-------------|------|----------|------|
| Start Source | `/par/StartSource` | ENUM | No | |
| Global Trigger Source | `/par/GlobalTriggerSource` | ENUM | No | |
| Test Pulse Period | `/par/TestPulsePeriod` | NUMBER | Yes | |
| Test Pulse Width | `/par/TestPulseWidth` | NUMBER | Yes | |
| Record Length | `/par/ChRecordLengthT` | NUMBER | No | |
| Waveforms | (board config) | BOOL | No | |
| Analog Probe 1 | `/ch/0/par/WaveAnalogProbe0` | ENUM | Yes | Waveforms=ON |
| Analog Probe 2 | `/ch/0/par/WaveAnalogProbe1` | ENUM | Yes | Waveforms=ON |
| Digital Probe 1 | `/ch/0/par/WaveDigitalProbe0` | ENUM | Yes | Waveforms=ON |
| Digital Probe 2 | `/ch/0/par/WaveDigitalProbe1` | ENUM | Yes | Waveforms=ON |

※ Probe 設定は DevTree 上はチャンネル単位だが、実運用では全チャンネル共通に設定する。
Board タブで設定し、適用時に全チャンネルへ書き込む。

**Frequent Channel:**
| Parameter | DevTree Path | Type | setinrun |
|-----------|-------------|------|----------|
| Enable | `/ch/{n}/par/ChEnable` | ENUM | No |
| DC Offset | `/ch/{n}/par/DCOffset` | NUMBER (%) | Yes |
| Polarity | `/ch/{n}/par/PulsePolarity` | ENUM | No |
| Trigger Threshold | `/ch/{n}/par/TriggerThr` | NUMBER | Yes |
| Gate Long | `/ch/{n}/par/GateLongLengthT` | NUMBER (ns) | No |
| Gate Short | `/ch/{n}/par/GateShortLengthT` | NUMBER (ns) | No |
| Event Trigger Source | `/ch/{n}/par/EventTriggerSource` | ENUM | No |

**Advanced Channel:**
| Parameter | DevTree Path | Type | setinrun |
|-----------|-------------|------|----------|
| Wave Trigger Source | `/ch/{n}/par/WaveTriggerSource` | ENUM | No |
| CFD Delay | `/ch/{n}/par/CFDDelayT` | NUMBER (ns) | No |
| CFD Fraction | `/ch/{n}/par/CFDFraction` | ENUM | No |
| Smoothing Factor | `/ch/{n}/par/SmoothingFactor` | ENUM | No |
| Pre-Trigger | `/ch/{n}/par/PreTriggerT` | NUMBER (ns) | No |

### PSD1 (DT5730B / x725 / x730)

**Board:**
| Parameter | DevTree Path | Type |
|-----------|-------------|------|
| Record Length | `/par/reclen` | NUMBER |
| Start Mode | `/par/startmode` | ENUM |
| Extras | `/par/extras` | ENUM |
| Waveforms | `/par/waveforms` | ENUM |
| SW Trigger Enable | `/par/trg_sw_enable` | ENUM |
| Ext Trigger Enable | `/par/trg_ext_enable` | ENUM |

**Frequent Channel:**
| Parameter | DevTree Path | Type |
|-----------|-------------|------|
| Enable | `/ch/{n}/par/ch_enabled` | ENUM |
| DC Offset | `/ch/{n}/par/ch_dcoffset` | NUMBER (%) |
| Polarity | `/ch/{n}/par/ch_polarity` | ENUM |
| Threshold | `/ch/{n}/par/ch_threshold` | NUMBER |
| Gate Long | `/ch/{n}/par/ch_gate` | NUMBER (samples) |
| Gate Short | `/ch/{n}/par/ch_gateshort` | NUMBER (samples) |
| Gate Pre | `/ch/{n}/par/ch_gatepre` | NUMBER (samples) |
| Self Trigger | `/ch/{n}/par/ch_self_trg_enable` | ENUM |

**Advanced Channel:**
| Parameter | DevTree Path | Type |
|-----------|-------------|------|
| CFD Delay | `/ch/{n}/par/ch_cfd_delay` | NUMBER |
| CFD Fraction | `/ch/{n}/par/ch_cfd_fraction` | NUMBER |
| CFD Smoothing | `/ch/{n}/par/ch_cfd_smoothexp` | ENUM |
| Trigger Mode | `/ch/{n}/par/ch_trg_mode` | ENUM |
| Trigger Holdoff | `/ch/{n}/par/ch_trg_holdoff` | NUMBER |
| Baseline Mean | `/ch/{n}/par/ch_bline_nsmean` | ENUM |
| Baseline Fixed | `/ch/{n}/par/ch_bline_fixed` | NUMBER |
| Pre-Trigger | `/ch/{n}/par/ch_pretrg` | NUMBER |
| Pile-Up Rejection | `/ch/{n}/par/ch_pur_en` | ENUM |
| Pile-Up Gap | `/ch/{n}/par/ch_purgap` | NUMBER |

---

## データフロー

```
[Angular UI]
     │
     ├── "Detect" ─── POST /api/digitizers/detect
     │                     │
     │                     ▼
     │               [Operator] ── ZMQ Detect ──► [Reader] ── FELib ──► [Digitizer]
     │                     │                           │
     │                     │◄── DeviceInfo ────────────┘
     │                     │
     │                     ▼
     │               MongoDB: serial_number で検索 or 新規作成
     │                     │
     │◄── 200 OK ──────────┘  (DeviceInfo + DigitizerConfig)
     │
     ├── Settings 表示/編集 ─── GET /api/digitizers/:id
     │
     ├── Apply ─── PUT /api/digitizers/:id
     │                  │
     │                  ▼
     │            [Operator]
     │                  │
     │                  ├─ Not Running: MongoDB に保存のみ
     │                  │
     │                  └─ Running: MongoDB に保存 + setinrun=true パラメータを
     │                              Reader 経由でハードウェアに適用
     │
     └── Configure ─── POST /api/configure
                            (保存済み設定をハードウェアに適用)
```

**注意:** Emulator ソースは Settings UI に表示しない。デジタイザ設定は実機のみ。

### チャンネルデータ変換 (UI ↔ Config)

```
UI Table:           Config JSON:
Ch 0: thresh=100    channel_defaults.trigger_threshold = 1000
Ch 1: thresh=100    channel_overrides:
Ch 2: thresh=100      "4": { trigger_threshold: 500 }
Ch 3: thresh=100
Ch 4: thresh=500    ← override
Ch 5: thresh=100
...
```

UI ではすべてのチャンネルの値をフラットに表示する。
保存時にデフォルト値と比較して override を自動生成する。

---

## 実装計画

### Step 1: Rust — Reader に Detect コマンド追加

**変更:** `src/reader/mod.rs` (または `src/reader/caen/`)

- Reader の ZMQ CMD ハンドラに `Detect` コマンド追加
- Detect: FELib 接続 → `get_device_info()` → DeviceInfo を返す → 切断
- Configure とは独立。ステートマシンに影響しない。

### Step 2: Rust — MongoDB スキーマ + シリアル番号検索

**変更:** `src/config/digitizer.rs`, `src/operator/digitizer_repository.rs`

- `DigitizerConfig` に `serial_number: Option<String>`, `model: Option<String>` 追加
- `DigitizerConfigDocument` に `serial_number`, `model` フィールド追加
- `get_config_by_serial(serial: &str)` クエリ追加
- Detect 応答時: serial で DB 検索 → 設定ロード or デフォルト作成

### Step 3: Rust — REST API 拡張

**変更:** `src/operator/routes.rs`

- `POST /api/digitizers/detect` — Reader 経由で DeviceInfo 取得 + DB 設定ロード/新規作成
- `GET /api/digitizers/by-serial/:serial` — シリアル番号で設定検索
- `GET /api/digitizers/:id` のレスポンスに `serial_number`, `model`, `num_channels` 追加
- 既存 API (`PUT`, `POST /save`) は変更なし

### Step 4: Angular — チャンネルテーブルコンポーネント

**新規作成:** `channel-table.component.ts`

- 再利用可能なテーブルコンポーネント
- Input: パラメータ定義リスト + チャンネル数 + 値マップ
- Output: 値変更イベント
- 横スクロール対応（パラメータ名列 + All 列は sticky 固定）
- セル型: number input, dropdown (ENUM), checkbox (boolean)
- デフォルトと異なる値のハイライト

### Step 5: Angular — digitizer-settings を3タブに刷新

**変更:** `digitizer-settings.component.ts`

- 既存の defaults/overrides カード → 3タブ構成に置き換え
- Tab 1: Board Settings (現行UIベースのフォーム + Waveform Probes)
- Tab 2: Frequent → channel-table 使用
- Tab 3: Advanced → channel-table 使用
- FW種別 (PSD1/PSD2) に応じてパラメータリストを切り替え
- チャンネル数はAPIから取得（ハードウェア検出値）
- Emulator ソースは表示しない（デジタイザのみ）
- "Detect" ボタンでハードウェア検出を実行

### Step 6: Angular — config 展開/圧縮ロジック

**変更:** `digitizer.service.ts`

- `expandConfig()`: defaults + overrides → 全チャンネル値のフラット配列
- `compressConfig()`: フラット配列 → defaults + overrides (差分のみ)

### Step 7: 結合テスト

- UI 表示・編集・保存の動作確認
- (実機がある場合) Detect ボタン → DeviceInfo 取得 → DB 設定復元の確認

---

## 変更ファイル一覧 (予定)

| Action | File | Description |
|--------|------|-------------|
| Modify | `src/reader/mod.rs` | Detect コマンド追加 (Reader 側) |
| Modify | `src/config/digitizer.rs` | serial_number, model フィールド追加 |
| Modify | `src/operator/digitizer_repository.rs` | serial 検索クエリ追加 |
| Modify | `src/operator/routes.rs` | detect / by-serial API 追加 |
| Create | `web/.../channel-table/channel-table.component.ts` | チャンネルテーブル汎用コンポーネント |
| Modify | `web/.../digitizer-settings/digitizer-settings.component.ts` | 3タブ化 + Detect ボタン |
| Modify | `web/.../digitizer-settings/digitizer-settings.component.html` | テンプレート更新 |
| Modify | `web/.../services/digitizer.service.ts` | expand/compress + detect API 追加 |

---

## 設計判断

1. **チャンネル数はハードウェアから自動検出** — `get_device_info().num_channels` を使用。決め打ちしない。
2. **シリアル番号で設定を自動復元** — 同一デジタイザなら過去の設定をDBから自動ロード。
3. **デフォルト+オーバーライドは内部保持のみ** — UIでは全チャンネルをフラット表示。保存時に自動圧縮。
4. **FW別パラメータ定義** — PSD1/PSD2のパラメータリストをTypeScriptで静的に定義。DevTree動的UIは将来。
5. **setinrun=true/false** — Running中はsetinrun=falseのセルをグレーアウト。
6. **横スクロール** — 全チャンネル表示。sticky列でパラメータ名を固定。
7. **Waveform Probes は条件付き表示** — Board の Waveforms=ON のときのみ Probe 設定セクションを表示。Probe は全チャンネル共通として Board タブに配置。
8. **Reader 経由の通信** — Operator はデジタイザに直接接続しない。全ての通信は Reader プロセス経由（ZMQ CMD/REP）。
9. **Detect は独立ステップ** — Settings UI の "Detect" ボタンで Reader がハードウェアに一時接続して DeviceInfo を取得し、切断する。Configure とは独立。
10. **Emulator は Settings UI に非表示** — デジタイザ設定UIは実機専用。Emulator ソースはリストに表示しない。

---

## 実機検証ログ (2026-02-03)

**ハードウェア:** VX2730 (Serial: 52621, DPP_PSD, 32ch, 14bit, 500Msps)
**アドレス:** dig2://172.18.4.57
**設定ファイル:** `config/config_psd2_test.toml` + `config/digitizers/psd2_test.json`

### 発見・修正した問題

#### 1. Config 読み込み ID 衝突
- **症状:** `GET /api/digitizers` で PSD1 設定が返る (PSD2 が表示されない)
- **原因:** `load_digitizer_configs()` が `config/digitizers/` 内の全 JSON を読み込み、psd1_test.json と psd2_test.json が同じ `digitizer_id: 0` で衝突
- **修正:** TOML source の `config_file` フィールドで指定されたファイルのみロードするよう変更
- **ファイル:** `src/operator/routes/mod.rs` (load_digitizer_configs_from_files), `src/bin/operator.rs` (config_files 抽出)

#### 2. Detect 後に config が UI ドロップダウンに反映されない
- **症状:** Detect 成功 (hardware 検出) しても設定 UI に何も表示されない
- **原因:** Detect は MongoDB のみ検索、in-memory config を更新しない。`config_found: false` で config が null 返却
- **修正:** Detect 後に (a) 既存 in-memory config を hardware info で更新、(b) 未設定時はファームウェアに基づくデフォルト自動生成、(c) `state.digitizer_configs` に挿入
- **ファイル:** `src/operator/routes/digitizer.rs` (detect_digitizers + firmware_from_device_info + default_board_config + default_channel_config)

#### 3. Detect 後の手動選択が不便
- **症状:** Detect でデジタイザ検出後、ドロップダウンから手動選択が必要
- **修正:** 検出された最初のデジタイザを自動選択
- **ファイル:** `web/operator-ui/src/app/components/digitizer-settings/digitizer-settings.component.ts`

#### 4. デフォルト config.toml の IP アドレス
- **症状:** `start_daq.sh` が `config.toml` (172.18.4.56, Multi_Digitizer_Test) をデフォルトで使用
- **対処:** `./scripts/start_daq.sh config/config_psd2_test.toml` で明示指定

### API 動作確認結果

| API | 結果 |
|-----|------|
| `GET /api/status` | ✅ PSD2_Test, 全コンポーネント Idle |
| `GET /api/digitizers` | ✅ PSD2 config のみ (ID衝突なし) |
| `POST /api/digitizers/detect` | ✅ VX2730 検出, serial_number/model 反映済み |
| `GET /api/digitizers` (detect 後) | ✅ serial_number: "52621", model: "VX2730" 追加 |

#### 5. デジタイザ表示名の編集機能 ✅ (2026-02-03)

- **背景:** JSON ファイルの `name` フィールドがデジタイザ選択ドロップダウンの表示名になる。現状では名前を変えるには JSON ファイルを直接編集して再起動が必要。
- **要件:** UI 上でデジタイザ名を編集し、Apply/Save で永続化できるようにする。将来 MongoDB に設定を格納する際にも UI 経由で変更できることが重要。
- **実装:** ドロップダウン右隣に Name 入力フィールドを追加。`[(ngModel)]="config.name"` で直接バインド。Apply/Save で JSON に永続化。バックエンドは変更不要（既存の PUT/Apply が name を処理済み）。
- **ファイル:** `web/operator-ui/src/app/components/digitizer-settings/digitizer-settings.component.ts` (テンプレート + CSS)

### 残タスク

- [ ] Angular UI タブ表示確認 (Board / Frequent / Advanced)
- [ ] Apply → PUT /api/digitizers/:id → Configure 動作確認
- [ ] Save → JSON ファイル書き出し確認
- [ ] Configure → Arm → Start でデータ取得確認
- [x] デジタイザ名編集フィールドの追加 (上記 §5)
