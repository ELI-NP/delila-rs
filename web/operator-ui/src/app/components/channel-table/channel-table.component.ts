import {
  Component,
  input,
  output,
  computed,
} from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { ChMaskEditorComponent } from '../ch-mask-editor/ch-mask-editor.component';

/**
 * Definition of a channel parameter (one row in the table)
 */
export interface ChannelParamDef {
  /** Key in ChannelConfig (e.g., 'trigger_threshold') */
  key: string;
  /** Display label (e.g., 'Threshold') */
  label: string;
  /** Input type */
  type: 'number' | 'enum' | 'boolean' | 'ch-mask';
  /** Options for enum type (e.g., ['Positive', 'Negative']) */
  options?: string[];
  /** Unit label (e.g., 'ns', '%', 'ADC') */
  unit?: string;
  /** Min value for number type */
  min?: number;
  /** Max value for number type */
  max?: number;
  /** Step increment for number type (from DevTree). Used for on-blur snapping. */
  step?: number;
  /** If true, parameter can be changed while DAQ is Running */
  setInRun?: boolean;
  /** Optional hover tooltip (e.g. FW register address for AMax). */
  tooltip?: string;
  /** Width of the bit mask (only meaningful when type === 'ch-mask'). */
  bitWidth?: number;
  /** Wire-format encoding for ch-mask values: hex string (PSD2/PHA2/AMax)
   *  or plain number (PSD1/PHA1). */
  encoding?: 'hex-string' | 'number';
}

/**
 * Emitted when the "All" (default) column value changes
 */
export interface DefaultValueChange {
  key: string;
  value: unknown;
}

/**
 * Emitted when a specific channel's value changes
 */
export interface ChannelValueChange {
  channel: number;
  key: string;
  value: unknown;
}

/**
 * Reusable channel parameter table component.
 *
 * Displays parameters as rows, channels as columns.
 * The leftmost columns (Parameter name + All) are sticky.
 * Cells that differ from the "All" column are highlighted.
 *
 * Usage:
 * ```html
 * <app-channel-table
 *   [params]="frequentParams"
 *   [numChannels]="32"
 *   [defaultValues]="channelDefaults"
 *   [channelValues]="expandedChannelValues"
 *   (defaultChange)="onDefaultChange($event)"
 *   (channelChange)="onChannelChange($event)"
 * />
 * ```
 */
