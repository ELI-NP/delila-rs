# TODO 62 — V1743 decoder から rollover を撤去（生 TDC 直接化）

**Status: COMPLETED (2026-07-09、commit `40e87bf`、master merge + push 済)**

## 実装結果 (2026-07-09)

- `x743_std_event_to_event_data`: rollover 撤去、`timestamp_ns = (group.TDC & 0xFF_FFFF_FFFF)×5 + cfd`。`FLAG_TDC_UNDERFLOW` 削除、per-group tracker/reset 撤去。
- 診断専用 `X743TdcDiag`（timestamp 不変の observer）。**敵対的レビュー(7エージェント)の指摘2件を反映**: ① 後退ログを `backward_budget`(50/group) で上限化 + `backward_count` 累計保持（グリッチ嵐でホットパス同期ログが無制限化しない）② `test_x743_tdc_diag_backward_detection_and_rearm` で observe/rearm ロジックを検証。
- 旧 rollover テスト → `test_x743_std_event_to_event_data_raw_tdc_self_heals`（glitch イベントのみズレ次で自己回復を証明）。
- `RolloverTracker` 本体・`rollover.rs` は無変更（PSD1/PHA1 が使用）。`prev_raw()` accessor は撤去し HEAD と同一に。
- **検証**: host_side3 で `cargo build/clippy --features x743 --tests -D warnings` クリーン、TODO62 テスト4件 pass、x743 スイート 17 pass / 2 fail（2件は既存 CFD 問題 = [TODO 63](63_v1743_cfd_search_window.md)、本件無関係と stash で確認済）。
- **実機確認済 (run46, Web UI 経由, 1分/1.19M events)**: dropped=0、`2.199e13`/underflow 伝播なし、timestamp 単調。診断が run30 の**真犯人を捕捉** = 起動後 ~1ms の**ゴミ TDC 6件**（未初期化 DMA バッファ; `0x1B1B1B1B`/`0x04040404` バイト反復）。旧コードはこれを rollover 永続状態が恒久破損に増幅していた。新コードでは各ゴミが自分だけ狂い次で自己回復（伝播ゼロ）。
- **ゴミは受け入れ方針**（落とさない=絶対ルール遵守、解析で run 冒頭 ~1ms カット）。`docs/operations_manual.md §9`(JA/EN) に明記。根治（なぜ CAEN が未初期化バッファを読むか）は将来課題。
- daq_ctl.sh CLI は configure 後に hang するため実機ランは Web UI で駆動（memo）。

---

**Status(原計画): 📋 PLANNED (2026-07-08 立案)**

## 背景 / 決定

run30.root で V1743 のタイムスタンプが破損していた（[[v1743_tdc_clean_run30_anomaly]]）:
- start-of-run: ch8 が 24–34ns に潰れ、全て `FLAG_TDC_UNDERFLOW`
- mid-run: `timestamp = 2.199e13 ns` = ちょうど `4×2^40×5ns` = `RolloverTracker.rollover_count = 4`（376秒ランで本物 wrap=91分は不可能 → 偽 rollover）

**診断で確定した事実（"V1743 TDC diag" トラップを仕込んで実測）:**
- **生 `group.TDC` はクリーン**（単調増加・上位ビット健全・10kHz周期一致）。ユーザー(Aogaki)の当初の主張が正しかった。
- **フレッシュ reader 再起動後のランは underflow=0 / 偽rollover=0**（20秒・5分いずれも）。run30 は再現せず。

**結論（ユーザー判断）:** run30 の破損は「どこかで生 TDC が一瞬チグハグになった局所 glitch」を、`RolloverTracker` の**永続状態（`prev_raw` / `rollover_count`）が恒久破損に増幅**したもの。生 TDC を直接使えば glitch は自己回復する（その1イベントだけズレて次で復帰）。KISS 原則 + [[layering_principle_clock_sync]]（時刻補正は decoder 層でやらない）にも合致。

## 運用ルール（前提条件）

- **ランは絶対に 90分未満**。実運用は **60分で新ランを開始**する。
- これにより 40-bit TDC @ 5ns（wrap=91.6分）は**そもそも wrap しない** → decoder 側で rollover 補正が不要になる。
- （wrap 対応が将来必要になったら EB 層で。decoder には戻さない。）

## 実装（`src/reader/mod.rs`, `x743_std_event_to_event_data`）

### 1. rollover extend を撤去、生 TDC 直接

