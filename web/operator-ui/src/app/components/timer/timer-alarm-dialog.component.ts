import { Component, inject, OnInit, OnDestroy } from '@angular/core';
import { MAT_DIALOG_DATA, MatDialogModule, MatDialogRef } from '@angular/material/dialog';
import { MatButtonModule } from '@angular/material/button';
import { TimerService } from '../../services/timer.service';
import { TimerExpiredNotice } from '../../models/types';

/** Shown on every open page when the Operator reports that a run's time is up (TODO 86). */
@Component({
  selector: 'app-timer-alarm-dialog',
  standalone: true,
  imports: [MatDialogModule, MatButtonModule],
  template: `
    <h2 mat-dialog-title>Run {{ notice.run_number }}: time is up</h2>
    <mat-dialog-content>
      <p>The {{ notice.minutes }} min run timer has expired.</p>
      @if (notice.auto_stop) {
        <p data-testid="timer-alarm-stopped">The Operator stopped the run.</p>
      } @else {
        <p data-testid="timer-alarm-going-on">The run goes on: stop it when ready.</p>
      }
    </mat-dialog-content>
    <mat-dialog-actions align="end">
      <button mat-raised-button color="primary" (click)="onClose()">Dismiss</button>
    </mat-dialog-actions>
  `,
  styles: `
    :host {
      display: block;
      text-align: center;
    }
    h2 {
      color: #f44336;
    }
    p {
      font-size: 18px;
      margin: 16px 0;
    }
  `,
})
export class TimerAlarmDialogComponent implements OnInit, OnDestroy {
  private readonly dialogRef = inject(MatDialogRef<TimerAlarmDialogComponent>);
  private readonly timer = inject(TimerService);
  readonly notice: TimerExpiredNotice = inject(MAT_DIALOG_DATA);

  ngOnInit(): void {
    this.timer.startAlarm();
  }

  ngOnDestroy(): void {
    this.timer.stopAlarm();
  }

  onClose(): void {
    this.dialogRef.close();
  }
}
