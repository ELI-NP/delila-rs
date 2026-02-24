import { Component, OnInit, inject, signal } from '@angular/core';
import { CommonModule } from '@angular/common';
import { HttpClient } from '@angular/common/http';
import { MatTableModule } from '@angular/material/table';
import { MatSortModule, Sort } from '@angular/material/sort';
import { MatButtonModule } from '@angular/material/button';
import { MatIconModule } from '@angular/material/icon';
import { MatTabsModule } from '@angular/material/tabs';
import { MatProgressSpinnerModule } from '@angular/material/progress-spinner';
import {
  trigger,
  state,
  style,
  animate,
  transition,
} from '@angular/animations';
import { DigitizerConfig, BoardConfig, ChannelConfig } from '../../models/types';

interface RunHistoryItem {
  run_number: number;
  exp_name: string;
  comment: string;
  start_time: number;
  end_time: number | null;
  duration_secs: number | null;
  status: string;
  stats: { total_events: number; total_bytes: number; average_rate: number };
}

@Component({
  selector: 'app-runs-page',
  standalone: true,
  imports: [
    CommonModule,
    MatTableModule,
    MatSortModule,
    MatButtonModule,
    MatIconModule,
    MatTabsModule,
    MatProgressSpinnerModule,
  ],
  animations: [
    trigger('detailExpand', [
      state('collapsed,void', style({ height: '0px', minHeight: '0' })),
      state('expanded', style({ height: '*' })),
      transition(
        'expanded <=> collapsed',
        animate('225ms cubic-bezier(0.4, 0.0, 0.2, 1)'),
      ),
    ]),
  ],
  template: `
    <div class="runs-container">
      <div class="header-row">
        <h2>Run History</h2>
        <button mat-stroked-button (click)="loadRuns()">
          <mat-icon>refresh</mat-icon> Refresh
        </button>
      </div>

      @if (loading()) {
        <div class="loading-container">
          <mat-spinner diameter="40"></mat-spinner>
        </div>
      } @else if (runs().length === 0) {
        <p class="no-data">No runs found.</p>
      } @else {
        <table mat-table [dataSource]="sortedRuns()" multiTemplateDataRows matSort (matSortChange)="onSortChange($event)" class="runs-table">
          <ng-container matColumnDef="run_number">
            <th mat-header-cell *matHeaderCellDef mat-sort-header>Run#</th>
            <td mat-cell *matCellDef="let run">{{ run.run_number }}</td>
          </ng-container>

          <ng-container matColumnDef="comment">
            <th mat-header-cell *matHeaderCellDef>Comment</th>
            <td mat-cell *matCellDef="let run" class="comment-cell">{{ run.comment }}</td>
          </ng-container>

          <ng-container matColumnDef="start_time">
            <th mat-header-cell *matHeaderCellDef mat-sort-header>Start</th>
            <td mat-cell *matCellDef="let run">{{ formatTime(run.start_time) }}</td>
          </ng-container>

          <ng-container matColumnDef="duration">
            <th mat-header-cell *matHeaderCellDef mat-sort-header="duration_secs">Duration</th>
            <td mat-cell *matCellDef="let run">{{ formatDuration(run.duration_secs) }}</td>
          </ng-container>

          <ng-container matColumnDef="end_time">
            <th mat-header-cell *matHeaderCellDef mat-sort-header>End</th>
            <td mat-cell *matCellDef="let run">{{ run.end_time ? formatTime(run.end_time) : '—' }}</td>
          </ng-container>

          <!-- Expanded detail column -->
          <ng-container matColumnDef="expandedDetail">
            <td mat-cell *matCellDef="let run" [attr.colspan]="displayedColumns.length">
              <div class="detail-expand"
                   [@detailExpand]="selectedRun()?.run_number === run.run_number ? 'expanded' : 'collapsed'">
                @if (selectedRun()?.run_number === run.run_number) {
                  <div class="detail-content">
                    <div class="detail-grid">
                      <div class="detail-item">
                        <span class="label">Experiment</span>
                        <span class="value">{{ run.exp_name }}</span>
                      </div>
                      <div class="detail-item">
                        <span class="label">Status</span>
                        <span class="value" [class]="'status-' + run.status">{{ run.status }}</span>
                      </div>
                      <div class="detail-item">
                        <span class="label">Comment</span>
                        <span class="value">{{ run.comment || '(none)' }}</span>
                      </div>
                      <div class="detail-item">
                        <span class="label">Start</span>
                        <span class="value">{{ formatTimeFull(run.start_time) }}</span>
                      </div>
                      <div class="detail-item">
                        <span class="label">End</span>
                        <span class="value">{{ run.end_time ? formatTimeFull(run.end_time) : '—' }}</span>
                      </div>
                      <div class="detail-item">
                        <span class="label">Duration</span>
                        <span class="value">{{ formatDuration(run.duration_secs) }}</span>
                      </div>
                      <div class="detail-item">
                        <span class="label">Events</span>
                        <span class="value">{{ formatCount(run.stats.total_events) }}</span>
                      </div>
                      <div class="detail-item">
                        <span class="label">Data Size</span>
                        <span class="value">{{ formatBytes(run.stats.total_bytes) }}</span>
                      </div>
                      <div class="detail-item">
                        <span class="label">Avg Rate</span>
                        <span class="value">{{ formatRate(run.stats.average_rate) }}</span>
                      </div>
                    </div>

                    <!-- Config Snapshot -->
                    <h3 class="section-title">Digitizer Configurations</h3>
                    @if (configLoading()) {
                      <mat-spinner diameter="24"></mat-spinner>
                    } @else if (configSnapshot() === null || configSnapshot()!.length === 0) {
                      <p class="no-data">No configuration snapshot available for this run.</p>
                    } @else {
                      <nav mat-tab-nav-bar [tabPanel]="configPanel">
                        @for (cfg of configSnapshot()!; track cfg.digitizer_id) {
                          <a mat-tab-link
                             [active]="selectedConfigIdx() === $index"
                             (click)="selectedConfigIdx.set($index)">
                            {{ cfg.name }}
                          </a>
                        }
                      </nav>
                      <mat-tab-nav-panel #configPanel>
                        @if (selectedConfig(); as cfg) {
                          <div class="config-detail">
                            <div class="config-header">
                              <span><strong>Model:</strong> {{ cfg.model || '—' }}</span>
                              <span><strong>FW:</strong> {{ cfg.firmware }}</span>
                              <span><strong>Serial:</strong> {{ cfg.serial_number || '—' }}</span>
                              <span><strong>Channels:</strong> {{ cfg.num_channels }}</span>
                              @if (cfg.is_master) { <span class="master-badge">Master</span> }
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
                  </div>
                }
              </div>
            </td>
          </ng-container>

          <tr mat-header-row *matHeaderRowDef="displayedColumns"></tr>
          <tr mat-row *matRowDef="let run; columns: displayedColumns"
              class="data-row"
              [class.expanded-row]="selectedRun()?.run_number === run.run_number"
              (click)="selectRun(run)"></tr>
          <tr mat-row *matRowDef="let run; columns: ['expandedDetail']"
              class="detail-row"></tr>
        </table>
      }
    </div>
  `,
  styles: `
    .runs-container {
      padding: 16px;
      max-width: 1200px;
      margin: 0 auto;
    }
    .header-row {
      display: flex;
      justify-content: space-between;
      align-items: center;
      margin-bottom: 16px;
    }
    .header-row h2 { margin: 0; }
    .loading-container {
      display: flex;
      justify-content: center;
      padding: 32px;
    }
    .no-data {
      color: rgba(0,0,0,0.38);
      text-align: center;
      padding: 16px;
    }
    .runs-table {
      width: 100%;
    }
    tr.data-row {
      cursor: pointer;
    }
    tr.data-row:hover {
      background: rgba(0,0,0,0.04);
    }
    tr.expanded-row {
      background: rgba(33,150,243,0.12);
    }
    tr.detail-row {
      height: 0;
    }
    .comment-cell {
      max-width: 250px;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }
    .numeric-cell {
      text-align: right;
      font-variant-numeric: tabular-nums;
    }
    .detail-expand {
      overflow: hidden;
    }
    .detail-content {
      padding: 16px 8px;
    }
    .detail-grid {
      display: grid;
      grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
      gap: 12px;
      margin-bottom: 16px;
    }
    .detail-item {
      display: flex;
      flex-direction: column;
    }
    .detail-item .label {
      font-size: 12px;
      color: rgba(0,0,0,0.54);
      margin-bottom: 2px;
    }
    .detail-item .value {
      font-size: 14px;
    }
    .status-completed { color: #4caf50; }
    .status-running { color: #2196f3; }
    .status-error { color: #f44336; }
    .status-aborted { color: #ff9800; }
    .section-title {
      margin-top: 24px;
      margin-bottom: 8px;
      border-bottom: 1px solid rgba(0,0,0,0.12);
      padding-bottom: 4px;
    }
    .config-detail {
      padding: 12px 0;
    }
    .config-header {
      display: flex;
      gap: 16px;
      flex-wrap: wrap;
      margin-bottom: 12px;
      padding: 8px 12px;
      background: rgba(0,0,0,0.04);
      border-radius: 4px;
    }
    .master-badge {
      background: #ff9800;
      color: #fff;
      padding: 0 6px;
      border-radius: 4px;
      font-size: 12px;
      font-weight: bold;
    }
    h4 {
      margin: 12px 0 4px;
      font-size: 13px;
      color: rgba(0,0,0,0.6);
    }
    .kv-table {
      width: 100%;
      border-collapse: collapse;
      font-size: 13px;
    }
    .kv-table.inline {
      margin-left: 16px;
    }
    .kv-table td {
      padding: 2px 8px;
      border-bottom: 1px solid rgba(0,0,0,0.06);
    }
    td.kv-key {
      color: rgba(0,0,0,0.7);
      width: 220px;
      white-space: nowrap;
      font-weight: 500;
    }
    td.kv-value {
      color: rgba(0,0,0,0.87);
      font-family: monospace;
    }
    .override-section {
      margin: 4px 0;
    }
  `,
})
export class RunsPageComponent implements OnInit {
  private http = inject(HttpClient);

