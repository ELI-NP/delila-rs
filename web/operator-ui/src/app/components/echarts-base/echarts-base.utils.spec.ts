import {
  buildDualSliderDataZoom,
  defaultGrid,
  siCountFormatter,
} from './echarts-base.utils';

describe('echarts-base.utils', () => {
  describe('siCountFormatter (linear)', () => {
    const fmt = siCountFormatter('linear');

    it('returns "0" for zero', () => {
      expect(fmt(0)).toBe('0');
    });

    it('uses k suffix at >= 1e3', () => {
      expect(fmt(1500)).toBe('2k');
      expect(fmt(99999)).toBe('100k');
    });

    it('uses M / G with one decimal at >= 1e6 / 1e9', () => {
      expect(fmt(2_500_000)).toBe('2.5M');
      expect(fmt(1_200_000_000)).toBe('1.2G');
    });

    it('floors small positive integers', () => {
      expect(fmt(7)).toBe('7');
      expect(fmt(7.9)).toBe('7');
    });

    it('handles negative magnitudes via abs', () => {
      expect(fmt(-2_500_000)).toBe('-2.5M');
    });
  });

  describe('siCountFormatter (log)', () => {
    const fmt = siCountFormatter('log');

    it('returns "0" for zero', () => {
      expect(fmt(0)).toBe('0');
    });

    it('uses 0-decimal precision', () => {
      // (1.5).toFixed(0) rounds half-away-from-zero in V8 → '2'.
      expect(fmt(1_500_000)).toBe('2M');
      expect(fmt(2_500_000_000)).toBe('3G');
      expect(fmt(7_000_000)).toBe('7M');
      expect(fmt(1_400_000)).toBe('1M');
    });
  });

  describe('defaultGrid', () => {
    it('returns sensible default margins', () => {
      const g = defaultGrid() as Record<string, number>;
      expect(g['top']).toBe(30);
      expect(g['right']).toBe(60);
      expect(g['bottom']).toBe(60);
      expect(g['left']).toBe(60);
    });

    it('applies overrides without mutating defaults', () => {
      const g = defaultGrid({ top: 100, right: 200 }) as Record<string, number>;
      expect(g['top']).toBe(100);
      expect(g['right']).toBe(200);
      expect(g['bottom']).toBe(60); // default preserved
    });

    it('returns a fresh object per call', () => {
      const a = defaultGrid();
      const b = defaultGrid();
      expect(a).not.toBe(b);
    });
  });

  describe('buildDualSliderDataZoom', () => {
    it('returns inside + slider for both axes', () => {
      const dz = buildDualSliderDataZoom() as Array<Record<string, unknown>>;
      expect(dz).toHaveSize(4);
      expect(dz[0]['type']).toBe('inside');
      expect(dz[2]['type']).toBe('slider');
    });
  });
});
