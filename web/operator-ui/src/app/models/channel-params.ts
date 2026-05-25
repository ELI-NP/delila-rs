/**
 * Shared channel parameter definitions for digitizer settings.
 * Used by both digitizer-settings component and Tune Up mode.
 * Source: docs/compass_devtree_mapping.md
 * Step values: docs/devtree_examples/ (DevTree increment field)
 */

import { ChannelParamDef } from '../components/channel-table/channel-table.component';
import { FirmwareType } from './types';

// =============================================================================
// Channel Parameter Definitions — 5 categories × 3 firmware types
// =============================================================================

// --- Input -------------------------------------------------------------------

const PSD2_INPUT_PARAMS: ChannelParamDef[] = [
  { key: 'enabled', label: 'Enable', type: 'boolean', setInRun: true },
  { key: 'polarity', label: 'Polarity', type: 'enum', options: ['Positive', 'Negative'] },
  { key: 'dc_offset', label: 'DC Offset', type: 'number', unit: '%', min: 0, max: 100, step: 0.001, setInRun: true },
  { key: 'vga_gain', label: 'VGA Gain', type: 'number', unit: 'dB', min: 0, max: 29, step: 1, setInRun: true },
  { key: 'baseline_avg', label: 'N Samples Baseline', type: 'enum', options: ['Fixed', 'Low', 'MediumLow', 'MediumHigh', 'High'] },
  { key: 'fixed_baseline', label: 'Fixed Baseline', type: 'number', unit: 'ADC', min: 0, max: 16383, step: 1, setInRun: true },
  { key: 'record_length_ns', label: 'Record Length', type: 'number', unit: 'ns', min: 32, max: 16200, step: 8 },
  { key: 'pre_trigger_ns', label: 'Pre-trigger', type: 'number', unit: 'ns', min: 32, max: 8000, step: 8, setInRun: true },
  { key: 'wave_downsampling', label: 'Wave Downsampling', type: 'enum', options: ['1', '2', '4', '8'] },
];

const PSD1_INPUT_PARAMS: ChannelParamDef[] = [
  { key: 'enabled', label: 'Enable', type: 'boolean', setInRun: true },
  { key: 'polarity', label: 'Polarity', type: 'enum', options: ['POLARITY_POSITIVE', 'POLARITY_NEGATIVE'], setInRun: true },
  { key: 'dc_offset', label: 'DC Offset', type: 'number', unit: '%', min: 0, max: 100, step: 0.1, setInRun: true },
  { key: 'input_dynamic', label: 'Input Dynamic', type: 'enum', options: ['INDYN_2_0_VPP', 'INDYN_0_5_VPP'], setInRun: true },
  { key: 'baseline_avg', label: 'N Samples Baseline', type: 'enum', options: ['BLINE_NSMEAN_FIXED', 'BLINE_NSMEAN_16', 'BLINE_NSMEAN_64', 'BLINE_NSMEAN_256', 'BLINE_NSMEAN_1024'] },
  { key: 'fixed_baseline', label: 'Fixed Baseline', type: 'number', min: 0, max: 16383, step: 1, setInRun: true },
  { key: 'pre_trigger_ns', label: 'Pre-trigger', type: 'number', unit: 'ns', min: 80, max: 4032, step: 8 },
];

const PHA2_INPUT_PARAMS: ChannelParamDef[] = [
  { key: 'enabled', label: 'Enable', type: 'boolean', setInRun: true },
  { key: 'polarity', label: 'Polarity', type: 'enum', options: ['Positive', 'Negative'] },
  { key: 'dc_offset', label: 'DC Offset', type: 'number', unit: '%', min: 0, max: 100, step: 0.001, setInRun: true },
  { key: 'vga_gain', label: 'VGA Gain', type: 'number', unit: 'dB', min: 0, max: 29, step: 1, setInRun: true },
  { key: 'record_length_ns', label: 'Record Length', type: 'number', unit: 'ns', min: 32, max: 16200, step: 8 },
  { key: 'pre_trigger_ns', label: 'Pre-trigger', type: 'number', unit: 'ns', min: 16, max: 4000, step: 8, setInRun: true },
  { key: 'wave_downsampling', label: 'Wave Downsampling', type: 'enum', options: ['1', '2', '4', '8'] },
];

const PHA1_INPUT_PARAMS: ChannelParamDef[] = [
  { key: 'enabled', label: 'Enable', type: 'boolean', setInRun: true },
  { key: 'polarity', label: 'Polarity', type: 'enum', options: ['POLARITY_POSITIVE', 'POLARITY_NEGATIVE'], setInRun: true },
  { key: 'dc_offset', label: 'DC Offset', type: 'number', unit: '%', min: 0, max: 100, step: 0.1, setInRun: true },
  { key: 'coarse_gain', label: 'Coarse Gain', type: 'enum', options: ['COARSE_GAIN_X1', 'COARSE_GAIN_X4'], setInRun: true },
  { key: 'baseline_avg', label: 'N Samples Baseline', type: 'enum', options: ['BLINE_NSMEAN_FIXED', 'BLINE_NSMEAN_16', 'BLINE_NSMEAN_64', 'BLINE_NSMEAN_256', 'BLINE_NSMEAN_1024', 'BLINE_NSMEAN_4096', 'BLINE_NSMEAN_16384'] },
  { key: 'pre_trigger_ns', label: 'Pre-trigger', type: 'number', unit: 'ns', min: 128, max: 4000, step: 8 },
];

