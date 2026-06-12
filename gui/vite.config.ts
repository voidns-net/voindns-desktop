import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

export default defineConfig({
  plugins: [sveltekit()],
  // Tauri expects a fixed dev port and clear, unmangled output.
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    host: '0.0.0.0'
  }
});
