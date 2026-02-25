import { Component, inject, signal, effect, untracked } from '@angular/core';
import { CommonModule } from '@angular/common';
import { MatCardModule } from '@angular/material/card';
import { MatListModule } from '@angular/material/list';
import { MatIconModule } from '@angular/material/icon';
import { MatTooltipModule } from '@angular/material/tooltip';
import { OperatorService } from '../../services/operator.service';
import { ComponentState } from '../../models/types';

@Component({
  selector: 'app-status-panel',
  standalone: true,
  imports: [CommonModule, MatCardModule, MatListModule, MatIconModule, MatTooltipModule],
  template: `
    <mat-card>
      <mat-card-header>
        <mat-card-title>Component Status</mat-card-title>
      </mat-card-header>
      <mat-card-content>
        <!-- Readers group (collapsible) -->
        @if (operator.sourceComponents().length > 0) {
          <div class="group-header" role="button" tabindex="0" (click)="readersExpanded.set(!readersExpanded())" (keydown.enter)="readersExpanded.set(!readersExpanded())" (keydown.space)="readersExpanded.set(!readersExpanded())">
            <mat-icon class="expand-icon">
              {{ readersExpanded() ? 'expand_more' : 'chevron_right' }}
            </mat-icon>
            <span class="group-title">Readers</span>
            <span class="group-summary" [class.has-error]="operator.sourceSummary().hasError">
              {{ operator.sourceSummary().online }}/{{ operator.sourceSummary().total }} Online
              @if (operator.sourceSummary().totalRate > 0) {
                · {{ formatRate(operator.sourceSummary().totalRate) }}
              }
            </span>
          </div>
          @if (readersExpanded()) {
            <mat-list dense>
              @for (component of operator.sourceComponents(); track component.name) {
                <mat-list-item [matTooltip]="component.error ?? ''" [matTooltipDisabled]="!component.error">
                  <mat-icon matListItemIcon [style.color]="getStateColor(component.state, component.online)">
                    {{ getStateIcon(component.state, component.online) }}
                  </mat-icon>
                  <span matListItemTitle>{{ component.name }}</span>
                  <span matListItemLine>
                    {{ component.state }}
                    @if (component.metrics && component.metrics.events_processed > 0) {
                      <span class="metrics-inline">
                        · {{ formatEvents(component.metrics.events_processed) }}
                        @if (component.metrics.event_rate > 0) {
                          · {{ formatRate(component.metrics.event_rate) }}
                        }
                        @if (component.metrics.bytes_transferred > 0) {
                          · {{ formatBytes(component.metrics.bytes_transferred) }}
                        }
                        @if (component.metrics.trigger_loss_count && component.metrics.trigger_loss_count > 0) {
                          <span class="trigger-loss">
                            · LOSS: {{ formatEvents(component.metrics.trigger_loss_count) }}
                            @if (component.metrics.trigger_loss_rate && component.metrics.trigger_loss_rate > 0.01) {
                              ({{ component.metrics.trigger_loss_rate.toFixed(2) }}%)
                            }
                          </span>
                        }
                      </span>
                    }
                  </span>
                </mat-list-item>
              }
            </mat-list>
          }
        }

        <!-- Pipeline group (always visible) -->
        @if (operator.pipelineComponents().length > 0) {
          <div class="group-header pipeline-header">
            <mat-icon class="expand-icon">expand_more</mat-icon>
            <span class="group-title">Pipeline</span>
          </div>
          <mat-list dense>
            @for (component of operator.pipelineComponents(); track component.name) {
              <mat-list-item [matTooltip]="component.error ?? ''" [matTooltipDisabled]="!component.error">
                <mat-icon matListItemIcon [style.color]="getStateColor(component.state, component.online)">
                  {{ getStateIcon(component.state, component.online) }}
                </mat-icon>
                <span matListItemTitle>{{ component.name }}</span>
                <span matListItemLine>
                  {{ component.state }}
                  @if (component.metrics && component.metrics.events_processed > 0) {
                    <span class="metrics-inline">
                      · {{ formatEvents(component.metrics.events_processed) }}
                      @if (component.metrics.event_rate > 0) {
                        · {{ formatRate(component.metrics.event_rate) }}
                      }
                      @if (component.metrics.bytes_transferred > 0) {
                        · {{ formatBytes(component.metrics.bytes_transferred) }}
                      }
                    </span>
                  }
                </span>
              </mat-list-item>
            }
          </mat-list>
        }

        @if (operator.components().length === 0) {
          <mat-list>
            <mat-list-item>
              <span matListItemTitle>No components</span>
            </mat-list-item>
          </mat-list>
        }
      </mat-card-content>
    </mat-card>
  `,
  styles: `
    mat-card {
      height: 100%;
    }
    .group-header {
      display: flex;
      align-items: center;
      padding: 8px 16px 0;
      cursor: pointer;
      user-select: none;
    }
    .group-header:hover {
      background: rgba(0, 0, 0, 0.04);
    }
    .pipeline-header {
      cursor: default;
    }
    .pipeline-header:hover {
      background: transparent;
    }
    .expand-icon {
      font-size: 20px;
      width: 20px;
      height: 20px;
      margin-right: 4px;
      color: #666;
    }
    .group-title {
      font-weight: 500;
      font-size: 14px;
      margin-right: 8px;
    }
    .group-summary {
      color: #666;
      font-size: 13px;
    }
    .group-summary.has-error {
      color: #f44336;
    }
    mat-list-item {
      cursor: default;
    }
    .metrics-inline {
      color: #666;
      font-size: 0.85em;
    }
    .trigger-loss {
      color: #ff9800;
      font-weight: 500;
    }
  `,
})
export class StatusPanelComponent {
  readonly operator = inject(OperatorService);
  readonly readersExpanded = signal(false);

