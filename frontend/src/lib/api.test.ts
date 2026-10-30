import { describe, test, expect } from 'bun:test';
import { SettingsWriter } from './api';
import { initialSettings, nearestStop, responseMode, blendMode } from './settings';
describe('settings migration invariants', () => {
  test('complete snapshots serialize and collapse queued edits to the newest', async () => {
    const writes: number[] = [];
    let release: () => void = () => {};
    const writer = new SettingsWriter<number>(async (value) => {
      writes.push(value);
      if (value === 1) await new Promise<void>((resolve) => (release = resolve));
    });
    const pending = writer.push(1);
    writer.push(2);
    writer.push(3);
    expect(writes).toEqual([1]);
    release();
    await pending;
    expect(writes).toEqual([1, 3]);
    expect(writer.pending).toBe(false);
  });
  test('failed writes remain retryable, preserving the newest complete snapshot', async () => {
    let fail = true;
    const writes: number[] = [];
    const writer = new SettingsWriter<number>(async (value) => {
      if (fail) throw Error('offline');
      writes.push(value);
    });
    await expect(writer.push(4)).rejects.toThrow('offline');
    fail = false;
    await writer.flush();
    expect(writes).toEqual([4]);
  });
  test('exact tuning survives display and dimensions remain independently identifiable', () => {
    const exact = { ...initialSettings, saturation_boost: 1.65, gamma: 1.18 };
    expect(nearestStop(exact.saturation_boost, [0.5, 1, 1.5, 2, 2.5])).toBe(2);
    expect(exact.saturation_boost).toBe(1.65);
    expect(responseMode(exact)).toBe('default');
    expect(blendMode(exact)).toBe('balanced');
    expect(responseMode({ ...exact, max_color_step: 19 })).toBe('');
    expect(blendMode({ ...exact, spatial_blend: 0.3 })).toBe('soft');
  });
});
