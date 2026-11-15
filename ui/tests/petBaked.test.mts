import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { importTs } from '../scripts/_nodeEnv.mjs';

// package.json is "type":"commonjs", so a .ts source is loaded through the same type-stripping the bake uses
const { bakedUrl } = await importTs('src/components/yinyue/petBaked.ts');

test('a clip URL in the manifest reads its baked twin, never the raw file', () => {
  assert.equal(bakedUrl('/anim/idle2.fbx'), '/anim-baked/idle2.bin');
  assert.equal(bakedUrl('/anim/VRMA_03.vrma'), '/anim-baked/VRMA_03.bin');
});

test('every clip the action manifest names maps into the baked folder', () => {
  const manifest = JSON.parse(fs.readFileSync(new URL('../public/anim/actions.json', import.meta.url), 'utf8'));
  const urls: string[] = manifest.intents.flatMap((i: { clips?: string[] }) => i.clips ?? []);
  assert.ok(urls.length > 0);
  for (const u of urls) assert.match(bakedUrl(u.split('#')[0]), /^\/anim-baked\/[\w-]+\.bin$/, u);
});
