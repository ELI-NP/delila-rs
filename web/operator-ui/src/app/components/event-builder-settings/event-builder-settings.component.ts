import { Component, OnInit, inject } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { MatCardModule } from '@angular/material/card';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatInputModule } from '@angular/material/input';
import { MatSelectModule } from '@angular/material/select';
import { MatButtonModule } from '@angular/material/button';
import { MatIconModule } from '@angular/material/icon';
import { MatDividerModule } from '@angular/material/divider';
import { MatSnackBar, MatSnackBarModule } from '@angular/material/snack-bar';
import { MatTooltipModule } from '@angular/material/tooltip';
import { MatTabsModule } from '@angular/material/tabs';
import { MatTableModule } from '@angular/material/table';
import { MatCheckboxModule } from '@angular/material/checkbox';
import { MatProgressSpinnerModule } from '@angular/material/progress-spinner';
import { MatChipsModule } from '@angular/material/chips';
import { MatExpansionModule } from '@angular/material/expansion';
import { MatDialogModule, MatDialog } from '@angular/material/dialog';
import { EventBuilderService } from '../../services/event-builder.service';
import { EventBuilderConfig, ChSettings, L2Setting } from '../../models/types';

@Component({
  selector: 'app-event-builder-settings',
  standalone: true,
  imports: [
    CommonModule,
    FormsModule,
    MatCardModule,
    MatFormFieldModule,
    MatInputModule,
    MatSelectModule,
    MatButtonModule,
    MatIconModule,
    MatDividerModule,
    MatSnackBarModule,
    MatTooltipModule,
    MatTabsModule,
    MatTableModule,
    MatCheckboxModule,
    MatProgressSpinnerModule,
    MatChipsModule,
    MatExpansionModule,
    MatDialogModule,
  ],
  template: `
    <div class="event-builder-settings">
      @if (service.loading()) {
        <div class="loading-overlay">
          <mat-spinner diameter="40"></mat-spinner>
        </div>
      }

      <!-- Experiment/Config Selection -->
      <mat-card class="selection-card">
        <mat-card-header>
          <mat-card-title>Event Builder Configuration</mat-card-title>
        </mat-card-header>
        <mat-card-content>
          <div class="selection-row">
            <mat-form-field>
              <mat-label>Experiment</mat-label>
              <mat-select [(ngModel)]="selectedExp" (selectionChange)="onExpChange()">
                @for (exp of service.experiments(); track exp) {
                  <mat-option [value]="exp">{{ exp }}</mat-option>
                }
              </mat-select>
            </mat-form-field>

            <mat-form-field>
              <mat-label>Configuration</mat-label>
              <mat-select [(ngModel)]="selectedConfig" (selectionChange)="onConfigChange()" [disabled]="!selectedExp">
                @for (config of service.configs(); track config.name) {
                  <mat-option [value]="config.name">{{ config.name }} (v{{ config.version }})</mat-option>
                }
              </mat-select>
            </mat-form-field>

            <button mat-raised-button color="primary" (click)="createNewConfig()" [disabled]="!selectedExp">
              <mat-icon>add</mat-icon>
              New Config
            </button>
          </div>

          @if (service.error()) {
            <div class="error-banner">
              <mat-icon>error</mat-icon>
              {{ service.error() }}
            </div>
          }
        </mat-card-content>
      </mat-card>

      @if (service.currentConfig(); as config) {
        <!-- Config Info -->
        <mat-card class="info-card">
          <mat-card-content>
            <div class="config-info">
              <div class="info-item">
                <span class="info-label">Name:</span>
                <span class="info-value">{{ config.name }}</span>
              </div>
              <div class="info-item">
                <span class="info-label">Experiment:</span>
                <span class="info-value">{{ config.exp_name }}</span>
              </div>
              <div class="info-item">
                <span class="info-label">Version:</span>
                <span class="info-value">{{ config.version }}</span>
              </div>
              <div class="info-item">
                <span class="info-label">Coincidence Window:</span>
                <span class="info-value">{{ config.coincidence_window_ns }} ns</span>
              </div>
              <div class="info-item">
                <span class="info-label">Channels:</span>
                <span class="info-value">{{ countChannels(config) }}</span>
              </div>
              <div class="info-item">
                <span class="info-label">Triggers:</span>
                <span class="info-value">{{ countTriggers(config) }}</span>
              </div>
            </div>
          </mat-card-content>
        </mat-card>

        <!-- Tabs for settings -->
        <mat-tab-group>
          <!-- Channel Settings Tab -->
          <mat-tab label="Channel Settings (chSettings)">
            <div class="tab-content">
              <div class="channel-table-container">
                <table mat-table [dataSource]="flattenedChannels()" class="channel-table">
                  <ng-container matColumnDef="id">
                    <th mat-header-cell *matHeaderCellDef>ID</th>
                    <td mat-cell *matCellDef="let ch">{{ ch.ID }}</td>
                  </ng-container>

                  <ng-container matColumnDef="module">
                    <th mat-header-cell *matHeaderCellDef>Module</th>
                    <td mat-cell *matCellDef="let ch">{{ ch.Module }}</td>
                  </ng-container>

                  <ng-container matColumnDef="channel">
                    <th mat-header-cell *matHeaderCellDef>Channel</th>
                    <td mat-cell *matCellDef="let ch">{{ ch.Channel }}</td>
                  </ng-container>

                  <ng-container matColumnDef="detectorType">
                    <th mat-header-cell *matHeaderCellDef>Detector Type</th>
                    <td mat-cell *matCellDef="let ch">
                      <mat-form-field class="compact-field">
                        <input matInput [(ngModel)]="ch.DetectorType" />
                      </mat-form-field>
                    </td>
                  </ng-container>

                  <ng-container matColumnDef="isTrigger">
                    <th mat-header-cell *matHeaderCellDef>Trigger</th>
                    <td mat-cell *matCellDef="let ch">
                      <mat-checkbox [(ngModel)]="ch.IsEventTrigger"></mat-checkbox>
                    </td>
                  </ng-container>

                  <ng-container matColumnDef="threshold">
                    <th mat-header-cell *matHeaderCellDef>Threshold (ADC)</th>
                    <td mat-cell *matCellDef="let ch">
                      <mat-form-field class="compact-field">
                        <input matInput type="number" [(ngModel)]="ch.ThresholdADC" />
                      </mat-form-field>
                    </td>
                  </ng-container>

                  <ng-container matColumnDef="hasAC">
                    <th mat-header-cell *matHeaderCellDef>Has AC</th>
                    <td mat-cell *matCellDef="let ch">
                      <mat-checkbox [(ngModel)]="ch.HasAC"></mat-checkbox>
                    </td>
                  </ng-container>

                  <ng-container matColumnDef="acModule">
                    <th mat-header-cell *matHeaderCellDef>AC Module</th>
                    <td mat-cell *matCellDef="let ch">
                      <mat-form-field class="compact-field">
                        <input matInput type="number" [(ngModel)]="ch.ACModule" [disabled]="!ch.HasAC" />
                      </mat-form-field>
                    </td>
                  </ng-container>

                  <ng-container matColumnDef="acChannel">
                    <th mat-header-cell *matHeaderCellDef>AC Channel</th>
                    <td mat-cell *matCellDef="let ch">
                      <mat-form-field class="compact-field">
                        <input matInput type="number" [(ngModel)]="ch.ACChannel" [disabled]="!ch.HasAC" />
                      </mat-form-field>
                    </td>
                  </ng-container>

                  <ng-container matColumnDef="tags">
                    <th mat-header-cell *matHeaderCellDef>Tags</th>
                    <td mat-cell *matCellDef="let ch">
                      <mat-chip-set>
                        @for (tag of ch.Tags; track tag) {
                          <mat-chip>{{ tag }}</mat-chip>
                        }
                      </mat-chip-set>
                    </td>
                  </ng-container>

                  <tr mat-header-row *matHeaderRowDef="channelColumns; sticky: true"></tr>
                  <tr mat-row *matRowDef="let row; columns: channelColumns;"
                      [class.trigger-row]="row.IsEventTrigger"></tr>
                </table>
              </div>

              <div class="action-buttons">
                <button mat-raised-button color="primary" (click)="saveChSettings()">
                  <mat-icon>save</mat-icon>
                  Save Channel Settings
                </button>
              </div>
            </div>
          </mat-tab>

          <!-- Time Calibration Tab -->
          <mat-tab label="Time Calibration (timeSettings)">
            <div class="tab-content">
              @if (config.time_settings) {
                <mat-card>
                  <mat-card-header>
                    <mat-card-title>Reference Channel</mat-card-title>
                  </mat-card-header>
                  <mat-card-content>
                    <p>Module: {{ config.time_settings.ref_module }}, Channel: {{ config.time_settings.ref_channel }}</p>
                    <p>Total offsets: {{ Object.keys(config.time_settings.offsets).length }}</p>
                  </mat-card-content>
                </mat-card>

                <mat-card class="offsets-card">
                  <mat-card-header>
                    <mat-card-title>Time Offsets (ns)</mat-card-title>
                  </mat-card-header>
                  <mat-card-content>
                    <table class="offsets-table">
                      <tr>
                        <th>Module_Channel</th>
                        <th>Offset (ns)</th>
                      </tr>
                      @for (entry of timeOffsetEntries(); track entry.key) {
                        <tr>
                          <td>{{ entry.key }}</td>
                          <td>{{ entry.value | number:'1.2-2' }}</td>
                        </tr>
                      }
                    </table>
                  </mat-card-content>
                </mat-card>
              } @else {
                <mat-card>
                  <mat-card-content>
                    <p>No time calibration data. Run time-calib command to generate offsets.</p>
                    <code>./event_builder time-calib -i input.root -o time_calib.json --ref-module 9 --ref-channel 2</code>
                  </mat-card-content>
                </mat-card>
              }
            </div>
          </mat-tab>

          <!-- L2 Settings Tab -->
          <mat-tab label="L2 Filters (L2Settings)">
            <div class="tab-content">
              @if (config.l2_settings && config.l2_settings.length > 0) {
                @for (setting of config.l2_settings; track $index) {
                  <mat-expansion-panel>
                    <mat-expansion-panel-header>
                      <mat-panel-title>
                        <mat-icon>{{ getL2Icon(setting) }}</mat-icon>
                        {{ setting.Name }}
                      </mat-panel-title>
                      <mat-panel-description>
                        {{ setting.Type }}
                      </mat-panel-description>
                    </mat-expansion-panel-header>

                    @switch (setting.Type) {
                      @case ('Counter') {
                        <div class="l2-details">
                          <p><strong>Tags:</strong> {{ setting.Tags.join(', ') }}</p>
                        </div>
                      }
                      @case ('Flag') {
                        <div class="l2-details">
                          <p><strong>Monitor:</strong> {{ setting.Monitor }}</p>
                          <p><strong>Condition:</strong> {{ setting.Operator }} {{ setting.Value }}</p>
                        </div>
                      }
                      @case ('Accept') {
                        <div class="l2-details">
                          <p><strong>Monitor:</strong> {{ setting.Monitor.join(', ') }}</p>
                          <p><strong>Operator:</strong> {{ setting.Operator }}</p>
                        </div>
                      }
                    }
                  </mat-expansion-panel>
                }
              } @else {
                <mat-card>
                  <mat-card-content>
                    <p>No L2 filter settings defined.</p>
                    <p>L2 filters are optional post-processing rules for event selection.</p>
                  </mat-card-content>
                </mat-card>
              }
            </div>
          </mat-tab>
        </mat-tab-group>
      }
    </div>
  `,
  styles: `
    .event-builder-settings {
      padding: 16px;
      position: relative;
    }

    .loading-overlay {
      position: absolute;
      top: 0;
      left: 0;
      right: 0;
      bottom: 0;
      background: rgba(255, 255, 255, 0.8);
      display: flex;
      align-items: center;
      justify-content: center;
      z-index: 100;
    }

    .selection-card {
      margin-bottom: 16px;
    }

    .selection-row {
      display: flex;
      gap: 16px;
      align-items: center;
      flex-wrap: wrap;
    }

    .selection-row mat-form-field {
      min-width: 200px;
    }

    .error-banner {
      display: flex;
      align-items: center;
      gap: 8px;
      color: #f44336;
      margin-top: 8px;
      padding: 8px;
      background: #ffebee;
      border-radius: 4px;
    }

    .info-card {
      margin-bottom: 16px;
    }

    .config-info {
      display: grid;
      grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
      gap: 16px;
    }

    .info-item {
      display: flex;
      flex-direction: column;
      gap: 4px;
    }

    .info-label {
      font-size: 12px;
      color: rgba(0, 0, 0, 0.6);
    }

    .info-value {
      font-size: 16px;
      font-weight: 500;
    }

    .tab-content {
      padding: 16px;
    }

    .channel-table-container {
      max-height: 500px;
      overflow: auto;
    }

    .channel-table {
      width: 100%;
    }

    .compact-field {
      width: 80px;
    }

    .compact-field ::ng-deep .mat-mdc-form-field-infix {
      padding: 4px 0;
      min-height: 32px;
    }

    .trigger-row {
      background-color: #e3f2fd;
    }

    .action-buttons {
      display: flex;
      gap: 8px;
      margin-top: 16px;
    }

    .offsets-card {
      margin-top: 16px;
    }

    .offsets-table {
      width: 100%;
      border-collapse: collapse;
    }

    .offsets-table th,
    .offsets-table td {
      padding: 8px;
      text-align: left;
      border-bottom: 1px solid #e0e0e0;
    }

    .offsets-table th {
      background-color: #f5f5f5;
      font-weight: 500;
    }

    .l2-details {
      padding: 16px;
    }

    .l2-details p {
      margin: 8px 0;
    }

    mat-expansion-panel {
      margin-bottom: 8px;
    }

    mat-expansion-panel-header mat-icon {
      margin-right: 8px;
    }
  `,
})
export class EventBuilderSettingsComponent implements OnInit {
  readonly service = inject(EventBuilderService);
  private readonly snackBar = inject(MatSnackBar);
  private readonly dialog = inject(MatDialog);

