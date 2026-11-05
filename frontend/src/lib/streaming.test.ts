import { expect, test } from 'bun:test';
import { companionOrigin } from './streaming';

test('companion origins normalize hosts and reject credentialed or executable URLs', () => {
  expect(companionOrigin(' https://example.test:18090/ ')).toBe('https://example.test:18090');
  expect(companionOrigin('http://[::1]:18090')).toBe('http://[::1]:18090');
  for (const value of [
    '',
    'javascript:alert(1)',
    'file:///tmp',
    'http://user:secret@example.test',
    'https://example.test/path',
    'https://example.test?x=1',
    'https://example.test/#fragment',
  ]) {
    expect(() => companionOrigin(value)).toThrow();
  }
});
