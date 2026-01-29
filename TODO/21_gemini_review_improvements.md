# Gemini レビュー指摘事項の改善

**作成日:** 2026-01-29
**完了日:** 2026-01-29
**状態:** ✅ 実装完了

## 概要

Gemini によるコードレビューで指摘された問題点を修正する。
MVP 目標（2026年3月中旬）に向けて、堅牢性を優先しつつパフォーマンス改善も実施。

---

## Phase 1: 即座に修正 (堅牢性向上) ✅

### R1: f64 比較の unwrap 除去 ✅
- **箇所:** `src/reader/decoder/psd2.rs`
- **問題:** `partial_cmp().unwrap()` は NaN で panic
- **修正:** `unwrap_or(std::cmp::Ordering::Equal)` に変更
- **備考:** psd1.rs と pha1.rs は既に修正済みだった

### R2: シャットダウン処理の unwrap 除去 ✅
- **箇所:** `src/common/shutdown.rs`
- **問題:** 受信側がない場合に panic
- **確認結果:** 既に `let _ = tx_clone.send(());` で対応済み

### R3: Operator Bytes::into_vec() 削除 ✅
- **箇所:** `src/operator/client.rs`
- **確認結果:** コードベースに該当パターンなし（Gemini の誤検出）

### R4: ファイル名衝突回避の強化 ✅
- **箇所:** `src/recorder/mod.rs` の `generate_filename`
- **問題:** 秒単位タイムスタンプでは 1 秒以内の再起動で衝突
- **修正:** `as_secs()` → `as_nanos()` に変更

### R5: try_into().unwrap() のエラーハンドリング ✅
- **箇所:** `src/reader/decoder/psd1.rs`, `pha1.rs` の `read_u32` 関数
- **問題:** データ破損時に panic
- **修正:** `data.get()` + `unwrap_or(0)` で境界外アクセス時も安全に

---

## Phase 2: 設計検討・改善 ✅

### D1: SystemState 優先順位の修正 ✅
- **箇所:** `src/operator/mod.rs`
- **問題:** `Degraded` が `Error` より優先されている
- **修正:** `Error` チェックを先に行うよう順序変更
- **テスト:** 新規テスト `test_system_state_error_takes_priority_over_degraded` 追加

### D2: MongoDB unwrap の適切なエラーハンドリング ✅
- **箇所:** `src/operator/run_repository.rs`
- **問題:** `to_bson().unwrap()` が多数
- **修正:** `.expect("TypeName serializes to BSON")` で意図を明示
- **理由:** Rust → BSON 変換は制御下の型なので常に成功。expect で仮定を文書化

### D3: 設定ファイルの柔軟なハンドリング ✅
- **確認結果:** 既に `if let Some(...)` パターンで対応済み

---

## Phase 3: パフォーマンス最適化 ✅

### F1: デコーダ Vec 再利用 ✅
- **箇所:** `src/reader/decoder/*.rs`, `src/reader/mod.rs`
- **問題:** decode() 毎に Vec<EventData> を新規作成
- **修正:**
  - `decode_into(&mut Vec<EventData>)` メソッドを全デコーダに追加
  - Reader の DecodeLoop で `events_buffer` を再利用
  - `drain(..)` でバッファ容量を維持しつつイベント取り出し

### F2: Monitor list_histograms 最適化 ✅
- **箇所:** `src/monitor/mod.rs`
- **問題:** `list_histograms` が全ヒストグラムをクローン（65k bins × N channels）
- **修正:**
  - `HistogramListSummary` 構造体追加（ビンデータなし）
  - `GetListSummary` メッセージ追加
  - `list_summary()` メソッド追加（キーとカウントのみ収集）
  - `list_histograms` エンドポイントを軽量サマリーに変更

### F3: Monitor receiver_task の最適化 ✅
- **確認結果:** 現状の設計が最適
  - チャンネルベース設計は clean で保守しやすい
  - Bytes を histogram_task に渡しても総コストは同じ
  - 複雑化のリスクに対してメリットが小さい

---

## 検証結果 ✅

### 単体テスト
- [x] 全 `cargo test` パス (319 tests)
- [x] clippy 警告なし

### CI 検証
- [x] cargo fmt --check パス
- [x] cargo clippy パス
- [x] cargo test パス

### 統合テスト ✅
- [x] マルチデジタイザ統合テスト (PSD2 + PHA1) — Run 7 完了

| Digitizer | 場所 | イベントレート | 総イベント数 |
|-----------|------|---------------|-------------|
| PSD2 (VX2730) | ローカル | ~10,000 evt/s | 341,246 |
| PHA1 (DT5730B) | リモート (172.18.4.147) | ~1,000 evt/s | 34,057 |
| **Recorder 合計** | — | ~11,000 evt/s | 375,303 |

---

## 修正ファイル一覧

| ファイル | 変更内容 |
|---------|----------|
| `src/reader/decoder/psd2.rs` | R1: sort_by unwrap 修正, F1: decode_into 追加 |
| `src/reader/decoder/psd1.rs` | R5: read_u32 安全化, F1: decode_into 追加 |
| `src/reader/decoder/pha1.rs` | R5: read_u32 安全化, F1: decode_into 追加 |
| `src/reader/mod.rs` | F1: Vec 再利用, decode → decode_into 移行 |
| `src/recorder/mod.rs` | R4: ナノ秒タイムスタンプ |
| `src/operator/mod.rs` | D1: SystemState 優先順位, テスト追加 |
| `src/operator/run_repository.rs` | D2: expect メッセージ追加 |
| `src/monitor/mod.rs` | F2: HistogramListSummary, GetListSummary |

---

## 参考資料

- `GEMINI.md` - 元のレビュー内容
- `TODO/20_data_integrity_and_performance_audit.md` - 既存のパフォーマンス改善
- `CLAUDE.md` - コーディング規約
