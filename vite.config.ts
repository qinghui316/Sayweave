import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 5173, strictPort: true, host: '127.0.0.1', watch: { ignored: ['**/src-tauri/**', '**/.local/**', '**/test-results/**', '**/playwright-report/**', '**/tests/visual/**', '**/design/**', '**/.agents/**'] } },
  envPrefix: ['VITE_', 'TAURI_ENV_'],
  build: { target: 'chrome110' },
  test: { environment: 'jsdom', setupFiles: ['./tests/setup.ts'], include: ['src/**/*.test.{ts,tsx}', 'tests/contracts/**/*.test.ts', 'tests/*.test.{ts,tsx}'], exclude: ['**/node_modules/**', '**/src-tauri/**', '**/.local/**', '**/design/**', '**/.agents/**'] },
});