  selectedExp = '';
  selectedConfig = '';

  // Table columns
  channelColumns = ['id', 'module', 'channel', 'detectorType', 'isTrigger', 'threshold', 'hasAC', 'acModule', 'acChannel', 'tags'];

  // For Object.keys in template
  Object = Object;

  async ngOnInit(): Promise<void> {
    await this.service.loadExperiments();
  }

  async onExpChange(): Promise<void> {
    this.selectedConfig = '';
    this.service.currentConfig.set(null);
    if (this.selectedExp) {
      await this.service.loadConfigs(this.selectedExp);
    }
  }

  async onConfigChange(): Promise<void> {
    if (this.selectedExp && this.selectedConfig) {
      await this.service.loadConfig(this.selectedExp, this.selectedConfig);
    }
  }

  flattenedChannels(): ChSettings[] {
    const config = this.service.currentConfig();
    if (!config) return [];
    return config.ch_settings.flat();
  }

  countChannels(config: EventBuilderConfig): number {
    return config.ch_settings.flat().length;
  }

  countTriggers(config: EventBuilderConfig): number {
    return config.ch_settings.flat().filter(ch => ch.IsEventTrigger).length;
  }

  timeOffsetEntries(): { key: string; value: number }[] {
    const config = this.service.currentConfig();
    if (!config?.time_settings?.offsets) return [];
    return Object.entries(config.time_settings.offsets)
      .map(([key, value]) => ({ key, value }))
      .sort((a, b) => a.key.localeCompare(b.key));
  }

