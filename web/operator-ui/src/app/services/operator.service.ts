import { Injectable, signal, computed, inject } from '@angular/core';
import { HttpClient } from '@angular/common/http';
import { Observable, interval, switchMap, catchError, of, tap } from 'rxjs';
import {
  SystemStatus,
  ConfigureRequest,
  ApiResponse,
  getButtonStates,
  ButtonStates,
  CurrentRunInfo,
  RunNote,
  LastRunInfo,
} from '../models/types';

@Injectable({
  providedIn: 'root',
})
export class OperatorService {
  private readonly baseUrl = '/api';
  private readonly pollingInterval = 1000; // 1 second

  // Signals for reactive state
  readonly status = signal<SystemStatus | null>(null);
  readonly error = signal<string | null>(null);
  readonly isPolling = signal(false);

  /**
   * Last unresolved Apply failure. Set by digitizer-settings on apply error;
   * cleared on a successful Configure. Surfaces an inline guidance alert near
   * the Configure button so the operator knows the previous Apply did not stick
   * and the next step is to fix the config and re-Configure.
   *
   * Audit item X-5 (docs/ui_audit_2026-05-07.md). Backend already hard-fails
   * the Apply via `check_firmware_match` (commit c7d1fc8) and blocks Arm via
   * `auto_config_failed`, but the UI only flashed a sticky red snackbar — no
   * persistent guidance.
   */
  readonly lastApplyFailure = signal<{ digitizerName: string; message: string } | null>(null);

  // Computed values
  readonly systemState = computed(() => this.status()?.system_state ?? 'Offline');
  readonly components = computed(() => this.status()?.components ?? []);
  readonly buttonStates = computed<ButtonStates>(() => getButtonStates(this.systemState()));
  readonly runInfo = computed<CurrentRunInfo | null>(() => this.status()?.run_info ?? null);
  /** Experiment name (server-authoritative, from config file) */
  readonly experimentName = computed(() => this.status()?.experiment_name ?? '');
  /** Next run number from MongoDB (for multi-client sync) */
  readonly nextRunNumber = computed(() => this.status()?.next_run_number ?? null);
  /** Last run info for pre-filling comment (from MongoDB) */
  readonly lastRunInfo = computed<LastRunInfo | null>(() => this.status()?.last_run_info ?? null);
  /** Whether Tune Up mode is active */
  readonly isTuneUp = computed(() => this.status()?.tuneup_mode ?? false);
  /** Digitizer ID being tuned */
  readonly tuneupDigitizerId = computed(() => this.status()?.tuneup_digitizer_id ?? null);

  // Grouped components by role
  readonly sourceComponents = computed(() =>
    this.components().filter((c) => c.role === 'source')
  );
  readonly pipelineComponents = computed(() =>
    this.components().filter((c) => c.role !== 'source')
  );
  readonly sourceSummary = computed(() => {
    const sources = this.sourceComponents();
    return {
      total: sources.length,
      online: sources.filter((c) => c.online).length,
      hasError: sources.some((c) => c.state === 'Error' || !c.online),
      totalRate: sources.reduce((sum, c) => sum + (c.metrics?.event_rate ?? 0), 0),
    };
  });

  // Metrics from Recorder (authoritative source for recorded data)
  readonly recorderMetrics = computed(() => {
    const comps = this.components();
    return comps.find((c) => c.name === 'Recorder')?.metrics ?? null;
  });

  readonly totalEvents = computed(() => {
    return this.recorderMetrics()?.events_processed ?? 0;
  });

  readonly totalRate = computed(() => {
    return this.recorderMetrics()?.event_rate ?? 0;
  });

  private readonly http = inject(HttpClient);

  // Start polling for status
  startPolling(): void {
    if (this.isPolling()) return;
    this.isPolling.set(true);

    interval(this.pollingInterval)
      .pipe(
        switchMap(() =>
          this.getStatus().pipe(
            tap((status) => {
              this.status.set(status);
              this.error.set(null);
            }),
            catchError(() => {
              this.error.set('Failed to connect to Operator');
              this.status.set(null);
              return of(null);
            })
          )
        )
      )
      .subscribe();

    // Initial fetch
    this.getStatus().subscribe({
      next: (status) => {
        this.status.set(status);
        this.error.set(null);
      },
      error: () => {
        this.error.set('Failed to connect to Operator');
        this.status.set(null);
      },
    });
  }

  stopPolling(): void {
    this.isPolling.set(false);
  }

  // API calls
  getStatus(): Observable<SystemStatus> {
    return this.http.get<SystemStatus>(`${this.baseUrl}/status`);
  }

  configure(request: ConfigureRequest): Observable<ApiResponse> {
    return this.http.post<ApiResponse>(`${this.baseUrl}/configure`, request);
  }

  // Full auto-cycle: backend does Reset → Configure → Arm → Start
  start(runNumber: number, comment = ''): Observable<ApiResponse> {
    return this.http.post<ApiResponse>(`${this.baseUrl}/run/start`, {
      run_number: runNumber,
      comment,
      exp_name: this.experimentName(),
    });
  }

  stop(): Observable<ApiResponse> {
    return this.http.post<ApiResponse>(`${this.baseUrl}/stop`, {});
  }

  reset(): Observable<ApiResponse> {
    return this.http.post<ApiResponse>(`${this.baseUrl}/reset`, {});
  }

  // Get next available run number from MongoDB
  getNextRunNumber(): Observable<{ next_run_number: number }> {
    return this.http.get<{ next_run_number: number }>(`${this.baseUrl}/runs/next`);
  }

  // Add a note to the current running run
  addNote(text: string): Observable<RunNote> {
    return this.http.post<RunNote>(`${this.baseUrl}/runs/current/note`, { text });
  }

  // Tune Up mode
  tuneupStart(digitizerId: number): Observable<ApiResponse> {
    return this.http.post<ApiResponse>(`${this.baseUrl}/tuneup/start`, { digitizer_id: digitizerId });
  }

  tuneupStop(): Observable<ApiResponse> {
    return this.http.post<ApiResponse>(`${this.baseUrl}/tuneup/stop`, {});
  }

  tuneupApply(digitizerId: number, config: unknown): Observable<ApiResponse> {
    return this.http.post<ApiResponse>(`${this.baseUrl}/tuneup/apply/${digitizerId}`, config);
  }
}
