# PSD1/PHA1 Parameter Reference

This document provides a complete reference for DT5730B PSD1 and PHA1 firmware parameters.

## Parameter Path Differences (PSD1/PHA1 vs PSD2)

| Config Field | PSD2 (VX2730) | PSD1/PHA1 (DT5730B) |
|-------------|---------------|---------------------|
| start_source | `/par/startsource` | `/par/startmode` |
| gpio_mode | `/par/gpiomode` | `/par/out_selection` |
| global_trigger_source | `/par/globaltriggersource` | **NOT EXISTS** |
| record_length | `/ch/N/par/ChRecordLengthT` | `/par/reclen` (board-level) |

## Board-Level Parameters

### Acquisition Control

| Parameter | DevTree Path | Type | Range/Values | Default | SetInRun |
|-----------|-------------|------|--------------|---------|----------|
| startmode | `/par/startmode` | STRING | START_MODE_SW, START_MODE_S_IN, START_MODE_FIRST_TRG | START_MODE_SW | No |
| reclen | `/par/reclen` | NUMBER | 16-262112 (step 16) | 20000 | No |
| eventaggr | `/par/eventaggr` | NUMBER | 1-1023 | 1023 | No |
| start_delay | `/par/start_delay` | NUMBER | 0-4080 (step 16) | 0 | No |
| autoflush_enable | `/par/autoflush_enable` | STRING | FALSE, TRUE | TRUE | No |

### Trigger Configuration

| Parameter | DevTree Path | Type | Range/Values | Default | SetInRun |
|-----------|-------------|------|--------------|---------|----------|
| trg_sw_enable | `/par/trg_sw_enable` | STRING | FALSE, TRUE | TRUE | Yes |
| trg_ext_enable | `/par/trg_ext_enable` | STRING | FALSE, TRUE | FALSE | Yes |
| trg_sw_out_propagate | `/par/trg_sw_out_propagate` | STRING | FALSE, TRUE | FALSE | Yes |
| trg_ext_out_propagate | `/par/trg_ext_out_propagate` | STRING | FALSE, TRUE | FALSE | Yes |
| trgval_propagate | `/par/trgval_propagate` | STRING | FALSE, TRUE | TRUE | Yes |
| coinc_trgout | `/par/coinc_trgout` | NUMBER | 0-8184 (step 8) | 96 | Yes |

### Output Configuration

| Parameter | DevTree Path | Type | Values | Default |
|-----------|-------------|------|--------|---------|
| out_selection | `/par/out_selection` | STRING | OUT_PROPAGATION_TRIGGER, OUT_PROPAGATION_S_IN, OUT_PROPAGATION_S_IN_SYNCHRONIZED, OUT_PROPAGATION_BUSY, OUT_PROPAGATION_RUN, OUT_PROPAGATION_PLL_CLK, OUT_PROPAGATION_PLL_CLK_DIV2, OUT_PROPAGATION_PLL_CLK_DIV4, OUT_PROPAGATION_LEVEL0, OUT_PROPAGATION_LEVEL1, OUT_PROPAGATION_TEST_PULSE | OUT_PROPAGATION_TRIGGER |

### Clock & Sync

| Parameter | DevTree Path | Type | Values | Default |
|-----------|-------------|------|--------|---------|
| dt_ext_clock | `/par/dt_ext_clock` | STRING | FALSE, TRUE | FALSE |
| iolevel | `/par/iolevel` | STRING | FPIOTYPE_NIM, FPIOTYPE_TTL | FPIOTYPE_NIM |

### Data Options

| Parameter | DevTree Path | Type | Values | Default |
|-----------|-------------|------|--------|---------|
| waveforms | `/par/waveforms` | STRING | FALSE, TRUE | FALSE |
| extras | `/par/extras` | STRING | FALSE, TRUE | TRUE |

## Channel-Level Parameters

### Input Configuration

| Parameter | DevTree Path | Type | Range/Values | Default | SetInRun |
|-----------|-------------|------|--------------|---------|----------|
| ch_enabled | `ch_enabled` | STRING | FALSE, TRUE | TRUE | No |
| ch_polarity | `ch_polarity` | STRING | POLARITY_POSITIVE, POLARITY_NEGATIVE | POLARITY_POSITIVE | Yes |
| ch_dcoffset | `ch_dcoffset` | NUMBER | 0.0-100.0 (%) | 20 | Yes |
| ch_indyn | `ch_indyn` | STRING | INDYN_0_5_VPP, INDYN_2_0_VPP | INDYN_2_0_VPP | No |

