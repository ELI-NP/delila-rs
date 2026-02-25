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

  readonly chartOptions = signal<EChartsCoreOption>(this.buildInitialOptions());
  readonly mergeOptions = signal<EChartsCoreOption>({});

  ngOnChanges(changes: SimpleChanges): void {
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
          return `Energy: ${x}<br>PSD: ${y.toFixed(3)}<br>Counts: ${count}`;
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
        name: 'Energy (ADC)',
        nameLocation: 'middle',
        nameGap: 30,
        splitArea: { show: false },
        axisLabel: { show: true, interval: 'auto' },
      },
      yAxis: {
        type: 'category',
        name: 'PSD',
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

    // Build X and Y category labels (downsampled for display)
    const xLabels: string[] = [];
    for (let i = 0; i < xBins; i++) {
      const v = xMin + (i + 0.5) * xStep;
      xLabels.push(Math.round(v).toString());
    }
    const yLabels: string[] = [];
    for (let i = 0; i < yBins; i++) {
      const v = yMin + (i + 0.5) * yStep;
      yLabels.push(v.toFixed(2));
    }

    // Convert flat bins to [x, y, value] triplets, skip zeros for sparse transfer
    const data: [number, number, number][] = [];
    let maxCount = 0;
    for (let y = 0; y < yBins; y++) {
      for (let x = 0; x < xBins; x++) {
        const count = hist.bins[y * xBins + x];
        if (count > 0) {
          data.push([x, y, count]);
          if (count > maxCount) maxCount = count;
        }
      }
    }

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
        axisLabel: {
          interval: Math.max(0, Math.floor(xBins / 8) - 1),
          rotate: 0,
        },
      },
      yAxis: {
        data: yLabels,
        axisLabel: {
          interval: Math.max(0, Math.floor(yBins / 8) - 1),
        },
      },
      visualMap: {
        min: vmMin,
        max: vmMax || 1,
        formatter: useLog
          ? (value: number) => Math.round(Math.pow(10, value)).toString()
          : undefined,
      },
      tooltip: {
        formatter: (params: { value?: number[] }) => {
          if (!params.value) return '';
          const [xi, yi, rawVal] = params.value;
          const count = useLog ? Math.round(Math.pow(10, rawVal)) : rawVal;
          const energy = xLabels[xi] ?? xi;
          const psd = yLabels[yi] ?? yi;
          return `Energy: ${energy}<br>PSD: ${psd}<br>Counts: ${count}`;
        },
      },
      series: [{
        data: displayData,
      }],
    });
  }
}
