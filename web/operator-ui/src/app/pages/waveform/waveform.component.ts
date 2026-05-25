import {
  Component,
  OnInit,
  OnDestroy,
  inject,
  signal,
  computed,
  effect,
  untracked,
} from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { MatCardModule } from '@angular/material/card';
import { MatSelectModule } from '@angular/material/select';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatButtonModule } from '@angular/material/button';
import { MatIconModule } from '@angular/material/icon';
import { MatCheckboxModule } from '@angular/material/checkbox';
import { MatButtonToggleModule } from '@angular/material/button-toggle';
import { MatProgressSpinnerModule } from '@angular/material/progress-spinner';
import { MatSnackBarModule } from '@angular/material/snack-bar';
import { MatInputModule } from '@angular/material/input';
import { MatSlideToggleModule } from '@angular/material/slide-toggle';
import { MatDividerModule } from '@angular/material/divider';
import { MatTooltipModule } from '@angular/material/tooltip';
import { NgxEchartsDirective } from 'ngx-echarts';
import type { EChartsCoreOption, ECharts } from 'echarts/core';
import {
  Subject,
  Subscription,
  interval,
  takeUntil,
  switchMap,
  forkJoin,
  of,
  map,
  tap,
  finalize,
} from 'rxjs';
import { HistogramService } from '../../services/histogram.service';
import { OperatorService } from '../../services/operator.service';
import { DigitizerService } from '../../services/digitizer.service';
import { NotificationService } from '../../services/notification.service';
import {
  WaveformChannelInfo,
  LatestWaveform,
  Histogram1D,
  Histogram2D,
  ANALOG_PROBE_TYPE_LABELS,
  DIGITAL_PROBE_TYPE_LABELS,
  UNKNOWN_PROBE_TYPE,
} from '../../models/histogram.types';
import { DigitizerConfig } from '../../models/types';
import {
  ChannelTableComponent,
  DefaultValueChange,
  ChannelValueChange,
} from '../../components/channel-table/channel-table.component';
import {
  getCategoryParams,
  getAllChannelParams,
  getProbeOptions,
  ChannelCategory,
  CHANNEL_CATEGORIES,
  CHANNEL_CATEGORY_LABELS,
  ProbeOption,
} from '../../models/channel-params';
import { HistogramChartComponent, RangeChangeEvent } from '../../components/histogram-chart/histogram-chart.component';
import { HeatmapChartComponent } from '../../components/heatmap-chart/heatmap-chart.component';

/** Probe visibility flags. Analog 1..3 stay flat because the carrier
 *  `Waveform` struct fixes the count at 3. Digital probes are
 *  index-keyed (0..15) so we can data-drive the toolbar from
 *  `digital_probe_type[]` instead of one hard-coded checkbox per slot. */
interface ProbeConfig {
  analog1: boolean;
  analog2: boolean;
  analog3: boolean;
  /** Per-slot digital probe visibility, keyed by slot index 0..15.
   *  Missing key = default visibility (computed in
   *  `digitalProbeVisible()` — D0..D1 default-on for PSD1/PHA1
   *  back-compat; AMax-typed slots default-on; others default-off). */
  digital: Record<number, boolean>;
}

/** Cap on digital probe slots — must match the carrier `Waveform`
 *  struct (`digital_probe1..16` / `digital_probe_type: [u8; 16]`) in
 *  `src/reader/decoder/common.rs`. */
const DIGITAL_PROBE_SLOTS = 16;

interface ChannelChart {
  label: string;
  moduleId: number;
  channelId: number;
  energy: number;
  samples: number;
  nsPerSample: number;
  options: EChartsCoreOption;
}

