import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import tailwindcss from '@tailwindcss/vite';
import { viteSingleFile } from 'vite-plugin-singlefile';
import { mockApi } from './preview/mock-api.ts';
export default defineConfig({
  plugins: [svelte(), tailwindcss(), viteSingleFile(), mockApi()],
  resolve: { alias: { $lib: new URL('./src/lib', import.meta.url).pathname } },
  build: { outDir: 'dist', emptyOutDir: true, target: 'es2022' },
  server: { host: '127.0.0.1', port: 18089, strictPort: true },
});
