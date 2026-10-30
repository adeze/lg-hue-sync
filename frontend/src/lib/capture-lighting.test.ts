import { expect, test } from 'bun:test';
import { captureLightingState } from './capture-lighting';
import type { DiagnosticStatus } from './diagnostics-types';

const status: DiagnosticStatus = {
  pipeline_state: 'running',
  capture_resolution: '320x180',
  fps: 60,
  light_update_fps: 49.75,
  capture_hardware: true,
  hue_connected: true,
  nanoleaf_connected: false,
  is_syncing: true,
};

test('rates remain independent and unavailable receipts never appear live', () => {
  const live = captureLightingState(status, 20_000, 20_500);
  expect(live.captureCeiling).toBe(60);
  expect(live.lightUpdates).toBe(49.75);
  expect(live.captureArc).toBe(50);
  expect(live.hue).toBe('Connected');
  expect(live.nanoleaf).toBe('Disconnected');
  for (const [receipt, now, online] of [
    [0, 20_500, true],
    [20_000, 30_001, true],
    [40_000, 20_500, true],
    [20_000, 20_500, false],
  ] as const) {
    const unavailable = captureLightingState(status, receipt, now, online);
    expect(unavailable.running).toBe(false);
    expect(unavailable.captureCeiling).toBeNull();
    expect(unavailable.lightUpdates).toBeNull();
    expect(unavailable.hue).toBe('Unavailable');
    expect(unavailable.captureArc + unavailable.lightsArc).toBe(0);
  }
});

test('paused, disabled and invalid readings do not imply healthy light output', () => {
  const paused = captureLightingState(
    { ...status, pipeline_state: 'paused', is_syncing: false },
    20_000,
    20_500,
  );
  expect(paused.captureCeiling).toBe(60);
  expect(paused.lightUpdates).toBe(0);
  expect(paused.hue).toBe('Paused');
  expect(paused.captureArc + paused.lightsArc).toBe(0);
  const disabled = captureLightingState(
    { ...status, settings: { hue_sync_enabled: false } },
    20_000,
    20_500,
  );
  expect(disabled.hue).toBe('Disabled');
  const invalid = captureLightingState(
    { ...status, fps: NaN, light_update_fps: -1 },
    20_000,
    20_500,
  );
  expect(invalid.captureCeiling).toBeNull();
  expect(invalid.lightUpdates).toBeNull();
  const excessive = captureLightingState({ ...status, fps: 200 }, 20_000, 20_500);
  expect(excessive.captureCeiling).toBe(200);
  expect(excessive.captureArc).toBe(100);
});
