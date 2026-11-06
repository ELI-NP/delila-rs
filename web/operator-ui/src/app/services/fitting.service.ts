import { Injectable } from '@angular/core';
import { levenbergMarquardt } from 'ml-levenberg-marquardt';
import { Matrix, pseudoInverse } from 'ml-matrix';

/** Boundary for piecewise BG: left/right BG applies outside μ ± BG_RANGE*σ */
export const BG_RANGE = 2.5;

/** Singular values below this (columns scaled to 1) are directions the data cannot see. */
const COVARIANCE_RCOND = 1e-6;

export interface FitInput {
  bins: number[];
  binWidth: number;
  minValue: number;
  fitRangeMin: number;
  fitRangeMax: number;
}

export interface LinearFit {
  slope: number;
  intercept: number;
}

export interface GaussianFitResult {
  // Gaussian parameters
  amplitude: number;
  center: number;
  sigma: number;

  // Background lines
  leftLine: LinearFit;
  rightLine: LinearFit;
  bgLine: LinearFit; // connecting line in peak region (derived)

  // Derived values
  fwhm: number;
  /** Counts in the peak, whatever the unit of the X axis. */
  netArea: number;

  // Statistical errors, NaN if they could not be worked out
  centerError: number;
  sigmaError: number;
  amplitudeError: number;
  netAreaError: number;

  // Goodness of fit
  chi2: number;
  ndf: number;
}

/**
 * Evaluate piecewise background at x.
 * - x < μ - BG_RANGE*σ  → left linear BG
 * - x > μ + BG_RANGE*σ  → right linear BG
 * - otherwise            → linear interpolation connecting the two
 */
export function evaluatePiecewiseBg(
  x: number,
  center: number,
  sigma: number,
  leftLine: LinearFit,
  rightLine: LinearFit,
): number {
  return Math.max(0, unclampedPiecewiseBg(x, center, sigma, leftLine, rightLine));
}

function unclampedPiecewiseBg(
  x: number,
  center: number,
  sigma: number,
  leftLine: LinearFit,
  rightLine: LinearFit,
): number {
  const limitLow = center - BG_RANGE * sigma;
  const limitHigh = center + BG_RANGE * sigma;

  let bg: number;
  if (x < limitLow) {
    bg = leftLine.intercept + leftLine.slope * x;
  } else if (x > limitHigh) {
    bg = rightLine.intercept + rightLine.slope * x;
  } else {
    const yLow = leftLine.intercept + leftLine.slope * limitLow;
    const yHigh = rightLine.intercept + rightLine.slope * limitHigh;
    const slope = (yHigh - yLow) / (limitHigh - limitLow);
    bg = yLow + slope * (x - limitLow);
  }

  return bg;
}

