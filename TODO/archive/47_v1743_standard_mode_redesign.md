# V1743 Standard Mode 再設計（DPP-CI 撤退）

**作成日:** 2026-04-20
**更新日:** 2026-04-23 — Step 3 完全完了（RolloverTracker 統一 + 95 min long-run で TDC rollover 通過確認）
**ステータス:** Step 1-3 完了 / Step 4-7 はハードウェア拡張（V1743 2 台目 + S1 分配）待ち
**置換対象:** [45_v1743_support.md](45_v1743_support.md) の Phase 2 (DPP-CI) 以降
**設計書:** [docs/plans/x743_standard_mode_design.md](../docs/plans/x743_standard_mode_design.md)
**権威ソース:** [UM2750 V1743 User Manual Rev.5](../legacy/UM2750_V1743_User_Manual_rev5.pdf) (May 2025, FW 4.29_2.24)
**物理要件:** 時間情報から 2D 位置を計算する位置感応型検出器 → 時間分解能最優先、電荷は副次的
**背景:**
1. CAEN App Note `docs/Setting_CHARGE_MODE_for_the_x743_modules.pdf` で現 `apply_config_dpp_ci` の構造体誤流用バグ発覚
2. User Manual UM2750 Rev.5 **Fig 10.9** で Charge Mode ワイヤフォーマットに **TDC が存在しない**ことが確定
3. 同 Sec 10.10.3.1 太字注記が「物理時刻は Standard mode の 40-bit TDC のみ」と明示
→ Charge Mode を使う選択肢は物理的に消滅。Standard mode 一本化で確定
**協議:** Gemini 2.5 Pro (2026-04-20) — per-board BTT 妥当性、S_IN 同期、pitfall レビュー

---

## 1. 撤退理由（要約）

### 1.1 決定的根拠: User Manual Fig 10.9 に TDC が無い

UM2750 Rev.5 Fig 10.9 "Group Data Format in Charge Mode" より、Charge Mode のワイヤフォーマット：

```
[ 00 | REF_CELL_COLUMN (10-bit) | 1 | CHARGE (23-bit, 2's complement, pC) ]
× 256 events per channel FIFO
```

**TDC フィールドが存在しない**。Standard mode の Fig 10.8 には per-group `TDC` (40-bit @ 5 ns) があり、
Sec 10.10.3.1 の太字注記が「物理時刻は TDC のみ、EVENT TIME TAG は readout 時刻で物理量ではない」と明示。

**本プロジェクトは時間情報から 2D 位置を計算する → Charge Mode は物理的に使用不可**。

### 1.2 実装バグ（二次的撤退理由）

`apply_config_dpp_ci` が `DPP_CI_Params_t` (x720 用) を使用。x743 用 `DPP_X743_Params_t` と
構造体レイアウト不一致 → FW は意味不明なビットを読む。`GetDPPEvents` 後のイベント型も同様にミスマッチ。
ただし修正しても §1.1 で撤退決定されるため、このバグ自体を直す意味はない。

### 1.3 既存 probe テスト doc との矛盾

`docs/x743_dpp_ci_parameters.md` (2026-04-15) は「DPP_CI 切替後も `X743_EVENT_t.TDC` が取れる」と主張するが、
User Manual Fig 10.9 のワイヤフォーマットに TDC ビットが無い以上、`X743_EVENT_t.TDC` の値は：

- 未初期化メモリ or 前 Run の残骸
- FW の undocumented 副作用（CAEN は保証せず）

のいずれか。実測値が偶然 plausible に見えても運用には採用不可。

### 1.4 冗長 API 呼び出し

`SetDPPAcquisitionMode`, `SetDPPEventAggregation`, `SetDPPPreTriggerSize` は x720 系 DPP 用。
x743 FW 状態を壊す可能性あり（Gemini 指摘）。呼び出し自体を削除する。

## ★ 時間分解能 (V1743 採用の主要目的)

V1743 の核心特性は **Time INL Correction により 5 ps RMS**（マニュアル Sec 10.9）。本プロジェクトは
時間情報から 2D 位置を計算するため、この分解能を確実に確保する。

