# AMax ファームウェア トリガー修正ガイド

## 概要

TrapezoidalfilterMCA ファームウェアでは、MCA HLS の内部トリガー出力が CAEN LIST のトリガー入力に接続されていないため、自律的なセルフトリガーが動作しません。本ドキュメントでは、この問題を解決するためのファームウェア修正手順を説明します。

---

## 問題の詳細

### 現在の信号フロー

```
信号源                    処理                      出力先
========                  ====                      ======

[IN PIN GBL_TRIG] ------> [input_mux] -----------> [CAEN LIST-0.TRIGGER]
     |                                                    |
     |                                                    v
     |                                              イベント取得
     |
     +-- 外部トリガー(TRGIN)やITLからの信号


[MCA HLS.TRIGGER] ------> [page_shaping_1.trgg]
     |
     +-----------------> [page_normalize.trigg]
     |
     +-----------------> [Oscilloscope.START]
     |
     X-- CAEN LIST には接続されていない！
```

### 問題点

| 項目 | 状態 |
|------|------|
| MCA HLS 内部トリガー生成 | 正常動作（THRS超過で発火） |
| 内部トリガー → 波形取得 | 接続済み |
| 内部トリガー → スペクトル | 接続済み |
| **内部トリガー → CAEN LIST** | **未接続（問題）** |

---

## 修正方法

### 修正後の信号フロー

```
信号源                    処理                      出力先
========                  ====                      ======

[IN PIN GBL_TRIG] ------> [input_mux] ----+
                                          |
                                          v
                                     +--------+
                                     |   OR   | -----> [CAEN LIST-0.TRIGGER]
                                     |  Gate  |              |
                                     +--------+              v
                                          ^            イベント取得
                                          |
[MCA HLS.TRIGGER] -----------------------+
     |
     +-----------------> [page_shaping_1.trgg]    (既存接続維持)
     |
     +-----------------> [page_normalize.trigg]   (既存接続維持)
     |
     +-----------------> [Oscilloscope.START]     (既存接続維持)
```

---

## SciCompiler での修正手順

### Step 1: ORゲートの追加

1. ツールパレットを開く
2. **Logic** または **Basic** カテゴリを選択
3. **OR** ゲートをドラッグしてダイアグラムに配置
4. 配置位置: `GBL_TRIG の input_mux` と `CAEN LIST - 0` の間

**重要**: ビット幅を **1ビット** に設定（信号は `[1]` です）

### Step 2: 既存接続の切断

1. 以下の配線を選択して削除:

```
削除する接続:
[input_mux (GBL_TRIG出力)] ----X----> [CAEN LIST - 0.TRIGGER]
```

2. 配線をクリックして選択
3. Delete キーで削除

### Step 3: 新しい接続の作成

以下の3本の配線を新規作成:

| # | 接続元 | 接続先 | 説明 |
|---|--------|--------|------|
| 1 | `input_mux` (GBL_TRIG出力) | OR Gate 入力A | 外部トリガーパス |
| 2 | `MCA HLS.TRIGGER` (U54) | OR Gate 入力B | 内部トリガーパス **(新規)** |
| 3 | OR Gate 出力 | `CAEN LIST - 0.TRIGGER` | 結合トリガー |

### Step 4: 接続の確認

#### 維持すべき既存接続

以下の接続は **削除しないでください**:

- `MCA HLS.TRIGGER` → `page_shaping_1.trgg`
- `MCA HLS.TRIGGER` → `page_normalize_fara_trigg_0.trigg`
- `MCA HLS.TRIGGER` → 各 Oscilloscope の START 入力

#### 新規接続の確認

- OR Gate の両入力に信号が接続されていること
- OR Gate の出力が `CAEN LIST - 0.TRIGGER` に接続されていること

---

## ダイアグラム上の位置関係

### 修正前

