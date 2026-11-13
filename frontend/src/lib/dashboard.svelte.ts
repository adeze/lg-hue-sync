import { request, SettingsWriter } from './api';
import type { LiveSettings } from './settings';
import type { PreviewStatus } from './preview-types';
import type { DiagnosticStatus } from './diagnostics-types';
export interface Status extends PreviewStatus, DiagnosticStatus {
  settings: LiveSettings;
  display_enabled?: boolean | null;
  active_preset: string | null;
  preset_modified: boolean;
  bonjour_enabled: boolean;
  hue_bridge_ip: string;
  nanoleaf_ip: string;
  calibration_pattern?: string | null;
}
export class Dashboard {
  status = $state.raw<Status | null>(null);
  settings = $state.raw<LiveSettings | null>(null);
  receivedAt = $state(0);
  display = $state<boolean | null>(null);
  busy = $state(false);
  saving = $state(false);
  error = $state('');
  notice = $state('');
  dirty = $state(false);
  online = $state(false);
  private events: EventSource | undefined;
  private saveTimer: ReturnType<typeof setTimeout> | undefined;
  private stopped = false;
  private polling = false;
  private revision = 0;
  private writer = new SettingsWriter<LiveSettings>(async (value) => {
    await request('/api/settings', value);
  });
  async refresh() {
    if (this.polling || this.stopped) return;
    this.polling = true;
    const revision = this.revision;
    try {
      const status = await request<Status>('/api/status');
      if (!status.settings || typeof status.pipeline_state !== 'string')
        throw new Error('Invalid status response');
      this.status = status;
      this.display = status.display_enabled ?? null;
      this.receivedAt = Date.now();
      this.online = true;
      if (!this.writer.pending && !this.busy && !this.dirty && revision === this.revision)
        this.settings = status.settings;
    } catch (error) {
      this.online = false;
      this.error = message(error);
    } finally {
      this.polling = false;
    }
  }
  start() {
    this.stopped = false;
    const events = new EventSource('/api/status/events');
    this.events = events;
    events.onmessage = ({ data }) => {
      const status: Status = JSON.parse(data);
      if (!status.settings || typeof status.pipeline_state !== 'string') return;
      this.status = status;
      this.display = status.display_enabled ?? null;
      this.receivedAt = Date.now();
      this.online = true;
      if (!this.writer.pending && !this.busy && !this.dirty) this.settings = status.settings;
    };
    events.onerror = () => {
      this.online = false;
      this.display = null;
    };
    return () => this.stop();
  }
  stop() {
    this.stopped = true;
    this.events?.close();
    clearTimeout(this.saveTimer);
  }
  patch(patch: Partial<LiveSettings>) {
    if (!this.settings || this.busy) return;
    this.revision++;
    this.settings = { ...this.settings, ...patch };
    this.dirty = true;
    this.error = '';
    const revision = this.revision;
    void this.writer
      .push(structuredClone(this.settings))
      .then(() => {
        if (revision === this.revision) this.scheduleSave();
      })
      .catch((error) => {
        this.error = message(error);
      });
  }
  private scheduleSave() {
    if (this.stopped) return;
    clearTimeout(this.saveTimer);
    this.saveTimer = setTimeout(() => void this.save(), 1000);
  }
  async save() {
    if (this.busy || this.saving) {
      this.scheduleSave();
      return;
    }
    clearTimeout(this.saveTimer);
    this.saving = true;
    const revision = this.revision;
    try {
      await this.writer.flush();
      await request('/api/save-config', undefined, 'POST');
      if (revision === this.revision) {
        this.dirty = false;
        this.notice = 'Saved automatically.';
      }
      await this.refresh();
    } catch (error) {
      this.error = 'Could not save: ' + message(error);
    } finally {
      this.saving = false;
    }
  }
  async preset(name: string, reset = false) {
    if (this.busy || (!reset && this.status?.active_preset === name)) return;
    this.busy = true;
    this.error = '';
    try {
      await this.writer.flush();
      await request(
        `/api/presets/${encodeURIComponent(name)}${reset ? '/reset' : ''}`,
        undefined,
        'POST',
      );
      this.revision++;
      const status = await request<Status>('/api/status');
      this.status = status;
      this.settings = status.settings;
      this.receivedAt = Date.now();
      this.dirty = true;
      this.scheduleSave();
    } catch (error) {
      this.error = message(error);
    } finally {
      this.busy = false;
    }
  }
  async command<T = unknown>(path: string, body?: unknown): Promise<T | undefined> {
    if (this.busy) return;
    this.busy = true;
    this.error = '';
    try {
      await this.writer.flush();
      const result = await request<T>(path, body, 'POST');
      this.notice = 'Applied.';
      return result;
    } catch (error) {
      this.error = message(error);
      throw error;
    } finally {
      this.busy = false;
      await this.refresh();
    }
  }
  get requested() {
    return (
      !!this.status &&
      (this.status.pipeline_state === 'running' ||
        (!['paused', 'faulted'].includes(this.status.pipeline_state) &&
          this.status.requested_running === true))
    );
  }
}
export function message(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}
