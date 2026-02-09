import { Injectable, inject, signal } from '@angular/core';
import { HttpClient } from '@angular/common/http';
import { firstValueFrom } from 'rxjs';
import { EventBuilderConfig, EventBuilderHistoryItem, ChSettings, TimeCalibration, L2Setting } from '../models/types';

@Injectable({
  providedIn: 'root'
})
export class EventBuilderService {
  private readonly http = inject(HttpClient);
  private readonly apiUrl = '/api/event-builder';

  // Signals for reactive state
  experiments = signal<string[]>([]);
  configs = signal<EventBuilderConfig[]>([]);
  currentConfig = signal<EventBuilderConfig | null>(null);
  loading = signal(false);
  error = signal<string | null>(null);

  /** Load list of experiments */
  async loadExperiments(): Promise<void> {
    this.loading.set(true);
    this.error.set(null);
    try {
      const experiments = await firstValueFrom(
        this.http.get<string[]>(`${this.apiUrl}/experiments`)
      );
      this.experiments.set(experiments);
    } catch (err) {
      console.error('Failed to load experiments:', err);
      this.error.set('Failed to load experiments');
      this.experiments.set([]);
    } finally {
      this.loading.set(false);
    }
  }

  /** Load configs for an experiment */
  async loadConfigs(expName: string): Promise<void> {
    this.loading.set(true);
    this.error.set(null);
    try {
      const configs = await firstValueFrom(
        this.http.get<EventBuilderConfig[]>(`${this.apiUrl}/configs`, {
          params: { exp_name: expName }
        })
      );
      this.configs.set(configs);
    } catch (err) {
      console.error('Failed to load configs:', err);
      this.error.set('Failed to load configurations');
      this.configs.set([]);
    } finally {
      this.loading.set(false);
    }
  }

  /** Load a specific config */
  async loadConfig(expName: string, name: string): Promise<EventBuilderConfig | null> {
    this.loading.set(true);
    this.error.set(null);
    try {
      const config = await firstValueFrom(
        this.http.get<EventBuilderConfig>(`${this.apiUrl}/configs/${expName}/${name}`)
      );
      this.currentConfig.set(config);
      return config;
    } catch (err) {
      console.error('Failed to load config:', err);
      this.error.set('Failed to load configuration');
      this.currentConfig.set(null);
      return null;
    } finally {
      this.loading.set(false);
    }
  }

  /** Create or update a config */
  async saveConfig(config: Partial<EventBuilderConfig> & { name: string; exp_name: string; ch_settings: ChSettings[][] }): Promise<EventBuilderConfig | null> {
    this.loading.set(true);
    this.error.set(null);
    try {
      const saved = await firstValueFrom(
        this.http.post<EventBuilderConfig>(`${this.apiUrl}/configs`, config)
      );
      this.currentConfig.set(saved);
      // Refresh config list
      await this.loadConfigs(config.exp_name);
      return saved;
    } catch (err) {
      console.error('Failed to save config:', err);
      this.error.set('Failed to save configuration');
      return null;
    } finally {
      this.loading.set(false);
    }
  }

  /** Update chSettings only */
  async updateChSettings(expName: string, name: string, chSettings: ChSettings[][]): Promise<EventBuilderConfig | null> {
    this.loading.set(true);
    this.error.set(null);
    try {
      const updated = await firstValueFrom(
        this.http.put<EventBuilderConfig>(
          `${this.apiUrl}/configs/${expName}/${name}/ch-settings`,
          { ch_settings: chSettings }
        )
      );
      this.currentConfig.set(updated);
      return updated;
    } catch (err) {
      console.error('Failed to update chSettings:', err);
      this.error.set('Failed to update channel settings');
      return null;
    } finally {
      this.loading.set(false);
    }
  }

  /** Update timeSettings only */
  async updateTimeSettings(expName: string, name: string, timeSettings: TimeCalibration): Promise<EventBuilderConfig | null> {
    this.loading.set(true);
    this.error.set(null);
    try {
      const updated = await firstValueFrom(
        this.http.put<EventBuilderConfig>(
          `${this.apiUrl}/configs/${expName}/${name}/time-settings`,
          { time_settings: timeSettings }
        )
      );
      this.currentConfig.set(updated);
      return updated;
    } catch (err) {
      console.error('Failed to update timeSettings:', err);
      this.error.set('Failed to update time settings');
      return null;
    } finally {
      this.loading.set(false);
    }
  }

  /** Update L2Settings only */
  async updateL2Settings(expName: string, name: string, l2Settings: L2Setting[]): Promise<EventBuilderConfig | null> {
    this.loading.set(true);
    this.error.set(null);
    try {
      const updated = await firstValueFrom(
        this.http.put<EventBuilderConfig>(
          `${this.apiUrl}/configs/${expName}/${name}/l2-settings`,
          { l2_settings: l2Settings }
        )
      );
      this.currentConfig.set(updated);
      return updated;
    } catch (err) {
      console.error('Failed to update L2Settings:', err);
      this.error.set('Failed to update L2 settings');
      return null;
    } finally {
      this.loading.set(false);
    }
  }

  /** Get version history */
  async getHistory(expName: string, name: string, limit = 20): Promise<EventBuilderHistoryItem[]> {
    try {
      return await firstValueFrom(
        this.http.get<EventBuilderHistoryItem[]>(
          `${this.apiUrl}/configs/${expName}/${name}/history`,
          { params: { limit: limit.toString() } }
        )
      );
    } catch (err) {
      console.error('Failed to get history:', err);
      return [];
    }
  }

  /** Restore a version */
  async restoreVersion(expName: string, name: string, version: number): Promise<EventBuilderConfig | null> {
    this.loading.set(true);
    this.error.set(null);
    try {
      const restored = await firstValueFrom(
        this.http.post<EventBuilderConfig>(
          `${this.apiUrl}/configs/${expName}/${name}/restore`,
          { version }
        )
      );
      this.currentConfig.set(restored);
      return restored;
    } catch (err) {
      console.error('Failed to restore version:', err);
      this.error.set('Failed to restore version');
      return null;
    } finally {
      this.loading.set(false);
    }
  }

  /** Delete a config */
  async deleteConfig(expName: string, name: string): Promise<boolean> {
    this.loading.set(true);
    this.error.set(null);
    try {
      await firstValueFrom(
        this.http.delete(`${this.apiUrl}/configs/${expName}/${name}`)
      );
      // Refresh config list
      await this.loadConfigs(expName);
      if (this.currentConfig()?.name === name && this.currentConfig()?.exp_name === expName) {
        this.currentConfig.set(null);
      }
      return true;
    } catch (err) {
      console.error('Failed to delete config:', err);
      this.error.set('Failed to delete configuration');
      return false;
    } finally {
      this.loading.set(false);
    }
  }

  /** Create empty chSettings for a new config */
  createEmptyChSettings(modules: number, channelsPerModule: number): ChSettings[][] {
    const settings: ChSettings[][] = [];
    let id = 0;
    for (let m = 0; m < modules; m++) {
      const moduleSettings: ChSettings[] = [];
      for (let c = 0; c < channelsPerModule; c++) {
        moduleSettings.push({
          ID: id++,
          Module: m,
          Channel: c,
          IsEventTrigger: false,
          ThresholdADC: 0,
          HasAC: false,
          ACModule: 128,
          ACChannel: 128,
          DetectorType: 'Unknown',
          Tags: [],
          P0: 0,
          P1: 1,
          P2: 0,
          P3: 0,
        });
      }
      settings.push(moduleSettings);
    }
    return settings;
  }
}