@Injectable({
  providedIn: 'root',
})
export class FittingService {
  /**
   * Fit a Gaussian peak with piecewise linear background.
   * 7-parameter simultaneous fit: [A, μ, σ, b_L, m_L, b_R, m_R]
   *
   * Background model:
   *   x < μ - 2.5σ  → b_L + m_L * x    (left region)
   *   x > μ + 2.5σ  → b_R + m_R * x    (right region)
   *   otherwise      → linear interpolation connecting boundary values
   */
  fitGaussian(input: FitInput): GaussianFitResult | null {
    const { bins, binWidth, minValue, fitRangeMin, fitRangeMax } = input;

    // Extract data within fit range
    const { xData, yData } = this.extractRange(bins, binWidth, minValue, fitRangeMin, fitRangeMax);

    if (xData.length < 10) {
      return null;
    }

    // Check if there's any signal
    const maxY = Math.max(...yData);
    if (maxY <= 0) {
      return null;
    }

    // Start from both the raw and a smoothed spectrum, and keep the better fit. With few
    // counts per bin one noisy bin sets the raw top and width, and the fit locks on to
    // that spike. Smoothing cures that but starts a narrow peak in a wide range too wide
    // to converge.
    const starts = [this.estimateInitialParams(xData, yData)];
    const smoothWidth = Math.floor(xData.length / 20);
    if (smoothWidth >= 2) {
      starts.push(this.estimateInitialParams(xData, this.smooth(yData, smoothWidth)));
    }

    // 7-parameter piecewise model
    const makeModel = (bgAt: typeof evaluatePiecewiseBg) => (params: number[]) => (x: number) => {
      const [A, mu, sigma, bL, mL, bR, mR] = params;
      const absSigma = Math.abs(sigma);

      const gaussian = A * Math.exp(-0.5 * Math.pow((x - mu) / sigma, 2));
      const bg = bgAt(
        x, mu, absSigma,
        { intercept: bL, slope: mL },
        { intercept: bR, slope: mR },
      );

      return gaussian + bg;
    };
    const piecewiseModel = makeModel(evaluatePiecewiseBg);

    try {
      let result: { parameterValues: number[] } | null = null;
      let bestDeviance = Infinity;
      for (const initialValues of starts) {
        if (!initialValues) continue;
        const candidate = levenbergMarquardt(
          { x: xData, y: yData },
          piecewiseModel,
          {
            damping: 1.5,
            initialValues,
            gradientDifference: 1e-6,
            maxIterations: 200,
            errorTolerance: 1e-8,
          }
        );
        // Validate results
        const [a, , s] = candidate.parameterValues;
        if (s <= 0 || a <= 0) continue;
        const deviance = this.poissonDeviance(xData, yData, piecewiseModel(candidate.parameterValues));
        if (deviance < bestDeviance) {
          bestDeviance = deviance;
          result = candidate;
        }
      }
      if (!result) {
        return null;
      }

      const [amplitude, center, sigma, bL, mL, bR, mR] = result.parameterValues;

      // Calculate chi-squared (7 parameters)
      const { chi2, ndf } = this.calculateChi2(xData, yData, result.parameterValues, piecewiseModel);

      const absSigma = Math.abs(sigma);
      const fwhm = 2.355 * absSigma;
      // The bins hold counts per bin: dividing by the bin width gives counts, not
      // counts x axis unit.
      const netArea = amplitude * absSigma * Math.sqrt(2 * Math.PI) / binWidth;

      // Sensitivities without the clamp at 0: where the fitted background sits at 0 it
      // is still free to take up counts, and the errors have to include that.
      const cov = this.parameterCovariance(
        xData,
        result.parameterValues,
        makeModel(unclampedPiecewiseBg),
        piecewiseModel(result.parameterValues),
      );
      const amplitudeError = cov ? Math.sqrt(cov[0][0]) : NaN;
      const centerError = cov ? Math.sqrt(cov[1][1]) : NaN;
      const sigmaError = cov ? Math.sqrt(cov[2][2]) : NaN;

      // A and sigma are correlated: a wider peak fits with a lower top.
      const areaRelVariance = cov
        ? cov[0][0] / (amplitude * amplitude) +
          cov[2][2] / (absSigma * absSigma) +
          (2 * cov[0][2]) / (amplitude * absSigma)
        : NaN;
      const netAreaError = netArea * Math.sqrt(Math.max(areaRelVariance, 0));

      const leftLine: LinearFit = { slope: mL, intercept: bL };
      const rightLine: LinearFit = { slope: mR, intercept: bR };

      // Compute connecting line in peak region for convenience
      const limitLow = center - BG_RANGE * absSigma;
      const limitHigh = center + BG_RANGE * absSigma;
      const yLow = bL + mL * limitLow;
      const yHigh = bR + mR * limitHigh;
      const bgSlope = (yHigh - yLow) / (limitHigh - limitLow);
      const bgIntercept = yLow - bgSlope * limitLow;
      const bgLine: LinearFit = { slope: bgSlope, intercept: bgIntercept };

      return {
        amplitude,
        center,
        sigma: absSigma,
        leftLine,
        rightLine,
        bgLine,
        fwhm,
        netArea,
        centerError,
        sigmaError,
        amplitudeError,
        netAreaError,
        chi2,
        ndf,
      };
    } catch {
      console.error('Fitting failed');
      return null;
    }
  }

