import {
  REBIN_FACTORS,
  allowedRebinFactors,
  nativeBinsIn,
  rebin1d,
  rebinFactorOf,
  resolveRebinView,
} from './rebin';

describe('rebin choices (TODO 87)', () => {
  const energy = { num_bins: 65536, min_value: 0, max_value: 65536 };

  it('offers x1, x2, x4 and x8 only', () => {
    expect([...REBIN_FACTORS]).toEqual([1, 2, 4, 8]);
  });

  it('counts the native bins inside the shown range', () => {
    expect(nativeBinsIn(energy, 0, 65536)).toBe(65536);
    expect(nativeBinsIn(energy, 0, 4096)).toBe(4096);
    // A keV-calibrated axis keeps the bin count, only the scale changes.
    const kev = { num_bins: 65536, min_value: 126.5, max_value: 126.5 + 65536 * 4 };
    expect(nativeBinsIn(kev, 126.5, 126.5 + 4096 * 4)).toBe(4096);
  });

  it('allows only factors that split the range evenly', () => {
    expect(allowedRebinFactors(65536)).toEqual([1, 2, 4, 8]);
    expect(allowedRebinFactors(200)).toEqual([1, 2, 4, 8]);
    expect(allowedRebinFactors(1500)).toEqual([1, 2, 4]);
    expect(allowedRebinFactors(1001)).toEqual([1]);
  });

  it('reads the factor, or the bin count of a layout saved before TODO 87', () => {
    expect(rebinFactorOf({ rebin: 4 }, 65536)).toBe(4);
    expect(rebinFactorOf(undefined, 65536)).toBe(1);
    // The LS tab on pf-qaq: 1024 bins = x64, now the largest choice.
    expect(rebinFactorOf({ bins: 1024 }, 65536)).toBe(8);
    expect(rebinFactorOf({ bins: 65536 }, 65536)).toBe(1);
    // More bins than the range has (old bins with a narrowed range): x1, not a comb.
    expect(rebinFactorOf({ bins: 65536 }, 4096)).toBe(1);
    // A factor the range cannot take falls back to the next one that fits.
    expect(rebinFactorOf({ rebin: 8 }, 1500)).toBe(4);
  });

  it('rebins the shown range to native / factor bins', () => {
    expect(resolveRebinView({ rebin: 2 }, energy)).toEqual({ min: 0, max: 65536, bins: 32768 });
    expect(resolveRebinView({ min: 0, max: 4096, rebin: 2 }, energy)).toEqual({
      min: 0,
      max: 4096,
      bins: 2048,
    });
    expect(resolveRebinView({ min: 0, max: 65536, bins: 1024 }, energy).bins).toBe(8192);
  });

  it('puts exactly `factor` native bins into each shown bin (no comb)', () => {
    const src = {
      bins: Array.from({ length: 16 }, () => 1),
      config: { num_bins: 16, min_value: 0, max_value: 16 },
      total_counts: 16,
      overflow: 0,
      underflow: 0,
    };
    const out = rebin1d(src, resolveRebinView({ min: 0, max: 12, rebin: 4 }, src.config));
    expect(out.bins).toEqual([4, 4, 4]);
  });
});
