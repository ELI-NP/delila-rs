import { Component, inject, signal, output, computed, effect } from '@angular/core';
import { CommonModule, DatePipe } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { MatCardModule } from '@angular/material/card';
import { MatButtonModule } from '@angular/material/button';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatInputModule } from '@angular/material/input';
import { MatSnackBarModule } from '@angular/material/snack-bar';
import { NotificationService } from '../../services/notification.service';
import { MatIconModule } from '@angular/material/icon';
import { MatDividerModule } from '@angular/material/divider';
import { MatTooltipModule } from '@angular/material/tooltip';
import { MatDialog } from '@angular/material/dialog';
import { OperatorService } from '../../services/operator.service';
import { DigitizerService } from '../../services/digitizer.service';
import { WaveformWarningDialogComponent } from './waveform-warning-dialog.component';

@Component({
  selector: 'app-control-panel',
  standalone: true,
  imports: [
    CommonModule,
    FormsModule,
    MatCardModule,
    MatButtonModule,
    MatFormFieldModule,
    MatInputModule,
    MatSnackBarModule,
    MatIconModule,
    MatDividerModule,
    MatTooltipModule,
    DatePipe,
  ],
  template: `
    <mat-card>
      <mat-card-header>
        <mat-card-title>Control Panel</mat-card-title>
      </mat-card-header>
      <mat-card-content>
        <div class="form-fields">
          <mat-form-field appearance="outline">
            <mat-label>Experiment Name</mat-label>
            <input matInput [value]="expName()" disabled />
          </mat-form-field>

          <!-- Run Number — collapsed by default; click "Override" to expand the editor.
               Common case is auto-increment, so the operator never has to look at it. -->
          @if (!isEditMode()) {
            <div class="run-number-collapsed">
              <span class="run-number-readonly">
                Next run: <strong>#{{ displayRunNumber() }}</strong>
              </span>
              <button
                mat-button
                class="override-link"
                (click)="enterEditMode()"
                [disabled]="!canEnterEditMode()"
                matTooltip="One-time override. Will return to auto after this run."
              >
                Override…
              </button>
            </div>
          } @else {
            <div class="run-number-row">
              <mat-form-field appearance="outline" class="run-number-field">
                <mat-label>Run Number (override)</mat-label>
                <input
                  matInput
                  type="number"
                  [ngModel]="displayRunNumber()"
                  (ngModelChange)="onRunNumberInput($event)"
                />
              </mat-form-field>
              <button
                mat-icon-button
                color="primary"
                (click)="confirmEdit()"
                matTooltip="Confirm — applied to the next run, then auto resumes"
                aria-label="Confirm edit"
              >
                <mat-icon>check</mat-icon>
              </button>
              <button
                mat-icon-button
                (click)="cancelEdit()"
                matTooltip="Cancel"
                aria-label="Cancel edit"
              >
                <mat-icon>close</mat-icon>
              </button>
            </div>
            <div class="edit-hint">
              <mat-icon>info</mat-icon>
              <span>One-time override. Will return to auto after this run.</span>
            </div>
          }

          <mat-form-field appearance="outline">
            <mat-label>Comment (for Start)</mat-label>
            <textarea
              matInput
              [(ngModel)]="comment"
              rows="2"
              [disabled]="isRunning()"
              placeholder="Beam/target info for this run"
            ></textarea>
          </mat-form-field>
        </div>

        <div class="button-grid">
          <button
            mat-raised-button
            color="primary"
            [disabled]="!operator.buttonStates().configure"
            (click)="onConfigure()"
          >
            Configure
          </button>
          <button
            mat-raised-button
            [disabled]="!operator.buttonStates().reset"
            (click)="onReset()"
            matTooltip="Drop all components back to Idle (recovery from a stuck state)"
          >Force Reset</button>
          <button
            mat-raised-button
            color="accent"
            [disabled]="!canStart()"
            (click)="onStart()"
          >
            Start
          </button>
          <button mat-raised-button color="warn" [disabled]="!operator.buttonStates().stop" (click)="onStop()">
            Stop
          </button>
        </div>

        <div class="state-display">
          <strong>System State:</strong> {{ operator.systemState() }}
        </div>

        <!-- Notes section (only visible during run) -->
        @if (isRunning()) {
          <mat-divider class="notes-divider"></mat-divider>
          <div class="notes-section">
            <div class="notes-header">
              <strong>Run Notes</strong>
              <span class="notes-count">({{ runNotes().length }} entries)</span>
            </div>

            <!-- Existing notes -->
            @if (runNotes().length > 0) {
              <div class="notes-list">
                @for (note of runNotes(); track note.time) {
                  <div class="note-entry">
                    <span class="note-time">{{ note.time | date:'HH:mm:ss' }}</span>
                    <span class="note-text">{{ note.text }}</span>
                  </div>
                }
              </div>
            }

            <!-- Add note input -->
            <div class="add-note">
              <mat-form-field appearance="outline" class="note-input">
                <mat-label>Add Note</mat-label>
                <input
                  matInput
                  [(ngModel)]="newNote"
                  (keyup.enter)="onAddNote()"
                  placeholder="e.g., Beam intensity increased"
                />
              </mat-form-field>
              <button
                mat-mini-fab
                color="primary"
                [disabled]="!newNote.trim()"
                (click)="onAddNote()"
                aria-label="Add note"
              >
                <mat-icon>add</mat-icon>
              </button>
            </div>
          </div>
        }
      </mat-card-content>
    </mat-card>
  `,
  styles: `
    mat-card {
      height: 100%;
    }
    .form-fields {
      display: flex;
      flex-direction: column;
      gap: 8px;
      margin-bottom: 16px;
    }
    .run-number-row {
      display: flex;
      align-items: center;
      gap: 4px;
    }
    .run-number-field {
      flex: 1;
    }
    .run-number-collapsed {
      display: flex;
      align-items: center;
      justify-content: space-between;
      padding: 4px 4px 4px 8px;
      font-size: 14px;
      color: rgba(0, 0, 0, 0.7);
    }
    .run-number-readonly strong {
      color: rgba(0, 0, 0, 0.87);
      font-variant-numeric: tabular-nums;
    }
    .override-link {
      font-size: 12px;
      color: rgba(0, 0, 0, 0.55);
      letter-spacing: 0.2px;
    }
    .edit-hint {
      display: flex;
      align-items: center;
      gap: 8px;
      padding: 8px 12px;
      background: #fff3e0;
      border-radius: 4px;
      font-size: 0.85em;
      color: #e65100;
      margin-top: -4px;
    }
    .edit-hint mat-icon {
      font-size: 18px;
      width: 18px;
      height: 18px;
    }
    .button-grid {
      display: grid;
      grid-template-columns: 1fr 1fr;
      gap: 8px;
      margin-bottom: 16px;
    }
    .state-display {
      text-align: center;
      padding: 8px;
      background: #f5f5f5;
      border-radius: 4px;
    }
    .notes-divider {
      margin: 16px 0;
    }
    .notes-section {
      margin-top: 8px;
    }
    .notes-header {
      display: flex;
      align-items: center;
      gap: 8px;
      margin-bottom: 8px;
    }
    .notes-count {
      color: #666;
      font-size: 0.85em;
    }
    .notes-list {
      max-height: 150px;
      overflow-y: auto;
      background: #fafafa;
      border-radius: 4px;
      padding: 8px;
      margin-bottom: 8px;
    }
    .note-entry {
      display: flex;
      gap: 8px;
      padding: 4px 0;
      border-bottom: 1px solid #eee;
    }
    .note-entry:last-child {
      border-bottom: none;
    }
    .note-time {
      color: #666;
      font-family: monospace;
      font-size: 0.85em;
      white-space: nowrap;
    }
    .note-text {
      flex: 1;
    }
    .add-note {
      display: flex;
      gap: 8px;
      align-items: center;
    }
    .note-input {
      flex: 1;
    }
  `,
})
export class ControlPanelComponent {
  readonly operator = inject(OperatorService);
  private readonly notify = inject(NotificationService);
  private readonly dialog = inject(MatDialog);
  private readonly digitizerService = inject(DigitizerService);

