import { defineConfig } from 'vite';

export default defineConfig({
  build: {
    target: 'es2022',
    // Two windows, two HTML entry points (`src-tauri`'s "search" window loads
    // `index.html`, its on-demand "settings" window loads `settings.html`,
    // TASK-704) - without this, Vite only builds `index.html` by default.
    rollupOptions: {
      input: {
        main: 'index.html',
        settings: 'settings.html',
      },
    },
  },
});
