import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { fileURLToPath } from 'node:url';

const r = (p: string) => fileURLToPath(new URL(p, import.meta.url));

// Multi-page build: popup.html and settings.html each get their own entry chunk.
// Shared code (Svelte runtime, IPC, tokens) lands in shared chunks.
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  preview: { port: 4173 },
  envPrefix: ['VITE_', 'TAURI_ENV_'],
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    // WebView2 is evergreen Chromium.
    target: 'chrome120',
    cssTarget: 'chrome120',
    modulePreload: { polyfill: false },
    reportCompressedSize: true,
    rollupOptions: {
      input: {
        popup: r('./popup.html'),
        settings: r('./settings.html'),
      },
    },
  },
});