@Component({
  selector: 'app-waveform-page',
  standalone: true,
  imports: [
    CommonModule,
    FormsModule,
    MatCardModule,
    MatSelectModule,
    MatFormFieldModule,
    MatInputModule,
    MatButtonModule,
    MatIconModule,
    MatCheckboxModule,
    MatButtonToggleModule,
    MatSlideToggleModule,
    MatProgressSpinnerModule,
    MatSnackBarModule,
    MatDividerModule,
    MatTooltipModule,
    NgxEchartsDirective,
    ChannelTableComponent,
    HistogramChartComponent,
    HeatmapChartComponent,
  ],
  template: `
    <div class="waveform-page">
      @if (isTuneUp()) {
        <!-- ==================== Tune Up Mode ==================== -->
        <div class="tuneup-toolbar">
          <span class="tuneup-badge">TUNE UP</span>
          <span class="tuneup-digitizer">{{ tuneUpConfig()?.name ?? ('Digitizer ' + tuneUpDigitizerId()) }}</span>

          @if (tuneUpConfig()?.firmware === 'AMax') {
            <!-- AMax-only sub-mode toggle. "Standard" keeps the regular
                 Tune Up layout; "Debug View" surfaces ch0 register
                 inspector + ENABLE_ACQ quick toggle so the operator can
                 flip debug acquisition without leaving Tune Up. -->
            <mat-button-toggle-group
              class="amax-view-toggle"
              [value]="tuneupView()"
              (change)="tuneupView.set($event.value)"
              hideSingleSelectionIndicator
            >
              <mat-button-toggle value="standard" matTooltip="Standard probe view">
                Standard
              </mat-button-toggle>
              <mat-button-toggle value="amax-debug" matTooltip="Debug-FW probe view: ch0 + register inspector">
                <mat-icon class="inline-icon">bug_report</mat-icon>
                Debug
              </mat-button-toggle>
            </mat-button-toggle-group>
            @if (tuneupView() === 'amax-debug') {
              <mat-slide-toggle
                class="amax-enable-acq"
                [checked]="amaxEnableAcq()"
                (change)="onAmaxEnableAcqToggle($event.checked)"
                color="primary"
                matTooltip="Toggle the AMax debug FW's ENABLE_ACQ register. ON = ch0 events carry the 4-lane debug payload (raw / trap / triangle / 16-bit digital). Sends a partial config update via tuneupApply."
              >
                ENABLE_ACQ
              </mat-slide-toggle>
            }
            <mat-slide-toggle
              class="amax-waveforms-enabled"
              [checked]="amaxWaveformsEnabled()"
              (change)="onAmaxWaveformsEnabledToggle($event.checked)"
              color="primary"
              matTooltip="Toggle OpenDPP waveform delivery for this digitizer. ON = events carry waveform samples (required for any probe display); OFF = energy/timestamp only. Sends a partial config update via tuneupApply, which hot-rebinds the FELib endpoint format while acquisition is stopped."
            >
              Waveforms
            </mat-slide-toggle>
          }

          <mat-form-field appearance="outline" class="channel-select">
            <mat-label>Channel</mat-label>
            <mat-select
              [value]="selectedChannels().length > 0 ? selectedChannels()[0] : ''"
              (selectionChange)="onTuneUpChannelSelect($event.value)"
            >
              @for (ch of tuneUpChannels(); track ch.module_id + ':' + ch.channel_id) {
                <mat-option [value]="ch.module_id + ':' + ch.channel_id">
                  {{ ch.name ?? ('Src' + ch.module_id + '/Ch' + ch.channel_id) }}
                </mat-option>
              }
            </mat-select>
          </mat-form-field>

          <div class="probe-toggles">
            <mat-checkbox [checked]="probeConfig().analog1" (change)="toggleProbe('analog1')" color="primary">
              <span class="probe-label" [style.border-bottom-color]="probeColors.analog1">{{ probeLabels().a0Short }}</span>
            </mat-checkbox>
            <mat-checkbox [checked]="probeConfig().analog2" (change)="toggleProbe('analog2')" color="primary">
              <span class="probe-label" [style.border-bottom-color]="probeColors.analog2">{{ probeLabels().a1Short }}</span>
            </mat-checkbox>
            <mat-checkbox [checked]="probeConfig().analog3" (change)="toggleProbe('analog3')" color="primary">
              <span class="probe-label" [style.border-bottom-color]="probeColors.analog3">{{ probeLabels().a2Short }}</span>
            </mat-checkbox>
            @for (slot of activeDigitalProbeSlots(); track slot) {
              <mat-checkbox
                [checked]="digitalProbeVisible(slot)"
                (change)="toggleDigitalProbe(slot)"
                color="primary"
              >
                <span class="probe-label" [style.border-bottom-color]="digitalProbeColor(slot)">
                  {{ digitalProbeLabel(slot, 'short') }}
                </span>
              </mat-checkbox>
            }
          </div>

          <div class="trigger-controls">
            <button mat-stroked-button
              [class.active]="triggerMode() === 'running'"
              (click)="onTriggerRun()"
              matTooltip="Continuous update">
              <mat-icon>play_arrow</mat-icon>
              Run
            </button>
            <button mat-stroked-button
              [class.active]="triggerMode() !== 'running'"
              (click)="onTriggerSingle()"
              matTooltip="Capture next waveform and hold">
              <mat-icon>skip_next</mat-icon>
              Single
            </button>
            @if (triggerMode() === 'armed') {
              <span class="trigger-status armed">ARMED</span>
            }
            @if (triggerMode() === 'held') {
              <span class="trigger-status held">HOLD</span>
            }
          </div>

          <div class="accumulate-controls">
            <mat-slide-toggle
              [checked]="accumulateEnabled()"
              (change)="onAccumulateToggle($event.checked)"
              color="primary"
            >Acc</mat-slide-toggle>
            @if (accumulateEnabled()) {
              <mat-form-field appearance="outline" class="accumulate-count">
                <mat-label>N</mat-label>
                <input matInput type="number"
                  [ngModel]="accumulateMax()"
                  (ngModelChange)="accumulateMax.set($event)"
                  min="2" max="100" />
              </mat-form-field>
              <span class="accumulate-info">{{ waveformHistory().length }}/{{ accumulateMax() }}</span>
              <button mat-icon-button (click)="clearAccumulation()" matTooltip="Clear accumulated waveforms">
                <mat-icon>delete_sweep</mat-icon>
              </button>
            }
          </div>

          <span class="spacer"></span>

          <button mat-stroked-button color="warn" (click)="onStopTuneUp()" [disabled]="tuneUpLoading()">
            <mat-icon>stop</mat-icon>
            Stop
          </button>
        </div>

        @if (tuneupView() === 'amax-debug' && tuneUpConfig()?.firmware === 'AMax') {
          <!-- AMax debug FW register inspector. Shows live read-back of
               the board-level registers (currently ENABLE_ACQ; auto-extends
               via codegen-emitted all_board_registers() when fw_params.json
               grows new entries) so the operator can spot config-vs-hardware
               drift at a glance. Polled at 1 Hz only while this view is
               active. -->
          <mat-card class="amax-register-inspector">
            <mat-card-content>
              <div class="amax-register-header">
                <span class="amax-register-title">
                  <mat-icon class="inline-icon">memory</mat-icon>
                  AMax board registers (live)
                </span>
                @if (amaxBoardRegistersDrifting()) {
                  <span class="amax-register-drift" matTooltip="Live readback disagrees with config — someone may have written the register out-of-band">
                    <mat-icon class="inline-icon">sync_problem</mat-icon>
                    out of sync
                  </span>
                }
              </div>
              @if ((amaxBoardRegisters() | keyvalue).length === 0) {
                <span class="amax-register-empty">— no registers yet (waiting for first poll) —</span>
              } @else {
                <div class="amax-register-grid">
                  @for (entry of amaxBoardRegisters() | keyvalue; track entry.key) {
                    <span class="amax-register-name">{{ entry.key }}:</span>
                    <span class="amax-register-value">{{ formatAmaxRegister(entry.key, $any(entry.value)) }}</span>
                  }
                </div>
              }
            </mat-card-content>
          </mat-card>
        }

        <div class="tuneup-content">
          <!-- Top row: Waveform (left) + Histogram (right) -->
          <div class="tuneup-top-row">
            <div class="tuneup-waveform-panel">
              @if (channelCharts()[0]; as chart) {
                <div class="channel-header">
                  <span class="channel-label">{{ chart.label }}</span>
                  <span class="channel-info">E: {{ chart.energy }} | S: {{ chart.samples }}</span>
                </div>
              }
              <!-- Chart always in DOM to preserve zoom state across Apply -->
              <div class="tuneup-chart-fill">
                <div echarts [options]="tuneUpWfOptions()" (chartInit)="onTuneUpWfChartInit($event)" class="waveform-chart"></div>
              </div>
            </div>

            <div class="tuneup-histogram-panel">
              <div class="panel-header">
                <mat-button-toggle-group
                  [value]="tuneUpDisplayMode()"
                  (change)="onDisplayModeChange($event.value)"
                  class="display-mode-toggle"
                >
                  <mat-button-toggle value="energy">Energy</mat-button-toggle>
                  <mat-button-toggle value="psd2d">PSD 2D</mat-button-toggle>
                </mat-button-toggle-group>
                @if (tuneUpDisplayMode() === 'energy' && tuneUpHistogram(); as hist) {
                  <span class="hist-counts">{{ hist.total_counts | number }} counts</span>
                }
                @if (tuneUpDisplayMode() === 'psd2d' && tuneUpHistogram2d(); as hist2d) {
                  <span class="hist-counts">{{ hist2d.total_counts | number }} counts</span>
                }
                <span class="spacer"></span>
                <mat-checkbox
                  [checked]="histLogScale()"
                  (change)="histLogScale.set($event.checked)"
                  color="primary"
                >Log</mat-checkbox>
              </div>
              @if (tuneUpDisplayMode() === 'energy') {
                @if (tuneUpHistogram(); as hist) {
                  <app-histogram-chart
                    [histogram]="hist"
                    [logScale]="histLogScale()"
                    [xRange]="histXRange()"
                    (rangeChange)="onHistRangeChange($event)"
                  />
                } @else {
                  <div class="hist-empty-placeholder">
                    <mat-icon>hourglass_empty</mat-icon>
                    <span>Waiting for events…</span>
                  </div>
                }
              } @else {
                @if (tuneUpHistogram2d(); as hist2d) {
                  <app-heatmap-chart
                    [histogram]="hist2d"
                    [logScale]="histLogScale()"
                  />
                } @else {
                  <div class="hist-empty-placeholder">
                    <mat-icon>hourglass_empty</mat-icon>
                    <span>Waiting for events…</span>
                  </div>
                }
              }
            </div>
          </div>

          <!-- Bottom row: Parameter Table -->
          <div class="tuneup-bottom-row" tabindex="0" (keydown.enter)="onTuneUpEnterKey($event)">
            <div class="param-controls">
              <mat-button-toggle-group
                [value]="selectedCategory()"
                (change)="onCategoryChange($event.value)"
              >
                <mat-button-toggle value="all">All</mat-button-toggle>
                @for (cat of availableCategories(); track cat.key) {
                  <mat-button-toggle [value]="cat.key">{{ cat.label }}</mat-button-toggle>
                }
              </mat-button-toggle-group>
              <span class="spacer"></span>
              <button mat-flat-button color="primary" (click)="onApplyTuneUp()" [disabled]="applyLoading()">
                @if (applyLoading()) {
                  <mat-spinner diameter="18"></mat-spinner>
                } @else {
                  <mat-icon>send</mat-icon>
                }
                Apply
              </button>
            </div>
            <div class="param-table-wrapper">
              @if (tuneUpConfig(); as config) {
                @if (selectedCategory() === 'all') {
                  <div class="param-grid">
                    @for (cat of categoryGrid(); track cat.key) {
                      <div class="param-grid-cell">
                        <div class="param-grid-header">{{ cat.label }}</div>
                        @if (cat.key === 'waveform' && cat.params.length === 0) {
                          <!-- PSD1/PHA: Board-level waveform settings -->
                          <div class="board-waveform-panel">
                            <div class="board-waveform-row">
                              <mat-slide-toggle [(ngModel)]="config.board.waveforms_enabled">
                                Enable
                              </mat-slide-toggle>
                              <mat-form-field appearance="outline" class="compact-field">
                                <mat-label>Record Length (ns)</mat-label>
                                <input matInput type="number" [(ngModel)]="config.board.record_length" />
                              </mat-form-field>
                            </div>
                            <mat-divider></mat-divider>
                            <div class="board-waveform-row">
                              <mat-form-field appearance="outline" class="compact-field">
                                <mat-label>Analog 0</mat-label>
                                <mat-select [(value)]="config.board.vtrace_probe_0">
                                  @for (opt of probeOptions()[0]; track opt.value) {
                                    <mat-option [value]="opt.value">{{ opt.label }}</mat-option>
                                  }
                                </mat-select>
                              </mat-form-field>
                              <mat-form-field appearance="outline" class="compact-field">
                                <mat-label>Analog 1</mat-label>
                                <mat-select [(value)]="config.board.vtrace_probe_1">
                                  @for (opt of probeOptions()[1]; track opt.value) {
                                    <mat-option [value]="opt.value">{{ opt.label }}</mat-option>
                                  }
                                </mat-select>
                              </mat-form-field>
                            </div>
                            <div class="board-waveform-row">
                              <mat-form-field appearance="outline" class="compact-field">
                                <mat-label>Digital 0</mat-label>
                                <mat-select [(value)]="config.board.vtrace_probe_2">
                                  @for (opt of probeOptions()[2]; track opt.value) {
                                    <mat-option [value]="opt.value">{{ opt.label }}</mat-option>
                                  }
                                </mat-select>
                              </mat-form-field>
                              <mat-form-field appearance="outline" class="compact-field">
                                <mat-label>Digital 1</mat-label>
                                <mat-select [(value)]="config.board.vtrace_probe_3">
                                  @for (opt of probeOptions()[3]; track opt.value) {
                                    <mat-option [value]="opt.value">{{ opt.label }}</mat-option>
                                  }
                                </mat-select>
                              </mat-form-field>
                            </div>
                          </div>
                        } @else {
                          <app-channel-table
                            [params]="cat.params"
                            [numChannels]="config.num_channels"
                            [defaultValues]="defaultValues()"
                            [channelValues]="channelValues()"
                            [visibleChannels]="visibleChannelIndices()"
                            (defaultChange)="onTuneUpDefaultChange($event)"
                            (channelChange)="onTuneUpChannelChange($event)"
                          />
                        }
                      </div>
                    }
                  </div>
                } @else {
                  <app-channel-table
                    [params]="categoryParams()"
                    [numChannels]="config.num_channels"
                    [defaultValues]="defaultValues()"
                    [channelValues]="channelValues()"
                    [visibleChannels]="visibleChannelIndices()"
                    (defaultChange)="onTuneUpDefaultChange($event)"
                    (channelChange)="onTuneUpChannelChange($event)"
                  />
                }
              }
            </div>
          </div>
        </div>

      } @else {
        <!-- ==================== Normal Mode ==================== -->
        <div class="toolbar">
          <mat-form-field appearance="outline" class="channel-select">
            <mat-label>Select Channels</mat-label>
            <mat-select
              [value]="selectedChannels()"
              (selectionChange)="onChannelSelectionChange($event.value)"
              multiple
            >
              @for (ch of availableChannels(); track ch.module_id + ':' + ch.channel_id) {
                <mat-option [value]="ch.module_id + ':' + ch.channel_id">
                  {{ ch.name ?? ('Src' + ch.module_id + '/Ch' + ch.channel_id) }}
                </mat-option>
              }
            </mat-select>
          </mat-form-field>

          <div class="probe-toggles">
            <mat-checkbox
              [checked]="probeConfig().analog1"
              (change)="toggleProbe('analog1')"
              color="primary"
            >
              <span class="probe-label" [style.border-bottom-color]="probeColors.analog1">{{ probeLabels().a0Long }}</span>
            </mat-checkbox>
            <mat-checkbox
              [checked]="probeConfig().analog2"
              (change)="toggleProbe('analog2')"
              color="primary"
            >
              <span class="probe-label" [style.border-bottom-color]="probeColors.analog2">{{ probeLabels().a1Long }}</span>
            </mat-checkbox>
            <mat-checkbox
              [checked]="probeConfig().analog3"
              (change)="toggleProbe('analog3')"
              color="primary"
            >
              <span class="probe-label" [style.border-bottom-color]="probeColors.analog3">{{ probeLabels().a2Long }}</span>
            </mat-checkbox>
            @for (slot of activeDigitalProbeSlots(); track slot) {
              <mat-checkbox
                [checked]="digitalProbeVisible(slot)"
                (change)="toggleDigitalProbe(slot)"
                color="primary"
              >
                <span class="probe-label" [style.border-bottom-color]="digitalProbeColor(slot)">
                  {{ digitalProbeLabel(slot, 'long') }}
                </span>
              </mat-checkbox>
            }
          </div>

          <mat-button-toggle-group
            [value]="yAxisMode()"
            (change)="onYAxisModeChange($event.value)"
            class="y-axis-toggle"
          >
            <mat-button-toggle value="auto">Auto Y</mat-button-toggle>
            <mat-button-toggle value="fixed">Fixed Y</mat-button-toggle>
          </mat-button-toggle-group>

          <button mat-stroked-button (click)="onRefresh()" [disabled]="isLoading()">
            <mat-icon>refresh</mat-icon>
            Refresh
          </button>

          <span class="spacer"></span>

          <!-- Tune Up Start Controls -->
          @if (systemState() === 'Idle' && digitizers().length > 0) {
            <mat-form-field appearance="outline" class="digitizer-select">
              <mat-label>Digitizer</mat-label>
              <mat-select [value]="tuneUpTargetId()" (selectionChange)="tuneUpTargetId.set($event.value)">
                @for (d of digitizers(); track d.digitizer_id) {
                  <mat-option [value]="d.digitizer_id">{{ d.name }}</mat-option>
                }
              </mat-select>
            </mat-form-field>
            <button
              mat-flat-button
              color="accent"
              (click)="onStartTuneUp()"
              [disabled]="tuneUpTargetId() === null || tuneUpLoading()"
            >
              @if (tuneUpLoading()) {
                <mat-spinner diameter="18"></mat-spinner>
              } @else {
                <mat-icon>tune</mat-icon>
              }
              Tune Up
            </button>
          }

          <span class="status-text">
            @if (waveforms().length > 0) {
              {{ waveforms().length }} waveform(s) loaded
            } @else {
              No waveforms available
            }
          </span>
        </div>

        <!-- Per-channel Charts -->
        <div class="charts-scroll">
          @if (channelCharts().length > 0) {
            @for (chart of channelCharts(); track chart.moduleId + ':' + chart.channelId) {
              <div class="channel-card">
                <div class="channel-header">
                  <span class="channel-label">{{ chart.label }}</span>
                  <span class="channel-info">Energy: {{ chart.energy }} | Samples: {{ chart.samples }}</span>
                </div>
                <div class="chart-container">
                  <div
                    echarts
                    [options]="normalBaseOptions()"
                    [merge]="chart.options"
                    class="waveform-chart"
                  ></div>
                </div>
              </div>
            }
          } @else {
            <div class="no-data">
              <mat-icon>show_chart</mat-icon>
              <p>No waveform data available</p>
              <p class="hint">
                Make sure the DAQ is running with waveform enabled
                and select channels from the dropdown above.
              </p>
            </div>
          }
        </div>
      }
    </div>
  `,
  styles: `
    :host {
      display: block;
      height: 100%;
    }

    .waveform-page {
      display: flex;
      flex-direction: column;
      height: 100%;
      padding: 16px;
      gap: 12px;
    }

    /* ================ Shared ================ */

    .toolbar,
    .tuneup-toolbar {
      display: flex;
      align-items: center;
      gap: 16px;
      flex-wrap: wrap;
    }

    .amax-view-toggle {
      font-size: 13px;
    }

    .amax-enable-acq {
      margin-left: 4px;
    }

    .inline-icon {
      font-size: 18px;
      vertical-align: middle;
      width: 18px;
      height: 18px;
    }

    .amax-register-inspector {
      margin-bottom: 12px;
      background: #fff8e1;
      border-left: 4px solid #ffa000;
    }
    .amax-register-header {
      display: flex;
      align-items: center;
      justify-content: space-between;
      margin-bottom: 8px;
    }
    .amax-register-title {
      font-weight: 500;
      display: inline-flex;
      align-items: center;
      gap: 6px;
    }
    .amax-register-drift {
      color: #c62828;
      font-size: 12px;
      display: inline-flex;
      align-items: center;
      gap: 4px;
    }
    .amax-register-grid {
      display: grid;
      grid-template-columns: max-content 1fr;
      gap: 4px 16px;
      font-family: 'Roboto Mono', monospace;
      font-size: 13px;
    }
    .amax-register-name {
      color: #5d4037;
    }
    .amax-register-value {
      color: #1b5e20;
    }
    .amax-register-empty {
      font-style: italic;
      color: #757575;
      font-size: 13px;
    }

    .channel-select {
      min-width: 200px;
    }

    .digitizer-select {
      min-width: 160px;
    }

    .probe-toggles {
      display: flex;
      gap: 12px;
      flex-wrap: wrap;
    }

    .probe-label {
      border-bottom: 3px solid;
      padding-bottom: 1px;
    }

    .y-axis-toggle {
      height: 36px;
    }

    .trigger-controls {
      display: flex;
      align-items: center;
      gap: 4px;
      margin-right: 8px;
    }
    .trigger-controls button.active {
      background-color: rgba(25, 118, 210, 0.12);
      border-color: #1976d2;
    }
    .trigger-status {
      font-size: 12px;
      font-weight: 700;
      letter-spacing: 0.6px;
      padding: 4px 10px;
      border-radius: 4px;
      margin-left: 4px;
      border: 1px solid currentColor;
      animation: trigger-status-pulse 1.4s ease-in-out infinite;
    }
    .trigger-status.armed {
      background-color: #ffe0b2;
      color: #bf360c;
    }
    .trigger-status.held {
      background-color: #c8e6c9;
      color: #1b5e20;
    }
    @keyframes trigger-status-pulse {
      0%, 100% { opacity: 1; }
      50% { opacity: 0.65; }
    }

    .accumulate-controls {
      display: flex;
      align-items: center;
      gap: 8px;
    }
    .accumulate-count {
      width: 70px;
    }
    .accumulate-count .mat-mdc-form-field-subscript-wrapper {
      display: none;
    }
    .accumulate-info {
      font-size: 12px;
      color: #666;
      white-space: nowrap;
    }

    .spacer {
      flex: 1;
    }

    .status-text {
      color: #666;
      font-size: 14px;
    }

    /* ================ Normal Mode ================ */

    .charts-scroll {
      flex: 1;
      overflow-y: auto;
      display: flex;
      flex-direction: column;
      gap: 12px;
    }

    .channel-card {
      background: white;
      border: 1px solid #e0e0e0;
      border-radius: 4px;
      overflow: hidden;
    }

    .channel-header {
      display: flex;
      align-items: center;
      gap: 16px;
      padding: 8px 16px;
      background: #fafafa;
      border-bottom: 1px solid #e0e0e0;
      font-size: 13px;
    }

    .channel-label {
      font-weight: 600;
    }

    .channel-info {
      color: #666;
    }

    .chart-container {
      height: 350px;
    }

    .waveform-chart {
      width: 100%;
      height: 100%;
    }

    .no-data {
      display: flex;
      flex-direction: column;
      align-items: center;
      justify-content: center;
      height: 100%;
      color: #999;

      mat-icon {
        font-size: 64px;
        width: 64px;
        height: 64px;
        margin-bottom: 16px;
      }

      p {
        margin: 4px 0;
      }

      .hint {
        font-size: 12px;
        color: #bbb;
      }
    }

    /* ================ Tune Up Mode ================ */

    .tuneup-badge {
      background: #ff6f00;
      color: white;
      padding: 4px 12px;
      border-radius: 4px;
      font-weight: 700;
      font-size: 13px;
      letter-spacing: 1px;
    }

    .tuneup-digitizer {
      font-weight: 600;
      font-size: 15px;
    }

    .tuneup-content {
      flex: 1;
      display: grid;
      grid-template-rows: 1fr 1fr;
      gap: 8px;
      min-height: 0;
    }

    .tuneup-top-row {
      display: grid;
      grid-template-columns: 1fr 1fr;
      gap: 8px;
      min-height: 0;
    }

    .tuneup-waveform-panel,
    .tuneup-histogram-panel {
      border: 1px solid #e0e0e0;
      border-radius: 4px;
      overflow: hidden;
      display: flex;
      flex-direction: column;
      min-height: 0;
    }

    .tuneup-chart-fill {
      flex: 1;
      min-height: 0;
    }

    .hist-empty-placeholder {
      flex: 1;
      display: flex;
      flex-direction: column;
      align-items: center;
      justify-content: center;
      gap: 8px;
      color: rgba(0, 0, 0, 0.45);
      font-size: 13px;
    }
    .hist-empty-placeholder mat-icon {
      font-size: 36px;
      width: 36px;
      height: 36px;
      opacity: 0.5;
    }

    .panel-header {
      display: flex;
      align-items: center;
      gap: 12px;
      padding: 6px 12px;
      background: #fafafa;
      border-bottom: 1px solid #e0e0e0;
      font-weight: 600;
      font-size: 13px;
    }

    .display-mode-toggle {
      height: 28px;
    }

    .display-mode-toggle .mat-button-toggle-label-content {
      line-height: 28px;
      padding: 0 8px;
      font-size: 12px;
    }

    .hist-counts {
      font-weight: 400;
      color: #666;
      font-size: 12px;
    }

    .no-data.compact {
      padding: 24px;
      mat-icon {
        font-size: 40px;
        width: 40px;
        height: 40px;
        margin-bottom: 8px;
      }
    }

    .tuneup-bottom-row {
      display: flex;
      flex-direction: column;
      min-height: 0;
      border: 1px solid #e0e0e0;
      border-radius: 4px;
      overflow: hidden;
    }

    .param-controls {
      display: flex;
      align-items: center;
      gap: 12px;
      padding: 6px 12px;
      background: #fafafa;
      border-bottom: 1px solid #e0e0e0;
    }

    .param-table-wrapper {
      flex: 1;
      overflow: auto;
    }

    .param-grid {
      display: grid;
      grid-template-columns: repeat(auto-fill, minmax(310px, 1fr));
      gap: 4px;
      padding: 4px;
    }

    .param-grid-cell {
      border: 1px solid #e0e0e0;
      border-radius: 4px;
      overflow: hidden;
    }

    .param-grid-header {
      padding: 4px 8px;
      background: #e3f2fd;
      font-weight: 600;
      font-size: 12px;
      border-bottom: 1px solid #e0e0e0;
    }

    .board-waveform-panel {
      padding: 8px;
      display: flex;
      flex-direction: column;
      gap: 8px;
    }

    .board-waveform-row {
      display: flex;
      gap: 8px;
      align-items: center;
      flex-wrap: wrap;
    }

    .compact-field {
      flex: 1;
      min-width: 120px;
    }

    .compact-field .mat-mdc-form-field-subscript-wrapper {
      display: none;
    }
  `,
})
export class WaveformPageComponent implements OnInit, OnDestroy {
  // ===========================================================================
  // Service Injections
  // ===========================================================================
  private readonly histogramService = inject(HistogramService);
  private readonly operatorService = inject(OperatorService);
  private readonly digitizerService = inject(DigitizerService);
  private readonly notify = inject(NotificationService);