@Component({
  selector: 'app-channel-table',
  standalone: true,
  imports: [CommonModule, FormsModule, ChMaskEditorComponent],
  template: `
    <div class="channel-table-wrapper">
      <table class="channel-table">
        <thead>
          <tr>
            <th class="sticky-col param-header">Parameter</th>
            <th class="sticky-col all-header">All</th>
            @for (ch of channelIndices(); track ch) {
              <th class="ch-header">{{ ch }}</th>
            }
          </tr>
        </thead>
        <tbody>
          @for (param of params(); track param.key) {
            <tr [class.disabled-row]="isDisabled(param.key)">
              <td class="sticky-col param-cell" [attr.title]="param.tooltip || null">
                {{ param.label }}
                @if (param.unit) {
                  <span class="unit">({{ param.unit }})</span>
                }
                @if (param.tooltip) {
                  <span class="info-icon" aria-hidden="true">ⓘ</span>
                }
              </td>
              <td class="sticky-col all-cell">
                @switch (param.type) {
                  @case ('number') {
                    <input
                      type="number"
                      class="cell-input"
                      [value]="getDefault(param.key)"
                      [min]="param.min"
                      [max]="param.max"
                      [step]="param.step ?? 'any'"
                      [title]="rangeHint(param)"
                      [disabled]="isDisabled(param.key)"
                      (change)="onDefaultInput(param.key, $event)"
                      (blur)="snapValue($event, param, null)"
                    />
                  }
                  @case ('enum') {
                    <select
                      class="cell-select"
                      [ngModel]="getDefault(param.key)"
                      [disabled]="isDisabled(param.key)"
                      (ngModelChange)="defaultChange.emit({ key: param.key, value: $event })"
                    >
                      @for (opt of param.options ?? []; track opt) {
                        <option [value]="opt">{{ opt }}</option>
                      }
                    </select>
                  }
                  @case ('boolean') {
                    <select
                      class="cell-select"
                      [ngModel]="getDefault(param.key) ?? 'True'"
                      [disabled]="isDisabled(param.key)"
                      (ngModelChange)="defaultChange.emit({ key: param.key, value: $event })"
                    >
                      <option value="True">ON</option>
                      <option value="False">OFF</option>
                    </select>
                  }
                  @case ('ch-mask') {
                    <app-ch-mask-editor
                      [value]="$any(getDefault(param.key))"
                      [bitWidth]="param.bitWidth ?? 32"
                      [encoding]="param.encoding ?? 'hex-string'"
                      [chCount]="numChannels()"
                      [disabled]="isDisabled(param.key)"
                      (valueChange)="defaultChange.emit({ key: param.key, value: $event })"
                    />
                  }
                }
              </td>
              @for (ch of channelIndices(); track ch) {
                <td
                  class="ch-cell"
                  [class.override]="isOverride(ch, param.key)"
                >
                  @switch (param.type) {
                    @case ('number') {
                      <input
                        type="number"
                        class="cell-input"
                        [value]="getChannel(ch, param.key)"
                        [min]="param.min"
                        [max]="param.max"
                        [step]="param.step ?? 'any'"
                        [title]="rangeHint(param)"
                        [disabled]="isDisabled(param.key)"
                        (change)="onChannelInput(ch, param.key, $event)"
                        (blur)="snapValue($event, param, ch)"
                      />
                    }
                    @case ('enum') {
                      <select
                        class="cell-select"
                        [ngModel]="getChannel(ch, param.key)"
                        [disabled]="isDisabled(param.key)"
                        (ngModelChange)="channelChange.emit({ channel: ch, key: param.key, value: $event })"
                      >
                        @for (opt of param.options ?? []; track opt) {
                          <option [value]="opt">{{ opt }}</option>
                        }
                      </select>
                    }
                    @case ('boolean') {
                      <select
                        class="cell-select"
                        [ngModel]="getChannel(ch, param.key) ?? 'True'"
                        [disabled]="isDisabled(param.key)"
                        (ngModelChange)="channelChange.emit({ channel: ch, key: param.key, value: $event })"
                      >
                        <option value="True">ON</option>
                        <option value="False">OFF</option>
                      </select>
                    }
                    @case ('ch-mask') {
                      <app-ch-mask-editor
                        [value]="$any(getChannel(ch, param.key))"
                        [bitWidth]="param.bitWidth ?? 32"
                        [encoding]="param.encoding ?? 'hex-string'"
                        [chCount]="numChannels()"
                        [selfBit]="ch"
                        [disabled]="isDisabled(param.key)"
                        (valueChange)="channelChange.emit({ channel: ch, key: param.key, value: $event })"
                      />
                    }
                  }
                </td>
              }
            </tr>
          }
        </tbody>
      </table>
    </div>
  `,
  styles: `
    .channel-table-wrapper {
      overflow-x: auto;
      max-width: 100%;
      border: 1px solid #e0e0e0;
      border-radius: 4px;
    }

    .channel-table {
      border-collapse: separate;
      border-spacing: 0;
      font-size: 13px;
      white-space: nowrap;
    }

    th, td {
      padding: 4px 6px;
      border-bottom: 1px solid #e0e0e0;
      border-right: 1px solid #f0f0f0;
    }

    thead th {
      background: #fafafa;
      font-weight: 500;
      text-align: center;
      position: sticky;
      top: 0;
      z-index: 1;
    }

    /* Sticky columns: Parameter name + All */
    .sticky-col {
      position: sticky;
      z-index: 2;
      background: #fff;
    }

    thead .sticky-col {
      z-index: 3;
      background: #fafafa;
    }

    .param-header, .param-cell {
      left: 0;
      min-width: 120px;
      max-width: 160px;
      font-weight: 500;
      border-right: 2px solid #e0e0e0;
    }

    .all-header, .all-cell {
      left: 120px;
      min-width: 80px;
      border-right: 2px solid #1976d2;
      background: #e3f2fd;
      box-shadow: 4px 0 6px rgba(0, 0, 0, 0.08);
    }

    thead .all-header {
      background: #bbdefb;
      font-weight: 600;
      color: #1565c0;
    }

    .ch-header {
      min-width: 72px;
      text-align: center;
    }

    .ch-cell {
      text-align: center;
    }

    /* Highlight overridden cells */
    .ch-cell.override {
      background-color: #fff3e0;
    }

    .ch-cell.override .cell-input,
    .ch-cell.override .cell-select {
      background-color: #fff3e0;
    }

    .unit {
      font-size: 11px;
      color: #999;
      margin-left: 2px;
    }

    .info-icon {
      font-size: 11px;
      color: #1976d2;
      margin-left: 4px;
      cursor: help;
    }

    .param-cell[title] {
      cursor: help;
    }

    /* Compact inputs */
    .cell-input {
      width: 64px;
      padding: 2px 4px;
      border: 1px solid #ccc;
      border-radius: 3px;
      font-size: 13px;
      text-align: center;
      background: transparent;
    }

    .cell-input:focus {
      outline: none;
      border-color: #1976d2;
    }

    /* Out-of-range / non-step input gets clamped on blur — flash the
     * background and outline so the user sees the auto-fix instead of
     * silently watching their typed value mutate. ~1 s, then back to
     * normal. */
    @keyframes clamped-flash {
      0%   { background-color: #fff3cd; box-shadow: 0 0 0 2px #f0ad4e inset; }
      80%  { background-color: #fff3cd; box-shadow: 0 0 0 2px #f0ad4e inset; }
      100% { background-color: transparent; box-shadow: none; }
    }
    .cell-input.clamped-flash {
      animation: clamped-flash 1s ease-out;
    }

    .cell-select {
      width: 72px;
      padding: 2px;
      border: 1px solid #ccc;
      border-radius: 3px;
      font-size: 12px;
      background: transparent;
      cursor: pointer;
    }

    .cell-select:focus {
      outline: none;
      border-color: #1976d2;
    }

    /* Zebra striping — exclude sticky columns to keep opaque backgrounds */
    tbody tr:nth-child(even) td.ch-cell:not(.override) {
      background-color: #fafafa;
    }

    tbody tr:hover td.ch-cell {
      background-color: #f5f5f5;
    }

    tbody tr:hover td.param-cell {
      background-color: #f5f5f5;
    }

    tbody tr:hover td.all-cell {
      background-color: #e3f2fd;
    }

    tbody tr:hover td.ch-cell.override {
      background-color: #ffe0b2;
    }

    /* Disabled row (non-SetInRun params during Running) */
    tr.disabled-row td {
      opacity: 0.45;
    }

    tr.disabled-row .cell-input,
    tr.disabled-row .cell-select {
      cursor: not-allowed;
    }
  `,
})
export class ChannelTableComponent {
  /** Parameter definitions (one per row) */
  readonly params = input.required<ChannelParamDef[]>();
  /** Number of channels */
  readonly numChannels = input.required<number>();
  /** Default values (the "All" column) — keyed by param.key */
  readonly defaultValues = input.required<Record<string, unknown>>();
  /** Per-channel values — array of length numChannels, each keyed by param.key */
  readonly channelValues = input.required<Record<string, unknown>[]>();
  /** Keys of parameters that should be disabled (e.g., non-SetInRun params during Running) */
  readonly disabledKeys = input<string[]>([]);
  /** Optional filter: only show these channel indices. null = show all channels. */
  readonly visibleChannels = input<number[] | null>(null);

