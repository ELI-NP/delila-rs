import { Component, inject, signal, computed, effect } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { MatCardModule } from '@angular/material/card';
import { MatSelectModule } from '@angular/material/select';
import { MatInputModule } from '@angular/material/input';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatButtonModule } from '@angular/material/button';
import { MatSlideToggleModule } from '@angular/material/slide-toggle';
import { MatIconModule } from '@angular/material/icon';
import { MatSnackBar, MatSnackBarModule } from '@angular/material/snack-bar';
import { MatDividerModule } from '@angular/material/divider';
import { MatTabsModule } from '@angular/material/tabs';
import { MatTooltipModule } from '@angular/material/tooltip';
import { MatProgressSpinnerModule } from '@angular/material/progress-spinner';
import { DigitizerService } from '../../services/digitizer.service';
import { OperatorService } from '../../services/operator.service';
import { FirmwareType } from '../../models/types';
import {
  ChannelTableComponent,
  ChannelParamDef,
  DefaultValueChange,
  ChannelValueChange,
} from '../channel-table/channel-table.component';

// =============================================================================
// Channel Parameter Definitions — 5 categories × 3 firmware types
// Source: docs/compass_devtree_mapping.md
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
  { key: 'pre_trigger', label: 'Pre-trigger', type: 'number', unit: 'samples', min: 40, max: 2016 },
];

const PHA_INPUT_PARAMS: ChannelParamDef[] = [
  { key: 'enabled', label: 'Enable', type: 'boolean', setInRun: true },
  { key: 'polarity', label: 'Polarity', type: 'enum', options: ['POLARITY_POSITIVE', 'POLARITY_NEGATIVE'], setInRun: true },
  { key: 'dc_offset', label: 'DC Offset', type: 'number', unit: '%', min: 0, max: 100, setInRun: true },
  { key: 'coarse_gain', label: 'Coarse Gain', type: 'enum', options: ['COARSE_GAIN_X1', 'COARSE_GAIN_X4'], setInRun: true },
  { key: 'baseline_avg', label: 'N Samples Baseline', type: 'enum', options: ['BLINE_NSMEAN_FIXED', 'BLINE_NSMEAN_16', 'BLINE_NSMEAN_64', 'BLINE_NSMEAN_256', 'BLINE_NSMEAN_1024', 'BLINE_NSMEAN_4096', 'BLINE_NSMEAN_16384'] },
  { key: 'pre_trigger', label: 'Pre-trigger', type: 'number', unit: 'samples', min: 64, max: 2000 },
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
  { key: 'cfd_delay_ns', label: 'CFD Delay', type: 'number', unit: 'samples', min: 0, max: 510, setInRun: true },
  { key: 'cfd_fraction', label: 'CFD Fraction', type: 'enum', options: ['CFD_FRACTLIST_25', 'CFD_FRACTLIST_50', 'CFD_FRACTLIST_75', 'CFD_FRACTLIST_100'], setInRun: true },
  { key: 'input_smoothing', label: 'Input Smoothing', type: 'enum', options: ['CFD_SMOOTH_EXP_1', 'CFD_SMOOTH_EXP_2', 'CFD_SMOOTH_EXP_4', 'CFD_SMOOTH_EXP_8', 'CFD_SMOOTH_EXP_16'] },
  { key: 'trigger_holdoff', label: 'Trigger Holdoff', type: 'number', unit: 'samples', min: 0, max: 524280, setInRun: true },
  { key: 'self_trigger', label: 'Self Trigger', type: 'enum', options: ['FALSE', 'TRUE'], setInRun: true },
  { key: 'global_trigger_gen', label: 'Global Trigger Gen', type: 'enum', options: ['FALSE', 'TRUE'], setInRun: true },
  { key: 'trigger_out_propagate', label: 'Trigger Out Prop.', type: 'enum', options: ['FALSE', 'TRUE'], setInRun: true },
];

