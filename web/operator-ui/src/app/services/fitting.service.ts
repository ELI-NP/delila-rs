import { Injectable } from '@angular/core';
import { levenbergMarquardt } from 'ml-levenberg-marquardt';

/** Boundary for piecewise BG: left/right BG applies outside μ ± BG_RANGE*σ */
export const BG_RANGE = 2.5;

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
  netArea: number;

  // Errors (from covariance matrix)
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

  return Math.max(0, bg);
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

    // Estimate initial parameters [A, μ, σ, b_L, m_L, b_R, m_R]
    const initialParams = this.estimateInitialParams(xData, yData);
    if (!initialParams) {
      return null;
    }

    // 7-parameter piecewise model
    const piecewiseModel = (params: number[]) => (x: number) => {
      const [A, mu, sigma, bL, mL, bR, mR] = params;
      const absSigma = Math.abs(sigma);

      const gaussian = A * Math.exp(-0.5 * Math.pow((x - mu) / sigma, 2));
      const bg = evaluatePiecewiseBg(
        x, mu, absSigma,
        { intercept: bL, slope: mL },
        { intercept: bR, slope: mR },
      );

      return gaussian + bg;
    };

    try {
      const result = levenbergMarquardt(
        { x: xData, y: yData },
        piecewiseModel,
        {
          damping: 1.5,
          initialValues: initialParams,
          gradientDifference: 1e-6,
          maxIterations: 200,
          errorTolerance: 1e-8,
        }
      );

      const [amplitude, center, sigma, bL, mL, bR, mR] = result.parameterValues;

      // Validate results
      if (sigma <= 0 || amplitude <= 0) {
        return null;
      }

      // Calculate chi-squared (7 parameters)
      const { chi2, ndf } = this.calculateChi2(xData, yData, result.parameterValues, piecewiseModel);

      const absSigma = Math.abs(sigma);
      const fwhm = 2.355 * absSigma;
      const netArea = amplitude * absSigma * Math.sqrt(2 * Math.PI);

      // Parameter errors
      const paramErrors = Array.isArray(result.parameterError)
        ? result.parameterError
        : [0, 0, 0, 0, 0, 0, 0];
      const amplitudeError = paramErrors[0] ?? 0;
      const centerError = paramErrors[1] ?? 0;
      const sigmaError = paramErrors[2] ?? 0;

      const netAreaError = netArea * Math.sqrt(
        Math.pow(amplitudeError / amplitude, 2) +
        Math.pow(sigmaError / absSigma, 2)
      );

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