  readonly runs = signal<RunHistoryItem[]>([]);
  readonly selectedRun = signal<RunHistoryItem | null>(null);
  readonly configSnapshot = signal<DigitizerConfig[] | null>(null);
  readonly configLoading = signal(false);
  readonly loading = signal(false);
  readonly selectedConfigIdx = signal(0);
  readonly sortState = signal<Sort>({ active: 'run_number', direction: 'desc' });

  displayedColumns = ['run_number', 'comment', 'start_time', 'end_time', 'duration'];

  selectedConfig = () => {
    const snapshot = this.configSnapshot();
    if (!snapshot || snapshot.length === 0) return null;
    return snapshot[this.selectedConfigIdx()] ?? null;
  };

  sortedRuns = () => {
    const runs = this.runs();
    const sort = this.sortState();
    if (!sort.active || sort.direction === '') return runs;

    return [...runs].sort((a, b) => {
      const dir = sort.direction === 'asc' ? 1 : -1;
      switch (sort.active) {
        case 'run_number': return (a.run_number - b.run_number) * dir;
        case 'start_time': return (a.start_time - b.start_time) * dir;
        case 'end_time': return ((a.end_time ?? 0) - (b.end_time ?? 0)) * dir;
        case 'duration_secs': return ((a.duration_secs ?? 0) - (b.duration_secs ?? 0)) * dir;
        default: return 0;
      }
    });
  };

