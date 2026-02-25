# トリガーロス・ビジー検出 — DIG1 / DIG2 比較調査

**日付:** 2026-02-25
**Status:** DIG1 + DIG2 Phase 1 実機検証完了

---

## 1. 背景

一晩ランで一部デジタイザのタイムスタンプが途中で不連続になった問題を調査した結果:

- **真因**: 通信エラー時に Close+Open で再接続 → タイムスタンプカウンタがリセット → データ中に 0 に戻るタイムスタンプが混入
- **解決済**: ReadLoop transient error retry (10ms wait + retry、再接続しない) でドライバ回復を待つ方式に変更

残る課題: 通信エラーの 10ms リトライ中にトリガーが失われないかの検出方法。

---

## 2. DIG2 (PSD2, VX2730) — 統計カウンタが豊富

DIG2 は独立した統計エンドポイントとパラメータを持つ。

### 2.1 パラメータ (FELib `get_value()` で読める)

| パラメータ | パス | 説明 |
|-----------|------|------|
| `ChRealtimeMonitor` | `/ch/{ch}/par/chrealtimemonitor` | リアルタイム (ns, 48-bit) |
| `ChDeadtimeMonitor` | `/ch/{ch}/par/chdeadtimemonitor` | デッドタイム (ns, 524288ns 刻み) |
| `ChTriggerCnt` | `/ch/{ch}/par/chtriggercnt` | 受信トリガー数 (24-bit) |
| `ChSavedEventCnt` | `/ch/{ch}/par/chsavedeventcnt` | 保存イベント数 (24-bit) |
| `AcquisitionStatus` | `/par/acquisitionstatus` | bit0:Armed, bit1:Run, etc. |

**ロストトリガー = `ChTriggerCnt - ChSavedEventCnt`**

### 2.2 統計エンドポイント

`/endpoint/stats` に REAL_TIME, DEAD_TIME, LIVE_TIME, TRIGGER_CNT, SAVED_EVENT_CNT がまとまっている。

### 2.3 実装方針

定期ポーリング (数秒おき) で `ChTriggerCnt` / `ChSavedEventCnt` を読み、差分をメトリクスに出力。ReadLoop に影響なし。

---

## 3. DIG1 (PSD1/PHA1, DT5730B/VX1730B) — 制約あり

DIG1 には統計エンドポイント (`/endpoint/stats`) が**存在しない**。
DevTree (`docs/devtree_examples/dt5730b_psd1_sn990.json`) にあるエンドポイントは `/endpoint/raw` と `/endpoint/dpppsd` の 2 つのみ。

### 3.1 EXTRAS フラグ (現在の設定で取れる — 最有力)

現在強制している `ch_extras_opt = 2` (EX=010, `EXTRAS_OPT_TT48_FLAGS_FINETT`) の EXTRAS ワードフォーマット:

```
EXTRAS word (32-bit):
[31:16] = 拡張タイムスタンプ (48-bit の上位 16 ビット)
[15:10] = フラグ
  bit[15]: Trigger Lost     — ロスト直後の最初のイベントでセット
  bit[14]: Over-range        — ゲート内飽和 (クリッピング)
  bit[13]: 1024 trigger counted — 1024 イベントごとにハイ
  bit[12]: N lost trigger counted — N 回ロストごとにハイ
           (N はレジスタ 0x1n84 bits[17:16] で設定)
[9:0]   = fine timestamp (T_fine)
```

**48-bit タイムスタンプと排他ではない。現在の設定のままで使える。**

ソース: UM4380 Rev.6, p.108 (EX=010 の定義)

### 3.2 ch_extras_opt 全オプション一覧

| EX 値 | FELib 名 | EXTRAS 内容 | 48-bit TS |
|-------|---------|------------|-----------|
| 000 | `EXTRAS_OPT_TT48_BL4` | 拡張 TS + baseline×4 | Yes |
| 001 | `EXTRAS_OPT_TT48_FLAGS` | 拡張 TS + 16-bit flags | Yes |
| 010 | `EXTRAS_OPT_TT48_FLAGS_FINETT` | 拡張 TS + 6-bit flags + fine TS **(現在強制)** | Yes |
| 100 | `EXTRAS_OPT_LOSTTRG_TOTTRG` | Lost Trigger Counter [31:16] + Total Trigger Counter [15:0] | **No** |
| 101 | `EXTRAS_OPT_SBZC_SAZC` | CFD sample after/before zero crossing | **No** |

### 3.3 レジスタによるビジー検出

DIG1 にはトリガーカウンタやデッドタイムカウンタの専用レジスタは存在しない。
利用可能なステータスレジスタ:

| レジスタ | アドレス | 関連ビット | 説明 |
|---------|---------|----------|------|
| Acquisition Status | 0x8104 | bit[4] Event Full | バッファが FULL → トリガーロスト中 |
| | | bit[2] Running | アクイジション実行中 |
| | | bit[3] Event Ready | リードアウト可能なデータあり |
| Readout Status | 0xEF04 | bit[0] Event Ready | データ読み出し可能 |
| Channel n Status | 0x1n88 | bit[2] SPI busy, bit[8] ADC Power Down | チャンネル状態 (トリガー情報なし) |

`get_user_register(0x8104)` で bit[4] を監視すればバッファ Full (=ビジー) を検出可能。

### 3.4 DPP Algorithm Control (0x1n84) — N ロストトリガーカウンタ設定

EXTRAS bit[12] ("N lost trigger counted") の N はレジスタ 0x1n84 bits[17:16] で設定。
→ 要確認: N の値の意味 (2^N? N 直値?) と現在のデフォルト値。

