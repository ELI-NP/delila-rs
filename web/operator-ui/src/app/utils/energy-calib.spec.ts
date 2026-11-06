import {
  ADC_NOT_CALIBRATED,
  axisName1D,
  axisName2D,
  calibrateHistogram1D,
  calibrateHistogram2D,
  calibStatusDetail,
  calibStatusText,
  dropStaleCalibState,
  fitTextLines,
  formatAxisValue,
  kevCellView,
  lookupCalib,
  migrateXAxisLabel,
  tabShowsKev,
  toKev,
  unitOfAxisName,
} from './energy-calib';
import { FittingService } from '../services/fitting.service';
import {
  CalibrationResponse,
  Histogram1D,
  Histogram2D,
  ViewCell,
  ViewCellFitResult,
  ViewTab,
} from '../models/histogram.types';

function calibration(
  entries: { module: number; channel: number; kev_per_adc: number; offset_kev: number }[],
): CalibrationResponse {
  return {
    configured: true,
    path: 'config/energy_calib_test.txt',
    status: 'ok',
    unit: 'keV',
    loaded_at: '2026-11-06T10:00:00Z',
    entries,
    errors: [],
  };
}

function hist1d(): Histogram1D {
  return {
    module_id: 0,
    channel_id: 1,
    config: { num_bins: 1000, min_value: 0, max_value: 1000 },
    bins: new Array(1000).fill(0),
    total_counts: 0,
    overflow: 0,
    underflow: 0,
  };
}

function hist2d(): Histogram2D {
  return {
    module_id: 0,
    channel_id: 1,
    x_config: { num_bins: 4, min_value: 0, max_value: 400 },
    y_config: { num_bins: 2, min_value: 0, max_value: 1 },
    bins: new Array(8).fill(0),
    total_counts: 0,
    overflow: 0,
  };
}

const FIT: ViewCellFitResult = {
  center: 1332.456,
  centerError: 0.031,
  sigma: 1.5,
  sigmaError: 0.02,
  fwhm: 3.532,
  netArea: 1000,
  netAreaError: 30,
  chi2: 10,
  ndf: 10,
  leftLine: { slope: 0, intercept: 0 },
  rightLine: { slope: 0, intercept: 0 },
  bgLine: { slope: 0, intercept: 0 },
  amplitude: 100,
};

function cell(sourceId: number, channelId: number, extra: Partial<ViewCell> = {}): ViewCell {
  return {
    sourceId,
    channelId,
    xRange: 'auto',
    yRange: 'auto',
    isLocked: false,
    isEmpty: false,
    ...extra,
  };
}

function tab(cells: ViewCell[], extra: Partial<ViewTab> = {}): ViewTab {
  return {
    id: 't',
    name: 't',
    gridRows: 1,
    gridCols: cells.length,
    xAxisLabel: 'keV',
    histogramType: 'energy',
    cells,
    ...extra,
  };
}