  private readonly destroy$ = new Subject<void>();
  private readonly refreshInterval = 500;
  private histPolling$ = new Subscription();

  // ===========================================================================
  // Shared State (used in both modes)
  // ===========================================================================
  readonly availableChannels = signal<WaveformChannelInfo[]>([]);
  readonly selectedChannels = this.digitizerService.selectedWaveformChannels;
  readonly waveforms = signal<LatestWaveform[]>([]);
  readonly isLoading = signal(false);
  readonly probeConfig = signal<ProbeConfig>({
    analog1: true,
    analog2: true,
    analog3: true,
    digital: {},
  });
  readonly yAxisMode = signal<'auto' | 'fixed'>('auto');

  readonly probeColors = {
    analog1: '#1565c0',
    analog2: '#e65100',
    analog3: '#fb8c00',
  };

  /** Pre-computed digital probe colors for slots 0..15. The first 5 match
   *  the original PHA1/AMax-debug palette so existing operators see the
   *  same colors. Slots 5..15 fan out via HSL hue rotation so any future
   *  bit assignment gets a visually distinct band without UI churn. */
  private readonly DIGITAL_PROBE_COLORS: readonly string[] = [
    '#00897b', '#c62828', '#6a1b9a', '#5c6bc0', '#00838f', // legacy 0..4
    ...Array.from({ length: DIGITAL_PROBE_SLOTS - 5 }, (_, i) => {
      // 11 distinct hues at 70%/45% saturation/lightness. Hue starts at
      // ~190 (close to existing teal) and steps ~30° to fill the wheel.
      const hue = (190 + (i + 5) * 33) % 360;
      return `hsl(${hue} 60% 42%)`;
    }),
  ];

