import {
  Component,
  ElementRef,
  HostListener,
  TemplateRef,
  ViewChild,
  ViewContainerRef,
  computed,
  inject,
  input,
  output,
  signal,
} from '@angular/core';
import { CommonModule } from '@angular/common';
import { Overlay, OverlayRef } from '@angular/cdk/overlay';
import { TemplatePortal } from '@angular/cdk/portal';
import { take } from 'rxjs';

import {
  ChMaskEncoding,
  formatMaskCompact,
  maskToBoolArray,
  serializeMask,
} from '../../utils/ch-mask';

/**
 * Per-channel trigger-mask editor.
 *
 * Renders a compact cell showing the current selection (e.g. "0-3,7" or
 * "(none)"). Clicking opens a popover with one checkbox per channel plus
 * batch helpers (All / None / Self / Invert). Apply commits the mask via
 * `valueChange`; Cancel / outside-click / Escape discards.
 *
 * Used for both `ch_trigger_mask` (PSD2/PHA2/AMax, 32-bit hex) and
 * `coinc_mask` (PSD1/PHA1, 4-bit number) — `bitWidth` and `encoding`
 * inputs decide how the mask is parsed and serialized.
 *
 * The popover is rendered through `@angular/cdk/overlay` so it sits on
 * the CDK overlay container (a direct child of <body>) and is therefore
 * unaffected by table-level `overflow: auto` clipping or any
 * transform / filter that would otherwise turn `position: fixed` into a
 * containing-block-relative one.
 */
@Component({
  selector: 'app-ch-mask-editor',
  standalone: true,
  imports: [CommonModule],
  template: `
    <button
      type="button"
      class="mask-cell"
      [class.disabled]="disabled()"
      [disabled]="disabled()"
      (click)="toggle($event)"
      [title]="displayText()"
    >
      <span class="mask-text">{{ displayText() }}</span>
      <span class="edit-icon" aria-hidden="true">&#9998;</span>
    </button>

    <ng-template #popoverTpl>
      <div class="popover" (click)="$event.stopPropagation()">
        <div class="popover-header">Triggered by:</div>
        <div class="grid">
          @for (i of indices(); track i) {
            <label class="cb" [class.self-bit]="i === selfBit()">
              <input
                type="checkbox"
                [checked]="draft()[i]"
                (change)="toggleBit(i, $event)"
              />
              <span>{{ i }}</span>
            </label>
          }
        </div>
        <div class="actions">
          <button type="button" (click)="setAll(true)">All</button>
          <button type="button" (click)="setAll(false)">None</button>
          @if (selfBit() != null) {
            <button type="button" (click)="setSelfOnly()">Self</button>
          }
          <button type="button" (click)="invert()">Invert</button>
        </div>
        <div class="footer">
          <span class="preview">{{ draftPreview() }}</span>
          <span class="spacer"></span>
          <button type="button" (click)="cancel()">Cancel</button>
          <button type="button" class="primary" (click)="apply()">Apply</button>
        </div>
      </div>
    </ng-template>
  `,
  styles: `
    :host {
      display: inline-block;
    }
    .mask-cell {
      display: inline-flex;
      align-items: center;
      gap: 4px;
      min-width: 64px;
      max-width: 140px;
      padding: 2px 6px;
      border: 1px solid #ccc;
      border-radius: 3px;
      background: transparent;
      cursor: pointer;
      font-size: 12px;
    }
    .mask-cell:disabled,
    .mask-cell.disabled {
      cursor: not-allowed;
      opacity: 0.5;
    }
    .mask-cell:focus {
      outline: none;
      border-color: #1976d2;
    }
    .mask-text {
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
      flex: 1;
      text-align: left;
      font-family: monospace;
    }
    .edit-icon {
      font-size: 11px;
      color: #1976d2;
    }
    .popover {
      background: #fff;
      border: 1px solid #1976d2;
      border-radius: 4px;
      padding: 8px;
      box-shadow: 0 2px 8px rgba(0, 0, 0, 0.15);
      min-width: 280px;
      white-space: normal;
    }
    .popover-header {
      font-weight: 500;
      font-size: 12px;
      margin-bottom: 6px;
      color: #555;
    }
    .grid {
      display: grid;
      grid-template-columns: repeat(4, 1fr);
      gap: 2px 8px;
      max-height: 240px;
      overflow-y: auto;
    }
    .cb {
      display: inline-flex;
      align-items: center;
      gap: 4px;
      font-size: 12px;
      cursor: pointer;
    }
    .cb input {
      margin: 0;
    }
    .cb.self-bit span {
      font-weight: 700;
      color: #1976d2;
    }
    .actions {
      display: flex;
      gap: 4px;
      margin-top: 8px;
      padding-top: 6px;
      border-top: 1px dashed #e0e0e0;
    }
    .actions button {
      font-size: 11px;
      padding: 2px 8px;
      border: 1px solid #bbb;
      border-radius: 3px;
      background: #f5f5f5;
      cursor: pointer;
    }
    .actions button:hover {
      background: #e0e0e0;
    }
    .footer {
      display: flex;
      align-items: center;
      gap: 6px;
      margin-top: 8px;
      padding-top: 6px;
      border-top: 1px solid #e0e0e0;
    }
    .footer .preview {
      font-family: monospace;
      font-size: 11px;
      color: #666;
      flex: 1;
      overflow: hidden;
      text-overflow: ellipsis;
    }
    .footer .spacer {
      flex: 0;
    }
    .footer button {
      font-size: 12px;
      padding: 3px 10px;
      border: 1px solid #bbb;
      border-radius: 3px;
      background: #fff;
      cursor: pointer;
    }
    .footer button.primary {
      background: #1976d2;
      color: #fff;
      border-color: #1976d2;
    }
  `,
})
export class ChMaskEditorComponent {
  readonly value = input<string | number | null | undefined>(null);
  readonly bitWidth = input.required<number>();
  readonly encoding = input.required<ChMaskEncoding>();
  /** Number of channels this digitizer actually has (caps the checkbox grid). */
  readonly chCount = input.required<number>();
  /** Index of the row's own channel (used by the "Self only" helper). */
  readonly selfBit = input<number | null>(null);
  readonly disabled = input(false);