  // Edit mode state
  private editMode = signal(false);
  private editValue = signal<number | null>(null);

  // One-shot override: confirmed override value (used until Configure/Start completes)
  private overrideRunNumber = signal<number | null>(null);

  comment = '';
  newNote = '';
  private commentInitialized = false;

  // Experiment name from server (read-only)
  readonly expName = computed(() => this.operator.experimentName() || 'Loading...');

  // Suggested comment from last run (comment + notes formatted)
  readonly suggestedComment = computed(() => {
    const lastRun = this.operator.lastRunInfo();
    if (!lastRun) return '';

    let result = lastRun.comment || '';

    if (lastRun.notes && lastRun.notes.length > 0) {
      if (result) result += '\n---\n';
      result += lastRun.notes
        .map((n) => {
          const time = new Date(n.time).toLocaleTimeString('ja-JP', {
            hour: '2-digit',
            minute: '2-digit',
            second: '2-digit',
          });
          return `[${time}] ${n.text}`;
        })
        .join('\n');
    }

    return result;
  });

  constructor() {
    // Ensure digitizer configs are loaded for waveform warning check
    this.digitizerService.loadDigitizers();

    // Sync comment with server state
    effect(() => {
      const runInfo = this.operator.runInfo();
      const isRunning = runInfo?.status === 'running';
      const suggested = this.suggestedComment();

      if (isRunning) {
        // During run: show the comment from server (for browser reload case)
        if (runInfo?.comment && !this.commentSyncedForCurrentRun) {
          this.comment = runInfo.comment;
          this.commentSyncedForCurrentRun = true;
        }
        // Reset initialization flag for next stop
        this.commentInitialized = false;
      } else {
        // Not running: auto-fill from last run (one-time)
        this.commentSyncedForCurrentRun = false;
        if (!this.commentInitialized && suggested) {
          this.comment = suggested;
          this.commentInitialized = true;
        }
      }
    });
  }

  // Track if we've synced comment for current run (for browser reload during run)
  private commentSyncedForCurrentRun = false;

