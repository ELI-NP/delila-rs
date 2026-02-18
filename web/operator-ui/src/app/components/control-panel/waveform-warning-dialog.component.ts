import { Component, inject } from '@angular/core';
import { MatDialogModule, MAT_DIALOG_DATA } from '@angular/material/dialog';
import { MatButtonModule } from '@angular/material/button';

@Component({
  selector: 'app-waveform-warning-dialog',
  standalone: true,
  imports: [MatDialogModule, MatButtonModule],
  template: `
    <h2 mat-dialog-title>Waveform Recording Enabled</h2>
    <mat-dialog-content>
      <p>Waveform recording is enabled on:</p>
      <ul>
        @for (name of data.digitizerNames; track name) {
          <li>{{ name }}</li>
        }
      </ul>
      <p>This significantly increases data size. Consider disabling waveform recording for production runs.</p>
    </mat-dialog-content>
    <mat-dialog-actions align="end">
      <button mat-button mat-dialog-close>Cancel</button>
      <button mat-flat-button color="warn" [mat-dialog-close]="true">Start Anyway</button>
    </mat-dialog-actions>
  `,
  styles: `
    ul {
      margin: 8px 0;
      padding-left: 20px;
    }
    li {
      font-weight: 500;
    }
  `,
})
export class WaveformWarningDialogComponent {
  readonly data = inject<{ digitizerNames: string[] }>(MAT_DIALOG_DATA);
}