const PHA_TRIGGER_PARAMS: ChannelParamDef[] = [
  { key: 'trigger_threshold', label: 'Threshold', type: 'number', unit: 'LSB', min: 0, max: 16383, setInRun: true },
  { key: 'trigger_holdoff', label: 'Trigger Holdoff', type: 'number', unit: 'samples', min: 8, max: 8184, setInRun: true },
  { key: 'fast_discr_smoothing', label: 'Fast Discr Smooth', type: 'enum', options: ['RCCR2_SMTH_1', 'RCCR2_SMTH_2', 'RCCR2_SMTH_4', 'RCCR2_SMTH_8', 'RCCR2_SMTH_16', 'RCCR2_SMTH_32', 'RCCR2_SMTH_64', 'RCCR2_SMTH_128'] },
  { key: 'input_rise_time', label: 'Input Rise Time', type: 'number', unit: 'samples', min: 16, max: 2040 },
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
  { key: 'gate_long_ns', label: 'Gate Long', type: 'number', unit: 'samples', min: 4, max: 32766, setInRun: true },
  { key: 'gate_short_ns', label: 'Gate Short', type: 'number', unit: 'samples', min: 2, max: 2046, setInRun: true },
  { key: 'gate_pre_ns', label: 'Pre-gate', type: 'number', unit: 'samples', min: 0, max: 510 },
  { key: 'charge_pedestal_en', label: 'Charge Pedestal', type: 'enum', options: ['FALSE', 'TRUE'] },
];

