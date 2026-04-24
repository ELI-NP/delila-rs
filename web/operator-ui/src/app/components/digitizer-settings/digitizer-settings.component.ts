import { Component, inject, signal, computed, effect } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { MatCardModule } from '@angular/material/card';
import { MatSelectModule } from '@angular/material/select';
import { MatInputModule } from '@angular/material/input';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatButtonModule } from '@angular/material/button';
import { MatSlideToggleModule } from '@angular/material/slide-toggle';
import { MatCheckboxModule } from '@angular/material/checkbox';
import { MatIconModule } from '@angular/material/icon';
import { MatSnackBar, MatSnackBarModule } from '@angular/material/snack-bar';
import { MatDividerModule } from '@angular/material/divider';
import { MatTabsModule } from '@angular/material/tabs';
import { MatTooltipModule } from '@angular/material/tooltip';
import { MatProgressSpinnerModule } from '@angular/material/progress-spinner';
import { firstValueFrom } from 'rxjs';
import { DigitizerService } from '../../services/digitizer.service';
import { OperatorService } from '../../services/operator.service';
import { FirmwareType, X743Config } from '../../models/types';
import { getCategoryParams, getAllChannelParams, getProbeOptions, ProbeOption } from '../../models/channel-params';
import {
  ChannelTableComponent,
  DefaultValueChange,
  ChannelValueChange,
} from '../channel-table/channel-table.component';

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
    MatCheckboxModule,
    MatIconModule,
    MatSnackBarModule,
    MatDividerModule,
    MatTabsModule,
    MatProgressSpinnerModule,
    MatTooltipModule,
    ChannelTableComponent,
  ],
  template: `
    <div class="digitizer-settings" tabindex="0" (keydown.enter)="onEnterKey($event)">
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
          [disabled]="!selectedConfig() || applying()"
          [matTooltip]="isRunning() ? 'Only SetInRun parameters will be applied' : ''"
        >
          <mat-icon>check</mat-icon>
          {{ isRunning() ? 'Apply (Runtime)' : 'Apply' }}
        </button>
      </div>

      @if (selectedConfig(); as config) {
        <!-- 6-tab layout: Board / Input / Trigger / Energy / Coincidence / Waveform -->
        <mat-tab-group animationDuration="0ms" [selectedIndex]="selectedTabIndex()" (selectedIndexChange)="selectedTabIndex.set($event)">
          <!-- Tab 1: Board Settings -->
          <mat-tab label="Board">
            <div class="tab-content">
              @if (config.firmware === 'X743Std' && config.x743) {
                <mat-card class="config-card">
                  <mat-card-content>
                    <h3 class="section-title">SAM (Switched-Capacitor)</h3>
                    <div class="form-grid">
                      <mat-form-field appearance="outline">
                        <mat-label>Sampling Frequency</mat-label>
                        <mat-select panelClass="fit-content-panel" [(value)]="config.x743.sampling_frequency">
                          <mat-option value="3.2ghz">3.2 GHz</mat-option>
                          <mat-option value="1.6ghz">1.6 GHz</mat-option>
                          <mat-option value="800mhz">800 MHz</mat-option>
                          <mat-option value="400mhz">400 MHz</mat-option>
                        </mat-select>
                      </mat-form-field>

                      <mat-form-field appearance="outline">
                        <mat-label>Correction Level</mat-label>
                        <mat-select panelClass="fit-content-panel" [(value)]="config.x743.correction_level">
                          <mat-option value="all">All (Pedestal + INL + Timing)</mat-option>
                          <mat-option value="pedestal_only">Pedestal Only</mat-option>
                          <mat-option value="inl">INL Only</mat-option>
                          <mat-option value="disabled">Disabled</mat-option>
                        </mat-select>
                      </mat-form-field>
                    </div>

                    <mat-divider></mat-divider>
                    <h3 class="section-title">Acquisition</h3>
                    <div class="form-grid">
                      <mat-form-field appearance="outline">
                        <mat-label>Record Length (samples)</mat-label>
                        <input matInput type="number" [(ngModel)]="config.x743.record_length" min="16" max="1024" step="16" (blur)="snapBoardValue($event, 16, 16, 1024)" />
                      </mat-form-field>

                      <mat-form-field appearance="outline">
                        <mat-label>Post-Trigger Size</mat-label>
                        <input matInput type="number" [(ngModel)]="config.x743.post_trigger_size" min="1" max="255" step="1" />
                      </mat-form-field>

                      <mat-form-field appearance="outline">
                        <mat-label>Max Events / BLT</mat-label>
                        <input matInput type="number" [(ngModel)]="config.x743.max_num_events_blt" min="1" />
                      </mat-form-field>
                    </div>

                    <mat-divider></mat-divider>
                    <h3 class="section-title">Trigger &amp; I/O</h3>
                    <div class="form-grid">
                      <mat-form-field appearance="outline">
                        <mat-label>FPIO Level</mat-label>
                        <mat-select panelClass="fit-content-panel" [(value)]="config.x743.io_level">
                          <mat-option value="nim">NIM</mat-option>
                          <mat-option value="ttl">TTL</mat-option>
                        </mat-select>
                      </mat-form-field>

                      <mat-form-field appearance="outline">
                        <mat-label>Trigger Source</mat-label>
                        <mat-select panelClass="fit-content-panel" [(value)]="config.x743.trigger_source">
                          <mat-option value="software">Software</mat-option>
                          <mat-option value="external">External (TRG-IN)</mat-option>
                          <mat-option value="self">Self Trigger</mat-option>
                        </mat-select>
                      </mat-form-field>
                    </div>

                    <mat-divider></mat-divider>
                    <h3 class="section-title">Group Enable</h3>
                    <p class="hint-text">Each group covers 2 channels. Channels within a disabled group are skipped.</p>
                    <div class="group-grid">
                      @for (g of [0,1,2,3,4,5,6,7]; track g) {
                        <mat-checkbox
                          [checked]="isGroupEnabled(config.x743.group_enable_mask, g)"
                          (change)="toggleGroup(config.x743, g, $event.checked)"
                        >
                          Group {{ g }} (ch {{ g * 2 }}-{{ g * 2 + 1 }})
                        </mat-checkbox>
                      }
                    </div>

                    <mat-divider></mat-divider>
                    <h3 class="section-title">Test Pulse Generator</h3>
                    <div class="form-grid">
                      <mat-slide-toggle [(ngModel)]="config.x743.pulse_gen_enabled">
                        Enable Test Pulse
                      </mat-slide-toggle>

                      <mat-form-field appearance="outline">
                        <mat-label>Pulse Pattern (16-bit)</mat-label>
                        <input matInput type="number" [(ngModel)]="config.x743.pulse_pattern" min="0" max="65535" step="1" />
                      </mat-form-field>

                      <mat-form-field appearance="outline">
                        <mat-label>Pulse Source</mat-label>
                        <mat-select panelClass="fit-content-panel" [(value)]="config.x743.pulse_source">
                          <mat-option value="software">Software</mat-option>
                          <mat-option value="continuous">Continuous</mat-option>
                        </mat-select>
                      </mat-form-field>
                    </div>
                  </mat-card-content>
                </mat-card>
              } @else {
              <mat-card class="config-card">
                <mat-card-content>
                  <h3 class="section-title">Clock &amp; Sync</h3>
                  <div class="form-grid">
                    <mat-form-field appearance="outline">
                      <mat-label>Start Source</mat-label>
                      <mat-select panelClass="fit-content-panel" [(value)]="config.board.start_source">
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
                        <mat-select panelClass="fit-content-panel" [(value)]="config.board.extra!['clocksource']">
                          <mat-option value="Internal">Internal</mat-option>
                          <mat-option value="FPClkIn">FPClkIn</mat-option>
                        </mat-select>
                      </mat-form-field>

                      <mat-form-field appearance="outline">
                        <mat-label>Output Clock</mat-label>
                        <mat-select panelClass="fit-content-panel" [(value)]="config.board.extra!['enclockoutfp']">
                          <mat-option value="True">Enabled</mat-option>
                          <mat-option value="False">Disabled</mat-option>
                        </mat-select>
                      </mat-form-field>

                      <mat-form-field appearance="outline">
                        <mat-label>SyncOut Signal</mat-label>
                        <mat-select panelClass="fit-content-panel" [(value)]="config.board.extra!['syncoutmode']">
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
                        <input matInput type="number" [(ngModel)]="config.board.extra!['rundelay']" min="0" max="524280" step="8" (blur)="snapBoardValue($event, 0, 8, 524280)" />
                      </mat-form-field>
                    } @else {
                      <mat-form-field appearance="outline">
                        <mat-label>Ext Clock</mat-label>
                        <mat-select panelClass="fit-content-panel" [(value)]="config.board.ext_clock">
                          <mat-option value="FALSE">Disabled</mat-option>
                          <mat-option value="TRUE">Enabled</mat-option>
                        </mat-select>
                      </mat-form-field>

                      <mat-form-field appearance="outline">
                        <mat-label>Start Delay (ns)</mat-label>
                        <input matInput type="number" [(ngModel)]="config.board.start_delay" min="0" max="4080" step="16" (blur)="snapBoardValue($event, 0, 16, 4080)" />
                      </mat-form-field>
                    }
                  </div>

                  <mat-divider></mat-divider>
                  <h3 class="section-title">Trigger &amp; I/O</h3>
                  <div class="form-grid">
                    @if (config.firmware === 'PSD2') {
                      <mat-form-field appearance="outline">
                        <mat-label>Global Trigger Source</mat-label>
                        <mat-select panelClass="fit-content-panel" [(value)]="config.board.global_trigger_source">
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
                        </mat-select>
                      </mat-form-field>
                    }

                    <mat-form-field appearance="outline">
                      <mat-label>FPIO Type</mat-label>
                      <mat-select panelClass="fit-content-panel" [(value)]="config.board.io_level">
                        @if (config.firmware === 'PSD2') {
                          <mat-option value="NIM">NIM</mat-option>
                          <mat-option value="TTL">TTL</mat-option>
                        } @else {
                          <mat-option value="FPIOTYPE_NIM">NIM</mat-option>
                          <mat-option value="FPIOTYPE_TTL">TTL</mat-option>
                        }
                      </mat-select>
                    </mat-form-field>

                    @if (config.firmware === 'PSD2') {
                      <mat-form-field appearance="outline">
                        <mat-label>GPO Mode</mat-label>
                        <mat-select panelClass="fit-content-panel" [(value)]="config.board.gpio_mode">
                          @for (opt of gpoModeOptions(config.firmware); track opt) {
                            <mat-option [value]="opt">{{ opt }}</mat-option>
                          }
                        </mat-select>
                      </mat-form-field>
                    }

                    @if (config.firmware === 'PSD2') {
                      <mat-form-field appearance="outline">
                        <mat-label>TRG OUT Mode</mat-label>
                        <mat-select panelClass="fit-content-panel" [(value)]="config.board.extra!['trgoutmode']">
                          @for (opt of trgoutModeOptions(); track opt) {
                            <mat-option [value]="opt">{{ opt }}</mat-option>
                          }
                        </mat-select>
                      </mat-form-field>
                    } @else {
                      <mat-form-field appearance="outline">
                        <mat-label>TRG OUT / GPO</mat-label>
                        <mat-select panelClass="fit-content-panel" [(value)]="config.board.gpio_mode">
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
                        <mat-select panelClass="fit-content-panel" [(value)]="config.board.extra!['boardvetosource']">
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
                        <mat-select panelClass="fit-content-panel" [(value)]="config.board.extra!['boardvetopolarity']">
                          <mat-option value="ActiveHigh">ActiveHigh</mat-option>
                          <mat-option value="ActiveLow">ActiveLow</mat-option>
                        </mat-select>
                      </mat-form-field>

                      <mat-form-field appearance="outline">
                        <mat-label>Veto Width (ns)</mat-label>
                        <input matInput type="number" [(ngModel)]="config.board.extra!['boardvetowidth']" min="0" step="8" (blur)="snapBoardValue($event, 0, 8)" />
                      </mat-form-field>
                    </div>
                  }

                  <mat-divider></mat-divider>
                  <h3 class="section-title">Data Acquisition</h3>
                  <div class="form-grid">
                    <mat-form-field appearance="outline">
                      <mat-label>Record Length (ns)</mat-label>
                      <input matInput type="number" [(ngModel)]="config.board.record_length" min="16" step="16" (blur)="snapBoardValue($event, 16, 16)" />
                    </mat-form-field>

                    <mat-slide-toggle [(ngModel)]="config.board.waveforms_enabled">
                      Enable Waveforms
                    </mat-slide-toggle>

                    @if (config.firmware !== 'PSD2') {
                      <mat-form-field appearance="outline">
                        <mat-label>Fine TS Mode</mat-label>
                        <mat-select panelClass="fit-content-panel" [(value)]="config.board.fine_ts_mode">
                          <mat-option value="hardware">HW (FPGA)</mat-option>
                          <mat-option value="software">SW (SAZC/SBZC)</mat-option>
                        </mat-select>
                      </mat-form-field>

                      <mat-form-field appearance="outline">
                        <mat-label>Extras</mat-label>
                        <mat-select panelClass="fit-content-panel" [(value)]="config.board.extras_enabled">
                          <mat-option value="TRUE">Enabled</mat-option>
                          <mat-option value="FALSE">Disabled</mat-option>
                        </mat-select>
                      </mat-form-field>

                      <mat-form-field appearance="outline">
                        <mat-label>Event Aggregation</mat-label>
                        <input matInput type="number" [(ngModel)]="config.board.event_aggregation" min="1" max="1023" />
                      </mat-form-field>

                      <mat-form-field appearance="outline">
                        <mat-label>Coincidence Window (ns)</mat-label>
                        <input matInput type="number" [(ngModel)]="config.board.coinc_trgout" min="0" max="8184" step="8" (blur)="snapBoardValue($event, 0, 8, 8184)" />
                      </mat-form-field>
                    }
                  </div>

                </mat-card-content>
              </mat-card>
              }
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
              @if (config.firmware === 'X743Std' && config.x743) {
                <!-- V1743: board-level post-processing (amplitude from waveform) -->
                <mat-card class="config-card">
                  <mat-card-content>
                    <h3 class="section-title">Energy Post-Processing</h3>
                    <p class="hint-text">V1743 has no DPP energy; these settings control Rust-side amplitude extraction from the waveform.</p>
                    <div class="form-grid">
                      <mat-form-field appearance="outline">
                        <mat-label>Energy Source</mat-label>
                        <mat-select panelClass="fit-content-panel" [(value)]="config.x743.energy_source">
                          <mat-option value="amplitude">Amplitude (|peak − baseline|)</mat-option>
                          <mat-option value="charge">Charge (unavailable in Standard mode)</mat-option>
                          <mat-option value="soft">Soft Charge (reserved)</mat-option>
                        </mat-select>
                      </mat-form-field>

                      <mat-form-field appearance="outline">
                        <mat-label>Baseline Samples</mat-label>
                        <input matInput type="number" [(ngModel)]="config.x743.baseline_samples" min="1" />
                      </mat-form-field>

                      <mat-form-field appearance="outline">
                        <mat-label>Energy Scale</mat-label>
                        <input matInput type="number" [(ngModel)]="config.x743.energy_scale" step="0.01" />
                      </mat-form-field>

                      <mat-form-field appearance="outline">
                        <mat-label>Energy Offset</mat-label>
                        <input matInput type="number" [(ngModel)]="config.x743.energy_offset" step="0.01" />
                      </mat-form-field>
                    </div>
                  </mat-card-content>
                </mat-card>
              } @else {
                <app-channel-table
                  [params]="energyParams()"
                  [numChannels]="config.num_channels"
                  [defaultValues]="defaultValues()"
                  [channelValues]="channelValues()"
                  [disabledKeys]="disabledKeys()"
                  (defaultChange)="onDefaultChange($event)"
                  (channelChange)="onChannelChange($event)"
                />
              }
            </div>
          </mat-tab>

          <!-- Tab 5: Coincidence -->
          <mat-tab label="Coincidence">
            <div class="tab-content">
              @if (config.firmware === 'X743Std') {
                <mat-card class="config-card">
                  <mat-card-content>
                    <p class="na-message">Coincidence settings are not applicable for V1743 Standard mode.</p>
                  </mat-card-content>
                </mat-card>
              } @else {
                <app-channel-table
                  [params]="coincidenceParams()"
                  [numChannels]="config.num_channels"
                  [defaultValues]="defaultValues()"
                  [channelValues]="channelValues()"
                  [disabledKeys]="disabledKeys()"
                  (defaultChange)="onDefaultChange($event)"
                  (channelChange)="onChannelChange($event)"
                />
              }
            </div>
          </mat-tab>

          <!-- Tab 6: Waveform -->
          <mat-tab label="Waveform">
            <div class="tab-content">
              @if (config.firmware === 'X743Std' && config.x743) {
                <!-- V1743: board-level waveform + software CFD -->
                <mat-card class="config-card">
                  <mat-card-content>
                    <h3 class="section-title">Waveform Acquisition</h3>
                    <div class="form-grid">
                      <mat-slide-toggle [(ngModel)]="config.x743.save_waveform">
                        Save Waveform
                      </mat-slide-toggle>
                    </div>

                    <mat-divider></mat-divider>
                    <h3 class="section-title">Software CFD (Fine Timestamp)</h3>
                    <p class="hint-text">V1743 Standard mode has no hardware CFD; fine time is computed in Rust from the waveform.</p>
                    <div class="form-grid">
                      <mat-form-field appearance="outline">
                        <mat-label>CFD Delay (samples)</mat-label>
                        <input matInput type="number" [(ngModel)]="config.x743.cfd_delay_samples" min="1" step="1" />
                      </mat-form-field>

                      <mat-form-field appearance="outline">
                        <mat-label>CFD Fraction</mat-label>
                        <input matInput type="number" [(ngModel)]="config.x743.cfd_fraction" min="0.05" max="0.95" step="0.05" />
                      </mat-form-field>
                    </div>
                  </mat-card-content>
                </mat-card>
              } @else if (waveformParams().length > 0) {
                <!-- PSD2: Channel-level waveform settings -->
                <app-channel-table
                  [params]="waveformParams()"
                  [numChannels]="config.num_channels"
                  [defaultValues]="defaultValues()"
                  [channelValues]="channelValues()"
                  [disabledKeys]="disabledKeys()"
                  (defaultChange)="onDefaultChange($event)"
                  (channelChange)="onChannelChange($event)"
                />
              } @else {
                <!-- PSD1/PHA1: Board-level waveform settings -->
                <mat-card class="config-card">
                  <mat-card-content>
                    <h3 class="section-title">Waveform Acquisition</h3>
                    <div class="form-grid">
                      <mat-slide-toggle [(ngModel)]="config.board.waveforms_enabled">
                        Enable Waveforms
                      </mat-slide-toggle>

                      <mat-form-field appearance="outline">
                        <mat-label>Record Length (ns)</mat-label>
                        <input matInput type="number" [(ngModel)]="config.board.record_length" min="16" step="16" (blur)="snapBoardValue($event, 16, 16)" />
                      </mat-form-field>
                    </div>

                    <mat-divider></mat-divider>
                    <h3 class="section-title">Virtual Probes</h3>
                    <p class="hint-text">These settings apply to all channels (board-level)</p>
                    <div class="form-grid">
                      <mat-form-field appearance="outline">
                        <mat-label>Analog Probe 1</mat-label>
                        <mat-select panelClass="fit-content-panel" [(value)]="config.board.vtrace_probe_0">
                          @for (opt of probeOptions()[0]; track opt.value) {
                            <mat-option [value]="opt.value">{{ opt.label }}</mat-option>
                          }
                        </mat-select>
                      </mat-form-field>

                      <mat-form-field appearance="outline">
                        <mat-label>Analog Probe 2</mat-label>
                        <mat-select panelClass="fit-content-panel" [(value)]="config.board.vtrace_probe_1">
                          @for (opt of probeOptions()[1]; track opt.value) {
                            <mat-option [value]="opt.value">{{ opt.label }}</mat-option>
                          }
                        </mat-select>
                      </mat-form-field>

                      <mat-form-field appearance="outline">
                        <mat-label>Digital Probe 1</mat-label>
                        <mat-select panelClass="fit-content-panel" [(value)]="config.board.vtrace_probe_2">
                          @for (opt of probeOptions()[2]; track opt.value) {
                            <mat-option [value]="opt.value">{{ opt.label }}</mat-option>
                          }
                        </mat-select>
                      </mat-form-field>

                      <mat-form-field appearance="outline">
                        <mat-label>Digital Probe 2</mat-label>
                        <mat-select panelClass="fit-content-panel" [(value)]="config.board.vtrace_probe_3">
                          @for (opt of probeOptions()[3]; track opt.value) {
                            <mat-option [value]="opt.value">{{ opt.label }}</mat-option>
                          }
                        </mat-select>
                      </mat-form-field>
                    </div>
                  </mat-card-content>
                </mat-card>
              }
            </div>
          </mat-tab>
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

    .firmware-badge.x743std {
      background-color: #f3e5f5;
      color: #7b1fa2;
    }

    .group-grid {
      display: grid;
      grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
      gap: 8px 16px;
      padding: 8px 0;
    }

    .na-message {
      padding: 24px;
      text-align: center;
      color: #999;
      font-style: italic;
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

    .hint-text {
      font-size: 12px;
      color: #888;
      font-style: italic;
      margin: 4px 0 8px;
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
  readonly selectedId = this.digitizerService.selectedDigitizerId;
  readonly selectedTabIndex = this.digitizerService.selectedTabIndex;
  readonly detecting = signal(false);
  readonly applying = signal(false);

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

  /** Virtual Probe options per firmware (board-level, PSD1/PHA1 only) */
  readonly probeOptions = computed((): ProbeOption[][] => {
    const fw = this.selectedConfig()?.firmware;
    return fw ? [0, 1, 2, 3].map((i) => getProbeOptions(fw, i)) : [[], [], [], []];
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

  onEnterKey(event: Event): void {
    const target = event.target as HTMLElement;
    if (target.tagName === 'INPUT') {
      (target as HTMLInputElement).blur(); // commit value via change+blur handlers
      this.applyConfig();
    }
  }

  async applyConfig(): Promise<void> {
    const config = this.selectedConfig();
    if (!config || this.applying()) return;

    this.applying.set(true);

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
      let result: { success: boolean; message?: string };

      if (this.operator.isTuneUp()) {
        // Tune Up mode: use tuneupApply which does Stop → Apply → Arm → Start
        // This ensures SetInRun=False parameters (pre-trigger, record_length, etc.) are applied
        result = await firstValueFrom(
          this.operator.tuneupApply(config.digitizer_id, updatedConfig)
        );
      } else {
        // Normal mode: use direct apply
        result = await this.digitizerService.applyToHardware(updatedConfig);
      }

      this.snackBar.open(result.message || 'Configuration applied to hardware', 'OK', {
        duration: 5000,
      });
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : 'Failed to apply configuration';
      this.snackBar.open(message, 'Close', {
        duration: 5000,
      });
    } finally {
      this.applying.set(false);
    }
  }

  resetConfig(): void {
    if (this.selectedId() !== null) {
      this.digitizerService.loadDigitizers();
      this.snackBar.open('Configuration reset', 'OK', { duration: 2000 });
    }
  }

  // ===========================================================================
  // X743Std Board helpers
  // ===========================================================================

  /** Check whether group `g` (0..7) is set in the X743 group_enable_mask */
  isGroupEnabled(mask: number | undefined, g: number): boolean {
    return ((mask ?? 0) & (1 << g)) !== 0;
  }

  /** Flip bit `g` (0..7) in the X743 group_enable_mask. Mutates x743 in place. */
  toggleGroup(x743: X743Config, g: number, checked: boolean): void {
    const mask = x743.group_enable_mask ?? 0;
    x743.group_enable_mask = checked ? mask | (1 << g) : mask & ~(1 << g);
  }

  /** Snap a board-level number input to the nearest valid step on blur */
  snapBoardValue(event: Event, min: number, step: number, max?: number): void {
    const el = event.target as HTMLInputElement;
    if (el.value === '') return;
    const value = Number(el.value);
    if (isNaN(value)) return;
    const snapped = Math.round((value - min) / step) * step + min;
    const clamped = Math.min(Math.max(snapped, min), max ?? Infinity);
    if (clamped !== value) {
      el.value = String(clamped);
      el.dispatchEvent(new Event('input'));
    }
  }
}