  ngOnInit(): void {
    this.loadRuns();
  }

  loadRuns(): void {
    this.loading.set(true);
    this.http.get<RunHistoryItem[]>('/api/runs?limit=200').subscribe({
      next: (runs) => {
        this.runs.set(runs);
        this.loading.set(false);
      },
      error: () => this.loading.set(false),
    });
  }

  selectRun(run: RunHistoryItem): void {
    if (this.selectedRun()?.run_number === run.run_number) {
      this.selectedRun.set(null);
      this.configSnapshot.set(null);
      return;
    }
    this.selectedRun.set(run);
    this.configSnapshot.set(null);
    this.selectedConfigIdx.set(0);
    this.configLoading.set(true);
    this.http.get<DigitizerConfig[]>(`/api/runs/${run.run_number}/config`).subscribe({
      next: (configs) => {
        this.configSnapshot.set(configs);
        this.configLoading.set(false);
      },
      error: () => {
        this.configSnapshot.set(null);
        this.configLoading.set(false);
      },
    });
  }

  onSortChange(sort: Sort): void {
    this.sortState.set(sort);
  }

  // --- Formatting helpers ---

  formatKey(key: string): string {
    return key.replace(/_/g, ' ').replace(/\b\w/g, (c) => c.toUpperCase());
  }

  formatTime(ms: number): string {
    if (!ms) return '—';
    const d = new Date(ms);
    const month = d.toLocaleString('en', { month: 'short' });
    const day = d.getDate();
    const h = d.getHours().toString().padStart(2, '0');
    const m = d.getMinutes().toString().padStart(2, '0');
    return `${month} ${day} ${h}:${m}`;
  }

  formatTimeFull(ms: number): string {
    if (!ms) return '—';
    return new Date(ms).toLocaleString();
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
