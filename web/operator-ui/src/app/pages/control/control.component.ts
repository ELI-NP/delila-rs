import { Component, inject, ViewChild } from '@angular/core';
import { MatSnackBarModule } from '@angular/material/snack-bar';
import { StatusPanelComponent } from '../../components/status-panel/status-panel.component';
import { ControlPanelComponent } from '../../components/control-panel/control-panel.component';
import { RunInfoComponent } from '../../components/run-info/run-info.component';
import { TimerComponent } from '../../components/timer/timer.component';
import { OperatorService } from '../../services/operator.service';
import { NotificationService } from '../../services/notification.service';

@Component({
  selector: 'app-control-page',
  standalone: true,
  imports: [
    MatSnackBarModule,
    StatusPanelComponent,
    ControlPanelComponent,
    RunInfoComponent,
    TimerComponent,
  ],
  template: `
    <div class="control-content">
      <div class="left-column">
        <app-status-panel></app-status-panel>
        <app-run-info></app-run-info>
      </div>
      <div class="right-column">
        <app-control-panel
          #controlPanel
          (runStarted)="onRunStarted($event)"
          (runStopped)="onRunStopped()"
        ></app-control-panel>
        <app-timer (timerStarted)="onTimerStarted()" (timerExpired)="onTimerExpired()"></app-timer>
      </div>
    </div>
  `,
  styles: `
    :host {
      display: flex;
      flex-direction: column;
      height: 100%;
      min-height: 0;
    }

    .control-content {
      display: grid;
      grid-template-columns: 1fr 1fr;
      gap: 16px;
      padding: 16px;
      flex: 1;
      min-height: 0;
    }

    .left-column,
    .right-column {
      display: flex;
      flex-direction: column;
      gap: 16px;
    }

    @media (max-width: 800px) {
      .control-content {
        grid-template-columns: 1fr;
      }
    }
  `,
})
export class ControlPageComponent {
  private readonly operator = inject(OperatorService);
  private readonly notification = inject(NotificationService);

  @ViewChild('controlPanel') controlPanel!: ControlPanelComponent;
  @ViewChild(TimerComponent) timerComp!: TimerComponent;

  // Run info is now managed by the backend and displayed reactively
  // No need for manual startRun/stopRun calls

  onRunStarted(event: { runNumber: number; expName: string }): void {
    // Backend handles run_info update automatically
    // Just log for debugging
    console.log(`Run ${event.runNumber} started (${event.expName})`);
  }

  onRunStopped(): void {
    // Backend handles run_info update automatically
    console.log('Run stopped');
  }

  onTimerStarted(): void {
    const runNumber = this.controlPanel.displayRunNumber();
    const comment = this.controlPanel.comment;
    this.operator.start(runNumber, comment).subscribe({
      next: (res) => {
        if (res.success) {
          this.timerComp.confirmStarted();
          this.notification.success(`Started run ${runNumber} with timer`);
        } else {
          this.timerComp.confirmFailed();
          this.notification.error(`Start failed: ${res.message}`);
        }
      },
      error: (err: unknown) => {
        this.timerComp.confirmFailed();
        const e = err as { error?: { message?: string }; message?: string };
        const msg = e?.error?.message || e?.message || 'Network error';
        this.notification.error(`Start failed: ${msg}`);
      },
    });
  }

  onTimerExpired(): void {
    this.operator.stop().subscribe({
      next: (res) => {
        if (res.success) {
          // Backend handles run_info update automatically
          // Run number will be updated via polling from server
        }
      },
    });
  }
}