const X743STD_INPUT_PARAMS: ChannelParamDef[] = [
  { key: 'enabled', label: 'Enable', type: 'boolean' },
  { key: 'polarity', label: 'Polarity', type: 'enum', options: ['Positive', 'Negative'] },
  { key: 'dc_offset', label: 'DC Offset', type: 'number', unit: '%', min: 0, max: 100, step: 0.1 },
];

// --- Trigger -----------------------------------------------------------------

const PSD2_TRIGGER_PARAMS: ChannelParamDef[] = [
  { key: 'discriminator_mode', label: 'Discriminator', type: 'enum', options: ['LeadingEdge', 'CFD'] },
  { key: 'trigger_threshold', label: 'Threshold', type: 'number', unit: 'ADC', min: 1, max: 8191, step: 1, setInRun: true },
  { key: 'cfd_delay_ns', label: 'CFD Delay', type: 'number', unit: 'ns', min: 2, max: 2040, step: 2 },
  { key: 'cfd_fraction', label: 'CFD Fraction', type: 'enum', options: ['25', '50', '75', '100'] },
  { key: 'trigger_holdoff_ns', label: 'Trigger Holdoff', type: 'number', unit: 'ns', min: 8, max: 8000, step: 8 },
  { key: 'smoothing_factor', label: 'Smoothing Factor', type: 'enum', options: ['1', '2', '4', '8', '16'], setInRun: true },
  { key: 'time_filter_smoothing', label: 'Time Filter Smooth', type: 'enum', options: ['Enabled', 'Disabled'], setInRun: true },
  { key: 'event_trigger_source', label: 'Event Trigger', type: 'enum', options: ['Disabled', 'Ch64Trigger', 'ChSelfTrigger', 'SwTrg', 'TRGIN', 'GlobalTriggerSource', 'LVDS', 'ITLA', 'ITLB'], setInRun: true },
  { key: 'wave_trigger_source', label: 'Wave Trigger', type: 'enum', options: ['Disabled', 'Ch64Trigger', 'ITLA', 'ITLB', 'ChSelfTrigger', 'SwTrg', 'ADCOverSaturation', 'ADCUnderSaturation', 'ExternalInhibit', 'TRGIN', 'GlobalTriggerSource', 'LVDS'], setInRun: true },
];

const PSD1_TRIGGER_PARAMS: ChannelParamDef[] = [
  { key: 'discriminator_mode', label: 'Discriminator', type: 'enum', options: ['DISCR_MODE_LED', 'DISCR_MODE_CFD'], setInRun: true },
  { key: 'trigger_threshold', label: 'Threshold', type: 'number', unit: 'LSB', min: 0, max: 16383, step: 1, setInRun: true },
  { key: 'cfd_delay_ns', label: 'CFD Delay', type: 'number', unit: 'ns', min: 0, max: 1020, step: 2, setInRun: true },
  { key: 'cfd_fraction', label: 'CFD Fraction', type: 'enum', options: ['CFD_FRACTLIST_25', 'CFD_FRACTLIST_50', 'CFD_FRACTLIST_75', 'CFD_FRACTLIST_100'], setInRun: true },
  { key: 'cfd_interpolation_point', label: 'CFD Interp. Point', type: 'number', min: 0, max: 3, step: 1 },
  { key: 'input_smoothing', label: 'Input Smoothing', type: 'enum', options: ['CFD_SMOOTH_EXP_1', 'CFD_SMOOTH_EXP_2', 'CFD_SMOOTH_EXP_4', 'CFD_SMOOTH_EXP_8', 'CFD_SMOOTH_EXP_16'] },
  { key: 'trigger_holdoff_ns', label: 'Trigger Holdoff', type: 'number', unit: 'ns', min: 0, max: 1048560, step: 8, setInRun: true },
  { key: 'trigger_latency', label: 'Trigger Latency', type: 'enum', options: ['TRG_LATENCY_MODE_NONE', 'TRG_LATENCY_MODE_COUPLES', 'TRG_LATENCY_MODE_ONETOALL'], setInRun: true },
  { key: 'self_trigger', label: 'Self Trigger', type: 'enum', options: ['FALSE', 'TRUE'], setInRun: true },
  { key: 'global_trigger_gen', label: 'Global Trigger Gen', type: 'enum', options: ['FALSE', 'TRUE'], setInRun: true },
  { key: 'trigger_out_propagate', label: 'Trigger Out Prop.', type: 'enum', options: ['FALSE', 'TRUE'], setInRun: true },
];

const X743STD_TRIGGER_PARAMS: ChannelParamDef[] = [
  // V1743 threshold expressed as **input-referred volts** (DC-offset-aware).
  // The backend converts V→DAC accounting for the channel's DC offset, so users
  // type the threshold as it appears at the input. Range matches the V1743
  // input dynamic range (±1.25 V); 1 mV resolution covers the 38 µV LSB easily.
  { key: 'trigger_threshold_v', label: 'Threshold', type: 'number', unit: 'V', min: -1.25, max: 1.25, step: 0.001 },
  // Trigger edge is independent of pulse polarity (WaveDemo: TRIGGER_EDGE vs PULSE_POLARITY).
  // When unset, falls back to the Polarity field (Positive→Rising, Negative→Falling) for backward compat.
  { key: 'trigger_edge', label: 'Trigger Edge', type: 'enum', options: ['Rising', 'Falling'] },
  { key: 'self_trigger', label: 'Self Trigger', type: 'boolean' },
];

