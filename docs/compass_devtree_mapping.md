# CoMPASS → DevTree Parameter Mapping

**Created:** 2026-02-04
**Source:** CoMPASS screenshots (`legacy/CoMPASS/`) + 実機DevTree (`docs/devtree_examples/`)
**Status:** Confirmed with hardware

---

## PSD2 (VX2730, DPP_PSD, FW 1.0.57)

### Channel Parameters

| # | CoMPASS名 | DevTree名 | 型 | 範囲/選択肢 | SetInRun | UI実装 |
|---|---|---|---|---|---|---|
| 1 | Enable | `chenable` | STRING | True, False | Yes | 実装済 |
| 2 | Record Length | `chrecordlengtht` | NUMBER | 32-16200 ns | No | 実装済 |
| 3 | Pre-trigger | `chpretriggert` | NUMBER | 32-8000 ns | Yes | **新規** |
| 4 | Polarity | `pulsepolarity` | STRING | Positive, Negative | No | 実装済 |
| 5 | N samples baseline | `adcinputbaselineavg` | STRING | Fixed, Low, MediumLow, MediumHigh, High | No | **新規** |
| 6 | Fixed baseline value | `absolutebaseline` | NUMBER | 0-16383 ADC count | Yes | **新規** |
| 7 | DC Offset | `dcoffset` | NUMBER | 0-100 % | Yes | 実装済 |
| 8 | Waveform Downsampling | `wavedownsamplingfactor` | STRING | 1, 2, 4, 8 | No | **新規** |
| 9 | VGA Gain | `chgain` | NUMBER | 0-29 dB | Yes | **新規** |
| 10 | Discriminator mode | `triggerfilterselection` | STRING | LeadingEdge, CFD | No | **新規** |
| 11 | Threshold | `triggerthr` | NUMBER | 1-8191 ADC count | Yes | 実装済 |
| 12 | Trigger holdoff | `timefilterretriggerguardt` | NUMBER | 8-8000 ns | No | **新規** |
| 13 | CFD delay | `cfddelayt` | NUMBER | 2-2040 ns | No | 実装済 |
| 14 | CFD fraction | `cfdfraction` | STRING | 25, 50, 75, 100 | No | **新規** |
| 15 | Smoothing Factor | `smoothingfactor` | STRING | 1, 2, 4, 8, 16 | Yes | **新規** |
| 16 | Charge smoothing | `chargesmoothing` | STRING | Enabled, Disabled | Yes | **新規** |
| 17 | Time filter smoothing | `timefiltersmoothing` | STRING | Enabled, Disabled | Yes | **新規** |
| 18 | Energy coarse gain | `energygain` | STRING | x1, x4, x16, x64, x256 | No | **新規** |
| 19 | Gate (Long) | `gatelonglengtht` | NUMBER | 2-8000 ns | No | 実装済 |
| 20 | Short gate | `gateshortlengtht` | NUMBER | 2-8000 ns | No | 実装済 |
| 21 | Pre-gate | `gateoffsett` | NUMBER | 16-2000 ns | No | **修正必要** |
| 22 | Charge pedestal | `longchargeintegratorpedestal` | NUMBER | 0-1000 count | No | **新規** |
| 23 | Short charge pedestal | `shortchargeintegratorpedestal` | NUMBER | 0-1000 count | No | **新規** |
| 24 | Veto source (ch) | `channelvetosource` | STRING | Disabled, BoardVeto, ADCOverSaturation, ADCUnderSaturation | Yes | **新規** |
| 25 | Veto width (ch) | `adcvetowidth` | NUMBER | 0-524280 ns | Yes | **新規** |
| 26 | Ch trigger mask | `channelstriggermask` | STRING | hex mask | Yes | **新規** |
| 27 | Coincidence mask | `coincidencemask` | STRING | Disabled, Ch64Trigger, TRGIN, GlobalTriggerSource, ITLA, ITLB | Yes | **新規** |
| 28 | Anti-coincidence mask | `anticoincidencemask` | STRING | Disabled, Ch64Trigger, TRGIN, GlobalTriggerSource, ITLA, ITLB | Yes | **新規** |
| 29 | Coincidence window | `coincidencelengtht` | NUMBER | 0-524280 ns | Yes | **新規** |
| 30 | Event selector | `eventselector` | STRING | All, PileUp, EnergySkim | Yes | **新規** |
| 31 | Event trigger source | `eventtriggersource` | STRING | Disabled, Ch64Trigger, ChSelfTrigger, SwTrg, TRGIN, GlobalTriggerSource, LVDS, ITLA, ITLB | Yes | 実装済 |
| 32 | Wave trigger source | `wavetriggersource` | STRING | Disabled, Ch64Trigger, ITLA, ITLB, ChSelfTrigger, SwTrg, ADCOverSaturation, ADCUnderSaturation, ExternalInhibit, TRGIN, GlobalTriggerSource, LVDS | Yes | 実装済 |
| 33 | Wave saving | `wavesaving` | STRING | Always, OnRequest | Yes | **新規** |
| 34 | Analog Probe 0 | `waveanalogprobe0` | STRING | ADCInput, ADCInputBaseline, CFDFilter | Yes | **新規** |
| 35 | Analog Probe 1 | `waveanalogprobe1` | STRING | ADCInput, ADCInputBaseline, CFDFilter | Yes | **新規** |
| 36 | Digital Probe 0 | `wavedigitalprobe0` | STRING | Trigger, CFDFilterArmed, ADCSaturation, ADCInputNegativeOverthreshold, ReTriggerGuard, ADCInputBaselineFreeze, ADCInputOverthreshold, ChargeReady, LongGate, PileUpTrigger, ShortGate, ChargeOverRange | Yes | **新規** |
| 37 | Digital Probe 1 | `wavedigitalprobe1` | STRING | (同上) | Yes | **新規** |
| 38 | Digital Probe 2 | `wavedigitalprobe2` | STRING | (同上) | Yes | **新規** |
| 39 | Digital Probe 3 | `wavedigitalprobe3` | STRING | (同上) | Yes | **新規** |

