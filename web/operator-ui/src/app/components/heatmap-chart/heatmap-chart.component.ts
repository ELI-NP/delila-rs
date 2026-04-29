import {
  Component,
  Input,
  OnChanges,
  SimpleChanges,
  signal,
} from '@angular/core';
import { NgxEchartsDirective } from 'ngx-echarts';
import type { EChartsCoreOption } from 'echarts/core';
import { Histogram2D } from '../../models/histogram.types';

@Component({
  selector: 'app-heatmap-chart',
  standalone: true,
  imports: [NgxEchartsDirective],
  template: `
    <div
      echarts
      [options]="chartOptions()"
      [merge]="mergeOptions()"
      class="heatmap-chart"
    ></div>
  `,
  styles: `
    :host {
      display: block;
      width: 100%;
      height: 100%;
    }

    .heatmap-chart {
      width: 100%;
      height: 100%;
      min-height: 200px;
    }
  `,
})
export class HeatmapChartComponent implements OnChanges {
  @Input() histogram: Histogram2D | null = null;
  @Input() logScale = true;
  @Input() xAxisLabel = 'Energy (ADC)';
  @Input() yAxisLabel = 'PSD';

  readonly chartOptions = signal<EChartsCoreOption>(this.buildInitialOptions());
  readonly mergeOptions = signal<EChartsCoreOption>({});

  ngOnChanges(changes: SimpleChanges): void {
    if (changes['xAxisLabel'] || changes['yAxisLabel']) {
      this.chartOptions.set(this.buildInitialOptions());
    }
    if (changes['histogram'] || changes['logScale']) {
      this.updateChart();
    }
  }

  private buildInitialOptions(): EChartsCoreOption {
    return {
      animation: false,
      tooltip: {
        trigger: 'item',
        formatter: (params: { value?: number[] }) => {
          if (!params.value) return '';
          const [x, y, count] = params.value;
          return `${this.xAxisLabel}: ${x}<br>${this.yAxisLabel}: ${y.toFixed(3)}<br>Counts: ${count}`;
        },
      },
      toolbox: {
        feature: { restore: {} },
        right: 90,
        top: 0,
      },
      dataZoom: [
        { type: 'inside', xAxisIndex: 0 },
        { type: 'inside', yAxisIndex: 0 },
        { type: 'slider', xAxisIndex: 0, bottom: 5, height: 15 },
        { type: 'slider', yAxisIndex: 0, left: 5, width: 15 },
      ],
      grid: {
        top: 30,
        right: 80,
        bottom: 60,
        left: 60,
      },
      xAxis: {
        type: 'category',
        name: this.xAxisLabel,
        nameLocation: 'middle',
        nameGap: 30,
        splitArea: { show: false },
        axisLabel: { show: true, interval: 'auto' },
      },
      yAxis: {
        type: 'category',
        name: this.yAxisLabel,
        nameLocation: 'middle',
        nameGap: 40,
        splitArea: { show: false },
        axisLabel: { show: true, interval: 'auto' },
      },
      visualMap: {
        min: 1,
        max: 100,
        calculable: true,
        orient: 'vertical',
        right: 5,
        top: 'center',
        inRange: {
          color: [
            '#440154', '#482878', '#3e4989', '#31688e',
            '#26828e', '#1f9e89', '#35b779', '#6ece58',
            '#b5de2b', '#fde725',
          ],
        },
        text: ['High', 'Low'],
      },
      series: [{
        type: 'heatmap',
        data: [],
        emphasis: {
          itemStyle: { shadowBlur: 5, shadowColor: 'rgba(0,0,0,0.5)' },
        },
      }],
    };
  }