  readonly valueChange = output<string | number>();

  readonly open = signal(false);
  readonly draft = signal<boolean[]>([]);

  /** Limit checkbox count to whichever of bitWidth / chCount is smaller. */
  readonly effectiveLength = computed(() =>
    Math.min(this.bitWidth(), this.chCount()),
  );
  readonly indices = computed(() =>
    Array.from({ length: this.effectiveLength() }, (_, i) => i),
  );

  readonly displayText = computed(() => {
    const bits = this.open()
      ? this.draft()
      : maskToBoolArray(this.value(), this.effectiveLength());
    return formatMaskCompact(bits);
  });

  readonly draftPreview = computed(() => formatMaskCompact(this.draft()));

  @ViewChild('popoverTpl') popoverTpl!: TemplateRef<unknown>;

  private readonly host = inject(ElementRef<HTMLElement>);
  private readonly overlay = inject(Overlay);
  private readonly vcr = inject(ViewContainerRef);
  private overlayRef?: OverlayRef;

  toggle(ev: MouseEvent): void {
    ev.stopPropagation();
    if (this.disabled()) return;
    if (this.open()) {
      this.cancel();
      return;
    }
    this.draft.set(maskToBoolArray(this.value(), this.effectiveLength()));

    const positionStrategy = this.overlay
      .position()
      .flexibleConnectedTo(this.host)
      .withPositions([
        // Preferred: below, left-aligned with the cell.
        { originX: 'start', originY: 'bottom', overlayX: 'start', overlayY: 'top', offsetY: 4 },
        // Fallback: above when there's no room below.
        { originX: 'start', originY: 'top', overlayX: 'start', overlayY: 'bottom', offsetY: -4 },
        // Right-anchored variants for cells near the viewport's right edge.
        { originX: 'end', originY: 'bottom', overlayX: 'end', overlayY: 'top', offsetY: 4 },
        { originX: 'end', originY: 'top', overlayX: 'end', overlayY: 'bottom', offsetY: -4 },
      ])
      .withPush(true)
      .withViewportMargin(8);

    this.overlayRef = this.overlay.create({
      positionStrategy,
      scrollStrategy: this.overlay.scrollStrategies.reposition(),
      hasBackdrop: true,
      backdropClass: 'cdk-overlay-transparent-backdrop',
    });

    this.overlayRef
      .backdropClick()
      .pipe(take(1))
      .subscribe(() => this.cancel());
    this.overlayRef.keydownEvents().subscribe((event) => {
      if (event.key === 'Escape') this.cancel();
    });

    this.overlayRef.attach(new TemplatePortal(this.popoverTpl, this.vcr));
    this.open.set(true);
  }

  toggleBit(i: number, ev: Event): void {
    const checked = (ev.target as HTMLInputElement).checked;
    const next = [...this.draft()];
    next[i] = checked;
    this.draft.set(next);
  }

  setAll(state: boolean): void {
    this.draft.set(new Array(this.effectiveLength()).fill(state));
  }

  setSelfOnly(): void {
    const self = this.selfBit();
    if (self == null) return;
    const next = new Array(this.effectiveLength()).fill(false);
    if (self < next.length) next[self] = true;
    this.draft.set(next);
  }

  invert(): void {
    this.draft.set(this.draft().map((b) => !b));
  }

  apply(): void {
    this.valueChange.emit(serializeMask(this.draft(), this.encoding()));
    this.close();
  }

  cancel(): void {
    this.close();
  }

  private close(): void {
    if (this.overlayRef) {
      this.overlayRef.dispose();
      this.overlayRef = undefined;
    }
    this.open.set(false);
  }

  @HostListener('document:keydown.escape')
  onEscape(): void {
    if (this.open()) this.cancel();
  }
}