const PHA2_TRIGGER_PARAMS: ChannelParamDef[] = [
  // PHA2 trigger threshold acts on the time-filter signal output, not on raw ADC.
  // DevTree: 1..8191 ADC count, default 50.
  { key: 'trigger_threshold', label: 'Threshold', type: 'number', unit: 'ADC', min: 1, max: 8191, step: 1, setInRun: true },
  { key: 'time_filter_rise_time_ns', label: 'Time Filter Rise', type: 'number', unit: 'ns', min: 16, max: 500, step: 2, setInRun: true },
  { key: 'time_filter_retrigger_guard_ns', label: 'Time Filter Retrig Guard', type: 'number', unit: 'ns', min: 0, max: 8000, step: 8, setInRun: true },
  { key: 'event_trigger_source', label: 'Event Trigger', type: 'enum', options: ['Disabled', 'Ch64Trigger', 'ChSelfTrigger', 'SwTrg', 'TRGIN', 'GlobalTriggerSource', 'LVDS', 'ITLA', 'ITLB'], setInRun: true },
  { key: 'wave_trigger_source', label: 'Wave Trigger', type: 'enum', options: ['Disabled', 'Ch64Trigger', 'ChSelfTrigger', 'SwTrg', 'ADCOverSaturation', 'ADCUnderSaturation', 'ExternalInhibit', 'TRGIN', 'GlobalTriggerSource', 'LVDS', 'ITLA', 'ITLB'], setInRun: true },
];

const PHA1_TRIGGER_PARAMS: ChannelParamDef[] = [
  { key: 'trigger_threshold', label: 'Threshold', type: 'number', unit: 'LSB', min: 0, max: 16383, step: 1, setInRun: true },
  { key: 'trigger_holdoff_ns', label: 'Trigger Holdoff', type: 'number', unit: 'ns', min: 16, max: 16368, step: 8, setInRun: true },
  { key: 'fast_discr_smoothing', label: 'Fast Discr Smooth', type: 'enum', options: ['RCCR2_SMTH_1', 'RCCR2_SMTH_2', 'RCCR2_SMTH_4', 'RCCR2_SMTH_8', 'RCCR2_SMTH_16', 'RCCR2_SMTH_32', 'RCCR2_SMTH_64', 'RCCR2_SMTH_128'] },
  { key: 'cfd_interpolation_point', label: 'CFD Interp. Point', type: 'number', min: 0, max: 3, step: 1 },
  { key: 'input_rise_time_ns', label: 'Input Rise Time', type: 'number', unit: 'ns', min: 16, max: 2040, step: 8 },
  { key: 'trigger_latency', label: 'Trigger Latency', type: 'enum', options: ['TRG_LATENCY_MODE_NONE', 'TRG_LATENCY_MODE_COUPLES', 'TRG_LATENCY_MODE_ONETOALL'], setInRun: true },
  { key: 'self_trigger', label: 'Self Trigger', type: 'enum', options: ['FALSE', 'TRUE'], setInRun: true },
  { key: 'global_trigger_gen', label: 'Global Trigger Gen', type: 'enum', options: ['FALSE', 'TRUE'], setInRun: true },
  { key: 'trigger_out_propagate', label: 'Trigger Out Prop.', type: 'enum', options: ['FALSE', 'TRUE'], setInRun: true },
];

// --- Energy ------------------------------------------------------------------

const PSD2_ENERGY_PARAMS: ChannelParamDef[] = [
  { key: 'energy_coarse_gain', label: 'Energy Coarse Gain', type: 'enum', options: ['x1', 'x4', 'x16', 'x64', 'x256'] },
  { key: 'gate_long_ns', label: 'Gate Long', type: 'number', unit: 'ns', min: 2, max: 8000, step: 2 },
  { key: 'gate_short_ns', label: 'Gate Short', type: 'number', unit: 'ns', min: 2, max: 8000, step: 2 },
  { key: 'gate_pre_ns', label: 'Pre-gate', type: 'number', unit: 'ns', min: 16, max: 2000, step: 2 },
  { key: 'charge_pedestal', label: 'Charge Pedestal', type: 'number', unit: 'count', min: 0, max: 1000, step: 1 },
  { key: 'short_charge_pedestal', label: 'Short Charge Ped.', type: 'number', unit: 'count', min: 0, max: 1000, step: 1 },
  { key: 'charge_smoothing', label: 'Charge Smoothing', type: 'enum', options: ['Enabled', 'Disabled'], setInRun: true },
];

const PSD1_ENERGY_PARAMS: ChannelParamDef[] = [
  { key: 'energy_coarse_gain', label: 'Energy Coarse Gain', type: 'enum', options: ['CHARGESENS_2.5_FC_LSB_VPP', 'CHARGESENS_10_FC_LSB_VPP', 'CHARGESENS_40_FC_LSB_VPP', 'CHARGESENS_160_FC_LSB_VPP', 'CHARGESENS_640_FC_LSB_VPP', 'CHARGESENS_2560_FC_LSB_VPP'], setInRun: true },
  { key: 'gate_long_ns', label: 'Gate Long', type: 'number', unit: 'ns', min: 8, max: 65532, step: 2, setInRun: true },
  { key: 'gate_short_ns', label: 'Gate Short', type: 'number', unit: 'ns', min: 4, max: 4092, step: 2, setInRun: true },
  { key: 'gate_pre_ns', label: 'Pre-gate', type: 'number', unit: 'ns', min: 0, max: 1020, step: 2 },
  { key: 'charge_pedestal_en', label: 'Charge Pedestal', type: 'enum', options: ['FALSE', 'TRUE'] },
];