### Board Parameters

| # | CoMPASS名 | DevTree名 | 型 | 範囲/選択肢 | SetInRun | UI実装 |
|---|---|---|---|---|---|---|
| 40 | Clock source | `clocksource` | STRING | Internal, FPClkIn | No | **新規** |
| 41 | Output clock | `enclockoutfp` | STRING | True, False | No | **新規** |
| 42 | SyncOut signal | `syncoutmode` | STRING | Disabled, SyncIn, TestPulse, IntClk, Run, User | Yes | **新規** |
| 43 | Start mode | `startsource` | STRING | EncodedClkIn, SINlevel, SINedge, SWcmd, LVDS, P0 | No | 実装済 |
| 44 | TRG OUT mode | `trgoutmode` | STRING | Disabled, TrgIn, Run, RefClk, TestPulse, Busy, UserTrgout, ... (23個) | No | **新規** |
| 45 | GPO mode | `gpiomode` | STRING | Disabled, TrgIn, SwTrg, Run, RefClk, TestPulse, Busy, ... (18個) | No | 実装済 |
| 46 | Start delay | `rundelay` | NUMBER | 0-524280 ns | No | **新規** |
| 47 | Veto source (board) | `boardvetosource` | STRING | Disabled, SIN, GPIO, LVDS, P0, EncodedClkIn | Yes | **新規** |
| 48 | Veto polarity | `boardvetopolarity` | STRING | ActiveHigh, ActiveLow | Yes | **新規** |
| 49 | Veto width (board) | `boardvetowidth` | NUMBER | 0-34359738360 ns | Yes | **新規** |
| 50 | Global trigger source | `globaltriggersource` | STRING | TrgIn, P0, TestPulse, UserTrg, SwTrg, LVDS, ITLA, ITLB, ITLA_AND_ITLB, ITLA_OR_ITLB, EncodedClkIn, GPIO | No | 実装済 |
| 51 | FPIO type | `iolevel` | STRING | NIM, TTL | No | **新規** |
| 52 | Test pulse period | `testpulseperiod` | NUMBER | 0-34359738360 ns | No | 実装済 |
| 53 | Test pulse width | `testpulsewidth` | NUMBER | 0-34359738360 ns | No | 実装済 |

---

## PSD1 (DT5730B, DPP_PSD, FW 136.22)

### Channel Parameters

