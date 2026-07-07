# AMax ファームウェア更新マニュアル

AMax カスタム FW を更新したとき、delila-rs（本体 DAQ ＋ Operator UI）を新しいレジスタマップに追従させる手順。

**対象**: AMax FW 開発者 / DAQ 運用者
**関連**: [amax_firmware_trigger_modification.md](amax_firmware_trigger_modification.md)（FW 側の改造）, `scripts/update_amax_fw.sh`, `src/bin/amax_codegen.rs`

---

## 1. 概要

AMax FW は再ビルドのたびにレジスタのアドレスマップが平行移動し、ときにレジスタが増減する。delila-rs はビルド時に `RegisterFile.json` からコードを生成（型付き struct / レジスタアドレス / Operator UI の TS）して取り込む方式。

**`scripts/update_amax_fw.sh` 一発**で、以下が自動で行われる：

```
RegisterFile.json
   │  amax_codegen（アドレス自動導出）
   ├─→ src/config/amax_generated.rs                     （AMaxChannelConfig struct）
   ├─→ src/reader/caen/amax_registers_generated.rs      （REG_*, BROADCAST_BASE, channel_writes, merge_amax_channel_config）
   └─→ web/operator-ui/src/app/models/amax-generated.ts （UI 型 + パラメータ定義 + カテゴリマップ）
        │  cargo fmt → cargo build → ng build
        └─→ web/operator-ui/dist/                        （配信される UI バンドル）
```

**人間が手で当てる値はゼロ**（page_base / stride / broadcast_base はすべて RegisterFile.json から自動導出）。`handle.rs` を編集する必要もない。

---

## 2. 前提

| 項目 | 必要なもの |
|---|---|
| Rust | `cargo`（`source ~/.cargo/env`） |
| Node（UI ビルド用） | `node` + `npm`（gant は導入済。新マシンは §8 参照） |
| 入力 | FW 開発者からの `RegisterFile.json` |
| UI メタ | `tools/amax_viewer/fw_params.json`（レジスタの label / category / 型 / bit幅 / default） |

> node が無いマシンでも Rust 側（DAQ バイナリ）は完結する。その場合 UI(dist) は別の node 機で焼く（§7 参照）。

---

## 3. クイックスタート（通常ケース＝アドレスが動いただけ）

```bash
# 1. RegisterFile.json をリポに保存（慣習: FW/<日付>/）
#    例: FW/20260701/RegisterFile_1july.json

# 2. 1コマンド
scripts/update_amax_fw.sh FW/20260701/RegisterFile_1july.json
```

実行中にサマリが出る：

```
amax_codegen layout: PAGE_BASE=0x... (auto), PAGE_STRIDE=0x... (auto), BROADCAST_BASE=0x... (auto), canonical=per-channel ch0
amax_codegen: register set unchanged (29 registers)
```

`register set unchanged` なら完了。あとは §6 でコミット＆デプロイ。

**オプション**:
- `--no-ui` … UI ビルドをスキップ（Rust だけ素早く回す）
- `--with-viewer` … amax_viewer の `register_defs.json` も同じ入力から再生成（要 CAEN libs・ベストエフォート）
- `--help` … 使い方表示

---

## 4. ケース別手順

### A. アドレスが動いただけ
→ §3 のまま。`register set unchanged` が出る。

### B. レジスタが増減した
サマリにこう出る：

```
amax_codegen: register set changed — 2 added, 0 removed
  + NEW_REG_A, NEW_REG_B
warning: 2 register(s) in RegisterFile.json have no entry in fw_params.json (skipped):
  - NEW_REG_A
  - NEW_REG_B
```

`no entry in fw_params.json (skipped)` ＝ **そのレジスタは型にも UI にも出ない**。出したいレジスタだけ [`tools/amax_viewer/fw_params.json`](../tools/amax_viewer/fw_params.json) の `params` に1エントリ足す：

```json
"NEW_REG_A": { "bits": 16, "default": 100, "label": "New Reg A", "category": "energy", "type": "number", "unit": "ns" }
```

| フィールド | 意味 |
|---|---|
| `bits` | データビット幅（`max = 2^bits - 1` を自動計算） |
| `default` | 初期値 |
| `label` | UI 表示名 |
| `category` | UI タブ（`input`/`trigger`/`energy`/`amax`、または**新しい名前でも可**→§4C） |
| `type` | `number` または `enum`（`enum` のとき `options: [...]` も） |
| `unit` | 任意。表示単位 |