  constructor() {
    // Auto-expand readers when there's an error
    effect(() => {
      const summary = this.operator.sourceSummary();
      if (summary.hasError) {
        untracked(() => this.readersExpanded.set(true));
      }
    });
  }

  getStateColor(state: ComponentState, online: boolean): string {
    if (!online) return '#9e9e9e'; // grey
    switch (state) {
      case 'Running':
        return '#4caf50'; // green
      case 'Error':
        return '#f44336'; // red
      case 'Configured':
      case 'Armed':
        return '#2196f3'; // blue
      case 'Configuring':
      case 'Arming':
      case 'Starting':
      case 'Stopping':
        return '#ff9800'; // orange
      default:
        return '#9e9e9e'; // grey
    }
  }

  getStateIcon(state: ComponentState, online: boolean): string {
    if (!online) return 'cloud_off';
    switch (state) {
      case 'Running':
        return 'play_circle';
      case 'Error':
        return 'error';
      case 'Configured':
      case 'Armed':
        return 'check_circle';
      case 'Idle':
        return 'radio_button_unchecked';
      default:
        return 'pending';
    }
  }

  formatEvents(count: number): string {
    if (count >= 1_000_000_000) {
      return `${(count / 1_000_000_000).toFixed(2)}G`;
    } else if (count >= 1_000_000) {
      return `${(count / 1_000_000).toFixed(2)}M`;
    } else if (count >= 1_000) {
      return `${(count / 1_000).toFixed(1)}k`;
    }
    return count.toString();
  }

  formatBytes(bytes: number): string {
    if (bytes >= 1_073_741_824) {
      return `${(bytes / 1_073_741_824).toFixed(2)} GB`;
    } else if (bytes >= 1_048_576) {
      return `${(bytes / 1_048_576).toFixed(1)} MB`;
    } else if (bytes >= 1_024) {
      return `${(bytes / 1_024).toFixed(0)} KB`;
    }
    return `${bytes} B`;
  }

  formatRate(rate: number): string {
    if (rate >= 1_000_000) {
      return `${(rate / 1_000_000).toFixed(2)}M eve/s`;
    } else if (rate >= 1_000) {
      return `${(rate / 1_000).toFixed(1)}k eve/s`;
    }
    return `${rate.toFixed(0)} eve/s`;
  }
}
