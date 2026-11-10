/**
 * Client-side histogram rebinning helpers.
 *
 * The Monitor backend always serves at its native resolution (1D Energy =
 * 65536, 2D = 512x512, etc). These functions take that raw payload and
 * project it onto a user-chosen `[min, max] × bins` window — chart
 * components call them on every render so the slider feels live.
 *
 * Conventions:
 * - Each native bin's *center* is what we test against the user window;
 *   server bins straddling the window edge are dropped (no fractional
 *   accumulation), keeping the math straightforward and the count totals
 *   exact for any window aligned with native edges.
 * - User bin index for native value `v` is `floor((v - umin) / userW)`.
 */

export interface AxisRange {
  min: number;
  max: number;
  bins: number;
}

/**
 * Rebin choices of the Monitor (TODO 87): 1, 2, 4 or 8 native bins per shown
 * bin. Before, a slider offered every factor up to 4096 in 100 px (about 40 per
 * pixel), factors that do not divide the bin count drew a comb, and the factor
 * ignored a narrowed range.
 */
export const REBIN_FACTORS: readonly number[] = [1, 2, 4, 8];

type AxisConfig = { num_bins: number; min_value: number; max_value: number };
type RebinView = { min?: number; max?: number; bins?: number; rebin?: number };

/** Native bins of `cfg` inside `[min, max]` (ADC or keV alike: a calibrated
 *  axis keeps its bin count). */
export function nativeBinsIn(cfg: AxisConfig, min: number, max: number): number {
  const width = (cfg.max_value - cfg.min_value) / cfg.num_bins;
  if (!(width > 0) || !(max > min)) return 1;
  return Math.max(1, Math.round((max - min) / width));
}

/** The offered factors that split `n` native bins evenly. */
export function allowedRebinFactors(n: number): number[] {
  return REBIN_FACTORS.filter((f) => f <= n && n % f === 0);
}

/**
 * The factor a view means for a range of `n` native bins: its `rebin`, or, for
 * a layout saved before TODO 87, the factor its `bins` gave. Either way the
 * largest offered factor not above it that splits `n` evenly (at least 1).
 */
export function rebinFactorOf(view: RebinView | null | undefined, n: number): number {
  const wanted = view?.rebin ?? (view?.bins ? n / view.bins : 1);
  const fits = allowedRebinFactors(n).filter((f) => f <= wanted + 1e-9);
  return fits.length > 0 ? fits[fits.length - 1] : 1;
}

/** The window a chart rebins to: the view's range (default: the whole
 *  histogram) in `native / factor` bins. */
export function resolveRebinView(view: RebinView | null | undefined, cfg: AxisConfig): AxisRange {
  const min = view?.min ?? cfg.min_value;
  const max = view?.max ?? cfg.max_value;
  const n = nativeBinsIn(cfg, min, max);
  return { min, max, bins: n / rebinFactorOf(view, n) };
}

export interface Histogram1DLike {
  bins: number[];
  config: { num_bins: number; min_value: number; max_value: number };
  total_counts: number;
  overflow: number;
  underflow: number;
}

export interface Histogram2DLike {
  bins: number[];
  x_config: { num_bins: number; min_value: number; max_value: number };
  y_config: { num_bins: number; min_value: number; max_value: number };
  total_counts: number;
  overflow: number;
}

/**
 * Project a 1D server histogram onto the user's `[min, max] × bins` window.
 * Returns a new histogram-shaped object the chart can render directly.
 */
