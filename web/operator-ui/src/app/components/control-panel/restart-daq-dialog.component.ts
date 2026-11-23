import { Component, inject, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { MatButtonModule } from '@angular/material/button';
import { MAT_DIALOG_DATA, MatDialogModule } from '@angular/material/dialog';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatIconModule } from '@angular/material/icon';
import { MatInputModule } from '@angular/material/input';

/** One precondition as the page sees it (the Operator checks them again). */
export interface RestartCheckItem {
  label: string;
  ok: boolean;
}

export interface RestartDaqDialogData {
  checks: RestartCheckItem[];
}

/** The word to type before the restart is sent (TODO 91). */
export const RESTART_CONFIRM_WORD = 'RESTART';

/**
 * Confirmation for "Restart DAQ" (TODO 91): what happens, the preconditions,
 * and a typed confirmation. Closes with `true` only when confirmed.
 */
@Component({
  selector: 'app-restart-daq-dialog',
  standalone: true,
  imports: [MatDialogModule, MatButtonModule, MatFormFieldModule, MatInputModule, MatIconModule, FormsModule],
  template: `
    <h2 mat-dialog-title>Restart the DAQ</h2>
    <mat-dialog-content>
      <p>
        Every DAQ component stops and starts again with the same config (the Operator too:
        this page loses the connection for up to a minute and reconnects by itself). Use it
        between runs, e.g. when a digitizer cannot be re-opened.
      </p>
      <p>The Operator refuses unless all of these hold (it checks them itself):</p>
      <ul class="checks">
        @for (c of data.checks; track c.label) {
          <li [class.bad]="!c.ok" [attr.data-testid]="c.ok ? 'check-ok' : 'check-bad'">
            <mat-icon>{{ c.ok ? 'check_circle' : 'cancel' }}</mat-icon>
            <span>{{ c.label }}</span>
          </li>
        }
      </ul>
      <p class="hint">
        A restart does not fix hardware (clock cable, crate). If the same problem comes back,
        check the hardware before restarting again.
      </p>
      <mat-form-field appearance="outline" class="confirm">
        <mat-label>Type {{ word }} to confirm</mat-label>
        <input
          matInput
          data-testid="restart-confirm-input"
          autocomplete="off"
          [ngModel]="typed()"
          (ngModelChange)="typed.set($event)"
        />
      </mat-form-field>
    </mat-dialog-content>
    <mat-dialog-actions align="end">
      <button mat-button mat-dialog-close>Cancel</button>
      <button
        mat-flat-button
        color="warn"
        data-testid="restart-confirm-button"
        [disabled]="typed() !== word"
        [mat-dialog-close]="true"
      >
        Restart DAQ
      </button>
    </mat-dialog-actions>
  `,
  styles: `
    .checks {
      list-style: none;
      padding: 0;
      margin: 8px 0;
    }
    .checks li {
      display: flex;
      align-items: center;
      gap: 6px;
      color: var(--mat-sys-primary, #2e7d32);
    }
    .checks li.bad {
      color: var(--mat-sys-error, #c62828);
      font-weight: 500;
    }
    .hint {
      font-size: 0.9em;
      opacity: 0.8;
    }
    .confirm {
      width: 100%;
    }
  `,
})
export class RestartDaqDialogComponent {
  readonly data = inject<RestartDaqDialogData>(MAT_DIALOG_DATA);
  readonly word = RESTART_CONFIRM_WORD;
  readonly typed = signal('');
}
