import { expect, test } from 'bun:test';
import { assignColors, compositeSvg, overlaps } from './generate';

test('overlaps receive distinct primaries and their shared tile is additive', () => {
  const zones = [
    { channel_id: 0, name: 'left', x_min: 0, x_max: 0.6, y_min: 0, y_max: 1 },
    { channel_id: 1, name: 'right', x_min: 0.4, x_max: 1, y_min: 0, y_max: 1 },
  ];
  expect(overlaps(zones[0], zones[1])).toBe(true);
  const colors = assignColors(zones);
  expect(colors.get(0)).toEqual([255, 0, 0]);
  expect(colors.get(1)).toEqual([0, 255, 0]);
  expect(compositeSvg(zones, colors)).toContain('fill="#ffff00"');
});
