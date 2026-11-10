import {
  Component,
  Input,
  Output,
  EventEmitter,
  OnInit,
  OnDestroy,
  inject,
  signal,
  computed,
  ViewChildren,
  QueryList,
} from '@angular/core';
import { interval, Subject, takeUntil, switchMap, forkJoin, of, catchError } from 'rxjs';
import { MatButtonModule } from '@angular/material/button';
import { MatIconModule } from '@angular/material/icon';
import { HistogramChartComponent, RangeChangeEvent } from '../histogram-chart/histogram-chart.component';
import { HeatmapChartComponent } from '../heatmap-chart/heatmap-chart.component';
import { RebinPickerComponent } from '../rebin-picker/rebin-picker.component';
import {
  REBIN_FACTORS,
  allowedRebinFactors,
  nativeBinsInView,
  rebinFactorOf,
  withRebin,
} from '../../utils/rebin';
import { HistogramService } from '../../services/histogram.service';
import { DigitizerService } from '../../services/digitizer.service';
import {
  AxisSource,
  AxisView,
  ViewTab,
  Histogram1D,
  Histogram2D,
  AXIS_SOURCE_LABEL,
  DEFAULT_AXIS_VIEW,
  defaultYAxisSource,
} from '../../models/histogram.types';
import {
  LinearCalib,
  axisName1D,
  axisName2D,
  calibrateHistogram1D,
  calibrateHistogram2D,
  kevCellView,
  lookupCalib,
  tabShowsKev,
} from '../../utils/energy-calib';