  /** Emitted when a value in the "All" column changes */
  readonly defaultChange = output<DefaultValueChange>();
  /** Emitted when a specific channel value changes */
  readonly channelChange = output<ChannelValueChange>();

  /** Array of channel indices to display (filtered by visibleChannels if set) */
  readonly channelIndices = computed(() => {
    const visible = this.visibleChannels();
    if (visible != null) {
      return visible;
    }
    return Array.from({ length: this.numChannels() }, (_, i) => i);
  });

  /** Get the default (All column) value for a parameter */
  getDefault(key: string): unknown {
    return this.defaultValues()[key];
  }

  /** Get a channel's value for a parameter */
  getChannel(ch: number, key: string): unknown {
    const values = this.channelValues();
    return values[ch]?.[key];
  }

  /** Check if a parameter is disabled */
  isDisabled(key: string): boolean {
    return this.disabledKeys().includes(key);
  }

  /** Check if a channel value differs from the All column */
  isOverride(ch: number, key: string): boolean {
    const defaultVal = this.defaultValues()[key];
    const chVal = this.channelValues()[ch]?.[key];
    // Both undefined/null → not override
    if (defaultVal == null && chVal == null) return false;
    return defaultVal !== chVal;
  }

  /** Handle number input change in the All column */
  onDefaultInput(key: string, event: Event): void {
    const input = event.target as HTMLInputElement;
    const value = input.value === '' ? undefined : Number(input.value);
    this.defaultChange.emit({ key, value });
  }