### 時刻合成
```
total_time_ns = TDC * 5 (ns)                  // coarse, 40-bit @ 5ns
              + PosEdgeTimeStamp (ns, float)  // fine, サブサンプル補間 → 5 ps RMS
```

### 必須設定
- **`correction_level = "all"` を config デフォルトに固定** → Line Offset + Individual Pedestal + Time INL + Trigger DAC Offset
- `sampling_frequency = "3.2GHz"` (0.3125 ns LSB)
- `record_length` は rising edge が捉えられる十分な長さ（PosEdgeTimeStamp 補間に必要）
- `fine_time_source = "pos_edge"` (初期)、必要時 `"cfd_soft"` で CFD 補間に切替可能

### EventData 設計
- `fine_time: u16` フィールドに PosEdgeTimeStamp を詰める（~114 fs LSB で 1 tick 5 ns を充分カバー）
- Merger / EB は `global_timestamp_ps = timestamp_ns * 1000 + fine_time_ps` で ps 分解能を保持

## 2. Standard mode で得られるもの（マニュアル + WaveDemo 精読結果）

`CAEN_DGTZ_X743_GROUP_t` は Standard mode で以下を提供（マニュアル Sec 10.10.3.2 + CAENDigitizerType.h）：

- **`TDC: uint64_t`** (40-bit @ 5 ns SAMLONG clock = ~91 分レンジ, per-group, reset on acquisition start/S_IN)
- **`Charge: float`** (FW が波形積分、Standard mode でも populated — WaveDemo がこれを利用)
- **`Baseline: float`**, **`Peak: float`**, **`PeakIndex: uint16_t`**
- **`PosEdgeTimeStamp: float`**, **`NegEdgeTimeStamp: float`** (サブサンプル補間)
- **`DataChannel: float*`** — 1024 samples 波形（あるいは `SetRecordLength()` で短縮可）
- `TriggerCount`, `TimeCount`, `StartIndexCell`, `EventId`

→ **BTT per-channel は不要**（per-board 40→64-bit 軽量拡張で十分）
→ **Rust 側で波形積分する必要なし**（`Charge` を採用、必要なら波形も同時保存）
→ **S1 は heartbeat 不要で、クロスボード定数較正のみに使用**

## 2.1 バッファ・レート比較（マニュアル Sec 10.3, 10.10, 10.11）

|  | Standard | Charge (撤退) |
|---|---|---|
| SRAM 消費/event | **1024 × 12-bit × 2ch** | 波形破棄、**Charge FIFO 256 ev/ch** |
| Buffer depth (1024 sample時) | 7 events/ch | — |
| 律速 | PC 帯域 | SAMLONG 変換 DT (~125μs) |
| Max rate | 帯域次第 | ~7 kHz (manual) |
| TDC | ✅ | ❌ |

**レート対策**: `SetRecordLength()` で 1024 → 128-256 に減らすと buffer depth 倍増 + PC 帯域軽減（マニュアル Sec 10.3）。

## 3. 実装ステップ

