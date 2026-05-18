import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

export default defineConfig({
  plugins: [svelte()],
  server: {
    headers: {
      // Required for SharedArrayBuffer (Worker + Atomics.wait)
      'Cross-Origin-Opener-Policy': 'same-origin',
      'Cross-Origin-Embedder-Policy': 'require-corp',
    },
  },
  optimizeDeps: {
    // Don't pre-bundle workers
    exclude: ['**/http-worker.ts'],
  },
  worker: {
    format: 'es',
  },
});