@Component({
  selector: 'app-view-tab',
  standalone: true,
  imports: [
    HistogramChartComponent,
    HeatmapChartComponent,
    RebinPickerComponent,
    MatButtonModule,
    MatIconModule,
  ],
  template: `
    <div class="view-container">
      <!-- Toolbar -->
      <div class="view-toolbar">
        @if (!is2dHistType(histType())) {
          <button
            mat-stroked-button
            (click)="onApplyRangeToAll()"
            [disabled]="!hasLockedCell()"
            title="Apply the range from the last zoomed cell to all cells"
          >
            <mat-icon>content_copy</mat-icon>
            Apply Range to All
          </button>
          <button
            mat-stroked-button
            (click)="onResetAllRanges()"
            [disabled]="!hasLockedCell()"
          >
            <mat-icon>restart_alt</mat-icon>
            Reset All
          </button>
        }
        <button
          mat-stroked-button
          (click)="onToggleLogScale()"
          [class.active]="isAllLogScale()"
          title="Toggle logarithmic Y-axis scale"
        >
          <mat-icon>{{ isAllLogScale() ? 'linear_scale' : 'show_chart' }}</mat-icon>
          {{ isAllLogScale() ? 'Linear' : 'Log' }}
        </button>
        <button
          mat-stroked-button
          (click)="onSaveImage()"
          [disabled]="isSaving()"
          title="Save grid as PNG image"
        >
          <mat-icon>save_alt</mat-icon>
          {{ isSaving() ? 'Saving...' : 'Save Image' }}
        </button>
        <!-- Rebin x1/x2/x4/x8 (TODO 87): native bins per shown bin, relative to
             the shown range. The charts rebin client-side. -->
        <div class="binning-controls">
          <app-rebin-picker
            [label]="is2dHistType(histType()) ? 'X rebin:' : 'Rebin:'"
            [value]="xRebinFactor()"
            [allowed]="xAllowedRebin()"
            [nativeBins]="xNativeBins()"
            (valueChange)="onXRebin($event)"
          />
          @if (is2dHistType(histType())) {
            <app-rebin-picker
              label="Y rebin:"
              [value]="yRebinFactor()"
              [allowed]="yAllowedRebin()"
              [nativeBins]="yNativeBins()"
              (valueChange)="onYRebin($event)"
            />
          }
        </div>
        @if (showsKev()) {
          <span
            class="calib-count"
            title="Cells in keV; the others have no coefficients and are shown in ADC"
          >
            keV: {{ calibratedCount() }} / {{ usedCellCount() }} cells
          </span>
        }
        @if (!is2dHistType(histType())) {
          <span class="toolbar-hint">
            Drag to select range, Ctrl+Scroll for X-axis zoom
          </span>
        }
      </div>

      <!-- Grid -->
      <div
        class="view-grid"
        [style.grid-template-rows]="'repeat(' + tab.gridRows + ', 1fr)'"
        [style.grid-template-columns]="'repeat(' + tab.gridCols + ', 1fr)'"
      >
        @for (cell of tab.cells; track $index; let i = $index) {
          @if (i < tab.gridRows * tab.gridCols && !cell.isEmpty) {
            <div
              class="view-cell"
              [class.locked]="cell.isLocked"
              (dblclick)="onCellDoubleClick(i)"
            >
              <div class="cell-label">
                Src{{ cell.sourceId }}/Ch{{ cell.channelId }}
                @if (cell.isLocked) {
                  <span class="lock-icon">🔒</span>
                }
                @if (showsKev() && cellCalibs()[i] === null) {
                  <span class="adc-badge" title="No keV coefficients for this channel">ADC</span>
                }
              </div>
              <div class="chart-wrapper">
                @if (!is2dHistType(histType())) {
                  <app-histogram-chart
                    #chartRef
                    [histogram]="displayHistograms()[i] ?? null"
                    [xRange]="cell.xRange"
                    [yRange]="cell.yRange"
                    [showDataZoom]="true"
                    [logScale]="cell.logScale ?? false"
                    [xAxisLabel]="xAxisName1d(i)"
                    [xView]="cellXViews()[i]"
                    (rangeChange)="onRangeChange(i, $event)"
                  ></app-histogram-chart>
                } @else {
                  <app-heatmap-chart
                    #chartRef
                    [histogram]="displayHistograms2d()[i] ?? null"
                    [logScale]="cell.logScale ?? true"
                    [xAxisLabel]="xAxisName2d(i)"
                    [yAxisLabel]="yAxisName2d(i)"
                    [xView]="cellXViews()[i]"
                    [yView]="cellYViews()[i]"
                  ></app-heatmap-chart>
                }
              </div>
              <button class="expand-button" (click)="onExpandClick(i)" title="Expand">
                <span class="expand-icon">&#x26F6;</span>
              </button>
            </div>
          } @else if (i < tab.gridRows * tab.gridCols) {
            <div class="view-cell empty"></div>
          }
        }
      </div>
    </div>
  `,
  styles: `
    :host {
      display: block;
      height: 100%;
    }

    .view-container {
      display: flex;
      flex-direction: column;
      height: 100%;
      gap: 8px;
    }

    .view-toolbar {
      display: flex;
      align-items: center;
      gap: 12px;
      flex-shrink: 0;
    }

    .toolbar-hint {
      font-size: 12px;
      color: #666;
      margin-left: auto;
    }

    .binning-controls {
      display: flex;
      align-items: center;
      gap: 12px;
      padding: 0 8px;
      border-left: 1px solid #ddd;
      border-right: 1px solid #ddd;
    }

    .view-grid {
      display: grid;
      gap: 4px;
      flex: 1;
      min-height: 0;
    }

    .view-cell {
      position: relative;
      background: white;
      border: 1px solid #e0e0e0;
      border-radius: 4px;
      overflow: hidden;
      display: flex;
      flex-direction: column;
    }

    .view-cell.empty {
      background: #fafafa;
    }

    .view-cell.locked {
      border-color: #1976d2;
      border-width: 2px;
    }

    .cell-label {
      position: absolute;
      top: 2px;
      left: 4px;
      font-size: 10px;
      color: #666;
      z-index: 1;
      background: rgba(255, 255, 255, 0.8);
      padding: 0 4px;
      border-radius: 2px;
    }

    .lock-icon {
      font-size: 8px;
      margin-left: 2px;
    }

    .adc-badge {
      margin-left: 4px;
      padding: 0 4px;
      border-radius: 2px;
      background: #ffb300;
      color: #000;
      font-weight: 700;
    }

    .calib-count {
      font-size: 12px;
      color: #444;
      white-space: nowrap;
    }

    .chart-wrapper {
      flex: 1;
      min-height: 0;
    }

    .expand-button {
      position: absolute;
      top: 2px;
      right: 2px;
      width: 24px;
      height: 24px;
      border: none;
      background: rgba(255, 255, 255, 0.8);
      border-radius: 4px;
      cursor: pointer;
      display: flex;
      align-items: center;
      justify-content: center;
      opacity: 0;
      transition: opacity 0.2s;
      z-index: 1;
    }

    .view-cell:hover .expand-button {
      opacity: 1;
    }

    .expand-button:hover {
      background: #e0e0e0;
    }

    .expand-icon {
      font-size: 14px;
    }

    button.active {
      background-color: #1976d2;
      color: white;
    }
  `,
})
export class ViewTabComponent implements OnInit, OnDestroy {
  /**
   * `tab` is exposed as a normal property (preserving the existing
   * `this.tab.xxx` call-sites) but backed by a signal so `computed()`s
   * that read it (xView, xRebinFactor, etc.) invalidate when the parent
   * passes a new tab object. Without this the slider's displayed value
   * stays frozen at its initial reading because the cached computed
   * never sees the parent's update.
   */
  private readonly _tab = signal<ViewTab>(undefined as unknown as ViewTab);
  @Input({ required: true }) set tab(value: ViewTab) {
    this._tab.set(value);
  }
  get tab(): ViewTab {
    return this._tab();
  }
  @Output() tabChange = new EventEmitter<ViewTab>();
  @Output() cellExpand = new EventEmitter<number>();