| # | CoMPASS名 | DevTree名 | 型 | 範囲/選択肢 | SetInRun | UI実装 |
|---|---|---|---|---|---|---|
| 1 | Enable | `ch_enabled` | STRING | FALSE, TRUE | Yes | 実装済 |
| 2 | Pre-trigger | `ch_pretrg` | NUMBER | 40-2016 ns (expuom:-9) | No | **新規** |
| 3 | Polarity | `ch_polarity` | STRING | POLARITY_POSITIVE, POLARITY_NEGATIVE | Yes | 実装済 |
| 4 | N samples baseline | `ch_bline_nsmean` | STRING | BLINE_NSMEAN_FIXED, _16, _64, _256, _1024 | No | **新規** |
| 5 | Fixed baseline value | `ch_bline_fixed` | NUMBER | 0-16383 | Yes | **新規** |
| 6 | DC Offset | `ch_dcoffset` | NUMBER | 0-100 % | Yes | 実装済 |
| 7 | Input dynamic | `ch_indyn` | STRING | INDYN_2_0_VPP, INDYN_0_5_VPP | Yes | **新規** |
| 8 | Discriminator mode | `ch_discr_mode` | STRING | DISCR_MODE_LED, DISCR_MODE_CFD | Yes | **新規** |
| 9 | Threshold | `ch_threshold` | NUMBER | 0-16383 LSB | Yes | 実装済 |
| 10 | Trigger holdoff | `ch_trg_holdoff` | NUMBER | 0-524280 ns (expuom:-9) | Yes | **新規** |
| 11 | CFD delay | `ch_cfd_delay` | NUMBER | 0-510 ns (expuom:-9) | Yes | 実装済 |
| 12 | CFD fraction | `ch_cfd_fraction` | STRING | CFD_FRACTLIST_25, _50, _75, _100 | Yes | **新規** |
| 13 | Input Smoothing | `ch_cfd_smoothexp` | STRING | CFD_SMOOTH_EXP_1, _2, _4, _8, _16 | No | **新規** |
| 14 | Energy coarse gain | `ch_energy_cgain` | STRING | CHARGESENS_2.5/10/40/160/640/2560_FC_LSB_VPP | Yes | **新規** |
| 15 | Gate (Long) | `ch_gate` | NUMBER | 4-32766 ns (expuom:-9) | Yes | 実装済 |
| 16 | Short gate | `ch_gateshort` | NUMBER | 2-2046 ns (expuom:-9) | Yes | 実装済 |
| 17 | Pre-gate | `ch_gatepre` | NUMBER | 0-510 ns (expuom:-9) | No | 実装済 |
| 18 | Charge pedestal en. | `ch_pedestal_en` | STRING | FALSE, TRUE | No | **新規** |
| 19 | Self trigger enable | `ch_self_trg_enable` | STRING | FALSE, TRUE | Yes | **新規** |
| 20 | Global trigger gen | `ch_trg_global_gen` | STRING | FALSE, TRUE | Yes | **新規** |
| 21 | Trigger output propagate | `ch_out_propagate` | STRING | FALSE, TRUE | Yes | **新規** |
| 22 | Veto source | `ch_veto_src` | STRING | VETO_SRC_DISABLED, _COMMON, _INDIVIDUAL, _SATURATION | Yes | **新規** |
| 23 | Pileup rejection en. | `ch_pur_en` | STRING | FALSE, TRUE | Yes | **新規** |
| 24 | Coincidence mode | `ch_trg_mode` | STRING | TRIGGER_MODE_NORMAL, _COINC, _ANTICOINC | No | **新規** |

### Board Parameters

| # | CoMPASS名 | DevTree名 | 型 | 範囲/選択肢 | SetInRun | UI実装 |
|---|---|---|---|---|---|---|
| 25 | Record length | `reclen` | NUMBER | 16-131056 samples | No | 実装済 |
| 26 | Ext clock source | `dt_ext_clock` | STRING | FALSE, TRUE | Yes | **新規** |
| 27 | Start mode | `startmode` | STRING | START_MODE_SW, _S_IN, _FIRST_TRG | No | 実装済 |
| 28 | TRG OUT/GPO mode | `out_selection` | STRING | OUT_PROPAGATION_LEVEL0, _LEVEL1, _SYNCIN, _TRIGGER, _RUN, _DELAYED_RUN, _SAMPLE_CLK, _PLL_CLK, _BUSY, _PLL_UNLOCK, _VPROBE | No | **新規** |
| 29 | Start delay | `start_delay` | NUMBER | 0-4080 samples | Yes | **新規** |
| 30 | Coincidence window | `coinc_trgout` | NUMBER | 0-8184 samples | Yes | **新規** |
| 31 | FPIO type | `iolevel` | STRING | FPIOTYPE_NIM, FPIOTYPE_TTL | Yes | **新規** |
| 32 | Waveforms enable | `waveforms` | STRING | FALSE, TRUE | No | 実装済 |
| 33 | Extras enable | `extras` | STRING | FALSE, TRUE | No | **新規** |
| 34 | Event aggregation | `eventaggr` | NUMBER | 1-1023 | No | **新規** |

