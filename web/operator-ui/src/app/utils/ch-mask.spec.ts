import {
  boolArrayToHexString,
  boolArrayToNumber,
  formatMaskCompact,
  maskToBoolArray,
  serializeMask,
} from './ch-mask';

describe('ch-mask helpers', () => {
  describe('maskToBoolArray', () => {
    it('returns all-false for null/undefined/empty', () => {
      expect(maskToBoolArray(null, 4)).toEqual([false, false, false, false]);
      expect(maskToBoolArray(undefined, 4)).toEqual([false, false, false, false]);
      expect(maskToBoolArray('', 4)).toEqual([false, false, false, false]);
      expect(maskToBoolArray('   ', 4)).toEqual([false, false, false, false]);
    });

    it('parses hex strings (PSD2 form)', () => {
      // 0xA = 1010 → bit1, bit3
      expect(maskToBoolArray('A', 4)).toEqual([false, true, false, true]);
      expect(maskToBoolArray('0xA', 4)).toEqual([false, true, false, true]);
      expect(maskToBoolArray('0XFF', 4)).toEqual([true, true, true, true]);
      expect(maskToBoolArray('ff', 8)).toEqual(new Array(8).fill(true));
    });

    it('parses numeric values (PSD1 form)', () => {
      expect(maskToBoolArray(0, 4)).toEqual([false, false, false, false]);
      expect(maskToBoolArray(15, 4)).toEqual([true, true, true, true]);
      // 0b1010 = bit1, bit3
      expect(maskToBoolArray(10, 4)).toEqual([false, true, false, true]);
    });

    it('handles 32-bit values without sign-bit pollution', () => {
      // 0x80000000 sets only bit 31 — guard against signed-32 bitwise ops.
      const bits = maskToBoolArray('80000000', 32);
      expect(bits[31]).toBe(true);
      expect(bits.slice(0, 31)).toEqual(new Array(31).fill(false));
    });

    it('returns all-false on garbage input', () => {
      expect(maskToBoolArray('not-a-number', 4)).toEqual([false, false, false, false]);
      expect(maskToBoolArray(-1, 4)).toEqual([false, false, false, false]);
      expect(maskToBoolArray(NaN, 4)).toEqual([false, false, false, false]);
    });
  });

  describe('boolArrayToHexString', () => {
    it('produces a plain uppercase hex string with no 0x prefix', () => {
      expect(boolArrayToHexString([false, true, false, true])).toBe('A');
      expect(boolArrayToHexString([true, true, true, true])).toBe('F');
      expect(boolArrayToHexString(new Array(32).fill(true))).toBe('FFFFFFFF');
    });

    it('returns "0" for the empty / all-false case', () => {
      expect(boolArrayToHexString([])).toBe('0');
      expect(boolArrayToHexString([false, false, false, false])).toBe('0');
    });
  });

  describe('boolArrayToNumber', () => {
    it('produces an integer 0..(2^width - 1)', () => {
      expect(boolArrayToNumber([false, true, false, true])).toBe(10);
      expect(boolArrayToNumber([true, true, true, true])).toBe(15);
      expect(boolArrayToNumber([])).toBe(0);
    });
  });

  describe('serializeMask', () => {
    it('selects encoding by tag', () => {
      const bits = [false, true, false, true];
      expect(serializeMask(bits, 'hex-string')).toBe('A');
      expect(serializeMask(bits, 'number')).toBe(10);
    });
  });

  describe('round-trip', () => {
    it('hex-string survives parse → serialize', () => {
      for (const hex of ['0', '1', 'A', 'FF', 'ABCD', 'FFFFFFFF']) {
        const bits = maskToBoolArray(hex, 32);
        expect(boolArrayToHexString(bits)).toBe(hex);
      }
    });

    it('number survives parse → serialize', () => {
      for (const n of [0, 1, 5, 10, 15]) {
        const bits = maskToBoolArray(n, 4);
        expect(boolArrayToNumber(bits)).toBe(n);
      }
    });
  });

  describe('formatMaskCompact', () => {
    it('renders (none) when no bits set', () => {
      expect(formatMaskCompact([false, false, false, false])).toBe('(none)');
      expect(formatMaskCompact([])).toBe('(none)');
    });

    it('renders single channels and runs', () => {
      const bits = (set: number[]): boolean[] => {
        const a = new Array(8).fill(false);
        for (const i of set) a[i] = true;
        return a;
      };
      expect(formatMaskCompact(bits([0]))).toBe('0');
      expect(formatMaskCompact(bits([0, 1, 2, 3]))).toBe('0-3');
      expect(formatMaskCompact(bits([0, 1, 2, 3, 7]))).toBe('0-3,7');
      expect(formatMaskCompact(bits([0, 3, 5, 7]))).toBe('0,3,5,7');
      expect(formatMaskCompact(bits([0, 1, 2, 3, 4, 5, 6, 7]))).toBe('0-7');
    });
  });
});
