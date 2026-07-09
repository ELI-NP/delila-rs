# TODO 61: delila2root C++ 化 + 自己記述 `.delila` フォーマット (v3)

**Status: COMPLETED (2026-07-08、commits `fdc0721` format v3 + `90cd4ea` C++ TDelila/delila2root、master push 済)**
**Created:** 2026-07-08
**Hardware:** N/A (offline tool)。ただし C++ ビルド/変換検証は ROOT のある実機 (Side3) で実施
**Plan file:** `~/.claude/plans/ui-configure-bubbly-deer.md`
**Supersedes:** TODO 56 / 57 (Rust/oxyroot `delila2root`)。両者は完了扱いのまま、実装は本タスクで置換

## ゴール

Rust/oxyroot `delila2root` を退役し、ROOT ユーザ向けに:
1. **自己記述 `.delila` (format v3)** — ヘッダ `metadata["event_schema"]` にイベント構造 (フィールド順+型タグ) を JSON で埋め込む。
2. **`TDelila.hpp`** — 依存ゼロ (自作 MsgPack reader + JSON parser 埋め込み) の単一ヘッダ。マクロで `#include` して `.delila` を直読み。full-generic + 波形は遅延パース。v2 ファイルは組込みレイアウトで後方互換。
3. **`delila2root`** — TDelila 上の薄い変換ツール (`.C` マクロ / コンパイル tool)。全フィールドを branch 化 + **ROOT ネイティブ ZSTD 圧縮** (oxyroot は圧縮不可だった)。

## 背景

旧 Rust `delila2root` (oxyroot 0.1.25) の2つの不満:
1. **圧縮不可** → 無圧縮出力、`hadd -f404` 後処理が必須。
2. **49-branch 固定ツリー** (AMax debug FW 前提: analog_probe3 / digital_probe5..16 / probe_type) が V1743/PSD/PHA では大半空。

ユーザ判断: Rust 版は完全廃止。AMax 開発者向けには「存在する情報を全部書く」+ ZSTD 圧縮で空 branch はほぼゼロに畳まれる。「positional MsgPack への密結合」は自己記述スキーマで解消。

## 実装 (P1〜P4)

- **P1 (Rust, `src/common/`, `src/recorder/`)**:
  - `src/common/delila_schema.rs` 新設: `EventDataBatch`/`EventData`/`Waveform` の順序付きフィールド descriptor (手書き const) + `schema_json()` + **round-trip drift-guard テスト** (実 struct を serialize → dynamic decode → schema で walk して一致検証、フィールド追加/並替を検知)。
  - `EventData.waveform` の `#[serde(skip_serializing_if)]` 撤去 → 常に 8要素配列 (nil/value)。`#[serde(default)]` 維持で v2 読み後方互換。digital_probe の "packed" 誤コメント修正 (実際は 1 u8/sample)。
  - `FORMAT_VERSION` 2→3。recorder `open_new_file` がヘッダ metadata にスキーマ注入。
- **P2 (`tools/delila2root/TDelila.hpp`)**: 自作 MsgPack Value/Reader (BE int/float/nil/bool/array/str/map、skip_value)、最小 JSON parser、Schema (埋め込み JSON or v2 fallback)、Header/Footer、per-event 遅延 (scalar 即 / 波形は生バイト copy → `waveform()` で遅延パース、Event 所有でライフタイム安全)。
- **P3 (`tools/delila2root/`)**: `delila2root.C` (全フィールド branch + ZSTD 505 + schema coverage 警告 + footer 突合)、`example_analysis.C` (直読みマクロ例)、`README.md`。
- **P4**: `src/bin/delila_to_root.rs` + Cargo `[[bin]]` 削除 (oxyroot は event_builder が使うので dep 残置)。TODO/CURRENT 更新。

## 検証結果

### ローカル (Mac)
- `cargo fmt` / `cargo clippy --lib --tests -- -D warnings`: clean
- `cargo test --lib delila_schema`: 2/2 PASS (`schema_json_is_wellformed`, `schema_matches_wire_layout`)
- recorder/format/file_format tests: 全 PASS
- `TDelila.hpp` を clang++ -std=c++17 で単体ビルド (ROOT 不要):
  - **v2 実データ** `data/E9/run0382_0317_Fission2026.delila` (452 MB): **14,991,289 / 14,991,289 events 読取・footer 一致** (0.74 s)。schema_bytes=0 → v2 fallback 動作。
  - **v3 サンプル** (throwaway example で生成): version=3 / schema 1647 B 解析 / 波形遅延パース (analog1=5, digital1=5, ns_per_sample=0.312) / footer 一致。

### 実機 (Side3, ROOT 6.36.08 + g++)
- [x] `delila2root.C` を ROOT でビルド成功 (`g++ -std=c++17 $(root-config --cflags --libs)`)。ZSTD(505) 受理。
- [x] **v3 サンプル変換** → ROOT tree entries=2、波形値 exact (analog_probe1=[100,-200,300,-400,500], digital_probe1=[1,0,1,1,0], ns_per_sample=0.3125)、no-waveform event は empty vector。全 branch (`t->Show`) 出力確認。
- [x] **v2 実データ変換** (後方互換) `data/ThGEM_test/run0003_0000` (1073 MB): **2,672,904 events**、footer mismatch 警告なし、v2 fallback layout 動作。
- [x] **ZSTD 圧縮**: 1073 MB → 536 MB、tree compression factor **4.32x** (旧 oxyroot は無圧縮 + `hadd -f404` 後処理必須だった)。ROOT で読取・Scan 正常。
- [x] **実 V1743 波形付き v3 end-to-end** (新 recorder deploy + rebuild → run 41 短ラン): `/data/ThGEM_test/run0041_0000_data.delila` (header_size=1702 = schema 埋込)。変換 → **108,554 events 全て波形付き・footer 一致**、`analog_probe1` length=256 (record_length)、digital=0 (V1743 standard は digital probe 無し=正)、ns_per_sample=0.3125 (3.2 GHz)、ch0/ch8 の信号。ZSTD 3.34x。波形なし v3 ラン (run 40, 143678 ev) も footer 一致・7.02x。
- 検証後 `save_waveform` を false へ復元、DAQ 停止。

**Status 更新: コード完成・全経路 (local + Side3 C++ + 実 v3 end-to-end 波形あり/なし + v2 後方互換) 検証済。未コミットのみ。**

## メモ
- ROOT compression は `algorithm*100+level` の整数指定 (505=ZSTD5) で `ROOT::kZSTD` enum の版依存を回避。
- スキーマ手書き const + drift-guard テストを採用 (serde-reflection はワイヤ形式の癖 = skip/packed を捉えられず半自動に留まるため見送り)。将来ゼロ維持化したければ `#[derive(DelilaSchema)]` proc-macro。