  /**
   * Covariance of the fitted parameters from the Poisson scatter of the bins.
   *
   * The fit is unweighted least squares, so near the minimum the parameters follow
   * the data as dp = J+ dy, and Cov(p) = J+ V (J+)^T with V = diag(expected counts).
   * (J^T W J)^-1 is what a weighted fit would give, which this one is not.
   *
   * The pseudo-inverse copes with parameters the data cannot see: a background
   * clamped at 0, or a range with no bins beside the peak to fix a slope.
   * Returns null if it cannot be worked out, so that no error is ever made up.
   */
  private parameterCovariance(
    xData: number[],
    params: number[],
    model: (params: number[]) => (x: number) => number,
    expected: (x: number) => number,
  ): number[][] | null {
    try {
      const [amplitude, , sigma] = params;
      const span = Math.max(xData[xData.length - 1] - xData[0], 1e-9);
      const bgStep = 1e-3 * Math.max(amplitude, 1);
      const steps = [
        1e-3 * amplitude,
        1e-3 * sigma,
        1e-3 * sigma,
        bgStep,
        bgStep / span,
        bgStep,
        bgStep / span,
      ];

      // Jacobian by central differences, columns scaled to length 1 so that an
      // intercept and a slope of very different size stay comparable.
      const n = xData.length;
      const jacobian = Matrix.zeros(n, params.length);
      const scale: number[] = [];
      for (let k = 0; k < params.length; k++) {
        const up = [...params];
        const down = [...params];
        up[k] += steps[k];
        down[k] -= steps[k];
        const fUp = model(up);
        const fDown = model(down);
        let norm2 = 0;
        const column: number[] = [];
        for (let i = 0; i < n; i++) {
          const d = (fUp(xData[i]) - fDown(xData[i])) / (2 * steps[k]);
          column.push(d);
          norm2 += d * d;
        }
        const norm = Math.sqrt(norm2);
        scale.push(norm > 0 ? 1 / norm : 0);
        for (let i = 0; i < n; i++) jacobian.set(i, k, column[i] * scale[k]);
      }

      const pinv = pseudoInverse(jacobian, COVARIANCE_RCOND);
      const variance = xData.map((x) => Math.max(expected(x), 0));
      const cov = params.map((_, a) =>
        params.map((_, b) => {
          let sum = 0;
          for (let i = 0; i < n; i++) sum += pinv.get(a, i) * pinv.get(b, i) * variance[i];
          return sum * scale[a] * scale[b];
        }),
      );
      return cov.every((row) => row.every(Number.isFinite)) ? cov : null;
    } catch (e) {
      console.error('Fit errors could not be worked out', e);
      return null;
    }
  }

  /**
   * Fit a linear function to background region
   */
  fitLinearBackground(
    bins: number[],
    binWidth: number,
    minValue: number,
    rangeMin: number,
    rangeMax: number
  ): LinearFit | null {
    const { xData, yData } = this.extractRange(bins, binWidth, minValue, rangeMin, rangeMax);
    return this.fitLinearToData(xData, yData);
  }

  /**
   * Calculate background line connecting left and right edges
   */
  calculateBackgroundLine(
    leftLine: LinearFit,
    rightLine: LinearFit,
    fitRangeMin: number,
    fitRangeMax: number
  ): LinearFit {
    const yLeft = leftLine.slope * fitRangeMin + leftLine.intercept;
    const yRight = rightLine.slope * fitRangeMax + rightLine.intercept;

    const slope = (yRight - yLeft) / (fitRangeMax - fitRangeMin);
    const intercept = yLeft - slope * fitRangeMin;

    return { slope, intercept };
  }

  /**
   * Fit linear regression to x,y data arrays
   */
  private fitLinearToData(xData: number[], yData: number[]): LinearFit | null {
    if (xData.length < 3) {
      return null;
    }

    const n = xData.length;
    let sumX = 0, sumY = 0, sumXY = 0, sumX2 = 0;

    for (let i = 0; i < n; i++) {
      sumX += xData[i];
      sumY += yData[i];
      sumXY += xData[i] * yData[i];
      sumX2 += xData[i] * xData[i];
    }

    const denominator = n * sumX2 - sumX * sumX;
    if (Math.abs(denominator) < 1e-10) {
      return null;
    }

    const slope = (n * sumXY - sumX * sumY) / denominator;
    const intercept = (sumY - slope * sumX) / n;

    return { slope, intercept };
  }

