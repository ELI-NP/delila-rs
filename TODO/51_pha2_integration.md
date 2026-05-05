# TODO 51: PHA2 (DPP-PHA on x2730) firmware integration

**Status:** Phase 1〜5 ほぼ完了 (2026-05-05)。Phase 4 surface (UI) ✅ 完了 (`b258ab0`), Phase 5 docs ✅ 完了 (`6ba6cea`)。残: 校正源テスト (ハード待ち) + Phase 4.5 probe_type cross-cutting 拡張 (設計合意済、実装未着手)
**Created:** 2026-05-04
**Hardware:** VX2730 SN:52622 @ 172.18.4.56, FwType=`DPP_PHA`, FPGA FW 1.0.88, 32 ch, 500 MS/s, 14-bit
**DevTree (live):** [docs/devtree_examples/vx2730_pha2_sn52622.json](../docs/devtree_examples/vx2730_pha2_sn52622.json)
**Reference docs:** [legacy/PHA2_Parameters/](../legacy/PHA2_Parameters/) (CAEN FELib doxygen build 2026-03-02 — `a00107=Commands`, `a00108=Endpoints`, `a00109=Parameters`)

## Phase 1 完了サマリ (2026-05-04)

実機 (172.18.4.56) で **Idle → Configure → Arm → Start → Stop → Configure** が回り、生バイトが pipeline を流れて Recorder が `.delila` に書くところまで確認:

- Configure: 159 DevTree params 適用、全コンポーネント Configured 遷移成功
- Start (run 1): 5 秒で 1015 fake events、~100 events/s
- Stop: 即時 EOS、Reader → Merger → Recorder → Monitor すべて Configured へ正常復帰
- Recorder 出力: `data/run0001_0000_data_*.delila` 76 KB / 1864 events
- 副次確認: `caen_info` で `GateLongLengthT` は CAEN error -6 (PHA2 に存在しない) — PSD-only field 排除の正しさ実証

実装変更:
- `FirmwareType::PHA2`, `SourceType::Pha2` 追加
- `DecoderKind::Pha2`, `ReaderConfig::from_config` PHA2 マッピング
- `to_caen_parameters()` に PHA2 独立ブランチ (共通 46 ch param + PHA2 専用 19 trapezoidal-filter param)
- `BoardConfig.group_input_delay: Option<Vec<u16>>` plumb (16-VGA-group)
- `ChannelConfig` に PHA2 trap-filter 14 field 追加 (`time_filter_*`, `energy_filter_*`, `sin_function`, `gpi_function`)
- `Pha2Decoder` (dummy): aggregate header の byte order が PSD2 と異なる可能性が判明 (`30 00 00 03 ... 04` for Start signal — BE candidate) → Phase 1 では size-only classify で回避、Phase 2 で確定検証
- `config/digitizers/pha2_56.json` + `config/config_pha2_56.toml`
- 既存 550 unit test + 5 新規 PHA2 dummy test 全 PASS, clippy `-D warnings` 緑

Phase 2 へ持ち越す課題:
- **wire byte order**: PHA2 観測データ (`30 00 00 03 ...`) の最初の word は LE 解釈で `0x0400_0000_0300_0030` (top 4 bits = 0x0)、BE 解釈で `0x3000_0000_0300_0004` (top 4 bits = 0x3 = Start type)。CAEN spec は BE だと言っているが PSD2 decoder は LE を使用。Phase 2 の最初の作業として実 PSD2 データで検証して結論を出す
- **n_events 1 per Start signal**: FELib は Start signal にも `n_events=1` を返す。Phase 2 decoder は special event を event stream に混ぜない (Gemini レビューに沿う)

## Phase 2 完了サマリ (2026-05-04)

**実機 (172.18.4.56) で本物 decoder が 10 kHz TestPulse を 100% 取り切れた。**

### 主要成果
- **Wire byte order 確定**: PSD2 decoder は実は `from_be_bytes` を使っている (psd2.rs:674)。Phase 1 で誤読していた。CAEN spec 通り BE で正しい。新 `pha2.rs` も BE を採用
- **本物 PHA2 decoder 実装** (`src/reader/decoder/pha2.rs`, ~480 行) — psd2.rs を base に:
  - aggregate header (4-bit type / 16-bit counter / 32-bit total_size) は同一
  - per-event 1st word (channel + special + 48-bit TS) は同一
  - per-event 2nd word: bit63=last, bit62=wave, flags_low(12)+flags_high(8) は同一、**bits[41:26] は PHA2 で未使用** → `energy_short=0` ハードコード
  - waveform extras header: PHA2 固有 probe-type 情報を含むが、Phase 2 では parse して破棄 (Phase 4 で `EventData` 拡張時に活用)
  - waveform sample 32-bit packing (AP0[0:13]+DP0+DP1+AP1[16:29]+DP2+DP3) は PSD2 と完全一致
  - special event filter, single-word event, fine_ts 範囲チェック (1024 超え warn + clamp once-shot), 48-bit TS rollover 耐性

### 実機検証結果 (run 2, 48.7 秒)
- TestPulse 100µs period (期待値 10 kHz) → **9992 events/s 実測** (100% 取りこぼしなし)
- 487,301 events を `.delila` に記録、checksum 一致、footer complete
- 時刻範囲 83,618 ns → 48,730,083,621 ns (= 487,300 個の 100µs 間隔と一致 = TS 連続性 OK)
- Reader log: DECODE MISMATCH 0 件、fine_ts clamp warning 0 件、out-of-range channel 0 件
- ハードウェア trigger loss 2/450,904 = 0.00% (FW 内部のもの、decoder 由来ではない)
- Monitor histogram populated: ch0 energy in [0, 30] (TestPulse は内部生成で物理入力に注入されないため baseline noise)