const PHA2_ENERGY_PARAMS: ChannelParamDef[] = [
  { key: 'energy_filter_rise_time_ns', label: 'Energy Filter Rise', type: 'number', unit: 'ns', min: 16, max: 13000, step: 8, setInRun: true },
  { key: 'energy_filter_flat_top_ns', label: 'Energy Filter Flat Top', type: 'number', unit: 'ns', min: 32, max: 3000, step: 8, setInRun: true },
  { key: 'energy_filter_pole_zero_ns', label: 'Energy Filter Pole Zero', type: 'number', unit: 'ns', min: 32, max: 131000, step: 2, setInRun: true },
  { key: 'energy_filter_peaking_position', label: 'Peaking Position', type: 'number', unit: '%', min: 10, max: 90, step: 1, setInRun: true },
  { key: 'energy_filter_peaking_avg', label: 'Peaking Avg', type: 'enum', options: ['LowAVG', 'MediumAVG', 'HighAVG'], setInRun: true },
  { key: 'energy_filter_baseline_avg', label: 'Baseline Avg', type: 'enum', options: ['Fixed', 'VeryLow', 'Low', 'MediumLow', 'Medium', 'MediumHigh', 'High'], setInRun: true },
  { key: 'energy_filter_baseline_guard_ns', label: 'Baseline Guard', type: 'number', unit: 'ns', min: 0, max: 8000, step: 8, setInRun: true },
  { key: 'energy_filter_pileup_guard_ns', label: 'Pile-up Guard', type: 'number', unit: 'ns', min: 0, max: 64000, step: 64, setInRun: true },
  { key: 'energy_filter_fine_gain', label: 'Fine Gain', type: 'number', min: 1.0, max: 10.0, step: 0.001, setInRun: true },
  { key: 'energy_filter_lf_limitation', label: 'LF Limitation', type: 'enum', options: ['Off', 'On'], setInRun: true },
];

const PHA1_ENERGY_PARAMS: ChannelParamDef[] = [
  { key: 'trap_rise_time_ns', label: 'Trap Rise Time', type: 'number', unit: 'ns', min: 16, max: 65520, step: 8 },
  { key: 'trap_flat_top_ns', label: 'Trap Flat Top', type: 'number', unit: 'ns', min: 8, max: 8184, step: 8 },
  { key: 'trap_pole_zero_ns', label: 'Trap Pole Zero', type: 'number', unit: 'ns', min: 16, max: 1048560, step: 8 },
  { key: 'peaking_time', label: 'Peaking Time', type: 'number', unit: '%', min: 0, max: 100, step: 0.1, setInRun: true },
  { key: 'peak_nsmean', label: 'N Samples Peak', type: 'enum', options: ['PEAK_NSMEAN_1', 'PEAK_NSMEAN_4', 'PEAK_NSMEAN_16', 'PEAK_NSMEAN_64'] },
  { key: 'peak_holdoff_ns', label: 'Peak Holdoff', type: 'number', unit: 'ns', min: 8, max: 8184, step: 8 },
  { key: 'energy_fine_gain', label: 'Energy Fine Gain', type: 'number', min: 1.0, max: 10.0, step: 0.01, setInRun: true },
];

// --- Coincidence -------------------------------------------------------------

const PSD2_COINCIDENCE_PARAMS: ChannelParamDef[] = [
  { key: 'ch_trigger_mask', label: 'Ch Trigger Mask', type: 'ch-mask', bitWidth: 32, encoding: 'hex-string', setInRun: true },
  { key: 'coincidence_mask', label: 'Coincidence Mask', type: 'enum', options: ['Disabled', 'Ch64Trigger', 'TRGIN', 'GlobalTriggerSource', 'ITLA', 'ITLB'], setInRun: true },
  { key: 'anti_coincidence_mask', label: 'Anti-coinc Mask', type: 'enum', options: ['Disabled', 'Ch64Trigger', 'TRGIN', 'GlobalTriggerSource', 'ITLA', 'ITLB'], setInRun: true },
  { key: 'coincidence_window_ns', label: 'Coinc Window', type: 'number', unit: 'ns', min: 0, max: 524280, step: 8, setInRun: true },
  { key: 'ch_veto_source', label: 'Veto Source', type: 'enum', options: ['Disabled', 'BoardVeto', 'ADCOverSaturation', 'ADCUnderSaturation'], setInRun: true },
  { key: 'ch_veto_width_ns', label: 'Veto Width', type: 'number', unit: 'ns', min: 0, max: 524280, step: 8, setInRun: true },
  { key: 'event_selector', label: 'Event Selector', type: 'enum', options: ['All', 'PileUp', 'EnergySkim'], setInRun: true },
];

