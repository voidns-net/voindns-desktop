import adapter from '@sveltejs/adapter-static';
import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';

/** @type {import('@sveltejs/kit').Config} */
export default {
  preprocess: vitePreprocess(),
  kit: {
    // Tauri serves static files; SPA fallback for client-side routing.
    adapter: adapter({ fallback: 'index.html' })
  }
};