足したら**もう一度同じコマンド**。警告が消え、型付き struct ＋ UI に出る。
レジスタを**削除**した場合は何もしなくてよい（codegen が struct とマージ関数を同時に縮める。`handle.rs` 編集不要）。

### C. 新しいカテゴリ（タブ）を増やす
`fw_params.json` の `category` に既存以外の名前（例 `debug`）を書くだけ。codegen が `AMAX_DEBUG_PARAMS` と `AMAX_PARAMS_BY_CATEGORY` を生成し、**Operator UI の Settings / Tune Up に新タブが自動で出る**（TS の手編集不要、タブ名は PascalCase で自動付与）。

---

## 5. 安全装置

- **アドレス書き間違いは黙って通らない**: broadcast ページと per-channel ページの in-page レイアウトが食い違うと codegen が `Err` で停止する（誤った AMax アドレス書き込みは FW を破壊しうるためのガード）。
- **フラグは緊急脱出口のみ**: 自動導出が想定外のときだけ `--page-base 0x... --page-stride 0x... --broadcast-base 0x... --prefer-per-channel false`（10進/0x16進どちらも可）。通常は不要。
- **生成回帰の確認**: 既知良好な FW で `git diff src/reader/caen/amax_registers_generated.rs` を見て、意図しないアドレス変化が無いか目視できる。

---

## 6. コミット & gant へのデプロイ

UI を更新したら **生成物と `dist/` を同一コミットに**含める（Frontend Deployment Policy）。

```bash
git add src/config/amax_generated.rs \
        src/reader/caen/amax_registers_generated.rs \
        web/operator-ui/src/app/models/amax-generated.ts \
        web/operator-ui/dist/
git commit -m "amax: regen FW bindings from RegisterFile_1july"
git push origin master
```

gant（AMax dev 機, 172.18.6.114）へ：

```bash
ssh gant@172.18.6.114
cd /media/raid1/delila-rs
git pull --ff-only origin master
source ~/.cargo/env
cargo build --release --bins
```

> gant 上で直接 `scripts/update_amax_fw.sh` を回してもよい（node 導入済なので UI まで完結）。その場合は上の commit/push を gant 側で行う。

新バイナリを反映するには **DAQ スタックの再起動が必要**（稼働ラン中は中断するので状態を確認してから）：

```bash
# Configured/Idle（稼働ラン無し）であることを確認してから
scripts/stop_daq.sh
scripts/start_daq.sh config/config_amax_56_2Digitizer.toml
```

