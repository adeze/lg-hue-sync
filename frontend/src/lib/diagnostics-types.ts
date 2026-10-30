export interface VideoDiagnostics {
  available: boolean;
  input?: string | null;
  width?: number | null;
  height?: number | null;
  frame_rate?: number | null;
  scan_type?: string | null;
  hdr?: string | null;
  pixel_encoding?: string | null;
  range?: string | null;
  colourimetry?: string | null;
  bit_depth?: number | null;
}

export interface DisplayDiagnostics {
  available: boolean;
  source?: string | null;
  width?: number | null;
  height?: number | null;
  refresh_rate?: number | null;
  enabled?: boolean | null;
  muted?: boolean | null;
  locked?: boolean | null;
}

export interface DiagnosticSnapshot {
  sampled_unix_ms: number;
  display?: DisplayDiagnostics;
  video?: VideoDiagnostics;
  player?: VideoDiagnostics;
  output?: {
    available: boolean;
    arc_state?: string | null;
    passthrough?: boolean | null;
  };
}

export interface DiagnosticStatus {
  diagnostics?: DiagnosticSnapshot | null;
  pipeline_state: string;
  capture_resolution: string;
  fps: number;
  light_update_fps: number;
  capture_hardware: boolean;
  hue_connected: boolean;
  nanoleaf_connected: boolean;
  tv_power_state?: string;
  is_syncing?: boolean;
  requested_running?: boolean;
  last_error?: string | null;
  settings?: {
    hue_sync_enabled?: boolean;
    nanoleaf_sync_enabled?: boolean;
  };
}

export function diagnosticValue(
  value: string | number | boolean | null | undefined,
  suffix = '',
): string {
  if (value === null || value === undefined || value === '') return 'Unavailable';
  if (typeof value === 'boolean') return value ? 'Yes' : 'No';
  return String(value) + suffix;
}

export function diagnosticRate(value: number | null | undefined, suffix = ' Hz'): string {
  return typeof value === 'number' && Number.isFinite(value)
    ? String(Number(value.toFixed(3))) + suffix
    : 'Unavailable';
}

export function snapshotState(snapshot: DiagnosticSnapshot | null | undefined, now: number) {
  const sampled = snapshot?.sampled_unix_ms;
  const available = typeof sampled === 'number' && Number.isFinite(sampled) && sampled > 0;
  const age = available ? Math.max(0, now - sampled) : 0;
  const stale = available && (age > 10_000 || sampled > now + 10_000);
  return { available, stale, age };
}