  /** Lookup digital probe color by slot index. Falls back to neutral gray
   *  for out-of-range indices. */
  digitalProbeColor(slot: number): string {
    return this.DIGITAL_PROBE_COLORS[slot] ?? '#9e9e9e';
  }

  readonly channelCharts = computed<ChannelChart[]>(() => this.buildChannelCharts());

  // Normal mode base options: stable reference between polls, re-inits on user action
  readonly normalBaseOptions = computed<EChartsCoreOption>(() => {
    this.selectedChannels();
    this.probeConfig();
    const yMode = this.yAxisMode();
    return {
      animation: false,
      grid: { left: 60, right: 50, top: 40, bottom: 50 },
      legend: { show: true, top: 5, type: 'scroll' },
      tooltip: { trigger: 'axis', axisPointer: { type: 'cross' } },
      xAxis: { type: 'value', nameLocation: 'middle', nameGap: 25, min: 0 },
      yAxis: {
        type: 'value',
        name: 'ADC',
        ...(yMode === 'fixed' ? { min: -30000, max: 30000 } : {}),
        axisLabel: {
          formatter: (value: number) => {
            if (Math.abs(value) >= 1000) return (value / 1000).toFixed(1) + 'k';
            return value.toString();
          },
        },
      },
      dataZoom: [
        { type: 'inside', xAxisIndex: 0, yAxisIndex: [], zoomOnMouseWheel: 'shift', moveOnMouseMove: true, filterMode: 'none' },
        { type: 'inside', xAxisIndex: [], yAxisIndex: 0, zoomOnMouseWheel: 'ctrl', moveOnMouseMove: false, filterMode: 'none' },
        { type: 'slider', xAxisIndex: 0, height: 20, bottom: 5, filterMode: 'none' },
        { type: 'slider', yAxisIndex: 0, width: 20, right: 5, filterMode: 'none' },
      ],
      series: [],
    };
  });

  // ===========================================================================
  // Normal Mode State
  // ===========================================================================
  readonly digitizers = computed(() => this.digitizerService.digitizers());
  readonly systemState = computed(() => this.operatorService.systemState());
  readonly tuneUpTargetId = signal<number | null>(null);

  // ===========================================================================
  // Tune Up State
  // ===========================================================================
  readonly isTuneUp = computed(() => this.operatorService.isTuneUp());
  readonly tuneUpDigitizerId = computed(() => this.operatorService.tuneupDigitizerId());
  readonly tuneUpLoading = signal(false);
  readonly applyLoading = signal(false);
  readonly tuneUpConfig = signal<DigitizerConfig | null>(null);

  /** Sub-mode within the shared Tune Up. `'standard'` keeps the normal
   *  toolbar / probe panel layout; `'amax-debug'` surfaces an
   *  ENABLE_ACQ quick toggle so the operator can flip the AMax debug
   *  FW's acquisition mode without leaving Tune Up. Only meaningful
   *  when `tuneUpConfig()?.firmware === 'AMax'`; the toggle is hidden
   *  for other firmwares (Round 2 plan I.1). */
  readonly tuneupView = signal<'standard' | 'amax-debug'>('standard');

  /** Local mirror of `channel_defaults.amax.enable_acq` — kept as a
   *  signal so the Tune Up debug toolbar's slide toggle can flip it
   *  optimistically. Synced from `tuneUpConfig()` via an effect;
   *  flipping here writes through `tuneupApply`
   *  (see `onAmaxEnableAcqToggle`).
   *
   *  ENABLE_ACQ lived under `amax_board` (board-level register) in
   *  older FW builds. The 13may caenlist FW moved it onto the
   *  per-channel page as a broadcast write (page 0x200, single
   *  write fans out to all channels), so the toggle now reads
   *  from / writes to `channel_defaults.amax.enable_acq`. */
  readonly amaxEnableAcq = signal(false);

  /** Local mirror of `board.waveforms_enabled` — kept as a signal so the
   *  Tune Up toolbar's "Waveforms" slide toggle can flip waveform
   *  delivery without leaving Tune Up. Synced from `tuneUpConfig()` in
   *  `loadTuneUpConfig`. Flipping here writes through `tuneupApply`,
   *  which hot-rebinds the FELib OpenDPP endpoint format on the reader
   *  side (see `configure_opendpp_endpoint` in read_loop_dig2.rs). The
   *  FELib data-format JSON is locked at endpoint setup time, so this
   *  toggle is the only way to switch waveform delivery on/off without
   *  bouncing the reader process. */
  readonly amaxWaveformsEnabled = signal(false);

  /** Live AMax board-level register values polled from the digitizer
   *  (via `GET /api/digitizers/:id/amax-board-registers`). Polled at
   *  1 Hz only while the AMax Tune Up debug sub-mode is active. Empty
   *  object means "no AMax board registers / not yet polled". The
   *  Tune Up panel surfaces this so the operator can see what
   *  ENABLE_ACQ (and any future board register) is *actually* set to
   *  on hardware vs what's stored in the config. */
  readonly amaxBoardRegisters = signal<Record<string, number>>({});

  /** RxJS subscription handle for the AMax register poll loop —
   *  unsubscribed automatically when the operator leaves amax-debug
   *  view (see `amaxRegistersPollEffect` below). */
  private amaxRegPolling$ = Subscription.EMPTY;

  /** When the operator switches into amax-debug view, lock the channel
   *  selector to ch0 — that's the only channel the AMax debug FW
   *  instruments (the SE pin is hardwired to U57 only, see
   *  `decoder/amax.rs` spec ref). Without this, the operator could be
   *  staring at ch5 wondering why no debug-mode events show up. */
  private readonly amaxDebugCh0LockEffect = effect(() => {
    if (this.tuneupView() !== 'amax-debug') return;
    const channels = this.tuneUpChannels();
    if (channels.length === 0) return;
    const ch0 = channels.find((c) => c.channel_id === 0);
    if (!ch0) return;
    const ch0Key = `${ch0.module_id}:${ch0.channel_id}`;
    untracked(() => {
      const current = this.selectedChannels();
      if (current.length !== 1 || current[0] !== ch0Key) {
        this.selectedChannels.set([ch0Key]);
      }
    });
  });