const PSD1_COINCIDENCE_PARAMS: ChannelParamDef[] = [
  { key: 'coincidence_mode', label: 'Coincidence Mode', type: 'enum', options: ['TRIGGER_MODE_NORMAL', 'TRIGGER_MODE_COINC', 'TRIGGER_MODE_ANTICOINC'], setInRun: true },
  { key: 'coinc_mask', label: 'Coinc Mask', type: 'ch-mask', bitWidth: 4, encoding: 'number', setInRun: true },
  { key: 'coinc_operation', label: 'Coinc Operation', type: 'enum', options: ['COINC_OPERATION_OR', 'COINC_OPERATION_AND', 'COINC_OPERATION_MAJ'], setInRun: true },
  { key: 'coinc_majority_level', label: 'Majority Level', type: 'number', min: 0, max: 7, step: 1, setInRun: true },
  { key: 'coinc_trgext', label: 'Coinc TrgExt', type: 'enum', options: ['FALSE', 'TRUE'], setInRun: true },
  { key: 'coinc_trgsw', label: 'Coinc TrgSW', type: 'enum', options: ['FALSE', 'TRUE'], setInRun: true },
  { key: 'ch_veto_source', label: 'Veto Source', type: 'enum', options: ['VETO_SRC_DISABLED', 'VETO_SRC_COMMON', 'VETO_SRC_INDIVIDUAL', 'VETO_SRC_SATURATION'], setInRun: true },
  { key: 'pileup_rejection', label: 'Pileup Rejection', type: 'enum', options: ['FALSE', 'TRUE'], setInRun: true },
  { key: 'pileup_gap', label: 'Pileup Gap', type: 'number', unit: 'LSB', min: 0, max: 4095, step: 1, setInRun: true },
  { key: 'pileup_counting_en', label: 'Pileup Counting', type: 'enum', options: ['FALSE', 'TRUE'], setInRun: true },
];

// PHA2 coincidence is identical to PSD2 in path/values; bound from DevTree.
const PHA2_COINCIDENCE_PARAMS: ChannelParamDef[] = [
  { key: 'ch_trigger_mask', label: 'Ch Trigger Mask', type: 'ch-mask', bitWidth: 32, encoding: 'hex-string', setInRun: true },
  { key: 'coincidence_mask', label: 'Coincidence Mask', type: 'enum', options: ['Disabled', 'Ch64Trigger', 'TRGIN', 'GlobalTriggerSource', 'ITLA', 'ITLB'], setInRun: true },
  { key: 'anti_coincidence_mask', label: 'Anti-coinc Mask', type: 'enum', options: ['Disabled', 'Ch64Trigger', 'TRGIN', 'GlobalTriggerSource', 'ITLA', 'ITLB'], setInRun: true },
  { key: 'coincidence_window_ns', label: 'Coinc Window', type: 'number', unit: 'ns', min: 0, max: 524280, step: 8, setInRun: true },
  { key: 'ch_veto_source', label: 'Veto Source', type: 'enum', options: ['Disabled', 'BoardVeto', 'ADCOverSaturation', 'ADCUnderSaturation'], setInRun: true },
  { key: 'ch_veto_width_ns', label: 'Veto Width', type: 'number', unit: 'ns', min: 0, max: 524280, step: 8, setInRun: true },
  { key: 'event_selector', label: 'Event Selector', type: 'enum', options: ['All', 'PileUp', 'EnergySkim'], setInRun: true },
];

const PHA1_COINCIDENCE_PARAMS: ChannelParamDef[] = [
  { key: 'coincidence_mode', label: 'Coincidence Mode', type: 'enum', options: ['TRIGGER_MODE_NORMAL', 'TRIGGER_MODE_COINC', 'TRIGGER_MODE_ANTICOINC'], setInRun: true },
  { key: 'coinc_mask', label: 'Coinc Mask', type: 'ch-mask', bitWidth: 4, encoding: 'number', setInRun: true },
  { key: 'coinc_operation', label: 'Coinc Operation', type: 'enum', options: ['COINC_OPERATION_OR', 'COINC_OPERATION_AND', 'COINC_OPERATION_MAJ'], setInRun: true },
  { key: 'coinc_majority_level', label: 'Majority Level', type: 'number', min: 0, max: 7, step: 1, setInRun: true },
  { key: 'coinc_trgext', label: 'Coinc TrgExt', type: 'enum', options: ['FALSE', 'TRUE'], setInRun: true },
  { key: 'coinc_trgsw', label: 'Coinc TrgSW', type: 'enum', options: ['FALSE', 'TRUE'], setInRun: true },
  { key: 'ch_veto_source', label: 'Veto Source', type: 'enum', options: ['VETO_SRC_DISABLED', 'VETO_SRC_COMMON', 'VETO_SRC_INDIVIDUAL', 'VETO_SRC_SATURATION'], setInRun: true },
  { key: 'pileup_flag_en', label: 'Pileup Flag', type: 'enum', options: ['FALSE', 'TRUE'], setInRun: true },
];

// --- Waveform ----------------------------------------------------------------

