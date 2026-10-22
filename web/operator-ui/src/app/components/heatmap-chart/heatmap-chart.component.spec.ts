import type { ECharts } from 'echarts/core';
import { HeatmapChartComponent } from './heatmap-chart.component';

// View tab "Save Image" collects every #chartRef and calls getDataURL(); a 2D
// tab used to have no exportable chart, so the button silently did nothing.
describe('HeatmapChartComponent image export', () => {
  it('returns null before the chart is initialised', () => {
    const chart = new HeatmapChartComponent();
    expect(chart.getDataURL()).toBeNull();
  });

  it('exports the ECharts instance as PNG on a white background', () => {
    const chart = new HeatmapChartComponent();
    const instance = jasmine.createSpyObj<ECharts>('ECharts', ['getDataURL']);
    instance.getDataURL.and.returnValue('data:image/png;base64,AAAA');

    chart.onChartInit(instance);

    expect(chart.getDataURL(2)).toBe('data:image/png;base64,AAAA');
    expect(instance.getDataURL).toHaveBeenCalledWith({
      type: 'png',
      pixelRatio: 2,
      backgroundColor: '#fff',
    });
  });
});
