# AMax Viewer 改善プラン

## Phase 1: モジュール分割（基盤整備）

単一ファイル915行を機能単位に分離する。動作変更なし、リファクタリングのみ。

- [ ] `src/histogram.rs` — `Histogram2D`, `colormap()`, `to_texture()`
- [ ] `src/event_buffer.rs` — `EventBuffer`, ROOT出力ロジック
- [ ] `src/acquisition.rs` — `acquisition_thread()`, `apply_params()`
- [ ] `src/settings.rs` — `AppSettings`, `RegisterDef`, `load_register_defs()`, `init_param_values()`
- [ ] `src/main.rs` — `AmaxViewerApp`, `eframe::App` impl, `main()`のみ残す

## Phase 2: バグ修正

### 2-1. apply_params の read-back エラーカウント矛盾
- **現状**: read-back失敗時に `success += 1` しつつ `first_error` にも記録（L863-868）
- **修正**: `read_errors` カウントを追加、read-back失敗は success にカウントしない
- 戻り値を `(success, write_errors, read_errors, mismatches, first_error)` に変更

### 2-2. 取得スレッドのエラー非表示
- **現状**: `e.code != -12` のエラーは sleep するだけでUIに通知されない（L805-809）
- **修正**: エラーメッセージを `status_message` に反映、連続エラー回数も表示

## Phase 3: パフォーマンス改善

### 3-1. EventBuffer::write_root の不要な clone() 除去
- **現状**: 全Vecを `.clone()` してからイテレータ化（L104-114）
- **修正**: `.iter().copied()` に変更。メモリ使用量半減

### 3-2. Mutex contention 軽減
- **現状**: 取得スレッドがイベント毎にMutex取得（L772）、GUI描画中も同じMutexを複数回ロック
- **方針**: ヒストグラムのダブルバッファ or crossbeam channel 経由でGUIにデータ送信
- to_texture() をMutexロック外に移動（データのスナップショットを取ってからピクセル計算）

### 3-3. EventBuffer のメモリ制限
- **現状**: recording中はメモリ無制限成長
- **修正**: 上限イベント数を設定可能にし、到達時に警告表示 or 自動停止

## Phase 4: UX 改善

- [ ] カラーバー（凡例）の追加 — 現在のカウント範囲を色で表示
- [ ] 1Dプロジェクション — X軸/Y軸への射影ヒストグラム表示
- [ ] レジスタ値の16進数表示トグル — FWデバッグ時に便利
- [ ] Waveformパネルに閾値ライン表示（THRSレジスタ値を水平線で表示）
- [ ] `data_aspect(1.0)` 固定の解除 — Energy/AMax範囲が異なる場合にアスペクト比を自動調整
- [ ] ログスケール表示オプション（sqrt以外に log10 も選択可能に）

## Phase 5: その他

- [ ] waveform_buffer の二重管理解消（SharedState内とローカル変数の両方に存在）
- [ ] colormap の最終区間を明確化（yellow→white の意図を明示、または別カラーマップ追加）
- [ ] gen_defs: section推定ロジックの改善（マジックナンバー 1441792 にコメント追加）
