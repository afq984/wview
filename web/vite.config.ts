import { defineConfig } from 'vite';

// Library build with a fixed output name so the Rust embed step has a stable
// path (no content hashes while the PoC embeds a single file).
export default defineConfig({
  build: {
    lib: {
      entry: 'src/comments.ts',
      name: 'wviewComments',
      formats: ['iife'],
      fileName: () => 'wview.js',
    },
    outDir: 'dist',
  },
});
