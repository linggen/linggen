import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react-swc'

// https://vite.dev/config/
export default defineConfig({
  plugins: [react()],
  server: {
    // Tauri devUrl is fixed to http://localhost:5173 in src-tauri/tauri.conf.json.
    // If 5173 is already taken, Vite will otherwise auto-pick 5174, and Tauri would
    // still try to load 5173 (possibly some other app). Strict port avoids that.
    port: 5173,
    strictPort: true,
  },
})