  /**
   * Extract x,y data within specified range
   */
  private extractRange(
    bins: number[],
    binWidth: number,
    minValue: number,
    rangeMin: number,
    rangeMax: number
  ): { xData: number[]; yData: number[] } {
    const xData: number[] = [];
    const yData: number[] = [];

    for (let i = 0; i < bins.length; i++) {
      const x = minValue + (i + 0.5) * binWidth;
      if (x >= rangeMin && x <= rangeMax) {
        xData.push(x);
        yData.push(bins[i]);
      }
    }

    return { xData, yData };
  }

  /**
   * Estimate initial parameters for 7-param piecewise model.
   * Returns [A, μ, σ, b_L, m_L, b_R, m_R]
   */
  private estimateInitialParams(xData: number[], yData: number[]): number[] | null {
    if (xData.length === 0) return null;

    // Find peak
    let maxY = -Infinity;
    let maxIdx = 0;
    for (let i = 0; i < yData.length; i++) {
      if (yData[i] > maxY) {
        maxY = yData[i];
        maxIdx = i;
      }
    }
    if (maxY <= 0) return null;

    const center = xData[maxIdx];

    // Estimate BG from edges (15% each side, min 3 bins)
    const edgeBins = Math.max(3, Math.floor(xData.length * 0.15));

    const leftFit = this.fitLinearToData(
      xData.slice(0, edgeBins),
      yData.slice(0, edgeBins),
    );
    const rightFit = this.fitLinearToData(
      xData.slice(-edgeBins),
      yData.slice(-edgeBins),
    );

    if (!leftFit || !rightFit) return null;

    // Background at center (average of left/right extrapolations)
    const bgAtCenter = (
      leftFit.slope * center + leftFit.intercept +
      rightFit.slope * center + rightFit.intercept
    ) / 2;

    const amplitude = maxY - bgAtCenter;
    if (amplitude <= 0) return null;

    // Estimate sigma from FWHM
    const halfMax = bgAtCenter + amplitude / 2;
    let leftHalf = center, rightHalf = center;
    for (let i = maxIdx; i >= 0; i--) {
      if (yData[i] < halfMax) { leftHalf = xData[i]; break; }
    }
    for (let i = maxIdx; i < yData.length; i++) {
      if (yData[i] < halfMax) { rightHalf = xData[i]; break; }
    }
    const fwhmEstimate = rightHalf - leftHalf;
    const sigma = fwhmEstimate / 2.355 || (xData[xData.length - 1] - xData[0]) / 10;

    // [A, μ, σ, b_L, m_L, b_R, m_R]
    return [
      amplitude, center, sigma,
      leftFit.intercept, leftFit.slope,
      rightFit.intercept, rightFit.slope,
    ];
  }

  /**
   * How well counts fit a model: twice the Poisson log-likelihood ratio. Used to pick
   * between solutions, where plain squared residuals would favour a tall spike over
   * the broad peak the counts really come from.
   */
  private poissonDeviance(xData: number[], yData: number[], model: (x: number) => number): number {
    let deviance = 0;
    for (let i = 0; i < xData.length; i++) {
      const y = yData[i];
      const f = Math.max(model(xData[i]), 1e-9);
      deviance += 2 * (f - y + (y > 0 ? y * Math.log(y / f) : 0));
    }
    return deviance;
  }

  /** Moving average over `width` bins. */
  private smooth(y: number[], width: number): number[] {
    const half = Math.floor(width / 2);
    const prefix = [0];
    for (const v of y) prefix.push(prefix[prefix.length - 1] + v);
    return y.map((_, i) => {
      const lo = Math.max(0, i - half);
      const hi = Math.min(y.length, i + half + 1);
      return (prefix[hi] - prefix[lo]) / (hi - lo);
    });
  }

  /**
   * Calculate chi-squared and degrees of freedom
   */
  private calculateChi2(
    xData: number[],
    yData: number[],
    params: number[],
    model: (params: number[]) => (x: number) => number,
    nParams?: number
  ): { chi2: number; ndf: number } {
    const f = model(params);
    let chi2 = 0;

    for (let i = 0; i < xData.length; i++) {
      const observed = yData[i];
      const expected = f(xData[i]);
      // Poisson error estimate: σ² = max(observed, 1)
      const variance = Math.max(observed, 1);
      chi2 += Math.pow(observed - expected, 2) / variance;
    }

    const ndf = xData.length - (nParams ?? params.length);

    return { chi2, ndf: Math.max(ndf, 1) };
  }
}
