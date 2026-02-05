/**
 * Shared channel parameter definitions for digitizer settings.
 * Used by both digitizer-settings component and Tune Up mode.
 * Source: docs/compass_devtree_mapping.md
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
  { key: 'dc_offset', label: 'DC Offset', type: 'number', unit: '%', min: 0, max: 100, setInRun: true },
  { key: 'vga_gain', label: 'VGA Gain', type: 'number', unit: 'dB', min: 0, max: 29, setInRun: true },
  { key: 'baseline_avg', label: 'N Samples Baseline', type: 'enum', options: ['Fixed', 'Low', 'MediumLow', 'MediumHigh', 'High'] },
  { key: 'fixed_baseline', label: 'Fixed Baseline', type: 'number', unit: 'ADC', min: 0, max: 16383, setInRun: true },
  { key: 'record_length_ns', label: 'Record Length', type: 'number', unit: 'ns', min: 32, max: 16200 },
  { key: 'pre_trigger_ns', label: 'Pre-trigger', type: 'number', unit: 'ns', min: 32, max: 8000, setInRun: true },
  { key: 'wave_downsampling', label: 'Wave Downsampling', type: 'enum', options: ['1', '2', '4', '8'] },
];

const PSD1_INPUT_PARAMS: ChannelParamDef[] = [
  { key: 'enabled', label: 'Enable', type: 'boolean', setInRun: true },
  { key: 'polarity', label: 'Polarity', type: 'enum', options: ['POLARITY_POSITIVE', 'POLARITY_NEGATIVE'], setInRun: true },
  { key: 'dc_offset', label: 'DC Offset', type: 'number', unit: '%', min: 0, max: 100, setInRun: true },
  { key: 'input_dynamic', label: 'Input Dynamic', type: 'enum', options: ['INDYN_2_0_VPP', 'INDYN_0_5_VPP'], setInRun: true },
  { key: 'baseline_avg', label: 'N Samples Baseline', type: 'enum', options: ['BLINE_NSMEAN_FIXED', 'BLINE_NSMEAN_16', 'BLINE_NSMEAN_64', 'BLINE_NSMEAN_256', 'BLINE_NSMEAN_1024'] },
  { key: 'fixed_baseline', label: 'Fixed Baseline', type: 'number', min: 0, max: 16383, setInRun: true },
  { key: 'pre_trigger_ns', label: 'Pre-trigger', type: 'number', unit: 'ns', min: 80, max: 4032 },
];

const PHA_INPUT_PARAMS: ChannelParamDef[] = [
  { key: 'enabled', label: 'Enable', type: 'boolean', setInRun: true },
  { key: 'polarity', label: 'Polarity', type: 'enum', options: ['POLARITY_POSITIVE', 'POLARITY_NEGATIVE'], setInRun: true },
  { key: 'dc_offset', label: 'DC Offset', type: 'number', unit: '%', min: 0, max: 100, setInRun: true },
  { key: 'coarse_gain', label: 'Coarse Gain', type: 'enum', options: ['COARSE_GAIN_X1', 'COARSE_GAIN_X4'], setInRun: true },
  { key: 'baseline_avg', label: 'N Samples Baseline', type: 'enum', options: ['BLINE_NSMEAN_FIXED', 'BLINE_NSMEAN_16', 'BLINE_NSMEAN_64', 'BLINE_NSMEAN_256', 'BLINE_NSMEAN_1024', 'BLINE_NSMEAN_4096', 'BLINE_NSMEAN_16384'] },
  { key: 'pre_trigger_ns', label: 'Pre-trigger', type: 'number', unit: 'ns', min: 128, max: 4000 },
];

// --- Trigger -----------------------------------------------------------------

const PSD2_TRIGGER_PARAMS: ChannelParamDef[] = [
  { key: 'discriminator_mode', label: 'Discriminator', type: 'enum', options: ['LeadingEdge', 'CFD'] },
  { key: 'trigger_threshold', label: 'Threshold', type: 'number', unit: 'ADC', min: 1, max: 8191, setInRun: true },
  { key: 'cfd_delay_ns', label: 'CFD Delay', type: 'number', unit: 'ns', min: 2, max: 2040 },
  { key: 'cfd_fraction', label: 'CFD Fraction', type: 'enum', options: ['25', '50', '75', '100'] },
  { key: 'trigger_holdoff_ns', label: 'Trigger Holdoff', type: 'number', unit: 'ns', min: 8, max: 8000 },
  { key: 'smoothing_factor', label: 'Smoothing Factor', type: 'enum', options: ['1', '2', '4', '8', '16'], setInRun: true },
  { key: 'time_filter_smoothing', label: 'Time Filter Smooth', type: 'enum', options: ['Enabled', 'Disabled'], setInRun: true },
  { key: 'event_trigger_source', label: 'Event Trigger', type: 'enum', options: ['Disabled', 'Ch64Trigger', 'ChSelfTrigger', 'SwTrg', 'TRGIN', 'GlobalTriggerSource', 'LVDS', 'ITLA', 'ITLB'], setInRun: true },
  { key: 'wave_trigger_source', label: 'Wave Trigger', type: 'enum', options: ['Disabled', 'Ch64Trigger', 'ITLA', 'ITLB', 'ChSelfTrigger', 'SwTrg', 'ADCOverSaturation', 'ADCUnderSaturation', 'ExternalInhibit', 'TRGIN', 'GlobalTriggerSource', 'LVDS'], setInRun: true },
];

const PSD1_TRIGGER_PARAMS: ChannelParamDef[] = [
  { key: 'discriminator_mode', label: 'Discriminator', type: 'enum', options: ['DISCR_MODE_LED', 'DISCR_MODE_CFD'], setInRun: true },
  { key: 'trigger_threshold', label: 'Threshold', type: 'number', unit: 'LSB', min: 0, max: 16383, setInRun: true },
  { key: 'cfd_delay_ns', label: 'CFD Delay', type: 'number', unit: 'ns', min: 0, max: 1020, setInRun: true },
  { key: 'cfd_fraction', label: 'CFD Fraction', type: 'enum', options: ['CFD_FRACTLIST_25', 'CFD_FRACTLIST_50', 'CFD_FRACTLIST_75', 'CFD_FRACTLIST_100'], setInRun: true },
  { key: 'input_smoothing', label: 'Input Smoothing', type: 'enum', options: ['CFD_SMOOTH_EXP_1', 'CFD_SMOOTH_EXP_2', 'CFD_SMOOTH_EXP_4', 'CFD_SMOOTH_EXP_8', 'CFD_SMOOTH_EXP_16'] },
  { key: 'trigger_holdoff_ns', label: 'Trigger Holdoff', type: 'number', unit: 'ns', min: 0, max: 1048560, setInRun: true },
  { key: 'self_trigger', label: 'Self Trigger', type: 'enum', options: ['FALSE', 'TRUE'], setInRun: true },
  { key: 'global_trigger_gen', label: 'Global Trigger Gen', type: 'enum', options: ['FALSE', 'TRUE'], setInRun: true },
  { key: 'trigger_out_propagate', label: 'Trigger Out Prop.', type: 'enum', options: ['FALSE', 'TRUE'], setInRun: true },
];

const PHA_TRIGGER_PARAMS: ChannelParamDef[] = [
  { key: 'trigger_threshold', label: 'Threshold', type: 'number', unit: 'LSB', min: 0, max: 16383, setInRun: true },
  { key: 'trigger_holdoff_ns', label: 'Trigger Holdoff', type: 'number', unit: 'ns', min: 16, max: 16368, setInRun: true },
  { key: 'fast_discr_smoothing', label: 'Fast Discr Smooth', type: 'enum', options: ['RCCR2_SMTH_1', 'RCCR2_SMTH_2', 'RCCR2_SMTH_4', 'RCCR2_SMTH_8', 'RCCR2_SMTH_16', 'RCCR2_SMTH_32', 'RCCR2_SMTH_64', 'RCCR2_SMTH_128'] },
  { key: 'input_rise_time_ns', label: 'Input Rise Time', type: 'number', unit: 'ns', min: 32, max: 4080 },
  { key: 'self_trigger', label: 'Self Trigger', type: 'enum', options: ['FALSE', 'TRUE'], setInRun: true },
  { key: 'global_trigger_gen', label: 'Global Trigger Gen', type: 'enum', options: ['FALSE', 'TRUE'], setInRun: true },
  { key: 'trigger_out_propagate', label: 'Trigger Out Prop.', type: 'enum', options: ['FALSE', 'TRUE'], setInRun: true },
];

// --- Energy ------------------------------------------------------------------

const PSD2_ENERGY_PARAMS: ChannelParamDef[] = [
  { key: 'energy_coarse_gain', label: 'Energy Coarse Gain', type: 'enum', options: ['x1', 'x4', 'x16', 'x64', 'x256'] },
  { key: 'gate_long_ns', label: 'Gate Long', type: 'number', unit: 'ns', min: 2, max: 8000 },
  { key: 'gate_short_ns', label: 'Gate Short', type: 'number', unit: 'ns', min: 2, max: 8000 },
  { key: 'gate_pre_ns', label: 'Pre-gate', type: 'number', unit: 'ns', min: 16, max: 2000 },
  { key: 'charge_pedestal', label: 'Charge Pedestal', type: 'number', unit: 'count', min: 0, max: 1000 },
  { key: 'short_charge_pedestal', label: 'Short Charge Ped.', type: 'number', unit: 'count', min: 0, max: 1000 },
  { key: 'charge_smoothing', label: 'Charge Smoothing', type: 'enum', options: ['Enabled', 'Disabled'], setInRun: true },
];

const PSD1_ENERGY_PARAMS: ChannelParamDef[] = [
  { key: 'energy_coarse_gain', label: 'Energy Coarse Gain', type: 'enum', options: ['CHARGESENS_2.5_FC_LSB_VPP', 'CHARGESENS_10_FC_LSB_VPP', 'CHARGESENS_40_FC_LSB_VPP', 'CHARGESENS_160_FC_LSB_VPP', 'CHARGESENS_640_FC_LSB_VPP', 'CHARGESENS_2560_FC_LSB_VPP'], setInRun: true },
  { key: 'gate_long_ns', label: 'Gate Long', type: 'number', unit: 'ns', min: 8, max: 65532, setInRun: true },
  { key: 'gate_short_ns', label: 'Gate Short', type: 'number', unit: 'ns', min: 4, max: 4092, setInRun: true },
  { key: 'gate_pre_ns', label: 'Pre-gate', type: 'number', unit: 'ns', min: 0, max: 1020 },
  { key: 'charge_pedestal_en', label: 'Charge Pedestal', type: 'enum', options: ['FALSE', 'TRUE'] },
];

const PHA_ENERGY_PARAMS: ChannelParamDef[] = [
  { key: 'trap_rise_time_ns', label: 'Trap Rise Time', type: 'number', unit: 'ns', min: 16, max: 65520 },
  { key: 'trap_flat_top_ns', label: 'Trap Flat Top', type: 'number', unit: 'ns', min: 16, max: 16368 },
  { key: 'trap_pole_zero_ns', label: 'Trap Pole Zero', type: 'number', unit: 'ns', min: 16, max: 1048560 },
  { key: 'peaking_time', label: 'Peaking Time', type: 'number', unit: '%', min: 0, max: 100, setInRun: true },
  { key: 'peak_nsmean', label: 'N Samples Peak', type: 'enum', options: ['PEAK_NSMEAN_1', 'PEAK_NSMEAN_4', 'PEAK_NSMEAN_16', 'PEAK_NSMEAN_64'] },
  { key: 'peak_holdoff_ns', label: 'Peak Holdoff', type: 'number', unit: 'ns', min: 16, max: 16368 },
  { key: 'energy_fine_gain', label: 'Energy Fine Gain', type: 'number', min: 1.0, max: 10.0, setInRun: true },
];

// --- Coincidence -------------------------------------------------------------

const PSD2_COINCIDENCE_PARAMS: ChannelParamDef[] = [
  { key: 'ch_trigger_mask', label: 'Ch Trigger Mask', type: 'enum', options: [], setInRun: true },
  { key: 'coincidence_mask', label: 'Coincidence Mask', type: 'enum', options: ['Disabled', 'Ch64Trigger', 'TRGIN', 'GlobalTriggerSource', 'ITLA', 'ITLB'], setInRun: true },
  { key: 'anti_coincidence_mask', label: 'Anti-coinc Mask', type: 'enum', options: ['Disabled', 'Ch64Trigger', 'TRGIN', 'GlobalTriggerSource', 'ITLA', 'ITLB'], setInRun: true },
  { key: 'coincidence_window_ns', label: 'Coinc Window', type: 'number', unit: 'ns', min: 0, max: 524280, setInRun: true },
  { key: 'ch_veto_source', label: 'Veto Source', type: 'enum', options: ['Disabled', 'BoardVeto', 'ADCOverSaturation', 'ADCUnderSaturation'], setInRun: true },
  { key: 'ch_veto_width_ns', label: 'Veto Width', type: 'number', unit: 'ns', min: 0, max: 524280, setInRun: true },
  { key: 'event_selector', label: 'Event Selector', type: 'enum', options: ['All', 'PileUp', 'EnergySkim'], setInRun: true },
];

const PSD1_COINCIDENCE_PARAMS: ChannelParamDef[] = [
  { key: 'coincidence_mode', label: 'Coincidence Mode', type: 'enum', options: ['TRIGGER_MODE_NORMAL', 'TRIGGER_MODE_COINC', 'TRIGGER_MODE_ANTICOINC'] },
  { key: 'ch_veto_source', label: 'Veto Source', type: 'enum', options: ['VETO_SRC_DISABLED', 'VETO_SRC_COMMON', 'VETO_SRC_INDIVIDUAL', 'VETO_SRC_SATURATION'], setInRun: true },
  { key: 'pileup_rejection', label: 'Pileup Rejection', type: 'enum', options: ['FALSE', 'TRUE'], setInRun: true },
];

const PHA_COINCIDENCE_PARAMS: ChannelParamDef[] = [
  { key: 'coincidence_mode', label: 'Coincidence Mode', type: 'enum', options: ['TRIGGER_MODE_NORMAL', 'TRIGGER_MODE_COINC', 'TRIGGER_MODE_ANTICOINC'] },
  { key: 'ch_veto_source', label: 'Veto Source', type: 'enum', options: ['VETO_SRC_DISABLED', 'VETO_SRC_COMMON', 'VETO_SRC_INDIVIDUAL', 'VETO_SRC_SATURATION'], setInRun: true },
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

const PHA_WAVEFORM_PARAMS: ChannelParamDef[] = [];

// --- Category lookup ---------------------------------------------------------

export type ChannelCategory = 'input' | 'trigger' | 'energy' | 'coincidence' | 'waveform';

const CATEGORY_PARAMS: Record<FirmwareType, Record<ChannelCategory, ChannelParamDef[]>> = {
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
  PHA: {
    input: PHA_INPUT_PARAMS,
    trigger: PHA_TRIGGER_PARAMS,
    energy: PHA_ENERGY_PARAMS,
    coincidence: PHA_COINCIDENCE_PARAMS,
    waveform: PHA_WAVEFORM_PARAMS,
  },
};

/** Get channel params for a specific firmware and category */
export function getCategoryParams(fw: FirmwareType, category: ChannelCategory): ChannelParamDef[] {
  return CATEGORY_PARAMS[fw]?.[category] ?? [];
}

/** Get all channel params across all categories for a firmware */
export function getAllChannelParams(fw: FirmwareType): ChannelParamDef[] {
  const cats = CATEGORY_PARAMS[fw];
  if (!cats) return [];
  return [...cats.input, ...cats.trigger, ...cats.energy, ...cats.coincidence, ...cats.waveform];
}
