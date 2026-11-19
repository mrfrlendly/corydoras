import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// Tauri expects a fixed port in dev and no screen clearing.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ['**/src-tauri/**', '**/target/**'] },
  },
  build: {
    target: 'safari15',
    outDir: 'dist',
    emptyOutDir: true,
    // The desktop app, and the page that the phone link serves on the Wi-Fi.
    rollupOptions: { input: { main: 'index.html', phone: 'phone.html' } },
  },
});