  /** Handle select change in the All column */
  onDefaultSelect(key: string, event: Event): void {
    const select = event.target as HTMLSelectElement;
    this.defaultChange.emit({ key, value: select.value });
  }

  /** Handle number input change in a channel column */
  onChannelInput(ch: number, key: string, event: Event): void {
    const input = event.target as HTMLInputElement;
    const value = input.value === '' ? undefined : Number(input.value);
    this.channelChange.emit({ channel: ch, key, value });
  }

  /**
   * Snap a number input to the nearest valid step on blur (CoMPASS-style
   * round-to-nearest) and clamp to [min, max]. When the value the user
   * typed differs from the clamped/snapped value:
   *   1. Update the visible input value (so they see what was actually
   *      kept).
   *   2. Re-emit the change event so the parent's model also reflects
   *      the clamped value — without this the bound model would still
   *      hold the user-typed 20000 and Apply would send that to the
   *      backend, which would clamp it server-side and silently
   *      overwrite the UI on the next config refresh.
   *   3. Briefly flash the cell so the operator notices the auto-fix.
   *
   * `ch === null` means the "All" column (default-row); a numeric `ch`
   * is a per-channel override cell.
   */
  snapValue(event: Event, param: ChannelParamDef, ch: number | null): void {
    if (!param.step || param.min == null) return;
    const el = event.target as HTMLInputElement;
    if (el.value === '') return;
    const value = Number(el.value);
    if (isNaN(value)) return;
    const snapped = Math.round((value - param.min) / param.step) * param.step + param.min;
    const clamped = Math.min(Math.max(snapped, param.min), param.max ?? Infinity);
    if (clamped === value) return;

    el.value = String(clamped);
    if (ch === null) {
      this.defaultChange.emit({ key: param.key, value: clamped });
    } else {
      this.channelChange.emit({ channel: ch, key: param.key, value: clamped });
    }
    // CSS animation hook — auto-removed after the keyframe finishes.
    el.classList.remove('clamped-flash');
    // Force reflow so re-adding the class restarts the animation.
    void el.offsetWidth;
    el.classList.add('clamped-flash');
  }

  /** Tooltip text shown on hover ("Range: 32 – 16200, step 8") so the
   * valid range is visible without diving into the docs. */
  rangeHint(param: ChannelParamDef): string {
    const parts: string[] = [];
    if (param.min != null && param.max != null) {
      parts.push(`Range: ${param.min} – ${param.max}`);
    } else if (param.min != null) {
      parts.push(`Min: ${param.min}`);
    } else if (param.max != null) {
      parts.push(`Max: ${param.max}`);
    }
    if (param.step != null) {
      parts.push(`step ${param.step}`);
    }
    if (param.unit) {
      parts.push(`(${param.unit})`);
    }
    return parts.join(', ');
  }

  /** Handle select change in a channel column */
  onChannelSelect(ch: number, key: string, event: Event): void {
    const select = event.target as HTMLSelectElement;
    this.channelChange.emit({ channel: ch, key, value: select.value });
  }
}
