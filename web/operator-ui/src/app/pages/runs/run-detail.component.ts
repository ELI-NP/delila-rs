import { Component, OnInit, inject, signal, computed } from '@angular/core';
import { CommonModule } from '@angular/common';
import { HttpClient } from '@angular/common/http';
import { ActivatedRoute, RouterLink } from '@angular/router';
import { MatButtonModule } from '@angular/material/button';
import { MatIconModule } from '@angular/material/icon';
import { MatProgressSpinnerModule } from '@angular/material/progress-spinner';
import { MatTabsModule } from '@angular/material/tabs';
import { DigitizerConfig, BoardConfig, ChannelConfig } from '../../models/types';

interface RunHistoryItem {
  run_number: number;
  exp_name: string;
  comment: string;
  start_time: number;
  end_time: number | null;
  duration_secs: number | null;
  status: string;
  stats: { total_events: number; total_bytes: number; average_rate: number; trigger_loss_count?: number };
}

type SnapshotState = 'loading' | 'absent' | 'ready';

@Component({
  selector: 'app-run-detail-page',
  standalone: true,
  imports: [
    CommonModule,
    RouterLink,
    MatButtonModule,
    MatIconModule,
    MatProgressSpinnerModule,
    MatTabsModule,
  ],
  template: `
    <div class="detail-page">
      <div class="header-row">
        <a mat-stroked-button routerLink="/runs">
          <mat-icon>arrow_back</mat-icon> Back to runs
        </a>
        @if (run(); as r) {
          <h2 class="title">Run #{{ r.run_number }} <span class="exp">— {{ r.exp_name }}</span></h2>
          <span class="status-pill" [class]="'status-' + r.status">{{ r.status }}</span>
        } @else if (runLoading()) {
          <h2 class="title">Loading…</h2>
        } @else {
          <h2 class="title">Run not found</h2>
        }
      </div>

      @if (run(); as r) {
        <section class="meta">
          <div class="detail-grid">
            <div class="detail-item">
              <span class="label">Comment</span>
              <span class="value">{{ r.comment || '(none)' }}</span>
            </div>
            <div class="detail-item">
              <span class="label">Start</span>
              <span class="value">{{ formatTimeFull(r.start_time) }}</span>
            </div>
            <div class="detail-item">
              <span class="label">End</span>
              <span class="value">{{ r.end_time ? formatTimeFull(r.end_time) : '—' }}</span>
            </div>
            <div class="detail-item">
              <span class="label">Duration</span>
              <span class="value">{{ formatDuration(r.duration_secs) }}</span>
            </div>
            <div class="detail-item">
              <span class="label">Events</span>
              <span class="value">{{ formatCount(r.stats.total_events) }}</span>
            </div>
            <div class="detail-item">
              <span class="label">Data Size</span>
              <span class="value">{{ formatBytes(r.stats.total_bytes) }}</span>
            </div>
            <div class="detail-item">
              <span class="label">Avg Rate</span>
              <span class="value">{{ formatRate(r.stats.average_rate) }}</span>
            </div>
            <div class="detail-item">
              <span class="label">Trigger Loss</span>
              <span class="value" [class.trigger-loss]="(r.stats.trigger_loss_count ?? 0) > 0">
                {{ formatCount(r.stats.trigger_loss_count ?? 0) }}
                @if ((r.stats.trigger_loss_count ?? 0) > 0 && r.stats.total_events > 0) {
                  ({{ ((r.stats.trigger_loss_count ?? 0) / (r.stats.total_events + (r.stats.trigger_loss_count ?? 0)) * 100).toFixed(3) }}%)
                }
              </span>
            </div>
          </div>
        </section>

        <h3 class="section-title">Digitizer Configurations</h3>
        @switch (snapshotState()) {
          @case ('loading') {
            <div class="loading-row"><mat-spinner diameter="28"></mat-spinner><span>Loading config snapshot…</span></div>
          }
          @case ('absent') {
            <p class="hint">
              Configuration snapshot is not available for this run.
              Snapshots are only recorded for runs started after the 2026-02-19 capture
              feature shipped — older runs were never captured.
            </p>
          }
          @case ('ready') {
            <nav mat-tab-nav-bar [tabPanel]="configPanel">
              @for (cfg of sortedConfigSnapshot(); track cfg.digitizer_id; let i = $index) {
                <a mat-tab-link
                   [active]="selectedConfigIdx() === i"
                   (click)="selectedConfigIdx.set(i)"
                   (keydown.enter)="selectedConfigIdx.set(i)"
                   tabindex="0">
                  #{{ cfg.digitizer_id }} {{ cfg.name }}
                </a>
              }
            </nav>
            <mat-tab-nav-panel #configPanel>
              @if (selectedConfig(); as cfg) {
                <div class="config-detail">
                  <div class="config-header">
                    <span><strong>ID:</strong> {{ cfg.digitizer_id }}</span>
                    <span><strong>Model:</strong> {{ cfg.model || '—' }}</span>
                    <span><strong>FW:</strong> {{ cfg.firmware }}</span>
                    <span><strong>Serial:</strong> {{ cfg.serial_number || '—' }}</span>
                    <span><strong>Channels:</strong> {{ cfg.num_channels }}</span>
                  </div>

                  <h4>Board Settings</h4>
                  <table class="kv-table">
                    @for (entry of boardEntries(cfg.board); track entry.key) {
                      <tr>
                        <td class="kv-key">{{ formatKey(entry.key) }}</td>
                        <td class="kv-value">{{ entry.value }}</td>
                      </tr>
                    }
                  </table>

                  <h4>Channel Defaults</h4>
                  <table class="kv-table">
                    @for (entry of channelEntries(cfg.channel_defaults); track entry.key) {
                      <tr>
                        <td class="kv-key">{{ formatKey(entry.key) }}</td>
                        <td class="kv-value">{{ entry.value }}</td>
                      </tr>
                    }
                  </table>

                  @if (cfg.channel_overrides && objectKeys(cfg.channel_overrides).length > 0) {
                    <h4>Channel Overrides</h4>
                    @for (chKey of objectKeys(cfg.channel_overrides); track chKey) {
                      <div class="override-section">
                        <strong>Ch {{ chKey }}:</strong>
                        <table class="kv-table inline">
                          @for (entry of channelEntries(cfg.channel_overrides![+chKey]); track entry.key) {
                            <tr>
                              <td class="kv-key">{{ formatKey(entry.key) }}</td>
                              <td class="kv-value">{{ entry.value }}</td>
                            </tr>
                          }
                        </table>
                      </div>
                    }
                  }
                </div>
              }
            </mat-tab-nav-panel>
          }
        }
      } @else if (!runLoading()) {
        <p class="hint">
          No run with that number was found.
          <a routerLink="/runs">Go back to the run list.</a>
        </p>
      }
    </div>
  `,
  styles: `
    .detail-page {
      padding: 16px;
      max-width: 1200px;
      margin: 0 auto;
    }
    .header-row {
      display: flex;
      align-items: center;
      gap: 12px;
      margin-bottom: 16px;
    }
    .title { margin: 0; }
    .exp { font-weight: 400; color: rgba(0, 0, 0, 0.6); }
    .status-pill {
      padding: 2px 10px;
      border-radius: 10px;
      font-size: 12px;
      font-weight: 500;
      background: rgba(0, 0, 0, 0.06);
    }
    .status-completed { color: #1b5e20; background: #c8e6c9; }
    .status-running { color: #0d47a1; background: #bbdefb; }
    .status-error { color: #b71c1c; background: #ffcdd2; }
    .status-aborted { color: #bf360c; background: #ffe0b2; }

    .meta { margin-bottom: 24px; }
    .detail-grid {
      display: grid;
      grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
      gap: 12px;
    }
    .detail-item { display: flex; flex-direction: column; }
    .detail-item .label {
      font-size: 12px;
      color: rgba(0, 0, 0, 0.54);
      margin-bottom: 2px;
    }
    .detail-item .value { font-size: 14px; }
    .trigger-loss { color: #f44336; font-weight: 500; }

    .section-title {
      margin-top: 24px;
      margin-bottom: 8px;
      border-bottom: 1px solid rgba(0, 0, 0, 0.12);
      padding-bottom: 4px;
    }
    .loading-row {
      display: flex;
      align-items: center;
      gap: 12px;
      padding: 12px 0;
      color: rgba(0, 0, 0, 0.6);
    }
    .hint {
      color: rgba(0, 0, 0, 0.6);
      max-width: 60ch;
    }

    .config-detail { padding: 12px 0; }
    .config-header {
      display: flex;
      gap: 16px;
      flex-wrap: wrap;
      margin-bottom: 12px;
      padding: 8px 12px;
      background: rgba(0, 0, 0, 0.04);
      border-radius: 4px;
    }
    h4 {
      margin: 12px 0 4px;
      font-size: 13px;
      color: rgba(0, 0, 0, 0.6);
    }
    .kv-table {
      width: 100%;
      border-collapse: collapse;
      font-size: 13px;
    }
    .kv-table.inline { margin-left: 16px; }
    .kv-table td {
      padding: 2px 8px;
      border-bottom: 1px solid rgba(0, 0, 0, 0.06);
    }
    td.kv-key {
      color: rgba(0, 0, 0, 0.7);
      width: 220px;
      white-space: nowrap;
      font-weight: 500;
    }
    td.kv-value {
      color: rgba(0, 0, 0, 0.87);
      font-family: monospace;
    }
    .override-section { margin: 4px 0; }
  `,
})
export class RunDetailPageComponent implements OnInit {
  private readonly http = inject(HttpClient);
  private readonly route = inject(ActivatedRoute);

