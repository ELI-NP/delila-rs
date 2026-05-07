import { Component } from '@angular/core';
import { MatSnackBarModule } from '@angular/material/snack-bar';
import { StatusPanelComponent } from '../../components/status-panel/status-panel.component';
import { ControlPanelComponent } from '../../components/control-panel/control-panel.component';
import { RunInfoComponent } from '../../components/run-info/run-info.component';

@Component({
  selector: 'app-control-page',
  standalone: true,
  imports: [
    MatSnackBarModule,
    StatusPanelComponent,
    ControlPanelComponent,
    RunInfoComponent,
  ],
  template: `
    <div class="control-content">
      <div class="left-column">
        <app-status-panel></app-status-panel>
        <app-run-info></app-run-info>
      </div>
      <div class="right-column">
        <app-control-panel></app-control-panel>
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
export class ControlPageComponent {}
