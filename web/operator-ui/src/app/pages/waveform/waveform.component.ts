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
import { MatSnackBar, MatSnackBarModule } from '@angular/material/snack-bar';
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
import { getCategoryParams, getAllChannelParams, getProbeOptions, ChannelCategory, ProbeOption } from '../../models/channel-params';
import { HistogramChartComponent, RangeChangeEvent } from '../../components/histogram-chart/histogram-chart.component';
import { HeatmapChartComponent } from '../../components/heatmap-chart/heatmap-chart.component';

interface ProbeConfig {
  analog1: boolean;
  analog2: boolean;
  digital1: boolean;
  digital2: boolean;
  digital3: boolean;
  digital4: boolean;
}

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
            <mat-checkbox [checked]="probeConfig().digital1" (change)="toggleProbe('digital1')" color="primary">
              <span class="probe-label" [style.border-bottom-color]="probeColors.digital1">{{ probeLabels().d0Short }}</span>
            </mat-checkbox>
            <mat-checkbox [checked]="probeConfig().digital2" (change)="toggleProbe('digital2')" color="primary">
              <span class="probe-label" [style.border-bottom-color]="probeColors.digital2">{{ probeLabels().d1Short }}</span>
            </mat-checkbox>
            <mat-checkbox [checked]="probeConfig().digital3" (change)="toggleProbe('digital3')" color="primary">
              <span class="probe-label" [style.border-bottom-color]="probeColors.digital3">{{ probeLabels().d2Short }}</span>
            </mat-checkbox>
            <mat-checkbox [checked]="probeConfig().digital4" (change)="toggleProbe('digital4')" color="primary">
              <span class="probe-label" [style.border-bottom-color]="probeColors.digital4">{{ probeLabels().d3Short }}</span>
            </mat-checkbox>
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
                <app-histogram-chart
                  [histogram]="tuneUpHistogram()"
                  [logScale]="histLogScale()"
                  [xRange]="histXRange()"
                  (rangeChange)="onHistRangeChange($event)"
                />
              } @else {
                <app-heatmap-chart
                  [histogram]="tuneUpHistogram2d()"
                  [logScale]="histLogScale()"
                />
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
                <mat-button-toggle value="input">Input</mat-button-toggle>
                <mat-button-toggle value="trigger">Trigger</mat-button-toggle>
                <mat-button-toggle value="energy">Energy</mat-button-toggle>
                <mat-button-toggle value="coincidence">Coincidence</mat-button-toggle>
                <mat-button-toggle value="waveform">Waveform</mat-button-toggle>
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
              [checked]="probeConfig().digital1"
              (change)="toggleProbe('digital1')"
              color="primary"
            >
              <span class="probe-label" [style.border-bottom-color]="probeColors.digital1">{{ probeLabels().d0Long }}</span>
            </mat-checkbox>
            <mat-checkbox
              [checked]="probeConfig().digital2"
              (change)="toggleProbe('digital2')"
              color="primary"
            >
              <span class="probe-label" [style.border-bottom-color]="probeColors.digital2">{{ probeLabels().d1Long }}</span>
            </mat-checkbox>
            <mat-checkbox
              [checked]="probeConfig().digital3"
              (change)="toggleProbe('digital3')"
              color="primary"
            >
              <span class="probe-label" [style.border-bottom-color]="probeColors.digital3">{{ probeLabels().d2Long }}</span>
            </mat-checkbox>
            <mat-checkbox
              [checked]="probeConfig().digital4"
              (change)="toggleProbe('digital4')"
              color="primary"
            >
              <span class="probe-label" [style.border-bottom-color]="probeColors.digital4">{{ probeLabels().d3Long }}</span>
            </mat-checkbox>
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
      font-size: 11px;
      font-weight: 600;
      padding: 2px 6px;
      border-radius: 4px;
      margin-left: 4px;
    }
    .trigger-status.armed {
      background-color: #fff3e0;
      color: #e65100;
    }
    .trigger-status.held {
      background-color: #e8f5e9;
      color: #2e7d32;
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
  private readonly snackBar = inject(MatSnackBar);

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
    digital1: false,
    digital2: false,
    digital3: false,
    digital4: false,
  });
  readonly yAxisMode = signal<'auto' | 'fixed'>('auto');

  readonly probeColors = {
    analog1: '#1565c0',
    analog2: '#e65100',
    digital1: '#00897b',
    digital2: '#c62828',
    digital3: '#6a1b9a',
    digital4: '#5c6bc0',
  };

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

  /** All categories with their params (for grid layout) */
  private readonly allCategories: { key: ChannelCategory; label: string }[] = [
    { key: 'input', label: 'Input' },
    { key: 'trigger', label: 'Trigger' },
    { key: 'energy', label: 'Energy' },
    { key: 'coincidence', label: 'Coincidence' },
    { key: 'waveform', label: 'Waveform' },
  ];

  readonly categoryGrid = computed(() => {
    const config = this.tuneUpConfig();
    if (!config) return [];
    return this.allCategories
      .map(c => ({ ...c, params: getCategoryParams(config.firmware, c.key) }))
      .filter(c => c.params.length > 0 || c.key === 'waveform');
  });

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

  /** Probe labels for the toggle checkboxes — falls through "A0: TimeFilter"
   *  when the FW carries typed probe info on the wire (PHA2 today; future
   *  PSD2/PHA1 if/when their decoders parse it), otherwise "A0/A1/D0..D3"
   *  generic. We pick the first available waveform from `waveforms()`,
   *  falling back to the most recent in `waveformHistory()` so the
   *  labels survive a "Hold" / "Single Shot" pause. */
  readonly probeLabels = computed(() => {
    const wf = this.waveforms()[0]?.waveform
      ?? this.waveformHistory()[this.waveformHistory().length - 1]?.waveform;
    const apt = wf?.analog_probe_type;
    const dpt = wf?.digital_probe_type;
    const apLabel = (idx: 0 | 1, fallback: string) => {
      const code = apt?.[idx];
      if (code === undefined || code === UNKNOWN_PROBE_TYPE) return fallback;
      const name = ANALOG_PROBE_TYPE_LABELS[code];
      return name ? `${fallback}: ${name}` : fallback;
    };
    const dpLabel = (idx: 0 | 1 | 2 | 3, fallback: string) => {
      const code = dpt?.[idx];
      if (code === undefined || code === UNKNOWN_PROBE_TYPE) return fallback;
      const name = DIGITAL_PROBE_TYPE_LABELS[code];
      return name ? `${fallback}: ${name}` : fallback;
    };
    return {
      a0Short: apLabel(0, 'A0'),
      a1Short: apLabel(1, 'A1'),
      d0Short: dpLabel(0, 'D0'),
      d1Short: dpLabel(1, 'D1'),
      d2Short: dpLabel(2, 'D2'),
      d3Short: dpLabel(3, 'D3'),
      a0Long: apLabel(0, 'Analog 0'),
      a1Long: apLabel(1, 'Analog 1'),
      d0Long: dpLabel(0, 'Digital 0'),
      d1Long: dpLabel(1, 'Digital 1'),
      d2Long: dpLabel(2, 'Digital 2'),
      d3Long: dpLabel(3, 'Digital 3'),
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

  toggleProbe(probe: keyof ProbeConfig): void {
    const config = this.probeConfig();
    this.probeConfig.set({
      ...config,
      [probe]: !config[probe],
    });
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
          this.snackBar.open('Start failed: ' + resp.message, 'OK', { duration: 5000 });
        }
      },
      error: (err) => {
        this.tuneUpLoading.set(false);
        this.snackBar.open('Start error: ' + (err.error?.message ?? err.message), 'OK', {
          duration: 5000,
        });
      },
    });
  }

  onStopTuneUp(): void {
    this.tuneUpLoading.set(true);
    this.operatorService.tuneupStop().subscribe({
      next: () => {
        this.tuneUpLoading.set(false);
        this.snackBar.open('Tune Up stopped', 'OK', { duration: 3000 });
      },
      error: (err) => {
        this.tuneUpLoading.set(false);
        this.snackBar.open('Stop error: ' + (err.error?.message ?? err.message), 'OK', {
          duration: 5000,
        });
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
            this.snackBar.open('Configuration applied', 'OK', { duration: 3000 });
          } else {
            this.snackBar.open('Apply failed: ' + resp.message, 'OK', { duration: 5000 });
          }
        },
        error: (err) => {
          this.snackBar.open('Apply error: ' + (err.error?.message ?? err.message), 'OK', {
            duration: 5000,
          });
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

      const digitalProbes: { key: keyof ProbeConfig; data: number[]; index: number }[] = [
        { key: 'digital1', data: wf.waveform.digital_probe1, index: 0 },
        { key: 'digital2', data: wf.waveform.digital_probe2, index: 1 },
        { key: 'digital3', data: wf.waveform.digital_probe3, index: 2 },
        { key: 'digital4', data: wf.waveform.digital_probe4, index: 3 },
      ];

      for (const dp of digitalProbes) {
        if (config[dp.key] && dp.data.length > 0) {
          const colorKey = dp.key as keyof typeof this.probeColors;
          const baseColor = this.probeColors[colorKey];
          // Extract HIGH intervals from 0/1 array with minimum visible width
          const totalX = toX(dp.data.length - 1);
          const minWidth = totalX * 0.005; // 0.5% of total range
          const areas: unknown[][] = [];
          let start: number | null = null;
          for (let idx = 0; idx < dp.data.length; idx++) {
            if (dp.data[idx] && start === null) {
              start = toX(idx);
            } else if (!dp.data[idx] && start !== null) {
              const end = toX(idx);
              const width = end - start;
              areas.push([{ xAxis: start }, { xAxis: width < minWidth ? start + minWidth : end }]);
              start = null;
            }
          }
          if (start !== null) {
            const end = toX(dp.data.length - 1);
            const width = end - start;
            areas.push([{ xAxis: start }, { xAxis: width < minWidth ? start + minWidth : end }]);
          }
          // Invisible line series with markArea for full-height transparent bands
          series.push({
            name: `Digital ${dp.index}`,
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
