import { TestBed } from '@angular/core/testing';
import { FittingService, FitInput, GaussianFitResult, evaluatePiecewiseBg, BG_RANGE } from './fitting.service';

describe('FittingService', () => {
  let service: FittingService;

  beforeEach(() => {
    TestBed.configureTestingModule({});
    service = TestBed.inject(FittingService);
  });

  it('should be created', () => {
    expect(service).toBeTruthy();
  });

  describe('fitGaussian (7-param piecewise BG)', () => {
    it('should fit a simple Gaussian peak', () => {
      const center = 500;
      const sigma = 20;
      const amplitude = 1000;
      const bins = generateGaussian(center, sigma, amplitude, 0, 1000, 1000);

      const input: FitInput = {
        bins,
        binWidth: 1,
        minValue: 0,
        fitRangeMin: 400,
        fitRangeMax: 600,
      };

      const result = service.fitGaussian(input);

      expect(result).not.toBeNull();
      expect(result!.center).toBeCloseTo(center, 0);
      expect(result!.sigma).toBeCloseTo(sigma, 0);
      expect(result!.amplitude).toBeCloseTo(amplitude, -1);
    });

    it('should fit Gaussian with uniform linear background', () => {
      const center = 500;
      const sigma = 30;
      const amplitude = 800;
      const bgSlope = 0.5;
      const bgIntercept = 100;

      const bins = generateGaussianWithBackground(
        center, sigma, amplitude, bgSlope, bgIntercept,
        0, 1000, 1000
      );

      const input: FitInput = {
        bins,
        binWidth: 1,
        minValue: 0,
        fitRangeMin: 350,
        fitRangeMax: 650,
      };

      const result = service.fitGaussian(input);

      expect(result).not.toBeNull();
      expect(result!.center).toBeCloseTo(center, 0);
      expect(result!.sigma).toBeCloseTo(sigma, 0);
    });

    it('should fit Gaussian with asymmetric (piecewise) background', () => {
      const center = 500;
      const sigma = 20;
      const amplitude = 1000;
      // Left BG: steeper slope (Compton scattering effect)
      const leftSlope = 2.0;
      const leftIntercept = 50;
      // Right BG: flatter
      const rightSlope = 0.3;
      const rightIntercept = 300;

      const bins = generateGaussianWithPiecewiseBg(
        center, sigma, amplitude,
        leftSlope, leftIntercept,
        rightSlope, rightIntercept,
        0, 1000, 1000
      );

      const input: FitInput = {
        bins,
        binWidth: 1,
        minValue: 0,
        fitRangeMin: 350,
        fitRangeMax: 650,
      };

      const result = service.fitGaussian(input);

      expect(result).not.toBeNull();
      expect(result!.center).toBeCloseTo(center, 0);
      expect(result!.sigma).toBeCloseTo(sigma, 0);
      // Left and right BG lines should have different slopes
      expect(Math.abs(result!.leftLine.slope - result!.rightLine.slope)).toBeGreaterThan(0.5);
    });

    it('should return null for empty range', () => {
      const bins = new Array(100).fill(0);
      const input: FitInput = {
        bins,
        binWidth: 1,
        minValue: 0,
        fitRangeMin: 10,
        fitRangeMax: 20,
      };

      const result = service.fitGaussian(input);
      expect(result).toBeNull();
    });

    it('should calculate FWHM correctly', () => {
      const sigma = 25;
      const bins = generateGaussian(500, sigma, 1000, 0, 1000, 1000);

      const input: FitInput = {
        bins,
        binWidth: 1,
        minValue: 0,
        fitRangeMin: 400,
        fitRangeMax: 600,
      };

      const result = service.fitGaussian(input);

      expect(result).not.toBeNull();
      const expectedFwhm = 2.355 * sigma;
      expect(result!.fwhm).toBeCloseTo(expectedFwhm, 0);
    });

    it('should calculate net area correctly', () => {
      const center = 500;
      const sigma = 20;
      const amplitude = 1000;
      const bins = generateGaussian(center, sigma, amplitude, 0, 1000, 1000);

      const input: FitInput = {
        bins,
        binWidth: 1,
        minValue: 0,
        fitRangeMin: 400,
        fitRangeMax: 600,
      };

      const result = service.fitGaussian(input);

      expect(result).not.toBeNull();
      const expectedArea = amplitude * sigma * Math.sqrt(2 * Math.PI);
      expect(result!.netArea).toBeCloseTo(expectedArea, -2);
    });

    it('should calculate chi2 and ndf', () => {
      const bins = generateGaussian(500, 20, 1000, 0, 1000, 1000);

      const input: FitInput = {
        bins,
        binWidth: 1,
        minValue: 0,
        fitRangeMin: 400,
        fitRangeMax: 600,
      };

      const result = service.fitGaussian(input);

      expect(result).not.toBeNull();
      expect(result!.chi2).toBeGreaterThanOrEqual(0);
      expect(result!.ndf).toBeGreaterThan(0);
      expect(result!.chi2 / result!.ndf).toBeLessThan(5);
    });
  });

  describe('evaluatePiecewiseBg', () => {
    it('should return left line value below limitLow', () => {
      const leftLine = { slope: 2, intercept: 100 };
      const rightLine = { slope: -1, intercept: 800 };
      const center = 500;
      const sigma = 20;
      const x = 400; // well below 500 - 2.5*20 = 450

      const result = evaluatePiecewiseBg(x, center, sigma, leftLine, rightLine);
      const expected = leftLine.slope * x + leftLine.intercept;
      expect(result).toBeCloseTo(expected, 5);
    });

    it('should return right line value above limitHigh', () => {
      const leftLine = { slope: 2, intercept: 100 };
      const rightLine = { slope: -1, intercept: 800 };
      const center = 500;
      const sigma = 20;
      const x = 600; // well above 500 + 2.5*20 = 550

      const result = evaluatePiecewiseBg(x, center, sigma, leftLine, rightLine);
      const expected = rightLine.slope * x + rightLine.intercept;
      expect(result).toBeCloseTo(expected, 5);
    });

    it('should interpolate in peak region', () => {
      const leftLine = { slope: 0, intercept: 100 };
      const rightLine = { slope: 0, intercept: 200 };
      const center = 500;
      const sigma = 20;
      // limitLow = 500 - 2.5*20 = 450, limitHigh = 500 + 2.5*20 = 550
      // At center (midpoint), should be average of left(450) and right(550)
      const yLow = 100; // leftLine at 450
      const yHigh = 200; // rightLine at 550
      const expected = (yLow + yHigh) / 2;

      const result = evaluatePiecewiseBg(center, center, sigma, leftLine, rightLine);
      expect(result).toBeCloseTo(expected, 5);
    });

    it('should clamp negative background to zero', () => {
      const leftLine = { slope: 0, intercept: -50 };
      const rightLine = { slope: 0, intercept: -50 };
      const result = evaluatePiecewiseBg(500, 500, 20, leftLine, rightLine);
      expect(result).toBe(0);
    });
  });

  describe('fitLinearBackground', () => {
    it('should fit left background line', () => {
      const bins = new Array(1000).fill(0);
      for (let i = 0; i < 400; i++) {
        bins[i] = 2 * (i + 0.5) + 50;
      }

      const result = service.fitLinearBackground(bins, 1, 0, 100, 300);

      expect(result).not.toBeNull();
      expect(result!.slope).toBeCloseTo(2, 3);
      expect(result!.intercept).toBeCloseTo(50, 1);
    });

    it('should fit right background line', () => {
      const bins = new Array(1000).fill(0);
      for (let i = 600; i < 1000; i++) {
        bins[i] = -1.5 * (i + 0.5) + 1800;
      }

      const result = service.fitLinearBackground(bins, 1, 0, 700, 900);

      expect(result).not.toBeNull();
      expect(result!.slope).toBeCloseTo(-1.5, 3);
      expect(result!.intercept).toBeCloseTo(1800, 1);
    });
  });

  describe('calculateBackgroundLine', () => {
    it('should calculate background connecting left and right edges', () => {
      const leftLine = { slope: 1, intercept: 100 };
      const rightLine = { slope: -1, intercept: 1500 };

      const bgLine = service.calculateBackgroundLine(leftLine, rightLine, 400, 600);

      expect(bgLine.slope).toBeCloseTo(2, 5);
      expect(bgLine.intercept).toBeCloseTo(-300, 5);
    });
  });
});