const PHA_ENERGY_PARAMS: ChannelParamDef[] = [
  { key: 'trap_rise_time', label: 'Trap Rise Time', type: 'number', unit: 'samples', min: 8, max: 32760 },
  { key: 'trap_flat_top', label: 'Trap Flat Top', type: 'number', unit: 'samples', min: 8, max: 8184 },
  { key: 'trap_pole_zero', label: 'Trap Pole Zero', type: 'number', unit: 'samples', min: 8, max: 524280 },
  { key: 'peaking_time', label: 'Peaking Time', type: 'number', unit: '%', min: 0, max: 100, setInRun: true },
  { key: 'peak_nsmean', label: 'N Samples Peak', type: 'enum', options: ['PEAK_NSMEAN_1', 'PEAK_NSMEAN_4', 'PEAK_NSMEAN_16', 'PEAK_NSMEAN_64'] },
  { key: 'peak_holdoff', label: 'Peak Holdoff', type: 'number', unit: 'samples', min: 8, max: 8184 },
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

type ChannelCategory = 'input' | 'trigger' | 'energy' | 'coincidence' | 'waveform';

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

function getCategoryParams(fw: FirmwareType, category: ChannelCategory): ChannelParamDef[] {
  return CATEGORY_PARAMS[fw]?.[category] ?? [];
}

/** Get all channel params across all categories for a firmware (for disabledKeys computation) */
function getAllChannelParams(fw: FirmwareType): ChannelParamDef[] {
  const cats = CATEGORY_PARAMS[fw];
  if (!cats) return [];
  return [...cats.input, ...cats.trigger, ...cats.energy, ...cats.coincidence, ...cats.waveform];
}

@Component({
  selector: 'app-digitizer-settings',
  standalone: true,
  imports: [
    CommonModule,
    FormsModule,
    MatCardModule,
    MatSelectModule,
    MatInputModule,
    MatFormFieldModule,
    MatButtonModule,
    MatSlideToggleModule,
    MatIconModule,
    MatSnackBarModule,
    MatDividerModule,
    MatTabsModule,
    MatProgressSpinnerModule,
    MatTooltipModule,
    ChannelTableComponent,
  ],
  template: `
    <div class="digitizer-settings">
      <!-- Header: Digitizer selector + firmware badge + action buttons -->
      <div class="header-row">
        <mat-form-field appearance="outline" class="digitizer-select">
          <mat-label>Select Digitizer</mat-label>
          <mat-select [value]="selectedId()" (selectionChange)="onDigitizerChange($event.value)">
            @for (dig of digitizers(); track dig.digitizer_id) {
              <mat-option [value]="dig.digitizer_id">
                {{ dig.name }} (ID: {{ dig.digitizer_id }})
              </mat-option>
            }
          </mat-select>
        </mat-form-field>

        @if (selectedConfig(); as config) {
          <mat-form-field appearance="outline" class="name-input">
            <mat-label>Name</mat-label>
            <input matInput [(ngModel)]="config.name" />
          </mat-form-field>

          <span class="firmware-badge" [class]="config.firmware.toLowerCase()">
            {{ config.firmware }}
          </span>
          @if (config.serial_number) {
            <span class="serial-info">S/N: {{ config.serial_number }}</span>
          }
        }

        <span class="spacer"></span>

        <button mat-button (click)="onDetect()" [disabled]="detecting()">
          @if (detecting()) {
            <mat-spinner diameter="18" class="inline-spinner"></mat-spinner>
          } @else {
            <mat-icon>search</mat-icon>
          }
          Detect
        </button>
        <button mat-button (click)="resetConfig()" [disabled]="!selectedConfig()">
          <mat-icon>refresh</mat-icon>
          Reset
        </button>
        <button
          mat-raised-button
          color="primary"
          (click)="applyConfig()"
          [disabled]="!selectedConfig()"
          [matTooltip]="isRunning() ? 'Only SetInRun parameters will be applied' : ''"
        >
          <mat-icon>check</mat-icon>
          {{ isRunning() ? 'Apply (Runtime)' : 'Apply' }}
        </button>
        <button
          mat-raised-button
          color="accent"
          (click)="saveConfig()"
          [disabled]="!selectedConfig()"
        >
          <mat-icon>save</mat-icon>
          Save
        </button>
      </div>

      @if (selectedConfig(); as config) {
        <!-- 6-tab layout: Board / Input / Trigger / Energy / Coincidence / Waveform -->
        <mat-tab-group animationDuration="0ms">
          <!-- Tab 1: Board Settings -->
          <mat-tab label="Board">
            <div class="tab-content">
              <mat-card class="config-card">
                <mat-card-content>
                  <h3 class="section-title">Clock &amp; Sync</h3>
                  <div class="form-grid">
                    <mat-form-field appearance="outline">
                      <mat-label>Start Source</mat-label>
                      <mat-select [(value)]="config.board.start_source">
                        @if (config.firmware === 'PSD2') {
                          <mat-option value="EncodedClkIn">EncodedClkIn</mat-option>
                          <mat-option value="SINlevel">SINlevel</mat-option>
                          <mat-option value="SINedge">SINedge</mat-option>
                          <mat-option value="SWcmd">SWcmd</mat-option>
                          <mat-option value="LVDS">LVDS</mat-option>
                          <mat-option value="P0">P0</mat-option>
                        } @else {
                          <mat-option value="START_MODE_SW">Software</mat-option>
                          <mat-option value="START_MODE_S_IN">S-IN</mat-option>
                          <mat-option value="START_MODE_FIRST_TRG">First Trigger</mat-option>
                        }
                      </mat-select>
                    </mat-form-field>

                    @if (config.firmware === 'PSD2') {
                      <mat-form-field appearance="outline">
                        <mat-label>Clock Source</mat-label>
                        <mat-select [(value)]="config.board.extra!['clocksource']">
                          <mat-option value="Internal">Internal</mat-option>
                          <mat-option value="FPClkIn">FPClkIn</mat-option>
                        </mat-select>
                      </mat-form-field>

                      <mat-form-field appearance="outline">
                        <mat-label>Output Clock</mat-label>
                        <mat-select [(value)]="config.board.extra!['enclockoutfp']">
                          <mat-option value="True">Enabled</mat-option>
                          <mat-option value="False">Disabled</mat-option>
                        </mat-select>
                      </mat-form-field>

                      <mat-form-field appearance="outline">
                        <mat-label>SyncOut Signal</mat-label>
                        <mat-select [(value)]="config.board.extra!['syncoutmode']">
                          <mat-option value="Disabled">Disabled</mat-option>
                          <mat-option value="SyncIn">SyncIn</mat-option>
                          <mat-option value="TestPulse">TestPulse</mat-option>
                          <mat-option value="IntClk">IntClk</mat-option>
                          <mat-option value="Run">Run</mat-option>
                          <mat-option value="User">User</mat-option>
                        </mat-select>
                      </mat-form-field>

                      <mat-form-field appearance="outline">
                        <mat-label>Start Delay (ns)</mat-label>
                        <input matInput type="number" [(ngModel)]="config.board.extra!['rundelay']" min="0" max="524280" />
                      </mat-form-field>
                    } @else {
                      <mat-form-field appearance="outline">
                        <mat-label>Ext Clock</mat-label>
                        <mat-select [(value)]="config.board.extra!['dt_ext_clock']">
                          <mat-option value="FALSE">Disabled</mat-option>
                          <mat-option value="TRUE">Enabled</mat-option>
                        </mat-select>
                      </mat-form-field>

                      <mat-form-field appearance="outline">
                        <mat-label>Start Delay (samples)</mat-label>
                        <input matInput type="number" [(ngModel)]="config.board.extra!['start_delay']" min="0" max="4080" />
                      </mat-form-field>
                    }
                  </div>

                  <mat-divider></mat-divider>
                  <h3 class="section-title">Trigger &amp; I/O</h3>
                  <div class="form-grid">
                    <mat-form-field appearance="outline">
                      <mat-label>Global Trigger Source</mat-label>
                      <mat-select [(value)]="config.board.global_trigger_source">
                        @if (config.firmware === 'PSD2') {
                          <mat-option value="TrgIn">TrgIn</mat-option>
                          <mat-option value="P0">P0</mat-option>
                          <mat-option value="TestPulse">TestPulse</mat-option>
                          <mat-option value="UserTrg">UserTrg</mat-option>
                          <mat-option value="SwTrg">SwTrg</mat-option>
                          <mat-option value="LVDS">LVDS</mat-option>
                          <mat-option value="ITLA">ITLA</mat-option>
                          <mat-option value="ITLB">ITLB</mat-option>
                          <mat-option value="ITLA_AND_ITLB">ITLA_AND_ITLB</mat-option>
                          <mat-option value="ITLA_OR_ITLB">ITLA_OR_ITLB</mat-option>
                          <mat-option value="EncodedClkIn">EncodedClkIn</mat-option>
                          <mat-option value="GPIO">GPIO</mat-option>
                        } @else {
                          <mat-option value="SwTrg">Software Trigger</mat-option>
                          <mat-option value="TestPulse">Test Pulse</mat-option>
                          <mat-option value="ITLA">Internal Trigger</mat-option>
                        }
                      </mat-select>
                    </mat-form-field>

                    <mat-form-field appearance="outline">
                      <mat-label>FPIO Type</mat-label>
                      <mat-select [(value)]="config.board.extra!['iolevel']">
                        @if (config.firmware === 'PSD2') {
                          <mat-option value="NIM">NIM</mat-option>
                          <mat-option value="TTL">TTL</mat-option>
                        } @else {
                          <mat-option value="FPIOTYPE_NIM">NIM</mat-option>
                          <mat-option value="FPIOTYPE_TTL">TTL</mat-option>
                        }
                      </mat-select>
                    </mat-form-field>

                    <mat-form-field appearance="outline">
                      <mat-label>GPO Mode</mat-label>
                      <mat-select [(value)]="config.board.gpio_mode">
                        @for (opt of gpoModeOptions(config.firmware); track opt) {
                          <mat-option [value]="opt">{{ opt }}</mat-option>
                        }
                      </mat-select>
                    </mat-form-field>

                    @if (config.firmware === 'PSD2') {
                      <mat-form-field appearance="outline">
                        <mat-label>TRG OUT Mode</mat-label>
                        <mat-select [(value)]="config.board.extra!['trgoutmode']">
                          @for (opt of trgoutModeOptions(); track opt) {
                            <mat-option [value]="opt">{{ opt }}</mat-option>
                          }
                        </mat-select>
                      </mat-form-field>
                    } @else {
                      <mat-form-field appearance="outline">
                        <mat-label>TRG OUT / GPO</mat-label>
                        <mat-select [(value)]="config.board.extra!['out_selection']">
                          @for (opt of outSelectionOptions(); track opt) {
                            <mat-option [value]="opt">{{ opt }}</mat-option>
                          }
                        </mat-select>
                      </mat-form-field>
                    }
                  </div>

                  <mat-divider></mat-divider>
                  <h3 class="section-title">Test Pulse</h3>
                  <div class="form-grid">
                    <mat-form-field appearance="outline">
                      <mat-label>Test Pulse Period (ns)</mat-label>
                      <input matInput type="number" [(ngModel)]="config.board.test_pulse_period" />
                    </mat-form-field>

                    <mat-form-field appearance="outline">
                      <mat-label>Test Pulse Width (ns)</mat-label>
                      <input matInput type="number" [(ngModel)]="config.board.test_pulse_width" />
                    </mat-form-field>
                  </div>

                  @if (config.firmware === 'PSD2') {
                    <mat-divider></mat-divider>
                    <h3 class="section-title">Board Veto</h3>
                    <div class="form-grid">
                      <mat-form-field appearance="outline">
                        <mat-label>Veto Source</mat-label>
                        <mat-select [(value)]="config.board.extra!['boardvetosource']">
                          <mat-option value="Disabled">Disabled</mat-option>
                          <mat-option value="SIN">SIN</mat-option>
                          <mat-option value="GPIO">GPIO</mat-option>
                          <mat-option value="LVDS">LVDS</mat-option>
                          <mat-option value="P0">P0</mat-option>
                          <mat-option value="EncodedClkIn">EncodedClkIn</mat-option>
                        </mat-select>
                      </mat-form-field>

                      <mat-form-field appearance="outline">
                        <mat-label>Veto Polarity</mat-label>
                        <mat-select [(value)]="config.board.extra!['boardvetopolarity']">
                          <mat-option value="ActiveHigh">ActiveHigh</mat-option>
                          <mat-option value="ActiveLow">ActiveLow</mat-option>
                        </mat-select>
                      </mat-form-field>

                      <mat-form-field appearance="outline">
                        <mat-label>Veto Width (ns)</mat-label>
                        <input matInput type="number" [(ngModel)]="config.board.extra!['boardvetowidth']" min="0" />
                      </mat-form-field>
                    </div>
                  }

                  <mat-divider></mat-divider>
                  <h3 class="section-title">Data Acquisition</h3>
                  <div class="form-grid">
                    <mat-form-field appearance="outline">
                      <mat-label>Record Length {{ config.firmware === 'PSD2' ? '(ns)' : '(samples)' }}</mat-label>
                      <input matInput type="number" [(ngModel)]="config.board.record_length" />
                    </mat-form-field>

                    <mat-slide-toggle [(ngModel)]="config.board.waveforms_enabled">
                      Enable Waveforms
                    </mat-slide-toggle>

                    @if (config.firmware !== 'PSD2') {
                      <mat-form-field appearance="outline">
                        <mat-label>Extras</mat-label>
                        <mat-select [(value)]="config.board.extra!['extras']">
                          <mat-option value="TRUE">Enabled</mat-option>
                          <mat-option value="FALSE">Disabled</mat-option>
                        </mat-select>
                      </mat-form-field>

                      <mat-form-field appearance="outline">
                        <mat-label>Event Aggregation</mat-label>
                        <input matInput type="number" [(ngModel)]="config.board.extra!['eventaggr']" min="1" max="1023" />
                      </mat-form-field>

                      <mat-form-field appearance="outline">
                        <mat-label>Coincidence Window (samples)</mat-label>
                        <input matInput type="number" [(ngModel)]="config.board.extra!['coinc_trgout']" min="0" max="8184" />
                      </mat-form-field>
                    }
                  </div>
                </mat-card-content>
              </mat-card>
            </div>
          </mat-tab>

          <!-- Tab 2: Input -->
          <mat-tab label="Input">
            <div class="tab-content">
              <app-channel-table
                [params]="inputParams()"
                [numChannels]="config.num_channels"
                [defaultValues]="defaultValues()"
                [channelValues]="channelValues()"
                [disabledKeys]="disabledKeys()"
                (defaultChange)="onDefaultChange($event)"
                (channelChange)="onChannelChange($event)"
              />
            </div>
          </mat-tab>

          <!-- Tab 3: Trigger -->
          <mat-tab label="Trigger">
            <div class="tab-content">
              <app-channel-table
                [params]="triggerParams()"
                [numChannels]="config.num_channels"
                [defaultValues]="defaultValues()"
                [channelValues]="channelValues()"
                [disabledKeys]="disabledKeys()"
                (defaultChange)="onDefaultChange($event)"
                (channelChange)="onChannelChange($event)"
              />
            </div>
          </mat-tab>

          <!-- Tab 4: Energy -->
          <mat-tab label="Energy">
            <div class="tab-content">
              <app-channel-table
                [params]="energyParams()"
                [numChannels]="config.num_channels"
                [defaultValues]="defaultValues()"
                [channelValues]="channelValues()"
                [disabledKeys]="disabledKeys()"
                (defaultChange)="onDefaultChange($event)"
                (channelChange)="onChannelChange($event)"
              />
            </div>
          </mat-tab>

          <!-- Tab 5: Coincidence -->
          <mat-tab label="Coincidence">
            <div class="tab-content">
              <app-channel-table
                [params]="coincidenceParams()"
                [numChannels]="config.num_channels"
                [defaultValues]="defaultValues()"
                [channelValues]="channelValues()"
                [disabledKeys]="disabledKeys()"
                (defaultChange)="onDefaultChange($event)"
                (channelChange)="onChannelChange($event)"
              />
            </div>
          </mat-tab>

          <!-- Tab 6: Waveform (hidden if empty) -->
          @if (waveformParams().length > 0) {
            <mat-tab label="Waveform">
              <div class="tab-content">
                <app-channel-table
                  [params]="waveformParams()"
                  [numChannels]="config.num_channels"
                  [defaultValues]="defaultValues()"
                  [channelValues]="channelValues()"
                  [disabledKeys]="disabledKeys()"
                  (defaultChange)="onDefaultChange($event)"
                  (channelChange)="onChannelChange($event)"
                />
              </div>
            </mat-tab>
          }
        </mat-tab-group>
      } @else {
        <mat-card class="no-selection">
          <mat-card-content>
            <mat-icon>memory</mat-icon>
            <p>Select a digitizer to configure</p>
          </mat-card-content>
        </mat-card>
      }
    </div>
  `,
  styles: `
    .digitizer-settings {
      padding: 16px;
    }

    .header-row {
      display: flex;
      align-items: center;
      gap: 12px;
      margin-bottom: 8px;
      flex-wrap: wrap;
    }

    .digitizer-select {
      width: 280px;
    }

    .name-input {
      width: 200px;
    }

    .firmware-badge {
      padding: 4px 12px;
      border-radius: 12px;
      font-size: 12px;
      font-weight: 500;
      text-transform: uppercase;
    }

    .firmware-badge.psd2 {
      background-color: #e3f2fd;
      color: #1976d2;
    }

    .firmware-badge.psd1 {
      background-color: #fff3e0;
      color: #f57c00;
    }

    .firmware-badge.pha {
      background-color: #e8f5e9;
      color: #388e3c;
    }

    .serial-info {
      font-size: 12px;
      color: #666;
      font-family: monospace;
    }

    .spacer {
      flex: 1;
    }

    .inline-spinner {
      display: inline-block;
      margin-right: 4px;
    }

    .tab-content {
      padding: 16px 0;
    }

    .config-card {
      max-width: 800px;
    }

    .form-grid {
      display: grid;
      grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
      gap: 16px;
      padding: 16px 0;
    }

    .section-title {
      margin: 16px 0 0;
      font-size: 14px;
      font-weight: 500;
      color: #666;
    }

    .no-params-msg {
      color: #999;
      font-style: italic;
      padding: 24px;
    }

    .no-selection {
      max-width: 400px;
      text-align: center;
      padding: 48px;
    }

    .no-selection mat-icon {
      font-size: 48px;
      width: 48px;
      height: 48px;
      opacity: 0.5;
    }

    .no-selection p {
      margin-top: 16px;
      color: rgba(0, 0, 0, 0.54);
    }
  `,
})
export class DigitizerSettingsComponent {
  private readonly digitizerService = inject(DigitizerService);
  private readonly operator = inject(OperatorService);
  private readonly snackBar = inject(MatSnackBar);

  readonly digitizers = this.digitizerService.digitizers;
  readonly selectedId = signal<number | null>(null);
  readonly detecting = signal(false);

  // Expanded channel data (mutable working copy)
  readonly defaultValues = signal<Record<string, unknown>>({});
  readonly channelValues = signal<Record<string, unknown>[]>([]);

  readonly selectedConfig = computed(() => {
    const id = this.selectedId();
    if (id === null) return null;
    return this.digitizers().find((d) => d.digitizer_id === id) ?? null;
  });

  readonly inputParams = computed(() => {
    const config = this.selectedConfig();
    return config ? getCategoryParams(config.firmware, 'input') : [];
  });

  readonly triggerParams = computed(() => {
    const config = this.selectedConfig();
    return config ? getCategoryParams(config.firmware, 'trigger') : [];
  });

  readonly energyParams = computed(() => {
    const config = this.selectedConfig();
    return config ? getCategoryParams(config.firmware, 'energy') : [];
  });

  readonly coincidenceParams = computed(() => {
    const config = this.selectedConfig();
    return config ? getCategoryParams(config.firmware, 'coincidence') : [];
  });

  readonly waveformParams = computed(() => {
    const config = this.selectedConfig();
    return config ? getCategoryParams(config.firmware, 'waveform') : [];
  });

  /** System state from OperatorService (auto-polled) */
  readonly isRunning = computed(() => this.operator.systemState() === 'Running');

  /** Keys of non-SetInRun params to disable when Running */
  readonly disabledKeys = computed(() => {
    if (!this.isRunning()) return [];
    const config = this.selectedConfig();
    if (!config) return [];
    return getAllChannelParams(config.firmware)
      .filter((p) => !p.setInRun)
      .map((p) => p.key);
  });

  constructor() {
    // Load digitizers on init
    this.digitizerService.loadDigitizers();

    // When selected config changes, expand it into flat channel arrays
    effect(() => {
      const config = this.selectedConfig();
      if (config) {
        // Ensure board.extra exists for waveform probe settings
        if (!config.board.extra) {
          config.board.extra = {};
        }
        this.defaultValues.set(this.digitizerService.extractDefaults(config));
        this.channelValues.set(this.digitizerService.expandConfig(config));
      } else {
        this.defaultValues.set({});
        this.channelValues.set([]);
      }
    });
  }

  onDigitizerChange(value: number): void {
    this.selectedId.set(value);
  }

  // ===========================================================================
  // Channel Table Event Handlers
  // ===========================================================================

  /**
   * "All" column changed — update default and propagate to all channels.
   */
  onDefaultChange(event: DefaultValueChange): void {
    const defaults = { ...this.defaultValues() };
    defaults[event.key] = event.value;
    this.defaultValues.set(defaults);

    // Propagate to all channels
    const channels = this.channelValues().map((ch) => ({
      ...ch,
      [event.key]: event.value,
    }));
    this.channelValues.set(channels);
  }

  /**
   * Individual channel changed — update only that channel.
   */
  onChannelChange(event: ChannelValueChange): void {
    const channels = [...this.channelValues()];
    channels[event.channel] = {
      ...channels[event.channel],
      [event.key]: event.value,
    };
    this.channelValues.set(channels);
  }

  // ===========================================================================
  // Board Tab Option Lists (FW-specific)
  // ===========================================================================

  gpoModeOptions(fw: FirmwareType): string[] {
    if (fw === 'PSD2') {
      return ['Disabled', 'TrgIn', 'SwTrg', 'Run', 'RefClk', 'TestPulse', 'Busy',
              'Fixed0', 'Fixed1', 'SyncIn', 'SIN', 'GPIO', 'AcceptTrg', 'EncodedClkIn'];
    }
    return ['OUT_PROPAGATION_LEVEL0', 'OUT_PROPAGATION_LEVEL1', 'OUT_PROPAGATION_SYNCIN',
            'OUT_PROPAGATION_TRIGGER', 'OUT_PROPAGATION_RUN', 'OUT_PROPAGATION_DELAYED_RUN',
            'OUT_PROPAGATION_SAMPLE_CLK', 'OUT_PROPAGATION_PLL_CLK', 'OUT_PROPAGATION_BUSY',
            'OUT_PROPAGATION_PLL_UNLOCK', 'OUT_PROPAGATION_VPROBE'];
  }

  trgoutModeOptions(): string[] {
    return ['Disabled', 'TrgIn', 'Run', 'RefClk', 'TestPulse', 'Busy', 'UserTrgout',
            'Fixed0', 'Fixed1', 'SyncIn', 'SIN', 'GPIO', 'AcceptTrg', 'EncodedClkIn',
            'ITLA', 'ITLB', 'ITLA_AND_ITLB', 'ITLA_OR_ITLB', 'LVDS',
            'SwTrg', 'P0', 'UserTrgout2', 'UserTrgout3'];
  }

  outSelectionOptions(): string[] {
    return ['OUT_PROPAGATION_LEVEL0', 'OUT_PROPAGATION_LEVEL1', 'OUT_PROPAGATION_SYNCIN',
            'OUT_PROPAGATION_TRIGGER', 'OUT_PROPAGATION_RUN', 'OUT_PROPAGATION_DELAYED_RUN',
            'OUT_PROPAGATION_SAMPLE_CLK', 'OUT_PROPAGATION_PLL_CLK', 'OUT_PROPAGATION_BUSY',
            'OUT_PROPAGATION_PLL_UNLOCK', 'OUT_PROPAGATION_VPROBE'];
  }

  // ===========================================================================
  // Actions
  // ===========================================================================

  async onDetect(): Promise<void> {
    this.detecting.set(true);
    try {
      const result = await this.digitizerService.detectDigitizers();
      if (result.success && result.digitizers.length > 0) {
        this.snackBar.open(result.message, 'OK', { duration: 5000 });
        // Reload digitizers to pick up any newly created/updated configs
        await this.digitizerService.loadDigitizers();
        // Auto-select the first detected digitizer
        const firstDetected = result.digitizers[0];
        if (firstDetected) {
          this.selectedId.set(firstDetected.source_id);
        }
      } else {
        this.snackBar.open(result.message || 'No digitizers detected', 'OK', {
          duration: 5000,
        });
      }
    } catch {
      this.snackBar.open('Failed to detect hardware', 'Close', {
        duration: 5000,
      });
    } finally {
      this.detecting.set(false);
    }
  }

  async applyConfig(): Promise<void> {
    const config = this.selectedConfig();
    if (!config) return;

    // Compress flat channel values back into defaults + overrides
    const { channel_defaults, channel_overrides } =
      this.digitizerService.compressConfig(
        this.defaultValues(),
        this.channelValues()
      );

    const updatedConfig = {
      ...config,
      channel_defaults,
      channel_overrides,
    };

    try {
      const result = await this.digitizerService.applyToHardware(updatedConfig);
      this.snackBar.open(result.message || 'Configuration applied to hardware', 'OK', {
        duration: 5000,
      });
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : 'Failed to apply configuration';
      this.snackBar.open(message, 'Close', {
        duration: 5000,
      });
    }
  }

  async saveConfig(): Promise<void> {
    const config = this.selectedConfig();
    if (!config) return;

    // First apply (compress & send), then save to disk
    await this.applyConfig();

    try {
      await this.digitizerService.saveDigitizer(config.digitizer_id);
      this.snackBar.open('Configuration saved to disk', 'OK', {
        duration: 3000,
      });
    } catch {
      this.snackBar.open('Failed to save configuration', 'Close', {
        duration: 5000,
      });
    }
  }

  resetConfig(): void {
    if (this.selectedId() !== null) {
      this.digitizerService.loadDigitizers();
      this.snackBar.open('Configuration reset', 'OK', { duration: 2000 });
    }
  }
}