const PSD2_WAVEFORM_PARAMS: ChannelParamDef[] = [
  { key: 'wave_saving', label: 'Wave Saving', type: 'enum', options: ['Always', 'OnRequest'], setInRun: true },
  { key: 'analog_probe_0', label: 'Analog Probe 0', type: 'enum', options: ['ADCInput', 'ADCInputBaseline', 'CFDFilter'], setInRun: true },
  { key: 'analog_probe_1', label: 'Analog Probe 1', type: 'enum', options: ['ADCInput', 'ADCInputBaseline', 'CFDFilter'], setInRun: true },
  { key: 'digital_probe_0', label: 'Digital Probe 0', type: 'enum', options: ['Trigger', 'CFDFilterArmed', 'ADCSaturation', 'ADCInputNegativeOverthreshold', 'ReTriggerGuard', 'ADCInputBaselineFreeze', 'ADCInputOverthreshold', 'ChargeReady', 'LongGate', 'PileUpTrigger', 'ShortGate', 'ChargeOverRange'], setInRun: true },
  { key: 'digital_probe_1', label: 'Digital Probe 1', type: 'enum', options: ['Trigger', 'CFDFilterArmed', 'ADCSaturation', 'ADCInputNegativeOverthreshold', 'ReTriggerGuard', 'ADCInputBaselineFreeze', 'ADCInputOverthreshold', 'ChargeReady', 'LongGate', 'PileUpTrigger', 'ShortGate', 'ChargeOverRange'], setInRun: true },
  { key: 'digital_probe_2', label: 'Digital Probe 2', type: 'enum', options: ['Trigger', 'CFDFilterArmed', 'ADCSaturation', 'ADCInputNegativeOverthreshold', 'ReTriggerGuard', 'ADCInputBaselineFreeze', 'ADCInputOverthreshold', 'ChargeReady', 'LongGate', 'PileUpTrigger', 'ShortGate', 'ChargeOverRange'], setInRun: true },
  { key: 'digital_probe_3', label: 'Digital Probe 3', type: 'enum', options: ['Trigger', 'CFDFilterArmed', 'ADCSaturation', 'ADCInputNegativeOverthreshold', 'ReTriggerGuard', 'ADCInputBaselineFreeze', 'ADCInputOverthreshold', 'ChargeReady', 'LongGate', 'PileUpTrigger', 'ShortGate', 'ChargeOverRange'], setInRun: true },
];

const PSD1_WAVEFORM_PARAMS: ChannelParamDef[] = [];

const PHA1_WAVEFORM_PARAMS: ChannelParamDef[] = [];

// PHA2 waveform — analog probes are PHA-flavoured (TimeFilter, EnergyFilter,
// EnergyFilterBaseline, EnergyFilterMinusBaseline) and digital probes carry
// 13 PHA-specific signals plus Trigger.
const PHA2_WAVEFORM_PARAMS: ChannelParamDef[] = [
  { key: 'wave_saving', label: 'Wave Saving', type: 'enum', options: ['Always', 'OnRequest'], setInRun: true },
  { key: 'analog_probe_0', label: 'Analog Probe 0', type: 'enum', options: ['ADCInput', 'ADCInput16', 'TimeFilter', 'EnergyFilter', 'EnergyFilterBaseline', 'EnergyFilterMinusBaseline'], setInRun: true },
  { key: 'analog_probe_1', label: 'Analog Probe 1', type: 'enum', options: ['ADCInput', 'TimeFilter', 'EnergyFilter', 'EnergyFilterBaseline', 'EnergyFilterMinusBaseline'], setInRun: true },
  { key: 'digital_probe_0', label: 'Digital Probe 0', type: 'enum', options: ['Trigger', 'TimeFilterArmed', 'ReTriggerGuard', 'EnergyFilterBaselineFreeze', 'EnergyFilterPeaking', 'EnergyFilterPeakReady', 'EnergyFilterPileupGuard', 'EventPileUp', 'ADCSaturation', 'ADCSaturationProtection', 'PostSaturationEvent', 'EnergyFilterSaturation', 'AcquisitionInhibit', 'CoincidenceAnticoincidence'], setInRun: true },
  { key: 'digital_probe_1', label: 'Digital Probe 1', type: 'enum', options: ['Trigger', 'TimeFilterArmed', 'ReTriggerGuard', 'EnergyFilterBaselineFreeze', 'EnergyFilterPeaking', 'EnergyFilterPeakReady', 'EnergyFilterPileupGuard', 'EventPileUp', 'ADCSaturation', 'ADCSaturationProtection', 'PostSaturationEvent', 'EnergyFilterSaturation', 'AcquisitionInhibit', 'CoincidenceAnticoincidence'], setInRun: true },
  { key: 'digital_probe_2', label: 'Digital Probe 2', type: 'enum', options: ['Trigger', 'TimeFilterArmed', 'ReTriggerGuard', 'EnergyFilterBaselineFreeze', 'EnergyFilterPeaking', 'EnergyFilterPeakReady', 'EnergyFilterPileupGuard', 'EventPileUp', 'ADCSaturation', 'ADCSaturationProtection', 'PostSaturationEvent', 'EnergyFilterSaturation', 'AcquisitionInhibit', 'CoincidenceAnticoincidence'], setInRun: true },
  { key: 'digital_probe_3', label: 'Digital Probe 3', type: 'enum', options: ['Trigger', 'TimeFilterArmed', 'ReTriggerGuard', 'EnergyFilterBaselineFreeze', 'EnergyFilterPeaking', 'EnergyFilterPeakReady', 'EnergyFilterPileupGuard', 'EventPileUp', 'ADCSaturation', 'ADCSaturationProtection', 'PostSaturationEvent', 'EnergyFilterSaturation', 'AcquisitionInhibit', 'CoincidenceAnticoincidence'], setInRun: true },
];

