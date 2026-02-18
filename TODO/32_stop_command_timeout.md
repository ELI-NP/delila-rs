# TODO #32: 高データレート時の Stop コマンドタイムアウト修正

**Created:** 2026-02-16
**Status: ✅ COMPLETED** (2026-02-16)
**Priority:** 1 (Tune Up モードに影響)

## 問題

Tune Up モードで高データレート時に Stop コマンドがUIでタイムアウトエラーになる。

## 根本原因

Reader の `decode_loop` (tokio::spawn) が CPU-bound なデコード＋シリアライズを行い、高データレートで yield せずに回り続ける。同じ tokio runtime 上の `command_task` がスケジュールされず、ZMQ REP がレスポンスを返せない → 5秒タイムアウト。

## 修正

### 変更1 (主因): decode_loop に yield_now() 追加
- File: `src/reader/mod.rs`
- `tokio::task::yield_now().await` で command_task にスケジュール機会を提供

### 変更2 (予防): Recorder writer_task を std::thread に分離
- File: `src/recorder/mod.rs`
- 通常Run時の同様の問題を防止（同期ディスクI/Oが tokio をブロック）
- チャンネル: `tokio::sync::mpsc` → `std::sync::mpsc`

### 変更3 (副次): Reader Stop信号送信を保証
- File: `src/reader/mod.rs`
- `try_send(ReadLoopOutput::Stop)` → リトライ付き（最大3秒）

### 変更4 (副次): Reader ドレインループに上限追加
- File: `src/reader/mod.rs`
- 最大1000イベント or 1秒

## 設計書

- `docs/plans/stop_command_timeout.md`

## テスト

- `cargo clippy && cargo test` ✅
- 実機: Tune Up モード高レート → Stop がタイムアウトしないこと ✅
- 実機: 通常Run高レート → Stop + Recorder ファイルクローズ正常 ✅

## 追加修正 (同セッション)

### Tune Up Apply スペクトラム混在修正
- **問題:** Apply 後に ~1-2秒間の古い設定データがヒストグラムに混入
- **原因:** Merger `sender_task` がステート非対応。Stop 後も mpsc→ZMQ PUB を無条件転送
- **修正:**
  - `src/merger/mod.rs`: sender_task に `watch::Receiver<ComponentState>` 追加。Running 時のみ転送、非 Running 時はドレイン
  - `src/operator/routes/tuneup.rs`: 全パイプライン Stop→Start（Reader+Merger+Monitor）
  - `src/operator/routes/mod.rs`: `tuneup_apply_lock` 追加（二重押し防止）
  - `web/operator-ui/.../waveform.component.ts`: RxJS chain (stopPolling + clearHistograms + finalize)
- **実機確認済み** ✅

### Probe ラベル 0始まり
- Analog Probe: A1/A2 → A0/A1, Analog 1/2 → Analog 0/1
- Digital Probe: D1-D4 → D0-D3, Digital 1-4 → Digital 0-3
- 対象: チェックボックスラベル、チャート凡例、Virtual Probe ドロップダウン

### PHA1 Waveform Decoder 修正
- **sign_extend_14bit()**: PHA1 は 14-bit 2の補数（負パルス→負値）。PSD1 は符号なし。
  - `(w & ANALOG_SAMPLE_MASK) as i16` → `sign_extend_14bit(w)` (PHA1のみ)
  - PSD1 は従来通り `(w & ANALOG_SAMPLE_MASK) as i16`（上位ビットが0なので符号拡張不要）
- **Digital Probe マッピング修正**: bit14=DP(vtrace/3, configurable), bit15=Tn(vtrace/2, fixed trigger)
  - 旧: DP→digital_probe1, Tn→digital_probe2
  - 新: Tn→digital_probe1(D0), DP→digital_probe2(D1) — vtrace UI順序と一致
- `common.rs` に `sign_extend_14bit()` 関数追加（テスト付き）

### PHA1 パラメータ min/max 修正
- `input_rise_time_ns`: min 32→16, max 4080→2040（DevTree実機値に合わせて修正）
- `trap_flat_top_ns`: max 16368→8184（DevTree実機値）
- `peak_holdoff_ns`: min 16→8, max 16368→8184（DevTree実機値）

### Apply ボタン二重押し防止（Frontend）
- `digitizer-settings.component.ts`: `applying()` signal 追加、Apply中はボタン disabled + early return

### ROOT マクロ ns_per_sample 対応
- `macros/read_delila.C`: Waveform 配列サイズ 8 or 9 対応、X軸 ns 表示

### 設定変更
- Operator デフォルトポート: 8080 → 9090（8080 は他サービスと競合回避）
- `start_daq.sh`: デフォルト `OPERATOR_PORT=9090`
- `docker-compose.yml`: mongo-express 8082→8083（8082=eLog）
- `config/config_pha1_test.toml`: Recorder セクション除去（Tune Up テスト用）、ポート 9090
