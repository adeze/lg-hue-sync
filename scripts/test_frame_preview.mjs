// Compatibility entry point; behavioural coverage now exercises typed Svelte helpers.
import { spawnSync } from 'node:child_process';
const tests = ['api.test.ts', 'preview.test.ts'];
const result = spawnSync('bun', ['test', ...tests.map(name => new URL('../frontend/src/lib/' + name, import.meta.url).pathname)], { stdio: 'inherit' });
if (result.error) throw result.error;
process.exit(result.status ?? 1);
