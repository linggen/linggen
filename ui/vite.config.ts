import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import fs from 'fs';
import path from 'path';

// Helper to get port from linggen.toml
function getBackendPort() {
  try {
    const tomlPath = path.resolve(__dirname, '../linggen.toml');
    const content = fs.readFileSync(tomlPath, 'utf-8');
    const match = content.match(/\[server\][\s\S]*?port\s*=\s*(\d+)/);
    return match ? match[1] : '9898';
  } catch (_e) {
    return '9898';
  }
}

const backendPort = getBackendPort();

export default defineConfig({
  plugins: [
    react(),
    tailwindcss(),
  ],
  server: {
    host: '0.0.0.0',
    proxy: {
      '/api': `http://localhost:${backendPort}`,
      '/apps': `http://localhost:${backendPort}`,
    },
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
  },
});