| Step | タスク | 成功条件 |
|---|---|---|
| 1 | ✅ **完了 (2026-04-22)** — `apply_config_dpp_ci` + DPP-CI 関連コード全削除。`apply_config_standard` のみ残す。`correction_level` デフォルト "all" 化 | `cargo build --features x743` 成功、`cargo clippy --features x743 -- -D warnings` OK |
| 2 | ✅ **実機確認完了 (2026-04-22)** — Reader V1743 ReadLoop Standard mode 一本化。パルサ 10 kHz で波形・E スペクトル確認済（SN:25, 172.18.4.147）。設定確定: record_length=256, post_trigger=40, threshold=45874 (=-0.5V), polarity=Negative | 10 kHz 100% 取得、peak@33ns/80ns窓, E ヒスト FWHM 8 bins, run2001 = 448MB 記録成功 |
| 3 | ✅ **Step 3 全完了 (2026-04-23)** — RolloverTracker 統一 + V1743 組込 + PSD1/PHA1 migration + 旧 TimestampTracker 削除 + **V1743 95 分連続ランで 40-bit TDC rollover 通過確認**（120M events, rollover_count 0→1, 新規 underflow 0 件, 0 error） | run6001 = 42 GB/42 files, timestamp 5745s (rollover period 5497s を 4.5% 超過), EB ソート正常 |
| 4 | ⏸️ **HW 拡張待ち** — self-trigger 閾値/極性 per-ch は実装済、`SinFanout` + `S_IN_CONTROLLED` 同期は V1743 2 台目の入手後 | arm_all_sync で V1743 含め同時 arm |
| 5 | ⏸️ **HW 拡張待ち** — 2-3 台拡張、S1 ΔT ヒスト (fine time 込み) で board 間オフセット ps 精度較正 | S1 ΔT FWHM < 100 ps |
| 6 | 📌 **保留** — V1743 energy は simple amplitude (|peak - baseline|) のまま。flat-top broadening は peak-finder の sample index 量子化由来で既知、timing 用途につき放置 ([memory](/Users/aogaki/.claude/projects/-Users-aogaki-WorkSpace-delila-rs/memory/v1743_energy_known_limitation.md)) | — |
| **7** | ⏸️ **HW 拡張待ち** — 時間分解能測定（同一 board 2 ch の S1 ΔT FWHM ~7 ps RMS 確認）、必要なら Rust CFD 改良（現状の soft CFD は Step 3 で稼働確認済） | **7 ps RMS 達成、本プロジェクトの要件充足** |

### Step 3 の副次発見 (2026-04-23)

- **パルサー vs V1743 水晶の温度由来 ~65 min 周期うねり**（~1500 ppm pp）を 95 min long-run で発見。単一 V1743 内部の timing には無影響（同一水晶のため）だが、多台構成 (Step 4) では board 間ドリフトとして顕在化 → SinFanout で共通クロック分配する強い動機づけ。[memory](/Users/aogaki/.claude/projects/-Users-aogaki-WorkSpace-delila-rs/memory/v1743_pulser_clock_beat.md)
- **旧 TimestampTracker の host-clock safety net は長時間ランで害になる**ことを shadow validation で実測確認 → 「ソフト時刻で hw 時刻を補正しない」原則を確立し PSD1/PHA1 も新 RolloverTracker に完全移行。[memory](/Users/aogaki/.claude/projects/-Users-aogaki-WorkSpace-delila-rs/memory/layering_principle_clock_sync.md)

## 4. 要実機検証項目（SN:25 FW 依存）

- [x] `DataGroup[g].TDC` が 0 以外を返すか（Step 2 で確認、run 6001 で長時間正常進行）
- [x] TDC reset タイミング（SW Start で 0 にリセットされることを run 6001 で実測確認）
- [ ] **EVENT_MODE bit (header word 2 bit[24]) を 1 に設定しないと何が起きるか**（マニュアル: "customized SW では手動設定必須"）— 現状未設定で Standard mode 動作する
- [ ] `Charge` の値域・ゲート幅（Standard mode でも FW が計算、ゲートは固定値）— 現状は simple amplitude 採用で `Charge` 未使用
- [x] `SetChannelTriggerThreshold` + `SetTriggerPolarity` の per-channel 反映（Step 2 で確認済）
- [ ] `SinFanout` slave 動作（PSD Master との接続）— Step 4, HW 拡張待ち
- [x] ADC Calibration のタイミング（Configure 終端に移動済、Step 2 で確認）
- [ ] `SetRecordLength()` 削減時の実レート（1024 → 256 → 128 で計測）— 256 で 21 kHz @ run 6001 は確認、より短くは未測定
- [ ] BUSY LED / FULL 発生頻度（マニュアル Sec 10.11.1）— run 6001 では trigger loss 報告なし

## 5. Config スキーマ刷新

`X743Config` から `dpp_ci_*` 全削除。以下追加：