  // 1D and 2D cells both carry #chartRef so "Save Image" works for either view.
  @ViewChildren('chartRef') chartRefs!: QueryList<HistogramChartComponent | HeatmapChartComponent>;

  private readonly histogramService = inject(HistogramService);
  private readonly digitizerService = inject(DigitizerService);
  private readonly destroy$ = new Subject<void>();
  private readonly refreshInterval = 1000;

  readonly histograms = signal<(Histogram1D | null)[]>([]);
  readonly histograms2d = signal<(Histogram2D | null)[]>([]);
  readonly isSaving = signal(false);
  readonly histType = computed(() => this.tab.histogramType ?? 'energy');
  /** Effective X / Y axis for 2D plots. Falls back to (energy, FW default) so
   *  old layout files written before Phase 2 still render something sensible —
   *  UserInfo[0] on all-AMax systems, psd otherwise (issue #25). */
  readonly xAxis = computed<AxisSource>(() => this.tab.xAxis ?? 'energy');
  readonly yAxis = computed<AxisSource>(
    () => this.tab.yAxis ?? defaultYAxisSource(this.digitizerService.allAMax()),
  );

  /** This view is in keV wherever a cell has coefficients (TODO 83). */
  readonly showsKev = computed(() => tabShowsKev(this.tab));
  private readonly xIsEnergy = computed(() =>
    this.is2dHistType(this.histType()) ? this.xAxis() === 'energy' : this.histType() === 'energy',
  );
  private readonly yIsEnergy = computed(
    () => this.is2dHistType(this.histType()) && this.yAxis() === 'energy',
  );

  /** Coefficients per cell; null = shown in ADC (always, in an ADC view). */
  readonly cellCalibs = computed<(LinearCalib | null)[]>(() => {
    const calibration = this.histogramService.calibration();
    const kev = this.showsKev();
    return this.tab.cells.map((c) =>
      kev && !c.isEmpty ? lookupCalib(calibration, c.sourceId, c.channelId) : null,
    );
  });
  readonly calibratedCount = computed(() => this.cellCalibs().filter((c) => c !== null).length);
  readonly usedCellCount = computed(() => this.tab.cells.filter((c) => !c.isEmpty).length);

  /** What the charts and fits see: the same counts, on a mapped range. */
  readonly displayHistograms = computed(() => {
    const calibs = this.cellCalibs();
    return this.histograms().map((h, i) => (h ? calibrateHistogram1D(h, calibs[i] ?? null) : null));
  });
  readonly displayHistograms2d = computed(() => {
    const calibs = this.cellCalibs();
    const x = this.xIsEnergy();
    const y = this.yIsEnergy();
    return this.histograms2d().map((h, i) => {
      if (!h) return null;
      const c = calibs[i] ?? null;
      return calibrateHistogram2D(h, x ? c : null, y ? c : null);
    });
  });

  /** Per-cell view: gains differ, so in keV each cell's range is its own. */
  readonly cellXViews = computed<AxisView[]>(() => {
    const base = this.xView();
    if (!this.showsKev() || !this.xIsEnergy()) return this.tab.cells.map(() => base);
    const calibs = this.cellCalibs();
    const two = this.is2dHistType(this.histType());
    return this.tab.cells.map((_, i) => {
      const cfg = two ? this.displayHistograms2d()[i]?.x_config : this.displayHistograms()[i]?.config;
      return cfg ? kevCellView(this.tab.xView, cfg, calibs[i] !== null) : base;
    });
  });
  readonly cellYViews = computed<AxisView[]>(() => {
    const base = this.yView();
    if (!this.showsKev() || !this.yIsEnergy()) return this.tab.cells.map(() => base);
    const calibs = this.cellCalibs();
    return this.tab.cells.map((_, i) => {
      const cfg = this.displayHistograms2d()[i]?.y_config;
      return cfg ? kevCellView(this.tab.yView, cfg, calibs[i] !== null) : base;
    });
  });

