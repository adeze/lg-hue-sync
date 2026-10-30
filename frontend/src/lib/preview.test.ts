import { expect, test } from 'bun:test';
import { FrozenPreview, groupHueZones, hueOrientation, validatePreviewFrame } from './preview';
import type { HueZone, PreviewFrame, PreviewSettings, PreviewState } from './preview-types';

const frame: PreviewFrame = { width: 1, height: 1, rgb: [10, 20, 30], captured_unix_ms: 1000 };
const settings: PreviewSettings = {
  brightness_multiplier: 1,
  saturation_boost: 1,
  gamma: 1,
  noise_gate_threshold: 0.02,
  hdr_tone_mapping: false,
};
const response = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });
async function until(predicate: () => boolean): Promise<void> {
  const deadline = Date.now() + 2000;
  while (!predicate() && Date.now() < deadline) await Bun.sleep(10);
  expect(predicate()).toBe(true);
}

test('default capture transport does not bind browser fetch to the controller', async () => {
  const original = globalThis.fetch;
  let latest: PreviewState | undefined;
  const preview = new FrozenPreview((state) => {
    latest = state;
  });
  try {
    globalThis.fetch = Object.assign(
      async function (this: unknown) {
        if (this !== undefined && this !== globalThis) throw new TypeError('Illegal invocation');
        return response(frame);
      },
      { preconnect: original.preconnect },
    );
    preview.update(settings, true, true);
    preview.capture();
    await until(() => latest?.rendered !== null && latest?.rendered !== undefined);
    expect(latest?.frame).toEqual(frame);
  } finally {
    preview.cancel();
    globalThis.fetch = original;
  }
});

test('bounded frames and device grouping preserve authoritative channel geometry', () => {
  validatePreviewFrame(frame);
  for (const invalid of [
    { ...frame, width: 129 },
    { ...frame, rgb: [0, 1] },
    { ...frame, rgb: [0, -1, 2] },
    { ...frame, captured_unix_ms: 0 },
  ]) {
    expect(() => validatePreviewFrame(invalid)).toThrow();
  }
  const zone: HueZone = {
    channel_id: 4,
    name: 'Strip · Segments 1–2',
    hue_device_id: 'strip',
    x_min: 0.6,
    x_max: 1,
    y_min: 0.2,
    y_max: 0.8,
    hue_positions: [
      [1, -0.7, 0],
      [1, 0.65, 0],
    ],
  };
  const zones = [
    zone,
    { ...zone, channel_id: 5, name: 'Strip · Segments 3–7', x_min: 0.55 },
    { ...zone, channel_id: 6, hue_device_id: null },
  ];
  const before = JSON.stringify(zones);
  const devices = groupHueZones(zones);
  expect(devices).toHaveLength(2);
  expect(devices[0].name).toBe('Strip');
  expect(devices[0].channels.map((zone) => zone.channel_id)).toEqual([4, 5]);
  expect(devices[0].x_min).toBe(0.55);
  expect(devices[0].positions).toHaveLength(2);
  expect(JSON.stringify(zones)).toBe(before);
  expect(hueOrientation(devices[0].positions)).toBe('Front ↔ back');
  expect(
    hueOrientation([
      [0, 0, 0],
      [0, 0, 1],
    ]),
  ).toBe('Up ↔ down');
  expect(
    hueOrientation([
      [0, 0, 0],
      [1, 1, 0],
    ]),
  ).toBe('Angled');
  expect(hueOrientation([])).toBe('');
});

test('frozen preview serializes capture/render, discards stale work and cancels while hidden', async () => {
  let latest: PreviewState | undefined;
  const renders: { settings: PreviewSettings; resolve: (response: Response) => void }[] = [];
  let captures = 0;
  const fetcher = (async (path: string, options?: RequestInit) => {
    if (path.endsWith('/frame'))
      return response(captures++ === 0 ? {} : frame, captures === 1 ? 202 : 200);
    return await new Promise<Response>((resolve) =>
      renders.push({ settings: JSON.parse(String(options?.body)).settings, resolve }),
    );
  }) as typeof fetch;
  const preview = new FrozenPreview((state) => {
    latest = state;
  }, fetcher);
  preview.update(settings, true, true);
  preview.capture();
  await until(() => renders.length === 1);
  expect(captures).toBe(2);
  preview.update({ ...settings, gamma: 2 }, true, true);
  preview.update({ ...settings, gamma: 3 }, true, true);
  await Bun.sleep(220);
  expect(renders).toHaveLength(1);
  renders[0].resolve(response({ ...frame, rgb: [40, 50, 60] }));
  await until(() => renders.length === 2);
  expect(latest?.rendered).toBeNull();
  expect(renders[1].settings.gamma).toBe(3);
  renders[1].resolve(response({ ...frame, rgb: [70, 80, 90] }));
  await until(() => latest?.rendered?.rgb[0] === 70);
  expect(latest?.frame?.rgb).toEqual(frame.rgb);
  preview.update({ ...settings, gamma: 4 }, true, false);
  preview.update({ ...settings, gamma: 4 }, false, false);
  await Bun.sleep(220);
  expect(renders).toHaveLength(2);
  expect(captures).toBe(2);
  preview.update({ ...settings, gamma: 4 }, true, false);
  await until(() => renders.length === 3);
  renders[2].resolve(response({ error: 'Cannot render this frame.' }, 422));
  await until(() => latest?.message === 'Cannot render this frame.');
  expect(latest?.rendered?.rgb).toEqual([70, 80, 90]);
  expect(captures).toBe(2);
  preview.cancel();

  let capturedResponse: ((response: Response) => void) | undefined;
  let signal: AbortSignal | null | undefined;
  const cancelled = new FrozenPreview(
    (state) => {
      latest = state;
    },
    (async (_path: string, options?: RequestInit) => {
      signal = options?.signal;
      return await new Promise<Response>((resolve) => {
        capturedResponse = resolve;
      });
    }) as typeof fetch,
  );
  cancelled.update(settings, true, true);
  cancelled.capture();
  cancelled.update(settings, false, true);
  expect(signal?.aborted).toBe(true);
  capturedResponse?.(response(frame));
  await until(() => latest?.busy === false);
  expect(latest?.frame).toBeNull();
  cancelled.cancel();
});
