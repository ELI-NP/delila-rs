import { Component, OnInit, inject, signal } from '@angular/core';
import { CommonModule } from '@angular/common';
import { HttpClient } from '@angular/common/http';
import { Router } from '@angular/router';
import { MatTableModule } from '@angular/material/table';
import { MatSortModule, Sort } from '@angular/material/sort';
import { MatButtonModule } from '@angular/material/button';
import { MatIconModule } from '@angular/material/icon';
import { MatProgressSpinnerModule } from '@angular/material/progress-spinner';

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

@Component({
  selector: 'app-runs-page',
  standalone: true,
  imports: [
    CommonModule,
    MatTableModule,
    MatSortModule,
    MatButtonModule,
    MatIconModule,
    MatProgressSpinnerModule,
  ],
  template: `
    <div class="runs-container">
      <div class="header-row">
        <h2>Run History</h2>
        <div class="header-actions">
          <button mat-stroked-button (click)="exportJson()" [disabled]="exporting()">
            <mat-icon>download</mat-icon> Export JSON
          </button>
          <button mat-stroked-button (click)="loadRuns()">
            <mat-icon>refresh</mat-icon> Refresh
          </button>
        </div>
      </div>

      @if (loading()) {
        <div class="loading-container">
          <mat-spinner diameter="40"></mat-spinner>
        </div>
      } @else if (runs().length === 0) {
        <p class="no-data">No runs found.</p>
      } @else {
        <table mat-table [dataSource]="sortedRuns()" matSort matSortActive="start_time" matSortDirection="desc" (matSortChange)="onSortChange($event)" class="runs-table">
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

          <ng-container matColumnDef="end_time">
            <th mat-header-cell *matHeaderCellDef mat-sort-header>End</th>
            <td mat-cell *matCellDef="let run">{{ run.end_time ? formatTime(run.end_time) : '—' }}</td>
          </ng-container>

          <ng-container matColumnDef="duration">
            <th mat-header-cell *matHeaderCellDef mat-sort-header="duration_secs">Duration</th>
            <td mat-cell *matCellDef="let run">{{ formatDuration(run.duration_secs) }}</td>
          </ng-container>

          <ng-container matColumnDef="avg_rate">
            <th mat-header-cell *matHeaderCellDef class="numeric-cell">Avg Rate</th>
            <td mat-cell *matCellDef="let run" class="numeric-cell">{{ formatRate(run.stats.average_rate) }}</td>
          </ng-container>

          <ng-container matColumnDef="trigger_loss">
            <th mat-header-cell *matHeaderCellDef class="numeric-cell">Trigger Loss</th>
            <td mat-cell *matCellDef="let run" class="numeric-cell" [class.trigger-loss]="(run.stats.trigger_loss_count ?? 0) > 0">
              {{ formatCount(run.stats.trigger_loss_count ?? 0) }}
            </td>
          </ng-container>

          <tr mat-header-row *matHeaderRowDef="displayedColumns"></tr>
          <tr mat-row *matRowDef="let run; columns: displayedColumns"
              class="data-row"
              (click)="openRun(run.run_number)"></tr>
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
    .header-actions { display: flex; gap: 8px; }
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
    .trigger-loss { color: #f44336; font-weight: 500; }
  `,
})
export class RunsPageComponent implements OnInit {
  private http = inject(HttpClient);
  private router = inject(Router);

  readonly runs = signal<RunHistoryItem[]>([]);
  readonly loading = signal(false);
  readonly sortState = signal<Sort>({ active: 'start_time', direction: 'desc' });
  readonly exporting = signal(false);

  displayedColumns = ['run_number', 'comment', 'start_time', 'end_time', 'duration', 'avg_rate', 'trigger_loss'];

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

  exportJson(): void {
    this.exporting.set(true);
    this.http.get<unknown>('/api/runs/export').subscribe({
      next: (data) => {
        const json = JSON.stringify(data, null, 2);
        const blob = new Blob([json], { type: 'application/json' });
        const url = URL.createObjectURL(blob);
        const a = document.createElement('a');
        a.href = url;
        const exp = (data as { experiment?: string })?.experiment ?? 'export';
        const date = new Date().toISOString().slice(0, 10);
        a.download = `${exp}_runs_${date}.json`;
        a.click();
        URL.revokeObjectURL(url);
        this.exporting.set(false);
      },
      error: () => this.exporting.set(false),
    });
  }

  openRun(runNumber: number): void {
    this.router.navigate(['/runs', runNumber]);
  }

  onSortChange(sort: Sort): void {
    this.sortState.set(sort);
  }

  // --- Formatting helpers ---

  formatTime(ms: number): string {
    if (!ms) return '—';
    const d = new Date(ms);
    const month = d.toLocaleString('en', { month: 'short' });
    const day = d.getDate();
    const h = d.getHours().toString().padStart(2, '0');
    const m = d.getMinutes().toString().padStart(2, '0');
    return `${month} ${day} ${h}:${m}`;
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

  formatRate(rate: number): string {
    if (!rate) return '—';
    if (rate >= 1e6) return `${(rate / 1e6).toFixed(1)} M/s`;
    if (rate >= 1e3) return `${(rate / 1e3).toFixed(1)} K/s`;
    return `${rate.toFixed(0)} /s`;
  }

  formatCount(n: number): string {
    if (n === 0) return '0';
    if (n >= 1e9) return `${(n / 1e9).toFixed(1)}G`;
    if (n >= 1e6) return `${(n / 1e6).toFixed(1)}M`;
    if (n >= 1e3) return `${(n / 1e3).toFixed(1)}K`;
    return n.toString();
  }
}