- `run_sync_mode`: `disabled` / `sin_fanout` / `trgout_sin_daisy`
- `clock_source`: `internal` / `daisy_in` / `external`
- `energy_source`: `lib` / `soft`（波形再積分）
- `energy_scale`, `energy_offset`
- `save_waveform`: per-channel override 可能
- `record_length`: 既存。rising edge 補間に必要な長さ維持しつつ最小化（例 128-256）で buffer depth 拡張
- **`fine_time_source`: `pos_edge` / `neg_edge` / `cfd_soft`** — 時間分解能戦略選択
- `cfd_delay_ns`, `cfd_fraction` — `cfd_soft` 選択時のみ使用

**既存 `correction_level` のデフォルトを `"all"` に固定**（5 ps RMS 時間分解能のため必須）。
`"disabled"` / `"pedestal"` 設定時は起動時 warn を出す。

`FirmwareType::X743CI` は schema 互換のため残すが、Reader はロード時に warn し Standard mode にフォールバック。

**Reader 初期化必須項目**: マニュアル Sec 10.10.3.1 より、自前 SW では EVENT_MODE bit (header word 2 bit[24])
を 1 に手動設定しないとデコードが狂う。`CAEN_DGTZ_WriteRegister` で Board Config レジスタの該当 bit を立てる。

## 6. 撤退する資産

- `src/reader/caen_legacy/handle.rs::apply_config_dpp_ci()` — 削除
- `set_dpp_parameters` (DPP_CI_Params_t 版) — 削除
- `DppEventBuffer` 及び `malloc_dpp_events` / `get_dpp_events` DPP-CI 経路 — 削除
- `set_dpp_acquisition_mode`, `set_dpp_event_aggregation`, `set_dpp_pre_trigger_size` — 使用停止
- `docs/x743_dpp_ci_parameters.md` — Archive / 冒頭に撤退理由明記
- `src/bin/x743_ci_probe.rs` — API 互換性検証用として保持（実機デバッグで有用）

## 7. 移行計画

1. ブランチ: `v1743-standard-redesign`
2. Step 1-2 を 1 PR にまとめる（削除 + Standard mode 実装）。実機 SN:25 で動作確認してから merge
3. Step 3 (BTT) は別 PR、実機で長時間 run 確認後
4. Step 4-5 は 2 台目導入後（ハードウェア依存）
5. Step 6 は physics calibration 後

## 8. Pitfalls (Gemini + DELILA 既知)

1. イベント順序非保証 → Merger 側ソート前提
2. 3 台 × 高 rate のスループット（USB/Optical/CPU 律速）→ `save_waveform` selective
3. FW バージョン依存性（TDC/Charge float の有効性）→ 実機確認
4. PSD Master S_OUT の fanout 負荷増加
5. S_IN レベルトリガ問題（PSD1 で経験） → arm_all_sync 徹底
6. 1 台エラーの全体同期崩壊 → ReadLoop transient retry + arm 時 fatal 停止
7. ADC calibration は Configure 終端で実行（Arm ブロックだと 650ms 遅延）

---

## 関連
- [TODO/45_v1743_support.md](45_v1743_support.md) — Phase 1 (FFI + 接続 + Standard mode 基礎) 完了。Phase 2 以降は本 TODO で置換
- [docs/plans/x743_standard_mode_design.md](../docs/plans/x743_standard_mode_design.md) — 詳細設計
- [docs/plans/x743_integration.md](../docs/plans/x743_integration.md) — 2026-02-19 の初期設計（DPP-CI 撤退部分は superseded）
- [docs/x743_dpp_ci_parameters.md](../docs/x743_dpp_ci_parameters.md) — DPP-CI パラメータ（撤退、User Manual Fig 10.9 と矛盾）
- `Setting_CHARGE_MODE_for_the_x743_modules` — CAEN App Note（実装バグ発見の根拠。CAEN 著作物のためリポジトリには非同梱、CAEN サイトから入手）
- **[legacy/UM2750_V1743_User_Manual_rev5.pdf](../legacy/UM2750_V1743_User_Manual_rev5.pdf)** — **決定的権威ソース (Sec 10.10.2, 10.10.3, Fig 10.8-10.9)**
