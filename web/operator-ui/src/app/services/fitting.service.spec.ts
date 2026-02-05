import { TestBed } from '@angular/core/testing';
import { FittingService, FitInput, evaluatePiecewiseBg, BG_RANGE } from './fitting.service';

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