### Virtual Probes (VTrace) — Waveform Signal Selection

| # | CoMPASS名 | DevTree名 | 型 | 範囲/選択肢 | SetInRun | UI実装 |
|---|---|---|---|---|---|---|
| 35 | Analog Probe 1 | `/vtrace/0/par/vtrace_probe` | STRING | VPROBE_INPUT, VPROBE_CFD | Yes | 実装済 |
| 36 | Analog Probe 2 | `/vtrace/1/par/vtrace_probe` | STRING | VPROBE_NONE, VPROBE_BASELINE, VPROBE_CFD | Yes | 実装済 |
| 37 | Digital Probe 1 | `/vtrace/2/par/vtrace_probe` | STRING | VPROBE_GATE, VPROBE_OVERTHRESHOLD, VPROBE_TRGOUT, VPROBE_TRGVALWIN, VPROBE_PILEUP, VPROBE_COINCIDENCE, VPROBE_TRIGGER | Yes | 実装済 |
| 38 | Digital Probe 2 | `/vtrace/3/par/vtrace_probe` | STRING | VPROBE_GATESHORT, VPROBE_OVERTHRESHOLD, VPROBE_TRGVAL, VPROBE_TRGHOLDOFF, VPROBE_PILEUP_TRIGGER, VPROBE_TRIGGER | Yes | 実装済 |

---

## PHA1 (DT5730B, DPP_PHA, FW 139.10)

### Channel Parameters

| # | CoMPASS名 | DevTree名 | 型 | 範囲/選択肢 | SetInRun | UI実装 |
|---|---|---|---|---|---|---|
| 1 | Enable | `ch_enabled` | STRING | FALSE, TRUE | Yes | 実装済 |
| 2 | Pre-trigger | `ch_pretrg` | NUMBER | 64-2000 ns (expuom:-9) | No | **新規** |
| 3 | Polarity | `ch_polarity` | STRING | POLARITY_POSITIVE, POLARITY_NEGATIVE | Yes | 実装済 |
| 4 | N samples baseline | `ch_bline_nsmean` | STRING | BLINE_NSMEAN_FIXED, _16, _64, _256, _1024, _4096, _16384 | No | **新規** |
| 5 | DC Offset | `ch_dcoffset` | NUMBER | 0-100 % | Yes | 実装済 |
| 6 | Coarse gain | `ch_cgain` | STRING | COARSE_GAIN_X1, _X4 | Yes | **新規** |
| 7 | Threshold | `ch_threshold` | NUMBER | 0-16383 LSB | Yes | 実装済 |
| 8 | Trigger holdoff | `ch_trg_holdoff` | NUMBER | 8-8184 ns (expuom:-9) | Yes | **新規** |
| 9 | Fast Discr smoothing | `ch_rccr2_smooth` | STRING | RCCR2_SMTH_1, _2, _4, _8, _16, _32, _64, _128 | No | **新規** |
| 10 | Input rise time | `ch_rccr2_rise` | NUMBER | 16-2040 ns (expuom:-9) | No | **新規** |
| 11 | Trap. rise time | `ch_trap_trise` | NUMBER | 8-32760 ns (expuom:-9) | No | **新規** |
| 12 | Trap. flat top | `ch_trap_tflat` | NUMBER | 8-8184 ns (expuom:-9) | No | **新規** |
| 13 | Trap. pole zero | `ch_tdecay` | NUMBER | 8-524280 ns (expuom:-9) | No | **新規** |
| 14 | Peaking time | `ch_trap_ftd` | NUMBER | 0-100 % | Yes | **新規** |
| 15 | N samples peak | `ch_peak_nsmean` | STRING | PEAK_NSMEAN_1, _4, _16, _64 | No | **新規** |
| 16 | Peak holdoff | `ch_peak_holdoff` | NUMBER | 8-8184 ns (expuom:-9) | No | **新規** |
| 17 | Energy fine gain | `ch_fgain` | NUMBER | 1.00-10.00 | Yes | **新規** |
| 18 | Self trigger enable | `ch_self_trg_enable` | STRING | FALSE, TRUE | Yes | **新規** |
| 19 | Global trigger gen | `ch_trg_global_gen` | STRING | FALSE, TRUE | Yes | **新規** |
| 20 | Trigger output propagate | `ch_out_propagate` | STRING | FALSE, TRUE | Yes | **新規** |
| 21 | Veto source | `ch_veto_src` | STRING | VETO_SRC_DISABLED, _COMMON, _INDIVIDUAL, _SATURATION | Yes | **新規** |
| 22 | Coincidence mode | `ch_trg_mode` | STRING | TRIGGER_MODE_NORMAL, _COINC, _ANTICOINC | No | **新規** |