  xAxisName1d(i: number): string {
    return axisName1D(this.showsKev(), this.cellCalibs()[i] !== null);
  }
  xAxisName2d(i: number): string {
    const x = this.xAxis();
    return axisName2D(this.axisLabel(x), x, this.showsKev(), this.cellCalibs()[i] !== null);
  }
  yAxisName2d(i: number): string {
    const y = this.yAxis();
    return axisName2D(this.axisLabel(y), y, this.showsKev(), this.cellCalibs()[i] !== null);
  }

  /**
   * Live view config — the chart components apply this client-side rebin.
   * Falls back to the matching `DEFAULT_AXIS_VIEW` for the current axis when
   * the tab doesn't carry an explicit override (legacy layouts and freshly
   * created views start there). The user's slider edits in Setup / Expand
   * patch `tab.xView` / `yView` directly and these computeds re-fire.
   */
  readonly xView = computed<AxisView>(() =>
    this.resolveView(
      this.tab.xView,
      this.histType() === '2d' ? this.xAxis() : this.energy1dAxis(),
    ),
  );
  readonly yView = computed<AxisView>(() =>
    this.resolveView(this.tab.yView, this.yAxis()),
  );

  /**
   * For 1D plots the axis is implied by the `histogramType` (energy / psd /
   * user_info0..3 → matching `AxisSource` for default lookup).
   */
  private energy1dAxis(): AxisSource {
    const t = this.histType();
    if (t === 'psd') return 'psd';
    if (t === 'user_info0') return 'user_info0';
    if (t === 'user_info1') return 'user_info1';
    if (t === 'user_info2') return 'user_info2';
    if (t === 'user_info3') return 'user_info3';
    return 'energy';
  }

  private resolveView(view: AxisView | undefined, axis: AxisSource): AxisView {
    // The defaults are in ADC. In keV the energy axis has none: see cellXViews.
    const def = this.showsKev() && axis === 'energy' ? undefined : DEFAULT_AXIS_VIEW[axis];
    return {
      min: view?.min ?? def?.min,
      max: view?.max ?? def?.max,
      // TODO 87: the rebin factor (unset = x1); `bins` only from old layouts.
      bins: view?.bins,
      rebin: view?.rebin,
    };
  }

  /** Native bins of the shown X range, from the first cell with data (TODO 87). */
  readonly xNativeBins = computed<number | null>(() => {
    const views = this.cellXViews();
    if (this.is2dHistType(this.histType())) {
      const hs = this.displayHistograms2d();
      const i = hs.findIndex((h) => h !== null);
      return i < 0 ? null : nativeBinsInView(hs[i]!.x_config, views[i]);
    }
    const hs = this.displayHistograms();
    const i = hs.findIndex((h) => h !== null);
    return i < 0 ? null : nativeBinsInView(hs[i]!.config, views[i]);
  });
  readonly yNativeBins = computed<number | null>(() => {
    if (!this.is2dHistType(this.histType())) return null;
    const views = this.cellYViews();
    const hs = this.displayHistograms2d();
    const i = hs.findIndex((h) => h !== null);
    return i < 0 ? null : nativeBinsInView(hs[i]!.y_config, views[i]);
  });

  /** Factors the shown range splits evenly (all four until data arrives). */
  readonly xAllowedRebin = computed(() => allowedOrAll(this.xNativeBins()));
  readonly yAllowedRebin = computed(() => allowedOrAll(this.yNativeBins()));

  /** The factor in use: the tab's `rebin`, or its pre-TODO 87 `bins`. */
  readonly xRebinFactor = computed(() => factorOrSaved(this.tab.xView, this.xNativeBins()));
  readonly yRebinFactor = computed(() => factorOrSaved(this.tab.yView, this.yNativeBins()));

  onXRebin(factor: number): void {
    this.tabChange.emit({ ...this.tab, xView: withRebin(this.tab.xView, factor) });
  }

  onYRebin(factor: number): void {
    this.tabChange.emit({ ...this.tab, yView: withRebin(this.tab.yView, factor) });
  }