### 3.5 Buffer Occupancy Gain (0x81B4) — 使用不可

VME ボード専用のアナログ出力 (LEMO MON/Sigma コネクタ) 用。DT5730B (USB/Desktop) では使用不可。

---

## 4. 比較まとめ

| 検出方法 | DIG1 (PSD1/PHA1) | DIG2 (PSD2) |
|---------|-------------------|-------------|
| トリガーロス (イベント内フラグ) | EXTRAS bit[15,12,13] — **デコード追加のみ** | 不要 (独立カウンタあり) |
| トリガーカウンタ (累積) | EXTRAS option 3 でのみ (48-bit TS と**排他**) | `ChTriggerCnt` / `ChSavedEventCnt` |
| バッファ Full (ビジー) | `0x8104 bit[4]` レジスタ読み | `AcquisitionStatus` パラメータ |
| デッドタイム | なし | `ChDeadtimeMonitor` |
| リアルタイム / ライブタイム | なし | `ChRealtimeMonitor` / 計算 |
| 独立統計 EP | **なし** | `/endpoint/stats` |

---

## 5. 実装計画

### Phase 1: DIG1 EXTRAS フラグデコード (最小コスト)

PSD1 デコーダ (`src/reader/decoder/psd1.rs`) で EXTRAS フラグを抽出。
設定変更不要、現在の `ch_extras_opt=2` のまま。

1. EXTRAS word の bits[15:10] からフラグを抽出
2. bit[15] (Trigger Lost) がセットされたイベント数をカウント → メトリクスに追加
3. bit[13] (1024 trigger counted) でおおよその総トリガー数を推定
4. ログ出力: `warn!` でトリガーロスト検出を通知

### Phase 2: DIG2 統計カウンタのポーリング

Reader の ReadLoop 内で数秒おきに `ChTriggerCnt` / `ChSavedEventCnt` を `get_value()` で読み取り。

### Phase 3: Acquisition Status レジスタ監視

DIG1: `get_user_register(0x8104)` で bit[4] (Event Full) を ReadLoop 内で監視。
バッファ Full 検出時に `warn!` ログ出力。

---

## 6. DIG1 実機検証結果 (2026-02-25)

**テストバイナリ:** `src/bin/trigger_loss_test.rs`
**ハードウェア:** DT5730B SN:990 (PSD1, ch4 パルサー ~10kHz), 172.18.4.147

| 項目 | Phase 1 (正常, 10s) | Phase 2 (1s delay, 30s) |
|------|-------------------|------------------------|
| イベントレート | 9,753 Hz | 1,798 Hz |
| 総イベント | 98,419 | 55,216 |
| Trigger Lost (bit[15]) | 0 | 26 |
| N Lost Counted (bit[12]) | 0 | 217 |
| 推定ロスト数 | 0 | 222,208 |
| Buffer Full 検出 | 0 | 27 |

**結論:** EXTRAS フラグ (bit[15], bit[12], bit[13]) はすべて正しく動作。設定変更ゼロで検出可能。
Waveform 有効 (record_length=1000, ~1KB/event) が必要 — waveform なしでは DT5730B バッファに余裕がありすぎて溢れない。

---

## 6b. DIG2 実機検証結果 (2026-02-25)

**テストバイナリ:** `src/bin/trigger_loss_test_dig2.rs`
**ハードウェア:** VX2730 SN:52622 (PSD2, ch16 パルサー ~10kHz), dig2://172.18.4.56
**Overrides:** record_length_ns=8192, waveform=enabled

| 項目 | Phase 1 (正常, 10s) | Phase 2 (1s delay, 30s) |
|------|-------------------|------------------------|
| TriggerCnt (FPGA) | 101,951 | 306,724 |
| SavedEventCnt (FPGA) | 101,968 | 109,294 |
| Lost (差分) | 0 | 197,430 |
| Loss rate | 0.00% | 64.37% |
| Deadtime (ch16) | 195.6 ms | 77,798 ms |

**結論:** ChTriggerCnt - ChSavedEventCnt で正確なトリガーロス検出が可能。ChRealtimeMonitor を先に読むことで FPGA カウンタがラッチされる。

---

## 7. 参照ドキュメント

| ドキュメント | 場所 | 内容 |
|------------|------|------|
| UM4380 Rev.6 | `legacy/UM4380_725-730_DPP_PSD_Registers_rev6.pdf` | DIG1 レジスタマップ (p.45: Acq Status, p.107-108: EXTRAS format) |
| x2730 DPP-PSD CUP Doc | `legacy/documentation_2024092000-2/` | DIG2 パラメータ・エンドポイント仕様 |
| DIG1 DevTree dump | `docs/devtree_examples/dt5730b_psd1_sn990.json` | DIG1 の実際のパラメータ・エンドポイント |
| DIG2 DevTree dump | `docs/devtree_examples/vx2730_psd2_sn52622.json` | DIG2 の実際のパラメータ・エンドポイント |

---

## 8. 注意事項

- **DIG1 の `/endpoint/stats` は存在しない**: 先行調査で DIG2 ドキュメントの情報を DIG1 に誤適用した経緯あり。DIG1 DevTree で確認済み。
- **EXTRAS bit[12] の N 設定**: レジスタ 0x1n84 bits[17:16] の確認が必要。FELib DevTree 経由でアクセスできるか要検証。
- **24-bit カウンタのラップアラウンド**: DIG2 の ChTriggerCnt/ChSavedEventCnt は 24-bit (16,777,215 でラップ)。長時間ランでは差分計算でラップを考慮する必要あり。