// Helper functions for generating test data

function generateGaussian(
  center: number,
  sigma: number,
  amplitude: number,
  minValue: number,
  maxValue: number,
  numBins: number
): number[] {
  const bins: number[] = [];
  const binWidth = (maxValue - minValue) / numBins;

  for (let i = 0; i < numBins; i++) {
    const x = minValue + (i + 0.5) * binWidth;
    const y = amplitude * Math.exp(-0.5 * Math.pow((x - center) / sigma, 2));
    bins.push(Math.round(y));
  }

  return bins;
}

function generateGaussianWithBackground(
  center: number,
  sigma: number,
  amplitude: number,
  bgSlope: number,
  bgIntercept: number,
  minValue: number,
  maxValue: number,
  numBins: number
): number[] {
  const bins: number[] = [];
  const binWidth = (maxValue - minValue) / numBins;

  for (let i = 0; i < numBins; i++) {
    const x = minValue + (i + 0.5) * binWidth;
    const gaussian = amplitude * Math.exp(-0.5 * Math.pow((x - center) / sigma, 2));
    const background = bgSlope * x + bgIntercept;
    bins.push(Math.round(gaussian + background));
  }

  return bins;
}

/** Generate test data with piecewise BG (different left/right slopes) */
function generateGaussianWithPiecewiseBg(
  center: number,
  sigma: number,
  amplitude: number,
  leftSlope: number,
  leftIntercept: number,
  rightSlope: number,
  rightIntercept: number,
  minValue: number,
  maxValue: number,
  numBins: number
): number[] {
  const bins: number[] = [];
  const binWidth = (maxValue - minValue) / numBins;
  const limitLow = center - BG_RANGE * sigma;
  const limitHigh = center + BG_RANGE * sigma;

  for (let i = 0; i < numBins; i++) {
    const x = minValue + (i + 0.5) * binWidth;
    const gaussian = amplitude * Math.exp(-0.5 * Math.pow((x - center) / sigma, 2));

    let bg: number;
    if (x < limitLow) {
      bg = leftIntercept + leftSlope * x;
    } else if (x > limitHigh) {
      bg = rightIntercept + rightSlope * x;
    } else {
      const yLow = leftIntercept + leftSlope * limitLow;
      const yHigh = rightIntercept + rightSlope * limitHigh;
      const slope = (yHigh - yLow) / (limitHigh - limitLow);
      bg = yLow + slope * (x - limitLow);
    }
    bg = Math.max(0, bg);

    bins.push(Math.round(gaussian + bg));
  }

  return bins;
}