  /** Heatmap-rendered histogram type (single 2D variant under the new model). */
  is2dHistType(t: string): boolean {
    return t === '2d';
  }

  /** Pretty label for an `AxisSource` used in chart titles + tooltips. */
  axisLabel(src: AxisSource): string {
    return AXIS_SOURCE_LABEL[src];
  }

  ngOnInit(): void {
    const cellCount = this.tab.cells.length;
    this.histograms.set(new Array(cellCount).fill(null));
    this.histograms2d.set(new Array(cellCount).fill(null));

    if (this.is2dHistType(this.histType())) {
      // Poll 2D histograms
      interval(this.refreshInterval)
        .pipe(
          takeUntil(this.destroy$),
          switchMap(() => this.fetchAll2d())
        )
        .subscribe((results) => {
          this.histograms2d.set(results);
        });
      this.fetchAll2d().subscribe((results) => {
        this.histograms2d.set(results);
      });
    } else {
      // Poll 1D histograms (energy or psd)
      interval(this.refreshInterval)
        .pipe(
          takeUntil(this.destroy$),
          switchMap(() => this.fetchAll1d())
        )
        .subscribe((results) => {
          this.histograms.set(results);
        });
      this.fetchAll1d().subscribe((results) => {
        this.histograms.set(results);
      });
    }
  }

  ngOnDestroy(): void {
    this.destroy$.next();
    this.destroy$.complete();
  }

  private fetchAll1d() {
    const t = this.histType();
    const requests = this.tab.cells.map((cell) => {
      if (cell.isEmpty) return of(null);
      if (t === 'psd') {
        return this.histogramService.fetchPsdHistogram(cell.sourceId, cell.channelId);
      }
      const slot = this.userInfoSlot(t);
      if (slot !== null) {
        return this.histogramService.fetchUserInfoHistogram(cell.sourceId, cell.channelId, slot);
      }
      return this.histogramService.fetchAndCacheHistogram(cell.sourceId, cell.channelId);
    });
    return forkJoin(requests);
  }

  /** Map a `'user_infoN'` HistogramType to its 0..3 slot, otherwise null. */
  private userInfoSlot(t: string): 0 | 1 | 2 | 3 | null {
    switch (t) {
      case 'user_info0': return 0;
      case 'user_info1': return 1;
      case 'user_info2': return 2;
      case 'user_info3': return 3;
      default: return null;
    }
  }

  private fetchAll2d() {
    const x = this.xAxis();
    const y = this.yAxis();
    const requests = this.tab.cells.map((cell) => {
      if (cell.isEmpty) return of(null);
      return this.histogramService.fetchHistogram2d(cell.sourceId, cell.channelId, x, y).pipe(
        catchError(() => of(null))
      );
    });
    return forkJoin(requests);
  }

  onRangeChange(index: number, event: RangeChangeEvent): void {
    const cells = [...this.tab.cells];
    cells[index] = {
      ...cells[index],
      xRange: event.xRange,
      yRange: event.yRange,
      isLocked: true,
    };
    this.tabChange.emit({ ...this.tab, cells, lastModifiedCellIndex: index });
  }

  onCellDoubleClick(index: number): void {
    this.cellExpand.emit(index);
  }

  onExpandClick(index: number): void {
    this.cellExpand.emit(index);
  }

  hasLockedCell(): boolean {
    return this.tab.cells.some((cell) => cell.isLocked);
  }

  isAllLogScale(): boolean {
    const nonEmptyCells = this.tab.cells.filter((cell) => !cell.isEmpty);
    return nonEmptyCells.length > 0 && nonEmptyCells.every((cell) => cell.logScale);
  }

  onToggleLogScale(): void {
    const newLogScale = !this.isAllLogScale();
    const cells = this.tab.cells.map((cell) => ({
      ...cell,
      logScale: cell.isEmpty ? false : newLogScale,
    }));
    this.tabChange.emit({ ...this.tab, cells });
  }