export function rebin1d(src: Histogram1DLike, view: AxisRange): Histogram1DLike {
  const sBins = src.config.num_bins;
  const sMin = src.config.min_value;
  const sMax = src.config.max_value;
  const sW = (sMax - sMin) / sBins;

  const uMin = view.min;
  const uMax = view.max;
  const uBinsRaw = Math.max(1, Math.floor(view.bins));
  // Cap at native resolution — going higher would just create empty bins.
  const uBins = Math.min(uBinsRaw, sBins);
  const uW = (uMax - uMin) / uBins;

  const out = new Array(uBins).fill(0);
  let outUnder = src.underflow;
  let outOver = src.overflow;
  let outTotal = 0;

  for (let i = 0; i < sBins; i++) {
    const c = src.bins[i];
    if (c === 0) continue;
    const center = sMin + (i + 0.5) * sW;
    if (center < uMin) {
      outUnder += c;
      continue;
    }
    if (center >= uMax) {
      outOver += c;
      continue;
    }
    const j = Math.floor((center - uMin) / uW);
    if (j >= 0 && j < uBins) {
      out[j] += c;
      outTotal += c;
    }
  }

  return {
    bins: out,
    config: { num_bins: uBins, min_value: uMin, max_value: uMax },
    total_counts: outTotal,
    overflow: outOver,
    underflow: outUnder,
  };
}

/**
 * Project a 2D server histogram onto the user's window. Cells outside the
 * window land in `overflow` (no underflow concept for 2D in the server
 * type). Bin counts are capped at native resolution per axis.
 */
export function rebin2d(
  src: Histogram2DLike,
  xView: AxisRange,
  yView: AxisRange,
): Histogram2DLike {
  const sxBins = src.x_config.num_bins;
  const syBins = src.y_config.num_bins;
  const sxMin = src.x_config.min_value;
  const sxMax = src.x_config.max_value;
  const syMin = src.y_config.min_value;
  const syMax = src.y_config.max_value;
  const sxW = (sxMax - sxMin) / sxBins;
  const syW = (syMax - syMin) / syBins;

  const uxBins = Math.min(Math.max(1, Math.floor(xView.bins)), sxBins);
  const uyBins = Math.min(Math.max(1, Math.floor(yView.bins)), syBins);
  const uxW = (xView.max - xView.min) / uxBins;
  const uyW = (yView.max - yView.min) / uyBins;

  const out = new Array(uxBins * uyBins).fill(0);
  let outOver = src.overflow;
  let outTotal = 0;

  for (let sy = 0; sy < syBins; sy++) {
    const yCenter = syMin + (sy + 0.5) * syW;
    if (yCenter < yView.min || yCenter >= yView.max) {
      // Aggregate the whole row into overflow.
      let rowSum = 0;
      const base = sy * sxBins;
      for (let sx = 0; sx < sxBins; sx++) rowSum += src.bins[base + sx];
      outOver += rowSum;
      continue;
    }
    const uy = Math.floor((yCenter - yView.min) / uyW);
    const base = sy * sxBins;
    for (let sx = 0; sx < sxBins; sx++) {
      const c = src.bins[base + sx];
      if (c === 0) continue;
      const xCenter = sxMin + (sx + 0.5) * sxW;
      if (xCenter < xView.min || xCenter >= xView.max) {
        outOver += c;
        continue;
      }
      const ux = Math.floor((xCenter - xView.min) / uxW);
      if (ux >= 0 && ux < uxBins && uy >= 0 && uy < uyBins) {
        out[uy * uxBins + ux] += c;
        outTotal += c;
      }
    }
  }

  return {
    bins: out,
    x_config: { num_bins: uxBins, min_value: xView.min, max_value: xView.max },
    y_config: { num_bins: uyBins, min_value: yView.min, max_value: yView.max },
    total_counts: outTotal,
    overflow: outOver,
  };
}

/** Native bins of the range a view shows on `cfg` (default: all of it). */
export function nativeBinsInView(cfg: AxisConfig, view: RebinView | null | undefined): number {
  return nativeBinsIn(cfg, view?.min ?? cfg.min_value, view?.max ?? cfg.max_value);
}

/** The view with a rebin factor; a bin count saved before TODO 87 goes. */
export function withRebin<V extends RebinView>(view: V | undefined, rebin: number): V {
  const { bins: _legacy, ...rest } = (view ?? {}) as V;
  return { ...rest, rebin } as V;
}
