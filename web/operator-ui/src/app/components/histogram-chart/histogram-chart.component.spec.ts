import { SimpleChange } from '@angular/core';
import { HistogramChartComponent } from './histogram-chart.component';
import { Histogram1D, ViewCellFitResult } from '../../models/histogram.types';
import { ADC_NOT_CALIBRATED } from '../../utils/energy-calib';

const FIT: ViewCellFitResult = {
  center: 500,
  centerError: 0.5,
  sigma: 20,
  sigmaError: 0.2,
  fwhm: 47,
  netArea: 1000,
  netAreaError: 30,
  chi2: 10,
  ndf: 10,
  leftLine: { slope: 0, intercept: 0 },
  rightLine: { slope: 0, intercept: 0 },
  bgLine: { slope: 0, intercept: 0 },
  amplitude: 100,
};

function hist(): Histogram1D {
  return {
    module_id: 0,
    channel_id: 1,
    config: { num_bins: 1000, min_value: 0, max_value: 1000 },
    bins: new Array(1000).fill(1),
    total_counts: 1000,
    overflow: 0,
    underflow: 0,
  };
}

/** Everything the chart would show: axis name, fit text, tooltip. */
function render(chart: HistogramChartComponent): string {
  chart.histogram = hist();
  chart.fitResult = FIT;
  chart.ngOnChanges({ histogram: new SimpleChange(null, chart.histogram, true) });
  const tooltip = (chart.chartOptions() as {
    tooltip: { formatter: (p: unknown) => string };
  }).tooltip.formatter([{ data: [100.5, 3], seriesIndex: 0 }]);
  const merged = chart.mergeOptions() as {
    xAxis: { name: string };
    graphic: { style: { text: string } }[];
  };
  return [merged.xAxis.name, merged.graphic[0].style.text, tooltip].join('\n');
}

describe('HistogramChartComponent units (TODO 83)', () => {
  it('says keV on a calibrated cell', () => {
    const chart = new HistogramChartComponent();
    chart.xAxisLabel = 'keV';
    const text = render(chart);
    expect(text).toContain('Center: 500.00 ± 0.50 keV');
    expect(text).toContain('keV: 101');
  });

  // The reason for this ticket's Phase 1: a label must never promise keV over ADC values.
  it('never says keV on a cell without coefficients', () => {
    const chart = new HistogramChartComponent();
    chart.xAxisLabel = ADC_NOT_CALIBRATED;
    const text = render(chart);
    expect(text).not.toContain('keV');
    expect(text).toContain(ADC_NOT_CALIBRATED);
    expect(text).toContain('ADC: 101');
  });

  it('is unchanged where nothing is asked for, as on the Tune Up page', () => {
    const chart = new HistogramChartComponent();
    const text = render(chart);
    expect(text).not.toContain('keV');
    expect(text).toContain('Channel: 101');
    expect(text).toContain('Center: 500.00 ± 0.50\n');
  });
});
