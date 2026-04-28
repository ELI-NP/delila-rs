import { Component, Input, Output, EventEmitter, inject, signal } from '@angular/core';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatInputModule } from '@angular/material/input';
import { MatSelectModule } from '@angular/material/select';
import { MatButtonModule } from '@angular/material/button';
import { MatIconModule } from '@angular/material/icon';
import { MatCardModule } from '@angular/material/card';
import { MatDividerModule } from '@angular/material/divider';
import { FormsModule } from '@angular/forms';
import { HistogramService } from '../../services/histogram.service';
import { DigitizerService } from '../../services/digitizer.service';
import {
  SetupConfig,
  SetupCell,
  ChannelSummary,
  XAxisLabel,
  HistogramType,
  createDefaultSetupCell,
} from '../../models/histogram.types';

@Component({
  selector: 'app-setup-tab',
  standalone: true,
  imports: [
    MatFormFieldModule,
    MatInputModule,
    MatSelectModule,
    MatButtonModule,
    MatIconModule,
    MatCardModule,
    MatDividerModule,
    FormsModule,
  ],
  template: `
    <div class="setup-container">
      <!-- Quick Create section -->
      <div class="quick-create">
        <span class="quick-create-label">Quick Create</span>
        <mat-form-field appearance="outline" class="digitizer-select">
          <mat-label>Digitizer</mat-label>
          <mat-select
            [value]="selectedDigitizerId()"
            (selectionChange)="selectedDigitizerId.set($event.value)"
          >
            @for (d of digitizerService.digitizers(); track d.digitizer_id) {
              <mat-option [value]="d.digitizer_id">
                #{{ d.digitizer_id }} {{ d.name }} ({{ d.num_channels }}ch)
              </mat-option>
            }
          </mat-select>
        </mat-form-field>
        <button
          mat-raised-button
          color="accent"
          (click)="onQuickCreate()"
          [disabled]="selectedDigitizerId() === null"
        >
          <mat-icon>auto_awesome</mat-icon>
          Quick Create
        </button>
      </div>

      <mat-divider></mat-divider>

      <!-- Manual setup -->
      <div class="setup-header">
        <mat-form-field appearance="outline" class="name-input">
          <mat-label>View Name</mat-label>
          <input
            matInput
            [value]="config.name"
            (input)="onNameChange($event)"
            placeholder="e.g., CRIB, LaBr3, Silicon"
          />
        </mat-form-field>

        <mat-form-field appearance="outline" class="size-input">
          <mat-label>Rows</mat-label>
          <input
            matInput
            type="number"
            [value]="config.gridRows"
            (change)="onRowsChange($event)"
            min="1"
            max="5"
          />
        </mat-form-field>

        <span class="size-separator">x</span>

        <mat-form-field appearance="outline" class="size-input">
          <mat-label>Cols</mat-label>
          <input
            matInput
            type="number"
            [value]="config.gridCols"
            (change)="onColsChange($event)"
            min="1"
            max="5"
          />
        </mat-form-field>

        <mat-form-field appearance="outline" class="histogram-type-select">
          <mat-label>Type</mat-label>
          <mat-select
            [value]="config.histogramType"
            (selectionChange)="onHistogramTypeChange($event.value)"
          >
            <mat-option value="energy">Energy</mat-option>
            <mat-option value="psd">PSD</mat-option>
            <mat-option value="psd2d">PSD 2D</mat-option>
            <mat-option value="amax2d">AMax 2D (E × UI[0])</mat-option>
          </mat-select>
        </mat-form-field>

        @if (config.histogramType !== 'psd2d' && config.histogramType !== 'amax2d') {
          <mat-form-field appearance="outline" class="axis-label-select">
            <mat-label>X-Axis</mat-label>
            <mat-select
              [value]="config.xAxisLabel"
              (selectionChange)="onXAxisLabelChange($event.value)"
            >
              <mat-option value="Channel">Channel</mat-option>
              <mat-option value="keV">keV</mat-option>
              <mat-option value="MeV">MeV</mat-option>
            </mat-select>
          </mat-form-field>
        }

        <button
          mat-raised-button
          color="primary"
          (click)="onCreateView()"
          [disabled]="!canCreateView()"
        >
          <mat-icon>add</mat-icon>
          Create View
        </button>
      </div>

      <div
        class="setup-grid"
        [style.grid-template-rows]="'repeat(' + config.gridRows + ', 1fr)'"
        [style.grid-template-columns]="'repeat(' + config.gridCols + ', 1fr)'"
      >
        @for (cell of config.cells; track $index; let i = $index) {
          @if (i < config.gridRows * config.gridCols) {
            <mat-card class="setup-cell" [class.filled]="cell.sourceId !== null">
              <mat-form-field appearance="outline" class="channel-select">
                <mat-label>Channel</mat-label>
                <mat-select
                  [value]="cellKey(cell)"
                  (selectionChange)="onCellChange(i, $event.value)"
                >
                  <mat-option value="">-- Empty --</mat-option>
                  @for (channel of availableChannels(); track channelKey(channel)) {
                    <mat-option [value]="channelKey(channel)">
                      Src {{ channel.module_id }} / Ch {{ channel.channel_id }}
                    </mat-option>
                  }
                </mat-select>
              </mat-form-field>
            </mat-card>
          }
        }
      </div>

      <div class="setup-footer">
        <span class="hint">
          Select channels for each cell, then click "Create View" to generate a histogram view.
        </span>
      </div>
    </div>
  `,
  styles: `
    .setup-container {
      display: flex;
      flex-direction: column;
      height: 100%;
      gap: 16px;
    }

    .quick-create {
      display: flex;
      align-items: center;
      gap: 12px;
    }

    .quick-create-label {
      font-weight: 500;
      font-size: 14px;
      color: #666;
      white-space: nowrap;
    }

    .digitizer-select {
      width: 280px;
    }

    .setup-header {
      display: flex;
      align-items: center;
      gap: 12px;
      flex-wrap: wrap;
    }

    .name-input {
      width: 200px;
    }

    .size-input {
      width: 80px;
    }

    .histogram-type-select {
      width: 120px;
    }

    .axis-label-select {
      width: 120px;
    }

    .size-separator {
      font-size: 18px;
      color: #666;
    }

    .setup-grid {
      display: grid;
      gap: 12px;
      flex: 1;
      min-height: 0;
    }

    .setup-cell {
      display: flex;
      align-items: center;
      justify-content: center;
      padding: 16px;
      background-color: #fafafa;
      border: 2px dashed #ddd;
    }

    .setup-cell.filled {
      background-color: #e3f2fd;
      border: 2px solid #2196f3;
    }

    .channel-select {
      width: 100%;
      max-width: 200px;
    }

    .setup-footer {
      padding: 8px 0;
    }

    .hint {
      font-size: 12px;
      color: #666;
    }

    ::ng-deep .setup-cell .mat-mdc-form-field-infix {
      padding: 8px 0 !important;
      min-height: 36px;
    }
  `,
})
export class SetupTabComponent {
  @Input() config!: SetupConfig;
  @Output() configChange = new EventEmitter<SetupConfig>();
  @Output() createView = new EventEmitter<SetupConfig>();
  @Output() quickCreate = new EventEmitter<SetupConfig[]>();