  onApplyRangeToAll(): void {
    // Use the last zoomed cell as reference, fallback to first locked cell
    const refIndex = this.tab.lastModifiedCellIndex;
    const refCell = refIndex != null
      ? this.tab.cells[refIndex]
      : this.tab.cells.find((cell) => cell.isLocked);
    if (!refCell?.isLocked) return;

    // Apply its range to all non-empty cells in this tab that use the same unit:
    // a keV range means nothing to a cell still in ADC.
    const calibs = this.cellCalibs();
    const refCalibrated = calibs[this.tab.cells.indexOf(refCell)] !== null;
    const cells = this.tab.cells.map((cell, i) => {
      if (cell.isEmpty || (calibs[i] !== null) !== refCalibrated) return cell;
      return {
        ...cell,
        xRange: refCell.xRange,
        yRange: refCell.yRange,
        isLocked: true,
      };
    });

    this.tabChange.emit({ ...this.tab, cells });
  }

  onResetAllRanges(): void {
    // Reset all cells to auto range
    const cells = this.tab.cells.map((cell) => ({
      ...cell,
      xRange: 'auto' as const,
      yRange: 'auto' as const,
      isLocked: false,
    }));

    this.tabChange.emit({ ...this.tab, cells, lastModifiedCellIndex: undefined });
  }

  async onSaveImage(): Promise<void> {
    if (this.isSaving()) return;
    this.isSaving.set(true);

    try {
      const charts = this.chartRefs.toArray();
      if (charts.length === 0) {
        this.isSaving.set(false);
        return;
      }

      // Get chart images as data URLs
      const chartImages: HTMLImageElement[] = [];
      for (const chart of charts) {
        const dataUrl = chart.getDataURL(2);
        if (dataUrl) {
          const img = await this.loadImage(dataUrl);
          chartImages.push(img);
        }
      }

      if (chartImages.length === 0) {
        this.isSaving.set(false);
        return;
      }

      // Calculate canvas size
      const cellWidth = chartImages[0].width;
      const cellHeight = chartImages[0].height;
      const cols = this.tab.gridCols;
      const rows = this.tab.gridRows;
      const padding = 4;
      const labelHeight = 24;

      const canvasWidth = cols * cellWidth + (cols + 1) * padding;
      const canvasHeight = rows * (cellHeight + labelHeight) + (rows + 1) * padding;

      // Create canvas
      const canvas = document.createElement('canvas');
      canvas.width = canvasWidth;
      canvas.height = canvasHeight;
      const ctx = canvas.getContext('2d');
      if (!ctx) {
        this.isSaving.set(false);
        return;
      }

      // Fill background
      ctx.fillStyle = '#f5f5f5';
      ctx.fillRect(0, 0, canvasWidth, canvasHeight);

      // Draw each chart with label
      let chartIndex = 0;
      for (let row = 0; row < rows; row++) {
        for (let col = 0; col < cols; col++) {
          const cellIndex = row * cols + col;
          const cell = this.tab.cells[cellIndex];

          if (!cell || cell.isEmpty) continue;

          const x = padding + col * (cellWidth + padding);
          const y = padding + row * (cellHeight + labelHeight + padding);

          // Draw label background
          ctx.fillStyle = '#ffffff';
          ctx.fillRect(x, y, cellWidth, labelHeight);

          // Draw label text
          ctx.fillStyle = '#333333';
          ctx.font = '14px sans-serif';
          ctx.textBaseline = 'middle';
          const label = `Src${cell.sourceId}/Ch${cell.channelId}`;
          ctx.fillText(label, x + 8, y + labelHeight / 2);

          // Draw chart image
          if (chartIndex < chartImages.length) {
            ctx.drawImage(chartImages[chartIndex], x, y + labelHeight);
            chartIndex++;
          }
        }
      }

      // Download as PNG
      const link = document.createElement('a');
      const timestamp = new Date().toISOString().replace(/[:.]/g, '-').slice(0, 19);
      link.download = `${this.tab.name}_${timestamp}.png`;
      link.href = canvas.toDataURL('image/png');
      link.click();
    } finally {
      this.isSaving.set(false);
    }
  }

  private loadImage(src: string): Promise<HTMLImageElement> {
    return new Promise((resolve, reject) => {
      const img = new Image();
      img.onload = () => resolve(img);
      img.onerror = reject;
      img.src = src;
    });
  }
}

/** Factors a range of `n` native bins splits evenly; all four while unknown. */
function allowedOrAll(n: number | null): readonly number[] {
  return n === null ? REBIN_FACTORS : allowedRebinFactors(n);
}

/** The factor a saved view means (its `rebin`, or old `bins`, TODO 87). */
function factorOrSaved(view: AxisView | undefined, n: number | null): number {
  return n === null ? (view?.rebin ?? 1) : rebinFactorOf(view, n);
}
