import { Injectable, inject, signal, computed } from '@angular/core';
import { HttpClient } from '@angular/common/http';
import { Observable, interval, switchMap, catchError, of, tap, Subject, takeUntil } from 'rxjs';
import {
  AxisSource,
  Histogram1D,
  Histogram2D,
  HistogramListResponse,
  MonitorStatusResponse,
  ChannelSummary,
  channelKeyString,
  WaveformListResponse,
  LatestWaveform,
} from '../models/histogram.types';
import { OperatorService } from './operator.service';

@Injectable({
  providedIn: 'root',
})
export class HistogramService {
  private readonly refreshInterval = 1000; // 1 second

  private readonly http = inject(HttpClient);
  private readonly operator = inject(OperatorService);

  // Dynamic Monitor base URL from Operator status
  readonly monitorBaseUrl = computed(() => {
    const port = this.operator.status()?.monitor_http_port;
    if (!port) return null;
    const host = window.location.hostname || 'localhost';
    return `http://${host}:${port}/api`;
  });
  private stopPolling$ = new Subject<void>();

  // Signals for reactive state
  readonly status = signal<MonitorStatusResponse | null>(null);
  readonly channelList = signal<ChannelSummary[]>([]);
  readonly histogramCache = signal<Map<string, Histogram1D>>(new Map());
  readonly isPolling = signal(false);
  readonly error = signal<string | null>(null);

  // Computed values
  readonly totalEvents = computed(() => this.status()?.total_events ?? 0);
  readonly eventRate = computed(() => this.status()?.event_rate ?? 0);
  readonly elapsedSecs = computed(() => this.status()?.elapsed_secs ?? 0);
  readonly numChannels = computed(() => this.status()?.num_channels ?? 0);

  // Get histogram from cache
  getHistogram(moduleId: number, channelId: number): Histogram1D | undefined {
    return this.histogramCache().get(channelKeyString(moduleId, channelId));
  }

  // Start polling for status and histogram list
  startPolling(): void {
    if (this.isPolling()) return;
    this.isPolling.set(true);

    // Poll status
    interval(this.refreshInterval)
      .pipe(
        takeUntil(this.stopPolling$),
        switchMap(() => this.fetchStatus()),
        tap((status) => {
          if (status) {
            this.status.set(status);
            this.error.set(null);
          }
        }),
        catchError(() => {
          this.error.set('Failed to connect to Monitor');
          return of(null);
        })
      )
      .subscribe();

    // Poll histogram list
    interval(this.refreshInterval)
      .pipe(
        takeUntil(this.stopPolling$),
        switchMap(() => this.fetchHistogramList()),
        tap((list) => {
          if (list) {
            this.channelList.set(list.channels);
          }
        }),
        catchError(() => of(null))
      )
      .subscribe();

    // Initial fetch
    this.fetchStatus().subscribe((status) => {
      if (status) this.status.set(status);
    });
    this.fetchHistogramList().subscribe((list) => {
      if (list) this.channelList.set(list.channels);
    });
  }

  stopPolling(): void {
    this.stopPolling$.next();
    this.isPolling.set(false);
  }

  // Fetch specific histogram and update cache
  fetchAndCacheHistogram(moduleId: number, channelId: number): Observable<Histogram1D | null> {
    return this.fetchHistogram(moduleId, channelId).pipe(
      tap((histogram) => {
        if (histogram) {
          const key = channelKeyString(moduleId, channelId);
          const cache = new Map(this.histogramCache());
          cache.set(key, histogram);
          this.histogramCache.set(cache);
        }
      }),
      catchError(() => of(null))
    );
  }

  // API calls — return of(null) if Monitor URL not yet available
  fetchStatus(): Observable<MonitorStatusResponse | null> {
    const url = this.monitorBaseUrl();
    if (!url) return of(null);
    return this.http.get<MonitorStatusResponse>(`${url}/status`).pipe(catchError(() => of(null)));
  }

  fetchHistogramList(): Observable<HistogramListResponse | null> {
    const url = this.monitorBaseUrl();
    if (!url) return of(null);
    return this.http.get<HistogramListResponse>(`${url}/histograms`).pipe(catchError(() => of(null)));
  }

  fetchHistogram(moduleId: number, channelId: number): Observable<Histogram1D | null> {
    const url = this.monitorBaseUrl();
    if (!url) return of(null);
    return this.http
      .get<Histogram1D>(`${url}/histograms/${moduleId}/${channelId}`)
      .pipe(catchError(() => of(null)));
  }

  fetchPsdHistogram(moduleId: number, channelId: number): Observable<Histogram1D | null> {
    const url = this.monitorBaseUrl();
    if (!url) return of(null);
    return this.http
      .get<Histogram1D>(`${url}/histograms/${moduleId}/${channelId}`, { params: { type: 'psd' } })
      .pipe(catchError(() => of(null)));
  }

  /**
   * Fetch a 2D histogram for `(moduleId, channelId)` with axes `(x, y)`.
   * The plot is created lazily on the backend on first request and lives
   * for ~60s after the last poll (TTL eviction); active subscribers keep
   * it alive automatically by polling on a 1-second interval.
   */
  fetchHistogram2d(
    moduleId: number,
    channelId: number,
    x: AxisSource = 'energy',
    y: AxisSource = 'psd',
  ): Observable<Histogram2D | null> {
    const url = this.monitorBaseUrl();
    if (!url) return of(null);
    return this.http
      .get<Histogram2D>(`${url}/histograms2d/${moduleId}/${channelId}`, {
        params: { x, y },
      })
      .pipe(catchError(() => of(null)));
  }

  clearHistograms(): Observable<void | null> {
    const url = this.monitorBaseUrl();
    if (!url) return of(null);
    return this.http.post<void>(`${url}/histograms/clear`, {});
  }

  // Waveform API calls
  fetchWaveformList(): Observable<WaveformListResponse | null> {
    const url = this.monitorBaseUrl();
    if (!url) return of(null);
    return this.http.get<WaveformListResponse>(`${url}/waveforms`).pipe(catchError(() => of(null)));
  }

  fetchWaveform(moduleId: number, channelId: number): Observable<LatestWaveform | null> {
    const url = this.monitorBaseUrl();
    if (!url) return of(null);
    return this.http
      .get<LatestWaveform>(`${url}/waveforms/${moduleId}/${channelId}`)
      .pipe(catchError(() => of(null)));
  }
}