### Baseline

| Parameter | DevTree Path | Type | Range/Values | Default | SetInRun |
|-----------|-------------|------|--------------|---------|----------|
| ch_bline_nsmean | `ch_bline_nsmean` | STRING | BLINE_NSMEAN_FIXED, _4, _16, _64, _256, _1024, _4096, _16384 | BLINE_NSMEAN_256 | No |
| ch_bline_fixed | `ch_bline_fixed` | NUMBER | 0-16383 | 8192 | Yes |

### Discriminator/Trigger

| Parameter | DevTree Path | Type | Range/Values | Default | SetInRun |
|-----------|-------------|------|--------------|---------|----------|
| ch_threshold | `ch_threshold` | NUMBER | 0-16383 | 100 | Yes |
| ch_discr_mode | `ch_discr_mode` | STRING | DISCR_MODE_LED, DISCR_MODE_CFD | DISCR_MODE_LED | No |
| ch_cfd_delay | `ch_cfd_delay` | NUMBER | 0-255 (×2ns) | 8 | Yes |
| ch_cfd_fraction | `ch_cfd_fraction` | STRING | CFD_FRACTLIST_25, _50, _75, _100 | CFD_FRACTLIST_50 | Yes |
| ch_cfd_smoothexp | `ch_cfd_smoothexp` | STRING | CFD_SMOOTH_EXP_1, _2, _4, _8, _16 | CFD_SMOOTH_EXP_1 | Yes |
| ch_trg_holdoff | `ch_trg_holdoff` | NUMBER | 8-8184 (×2ns) | 300 | Yes |
| ch_pretrg | `ch_pretrg` | NUMBER | 64-2000 (×2ns) | 2000 | No |
| ch_trg_latency | `ch_trg_latency` | STRING | TRG_LATENCY_MODE_NONE, _COUPLES, _ONETOALL | TRG_LATENCY_MODE_NONE | Yes |
| ch_self_trg_enable | `ch_self_trg_enable` | STRING | FALSE, TRUE | TRUE | Yes |
| ch_trg_global_gen | `ch_trg_global_gen` | STRING | FALSE, TRUE | FALSE | Yes |
| ch_out_propagate | `ch_out_propagate` | STRING | FALSE, TRUE | FALSE | Yes |

### Energy/Charge (PSD1 only)

| Parameter | DevTree Path | Type | Range/Values | Default | SetInRun |
|-----------|-------------|------|--------------|---------|----------|
| ch_energy_cgain | `ch_energy_cgain` | STRING | CHARGESENS_2_5_FC_LSB_VPP, _5, _10, _25, _40, _80, _160, _320, _640, _2560 | CHARGESENS_10_FC_LSB_VPP | No |
| ch_gate | `ch_gate` | NUMBER | 8-2040 (×2ns) | 300 | Yes |
| ch_gateshort | `ch_gateshort` | NUMBER | 8-2040 (×2ns) | 50 | Yes |
| ch_gatepre | `ch_gatepre` | NUMBER | 8-248 (×2ns) | 40 | No |
| ch_pedestal_en | `ch_pedestal_en` | STRING | FALSE, TRUE | FALSE | Yes |
| ch_extras_opt | `ch_extras_opt` | STRING | EXTRAS_OPT_TIME0, _TT32, _TT48, _TT64 | EXTRAS_OPT_TT48 | No |

### Trapezoidal Filter (PHA1 only)

| Parameter | DevTree Path | Type | Range/Values | Default | SetInRun |
|-----------|-------------|------|--------------|---------|----------|
| ch_trap_trise | `ch_trap_trise` | NUMBER | 8-32760 (ns) | 5000 | No |
| ch_trap_tflat | `ch_trap_tflat` | NUMBER | 8-8184 (ns) | 1000 | No |
| ch_trap_ftd | `ch_trap_ftd` | NUMBER | 0.0-100.0 (%) | 80 | Yes |
| ch_peak_holdoff | `ch_peak_holdoff` | NUMBER | 8-8184 (ns) | 960 | Yes |
| ch_peak_nsmean | `ch_peak_nsmean` | STRING | PEAK_NSMEAN_1, _4, _16, _64 | PEAK_NSMEAN_1 | No |
| ch_tdecay | `ch_tdecay` | NUMBER | 8-524280 (ns) | 50000 | No |
| ch_rccr2_rise | `ch_rccr2_rise` | NUMBER | 16-2040 (ns) | 96 | No |
| ch_rccr2_smooth | `ch_rccr2_smooth` | STRING | RCCR2_SMTH_1, _2, _4, _8, _16, _32, _64, _128 | RCCR2_SMTH_4 | No |
| ch_cgain | `ch_cgain` | STRING | COARSE_GAIN_X1, COARSE_GAIN_X4 | COARSE_GAIN_X1 | No |
| ch_fgain | `ch_fgain` | NUMBER | 1.00-10.00 | 1.0 | Yes |

