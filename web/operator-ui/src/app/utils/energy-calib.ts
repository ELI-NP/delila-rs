/**
 * Energy calibration for the display (TODO 83): `E_keV = kev_per_adc * ADC + offset_kev`.
 *
 * The coefficients are worked out outside delila-rs; the Monitor only serves them
 * (`GET /api/calibration`) and the histograms stay in ADC. A straight line maps equal
 * bins to equal bins, so a calibrated histogram is the same counts with a mapped
 * range: everything downstream (rebin, charts, Gaussian fit, zoom) works unchanged
 * and its numbers come out in keV.
 *
 * In a keV view a cell's `xRange`, `fitRange` and `fitResult`, and the tab's
 * `xView.min/max`, are in keV. A cell without coefficients stays in ADC and says so:
 * a label must never promise keV over ADC values.
 */

import {
  AxisSource,
  AxisView,
  CalibrationResponse,
  HistogramType,
  ViewCell,
  ViewCellFitResult,
  ViewTab,
  XAxisLabel,
} from '../models/histogram.types';
import { Histogram1DLike, Histogram2DLike } from './rebin';

export interface LinearCalib {
  kevPerAdc: number;
  offsetKev: number;
}

export const ADC_NOT_CALIBRATED = 'ADC (not calibrated)';

export function toKev(adc: number, c: LinearCalib): number {
  return c.kevPerAdc * adc + c.offsetKev;
}

export function lookupCalib(
  calibration: CalibrationResponse | null | undefined,
  module: number,
  channel: number,
): LinearCalib | null {
  const e = calibration?.entries.find((x) => x.module === module && x.channel === channel);
  if (!e) return null;
  // The Monitor refuses these, so this would be a bug; never draw a mirrored axis.
  if (!(e.kev_per_adc > 0) || !Number.isFinite(e.kev_per_adc) || !Number.isFinite(e.offset_kev)) {
    console.error(`Energy calibration: unusable line for m${module}c${channel}`, e);
    return null;
  }
  return { kevPerAdc: e.kev_per_adc, offsetKev: e.offset_kev };
}

function mapConfig<C extends { min_value: number; max_value: number }>(cfg: C, c: LinearCalib): C {
  return { ...cfg, min_value: toKev(cfg.min_value, c), max_value: toKev(cfg.max_value, c) };
}

export function calibrateHistogram1D<T extends Histogram1DLike>(h: T, c: LinearCalib | null): T {
  return c ? { ...h, config: mapConfig(h.config, c) } : h;
}

export function calibrateHistogram2D<T extends Histogram2DLike>(
  h: T,
  x: LinearCalib | null,
  y: LinearCalib | null,
): T {
  if (!x && !y) return h;
  return {
    ...h,
    x_config: x ? mapConfig(h.x_config, x) : h.x_config,
    y_config: y ? mapConfig(h.y_config, y) : h.y_config,
  };
}

/**
 * Does this view show keV? Only for the `energy` axis: `energy_short` is a different
 * gate with its own scale, and PSD and UserInfo are not energies.
 */
export function tabShowsKev(tab: {
  xAxisLabel?: XAxisLabel;
  histogramType?: HistogramType;
  xAxis?: AxisSource;
  yAxis?: AxisSource;
}): boolean {
  if (tab.xAxisLabel !== 'keV') return false;
  const type = tab.histogramType ?? 'energy';
  if (type === 'energy') return true;
  if (type === '2d') return (tab.xAxis ?? 'energy') === 'energy' || tab.yAxis === 'energy';
  return false;
}

export function axisName1D(showsKev: boolean, calibrated: boolean): string {
  if (!showsKev) return 'Channel';
  return calibrated ? 'keV' : ADC_NOT_CALIBRATED;
}

export function axisName2D(
  base: string,
  axis: AxisSource,
  showsKev: boolean,
  calibrated: boolean,
): string {
  if (!showsKev || axis !== 'energy') return base;
  return calibrated ? `${base} [keV]` : `${base} [ADC, not calibrated]`;
}

export function unitOfAxisName(name: string): string {
  if (name === 'keV') return 'keV';
  if (name === ADC_NOT_CALIBRATED) return 'ADC';
  return '';
}

/** A bin's value as text, with enough digits to tell neighbouring bins apart. */
export function formatAxisValue(value: number, step: number): string {
  if (!(step > 0) || step >= 1) return Math.round(value).toString();
  return value.toFixed(Math.min(3, Math.ceil(-Math.log10(step))));
}