```
+------------------+                              +------------------+
|   IN PIN         |                              |   CAEN LIST - 0  |
|   GBL_TRIG       |----[input_mux]-------------->|   TRIGGER        |
+------------------+                              +------------------+


+------------------+
|    MCA HLS       |
|    (U54)         |
|                  |
|    TRIGGER ------+---> [page_shaping_1.trgg]
|                  |
|                  +---> [page_normalize.trigg]
|                  |
|                  +---> [Oscilloscope.START]
+------------------+
```

### 修正後

```
+------------------+                              +------------------+
|   IN PIN         |                              |   CAEN LIST - 0  |
|   GBL_TRIG       |----[input_mux]----+          |                  |
+------------------+                   |          |                  |
                                       v          |                  |
                                   +------+       |                  |
                                   |  OR  |------>|   TRIGGER        |
                                   +------+       |                  |
                                       ^          +------------------+
                                       |
+------------------+                   |
|    MCA HLS       |                   |
|    (U54)         |                   |
|                  |                   |
|    TRIGGER ------+-------------------+
|                  |
|                  +---> [page_shaping_1.trgg]    (維持)
|                  |
|                  +---> [page_normalize.trigg]   (維持)
|                  |
|                  +---> [Oscilloscope.START]     (維持)
+------------------+
```

---

## ビルドと検証

### ビルド前チェックリスト

- [ ] OR ゲートのビット幅が 1 ビットである
- [ ] `input_mux` → OR Gate 入力A が接続されている
- [ ] `MCA HLS.TRIGGER` → OR Gate 入力B が接続されている
- [ ] OR Gate 出力 → `CAEN LIST - 0.TRIGGER` が接続されている
- [ ] 既存の `MCA HLS.TRIGGER` への接続が維持されている
- [ ] タイミング制約に問題がない

### 動作検証

修正後、以下のすべてが動作することを確認:

| テスト | 方法 | 期待結果 |
|--------|------|----------|
| 外部トリガー | `AcqTriggerSource = TRGIN` + 外部信号入力 | イベント取得 |
| ソフトウェアトリガー | `AcqTriggerSource = SwTrg` + sendswtrigger | イベント取得 |
| **内部セルフトリガー** | 信号入力のみ（トリガー不要） | **イベント自動取得** |

### テストコマンド (Rust)

```rust
// 修正後のファームウェアでの動作確認
// AcqTriggerSource は何でも良い（内部トリガーが OR されるため）

// 1. チャンネル有効化
handle.set_value("/ch/0/par/chenable", "True")?;

// 2. 適切なしきい値設定
handle.set_user_register(0x8, 100)?;  // THRS = 100

// 3. 取得開始
handle.send_command("/cmd/armacquisition")?;
handle.send_command("/cmd/swstartacquisition")?;

// 4. 信号入力 → 自動でイベント取得されるはず
// sendswtrigger は不要！
```

---

## 代替案: MUX による選択式

より柔軟な制御が必要な場合、OR ゲートの代わりに MUX を使用:

```
+------------------+
|   IN PIN         |
|   GBL_TRIG       |----[input_mux]----+
+------------------+                   |    +---------------+
                                       +--> |               |
                                       0    |     MUX       |---> [CAEN LIST.TRIGGER]
+------------------+                   +--> |               |
|    MCA HLS       |                   1    +---------------+
|    TRIGGER       |-------------------+          ^
+------------------+                              |
                                                  |
                                    +-------------------+
                                    | REG_TRIGGER_SEL   |
                                    | (新規レジスタ)     |
                                    | 0 = 外部          |
                                    | 1 = 内部          |
                                    +-------------------+
```

**注意**: MUX 方式は排他的選択となるため、OR 方式を推奨します。

---

## 関連ファイル

| ファイル | 説明 |
|----------|------|
| `TrapezoidalfilterMCA_dpp_tests/Diagram.pdf` | 現在のダイアグラム |
| `TrapezoidalfilterMCA_dpp_tests/HDL/user_dpp.vhd` | 生成されるVHDL |
| `TrapezoidalfilterMCA_dpp_tests/library/RegisterFile.json` | レジスタマップ |

---

## 変更履歴

| 日付 | 変更内容 |
|------|----------|
| 2026-01-29 | 初版作成 |