  /** Start/stop the AMax board-register poll loop based on
   *  amax-debug sub-mode + active Tune Up. Reads at 1 Hz so the
   *  inspector card stays roughly in sync without hammering the
   *  reader's CaenHandle. */
  private readonly amaxRegistersPollEffect = effect(() => {
    const active =
      this.isTuneUp() &&
      this.tuneupView() === 'amax-debug' &&
      this.tuneUpConfig()?.firmware === 'AMax';
    const digitizerId = this.tuneUpDigitizerId();

    untracked(() => {
      this.amaxRegPolling$.unsubscribe();
      this.amaxRegPolling$ = Subscription.EMPTY;
      if (!active || digitizerId === null) {
        this.amaxBoardRegisters.set({});
        return;
      }
      // One immediate read + 1 Hz refresh.
      this.refreshAmaxBoardRegisters(digitizerId);
      this.amaxRegPolling$ = interval(1000).subscribe(() => {
        this.refreshAmaxBoardRegisters(digitizerId);
      });
    });
  });

  private async refreshAmaxBoardRegisters(digitizerId: number): Promise<void> {
    try {
      const values = await this.digitizerService.readAmaxBoardRegisters(digitizerId);
      this.amaxBoardRegisters.set(values ?? {});
    } catch {
      // Backend returns {} for non-AMax / disconnected paths; only network
      // failures land here. Don't spam the operator — just clear the panel.
      this.amaxBoardRegisters.set({});
    }
  }

  /** Format a register value for the inspector card. Currently only
   *  ENABLE_ACQ (1-bit) gets a friendly name; future fields fall back
   *  to hex. Drives the inspector card template. */
  formatAmaxRegister(name: string, value: number): string {
    if (name === 'enable_acq') return value === 1 ? 'ON (debug)' : 'OFF (legacy)';
    return `0x${value.toString(16).padStart(8, '0').toUpperCase()}`;
  }

  /** True when the live readback for ENABLE_ACQ disagrees with the
   *  config-side mirror — surfaces a yellow "out of sync" badge so the
   *  operator notices if someone wrote the register out-of-band. */
  amaxBoardRegistersDrifting(): boolean {
    const live = this.amaxBoardRegisters()['enable_acq'];
    if (live === undefined) return false;
    return (live === 1) !== this.amaxEnableAcq();
  }
  readonly defaultValues = signal<Record<string, unknown>>({});
  readonly channelValues = signal<Record<string, unknown>[]>([]);
  readonly selectedCategory = signal<ChannelCategory | 'all'>('all');
  readonly tuneUpHistogram = signal<Histogram1D | null>(null);
  readonly tuneUpHistogram2d = signal<Histogram2D | null>(null);
  /** Display mode for the right panel: 'energy' = 1D histogram, 'psd2d' = 2D heatmap */
  readonly tuneUpDisplayMode = signal<'energy' | 'psd2d'>('energy');

  // Waveform Accumulation State
  readonly accumulateEnabled = signal(false);
  readonly accumulateMax = signal(20);
  readonly waveformHistory = signal<LatestWaveform[]>([]);

  // Single Shot State (oscilloscope-style trigger mode)
  readonly triggerMode = signal<'running' | 'armed' | 'held'>('running');
  private lastArmedTimestamp: number | null = null;

  /** ECharts instance for Tune Up waveform — used for replaceMerge to preserve zoom */
  private tuneUpWfChart: ECharts | null = null;

  // Tune Up waveform chart: options (re-init only on channel change)
  readonly tuneUpWfOptions = computed<EChartsCoreOption>(() => {
    this.selectedChannels(); // re-init chart when channel changes
    return this.buildTuneUpWfInitOptions();
  });

  /** In Tune Up mode, only show channels belonging to the target digitizer */
  readonly tuneUpChannels = computed(() => {
    const digitizerId = this.tuneUpDigitizerId();
    if (digitizerId == null) return this.availableChannels();
    return this.availableChannels().filter(ch => ch.module_id === digitizerId);
  });

  readonly categoryParams = computed(() => {
    const config = this.tuneUpConfig();
    if (!config) return [];
    const cat = this.selectedCategory();
    if (cat === 'all') return getAllChannelParams(config.firmware);
    return getCategoryParams(config.firmware, cat);
  });

  /** All channel-param categories in operator-pipeline order, driven by
   *  the canonical list in channel-params.ts. New firmware categories
   *  (e.g. AMax `debug`) show up automatically the moment the codegen
   *  emits a non-empty `AMAX_<CAT>_PARAMS` for that firmware — no
   *  hand-edit here. */
  private readonly allCategories: { key: ChannelCategory; label: string }[] =
    CHANNEL_CATEGORIES.map(key => ({ key, label: CHANNEL_CATEGORY_LABELS[key] }));

  /** Categories the Tune Up panel actually surfaces, filtered to those
   *  the current firmware exposes. Keeps an empty `waveform` cell only
   *  for PSD1/PHA1 (their Waveform sub-tab houses board-level Record
   *  Length + virtual-probe selectors with no channel params). AMax /
   *  PSD2 / PHA2 hide the tab outright when the category is empty. */
  readonly categoryGrid = computed(() => {
    const config = this.tuneUpConfig();
    if (!config) return [];
    const keepEmptyWaveform =
      config.firmware === 'PSD1' || config.firmware === 'PHA1';
    return this.allCategories
      .map(c => ({ ...c, params: getCategoryParams(config.firmware, c.key) }))
      .filter(c => {
        if (c.params.length > 0) return true;
        if (c.key === 'waveform' && keepEmptyWaveform) return true;
        return false;
      });
  });

  /** Same set as `categoryGrid` but exposes just the keys + labels, for
   *  rendering the Tune Up sub-tab button-toggle group. The `"all"`
   *  selector is rendered separately in the template. */
  readonly availableCategories = computed(() =>
    this.categoryGrid().map(c => ({ key: c.key, label: c.label })),
  );

  /** Check if firmware uses board-level waveform settings */
  readonly isBoardLevelWaveform = computed(() => {
    const config = this.tuneUpConfig();
    return config?.firmware === 'PSD1' || config?.firmware === 'PHA1';
  });

  /** Virtual Probe options per firmware (data-driven, PSD1/PHA1) */
  readonly probeOptions = computed((): ProbeOption[][] => {
    const fw = this.tuneUpConfig()?.firmware;
    return fw ? [0, 1, 2, 3].map((i) => getProbeOptions(fw, i)) : [[], [], [], []];
  });

  /** Analog probe labels for the toolbar checkboxes — falls through to
   *  "A0: TimeFilter" when the FW carries typed probe info on the wire
   *  (PHA2 today, AMax debug FW for slots 0..2), otherwise "A0/A1/A2"
   *  generic. Digital labels live in `digitalProbeLabel()` (data-driven
   *  per slot, see Phase H.1). */
  readonly probeLabels = computed(() => {
    const wf = this.waveforms()[0]?.waveform
      ?? this.waveformHistory()[this.waveformHistory().length - 1]?.waveform;
    const apt = wf?.analog_probe_type;
    const apLabel = (idx: 0 | 1 | 2, fallback: string) => {
      const code = apt?.[idx];
      if (code === undefined || code === UNKNOWN_PROBE_TYPE) return fallback;
      const name = ANALOG_PROBE_TYPE_LABELS[code];
      return name ? `${fallback}: ${name}` : fallback;
    };
    return {
      a0Short: apLabel(0, 'A0'),
      a1Short: apLabel(1, 'A1'),
      a2Short: apLabel(2, 'A2'),
      a0Long: apLabel(0, 'Analog 0'),
      a1Long: apLabel(1, 'Analog 1'),
      a2Long: apLabel(2, 'Analog 2'),
    };
  });

  readonly histLogScale = signal(false);
  readonly histXRange = signal<{ min: number; max: number } | 'auto'>('auto');

  /** Channel index from "moduleId:channelId" selection string */
  readonly visibleChannelIndices = computed<number[] | null>(() => {
    const selected = this.selectedChannels();
    if (selected.length === 0) return null;
    const channelId = Number(selected[0].split(':')[1]);
    return [channelId];
  });

  // React to Tune Up mode changes
  private readonly tuneUpEffect = effect(() => {
    const active = this.isTuneUp();
    const digitizerId = this.tuneUpDigitizerId();

    untracked(() => {
      if (active && digitizerId != null) {
        this.loadTuneUpConfig(digitizerId);
        this.fetchChannelList();
        this.startHistogramPolling();
      } else {
        this.tuneUpConfig.set(null);
        this.defaultValues.set({});
        this.channelValues.set([]);
        this.tuneUpHistogram.set(null);
        this.tuneUpHistogram2d.set(null);
        this.stopHistogramPolling();
      }
    });
  });

  // Update Tune Up waveform via replaceMerge (replaces series, preserves zoom/dataZoom)
  private readonly tuneUpWfEffect = effect(() => {
    const charts = this.channelCharts();
    const isTuneUp = this.isTuneUp();
    untracked(() => {
      if (!isTuneUp || charts.length === 0 || !this.tuneUpWfChart) return;
      const chart = charts[0];
      const useTime = chart.nsPerSample > 0;
      this.tuneUpWfChart.setOption(
        {
          series: (chart.options as Record<string, unknown>)['series'],
          xAxis: {
            name: useTime ? 'Time (ns)' : 'Sample',
            max: useTime ? chart.samples * chart.nsPerSample : (chart.samples || undefined),
          },
        },
        { replaceMerge: ['series'] }
      );
    });
  });