### Board Parameters

| # | CoMPASS名 | DevTree名 | 型 | 範囲/選択肢 | SetInRun | UI実装 |
|---|---|---|---|---|---|---|
| 23 | Record length | `reclen` | NUMBER | 16-262112 samples | No | 実装済 |
| 24 | Ext clock source | `dt_ext_clock` | STRING | FALSE, TRUE | Yes | **新規** |
| 25 | Start mode | `startmode` | STRING | START_MODE_SW, _S_IN, _FIRST_TRG | No | 実装済 |
| 26 | TRG OUT/GPO mode | `out_selection` | STRING | (PSD1と同じ11個) | No | **新規** |
| 27 | Start delay | `start_delay` | NUMBER | 0-4080 samples | Yes | **新規** |
| 28 | Coincidence window | `coinc_trgout` | NUMBER | 0-8184 samples | Yes | **新規** |
| 29 | FPIO type | `iolevel` | STRING | FPIOTYPE_NIM, FPIOTYPE_TTL | Yes | **新規** |
| 30 | Waveforms enable | `waveforms` | STRING | FALSE, TRUE | No | 実装済 |
| 31 | Extras enable | `extras` | STRING | FALSE, TRUE | No | **新規** |
| 32 | Event aggregation | `eventaggr` | NUMBER | 1-1023 | No | **新規** |

---

## PHA2 (VX2730, DPP_PHA, FW 1.0.88)

VX2730 に DPP-PHA firmware を flash した DIG2 系。**PSD2 と同一ハード SN:52622** で
firmware を切り替えて運用する。Channel 共通 46 params は PSD2 と一致、PHA-only
の 19 params が trapezoid filter まわり。

### Channel Parameters — 共通 46 (PSD2 と同じパス・型)

`enabled` / `pulsepolarity` / `dcoffset` / `chgain` / `chrecordlengtht` /
`chpretriggert` / `wavedownsamplingfactor` / `triggerthr` (PSD2 と field 名違い:
PSD2 は `triggerthreshold`) / `triggerholdoffl` / `eventtriggersource` /
`wavetriggersource` / `wavesaving` / `waveanalogprobe0..1` /
`wavedigitalprobe0..3` / coincidence (`channelstriggermask`,
`coincidencemask`, `anticoincidencemask`, `coincidencelengtht`,
`channelvetosource`, `adcvetowidth`) / `eventselector`

### Channel Parameters — PHA2-only 19

