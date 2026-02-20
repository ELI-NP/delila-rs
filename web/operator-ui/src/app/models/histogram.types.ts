function generateUUID(): string {
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') {
    return generateUUID();
  }
  // Fallback for non-secure contexts (HTTP)
  return '10000000-1000-4000-8000-100000000000'.replace(/[018]/g, (c) =>
    (
      +c ^
      (crypto.getRandomValues(new Uint8Array(1))[0] & (15 >> (+c / 4)))
    ).toString(16),
  );
}

// Histogram configuration
export interface HistogramConfig {
  num_bins: number;
  min_value: number;
  max_value: number;
}

// Single histogram data
export interface Histogram1D {
  module_id: number;
  channel_id: number;
  config: HistogramConfig;
  bins: number[];
  total_counts: number;
  overflow: number;
  underflow: number;
}

// 2D Histogram data (Energy vs PSD)
export interface Histogram2D {
  module_id: number;
  channel_id: number;
  x_config: HistogramConfig;
  y_config: HistogramConfig;
  bins: number[]; // flat array: bins[y * x_bins + x]
  total_counts: number;
  overflow: number;
}

// Channel summary for list response
export interface ChannelSummary {
  module_id: number;
  channel_id: number;
  total_counts: number;
  name?: string;
}

// Response from GET /api/histograms
export interface HistogramListResponse {
  total_events: number;
  elapsed_secs: number;
  event_rate: number;
  channels: ChannelSummary[];
}

// Response from GET /api/status
export interface MonitorStatusResponse {
  state: string;
  total_events: number;
  num_channels: number;
  elapsed_secs: number;
  event_rate: number;
}

// Channel identifier
export interface ChannelKey {
  moduleId: number;
  channelId: number;
}

// Helper to create channel key string for Maps
export function channelKeyString(moduleId: number, channelId: number): string {
  return `${moduleId}:${channelId}`;
}

// Helper to parse channel key string
export function parseChannelKey(key: string): ChannelKey {
  const [moduleId, channelId] = key.split(':').map(Number);
  return { moduleId, channelId };
}

// Monitor state for localStorage persistence
export interface MonitorState {
  setupConfig: SetupConfig;
  viewTabs: ViewTab[];
  activeTabId: string | null; // null = Setup tab is active
}

// X-axis label type
export type XAxisLabel = 'Channel' | 'keV' | 'MeV';

// Histogram type for tab-level selection
export type HistogramType = 'energy' | 'psd' | 'psd2d';

// Setup tab configuration (template for creating views)
export interface SetupConfig {
  name: string;
  gridRows: number;
  gridCols: number;
  xAxisLabel: XAxisLabel;
  histogramType: HistogramType;
  cells: SetupCell[];
}

// Setup cell - only channel assignment, no runtime state
export interface SetupCell {
  sourceId: number | null;
  channelId: number | null;
}

// View tab - created from setup, read-only layout
export interface ViewTab {
  id: string;
  name: string;
  gridRows: number;
  gridCols: number;
  xAxisLabel: XAxisLabel;
  histogramType?: HistogramType; // optional for backward compat (default: 'energy')
  cells: ViewCell[];
  lastModifiedCellIndex?: number;
}

// View cell - has runtime state for display
export interface ViewCell {
  sourceId: number;
  channelId: number;
  xRange: { min: number; max: number } | 'auto';
  yRange: { min: number; max: number } | 'auto';
  isLocked: boolean;
  isEmpty: boolean; // true for placeholder cells in grid
  logScale?: boolean; // Y-axis log scale
  fitResult?: ViewCellFitResult; // Gaussian fit result
  fitRange?: { min: number; max: number }; // Range used for fitting
}

// Simplified fit result for ViewCell (serializable to localStorage)
export interface ViewCellFitResult {
  center: number;
  centerError: number;
  sigma: number;
  sigmaError: number;
  fwhm: number;
  netArea: number;
  netAreaError: number;
  chi2: number;
  ndf: number;
  // Background lines for drawing
  leftLine: { slope: number; intercept: number };
  rightLine: { slope: number; intercept: number };
  bgLine: { slope: number; intercept: number };
  // Gaussian parameters for drawing
  amplitude: number;
}

