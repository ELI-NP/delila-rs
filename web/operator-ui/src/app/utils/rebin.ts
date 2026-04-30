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
