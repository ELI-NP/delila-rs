import { Component, EventEmitter, Input, Output } from '@angular/core';
import { REBIN_FACTORS } from '../../utils/rebin';

/**
 * Rebin choice ×1 / ×2 / ×4 / ×8 (TODO 87), for the View tab and Setup.
 * A factor the shown range cannot split evenly is disabled.
 */
@Component({
  selector: 'app-rebin-picker',
  standalone: true,
  template: `
    <span class="rebin-picker" role="group" [attr.aria-label]="label + ' rebin'">
      <span class="rebin-label">{{ label }}</span>
      @for (f of factors; track f) {
        <button
          type="button"
          class="rebin-btn"
          [attr.data-testid]="'rebin-' + f"
          [class.active]="value === f"
          [disabled]="!allowed.includes(f)"
          [title]="titleFor(f)"
          (click)="pick(f)"
        >
          ×{{ f }}
        </button>
      }
    </span>
  `,
  styles: `
    .rebin-picker {
      display: inline-flex;
      align-items: center;
      gap: 2px;
    }
    .rebin-label {
      font-size: 12px;
      margin-right: 4px;
      opacity: 0.8;
    }
    .rebin-btn {
      min-width: 32px;
      padding: 2px 6px;
      font-size: 12px;
      border: 1px solid rgba(0, 0, 0, 0.25);
      background: white;
      border-radius: 4px;
      cursor: pointer;
    }
    .rebin-btn.active {
      background: #3f51b5;
      border-color: #3f51b5;
      color: white;
    }
    .rebin-btn:disabled {
      opacity: 0.35;
      cursor: default;
    }
  `,
})
export class RebinPickerComponent {
  @Input() label = 'Rebin';
  @Input() value = 1;
  @Input() allowed: readonly number[] = REBIN_FACTORS;
  /** Native bins in the shown range, for the tooltip (null = not known yet). */
  @Input() nativeBins: number | null = null;
  @Output() valueChange = new EventEmitter<number>();

  readonly factors = REBIN_FACTORS;

  titleFor(f: number): string {
    const per = `${f} native bin${f === 1 ? '' : 's'} per bin`;
    return this.nativeBins ? `${this.nativeBins / f} bins, ${per}` : per;
  }

  pick(f: number): void {
    if (f !== this.value) this.valueChange.emit(f);
  }
}