  private readonly histogramService = inject(HistogramService);
  readonly digitizerService = inject(DigitizerService);

  readonly availableChannels = this.histogramService.channelList;
  readonly selectedDigitizerId = signal<number | null>(null);

  channelKey(channel: ChannelSummary): string {
    return `${channel.module_id}:${channel.channel_id}`;
  }

  cellKey(cell: SetupCell): string {
    if (cell.sourceId === null || cell.channelId === null) {
      return '';
    }
    return `${cell.sourceId}:${cell.channelId}`;
  }

  onNameChange(event: Event): void {
    const input = event.target as HTMLInputElement;
    this.emitConfigChange({ name: input.value });
  }

  onRowsChange(event: Event): void {
    const input = event.target as HTMLInputElement;
    const rows = Math.min(5, Math.max(1, parseInt(input.value, 10) || 1));
    this.updateGridSize(rows, this.config.gridCols);
  }

  onColsChange(event: Event): void {
    const input = event.target as HTMLInputElement;
    const cols = Math.min(5, Math.max(1, parseInt(input.value, 10) || 1));
    this.updateGridSize(this.config.gridRows, cols);
  }

  onHistogramTypeChange(value: HistogramType): void {
    this.emitConfigChange({ histogramType: value });
  }

  onXAxisLabelChange(value: XAxisLabel): void {
    this.emitConfigChange({ xAxisLabel: value });
  }

  private updateGridSize(rows: number, cols: number): void {
    const newCellCount = rows * cols;
    const currentCells = [...this.config.cells];

    while (currentCells.length < newCellCount) {
      currentCells.push(createDefaultSetupCell());
    }

    this.configChange.emit({
      ...this.config,
      gridRows: rows,
      gridCols: cols,
      cells: currentCells.slice(0, newCellCount),
    });
  }

  onCellChange(index: number, value: string): void {
    const cells = [...this.config.cells];

    if (!value) {
      cells[index] = { sourceId: null, channelId: null };
    } else {
      const [sourceId, channelId] = value.split(':').map(Number);
      cells[index] = { sourceId, channelId };
    }

    this.emitConfigChange({ cells });
  }

  canCreateView(): boolean {
    // At least one cell must have a channel assigned
    return this.config.cells.some((cell) => cell.sourceId !== null);
  }

  onCreateView(): void {
    if (this.canCreateView()) {
      this.createView.emit(this.config);
    }
  }

  onQuickCreate(): void {
    const id = this.selectedDigitizerId();
    if (id === null) return;

    const digitizer = this.digitizerService.digitizers().find((d) => d.digitizer_id === id);
    if (!digitizer) return;

    const numCh = digitizer.num_channels;
    const configs: SetupConfig[] = [];
    const chunkSize = numCh <= 8 ? numCh : 16;

    for (let offset = 0; offset < numCh; offset += chunkSize) {
      const end = Math.min(offset + chunkSize, numCh);
      const count = end - offset;
      const rows = count <= 8 ? 2 : 4;
      const cols = 4;
      const totalCells = rows * cols;

      const cells: SetupCell[] = [];
      for (let i = 0; i < totalCells; i++) {
        const ch = offset + i;
        if (ch < end) {
          cells.push({ sourceId: id, channelId: ch });
        } else {
          cells.push({ sourceId: null, channelId: null });
        }
      }

      const name =
        numCh <= chunkSize
          ? `${digitizer.name}`
          : `${digitizer.name} Ch${offset}-${end - 1}`;

      configs.push({
        name,
        gridRows: rows,
        gridCols: cols,
        xAxisLabel: 'Channel',
        histogramType: this.config.histogramType,
        cells,
      });
    }

    this.quickCreate.emit(configs);
  }

  private emitConfigChange(changes: Partial<SetupConfig>): void {
    this.configChange.emit({ ...this.config, ...changes });
  }
}