  private updateChart(): void {
    const hist = this.histogram;
    if (!hist) {
      this.mergeOptions.set({ series: [{ data: [] }] });
      return;
    }

    const xBins = hist.x_config.num_bins;
    const yBins = hist.y_config.num_bins;
    const xMin = hist.x_config.min_value;
    const xMax = hist.x_config.max_value;
    const yMin = hist.y_config.min_value;
    const yMax = hist.y_config.max_value;
    const xStep = (xMax - xMin) / xBins;
    const yStep = (yMax - yMin) / yBins;

    // Build X and Y category labels — use the bin LEFT EDGE so the axis
    // starts at the configured min (e.g. 0) rather than the first bin center
    // (which would be half a bin-width higher and confusing for physics).
    const xLabels: string[] = [];
    for (let i = 0; i < xBins; i++) {
      const v = xMin + i * xStep;
      xLabels.push(Math.round(v).toString());
    }
    const yLabels: string[] = [];
    for (let i = 0; i < yBins; i++) {
      const v = yMin + i * yStep;
      yLabels.push(v.toFixed(2));
    }

    // Convert flat bins to [x, y, value] triplets, skip zeros for sparse
    // transfer. Cells in the underflow row (xi=0 or yi=0) are dropped from
    // both display and auto-zoom — they're typically "no-data sentinels"
    // (e.g. AMax FW reports `energy=0` for every event, which would otherwise
    // dominate the plot and defeat the auto-zoom). Real physics activity
    // beyond the first bin still shows up normally.
    const data: [number, number, number][] = [];
    let maxCount = 0;
    let xMinPop = xBins, xMaxPop = -1;
    let yMinPop = yBins, yMaxPop = -1;
    for (let y = 0; y < yBins; y++) {
      for (let x = 0; x < xBins; x++) {
        const count = hist.bins[y * xBins + x];
        if (count > 0 && x > 0 && y > 0) {
          data.push([x, y, count]);
          if (count > maxCount) maxCount = count;
          if (x < xMinPop) xMinPop = x;
          if (x > xMaxPop) xMaxPop = x;
          if (y < yMinPop) yMinPop = y;
          if (y > yMaxPop) yMaxPop = y;
        }
      }
    }

    // Auto-zoom to populated extent with padding (and a hard min of 8 bins
    // so a single-bin peak still has some breathing room). When no cells
    // outside the underflow row are populated we fall back to the full range.
    const padBins = (lo: number, hi: number, total: number) => {
      if (hi < lo) return [1, total - 1];
      const pad = Math.max(8, Math.round((hi - lo + 1) * 0.5));
      return [Math.max(1, lo - pad), Math.min(total - 1, hi + pad)];
    };
    const [xZoomLo, xZoomHi] = padBins(xMinPop, xMaxPop, xBins);
    const [yZoomLo, yZoomHi] = padBins(yMinPop, yMaxPop, yBins);

    // Log scale: visualMap uses log of counts
    const useLog = this.logScale && maxCount > 1;
    const displayData = useLog
      ? data.map(([x, y, c]) => [x, y, Math.log10(c)] as [number, number, number])
      : data;
    const vmMax = useLog ? Math.log10(maxCount) : maxCount;
    const vmMin = useLog ? 0 : 1;

    this.mergeOptions.set({
      xAxis: {
        data: xLabels,
        // Zoom to the populated bin range — see padBins() above.
        min: xZoomLo,
        max: xZoomHi,
        axisLabel: {
          interval: Math.max(0, Math.floor((xZoomHi - xZoomLo + 1) / 8) - 1),
          rotate: 0,
        },
      },
      yAxis: {
        data: yLabels,
        min: yZoomLo,
        max: yZoomHi,
        axisLabel: {
          interval: Math.max(0, Math.floor((yZoomHi - yZoomLo + 1) / 8) - 1),
        },
      },
      visualMap: {
        min: vmMin,
        max: vmMax || 1,
        // Always pin the selected range to the live [min, max] so the colour
        // bar matches the current data (the previous "preserve user drag"
        // behaviour caused the range to lag behind a growing histogram).
        // Drag-to-filter is therefore transient — it resets on the next
        // poll. Out-of-range cells are kept faintly visible so a mid-drag
        // moment doesn't blank the plot.
        range: [vmMin, vmMax || 1],
        outOfRange: {
          color: 'rgba(180,180,180,0.35)',
        },
        formatter: useLog
          ? (value: number) => Math.round(Math.pow(10, value)).toString()
          : undefined,
      },
      tooltip: {
        formatter: (params: { value?: number[] }) => {
          if (!params.value) return '';
          const [xi, yi, rawVal] = params.value;
          const count = useLog ? Math.round(Math.pow(10, rawVal)) : rawVal;
          const xVal = xLabels[xi] ?? xi;
          const yVal = yLabels[yi] ?? yi;
          return `${this.xAxisLabel}: ${xVal}<br>${this.yAxisLabel}: ${yVal}<br>Counts: ${count}`;
        },
      },
      series: [{
        data: displayData,
      }],
    });
  }
}