  // ===========================================================================
  // Lifecycle
  // ===========================================================================

  ngOnInit(): void {
    this.fetchChannelList();
    this.digitizerService.loadDigitizers();

    // Waveform polling (500ms)
    interval(this.refreshInterval)
      .pipe(
        takeUntil(this.destroy$),
        switchMap(() => this.fetchWaveforms())
      )
      .subscribe();

    // Channel list polling (5s) — picks up newly registered channels
    interval(5000)
      .pipe(takeUntil(this.destroy$))
      .subscribe(() => this.fetchChannelList());
  }

  ngOnDestroy(): void {
    this.destroy$.next();
    this.destroy$.complete();
    this.stopHistogramPolling();
  }

  // ===========================================================================
  // Shared Actions
  // ===========================================================================

  onChannelSelectionChange(selected: string[]): void {
    this.selectedChannels.set(selected);
    this.fetchWaveforms().subscribe();
  }

  onTuneUpChannelSelect(value: string): void {
    this.selectedChannels.set([value]);
    this.waveformHistory.set([]);
    this.triggerMode.set('running');
    this.fetchWaveforms().subscribe();
  }

  clearAccumulation(): void {
    this.waveformHistory.set([]);
    // Next poll will rebuild channelCharts → tuneUpWfEffect uses replaceMerge to clear old series
  }

  onAccumulateToggle(enabled: boolean): void {
    this.accumulateEnabled.set(enabled);
    if (!enabled) {
      this.waveformHistory.set([]);
    }
  }

  onTriggerRun(): void {
    this.triggerMode.set('running');
  }

  onTriggerSingle(): void {
    if (this.triggerMode() === 'running' || this.triggerMode() === 'held') {
      this.lastArmedTimestamp = this.waveforms()[0]?.timestamp_ns ?? null;
      this.triggerMode.set('armed');
    }
  }

  onTuneUpWfChartInit(chart: ECharts): void {
    this.tuneUpWfChart = chart;
  }

  toggleProbe(probe: 'analog1' | 'analog2' | 'analog3'): void {
    const config = this.probeConfig();
    this.probeConfig.set({
      ...config,
      [probe]: !config[probe],
    });
  }

  /** True iff the digital probe at `slot` is currently visible. Defaults
   *  apply for slots not yet toggled by the operator: D0/D1 are on (PSD1/
   *  PHA1 back-compat), AMax-typed slots (probe-type 0x40+) are on, all
   *  others default to off so non-AMax operators don't get a noisy plot. */
  digitalProbeVisible(slot: number): boolean {
    const explicit = this.probeConfig().digital[slot];
    if (explicit !== undefined) return explicit;
    // First-time default: depends on whether the wire reports a typed probe
    // for this slot. AMax debug FW emits 0x40+ for slots 0..4; PSD1/PHA1
    // emit UNKNOWN but populate digital_probe1/2 with real bits.
    const wf = this.waveforms()[0]?.waveform
      ?? this.waveformHistory()[this.waveformHistory().length - 1]?.waveform;
    const code = wf?.digital_probe_type?.[slot];
    if (code !== undefined && code !== UNKNOWN_PROBE_TYPE) return true;
    // Slots 0/1 default-on for PSD1/PHA1 back-compat (those FWs populate
    // digital_probe1/2 with real bits but emit UNKNOWN probe_type).
    return slot < 2;
  }

  /** Flip the visibility of a digital probe at `slot`. */
  toggleDigitalProbe(slot: number): void {
    const config = this.probeConfig();
    const current = this.digitalProbeVisible(slot);
    this.probeConfig.set({
      ...config,
      digital: { ...config.digital, [slot]: !current },
    });
  }

  /** Slot indices to surface as toolbar checkboxes. Filters by
   *  `digital_probe_type[i] !== UNKNOWN_PROBE_TYPE` OR by non-empty
   *  `digital_probe{i+1}` array, so:
   *  - PSD1/PHA1 → slots 0..1 (the two real-data probes), UNKNOWN type
   *  - PHA2 → slots 0..3 (typed by wf-extras header)
   *  - AMax debug FW → slots 0..4 today, up to 0..15 when Rebeca wires
   *    the remaining bits
   *  - Non-debug AMax / other FW with no digital probes → empty list
   */
  readonly activeDigitalProbeSlots = computed<number[]>(() => {
    const wf = this.waveforms()[0]?.waveform
      ?? this.waveformHistory()[this.waveformHistory().length - 1]?.waveform;
    if (!wf) return [];
    const out: number[] = [];
    const probes: (number[] | undefined)[] = [
      wf.digital_probe1, wf.digital_probe2, wf.digital_probe3, wf.digital_probe4,
      wf.digital_probe5, wf.digital_probe6, wf.digital_probe7, wf.digital_probe8,
      wf.digital_probe9, wf.digital_probe10, wf.digital_probe11, wf.digital_probe12,
      wf.digital_probe13, wf.digital_probe14, wf.digital_probe15, wf.digital_probe16,
    ];
    for (let i = 0; i < DIGITAL_PROBE_SLOTS; i++) {
      const code = wf.digital_probe_type?.[i];
      const typed = code !== undefined && code !== UNKNOWN_PROBE_TYPE;
      const populated = (probes[i]?.length ?? 0) > 0;
      if (typed || populated) out.push(i);
    }
    return out;
  });

  /** Display label for a digital probe slot. Uses the typed name from
   *  `DIGITAL_PROBE_TYPE_LABELS` when available; falls back to "D{slot}". */
  digitalProbeLabel(slot: number, style: 'short' | 'long' = 'short'): string {
    const wf = this.waveforms()[0]?.waveform
      ?? this.waveformHistory()[this.waveformHistory().length - 1]?.waveform;
    const code = wf?.digital_probe_type?.[slot];
    const fallback = style === 'short' ? `D${slot}` : `Digital ${slot}`;
    if (code === undefined || code === UNKNOWN_PROBE_TYPE) return fallback;
    const name = DIGITAL_PROBE_TYPE_LABELS[code];
    return name ? `${fallback}: ${name}` : fallback;
  }

  onYAxisModeChange(mode: 'auto' | 'fixed'): void {
    this.yAxisMode.set(mode);
  }

  onRefresh(): void {
    this.fetchChannelList();
    this.fetchWaveforms().subscribe();
  }

  // ===========================================================================
  // Tune Up Actions
  // ===========================================================================

  onStartTuneUp(): void {
    const targetId = this.tuneUpTargetId();
    if (targetId == null) return;

    this.tuneUpLoading.set(true);
    this.operatorService.tuneupStart(targetId).subscribe({
      next: (resp) => {
        this.tuneUpLoading.set(false);
        if (!resp.success) {
          this.notify.error('Start failed: ' + resp.message);
        }
      },
      error: (err) => {
        this.tuneUpLoading.set(false);
        this.notify.error('Start error: ' + (err.error?.message ?? err.message));
      },
    });
  }

  onStopTuneUp(): void {
    this.tuneUpLoading.set(true);
    this.operatorService.tuneupStop().subscribe({
      next: () => {
        this.tuneUpLoading.set(false);
        this.notify.success('Tune Up stopped');
      },
      error: (err) => {
        this.tuneUpLoading.set(false);
        this.notify.error('Stop error: ' + (err.error?.message ?? err.message));
      },
    });
  }

  onTuneUpEnterKey(event: Event): void {
    const target = event.target as HTMLElement;
    if (target.tagName === 'INPUT') {
      (target as HTMLInputElement).blur(); // commit value via change+blur handlers
      this.onApplyTuneUp();
    }
  }

  onApplyTuneUp(): void {
    const config = this.tuneUpConfig();
    if (!config) return;

    this.applyLoading.set(true);
    this.stopHistogramPolling();
    this.tuneUpHistogram.set(null);
    this.tuneUpHistogram2d.set(null);
    this.waveformHistory.set([]);
    this.triggerMode.set('running');

    const { channel_defaults, channel_overrides } = this.digitizerService.compressConfig(
      this.defaultValues(),
      this.channelValues()
    );

    const updatedConfig: DigitizerConfig = {
      ...config,
      channel_defaults,
      channel_overrides,
    };

    this.operatorService
      .tuneupApply(config.digitizer_id, updatedConfig)
      .pipe(
        switchMap((resp) => {
          if (resp.success) {
            this.tuneUpConfig.set(updatedConfig);
            // Refresh global digitizers cache so Settings page shows updated values
            this.digitizerService.loadDigitizers();
            // Chain clear — wait for server to drain stale data + clear histograms
            return this.histogramService.clearHistograms().pipe(map(() => resp));
          }
          return of(resp);
        }),
        finalize(() => {
          this.applyLoading.set(false);
          this.startHistogramPolling();
        })
      )
      .subscribe({
        next: (resp) => {
          if (resp.success) {
            this.notify.success('Configuration applied');
          } else {
            this.notify.error('Apply failed: ' + resp.message);
          }
        },
        error: (err) => {
          this.notify.error('Apply error: ' + (err.error?.message ?? err.message));
        },
      });
  }

  onCategoryChange(category: ChannelCategory | 'all'): void {
    this.selectedCategory.set(category);
  }