| # | CoMPASS名 | DevTree名 | 型 | 範囲/選択肢 | SetInRun | UI実装 |
|---|---|---|---|---|---|---|
| 1 | Time filter rise time | `timefilterrisetimet` | NUMBER | 16–500 ns, step 2 | Yes | **Phase 4 待ち** |
| 2 | Time filter retrigger guard | `timefilterretriggerguardt` | NUMBER | 0–8000 ns, step 8 | Yes | **Phase 4 待ち** |
| 3 | Energy filter rise time | `energyfilterrisetimet` | NUMBER | 16–13000 ns, step 8, def 5000 | Yes | **Phase 4 待ち** |
| 4 | Energy filter flat top | `energyfilterflattopt` | NUMBER | 32–3000 ns, step 8, def 1000 | Yes | **Phase 4 待ち** |
| 5 | Energy filter pole zero | `energyfilterpolezerot` | NUMBER | 32–131000 ns, step 2, def 50000 | Yes | **Phase 4 待ち** |
| 6 | Energy filter peaking position | `energyfilterpeakingposition` | NUMBER | 10–90 %, step 1, def 50 | Yes | **Phase 4 待ち** |
| 7 | Energy filter peaking avg | `energyfilterpeakingavg` | STRING | LowAVG, MediumAVG, HighAVG | Yes | **Phase 4 待ち** |
| 8 | Energy filter baseline avg | `energyfilterbaselineavg` | STRING | Fixed, VeryLow, Low, MediumLow, Medium, MediumHigh, High | Yes | **Phase 4 待ち** |
| 9 | Energy filter baseline guard | `energyfilterbaselineguardt` | NUMBER | 0–8000 ns, step 8 | Yes | **Phase 4 待ち** |
| 10 | Energy filter pile-up guard | `energyfilterpileupguardt` | NUMBER | 0–64000 ns, step 64, def 240 | Yes | **Phase 4 待ち** |
| 11 | Energy filter fine gain | `energyfilterfinegain` | NUMBER | 1.000–10.000, step 0.001, def 1.000 | Yes | **Phase 4 待ち** |
| 12 | Energy filter LF limitation | `energyfilterlflimitation` | STRING | On, Off | Yes | **Phase 4 待ち** |
| 13 | S_IN function | `sinfunction` | STRING | None, ResetTimestamp | No | (デフォルト None で良い、UI 不要) |
| 14 | GPI function | `gpifunction` | STRING | None, ResetTimestamp | No | (デフォルト None で良い、UI 不要) |

### Board Parameters

PSD2 と同一(94 params 共通)。**IPE pulser 系(`ipeamplitude`, `iperate`,
`ipebaseline`, `ipedecaytime`, `ipetimemode`)は PSD2 のみで PHA2 にはない**。
逆に PHA2 のみ `/group/N/par/inputdelay`(VGA group 単位、16 group)が存在。

### Event Format (`dpppha` endpoint)

PSD2 と同じ Individual Trigger Mode (`format=0x2`) ベースだが、per-event 第 2
ワードで `bits[41:26]` が未使用(PSD2 はここに `charge_short`)。`EventData::energy_short = 0` 固定で運用。Decoder は [src/reader/decoder/pha2.rs](../src/reader/decoder/pha2.rs)。

**Decoder quirk(2026-05-04 fixed):** PHA2 firmware は bad state で waveform を
truncate するが `wf_size` word は嘘の値(200)を保つ。decoder は sample loop 中
の wf_header pattern (`bit63=1 ∧ bits[62:60]=0`) 検出で next event 境界に rewind
する resync ロジックを持つ。

---

## 注記

### Unit問題: PSD1/PHA1 のタイミングパラメーター

**重要（2026-02-06確認）:** DevTree `expuom: -9` は nanoseconds (10^-9秒) を意味する。

PSD1/PHA1の全タイミングパラメーターはナノ秒単位を直接受け付ける：

**PSD1 (全て expuom: -9, nanoseconds):**
- `ch_pretrg`, `ch_trg_holdoff`, `ch_cfd_delay`
- `ch_gate`, `ch_gateshort`, `ch_gatepre`

**PHA1 (全て expuom: -9, nanoseconds):**
- `ch_pretrg`, `ch_trg_holdoff`
- `ch_rccr2_rise` (Input rise time)
- `ch_trap_trise`, `ch_trap_tflat`, `ch_tdecay` (Trapezoid filter)
- `ch_peak_holdoff`

**注意:** 以前はこれらが samples 単位と誤解されていたため、backend で `/ TIME_STEP_NS` 変換をしていたが、これは誤り。
DevTree は nanoseconds を直接受け付けるため、変換なしで値を渡す。

### Enum値の表記差異

PSD1/PHA1はレジスタスタイル（`POLARITY_NEGATIVE`）、PSD2はフレンドリー名（`Negative`）。
UIでは統一的にフレンドリー名を表示し、apply時にファームウェアに応じて変換する。

### スキップしたパラメーター

| CoMPASS名 | 理由 |
|---|---|
| Channel time offset | CoMPASS内部用。DELILAではEventBuilderで処理 |
| E low/high cut | データカット不要 |
| PSD low/high cut | データカット不要 |
| Time intervals cut | データカット不要 |
| Calibrate ADC | 初期設定時のみ使用(PSD1, PHA1 では必ず呼ぶ必要がある)、UI不要 |