// Legacy type for backward compatibility during migration
export interface HistogramCell {
  sourceId: number | null;
  channelId: number | null;
  xRange: { min: number; max: number } | 'auto';
  yRange: { min: number; max: number } | 'auto';
  isLocked: boolean;
}

// Legacy type
export interface MonitorTab {
  id: string;
  name: string;
  gridRows: number;
  gridCols: number;
  cells: HistogramCell[];
}

// Gaussian fit result (for Phase 6)
export interface GaussianFitResult {
  amplitude: number;
  center: number;
  sigma: number;
  leftLine: { slope: number; intercept: number };
  rightLine: { slope: number; intercept: number };
  bgLine: { slope: number; intercept: number };
  fwhm: number;
  area: number;
  chi2: number;
}

// Create default setup cell
export function createDefaultSetupCell(): SetupCell {
  return {
    sourceId: null,
    channelId: null,
  };
}

// Create default setup config
export function createDefaultSetupConfig(): SetupConfig {
  return {
    name: '',
    gridRows: 2,
    gridCols: 2,
    xAxisLabel: 'Channel',
    histogramType: 'energy',
    cells: Array(4)
      .fill(null)
      .map(() => createDefaultSetupCell()),
  };
}

// Create view tab from setup config
export function createViewTabFromSetup(setup: SetupConfig): ViewTab | null {
  // Filter out empty cells
  const validCells = setup.cells.filter(
    (cell): cell is { sourceId: number; channelId: number } =>
      cell.sourceId !== null && cell.channelId !== null
  );

  if (validCells.length === 0) {
    return null; // No valid cells to display
  }

  const rows = setup.gridRows;
  const cols = setup.gridCols;

  return {
    id: generateUUID(),
    name: setup.name || `View ${Date.now()}`,
    gridRows: rows,
    gridCols: cols,
    xAxisLabel: setup.xAxisLabel,
    histogramType: setup.histogramType,
    cells: setup.cells.map((cell) => ({
      sourceId: cell.sourceId ?? 0,
      channelId: cell.channelId ?? 0,
      xRange: 'auto' as const,
      yRange: 'auto' as const,
      isLocked: false,
      isEmpty: cell.sourceId === null,
    })),
  };
}

// Create default monitor state
export function createDefaultMonitorState(): MonitorState {
  return {
    setupConfig: createDefaultSetupConfig(),
    viewTabs: [],
    activeTabId: null,
  };
}

// =============================================================================
// Waveform Types
// =============================================================================

// Waveform data from backend
export interface Waveform {
  analog_probe1: number[];
  analog_probe2: number[];
  digital_probe1: number[]; // Packed bits
  digital_probe2: number[];
  digital_probe3: number[];
  digital_probe4: number[];
  time_resolution: number;
  trigger_threshold: number;
  ns_per_sample?: number;
}

// Latest waveform response
export interface LatestWaveform {
  module_id: number;
  channel_id: number;
  energy: number;
  timestamp_ns: number;
  waveform: Waveform;
}

// Waveform list response
export interface WaveformListResponse {
  channels: WaveformChannelInfo[];
}

export interface WaveformChannelInfo {
  module_id: number;
  channel_id: number;
  name?: string;
}

// Legacy helpers (for backward compatibility)
export function createDefaultCell(): HistogramCell {
  return {
    sourceId: null,
    channelId: null,
    xRange: 'auto',
    yRange: 'auto',
    isLocked: false,
  };
}

export function createDefaultTab(name: string): MonitorTab {
  return {
    id: generateUUID(),
    name,
    gridRows: 2,
    gridCols: 2,
    cells: Array(4)
      .fill(null)
      .map(() => createDefaultCell()),
  };
}