現状:
```rust
let (tdc_ticks, tdc_underflow) = match tracker.extend(group.TDC) {
    Ok(t) => (t, false),
    Err(e) => { warn!(...); (group.TDC & 0xFF_FFFF_FFFF, true) }
};
let tdc_ns = tdc_ticks as f64 * TDC_NS;
```
↓
```rust
// 40-bit マスクのみ（上位ビット garbage を防御的に除去。<90min ラン運用で wrap しない）。
let tdc_ticks = group.TDC & 0xFF_FFFF_FFFF;
let tdc_ns = tdc_ticks as f64 * TDC_NS;
```
- `timestamp_ns = tdc_ns + s.cfd_time_ns` はそのまま（CFD 加算は不変）。
- **40-bit マスクは残す**: run30 の glitch が「上位ビット garbage」型ならマスクで消える（＝二重の防御）。lower 40bit の後退型 glitch でも、rollover が無いので局所で自己回復。

### 2. `FLAG_TDC_UNDERFLOW` 撤去

- `const FLAG_TDC_UNDERFLOW` と `if tdc_underflow { flags |= ... }` を削除。
- `FLAG_CFD_VALID` / `FLAG_WF_DECODE_FAIL` は残す。

### 3. per-group tracker / reset / diag param を撤去

- read loop の `let mut tdc_trackers: Vec<RolloverTracker> = ...`（~L1830）削除。
- SWStart branch の `for t in tdc_trackers.iter_mut() { t.reset(); }`（~L1991）削除。
- 関数シグネチャから `tdc_trackers: &mut [RolloverTracker]` を削除。
- 呼び出し2箇所（running path + Stop drain）と test 4箇所を更新。

### 4. glitch 可視化は残す（silent failure 禁止 / CLAUDE.md）

rollover は消すが、**生 TDC が後退したら info! ログ**する軽量チェックは残す。**タイムスタンプには一切影響させない**（増幅しないのが要点）:
```rust
// per-group の last_raw は「診断ログ専用」。timestamp 計算には使わない。
// raw が前イベントより後退したら（wrap 閾値未満の後退＝チグハグ）info! で可視化。
```
- 現行の "V1743 TDC diag" トラップ + `RolloverTracker::prev_raw()` accessor は、この軽量ログに**置き換え or 縮小**する（budget=最初のN件 + 後退検知時）。
- これで run30 級の生 TDC glitch が再発した時、根本原因（なぜ生 TDC が汚れるか）を後追いできる。

## テスト更新（`src/reader/mod.rs` tests）

- `test_x743_std_event_to_event_data_tdc_rollover` → **rollover を前提にしているので削除 or 「生 TDC 直接（wrap しない前提）で monotonic」テストに書き換え**。
- `test_..._no_waveform_fallback`（`TDC=100 → 500ns`）→ 生 TDC 直接でも 500ns のままのはず。UNDERFLOW flag assertion（bit26）は削除。
- `test_..._absent_event` → シグネチャ変更に追従。

## `RolloverTracker` 本体は残す

- `src/reader/decoder/rollover.rs` は **削除しない**。PSD1/PHA1 の SW Fine TS（32-bit BoardAgg TTT）で使用中（[TODO 47](47_v1743_standard_mode_redesign.md)）。
- V1743 からの参照だけ外す。`prev_raw()` accessor は残置で無害（or 未使用なら revert）。

## ビルド / デプロイ / 検証（host_side3）

- host_side3 (192.168.147.99, `~daq/delila-rs`) は **非git rsync コピー**。Mac から `rsync` で src を送る（[[host_side3_deployment]]）。
- **必ず `cargo build --release --features x743 --bin reader`**（[[side3_x743_build_feature]]）。
- 検証:
  1. 通常ラン（mask=17、10kHz パルサー）で timestamp が単調・クリーンか確認。
  2. **run30 再現を試みる**: reader を再起動せず Configure→Arm→Start→Stop を多数サイクル + 数分ラン。以前 run30 を生んだ状況（-15切断復帰 / rapid Stop+Apply / 再接続 = [[felib_stuck_after_rapid_stop_apply]]）を踏む。
  3. glitch が起きても **その1イベントだけのズレで自己回復**すること（恒久 +91分ズレや underflow ストームが**出ない**こと）を確認。
  4. Recorder `dropped=0` 継続。

## 関連 / 別件

- **config `group_enable_mask=17` は保持**（[[v1743_global_trigger_deadtime]]、2026-07-08 に 255→17 でデッドタイム解消、10kHz each 全取り確認済）。本タスクとは独立。信号を別 ch に繋ぐ時は mask 見直し。
- **未解決の根本原因**: 「なぜ生 TDC が一瞬チグハグになるか」は本タスクでは直さない（robust 化が目的）。上記検証 2–3 のログ（軽量トラップ）で後日追う。

## 現在のコード状態（引き継ぎ）

- Mac 作業ツリー + host_side3 に **診断コード "V1743 TDC diag" + `prev_raw()` accessor が入っている（未コミット、両方）**。本タスクでこれを置換する。
- 変更ファイル: `src/reader/mod.rs`, `src/reader/decoder/rollover.rs`。