### Coincidence

| Parameter | DevTree Path | Type | Range/Values | Default | SetInRun |
|-----------|-------------|------|--------------|---------|----------|
| ch_trg_mode | `ch_trg_mode` | STRING | TRIGGER_MODE_NORMAL, _COINC, _ANTICOINC | TRIGGER_MODE_NORMAL | Yes |
| ch_coinc_mask | `ch_coinc_mask` | NUMBER | 0-15 | 0 | Yes |
| ch_coinc_operation | `ch_coinc_operation` | STRING | COINC_OPERATION_OR, _AND, _MAJ | COINC_OPERATION_OR | Yes |
| ch_coinc_majlev | `ch_coinc_majlev` | NUMBER | 0-7 | 0 | Yes |
| ch_coinc_trgext | `ch_coinc_trgext` | STRING | FALSE, TRUE | FALSE | Yes |
| ch_coinc_trgsw | `ch_coinc_trgsw` | STRING | FALSE, TRUE | FALSE | Yes |

### Coupled Trigger

| Parameter | DevTree Path | Type | Values | Default |
|-----------|-------------|------|--------|---------|
| ch_couple_trg_mode | `ch_couple_trg_mode` | STRING | COUPLE_TRG_MODE_DISABLED, _AND, _EVEN_ONLY, _ODD_ONLY, _OR | COUPLE_TRG_MODE_DISABLED |
| ch_val_mode | `ch_val_mode` | STRING | COUPLE_VAL_MODE_DISABLED, _TWIN_ONLY, _MBOARD_ONLY, _AND, _OR | COUPLE_VAL_MODE_DISABLED |

### Pileup & Veto

| Parameter | DevTree Path | Type | Range/Values | Default | SetInRun |
|-----------|-------------|------|--------------|---------|----------|
| ch_pur_en | `ch_pur_en` | STRING | FALSE, TRUE | FALSE | Yes |
| ch_purgap | `ch_purgap` | NUMBER | 0-4095 | 100 | Yes |
| ch_pu_count_en | `ch_pu_count_en` | STRING | FALSE, TRUE | FALSE | Yes |
| ch_veto_src | `ch_veto_src` | STRING | VETO_SRC_DISABLED, _COMMON, _INDIVIDUAL, _SATURATION | VETO_SRC_DISABLED | Yes |

## Virtual Trace Parameters

| Parameter | DevTree Path | Type | Values |
|-----------|-------------|------|--------|
| vtrace_probe | `/vtrace/N/par/vtrace_probe` | STRING | **PSD1**: VPROBE_INPUT, VPROBE_CFD, VPROBE_GATE, VPROBE_GATESHORT, VPROBE_BASELINE, VPROBE_TRIGGER, VPROBE_NONE |
| | | | **PHA1**: VPROBE_INPUT, VPROBE_DELTA, VPROBE_DELTA2, VPROBE_TRAPEZOID |

### VTrace Index Mapping

| Index | Trace Type | Description |
|-------|------------|-------------|
| 0 | Analog Probe 1 | Primary analog trace |
| 1 | Analog Probe 2 | Secondary analog trace |
| 2 | Digital Probe 1 | Primary digital trace |
| 3 | Digital Probe 2 | Secondary digital trace |

## Timing Unit Conversion

DT5730B samples at 500 MHz (2 ns per sample).

| Config Unit | DevTree Unit | Conversion |
|-------------|--------------|------------|
| ns (nanoseconds) | samples (s) | samples = ns / 2 |

Parameters requiring conversion:
- `ch_cfd_delay`: config in ns → DevTree in samples
- `ch_trg_holdoff`: config in ns → DevTree in samples
- `ch_pretrg`: config in ns → DevTree in samples
- `ch_gate`, `ch_gateshort`, `ch_gatepre`: config in ns → DevTree in samples
- `reclen`: config in samples → DevTree in samples (no conversion)
