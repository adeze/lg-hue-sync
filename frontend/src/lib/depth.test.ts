import { expect, test } from 'bun:test';
import { demoSamplingBounds } from '../../preview/mock-api';
import { initialSettings } from './settings';

test('simulated depth footprints stay bounded, symmetric and independent of other settings', () => {
  expect(initialSettings.depth_influence).toBe(0.5);
  for (const [strength, expected] of [
    [0, 0.25],
    [0.5, 0.3125],
    [1, 0.375],
  ]) {
    const rear = demoSamplingBounds([0, -1, 0], strength);
    expect(rear.x_max - rear.x_min).toBeCloseTo(expected);
    expect(rear.y_max - rear.y_min).toBeCloseTo(expected);
    const front = demoSamplingBounds([0, 1, 0], strength);
    expect(front.x_max - front.x_min).toBe(0.25);
  }
  const left = demoSamplingBounds([-1, -0.6, 0], 1);
  const right = demoSamplingBounds([1, -0.6, 0], 1);
  expect(left.x_min).toBe(0);
  expect(right.x_max).toBe(1);
  expect(left.x_max - left.x_min).toBeCloseTo(right.x_max - right.x_min);
  expect(left.y_min).toBe(right.y_min);
  expect(left.y_max).toBe(right.y_max);
});
