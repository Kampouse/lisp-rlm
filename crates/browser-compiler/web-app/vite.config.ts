import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { execSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const repoRoot = fileURLToPath(new URL('../../../', import.meta.url));

// Build info injected as the __BUILD_INFO__ global — the footer badge shows
// exactly which compiler revision the site is running (we've been bitten by
// silent deploys / stale hashes more than once).
function buildInfo() {
  let rev = 'unknown';
  let dirty = false;
  let time = '';
  try {
    rev = execSync('git rev-parse --short HEAD', { cwd: repoRoot }).toString().trim();
    dirty = execSync('git status --porcelain', { cwd: repoRoot }).toString().trim().length > 0;
    time = new Date().toISOString();
  } catch {
    // not a git checkout / git missing — badge degrades gracefully
  }
  return {
    name: 'build-info-inject',
    config() {
      return { define: { __BUILD_INFO__: JSON.stringify({ rev, dirty, time }) } };
    },
  };
}

export default defineConfig({
  plugins: [svelte(), buildInfo()],
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