> **MongoDB について:** operator は接続情報を config の `[operator.mongodb]` から読む（`--mongodb-uri` 等の CLI フラグは上書き用の任意）。よって `--no-mongo` の有無に関わらずラン履歴は保存される。`--no-mongo` を**付けない**と、スクリプトが追加で永続稼働中の `delila_mongo` コンテナを ping で確認する（再起動はしない＝冪等）ぶん少し堅牢なので、付けないのを推奨。`--no-mongo` は start_daq.sh の mongo コンテナ確認と CLI フラグ付与をスキップするだけで、**operator は TOML フォールバックで接続する**（[operator.rs:319](../src/bin/operator.rs#L319) の解決ロジック）。

再起動後 `http://localhost:9090/api/status` で全コンポーネント Idle / online を確認。

---

## 7. node が無いマシンでの UI

`scripts/update_amax_fw.sh` は npm が無ければ **UI ビルドを警告してスキップ**し、こう案内する：

```
[update-amax] WARN: npm not found on <host> — Angular dist/ NOT rebuilt.
[update-amax] WARN: ...on a Node machine: cd web/operator-ui && npm ci && npm run build
```

このとき **生成 TS ソース（amax-generated.ts）は更新済**で、`dist/` だけが未ビルド。別の node 機で：

```bash
git pull
cd web/operator-ui && npm ci && npm run build
git add web/operator-ui/dist/ && git commit -m "amax: rebuild UI dist" && git push
```

として dist をコミットすれば全機に反映される（dist はリポ同梱・ServeDir 配信）。

---

## 8. 実機確認（HW 接続時）

新しいレジスタ書き込みロジック（`merge_amax_channel_config` ＋ 自動導出アドレス）は **HW 電源 on → 次の Configure** から有効。

1. デジタイザ/NIM クレートを電源 on（off だと reader は `CAEN -4 DEVICE NOT FOUND` を出し続けるが、これは正常。on で自動再接続）。
2. Operator REST 経由で Configure（直接 ZMQ コマンドは使わない）。
3. apply ログでアドレスを目視（`BROADCAST_BASE`, per-channel アドレスが想定どおりか）。
4. Start 後に ADC スペクトラム / 波形が正常か確認（PHA/PSD 同様、異常ならクレート電源リセット）。

---

## 9. トラブルシューティング

| 症状 | 原因 / 対処 |
|---|---|
| `amax_codegen` が `... layout mismatch ... codegen aborted` で停止 | broadcast と per-channel のレイアウトが食い違っている。RegisterFile.json を確認。意図的なら override フラグで明示 |
| サマリに `N unmatched` / `no entry in fw_params.json` | そのレジスタは非公開。出したいなら fw_params.json に追記して再実行（§4B） |
| 新レジスタが UI に出ない | fw_params.json にエントリが無い、または `dist/` 未再ビルド（§7） |
| 新カテゴリのタブが出ない | `dist/` が古い。node 機で `npm run build`→commit |
| `npm not found` 警告 | node 未導入。§7 の二段運用、または gant のように node を入れる |
| reader が `CAEN -4 DEVICE NOT FOUND` | デジタイザ/クレートが電源 off。on で自動再接続。デプロイ問題ではない |
| Configure 後にアドレスがおかしい / FW が無反応 | 旧アドレス(0x0–0x30)書き込みで FW 破壊の可能性。デジタイザ電源サイクル後、正しい page-based 生成物で再 Configure |
| ビルドが `struct AMaxChannelConfig has no field ...` | 生成物と handle.rs が不整合（古い生成物のまま）。`scripts/update_amax_fw.sh` で再生成すれば一致する |

---

## 10. 仕組み（リファレンス）

### 自動導出ロジック（`src/bin/amax_codegen.rs` の `derive_layout`）
`RegisterFile.json` のレジスタを `channel_index()` で3群に分類：
- broadcast 群 `page_amax_energy/<NAME>`（`channel_index()==None`）
- per-channel ch0 群 `page_amax_energy_4_0/<NAME>`（`Some(0)`）
- per-channel chN 群（stride 測定用）

| 値 | 導出 |
|---|---|
| `PAGE_BASE` | ch0 群の最小アドレス（無ければ broadcast 群の最小） |
| `PAGE_STRIDE` | `addr(ch1) − addr(ch0)`（per-channel 群が無ければ 0） |
| `BROADCAST_BASE` | broadcast 群の最小アドレス（無ければ `PAGE_BASE`） |
| 正準 REG_* | ch0 群（あれば）、無ければ broadcast 群 |

両群に共通のレジスタは `(addr_bc − BROADCAST_BASE) == (addr_ch0 − PAGE_BASE)` を満たす必要があり、破れると codegen 停止（§5）。

### handle.rs が生成物に依存する点
`apply_amax_channel_config`（`src/reader/caen/handle.rs`）は、生成された
- `channel_register_byte_addr` / `broadcast_register_byte_addr`（アドレス）
- `channel_writes`（書き込みリスト）
- `merge_amax_channel_config`（override/default マージ）

を呼ぶだけで、**レジスタのフィールド名を直接列挙しない**。だから FW がレジスタを増減しても handle.rs は不変。

### UI が生成物を消費する点
`web/operator-ui/src/app/models/channel-params.ts` が `AMAX_PARAMS_BY_CATEGORY` を反復してタブを動的構築（`getFirmwareCategories`）。新カテゴリは自動でタブ化される。

### gant の node 環境（2026-06-26 導入）
Node 22 LTS を user-local（sudo 不要）に導入済：
```bash
# 新マシンでの再現
curl -fsSL https://nodejs.org/dist/latest-v22.x/ | grep -o 'node-v22[0-9.]*-linux-x64.tar.xz' | head -1   # ファイル名確認
# → ~/.local/node に展開、PATH を ~/.profile + ~/.bashrc に追記
cd web/operator-ui && npm ci    # node_modules 導入
```
gant は nodejs.org / npm registry 到達可・glibc 2.35（Ubuntu 22.04）。