  readonly run = signal<RunHistoryItem | null>(null);
  readonly runLoading = signal(true);
  readonly configSnapshot = signal<DigitizerConfig[] | null>(null);
  readonly snapshotState = signal<SnapshotState>('loading');
  readonly selectedConfigIdx = signal(0);

  readonly sortedConfigSnapshot = computed<DigitizerConfig[]>(() => {
    const snap = this.configSnapshot();
    return snap ? [...snap].sort((a, b) => a.digitizer_id - b.digitizer_id) : [];
  });

  selectedConfig = (): DigitizerConfig | null => {
    const sorted = this.sortedConfigSnapshot();
    if (sorted.length === 0) return null;
    return sorted[this.selectedConfigIdx()] ?? null;
  };

  ngOnInit(): void {
    const param = this.route.snapshot.paramMap.get('runNumber');
    const runNumber = param ? Number(param) : NaN;
    if (!Number.isFinite(runNumber)) {
      this.runLoading.set(false);
      return;
    }
    this.loadRun(runNumber);
    this.loadConfig(runNumber);
  }

  private loadRun(runNumber: number): void {
    this.http.get<RunHistoryItem>(`/api/runs/${runNumber}`).subscribe({
      next: (r) => {
        this.run.set(r);
        this.runLoading.set(false);
      },
      error: () => {
        this.runLoading.set(false);
      },
    });
  }