  // Computed values for template
  readonly isRunning = computed(() => this.operator.runInfo()?.status === 'running');
  readonly runNotes = computed(() => this.operator.runInfo()?.notes ?? []);
  readonly isEditMode = computed(() => this.editMode());

  // Run number display logic:
  // - If running: show run_info.run_number
  // - If edit mode: show edit value
  // - If override set: show override value
  // - Otherwise: show server's next_run_number
  readonly displayRunNumber = computed(() => {
    const runInfo = this.operator.runInfo();
    if (runInfo?.status === 'running') {
      return runInfo.run_number;
    }

    if (this.editMode() && this.editValue() !== null) {
      return this.editValue()!;
    }

    if (this.overrideRunNumber() !== null) {
      return this.overrideRunNumber()!;
    }

    return this.operator.nextRunNumber() ?? 1;
  });

  // Can enter edit mode when not running and system allows configure
  readonly canEnterEditMode = computed(() => {
    return !this.isRunning() && this.operator.buttonStates().configure;
  });

  // Events for parent component
  readonly runStarted = output<{ runNumber: number; expName: string }>();
  readonly runStopped = output<void>();

  // Edit mode methods
  enterEditMode(): void {
    this.editMode.set(true);
    this.editValue.set(this.displayRunNumber());
  }

  onRunNumberInput(value: number): void {
    if (this.editMode()) {
      this.editValue.set(value);
    }
  }

  confirmEdit(): void {
    const value = this.editValue();
    if (value !== null && value > 0) {
      this.overrideRunNumber.set(value);
    }
    this.editMode.set(false);
    this.editValue.set(null);
  }

  cancelEdit(): void {
    this.editMode.set(false);
    this.editValue.set(null);
  }

  // Clear override after action completes (one-shot behavior)
  private clearOverride(): void {
    this.overrideRunNumber.set(null);
  }

  // Start is enabled from Idle, Configured, and Armed states
  // (backend does full Reset → Configure → Arm → Start cycle)
  canStart(): boolean {
    const state = this.operator.systemState();
    return state === 'Idle' || state === 'Configured' || state === 'Armed';
  }

  onConfigure(): void {
    const runNumber = this.displayRunNumber();
    const expName = this.expName();
    this.operator.configure({ run_number: runNumber, exp_name: expName }).subscribe({
      next: (res) => {
        if (res.success) {
          this.notify.success('Configured successfully');
          // Don't clear override here - user may want to use the same number for Start
        } else {
          this.notify.error(`Configure failed: ${res.message}`);
        }
      },
      error: (err: unknown) => this.notify.error(`Configure failed: ${this.extractError(err)}`),
    });
  }

  onStart(): void {
    // Check if any digitizer has waveform recording enabled
    const wfDigitizers = this.digitizerService
      .digitizers()
      .filter((d) => d.board.waveforms_enabled === true)
      .map((d) => d.name);

    if (wfDigitizers.length > 0) {
      const dialogRef = this.dialog.open(WaveformWarningDialogComponent, {
        data: { digitizerNames: wfDigitizers },
      });
      dialogRef.afterClosed().subscribe((confirmed) => {
        if (confirmed) this.doStart();
      });
      return;
    }
    this.doStart();
  }

  private doStart(): void {
    const runNumber = this.displayRunNumber();
    const expName = this.expName();
    const comment = this.comment;
    this.operator.start(runNumber, comment).subscribe({
      next: (res) => {
        if (res.success) {
          this.notify.success('Started successfully');
          this.runStarted.emit({ runNumber, expName });
          // Clear override after successful start - next stop will show server's next_run_number
          this.clearOverride();
        } else {
          this.notify.error(`Start failed: ${res.message}`);
        }
      },
      error: (err: unknown) => this.notify.error(`Start failed: ${this.extractError(err)}`),
    });
  }

  onStop(): void {
    this.operator.stop().subscribe({
      next: (res) => {
        if (res.success) {
          this.notify.success('Stopped successfully');
          this.runStopped.emit();
          // Override should already be cleared, but ensure it's cleared
          this.clearOverride();
        } else {
          this.notify.error(`Stop failed: ${res.message}`);
        }
      },
      error: (err: unknown) => this.notify.error(`Stop failed: ${this.extractError(err)}`),
    });
  }

  onReset(): void {
    this.operator.reset().subscribe({
      next: (res) => {
        if (res.success) {
          this.notify.success('Reset successfully');
          // Clear override on reset as well
          this.clearOverride();
        } else {
          this.notify.error(`Reset failed: ${res.message}`);
        }
      },
      error: (err: unknown) => this.notify.error(`Reset failed: ${this.extractError(err)}`),
    });
  }

  onAddNote(): void {
    const text = this.newNote.trim();
    if (!text) return;

    this.operator.addNote(text).subscribe({
      next: () => {
        this.newNote = '';
        this.notify.success('Note added');
      },
      error: () => this.notify.error('Failed to add note'),
    });
  }

  private extractError(err: unknown): string {
    const e = err as { error?: { message?: string }; message?: string };
    return e?.error?.message || e?.message || 'Network error';
  }
}