  onTuneUpDefaultChange(event: DefaultValueChange): void {
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

  onTuneUpChannelChange(event: ChannelValueChange): void {
    const channels = [...this.channelValues()];
    channels[event.channel] = {
      ...channels[event.channel],
      [event.key]: event.value,
    };
    this.channelValues.set(channels);
  }

  onHistRangeChange(event: RangeChangeEvent): void {
    this.histXRange.set(event.xRange);
  }

  onDisplayModeChange(mode: 'energy' | 'psd2d'): void {
    this.tuneUpDisplayMode.set(mode);
    // Trigger immediate fetch for the new mode
    const selected = this.selectedChannels();
    if (selected.length === 0) return;
    const [moduleId, channelId] = selected[0].split(':').map(Number);
    if (mode === 'psd2d') {
      this.histogramService.fetchHistogram2d(moduleId, channelId).subscribe((hist) => {
        if (hist) this.tuneUpHistogram2d.set(hist);
      });
    } else {
      this.histogramService.fetchHistogram(moduleId, channelId).subscribe((hist) => {
        if (hist) this.tuneUpHistogram.set(hist);
      });
    }
  }

  // ===========================================================================
  // Tune Up Helpers
  // ===========================================================================

  private async loadTuneUpConfig(digitizerId: number): Promise<void> {
    const config = await this.digitizerService.getDigitizer(digitizerId);
    if (!config) return;

    this.tuneUpConfig.set(config);
    this.defaultValues.set(this.digitizerService.extractDefaults(config));
    this.channelValues.set(this.digitizerService.expandConfig(config));
    // Sync the AMax debug-mode mirror from the just-loaded config so the
    // sub-mode slide toggle reflects what the hardware actually has set.
    // ENABLE_ACQ lives on the per-channel AMax page in the 13may caenlist
    // FW (broadcast write), so read from `channel_defaults.amax`.
    this.amaxEnableAcq.set(
      (config.channel_defaults?.amax?.enable_acq ?? 0) === 1,
    );
    // Mirror `board.waveforms_enabled` for the Tune Up toolbar's
    // "Waveforms" toggle. Defaults to false if absent — matches the
    // backend's `unwrap_or(false)` in read_loop_dig2.rs.
    this.amaxWaveformsEnabled.set(config.board?.waveforms_enabled === true);
  }

  /** Handle the ENABLE_ACQ slide toggle in the AMax debug Tune Up
   *  toolbar. Sends a partial config update via `tuneupApply` so the
   *  operator can flip the FW's debug acquisition mode without
   *  navigating to the Settings page.
   *
   *  ENABLE_ACQ moved off the board-level page onto the per-channel
   *  broadcast page in the 13may caenlist FW
   *  (`AMaxBoardConfig` is now empty; see
   *  `tools/amax_viewer/fw_params.json` _board_params_doc). The
   *  write goes through `apply_amax_channel_config` on the backend
   *  reading `channel_defaults.amax.enable_acq` — a single broadcast
   *  register write fans out to all channels in hardware, so the
   *  global "debug ON/OFF" semantics are preserved. */
  onAmaxEnableAcqToggle(checked: boolean): void {
    const config = this.tuneUpConfig();
    if (!config || config.firmware !== 'AMax') return;
    // Optimistic: update the local mirror immediately so the toggle
    // doesn't snap back during the round-trip. If the apply fails we
    // re-sync from the authoritative config below.
    this.amaxEnableAcq.set(checked);

    const updatedConfig: DigitizerConfig = {
      ...config,
      channel_defaults: {
        ...config.channel_defaults,
        amax: {
          ...(config.channel_defaults?.amax ?? {}),
          enable_acq: checked ? 1 : 0,
        },
      },
    };

    this.operatorService
      .tuneupApply(config.digitizer_id, updatedConfig)
      .subscribe({
        next: (resp) => {
          if (resp.success) {
            this.tuneUpConfig.set(updatedConfig);
            this.digitizerService.loadDigitizers();
            this.notify.success(checked ? 'Debug acquisition ON' : 'Debug acquisition OFF');
          } else {
            // Roll back the optimistic toggle on backend rejection.
            this.amaxEnableAcq.set(!checked);
            this.notify.error('ENABLE_ACQ toggle failed: ' + (resp.message ?? 'unknown'));
          }
        },
        error: (err: unknown) => {
          this.amaxEnableAcq.set(!checked);
          const e = err as { error?: { message?: string }; message?: string };
          this.notify.error('ENABLE_ACQ toggle failed: ' + (e.error?.message ?? e.message ?? 'unknown'));
        },
      });
  }

  /** Handle the "Waveforms" slide toggle in the AMax Tune Up toolbar.
   *  Sends a partial config update via `tuneupApply` so the operator
   *  can flip OpenDPP waveform delivery without leaving Tune Up.
   *
   *  The FELib OpenDPP endpoint's data-format JSON is locked at
   *  endpoint setup time, so changing `board.waveforms_enabled` alone
   *  isn't enough — the reader's ApplyConfig handler also rebinds the
   *  endpoint via `configure_opendpp_endpoint` while acquisition is
   *  stopped (Tune Up Apply cycles Stop → Configured → Arm → Start). */
  onAmaxWaveformsEnabledToggle(checked: boolean): void {
    const config = this.tuneUpConfig();
    if (!config || config.firmware !== 'AMax') return;
    // Optimistic: update the local mirror immediately so the toggle
    // doesn't snap back during the round-trip. Rolled back on failure.
    this.amaxWaveformsEnabled.set(checked);

    const updatedConfig: DigitizerConfig = {
      ...config,
      board: {
        ...config.board,
        waveforms_enabled: checked,
      },
    };

    this.operatorService
      .tuneupApply(config.digitizer_id, updatedConfig)
      .subscribe({
        next: (resp) => {
          if (resp.success) {
            this.tuneUpConfig.set(updatedConfig);
            this.digitizerService.loadDigitizers();
            this.notify.success(checked ? 'Waveforms ON' : 'Waveforms OFF');
          } else {
            this.amaxWaveformsEnabled.set(!checked);
            this.notify.error('Waveforms toggle failed: ' + (resp.message ?? 'unknown'));
          }
        },
        error: (err: unknown) => {
          this.amaxWaveformsEnabled.set(!checked);
          const e = err as { error?: { message?: string }; message?: string };
          this.notify.error('Waveforms toggle failed: ' + (e.error?.message ?? e.message ?? 'unknown'));
        },
      });
  }

  private startHistogramPolling(): void {
    this.histPolling$.unsubscribe();
    this.histPolling$ = interval(1000)
      .pipe(
        takeUntil(this.destroy$),
        switchMap(() => {
          const selected = this.selectedChannels();
          if (selected.length === 0) return of(null);
          const [moduleId, channelId] = selected[0].split(':').map(Number);
          if (this.tuneUpDisplayMode() === 'psd2d') {
            return this.histogramService.fetchHistogram2d(moduleId, channelId).pipe(
              tap((hist: Histogram2D | null) => { if (hist) this.tuneUpHistogram2d.set(hist); }),
              map(() => null as Histogram1D | null),
            );
          }
          return this.histogramService.fetchHistogram(moduleId, channelId);
        })
      )
      .subscribe((hist) => {
        if (hist) this.tuneUpHistogram.set(hist);
      });
  }

  private stopHistogramPolling(): void {
    this.histPolling$.unsubscribe();
    this.histPolling$ = new Subscription();
  }

  // ===========================================================================
  // Waveform Data
  // ===========================================================================

  private fetchChannelList(): void {
    this.histogramService.fetchWaveformList().subscribe((response) => {
      if (response) {
        this.availableChannels.set(response.channels);

        if (this.selectedChannels().length === 0 && response.channels.length > 0) {
          const first = response.channels[0];
          this.selectedChannels.set([`${first.module_id}:${first.channel_id}`]);
        }
      }
    });
  }

  private fetchWaveforms() {
    // Single Shot: held state — skip fetch, keep current waveform
    if (this.isTuneUp() && this.triggerMode() === 'held') {
      return of(null);
    }

    const selected = this.selectedChannels();
    if (selected.length === 0) {
      this.waveforms.set([]);
      return of(null);
    }

    this.isLoading.set(true);

    const requests = selected.map((key) => {
      const [moduleId, channelId] = key.split(':').map(Number);
      return this.histogramService.fetchWaveform(moduleId, channelId);
    });

    return forkJoin(requests).pipe(
      switchMap((results) => {
        const waveforms = results.filter((wf): wf is LatestWaveform => wf !== null);
        this.waveforms.set(waveforms);

        // Single Shot: armed state — capture on new timestamp
        if (this.isTuneUp() && this.triggerMode() === 'armed' && waveforms.length > 0) {
          const latestTs = waveforms[0].timestamp_ns;
          if (latestTs !== this.lastArmedTimestamp) {
            this.triggerMode.set('held');
          }
        }

        // Accumulate waveforms in Tune Up mode (FIFO buffer)
        if (this.isTuneUp() && this.accumulateEnabled() && waveforms.length > 0) {
          const max = this.accumulateMax();
          const currentHistory = this.waveformHistory();
          // Only append waveforms with new timestamps (deduplicate)
          const newWaveforms = waveforms.filter(
            (wf) =>
              !currentHistory.some(
                (h) =>
                  h.module_id === wf.module_id &&
                  h.channel_id === wf.channel_id &&
                  h.timestamp_ns === wf.timestamp_ns,
              ),
          );
          if (newWaveforms.length > 0) {
            const history = [...currentHistory, ...newWaveforms];
            this.waveformHistory.set(history.length > max ? history.slice(history.length - max) : history);
          }
        }

        this.isLoading.set(false);
        return of(null);
      })
    );
  }

  // ===========================================================================
  // Chart Building
  // ===========================================================================

  private buildChannelCharts(): ChannelChart[] {
    const waveforms = this.waveforms();
    const config = this.probeConfig();
    const isAccumulating = this.isTuneUp() && this.accumulateEnabled();
    const history = isAccumulating ? this.waveformHistory() : [];

    return waveforms.map((wf) => {
      const channelInfo = this.availableChannels().find(
        (ch) => ch.module_id === wf.module_id && ch.channel_id === wf.channel_id
      );
      const label = channelInfo?.name ?? `Src${wf.module_id}/Ch${wf.channel_id}`;
      const series: unknown[] = [];
      const nsPerSample = wf.waveform.ns_per_sample || 0;
      const toX = nsPerSample > 0 ? (i: number) => i * nsPerSample : (i: number) => i;
      // Apply the 14-bit centering offset (+8191) only when the backend
      // marks the probe as signed — PHA1's trapezoid / Delta probes go
      // negative around baseline, and the offset shifts them into the
      // 0..16383 visible band alongside unsigned probes. Unsigned probes
      // (PSD1/PSD2/AMax) keep their natural scale.
      const OFFSET_14BIT_SIGNED = 8191;
      const offsetFor = (signed: boolean | undefined): number =>
        signed ? OFFSET_14BIT_SIGNED : 0;

      // Render accumulated history traces first (older = more transparent, analog only)
      if (isAccumulating && history.length > 0) {
        const channelHistory = history.filter(
          (h) =>
            h.module_id === wf.module_id &&
            h.channel_id === wf.channel_id &&
            h.timestamp_ns !== wf.timestamp_ns,
        );
        channelHistory.forEach((hw, idx) => {
          const opacity = 0.1 + (0.3 * idx) / Math.max(channelHistory.length - 1, 1);
          const hNs = hw.waveform.ns_per_sample || 0;
          const hToX = hNs > 0 ? (i: number) => i * hNs : (i: number) => i;
          const off1 = offsetFor(hw.waveform.analog_probe1_is_signed);
          const off2 = offsetFor(hw.waveform.analog_probe2_is_signed);
          if (config.analog1 && hw.waveform.analog_probe1.length > 0) {
            series.push({
              type: 'line',
              data: hw.waveform.analog_probe1.map((v, i) => [hToX(i), v + off1]),
              symbol: 'none',
              lineStyle: { width: 1, color: this.probeColors.analog1, opacity },
              itemStyle: { color: this.probeColors.analog1 },
              silent: true,
            });
          }
          if (config.analog2 && hw.waveform.analog_probe2.length > 0) {
            series.push({
              type: 'line',
              data: hw.waveform.analog_probe2.map((v, i) => [hToX(i), v + off2]),
              symbol: 'none',
              lineStyle: { width: 1, color: this.probeColors.analog2, opacity },
              itemStyle: { color: this.probeColors.analog2 },
              silent: true,
            });
          }
        });
      }

      const latestOff1 = offsetFor(wf.waveform.analog_probe1_is_signed);
      const latestOff2 = offsetFor(wf.waveform.analog_probe2_is_signed);
      const latestOff3 = offsetFor(wf.waveform.analog_probe3_is_signed);

      // Latest waveform (full opacity)
      if (config.analog1 && wf.waveform.analog_probe1.length > 0) {
        series.push({
          name: 'Analog 0',
          type: 'line',
          data: wf.waveform.analog_probe1.map((v, i) => [toX(i), v + latestOff1]),
          symbol: 'none',
          lineStyle: { width: 1.5, color: this.probeColors.analog1 },
          itemStyle: { color: this.probeColors.analog1 },
        });
      }

      if (config.analog2 && wf.waveform.analog_probe2.length > 0) {
        series.push({
          name: 'Analog 1',
          type: 'line',
          data: wf.waveform.analog_probe2.map((v, i) => [toX(i), v + latestOff2]),
          symbol: 'none',
          lineStyle: { width: 1.5, color: this.probeColors.analog2 },
          itemStyle: { color: this.probeColors.analog2 },
        });
      }

      const analog3 = wf.waveform.analog_probe3 ?? [];
      if (config.analog3 && analog3.length > 0) {
        series.push({
          name: 'Analog 2',
          type: 'line',
          data: analog3.map((v, i) => [toX(i), v + latestOff3]),
          symbol: 'none',
          lineStyle: { width: 1.5, color: this.probeColors.analog3 },
          itemStyle: { color: this.probeColors.analog3 },
        });
      }

      // Index-keyed digital probe data lookup. The carrier `Waveform`
      // struct is fixed at 16 slots (`digital_probe1..16`), so this stays
      // a flat lookup; the toolbar's `activeDigitalProbeSlots()` already
      // filters out empty/UNKNOWN slots, but the visibility check
      // (`digitalProbeVisible`) is the authoritative gate per slot.
      const digitalSlotData: (number[] | undefined)[] = [
        wf.waveform.digital_probe1, wf.waveform.digital_probe2,
        wf.waveform.digital_probe3, wf.waveform.digital_probe4,
        wf.waveform.digital_probe5, wf.waveform.digital_probe6,
        wf.waveform.digital_probe7, wf.waveform.digital_probe8,
        wf.waveform.digital_probe9, wf.waveform.digital_probe10,
        wf.waveform.digital_probe11, wf.waveform.digital_probe12,
        wf.waveform.digital_probe13, wf.waveform.digital_probe14,
        wf.waveform.digital_probe15, wf.waveform.digital_probe16,
      ];

      for (let slot = 0; slot < DIGITAL_PROBE_SLOTS; slot++) {
        const data = digitalSlotData[slot] ?? [];
        if (this.digitalProbeVisible(slot) && data.length > 0) {
          const baseColor = this.digitalProbeColor(slot);
          // Extract HIGH intervals from 0/1 array with minimum visible width
          const totalX = toX(data.length - 1);
          const minWidth = totalX * 0.005; // 0.5% of total range
          const areas: unknown[][] = [];
          let start: number | null = null;
          for (let idx = 0; idx < data.length; idx++) {
            if (data[idx] && start === null) {
              start = toX(idx);
            } else if (!data[idx] && start !== null) {
              const end = toX(idx);
              const width = end - start;
              areas.push([{ xAxis: start }, { xAxis: width < minWidth ? start + minWidth : end }]);
              start = null;
            }
          }
          if (start !== null) {
            const end = toX(data.length - 1);
            const width = end - start;
            areas.push([{ xAxis: start }, { xAxis: width < minWidth ? start + minWidth : end }]);
          }
          const seriesName = this.digitalProbeLabel(slot, 'short');
          // Invisible line series with markArea for full-height transparent bands
          series.push({
            name: seriesName,
            type: 'line',
            data: [],
            symbol: 'none',
            lineStyle: { width: 0 },
            itemStyle: { color: baseColor },
            markArea: {
              silent: true,
              itemStyle: { color: baseColor, opacity: 0.15 },
              data: areas,
            },
          });
        }
      }

      const samples = wf.waveform.analog_probe1.length || wf.waveform.analog_probe2.length;

      return {
        label,
        moduleId: wf.module_id,
        channelId: wf.channel_id,
        energy: wf.energy,
        samples,
        nsPerSample,
        options: this.buildSingleChartOptions(series, this.yAxisMode(), nsPerSample > 0),
      };
    });
  }

  private buildSingleChartOptions(series: unknown[], yMode: 'auto' | 'fixed', useTime = false): EChartsCoreOption {
    return {
      animation: false,
      grid: {
        left: 60,
        right: 50,
        top: 40,
        bottom: 50,
      },
      legend: {
        show: true,
        top: 5,
        type: 'scroll',
      },
      tooltip: {
        trigger: 'axis',
        axisPointer: {
          type: 'cross',
        },
      },
      xAxis: {
        type: 'value',
        name: useTime ? 'Time (ns)' : 'Sample',
        nameLocation: 'middle',
        nameGap: 25,
        min: 0,
      },
      yAxis: {
        type: 'value',
        name: 'ADC',
        ...(yMode === 'fixed' ? { min: -30000, max: 30000 } : {}),
        axisLabel: {
          formatter: (value: number) => {
            if (Math.abs(value) >= 1000) {
              return (value / 1000).toFixed(1) + 'k';
            }
            return value.toString();
          },
        },
      },
      dataZoom: [
        {
          type: 'inside',
          xAxisIndex: 0,
          yAxisIndex: [],
          zoomOnMouseWheel: 'shift',
          moveOnMouseMove: true,
          filterMode: 'none',
        },
        {
          type: 'inside',
          xAxisIndex: [],
          yAxisIndex: 0,
          zoomOnMouseWheel: 'ctrl',
          moveOnMouseMove: false,
          filterMode: 'none',
        },
        {
          type: 'slider',
          xAxisIndex: 0,
          height: 20,
          bottom: 5,
          filterMode: 'none',
        },
        {
          type: 'slider',
          yAxisIndex: 0,
          width: 20,
          right: 5,
          filterMode: 'none',
        },
      ],
      series,
    };
  }

  /** Initial chart options for Tune Up waveform (skeleton — data filled via merge) */
  private buildTuneUpWfInitOptions(): EChartsCoreOption {
    return {
      animation: false,
      grid: { left: 60, right: 50, top: 40, bottom: 50 },
      legend: { show: true, top: 5, type: 'scroll' },
      tooltip: { trigger: 'axis', axisPointer: { type: 'cross' } },
      xAxis: {
        type: 'value',
        name: 'Sample',
        nameLocation: 'middle',
        nameGap: 25,
        min: 0,
      },
      yAxis: {
        type: 'value',
        name: 'ADC',
        min: 0,
        max: 30000,
        axisLabel: {
          formatter: (value: number) => {
            if (Math.abs(value) >= 1000) return (value / 1000).toFixed(1) + 'k';
            return value.toString();
          },
        },
      },
      dataZoom: [
        { type: 'inside', xAxisIndex: 0, yAxisIndex: [], zoomOnMouseWheel: 'shift', moveOnMouseMove: true, filterMode: 'none' },
        { type: 'inside', xAxisIndex: [], yAxisIndex: 0, zoomOnMouseWheel: 'ctrl', moveOnMouseMove: false, filterMode: 'none' },
        { type: 'slider', xAxisIndex: 0, height: 20, bottom: 5, filterMode: 'none' },
        { type: 'slider', yAxisIndex: 0, width: 20, right: 5, filterMode: 'none' },
      ],
      series: [],
    };
  }

}
