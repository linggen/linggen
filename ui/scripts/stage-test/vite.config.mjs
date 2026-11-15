// Builds the stage test page next to the app's own sources (never part of `npm run build`).
//   STAGE_OUT=/some/dir npx vite build --config scripts/stage-test/vite.config.mjs
import path from 'node:path';
import { defineConfig } from 'vite';

const here = import.meta.dirname;
export default defineConfig({
  root: here,
  publicDir: path.resolve(here, '../../public'),
  cacheDir: process.env.STAGE_CACHE || path.resolve(process.env.STAGE_OUT || here, '.vite-cache'),
  build: { outDir: process.env.STAGE_OUT, emptyOutDir: true },
});
