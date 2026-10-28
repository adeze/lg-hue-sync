import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const html = readFileSync(new URL('../src/web/ui.html', import.meta.url), 'utf8');
const source = html.slice(html.indexOf('    function groupHueZones('), html.indexOf('    function updateHueZones('));
const { groupHueZones, hueOrientation, hueOrientationGlyph } = new Function(source + '; return { groupHueZones, hueOrientation, hueOrientationGlyph };')();
const zone = { channel_id: 4, hue_device_id: 'strip', name: 'Strip · Segments 1–2',
  x_min: 0.6, x_max: 1, y_min: 0.2, y_max: 0.8, hue_positions: [[1, -0.7, 0], [1, 0.65, 0]] };
const zones = [zone, { ...zone, channel_id: 5, name: 'Strip · Segments 3–7', x_min: 0.55, y_min: 0.35, y_max: 0.65 },
  { ...zone, channel_id: 6, hue_device_id: 'other-strip', name: 'Other strip' },
  { ...zone, channel_id: 7, hue_device_id: null }, { ...zone, channel_id: 8, hue_device_id: null }];
const original = JSON.stringify(zones);
const devices = groupHueZones(zones);
assert.equal(devices.length, 4);
assert.equal(devices[0].name, 'Strip');
assert.deepEqual(devices[0].channels.map(channel => channel.channel_id), [4, 5]);
assert.deepEqual([devices[0].x_min, devices[0].x_max, devices[0].y_min, devices[0].y_max], [0.55, 1, 0.2, 0.8]);
assert.equal(devices[0].positions.length, 2);
assert.equal(JSON.stringify(zones), original, 'Preview must not mutate sampling metadata');
assert.deepEqual(groupHueZones([]), []);
for (const [points, expected] of [
  [[], ''], [[[0, 0, 0]], ''], [[[0, 0, 0], [0, 0, 0]], ''],
  [[[1, -0.7, 0], [1, 0.65, 0]], 'Front ↔ back'],
  [[[0, 0, -1], [0, 0, 1]], 'Up ↔ down'],
  [[[-1, 0, 0], [1, 0, 0]], 'Left ↔ right'],
  [[[0, 0, 0], [1, 1, 0]], 'Angled'],
  [[[0, 0, 0], [1, 0, 0], [1, 1, 0]], 'Multi-point layout'],
]) assert.equal(hueOrientation(points), expected);
assert.match(hueOrientationGlyph('Front ↔ back'), /<rect/);
assert.match(hueOrientationGlyph('Front ↔ back'), /<svg/);
assert.equal(hueOrientationGlyph(''), '');
console.log('Device grouping, bounds, legacy fallback, immutability and optional orientation passed.');
