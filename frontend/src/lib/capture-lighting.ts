import type { DiagnosticStatus } from './diagnostics-types';

export const rateScale = 120;
const rate = (value: unknown, positive = false): number | null =>
  typeof value === 'number' && Number.isFinite(value) && (positive ? value > 0 : value >= 0)
    ? value
    : null;

export function captureLightingState(
  status: DiagnosticStatus | null,
  receivedAt: number,
  now: number,
  online = true,
) {
  const knownReceipt = Number.isFinite(receivedAt) && receivedAt > 0;
  const fresh = Boolean(
    status &&
    online &&
    knownReceipt &&
    Number.isFinite(now) &&
    now - receivedAt >= -10_000 &&
    now - receivedAt <= 10_000,
  );
  const running = fresh && status?.pipeline_state === 'running' && status.is_syncing !== false;
  const paused = fresh && ['paused', 'unsupported_source'].includes(status?.pipeline_state ?? '');
  const captureCeiling = fresh ? rate(status?.fps, true) : null;
  const lightUpdates = running ? rate(status?.light_update_fps) : paused ? 0 : null;
  const sender = (enabled: boolean | undefined, connected: boolean | undefined) => {
    if (!fresh) return 'Unavailable';
    if (enabled === false) return 'Disabled';
    if (paused) return 'Paused';
    if (!running || typeof connected !== 'boolean') return 'Unavailable';
    return connected ? 'Connected' : 'Disconnected';
  };
  return {
    fresh,
    running,
    phase: !fresh
      ? status && knownReceipt
        ? 'Stale / offline'
        : 'Unavailable'
      : status!.pipeline_state.replaceAll('_', ' '),
    captureCeiling,
    lightUpdates,
    captureArc:
      running && captureCeiling !== null ? Math.min(100, (captureCeiling / rateScale) * 100) : 0,
    lightsArc:
      running && lightUpdates !== null ? Math.min(100, (lightUpdates / rateScale) * 100) : 0,
    hue: sender(status?.settings?.hue_sync_enabled, status?.hue_connected),
    nanoleaf: sender(status?.settings?.nanoleaf_sync_enabled, status?.nanoleaf_connected),
  };
}
