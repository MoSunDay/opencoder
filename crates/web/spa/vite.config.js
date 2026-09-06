import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// Build output contract (consumed by the Rust single-binary embed, wired
// elsewhere): dist/index.html + dist/static/app.js + dist/static/app.css —
// NO content hashes, so `cargo build` never needs node.
//
// CSS keeps the fixed name app.css regardless of the entry chunk name
// (Vite names the extracted stylesheet after the chunk, i.e. "index").
function assetName(assetInfo) {
  const names = assetInfo.names || (assetInfo.name ? [assetInfo.name] : []);
  if (names.some((n) => n.endsWith('.css'))) {
    return 'static/app.css';
  }
  return 'static/[name][extname]';
}

export default defineConfig({
  base: './',
  plugins: [react()],
  test: {
    // Cap forks pool: uncapped, vitest spawns ~nproc (16) parallel jsdom
    // environments and slow antd DOM suites cross the 5s testTimeout under
    // load (brainPanel/nav flakes, reproducible only in full-suite runs).
    poolOptions: {
      forks: { minForks: 1, maxForks: 4 },
    },
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    rollupOptions: {
      output: {
        entryFileNames: 'static/app.js',
        chunkFileNames: 'static/[name].js',
        assetFileNames: assetName,
      },
    },
  },
});