// Statistical errors are checked against what they promise: over many toy
// histograms, the scatter of a fitted value must match the error the fit reports.
describe('FittingService statistical errors', () => {
  const service = new FittingService();

  /** Small seeded generator (mulberry32), so a failure can be reproduced. */
  function rng(seed: number): () => number {
    let a = seed >>> 0;
    return () => {
      a = (a + 0x6d2b79f5) >>> 0;
      let t = a;
      t = Math.imul(t ^ (t >>> 15), t | 1);
      t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
  }

  function poisson(random: () => number, mean: number): number {
    const limit = Math.exp(-mean);
    let k = 0;
    let p = random();
    while (p > limit) {
      k++;
      p *= random();
    }
    return k;
  }

  interface Toy {
    center: number;
    sigma: number;
    amplitude: number;
    background: number;
  }

  function toyBins(toy: Toy, seed: number): number[] {
    const random = rng(seed);
    const bins: number[] = [];
    for (let i = 0; i < 1000; i++) {
      const g = toy.amplitude * Math.exp(-0.5 * Math.pow((i + 0.5 - toy.center) / toy.sigma, 2));
      bins.push(poisson(random, g + toy.background));
    }
    return bins;
  }

  function summary(values: number[]): { mean: number; rms: number } {
    const mean = values.reduce((s, v) => s + v, 0) / values.length;
    const variance = values.reduce((s, v) => s + (v - mean) * (v - mean), 0) / values.length;
    return { mean, rms: Math.sqrt(variance) };
  }

  function fitToys(toy: Toy, rangeMin: number, rangeMax: number, toys: number): GaussianFitResult[] {
    const fits: GaussianFitResult[] = [];
    for (let seed = 1; seed <= toys; seed++) {
      const fit = service.fitGaussian({
        bins: toyBins(toy, seed),
        binWidth: 1,
        minValue: 0,
        fitRangeMin: rangeMin,
        fitRangeMax: rangeMax,
      });
      if (fit) fits.push(fit);
    }
    expect(fits.length).toBe(toys);
    return fits;
  }

  /** Reported error / observed scatter; 1 means the error is right. */
  function errorScale(
    fits: GaussianFitResult[],
    value: (f: GaussianFitResult) => number,
    error: (f: GaussianFitResult) => number,
  ): number {
    return summary(fits.map(error)).mean / summary(fits.map(value)).rms;
  }

  function expectHonest(scale: number, name: string): void {
    expect(scale).withContext(name).toBeGreaterThan(0.75);
    expect(scale).withContext(name).toBeLessThan(1.33);
  }

  function expectHonestErrors(fits: GaussianFitResult[]): void {
    expectHonest(errorScale(fits, (f) => f.center, (f) => f.centerError), 'center');
    expectHonest(errorScale(fits, (f) => f.sigma, (f) => f.sigmaError), 'sigma');
    expectHonest(errorScale(fits, (f) => f.netArea, (f) => f.netAreaError), 'area');
  }

  it('match the scatter of a peak on a flat background', () => {
    expectHonestErrors(fitToys({ center: 500, sigma: 20, amplitude: 100, background: 5 }, 350, 650, 100));
  });

  it('match the scatter of a peak with few counts per bin', () => {
    expectHonestErrors(fitToys({ center: 500, sigma: 40, amplitude: 12, background: 1 }, 250, 750, 100));
  });

  // With no background at all the clamp at 0 makes the fit non-linear: the errors of
  // sigma and the area come out about 30 % small (TODO 83 section 14). The center holds.
  it('match the scatter of the center with no background', () => {
    const fits = fitToys({ center: 500, sigma: 20, amplitude: 100, background: 0 }, 350, 650, 100);
    expectHonest(errorScale(fits, (f) => f.center, (f) => f.centerError), 'center');
  });

  it('are within a factor of about 2 of sigma / sqrt(N) for the center', () => {
    const toy = { center: 500, sigma: 20, amplitude: 100, background: 5 };
    const fit = service.fitGaussian({
      bins: toyBins(toy, 7),
      binWidth: 1,
      minValue: 0,
      fitRangeMin: 350,
      fitRangeMax: 650,
    });
    expect(fit).not.toBeNull();
    const ideal = toy.sigma / Math.sqrt(toy.amplitude * toy.sigma * Math.sqrt(2 * Math.PI));
    expect(fit!.centerError).toBeGreaterThan(ideal);
    expect(fit!.centerError).toBeLessThan(ideal * 2.2);
  });

  function medianSigma(fits: GaussianFitResult[]): number {
    const sorted = fits.map((f) => f.sigma).sort((a, b) => a - b);
    return sorted[Math.floor(sorted.length / 2)];
  }

  // One noisy bin used to set the start: the fit locked on to it and gave sigma 18,
  // and failed outright in a third of these.
  it('finds a broad peak with very few counts per bin, not a noise spike', () => {
    const fits = fitToys({ center: 500, sigma: 40, amplitude: 3, background: 0.2 }, 250, 750, 60);
    expect(medianSigma(fits)).toBeGreaterThan(35);
    expect(medianSigma(fits)).toBeLessThan(45);
  });

  // ... and smoothing alone would miss this one, by starting it too wide.
  it('finds a narrow peak in a wide range', () => {
    const fits = fitToys({ center: 500, sigma: 5, amplitude: 200, background: 1 }, 0, 1000, 25);
    expect(medianSigma(fits)).toBeGreaterThan(4.5);
    expect(medianSigma(fits)).toBeLessThan(5.5);
  });

  it('survive a range with no background side bins', () => {
    const toy = { center: 500, sigma: 40, amplitude: 100, background: 5 };
    const fit = service.fitGaussian({
      bins: toyBins(toy, 3),
      binWidth: 1,
      minValue: 0,
      fitRangeMin: 440,
      fitRangeMax: 560,
    });
    expect(fit).not.toBeNull();
    expect(Number.isFinite(fit!.centerError)).toBeTrue();
    expect(fit!.centerError).toBeGreaterThan(0);
  });

  it('scale with the axis, and the area stays in counts', () => {
    const toy = { center: 500, sigma: 20, amplitude: 100, background: 5 };
    const bins = toyBins(toy, 11);
    const adc = service.fitGaussian({
      bins,
      binWidth: 1,
      minValue: 0,
      fitRangeMin: 350,
      fitRangeMax: 650,
    })!;
    // The same counts on a keV axis: E = 0.5 ADC - 10.
    const kev = service.fitGaussian({
      bins,
      binWidth: 0.5,
      minValue: -10,
      fitRangeMin: 165,
      fitRangeMax: 315,
    })!;
    expect(kev.center).toBeCloseTo(0.5 * adc.center - 10, 2);
    expect(kev.sigma).toBeCloseTo(0.5 * adc.sigma, 2);
    expect(kev.centerError).toBeCloseTo(0.5 * adc.centerError, 2);
    expect(kev.sigmaError).toBeCloseTo(0.5 * adc.sigmaError, 2);
    expect(kev.netArea).toBeCloseTo(adc.netArea, 0);
    expect(kev.netAreaError).toBeCloseTo(adc.netAreaError, 0);
    // The peak holds amplitude * sigma * sqrt(2 pi) counts.
    expect(adc.netArea).toBeGreaterThan(4800);
    expect(adc.netArea).toBeLessThan(5200);
  });
});