  private loadConfig(runNumber: number): void {
    this.snapshotState.set('loading');
    this.http.get<DigitizerConfig[]>(`/api/runs/${runNumber}/config`).subscribe({
      next: (configs) => {
        if (!configs || configs.length === 0) {
          this.configSnapshot.set(null);
          this.snapshotState.set('absent');
          return;
        }
        this.configSnapshot.set(configs);
        this.snapshotState.set('ready');
      },
      error: () => {
        this.configSnapshot.set(null);
        this.snapshotState.set('absent');
      },
    });
  }

  // --- formatters (mirrored from runs.component.ts) ---

  formatKey(key: string): string {
    return key.replace(/_/g, ' ').replace(/\b\w/g, (c) => c.toUpperCase());
  }

  /** ISO 8601 with seconds in local time: `2026-05-07 14:30:42`. */
  formatTimeFull(ms: number): string {
    if (!ms) return '—';
    const d = new Date(ms);
    const yyyy = d.getFullYear();
    const mm = (d.getMonth() + 1).toString().padStart(2, '0');
    const dd = d.getDate().toString().padStart(2, '0');
    const h = d.getHours().toString().padStart(2, '0');
    const m = d.getMinutes().toString().padStart(2, '0');
    const s = d.getSeconds().toString().padStart(2, '0');
    return `${yyyy}-${mm}-${dd} ${h}:${m}:${s}`;
  }

  formatDuration(secs: number | null): string {
    if (secs === null || secs === undefined) return '—';
    if (secs < 60) return `${secs}s`;
    if (secs < 3600) {
      const m = Math.floor(secs / 60);
      const s = secs % 60;
      return `${m}m${s.toString().padStart(2, '0')}s`;
    }
    const h = Math.floor(secs / 3600);
    const m = Math.floor((secs % 3600) / 60);
    return `${h}h${m.toString().padStart(2, '0')}m`;
  }

  formatCount(n: number): string {
    if (n === 0) return '0';
    if (n >= 1e9) return `${(n / 1e9).toFixed(1)}G`;
    if (n >= 1e6) return `${(n / 1e6).toFixed(1)}M`;
    if (n >= 1e3) return `${(n / 1e3).toFixed(1)}K`;
    return n.toString();
  }

  formatBytes(bytes: number): string {
    if (bytes === 0) return '0 B';
    if (bytes >= 1e12) return `${(bytes / 1e12).toFixed(1)} TB`;
    if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(1)} GB`;
    if (bytes >= 1e6) return `${(bytes / 1e6).toFixed(1)} MB`;
    if (bytes >= 1e3) return `${(bytes / 1e3).toFixed(1)} KB`;
    return `${bytes} B`;
  }

  formatRate(rate: number): string {
    if (!rate) return '—';
    if (rate >= 1e6) return `${(rate / 1e6).toFixed(1)} M/s`;
    if (rate >= 1e3) return `${(rate / 1e3).toFixed(1)} K/s`;
    return `${rate.toFixed(0)} /s`;
  }

  boardEntries(board: BoardConfig): { key: string; value: string }[] {
    return Object.entries(board)
      .filter(([, v]) => v !== undefined && v !== null)
      .map(([k, v]) => ({
        key: k,
        value: typeof v === 'object' ? JSON.stringify(v) : String(v),
      }));
  }

  channelEntries(ch: ChannelConfig): { key: string; value: string }[] {
    return Object.entries(ch)
      .filter(([, v]) => v !== undefined && v !== null)
      .map(([k, v]) => ({
        key: k,
        value: typeof v === 'object' ? JSON.stringify(v) : String(v),
      }));
  }

  objectKeys(obj: Record<string, unknown> | undefined): string[] {
    if (!obj) return [];
    return Object.keys(obj).sort((a, b) => +a - +b);
  }
}