// AMax custom-firmware per-channel registers. Keys are dotted paths into the
// nested `ChannelConfig.amax` struct; the digitizer.service expand/compress
// layer translates `'amax.polarity'` ↔ `cfg.amax.polarity` automatically.
// Bit widths and defaults match `tools/amax_viewer/fw_params.json` and the
// FW register table (`AMAX_firmware32_channel_4input_caenlist/...txt`).

// AMax param tables are auto-generated by `cargo run --bin amax_codegen`
// from RegisterFile.json + tools/amax_viewer/fw_params.json.
import {
  AMAX_INPUT_PARAMS,
  AMAX_TRIGGER_PARAMS,
  AMAX_ENERGY_PARAMS,
  AMAX_WAVEFORM_PARAMS,
  AMAX_DEBUG_PARAMS,
} from './amax-generated';

// AMax shares PSD2's per-channel `channelstriggermask` DevTree path
// (see `FirmwareType::PSD2 | FirmwareType::AMax` branch in
// src/config/digitizer.rs). The codegen output omits coincidence params,
// so add the single relevant entry by hand here.
const AMAX_COINCIDENCE_PARAMS: ChannelParamDef[] = [
  { key: 'ch_trigger_mask', label: 'Ch Trigger Mask', type: 'ch-mask', bitWidth: 32, encoding: 'hex-string', setInRun: true },
];

// --- Category lookup ---------------------------------------------------------

/** Channel-parameter categories surfaced in the Settings UI.
 *
 *  `input` / `trigger` / `energy` / `coincidence` / `waveform` are the core
 *  pipeline stages every CAEN firmware exposes. `debug` is AMax-only today
 *  (it carries the `delay_debug` register that shifts the debug-FW capture
 *  window — see `tools/amax_viewer/fw_params.json` for the canonical list);
 *  it's just another category name as far as the dispatch layer is
 *  concerned, so future firmwares that grow a `category=debug` field in
 *  their `fw_params.json` will round-trip through the same code path. */
export type ChannelCategory =
  | 'input'
  | 'trigger'
  | 'energy'
  | 'coincidence'
  | 'waveform'
  | 'debug';

/** Channel-param categories listed in the order operators expect to see
 *  them as Settings sub-tabs. The Settings component iterates this list
 *  so that a firmware which adds a new category (via codegen) only needs
 *  this extension — no template changes per-tab. */
export const CHANNEL_CATEGORIES: readonly ChannelCategory[] = [
  'input',
  'trigger',
  'energy',
  'coincidence',
  'waveform',
  'debug',
];

/** Human-facing label for each category (used as the `mat-tab label`). */
export const CHANNEL_CATEGORY_LABELS: Record<ChannelCategory, string> = {
  input: 'Input',
  trigger: 'Trigger',
  energy: 'Energy',
  coincidence: 'Coincidence',
  waveform: 'Waveform',
  debug: 'Debug',
};

const CATEGORY_PARAMS: Record<FirmwareType, Partial<Record<ChannelCategory, ChannelParamDef[]>>> = {
  PSD2: {
    input: PSD2_INPUT_PARAMS,
    trigger: PSD2_TRIGGER_PARAMS,
    energy: PSD2_ENERGY_PARAMS,
    coincidence: PSD2_COINCIDENCE_PARAMS,
    waveform: PSD2_WAVEFORM_PARAMS,
  },
  PSD1: {
    input: PSD1_INPUT_PARAMS,
    trigger: PSD1_TRIGGER_PARAMS,
    energy: PSD1_ENERGY_PARAMS,
    coincidence: PSD1_COINCIDENCE_PARAMS,
    waveform: PSD1_WAVEFORM_PARAMS,
  },
  PHA1: {
    input: PHA1_INPUT_PARAMS,
    trigger: PHA1_TRIGGER_PARAMS,
    energy: PHA1_ENERGY_PARAMS,
    coincidence: PHA1_COINCIDENCE_PARAMS,
    waveform: PHA1_WAVEFORM_PARAMS,
  },
  // PHA2: same DIG2 envelope as PSD2 (RAW endpoint, individual trigger
  // mode, common 46 channel params) + 14 PHA-only trapezoid + time-filter
  // params under Energy/Trigger.
  PHA2: {
    input: PHA2_INPUT_PARAMS,
    trigger: PHA2_TRIGGER_PARAMS,
    energy: PHA2_ENERGY_PARAMS,
    coincidence: PHA2_COINCIDENCE_PARAMS,
    waveform: PHA2_WAVEFORM_PARAMS,
  },
  // V1743 Standard mode: Energy/Coincidence/Waveform are board-level (handled in Settings component).
  X743Std: {
    input: X743STD_INPUT_PARAMS,
    trigger: X743STD_TRIGGER_PARAMS,
  },
  // AMax custom MCA+AMax FW: shares PSD2's per-channel trigger mask. Energy
  // tab packs trap + AMax HLS + baseline; Waveform tab carries pre-trigger;
  // Debug tab exposes `delay_debug` for shifting the debug-FW capture
  // window when ENABLE_ACQ=1.
  AMax: {
    input: AMAX_INPUT_PARAMS,
    trigger: AMAX_TRIGGER_PARAMS,
    energy: AMAX_ENERGY_PARAMS,
    coincidence: AMAX_COINCIDENCE_PARAMS,
    waveform: AMAX_WAVEFORM_PARAMS,
    debug: AMAX_DEBUG_PARAMS,
  },
};