/** A number as text; '---' if there is none (a layout saved as JSON holds NaN as null). */
function fitNumber(value: number | null | undefined, digits: number): string {
  return value != null && Number.isFinite(value) ? value.toFixed(digits) : '---';
}

export function fitTextLines(fit: ViewCellFitResult, unit: string): string[] {
  const u = unit ? ` ${unit}` : '';
  const chi2Ndf = fit.ndf > 0 ? (fit.chi2 / fit.ndf).toFixed(2) : '---';
  const resolution = fit.center > 0 ? `${((100 * fit.fwhm) / fit.center).toFixed(2)} %` : '---';
  return [
    `Center: ${fitNumber(fit.center, 2)} ± ${fitNumber(fit.centerError, 2)}${u}`,
    `Sigma: ${fitNumber(fit.sigma, 2)} ± ${fitNumber(fit.sigmaError, 2)}${u}`,
    `FWHM: ${fitNumber(fit.fwhm, 2)}${u}`,
    `Resolution: ${resolution}`,
    `Area: ${fitNumber(fit.netArea, 0)} ± ${fitNumber(fit.netAreaError, 0)}`,
    `χ²/ndf: ${chi2Ndf}`,
  ];
}

/**
 * The range one cell of a keV view shows on its energy axis: by default all of that
 * cell, since gains differ. A chosen range is in keV, so a cell in ADC ignores it.
 */
export function kevCellView(
  view: AxisView | undefined,
  config: { num_bins: number; min_value: number; max_value: number },
  calibrated: boolean,
): AxisView {
  return {
    min: calibrated ? (view?.min ?? config.min_value) : config.min_value,
    max: calibrated ? (view?.max ?? config.max_value) : config.max_value,
    bins: view?.bins ?? config.num_bins,
  };
}

/** One line for the Monitor page. */
export function calibStatusText(cal: CalibrationResponse): string {
  const first = cal.errors.length > 0 ? cal.errors[0] : '';
  const more = cal.errors.length > 1 ? ` (+${cal.errors.length - 1} more)` : '';
  switch (cal.status) {
    case 'none':
      return 'Energy calibration: none configured';
    case 'ok': {
      const at = cal.loaded_at ? `, loaded ${new Date(cal.loaded_at).toLocaleTimeString()}` : '';
      const n = cal.entries.length;
      return `Energy calibration: ${n} channel${n === 1 ? '' : 's'} (keV)${at}`;
    }
    case 'stale':
      return `Energy calibration STALE, previous table kept: ${first}${more}`;
    case 'error':
      return `Energy calibration ERROR, display in ADC: ${first}${more}`;
  }
}

/** The whole story, for the tooltip and the error message. */
export function calibStatusDetail(cal: CalibrationResponse): string {
  return [calibStatusText(cal), cal.path ?? '', ...cal.errors].filter((s) => s !== '').join('\n');
}

/** Saved layouts may still say 'MeV', which was only ever a label. */
export function migrateXAxisLabel(label: string | undefined): XAxisLabel {
  return label === 'keV' || label === 'MeV' ? 'keV' : 'Channel';
}

function sameLine(a: LinearCalib | null, b: LinearCalib | null): boolean {
  return a?.kevPerAdc === b?.kevPerAdc && a?.offsetKev === b?.offsetKev;
}

/**
 * New coefficients invalidate what was worked out with the old ones: in keV views a
 * changed channel loses its fit, and if it moved between ADC and keV also its range.
 * Re-reading the same numbers (every run start does) changes nothing.
 */
export function dropStaleCalibState(
  tabs: ViewTab[],
  prev: CalibrationResponse | null,
  next: CalibrationResponse | null,
): { tabs: ViewTab[]; changed: boolean; fitsCleared: number } {
  let changed = false;
  let fitsCleared = 0;
  const updated = tabs.map((tab) => {
    if (!tabShowsKev(tab)) return tab;
    let tabChanged = false;
    const cells = tab.cells.map((cell): ViewCell => {
      if (cell.isEmpty) return cell;
      const before = lookupCalib(prev, cell.sourceId, cell.channelId);
      const after = lookupCalib(next, cell.sourceId, cell.channelId);
      if (sameLine(before, after)) return cell;
      tabChanged = true;
      if (cell.fitResult) fitsCleared++;
      const out: ViewCell = { ...cell, fitResult: undefined, fitRange: undefined };
      if ((before === null) !== (after === null)) {
        out.xRange = 'auto';
        out.yRange = 'auto';
        out.isLocked = false;
      }
      return out;
    });
    if (!tabChanged) return tab;
    changed = true;
    return { ...tab, cells };
  });
  return { tabs: changed ? updated : tabs, changed, fitsCleared };
}
