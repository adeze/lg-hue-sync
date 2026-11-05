import { expect, test } from 'bun:test';
import { genreDiagnosticState, type DiagnosticSnapshot } from './diagnostics-types';

const snapshot: DiagnosticSnapshot = {
  sampled_unix_ms: 20_000,
  genre: {
    available: true,
    received_unix_ms: 20_000,
    scores: [100, 200, 300, 12_000],
    selected: 'Animation',
  },
};

test('genre scores preserve native order and priors without claiming confidence', () => {
  const result = genreDiagnosticState(snapshot, 20_500);
  expect(result.state).toBe('Available');
  expect(result.category).toBe('Animation');
  expect(result.scores).toEqual([100, 200, 300, 12_000]);
  expect(result.age).toBe(500);
  for (const genre of [
    { ...snapshot.genre!, scores: [0, 0, 0, 0] as [number, number, number, number] },
    { ...snapshot.genre!, selected: null },
    { ...snapshot.genre!, selected: 'Unknown' },
  ]) {
    const ambiguous = genreDiagnosticState({ ...snapshot, genre }, 20_500);
    expect(ambiguous.category).toBe('Unclassified');
  }
});

test('stale, future, offline and missing replies cannot classify the current picture', () => {
  for (const [value, now, online] of [
    [snapshot, 30_001, true],
    [{ ...snapshot, genre: { ...snapshot.genre!, received_unix_ms: 40_001 } }, 20_000, true],
    [snapshot, 20_500, false],
    [{ ...snapshot, sampled_unix_ms: 1 }, 20_500, true],
  ] as const) {
    const result = genreDiagnosticState(value, now, online);
    expect(result.state).toBe('Stale');
    expect(result.category).toBe('Unavailable');
    expect(result.scores).toBeNull();
  }
  for (const genre of [
    undefined,
    { ...snapshot.genre!, available: false },
    { ...snapshot.genre!, received_unix_ms: 0 },
    { ...snapshot.genre!, scores: null },
  ]) {
    expect(genreDiagnosticState({ ...snapshot, genre }, 20_500).state).toBe('Unavailable');
  }
});

test('malformed native scores are unavailable rather than a genre classification', () => {
  for (const scores of [
    [1, 2, 3],
    [1, 2, 3, -1],
    [1, 2, 3, NaN],
    [1, 2, 3, 0x1_0000_0000],
    [1, 2, 3, 4.5],
  ]) {
    const genre = { ...snapshot.genre!, scores } as NonNullable<DiagnosticSnapshot['genre']>;
    const result = genreDiagnosticState({ ...snapshot, genre }, 20_500);
    expect(result.state).toBe('Unavailable');
    expect(result.category).toBe('Unavailable');
  }
});