/** Get channel params for a specific firmware and category. Returns `[]`
 *  when the firmware doesn't surface that category — the Settings tab
 *  iterates `CHANNEL_CATEGORIES` and hides empty tabs. */
export function getCategoryParams(fw: FirmwareType, category: ChannelCategory): ChannelParamDef[] {
  return CATEGORY_PARAMS[fw]?.[category] ?? [];
}

/** Get all channel params across all categories for a firmware */
export function getAllChannelParams(fw: FirmwareType): ChannelParamDef[] {
  const cats = CATEGORY_PARAMS[fw];
  if (!cats) return [];
  return CHANNEL_CATEGORIES.flatMap((c) => cats[c] ?? []);
}

// =============================================================================
// Virtual Probe Options — Board-level, firmware-specific (PSD1/PHA1 only)
// Source: docs/devtree_examples/dt5730b_pha1_sn990.json, dt5730b_psd1_sn990.json
// =============================================================================

export interface ProbeOption {
  value: string;
  label: string;
}

const PSD1_PROBE_OPTIONS: ProbeOption[][] = [
  // probe_0 (Analog Probe 1)
  [
    { value: 'VPROBE_INPUT', label: 'Input' },
    { value: 'VPROBE_CFD', label: 'CFD' },
  ],
  // probe_1 (Analog Probe 2)
  [
    { value: 'VPROBE_NONE', label: 'None' },
    { value: 'VPROBE_BASELINE', label: 'Baseline' },
    { value: 'VPROBE_CFD', label: 'CFD' },
  ],
  // probe_2 (Digital Probe 1)
  [
    { value: 'VPROBE_GATE', label: 'Gate' },
    { value: 'VPROBE_OVERTHRESHOLD', label: 'Over Threshold' },
    { value: 'VPROBE_TRGOUT', label: 'Trigger Out' },
    { value: 'VPROBE_TRGVALWIN', label: 'Trg Val Window' },
    { value: 'VPROBE_PILEUP', label: 'Pileup' },
    { value: 'VPROBE_COINCIDENCE', label: 'Coincidence' },
    { value: 'VPROBE_TRIGGER', label: 'Trigger' },
  ],
  // probe_3 (Digital Probe 2)
  [
    { value: 'VPROBE_GATESHORT', label: 'Gate Short' },
    { value: 'VPROBE_OVERTHRESHOLD', label: 'Over Threshold' },
    { value: 'VPROBE_TRGVAL', label: 'Trg Validation' },
    { value: 'VPROBE_TRGHOLDOFF', label: 'Trg Holdoff' },
    { value: 'VPROBE_PILEUP_TRIGGER', label: 'Pileup Trigger' },
    { value: 'VPROBE_TRIGGER', label: 'Trigger' },
  ],
];

const PHA1_PROBE_OPTIONS: ProbeOption[][] = [
  // probe_0 (Analog Probe 1)
  [
    { value: 'VPROBE_INPUT', label: 'Input' },
    { value: 'VPROBE_DELTA', label: 'Delta' },
    { value: 'VPROBE_DELTA2', label: 'Delta\u00B2' },
    { value: 'VPROBE_TRAPEZOID', label: 'Trapezoid' },
  ],
  // probe_1 (Analog Probe 2)
  [
    { value: 'VPROBE_NONE', label: 'None' },
    { value: 'VPROBE_INPUT', label: 'Input' },
    { value: 'VPROBE_THRESHOLD', label: 'Threshold' },
    { value: 'VPROBE_TRAPCORRECTED', label: 'Trap Corrected' },
    { value: 'VPROBE_BASELINE', label: 'Baseline' },
  ],
  // probe_2 (Digital Probe 0) — PHA1: fixed Tn (trigger flag), not configurable
  [
    { value: 'VPROBE_TRIGGER', label: 'Trigger (fixed)' },
  ],
  // probe_3 (Digital Probe 2)
  [
    { value: 'VPROBE_ARMED', label: 'Armed' },
    { value: 'VPROBE_PKRUN', label: 'Peak Run' },
    { value: 'VPROBE_PILEUP', label: 'Pileup' },
    { value: 'VPROBE_PEAKING', label: 'Peaking' },
    { value: 'VPROBE_TRGVALWIN', label: 'Trg Val Window' },
    { value: 'VPROBE_TRGHOLDOFF', label: 'Trg Holdoff' },
    { value: 'VPROBE_TRGVAL', label: 'Trg Validation' },
    { value: 'VPROBE_ACQVETO', label: 'Acq Veto' },
    { value: 'VPROBE_EXTTRG', label: 'Ext Trigger' },
    { value: 'VPROBE_BUSY', label: 'Busy' },
  ],
];

const PROBE_OPTIONS: Record<string, ProbeOption[][]> = {
  PSD1: PSD1_PROBE_OPTIONS,
  PHA1: PHA1_PROBE_OPTIONS,
};

/** Get Virtual Probe options for a firmware and probe index (0-3) */
export function getProbeOptions(fw: FirmwareType, probeIndex: number): ProbeOption[] {
  return PROBE_OPTIONS[fw]?.[probeIndex] ?? [];
}