  getL2Icon(setting: L2Setting): string {
    switch (setting.Type) {
      case 'Counter': return 'calculate';
      case 'Flag': return 'flag';
      case 'Accept': return 'check_circle';
      default: return 'help';
    }
  }

  async saveChSettings(): Promise<void> {
    const config = this.service.currentConfig();
    if (!config) return;

    // Rebuild 2D array from flattened
    const flattened = this.flattenedChannels();
    const chSettings: ChSettings[][] = [];
    const moduleMap = new Map<number, ChSettings[]>();

    for (const ch of flattened) {
      if (!moduleMap.has(ch.Module)) {
        moduleMap.set(ch.Module, []);
      }
      moduleMap.get(ch.Module)!.push(ch);
    }

    const sortedModules = Array.from(moduleMap.keys()).sort((a, b) => a - b);
    for (const mod of sortedModules) {
      const channels = moduleMap.get(mod)!;
      channels.sort((a, b) => a.Channel - b.Channel);
      chSettings.push(channels);
    }

    const result = await this.service.updateChSettings(config.exp_name, config.name, chSettings);
    if (result) {
      this.snackBar.open('Channel settings saved', 'OK', { duration: 2000 });
    } else {
      this.snackBar.open('Failed to save channel settings', 'OK', { duration: 3000 });
    }
  }

  async createNewConfig(): Promise<void> {
    // Simple prompt for now (can be replaced with dialog)
    const name = prompt('Configuration name:', 'default');
    if (!name) return;

    const config = {
      name,
      exp_name: this.selectedExp,
      ch_settings: this.service.createEmptyChSettings(16, 16),
      coincidence_window_ns: 500,
      slice_duration_ns: 10_000_000,
    };

    const result = await this.service.saveConfig(config);
    if (result) {
      this.selectedConfig = name;
      this.snackBar.open('Configuration created', 'OK', { duration: 2000 });
    }
  }
}