describe('energy calibration (TODO 83)', () => {
  const CAL = { kevPerAdc: 0.5, offsetKev: -10 };

  describe('coefficients', () => {
    it('maps ADC to keV', () => {
      expect(toKev(100, CAL)).toBe(40);
    });

    it('finds a channel, and null for one not listed', () => {
      const c = calibration([{ module: 5, channel: 0, kev_per_adc: 0.3, offset_kev: -12 }]);
      expect(lookupCalib(c, 5, 0)).toEqual({ kevPerAdc: 0.3, offsetKev: -12 });
      expect(lookupCalib(c, 5, 1)).toBeNull();
      expect(lookupCalib(null, 5, 0)).toBeNull();
    });

    it('treats an impossible slope as no coefficient', () => {
      spyOn(console, 'error');
      const c = calibration([{ module: 0, channel: 0, kev_per_adc: 0, offset_kev: 1 }]);
      expect(lookupCalib(c, 0, 0)).toBeNull();
      expect(console.error).toHaveBeenCalled();
    });
  });

  describe('histograms', () => {
    it('maps the 1D range and leaves the counts alone', () => {
      const h = hist1d();
      const k = calibrateHistogram1D(h, CAL);
      expect(k.config).toEqual({ num_bins: 1000, min_value: -10, max_value: 490 });
      expect(k.bins).toBe(h.bins);
      expect(h.config.min_value).toBe(0);
    });

    it('returns the same object without coefficients', () => {
      const h = hist1d();
      expect(calibrateHistogram1D(h, null)).toBe(h);
      const h2 = hist2d();
      expect(calibrateHistogram2D(h2, null, null)).toBe(h2);
    });

    it('maps only the 2D axes that have coefficients', () => {
      const h = hist2d();
      const x = calibrateHistogram2D(h, CAL, null);
      expect(x.x_config).toEqual({ num_bins: 4, min_value: -10, max_value: 190 });
      expect(x.y_config).toEqual(h.y_config);
      expect(x.bins).toBe(h.bins);
      const y = calibrateHistogram2D(h, null, CAL);
      expect(y.x_config).toEqual(h.x_config);
      expect(y.y_config).toEqual({ num_bins: 2, min_value: -10, max_value: -9.5 });
    });

    it('a Gaussian fit on the mapped histogram comes out in keV', () => {
      const center = 500;
      const sigma = 20;
      const h = hist1d();
      h.bins = h.bins.map((_, i) =>
        Math.round(1000 * Math.exp(-0.5 * Math.pow((i + 0.5 - center) / sigma, 2))),
      );
      const k = calibrateHistogram1D(h, CAL);
      const width = (k.config.max_value - k.config.min_value) / k.config.num_bins;
      const fit = new FittingService().fitGaussian({
        bins: k.bins,
        binWidth: width,
        minValue: k.config.min_value,
        fitRangeMin: toKev(400, CAL),
        fitRangeMax: toKev(600, CAL),
      });
      expect(fit).not.toBeNull();
      expect(fit!.center).toBeCloseTo(toKev(center, CAL), 0);
      expect(fit!.sigma).toBeCloseTo(sigma * CAL.kevPerAdc, 0);
    });
  });

  describe('which views show keV', () => {
    it('an energy view does when asked', () => {
      expect(tabShowsKev({ xAxisLabel: 'keV', histogramType: 'energy' })).toBeTrue();
      expect(tabShowsKev({ xAxisLabel: 'keV' })).toBeTrue();
      expect(tabShowsKev({ xAxisLabel: 'Channel', histogramType: 'energy' })).toBeFalse();
    });

    it('PSD and UserInfo never do', () => {
      expect(tabShowsKev({ xAxisLabel: 'keV', histogramType: 'psd' })).toBeFalse();
      expect(tabShowsKev({ xAxisLabel: 'keV', histogramType: 'user_info0' })).toBeFalse();
    });

    it('a 2D view does only with an energy axis, and energy_short does not count', () => {
      const d = { xAxisLabel: 'keV', histogramType: '2d' } as const;
      expect(tabShowsKev({ ...d, xAxis: 'energy', yAxis: 'psd' })).toBeTrue();
      expect(tabShowsKev({ ...d, xAxis: 'psd', yAxis: 'energy' })).toBeTrue();
      expect(tabShowsKev({ ...d, xAxis: 'energy_short', yAxis: 'psd' })).toBeFalse();
    });
  });

  describe('axis names: never say keV over ADC values', () => {
    it('1D', () => {
      expect(axisName1D(true, true)).toBe('keV');
      expect(axisName1D(true, false)).toBe(ADC_NOT_CALIBRATED);
      expect(axisName1D(true, false)).not.toContain('keV');
      expect(axisName1D(false, false)).toBe('Channel');
    });

    it('2D: only the energy axis', () => {
      expect(axisName2D('Energy', 'energy', true, true)).toBe('Energy [keV]');
      const uncal = axisName2D('Energy', 'energy', true, false);
      expect(uncal).toContain('ADC');
      expect(uncal).not.toContain('keV');
      expect(axisName2D('PSD', 'psd', true, true)).toBe('PSD');
      expect(axisName2D('Energy Short', 'energy_short', true, true)).toBe('Energy Short');
      expect(axisName2D('Energy', 'energy', false, false)).toBe('Energy');
    });

    it('the unit follows the name', () => {
      expect(unitOfAxisName('keV')).toBe('keV');
      expect(unitOfAxisName(ADC_NOT_CALIBRATED)).toBe('ADC');
      expect(unitOfAxisName('Channel')).toBe('');
    });

    it('the fit text carries the unit', () => {
      const kev = fitTextLines(FIT, 'keV').join('\n');
      expect(kev).toContain('Center: 1332.46 ± 0.03 keV');
      expect(kev).toContain('FWHM: 3.53 keV');
      const adc = fitTextLines(FIT, 'ADC').join('\n');
      expect(adc).toContain('Center: 1332.5 ± 0.0 ADC');
      expect(adc).not.toContain('keV');
      expect(fitTextLines(FIT, '').join('\n')).toContain('FWHM: 3.5\n');
    });
  });

  describe('value labels', () => {
    it('are whole above one unit per bin', () => {
      expect(formatAxisValue(1332.4, 64)).toBe('1332');
      expect(formatAxisValue(100.5, 1)).toBe('101');
    });

    it('keep neighbouring bins apart below it', () => {
      expect(formatAxisValue(10.25, 0.5)).toBe('10.3');
      expect(formatAxisValue(10.25, 0.25)).not.toBe(formatAxisValue(10.5, 0.25));
      expect(formatAxisValue(0.123, 0.007)).toBe('0.123');
    });
  });

  describe('per-cell range in a keV view', () => {
    const cfg = { num_bins: 1000, min_value: -10, max_value: 490 };

    it('defaults to the whole of that cell', () => {
      expect(kevCellView(undefined, cfg, true)).toEqual({ min: -10, max: 490, bins: 1000 });
      expect(kevCellView({ bins: 100 }, cfg, true)).toEqual({ min: -10, max: 490, bins: 100 });
    });

    it('a chosen range is in keV', () => {
      expect(kevCellView({ min: 100, max: 200, bins: 50 }, cfg, true)).toEqual({
        min: 100,
        max: 200,
        bins: 50,
      });
    });

    it('a cell still in ADC ignores a keV range', () => {
      const adc = { num_bins: 1000, min_value: 0, max_value: 1000 };
      expect(kevCellView({ min: 100, max: 200, bins: 50 }, adc, false)).toEqual({
        min: 0,
        max: 1000,
        bins: 50,
      });
    });
  });

  describe('status line', () => {
    it('says what is in force', () => {
      const ok = calibration([{ module: 0, channel: 1, kev_per_adc: 0.5, offset_kev: 0 }]);
      expect(calibStatusText(ok)).toContain('1 channel (keV)');
      expect(calibStatusText({ ...ok, status: 'none', entries: [], configured: false, path: null }))
        .toContain('none configured');
    });

    it('shows the first problem and counts the rest', () => {
      const bad = { ...calibration([]), status: 'stale' as const, errors: ['a:1: x', 'a:2: y'] };
      expect(calibStatusText(bad)).toContain('STALE');
      expect(calibStatusText(bad)).toContain('a:1: x (+1 more)');
      expect(calibStatusText({ ...bad, status: 'error' })).toContain('display in ADC');
      expect(calibStatusDetail(bad)).toContain('a:2: y');
      expect(calibStatusDetail(bad)).toContain('config/energy_calib_test.txt');
    });
  });

  describe('saved layouts', () => {
    it('MeV becomes keV, the rest stay', () => {
      expect(migrateXAxisLabel('MeV')).toBe('keV');
      expect(migrateXAxisLabel('keV')).toBe('keV');
      expect(migrateXAxisLabel('Channel')).toBe('Channel');
      expect(migrateXAxisLabel(undefined)).toBe('Channel');
    });
  });

  describe('when the coefficients change', () => {
    const A = calibration([{ module: 0, channel: 1, kev_per_adc: 0.5, offset_kev: -10 }]);
    const B = calibration([{ module: 0, channel: 1, kev_per_adc: 0.6, offset_kev: -10 }]);
    const NONE = calibration([]);
    const range = { min: 100, max: 200 };

    it('reading the same numbers again changes nothing', () => {
      const t = [tab([cell(0, 1, { fitResult: FIT, fitRange: range })])];
      const again = { ...A, loaded_at: 'later', entries: [...A.entries] };
      const r = dropStaleCalibState(t, A, again);
      expect(r.changed).toBeFalse();
      expect(r.tabs).toBe(t);
    });

    it('new numbers drop the fit and keep the range, which is in keV', () => {
      const t = [tab([cell(0, 1, { fitResult: FIT, fitRange: range, xRange: range, isLocked: true })])];
      const r = dropStaleCalibState(t, A, B);
      expect(r.changed).toBeTrue();
      expect(r.fitsCleared).toBe(1);
      const c = r.tabs[0].cells[0];
      expect(c.fitResult).toBeUndefined();
      expect(c.fitRange).toBeUndefined();
      expect(c.xRange).toEqual(range);
      expect(c.isLocked).toBeTrue();
    });

    it('a cell changing unit also loses its range, which meant the other unit', () => {
      const t = [tab([cell(0, 1, { fitResult: FIT, xRange: range, isLocked: true })])];
      const r = dropStaleCalibState(t, A, NONE);
      const c = r.tabs[0].cells[0];
      expect(c.fitResult).toBeUndefined();
      expect(c.xRange).toBe('auto');
      expect(c.isLocked).toBeFalse();
    });

    it('other channels and ADC views are left alone', () => {
      const other = cell(0, 2, { fitResult: FIT, xRange: range, isLocked: true });
      const adcCell = cell(0, 1, { fitResult: FIT, xRange: range, isLocked: true });
      const t = [tab([other]), tab([adcCell], { xAxisLabel: 'Channel' })];
      const r = dropStaleCalibState(t, A, B);
      expect(r.changed).toBeFalse();
      expect(r.tabs).toBe(t);
    });
  });
});
