/**
 * Pure utility functions for ECharts option construction.
 *
 * R-P2 (Phase 1 refactor sprint 2026-Q2). Centralises the small overlap
 * between `histogram-chart` (538 LoC) and `heatmap-chart` (269 LoC) so
 * future chart components reuse one source of truth instead of copy-paste.
 *
 * Architecture choice (confirmed by user): pure utility functions, **not**
 * a `BaseChartComponent` superclass. Angular signals/computed flow stays
 * in the per-chart component; only the static option fragments live here.
 */

import type { EChartsCoreOption } from 'echarts/core';

/**
 * Build an axis-label formatter that abbreviates large counts with
 * SI suffixes (k / M / G).
 *
 * - `linear` mode: signed magnitudes, integer for the unit-less range
 *   (matches histogram-chart linear-Y formatter).
 * - `log`    mode: 0 special-cases as "0"; suffixes use 0-decimal precision
 *   (matches histogram-chart log-Y formatter).
 *
 * Pulled out so the histogram component stops carrying two near-identical
 * lambdas inline. Heatmap uses category axes today so it does not consume
 * this yet — but the utility is here for any future chart.
 */
export function siCountFormatter(scale: 'linear' | 'log'): (value: number) => string {
  if (scale === 'log') {
    return (value: number) => {
      if (value === 0) return '0';
      if (value >= 1e9) return (value / 1e9).toFixed(0) + 'G';
      if (value >= 1e6) return (value / 1e6).toFixed(0) + 'M';
      if (value >= 1e3) return (value / 1e3).toFixed(0) + 'k';
      return value.toString();
    };
  }
  return (value: number) => {
    if (value === 0) return '0';
    const abs = Math.abs(value);
    if (abs >= 1e9) return (value / 1e9).toFixed(1) + 'G';
    if (abs >= 1e6) return (value / 1e6).toFixed(1) + 'M';
    if (abs >= 1e3) return (value / 1e3).toFixed(0) + 'k';
    return Math.floor(value).toString();
  };
}

/**
 * Common ECharts `grid` configuration with sensible default margins.
 *
 * Each chart component still passes its own override fragment to leave
 * space for axis labels / visualMap, but the defaults below are what
 * `histogram-chart` and `heatmap-chart` agree on for a chart that fills
 * its parent container without clipping axis names.
 *
 * Returns a fresh object on every call so callers can safely mutate the
 * returned fragment without leaking state across chart instances.
 */
export function defaultGrid(
  override?: Partial<{ top: number; right: number; bottom: number; left: number }>,
): EChartsCoreOption['grid'] {
  return {
    top: 30,
    right: 60,
    bottom: 60,
    left: 60,
    ...(override ?? {}),
  };
}

/**
 * Common `dataZoom` for charts that want both axes scrollable via the
 * mouse-wheel inside the plot, plus a slider on each axis edge.
 *
 * Mirrors `heatmap-chart`'s 4-array literal. The histogram chart uses a
 * single inside zoom on the X axis and is *not* a target of this helper.
 */
export function buildDualSliderDataZoom(): EChartsCoreOption['dataZoom'] {
  return [
    { type: 'inside', xAxisIndex: 0 },
    { type: 'inside', yAxisIndex: 0 },
    { type: 'slider', xAxisIndex: 0, bottom: 5, height: 15 },
    { type: 'slider', yAxisIndex: 0, left: 5, width: 15 },
  ];
}
