import { Component } from '@angular/core';
import { CommonModule } from '@angular/common';
import { MatTabsModule } from '@angular/material/tabs';
import { DigitizerSettingsComponent } from '../../components/digitizer-settings/digitizer-settings.component';
import { EventBuilderSettingsComponent } from '../../components/event-builder-settings/event-builder-settings.component';

// Emulator panel intentionally removed (audit EM, 2026-05-07): the
// data_source_emulator binary still exists and is used in dev, but its
// runtime knobs (events_per_batch, batch_interval_ms, num_modules,
// waveform sizes) are configured via config.toml — operator UI does not
// need to expose them. Backend `/api/emulator/*` routes are left in place
// as harmless leftovers; remove if a follow-up backend cleanup wants them.
@Component({
  selector: 'app-settings-page',
  standalone: true,
  imports: [CommonModule, MatTabsModule, DigitizerSettingsComponent, EventBuilderSettingsComponent],
  template: `
    <div class="settings-container">
      <mat-tab-group>
        <mat-tab label="Digitizers">
          <app-digitizer-settings />
        </mat-tab>
        <mat-tab label="Event Builder">
          <app-event-builder-settings />
        </mat-tab>
      </mat-tab-group>
    </div>
  `,
  styles: `
    .settings-container {
      padding: 16px;
      height: 100%;
    }
  `,
})
export class SettingsPageComponent {}
