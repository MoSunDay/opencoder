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