### 実装成果物
- `src/reader/decoder/pha2.rs` 新規 (~480 行 + 9 unit test)
- 9 unit test 全 PASS:
  - classify_unknown_for_tiny_data
  - classify_start_signal_real_bytes (実機キャプチャした 32-byte Start signal で検証)
  - decode_single_event_basic
  - decode_zero_event_aggregate_header_only (キープアライブ aggregate 耐性)
  - pile_up_flag_propagated_in_flags_field
  - fine_ts_at_max_does_not_clamp (boundary 1023)
  - ts_rollover_synthetic_does_not_panic (48-bit TS 0xFFFF_FFFF_FFFE 跨ぎ)
  - special_event_filtered_out (bit55 set → drop)
  - reset_for_new_run_clears_state
- 既存 550 + 新規 5 = 555 unit test 全 PASS
- `cargo clippy --release -- -D warnings` 緑

### Phase 3 / Phase 4 へ持ち越す課題
- **probe-type 情報の `EventData` 露出**: PHA2 waveform extras header に含まれる analog probe type (3 bit) + is_signed (1 bit) + multiplication factor (2 bit) と digital probe type (4 bit) を `EventData` に追加し、UI/オフライン解析で使えるようにする。Phase 4 で全 FW + wire format + ROOT に波及する cross-cutting change として処理
- **analog_probe_is_signed フラグ**: 現状は default false (PSD2 と同じ)。EnergyFilter / TimeFilter / EnergyFilterBaseline 系は signed なので、probe-type を読み取って正しく設定する必要あり (Phase 4 で実施)
- **TestPulse → 物理入力ループバック検証**: 物理パルサー or 校正源でエネルギ peak が立つことを確認 (Phase 3)
- **設定ループバック検証 (Gemini レビュー#5)**: `apply_digitizer_config` で `energyfilterrisetimet` 等の applied 値ログ — Phase 3 で `apply` の中で実装

## Phase 3 進捗サマリ (2026-05-04)

### ✅ 完了

**3-1: 物理パルサーテスト + energy resolution 測定**
- 設定: [config/config_pha2_56_phys.toml](../config/config_pha2_56_phys.toml) + [config/digitizers/pha2_56_phys.json](../config/digitizers/pha2_56_phys.json) — global=`UserTrg`, ch0 self-trigger, polarity=Positive, dc_offset=20%, 波形ON
- 結果: 1 kHz physical pulser 入力で **FWHM/Center = 6.4 / 6327.6 = 0.101%** (Gauss fit Sigma=2.7, Area=193090, χ²/ndf=4.32)
- 観察: Analog 0 (ADCInput) は ~1.5k ADC 振幅 step, Analog 1 (EnergyFilter) は ~6.5 µs で trapezoid peak 到達
- 今後: 校正源 (60Co/137Cs) でフォトピーク確認は別セッション (要 source + シールド + 適切な PMT/scintillator)

**3-2: Decoder bug 発見 + 修正(致命傷)**
- スイープ実機テストで decoder 警告 `[PHA2] invalid waveform header — skipping waveform` を確認
- 原因究明用に [src/reader/decoder/pha2.rs](../src/reader/decoder/pha2.rs) に **fault-dump 機能** を追加 (one-shot per run、頭 256 word + boundary probes を warn ログに出す)
- 実バイトキャプチャで真因確定: **PHA2 firmware は bad state で waveform を truncate する**
  - 例: `wf_size` word は 200 を announce するが、実際は 5-22 sample words のみ出力 → そのまま次 event の `wf_header` に飛ぶ
  - 旧 decoder は wf_size を信じて 200 word 食う → 次 event の bytes を sample として誤読 → アグリゲート全体 desync → `events.len()` が 25 倍以上に膨張、out-of-range channel 警告連発
- **修正** ([src/reader/decoder/pha2.rs](../src/reader/decoder/pha2.rs#L486)):
  - sample loop 中に **wf_header pattern (bit63=1, bits[62:60]=0) を検出** → truncation 確定
  - `word_index -= 2` で次 event の first_word 位置に rewind (FW から見た first/second は既に sample として読まれているので、analog/digital probe arrays の末尾 4 sample = 2 sample word を drop)
  - sample data は AP1=14-bit + DPバイトで `bit63=0` 必須なので false positive なし
- **検証**:
  - 10/10 unit test PASS (新規 `fw_truncated_waveform_resyncs_to_next_event` 含む)
  - 実機ストレステストで 12 件の truncation を warn ログで正しく回復(旧 "invalid waveform header" は 1 件のみ)
  - `cargo clippy --release -- -D warnings` 緑

**3-3: FW state スタック現象 (decoder では fix できない別問題)**
- 連続 Configure cycle で stress すると、FW が "low rate output" 状態に陥る (期待 50 kHz → 実測 1-2 kHz、しばしば events=0)
- 一度入ると Stop+Reset+Configure では復旧しない
- **回復メカニズム**: 数分のアイドル放置で自然回復(実証: ストレステスト後 ~5 分 idle で 1 kHz テストが 3141 events / 3000 期待で完全復旧)。DAQ バイナリ再起動は不要
- 運用への含意:
  - 通常 production (1 Configure → 長時間 Run): **問題なし**
  - Throughput sweep (rapid Configure cycles): **iter 間に十分な cool-down (≥30s) が必要**
  - 通常の FW 切り替え/設定変更: 問題なし
- 将来的に `ApplyConfigRunning` (Configure cycle 不要) で rate のみ変更する版を作るとさらに安定

**3-4: Throughput 限界(単発 fresh start での実力)**
| rate | samples | events / duration | bytes/ev | loss | 備考 |
|---:|---:|---:|---:|---:|---|
| 10 kHz | 400 | 101,206 / 10s | 1631 | 0 | 100% delivery |
| 50 kHz | 400 | 506,136 / 10s | 1631 | 1 | 100% delivery |
| 100 kHz | 400 | 734,005 / 10s | 1487 | 1 | 73% delivery (FW limit) |
| 200 kHz | 1000 | 486,399 / 8s | 1800 | 0 | 30% delivery (FW limit) |
| 500 kHz | 200 | 1,182,560 / 8s | 740 | 0 | **decoder 0 errors @ 147k/s 持続** |

Decoder 単独は 500 kHz/200 samples ストレスでも完全動作。**FW の rate cliff は ~70-100 kHz/board 付近**(PSD2 の ~43 kHz より高い)。安全運転 ~30 kHz/board は OK。

### ✅ Phase 3 残作業 — 2026-05-04 後半に追加完了

**3-5: apply config loopback logging (Gemini #5)** — `512368c`
- `verify_loopback`: set_value 後に同 path を get_value して FW 適用値と比較
- `values_equivalent`: f64 1 ppm tolerance + case-insensitive string compare
- 範囲 broadcast path (`/ch/0..N/par/foo`) は読み取り不可のため skip
- Mismatch は INFO ログ + サマリ行に `loopback_mismatches=<N>` カウンタ + 詳細リスト
- `applied_value` フィールド (REST `/api/status` の history) に FW 報告値を反映
- 実機 PHA2 SN:52622: 45 params Apply、0 mismatches (DevTree validation で既に正しく snap されているため想定通り)
- 21/21 unit test PASS

**3-6: `.delila` → ROOT 出力** — `e423692`
- 新 binary `src/bin/delila_to_root.rs` (`cargo run --release --features root --bin delila_to_root`)
- 既存 C++ `tools/delila2root` は AMax 以前の wire format (6 scalar fields) を期待するため、`user_info[4]` が増えた現行 .delila ではパース失敗していた → Rust 側 DataFileReader + EventDataBatch path で実装
- 1 event = 1 TTree row、scalar branches: Module / Channel / TimestampNs / Energy / EnergyShort / Flags / UserInfo0..3 / HasWaveform
- Waveform は skip (event_builder + recover 系で対応)
- 実機検証 (PHA2 ch0 物理パルサー): 220 MB .delila → 344 KB root, 5950 events, **Mean=6317.14, StdDev=2.61, FWHM≈6.13** (Monitor の Gauss fit 6.4 と一致 ✓)

**3-7: Throughput sweep 堅牢化** — `a3aa0be`
- `--healthy-pct` (default 30%) で iter を OK/STUCK? 判定
- STUCK 時は `--stuck-cool-down` (default 60s) idle 後 `--max-retries` (default 1) 回 retry
- Healthy iter 間も `--cool-down` (default 5s) で FW を settling
- CSV 列追加: `expected_total`, `pct_of_target`, `healthy`, `attempt`
- End-of-run summary: `<N healthy> / <M stuck after retries> / <R retries triggered>`
- 実機検証: 36 iter / 32 healthy / 4 stuck / 9 retries triggered

### ⏳ Phase 3 残: 校正源テストのみ

8. **校正源 (60Co/137Cs) フォトピーク確認** — ハード準備待ち
   - 物理パルサーは FWHM 0.101% で確認済み
   - 校正源は線量計許可 + 検出器接続が必要なため別セッション

### 注意点
- PHA2 と PSD2 は同一ハード SN:52622 を共有するため、firmware 切り替えは flash 焼き直しが必要 (~5 分 + 設定再 apply)
- Phase 3 中に PSD2 ランも回したい場合は `start_daq.sh` の前に flash 確認

### Phase 3 で生まれた成果物 (commits)

- `fb8f66e` — feat(pha2): integrate DPP-PHA firmware on x2730 (Phase 1+2+3)
  - [src/reader/decoder/pha2.rs](../src/reader/decoder/pha2.rs) decoder + fault-dump + truncation resync
  - [config/digitizers/pha2_56*.json](../config/digitizers/) + corresponding TOMLs (TestPulse, 物理入力, throughput)
  - [scripts/throughput_sweep.py](../scripts/throughput_sweep.py) `--json-path` PSD2/PHA2 対応
  - [scripts/pha2_transition_stress.py](../scripts/pha2_transition_stress.py) Configure-cycle stress reproducer
- `512368c` — feat(caen): loopback-verify every applied parameter (Gemini #5)
  - [src/reader/caen/handle.rs](../src/reader/caen/handle.rs) `verify_loopback` + `values_equivalent`
- `a3aa0be` — feat(scripts): robustify throughput_sweep with cool-down + auto-retry
  - [scripts/throughput_sweep.py](../scripts/throughput_sweep.py) `--cool-down`/`--stuck-cool-down`/`--max-retries`/`--healthy-pct`
- `e423692` — feat(bin): add delila_to_root — flat TTree exporter (oxyroot)
  - [src/bin/delila_to_root.rs](../src/bin/delila_to_root.rs) PHA2/AMax 対応 .delila → ROOT

## ゴール

VX2730 に flash された **DPP-PHA (PHA2) firmware** を delila-rs のネイティブ FW として組み込む。trapezoidal-filter MCA としての普通の運用 (config → arm → start → stop → record) が回ることを目標にする。AMax (DPP_OPEN custom FW) と違い CAEN 純正 PHA なので OpenDPP endpoint ではなく **RAW endpoint + ホスト側デコーダ** で扱う (PSD2 と同じ路線)。

## DevTree 調査結果サマリ

PSD2 (同一ハード SN:52622) との差分:

| 領域 | 共通 | PHA2 only | PSD2 only |
|---|---:|---:|---:|
| `/par` (global) | 94 | 0 | 5 (IPE pulser: `ipeamplitude`, `iperate`, `ipebaseline`, `ipedecaytime`, `ipetimemode`) |
| `/ch/N/par` | 46 | **19** | 25 (PSD-specific: gates, charge integrator, CFD, etc.) |
| `/cmd` | 9 | 0 | 0 |
| `/endpoint` | RAW + Stats + ActiveEndpoint | `dpppha` | `dpppsd` |
| top-level extras | — | `/group/N/par/inputdelay` (16 group VGA) | — |

### PHA2-only 19 channel params (本実装の中核)

**Time filter:** `timefilterrisetimet/s` (16–500 ns, step 2), `timefilterretriggerguardt/s` (0–8000 ns, step 8)

**Energy filter (trapezoid):**
- `energyfilterrisetimet/s` (16–13000 ns, step 8, default 5000)
- `energyfilterflattopt/s` (32–3000 ns, step 8, default 1000)
- `energyfilterpolezerot/s` (32–131000 ns, step 2, default 50000)
- `energyfilterpeakingposition` (10–90 %, step 1, default 50)
- `energyfilterpeakingavg` (LowAVG/MediumAVG/HighAVG, default LowAVG)
- `energyfilterbaselineavg` (Fixed/VeryLow/Low/MediumLow/Medium/MediumHigh/High, default Medium)
- `energyfilterbaselineguardt/s` (0–8000 ns, step 8)
- `energyfilterpileupguardt/s` (0–64000 ns, step 64, default 240)
- `energyfilterfinegain` (1.000–10.000, step 0.001, default 1.000)
- `energyfilterlflimitation` (On/Off, default Off)

**per-channel S_IN/GPI:** `sinfunction`/`gpifunction` (None/ResetTimestamp) — PSD2 はグローバルのみ。MVP では default (None) のまま放置でよい。

### PSD2 でしか使えない field (PHA2 では送らない)
`gatelong/short/offset(t/s)`, `cfddelays/t`, `cfdfraction`, `chargesmoothing`, `longchargeintegratorpedestal`, `shortchargeintegratorpedestal`, `eventneutronreject`, `triggerfilterselection`, `pileupgap`, `triggerhysteresis`, `absolutebaseline`, `adcinputbaselineavg/guards/t`, `energygain`, `cfddelay*` …

### Event format (CAEN 公式 dpppha endpoint spec)

- 64-bit BE 4 ワード aggregate ヘッダ + Individual Trigger Mode (`format=0x2`) — **PSD2 と同一構造**
- per-event 1st word: ch (7 bit) + special (1 bit) + 48-bit TS、bit63=last header
- per-event 2nd word: `ENERGY (16)` + `FLAGS_LP (12)` + `FLAGS_HP (8)` + `FINE_TS (10) (1 LSB = 7.8125 ps)` + bit62=waveform present
- **Energy 一発のみ** (PSD2 の `charge_long`/`charge_short` ペアではない) → `EventData::energy_short = 0` 固定で運用
- Waveform レイアウトは PSD2 と完全一致 (AP0[0:13]+DP0+DP1+AP1[16:29]+DP2+DP3、32-bit per 2 sample groups)
- Stop Run = 3 ワード, Start Run = 4 ワード (special event)
- Extra word: Time (bit55) と Waveform (bit62) の二系統
- Probe 種別: 5 アナログ (ADCInput / TimeFilter / EnergyFilter / EnergyFilterBaseline / EnergyFilterMinusBaseline) × 13 デジタル (Trigger / TimeFilterArmed / RetriggerGuard / EnergyFilterBaselineFreeze / EnergyFilterPeaking / EnergyFilterPeakReady / EnergyFilterPileUpGuard / EventPileUp / ADCSaturation / ADCSaturationProtection / PostSaturationEvent / EnergyFilterSaturation / SignalInhibit)

## コード側の状況 (Phase 1 + 2 + 3 部分完了後 — 2026-05-04)

| 場所 | 状態 |
|---|---|
| [src/config/digitizer.rs](../src/config/digitizer.rs) `FirmwareType::PHA2` | ✅ 追加済 |
| [src/config/mod.rs](../src/config/mod.rs) `SourceType::Pha2` | ✅ 追加済 (alias `"PHA2"`/`"pha2"`) |
| [src/reader/mod.rs](../src/reader/mod.rs) `DecoderKind::Pha2` | ✅ 追加済 |
| [src/reader/decoder/pha2.rs](../src/reader/decoder/pha2.rs) | ✅ 実装済 (~580 行 + 10 unit test、Phase 3 で fault-dump + truncation resync 追加) |
| `to_caen_parameters()` PHA2 ブランチ | ✅ 独立ブランチ追加 (PSD2 と分離) |
| [src/operator/routes/digitizer.rs](../src/operator/routes/digitizer.rs) | ✅ `DPP_PHA` → `FirmwareType::PHA2` 検出 + default config 追加 |
| `BoardConfig.group_input_delay` | ✅ 追加済 (16-VGA-group, default `None`) |
| `ChannelConfig` PHA2 trap-filter fields | ✅ 14 field 追加 |
| `config/digitizers/pha2_56.json` | ✅ 作成済 (TestPulse 設定) |
| `config/config_pha2_56.toml` | ✅ 作成済 |
| `config/digitizers/pha2_56_phys.json` | ✅ 作成済 (物理入力 ch0 Positive、Phase 3) |
| `config/config_pha2_56_phys.toml` | ✅ 作成済 (Phase 3) |
| `config/digitizers/pha2_thrput.json` | ✅ 作成済 (throughput sweep、Phase 3) |
| `config/config_pha2_thrput.toml` | ✅ 作成済 (recorder 抜き、Phase 3) |
| `scripts/throughput_sweep.py` | ✅ `--json-path` で PSD2/PHA2 切り替え対応 |
| `scripts/pha2_transition_stress.py` | ✅ Configure-cycle stress テスト用 |
| `apply_validated_parameters` set→get→log | ⏳ Phase 3 残 (Gemini #5) |
| oxyroot で `.delila` → ROOT 出力 | ⏳ Phase 3 残 |
| [web/operator-ui/src/app/models/types.ts](../web/operator-ui/src/app/models/types.ts) `FirmwareType` literal | ⏳ Phase 4 |
| [web/operator-ui/src/app/models/channel-params.ts](../web/operator-ui/src/app/models/channel-params.ts) | ⏳ Phase 4 |
| `EventData` に probe_type 追加 | ⏳ Phase 4 (cross-cutting change) |

## プラン (5 フェーズ)

### Phase 1 — 配線 (decoder なしで生バイト ZMQ 通過)

目的: FELib 経由で接続・configure・arm・start・stop が回り、生バイトが ZMQ pipeline に流れることを確認する。decode は dummy。

1. `FirmwareType::PHA2` 追加 (url_scheme=`dig2://`, includes_n_events=true, is_dig1=false, new()=32 ch)
2. `SourceType::Pha2` (serde alias `"PHA2"`/`"pha2"`) + `to_firmware_type()` + `Display`
3. `ReaderConfig::from_config` の match に `Pha2 → PHA2`
4. `DigitizerConfig::to_caen_parameters()` に **PSD2 と独立した PHA2 ブランチ**を追加 — 共通 46 channel param のみ書き出し、PSD-only は呼ばない
5. 暫定 `Pha2Decoder` を `DecoderKind::Pha2` に追加 — **「バイトを捨てる」ではなく「aggregate header だけ parse して n_events 数の fake EventData (固定 ch=0, monotonic TS, energy=0xDEAD) を emit」** する。理由: pipeline 観測性確保 (Monitor で fake spectrum が見えれば configure/arm/start が成功した動かぬ証拠)、Phase 2 差し替え時に EventData 出力経路を変更しない (regression risk 低)
6. **`BoardConfig.group_input_delay: Option<Vec<u16>>` を Phase 1 で plumb** (length=16, VGA group). 初期値 None で OK だが後方互換性のため field を先に通しておく — VX2730 の analog input は 2ch で VGA 共有のハード仕様、コインシデンス解析で必須になる
7. `config/digitizers/pha2_56.json` (PSD2 template から PSD-only field を除いて作る)
8. `config/config_pha2_56.toml` (port 9090, monitor 8081, type=`pha2`)
9. **検証:** `/start-daq config/config_pha2_56.toml` → Configured/Armed/Running、Monitor histogram で fake spectrum (energy=0xDEAD spike) が見える → Recorder で `.delila` に fake events が記録される → Stop で即時 EOS

### Phase 2 — Decoder 実装 (本丸)

1. `src/reader/decoder/pha2.rs` を新規作成、`psd2.rs` を base にする
   - aggregate header (4 ワード) はそのまま流用 (フォーマット同一)
   - per-event 2nd word のビットフィールド差し替え (Energy / Flags_LP / Flags_HP / FineTS)
   - Stop Run (3 word) / Start Run (4 word) special event は **`tracing::info!` でログ出力した上で event stream には混ぜず drop** (hot path 0.79 M ev/s を保つ。`acquisition_width` 等は config TOML から既に取れているので、ハードウェアの応答が一致しているかのアサーション材料にする)
   - 波形 extras header の probe-type デコード (5 analog × 13 digital)
   - `EventData::energy = 16-bit ENERGY`, `energy_short = 0`
2. **`EventData` 構造体拡張の検討**: `analog_probe_type: [u8; 2], digital_probe_type: [u8; 4]` を追加するか? PHA 解析で「波形がどの probe か」が後日必須になる (baseline freeze, pile-up の根本原因追跡)。ただしこれは PSD1/PSD2/PHA1/AMax 全 FW + ZMQ wire format + oxyroot ROOT スキーマに波及する cross-cutting change。**判断**: Phase 2 では PHA2 decoder 内部のみ probe_type を保持して、`EventData` 拡張は **Phase 4 (UI で probe を表示するタイミング)** で別タスクとして提案する。Phase 2 の段階で生数値を decoder のローカル log にだけ出して spec 準拠を確認する
3. **設定ループバック検証 (FWHM 影響対策)**: `apply_digitizer_config` で `energyfilterrisetimet` 等を set した直後に同 path を get し直して、FELib が丸めた値 (sample 単位) を `tracing::info!(applied_value=...)` でログ。Pole-Zero / Rise Time / Flat Top の丸め誤差はエネルギ分解能 (FWHM) に直結する。「なぜ FWHM が悪いか」を後で追えるように、実際に FPGA に書いた値を必ず記録に残す
4. `src/reader/decoder/mod.rs` で re-export
5. `src/reader/mod.rs` の `DecoderKind::Pha2` を本実装に差し替え
6. ユニットテスト (合成 hex blob → 期待 EventData) — psd2 既存テストパターンを移植 + 以下の **PHA2 固有エッジケース**:
   - **Zero-event aggregate**: header のみ (n_events=0) — CAEN がキープアライブで送ってくることがある。decoder が panic しないこと
   - **Pile-up flag 立ち時の ENERGY 値**: FLAGS_LP の pile-up bit が立った時、ENERGY は「最初のパルス値」か「不正値 (0xFFFF)」か → 実機ログで確認、両方を許容するパースに
   - **Fine TS 境界値**: 負の値 / 1024 超えが来たら **panic ではなく warning + clamp** (defensive parsing)
   - **48-bit TS rollover**: 計算上数日でオーバーフロー。ReorderBuffer がまたいだ時に panic しないテスト (合成 blob で `0xFFFF_FFFF_FFFF` 直前→直後の TS sequence)
   - **Start/Stop special event 連続**: special event だけ来るケースで decoder が安定動作
7. **検証:** 自己トリガまたは TestPulse を撃って TS 単調性 + energy 正値 + fine_ts ∈ [0, 1024) を確認 + 上記 5 個のエッジケースが unit test で緑

### Phase 3 — TestPulse + スループット sweep

1. `config/config_pha2_thrput.toml` (TestPulse トリガ) で sweep
2. Monitor histogram にスペクトラムが出ることを目視
3. Recorder で `.delila` 取得 → oxyroot で ROOT 出力試走
4. PSD2 と同等のスループット (saturation 43 kHz/board 付近) を確認

### Phase 4 — UI (Angular)

1. `web/operator-ui/src/app/models/types.ts`: `FirmwareType` に `'PHA2'` 追加
2. `web/operator-ui/src/app/models/channel-params.ts`: `PHA2_INPUT/TRIGGER/ENERGY/COINCIDENCE/WAVEFORM_PARAMS` 5 ブロック追加 — bound/step は DevTree から直接 (e.g. `energyfilter_rise_time_ns`: min=16, max=13000, step=8)
3. 波形 probe enum は DevTree allowedvalues から生成 (PSD2 とは別リスト)
4. `cd web/operator-ui && npm run build` → `dist/` 同梱コミット

### Phase 5 — Integration & docs

1. `/test-daq config/config_pha2_56.toml` で end-to-end PASS
2. `docs/digitizer_system_spec.md` / `docs/compass_devtree_mapping.md` に PHA2 列追加
3. このファイル `Status: COMPLETED` 更新 + CURRENT.md の最近完了に移動

## 注意点 / リスク

- **`energyfilter*` の T/S 二重定義:** PSD2 の `gate*T/S` 同様、両方書くと board 側でぶつかる。**T (nanoseconds) のみ書く**で統一 (PSD2 と同方針)
- **Energy 単位:** `EventData.energy: u16` は PSD2 でも `charge_long`、PHA1 でも `energy` を入れている既存規約 — そのまま流用、UI 側で意味付け
- **per-ch `sinfunction`/`gpifunction`:** Phase 1 では JSON 出さずに default `None` 固定でよい。Phase 4 で `BoardConfig` 経由で扱うか検討
- **`/group/N/par/inputdelay`:** VGA グループ単位の入力遅延 (16 group)。Phase 1 では無視、UI で要求が出たら追加
- **Time vs Waveform extras の取りこぼし:** bit55 (Time extra) と bit62 (Waveform extra) は別物。両方 parse できるように
- **PSD2 と decoder を共通化したくなる衝動:** やめる。KISS で `pha2.rs` を独立ファイルにして psd2.rs から copy + 編集。Rule of Three (3 つ目の同系 FW が来たら) で初めて共通化検討
- **`dpppha` 既製 endpoint への誘惑:** 使わない。AMax は OpenDPP を使っているが、CAEN 内部 C++ で `malloc` + 可変長 struct を返す方式 → Rust 側で再 alloc 強制 → HWM=0 / zero-copy / 4 worker 並列の利点が破壊される。RAW endpoint + Rust 自前 parallel decode の対称構造を貫く
- **Multi-board sync の正しい戦略 (再考結果, 2026-05-04):** TS オフセットの目標は「全ボード 0」ではなく **「Run ごとに不変な定数オフセット (±8 ns 以内)」**。PSD1 の 650 ms 問題は「変動する」ことが本質的なバグで、ゼロでないこと自体は問題ではなかった。
  - **正しい解 (PSD1 で実施済 + PHA2/PSD2 で踏襲):** ① ADC calibration を Configure 段階で完了 ② `hw_state` で全ボード本当に Armed を待つ ③ `StartSource=SINedge` で edge-trigger 同期 (PSD1 は level だったが PHA2/PSD2 は edge を選べる) ④ ファンアウト + 等長ケーブル star topology で master S_OUT を全 slave S_IN に分配 ⑤ 残る ±10 ns 程度の定数オフセットは coincidence calibration run で測定 → MongoDB 保存 → EB で吸収
  - **per-ch `sinfunction=ResetTimestamp` の評価:** acquisition 中に手動で TS をリセットする操作は **Run-to-Run の variability を意図的に注入する悪手**。Run Control state machine とも乖離、ELOG/MongoDB の run start メタと TS=0 原点が乖離、ReorderBuffer の monotonic 仮定が崩れる。**汎用的な使い道はほぼない**。`ChannelConfig.sin_function: Option<String>` を default `None` で通しておくだけで十分、UI 前面化不要

## 検証チェックリスト

Phase 1 完了 (✅ 2026-05-04):
- [x] `cargo build --release` 緑
- [x] `cargo test` 緑 (既存 550 + dummy 5 = 555 PASS)
- [x] `/start-daq config/config_pha2_56.toml` で Idle→Configured→Armed→Running→Stop が通る
- [x] Recorder で 1864 fake events が `.delila` に記録される

Phase 2 完了 (✅ 2026-05-04):
- [x] TestPulse 10 kHz で decoded events が ZMQ に出る (9992 events/s 100% 取得)
- [x] TS 単調性 OK (.delila footer Time Range 83,618 ns → 48,730,083,621 ns 連続)
- [x] fine_ts ∈ [0, 1024)、energy > 0
- [x] Stop イベント後の EOS 発行が即時 (Recorder で 105 ms 以内に file close)
- [x] DECODE MISMATCH 0 件、fine_ts clamp warning 0 件、out-of-range channel 0 件
- [x] 既存 550 + 新規 9 unit test 全 PASS、clippy `-D warnings` 緑

Phase 3 部分完了 (✅ 2026-05-04):
- [x] Monitor で energy histogram が見える (1 kHz physical pulser で narrow peak)
- [x] 物理パルサーで energy peak が立つことを確認 (FWHM/Center = 0.101%, Sigma=2.7, Center=6327.6)
- [x] スループット fresh-start で実測: 50 kHz 100% delivery / 100 kHz 73% / 500 kHz @samples=200 で 147k/s decoder 0 errors → **PSD2 (~43 kHz) より高い、安全運転 ~30 kHz は問題なし**
- [x] **Decoder bug 発見・修正 + 10 unit test PASS** (FW truncation resync) — 詳細は「Phase 3 進捗サマリ」参照
- [x] FW state スタック現象の原因究明 (rapid Configure cycles, 数分 idle で自然回復)

Phase 3 完了 (✅ 2026-05-04 後半):
- [x] Recorder の `.delila` を oxyroot で ROOT 化 (`delila_to_root` 新 binary, FWHM 6.13 ≈ Monitor fit 6.4)
- [x] `apply_validated_parameters` set→get→log loopback (Gemini #5)
- [x] Throughput sweep スクリプト堅牢化 (cool-down + auto-retry + healthy-pct 検出)

Phase 3 残作業 (ハード準備待ち):
- [ ] 校正源 (60Co/137Cs) でフォトピーク確認(物理パルサーは確認済み)

Phase 4 完了 (✅ 2026-05-04, commits `b258ab0` `d980d79` `7ed3285`):
- [x] Operator UI で source type=PHA2 を選んだ時に trap-filter param 群が表示される (`b258ab0`)
- [x] Tune Up モードで PHA2 の Apply が通る (FW-agnostic、`b258ab0` で同時に検証済)
- [x] PHA2 waveform extras header の `is_signed` ビットを decoder で読み取り `Waveform.analog_probe[12]_is_signed` に伝搬 (`7ed3285`)
- [x] `analog_probe_is_signed` を wf-extras header bit3 から自動設定 (TimeFilter / EnergyFilter / EnergyFilterMinusBaseline で signed=true) (`7ed3285`)
- [x] Channel-table clamp 表示 (yellow flash + re-emit、隠さない設計) (`d980d79`)

Phase 4.5 — probe_type cross-cutting 拡張 (設計合意済、follow-up):
- [ ] `EventData.Waveform` に `analog_probe_type: [u8; 2], digital_probe_type: [u8; 4]` 追加 (`#[serde(default)]` で BC、`user_info` 前例踏襲)
- [ ] PHA2 wf-extras header bits[2:0] / bits[8:6] (analog) + 4-bit digital probe type を decoder でパース
- [ ] 他 FW (PSD1/PSD2/PHA1/AMax/V1743) は当面 `0xFF`=Unknown を出す
- [ ] `delila_to_root` に `AnalogProbeType0/1, DigitalProbeType0..3` の TBranch 追加
- [ ] Frontend `waveform.component.ts` の "A0/A1/D0..D3" hardcoded label を probe_type ベースで動的化 ("A0: TimeFilter" 等)
- 設計: u8 + PHA2 spec を canonical (analog: 0=ADCInput, 1=TimeFilter, 2=EnergyFilter, 3=EnergyFilterBaseline, 4=EnergyFilterMinusBaseline, 0xFF=Unknown)
- A1 decoder + Waveform fields → A2 ROOT branches → A3 Frontend model の 3 commit に分割

Phase 5 完了 (✅ 2026-05-04, commit `6ba6cea`):
- [x] `docs/digitizer_system_spec.md` / `docs/compass_devtree_mapping.md` に PHA2 列追加
- [x] supported-digitizers ドキュメント + ベンチマーク結果 close-out

Phase 5 残:
- [ ] `/test-daq` PASS (校正源テスト後の最終確認に組み込む)

## 5/4 後半の bug fixes と 5/5 の re-hardening

5/4 後半に「Phase 3 で書いた decoder 修正そのものが回帰だった」事案 + 設定検証の silent 経路 + UI が clamp を隠す設計、の 3 つの根の深い問題が連続発覚。本日 (5/5) の作業はこれらの再発防止に注力した。

### 5/4 後半に判明したバグ (4 件)
- `e641e99` PHA2 decoder の「mid-loop wf-header truncation 検出」 が legitimate sample (DP4 + AP2 baseline) を hit して全イベントの波形後半を drop していた。`pha2_simple_test` (CAEN-only 最小クライアント) で wf_size=2048 / event-spacing=2052 を直接検証して FW truncation が幻だと確定 → revert
- `e45e0ec` `param_cache` が case-sensitive で DevTree=lowercase / emit=CamelCase の毎回 cache miss → unvalidated `set_value` 経路に fallthrough → silent clamp bypass
- `d980d79` channel-table clamp 表示を「隠す」設計を「黄色 flash + re-emit」に
- `7ed3285` PHA2 wf-extras header の per-probe `is_signed` bit を hardcode `false` のまま放置 → Time-filter probe の +8191 オフセット欠落で wrap

### 5/5 re-hardening (本ファイル commit 群):
- **D**: `pha2_56.json` の未コミット ad-hoc を revert + signed-probe 検証用は `pha2_56_signed_probe_test.json` に分離
- **E1** ([handle.rs](../src/reader/caen/handle.rs)): `apply_params_validated` の cache-miss 経路 (旧 `debug!` + `result.ok += 1`) を `info!` + 新 `ParamApplyStatus::NoCache` + 新カウンタ `result.no_cache` + サマリログ + 詳細リスト出力に変更。新規 unit test 2 本 (`unknown_param_name_misses_cache`, `apply_config_result_round_trips_no_cache_status`)。前者は cache 引きの contract、後者は新フィールドの serde BC を pin
- **E2** ([caen_simple_test.rs](../src/bin/caen_simple_test.rs)): `pha2_simple_test` を `caen_simple_test --firmware {pha2,psd2,pha1}` に汎用化。PHA2/PSD2 は FELib path 互換、PHA1 (DIG1) は早期 bail で stub 化
- **E3** ([decoder/mod.rs](../src/reader/decoder/mod.rs) + [CLAUDE.md](../CLAUDE.md)): decoder hot-path heuristic 禁止 policy を明文化。spec page reference 必須 + `caen_simple_test` 検証必須

### 手動検証 — channel-table clamp 表示 (`d980d79` の retention)

Cypress 自動化は follow-up。以下の手動手順を残す:
1. `cd web/operator-ui && ng serve` → Settings tab → PHA2 channel
2. record_length_ns に DevTree max を超える値 (e.g. 17000) を入力 → blur
3. 期待: 黄色フラッシュ + 値が DevTree max (16200) にスナップ + reflect immediately
4. 期待しない: 値が見かけ反映されたまま FW では prev value 維持 (5/4 以前のバグ)
5. Backend log を確認: `Validated configuration applied total=N adjusted=1 ... no_cache=0` が出ること (E1 の新サマリ列)

### Follow-up TODOs (本日 out-of-scope)

- Phase 4.5 probe_type cross-cutting 拡張 (上述、3 commit 想定)
- `caen_simple_test --firmware pha1` の DIG1 namespace 実装 (PSD1/PHA1 decoder bug 調査時に着手)
- AMax 用 simple-test (OpenDPP endpoint, 別 binary 検討)
- Cypress による channel-table clamp e2e
- Grafana cache-miss metric 連携 (`no_cache` counter を panel に出す)

## Gemini Pro 協議メモ (2026-05-04)

プラン全体は LGTM。以下、レビューで反映済の重要指摘:

| # | 指摘 | 反映先 |
|---|---|---|
| 1 | Decoder は copy-and-edit が正解 (Rule of Three 厳守) | Phase 2 + 注意点 |
| 2 | dummy decoder はバイト破棄ではなく fake monotonic event を emit (pipeline 観測性) | Phase 1 step 5 |
| 3 | ~~`sinfunction=ResetTimestamp` はマルチボード sync の銀の弾丸候補~~ → **2026-05-04 撤回**: Run-to-Run variability を入れる悪手と判明。正しい sync 戦略は「定数オフセット化 + offline calibration」(注意点に再記載) | 注意点 (修正版) |
| 4 | RAW endpoint を貫く (`dpppha` decoded endpoint は使わない) | 注意点 |
| 5 | T (ns) 送信後に get で読み戻して FPGA に書かれた sample 値をログ (FWHM 追跡用) | Phase 2 step 3 |
| 6 | `EventData` に probe_type 追加 — Phase 4 で cross-cutting change として別タスク化 | Phase 2 step 2 |
| 7 | エッジケーステスト 5 個 (zero-event aggregate, pile-up, fine TS 境界, TS rollover, special-only) | Phase 2 step 6 |
| 8 | `BoardConfig.group_input_delay` を Phase 1 で plumb (16 group VGA skew, コインシデンス必須) | Phase 1 step 6 |
| 9 | StartRun/StopRun special event は log + drop, event stream に混ぜない | Phase 2 step 1 |
| 10 | FW update 追従は CI 不要、マニュアル diff で十分 | (現方針維持) |

## 関連参照

- [docs/devtree_examples/vx2730_pha2_sn52622.json](../docs/devtree_examples/vx2730_pha2_sn52622.json) — live DevTree
- [docs/devtree_examples/vx2730_psd2_sn52622.json](../docs/devtree_examples/vx2730_psd2_sn52622.json) — 比較対象 (同一ハード)
- [legacy/PHA2_Parameters/](../legacy/PHA2_Parameters/) — CAEN 公式 doxygen (Commands/Endpoints/Parameters)
- [legacy/UM5678_725-730_DPP_PHA_Registers_rev2.pdf](../legacy/UM5678_725-730_DPP_PHA_Registers_rev2.pdf) — 古い x725/730 DPP-PHA レジスタマニュアル (DIG1, 参考)
- [legacy/WEB_UM5678_725-730_DPP_PHA_Registers_rev5.pdf](../legacy/WEB_UM5678_725-730_DPP_PHA_Registers_rev5.pdf) — 同上 rev5
- [src/reader/decoder/psd2.rs](../src/reader/decoder/psd2.rs) — Phase 2 のベース
- [src/reader/decoder/pha1.rs](../src/reader/decoder/pha1.rs) — PHA energy word 仕様の参考 (DIG1 だが同じ trap filter)
