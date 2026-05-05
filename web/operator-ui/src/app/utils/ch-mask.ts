/**
 * Helpers for the per-channel "trigger mask" UI.
 *
 * Two concrete parameters share this concept:
 *   - PSD2 / PHA2 / AMax `ch_trigger_mask`  → DevTree STRING (hex), 32-bit
 *   - PSD1 / PHA1 `coinc_mask`              → DevTree NUMBER, 4-bit (0..15)
 *
 * Internally the editor works with `boolean[]` indexed by channel.
 * Serialization is delegated to `serializeMask()` based on `encoding`.
 *
 * Math.floor(n / 2**i) % 2 is used for bit extraction so we stay clear of
 * JavaScript's signed 32-bit bitwise ops (bit 31 would otherwise come back
 * negative on `n & (1 << 31)`).
 */

export type ChMaskEncoding = 'hex-string' | 'number';

export function maskToBoolArray(
  value: string | number | null | undefined,
  bitWidth: number,
): boolean[] {
  const bits = new Array<boolean>(bitWidth).fill(false);
  if (value == null) return bits;

  let n: number;
  if (typeof value === 'string') {
    const s = value.trim();
    if (s === '') return bits;
    n = parseInt(s.replace(/^0x/i, ''), 16);
  } else {
    n = value;
  }
  if (!Number.isFinite(n) || Number.isNaN(n) || n < 0) return bits;

  for (let i = 0; i < bitWidth; i++) {
    bits[i] = Math.floor(n / 2 ** i) % 2 === 1;
  }
  return bits;
}

export function boolArrayToHexString(bits: boolean[]): string {
  let n = 0;
  for (let i = 0; i < bits.length; i++) {
    if (bits[i]) n += 2 ** i;
  }
  return n.toString(16).toUpperCase();
}

export function boolArrayToNumber(bits: boolean[]): number {
  let n = 0;
  for (let i = 0; i < bits.length; i++) {
    if (bits[i]) n += 2 ** i;
  }
  return n;
}

export function serializeMask(
  bits: boolean[],
  encoding: ChMaskEncoding,
): string | number {
  return encoding === 'hex-string'
    ? boolArrayToHexString(bits)
    : boolArrayToNumber(bits);
}

/** Compact human-readable form: "0-3,7" / "(none)". */
export function formatMaskCompact(bits: boolean[]): string {
  const idx: number[] = [];
  for (let i = 0; i < bits.length; i++) {
    if (bits[i]) idx.push(i);
  }
  if (idx.length === 0) return '(none)';

  const runs: string[] = [];
  let start = idx[0];
  let prev = idx[0];
  for (let k = 1; k < idx.length; k++) {
    if (idx[k] === prev + 1) {
      prev = idx[k];
    } else {
      runs.push(start === prev ? `${start}` : `${start}-${prev}`);
      start = idx[k];
      prev = idx[k];
    }
  }
  runs.push(start === prev ? `${start}` : `${start}-${prev}`);
  return runs.join(',');
}
