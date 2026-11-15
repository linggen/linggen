// Shared by bake-anim.mjs / check-baked.mjs: load yinyue.vrm in plain Node
// (no GPU, no DOM) by dropping the texture references from the glTF JSON —
// the retarget only reads the humanoid skeleton, never a pixel.
import fs from 'node:fs';
import { GLTFLoader } from 'three/addons/loaders/GLTFLoader.js';
import { VRMLoaderPlugin } from '@pixiv/three-vrm';

function dropTextures(node) {
  if (Array.isArray(node)) return node.forEach(dropTextures);
  if (!node || typeof node !== 'object') return;
  for (const k of Object.keys(node)) {
    if (/texture$/i.test(k) && node[k] && typeof node[k] === 'object' && 'index' in node[k]) delete node[k];
    else dropTextures(node[k]);
  }
}

export function stripGlbTextures(buf) {
  const jsonLen = buf.readUInt32LE(12);
  const json = JSON.parse(buf.subarray(20, 20 + jsonLen).toString('utf8'));
  delete json.textures; delete json.images; delete json.samplers;
  dropTextures(json.materials);
  let s = JSON.stringify(json);
  while (Buffer.byteLength(s) % 4) s += ' ';
  const jb = Buffer.from(s);
  const rest = buf.subarray(20 + jsonLen);
  const head = Buffer.alloc(20);
  buf.copy(head, 0, 0, 12);
  head.writeUInt32LE(jb.length, 12); head.writeUInt32LE(0x4e4f534a, 16);
  const out = Buffer.concat([head, jb, rest]);
  out.writeUInt32LE(out.length, 8);
  return out;
}

export async function loadVrmNode(path) {
  const buf = stripGlbTextures(fs.readFileSync(path));
  const ab = buf.buffer.slice(buf.byteOffset, buf.byteOffset + buf.byteLength);
  const loader = new GLTFLoader();
  loader.register((p) => new VRMLoaderPlugin(p));
  const gltf = await new Promise((res, rej) => loader.parse(ab, '', res, rej));
  return gltf.userData.vrm;
}

// ---- run the app's real .ts modules and FBXLoader under plain Node ----
import { stripTypeScriptTypes } from 'node:module';
import { pathToFileURL } from 'node:url';
import { FBXLoader } from 'three/addons/loaders/FBXLoader.js';

const UI = new URL('..', import.meta.url).pathname;
export const uiPath = (...p) => UI + p.join('/');

/** Import an erasable-TS module from src/ as-is (package.json is "type":"commonjs",
 *  so Node will not load a .ts directly): strip the types to a temp .mjs beside
 *  this file — so bare imports like 'three' still resolve — import it, delete it. */
export async function importTs(relFromUi) {
  const src = fs.readFileSync(uiPath(relFromUi), 'utf8');
  const tmp = uiPath('scripts', `.tmp-${relFromUi.split('/').pop()}-${process.pid}.mjs`);
  fs.writeFileSync(tmp, stripTypeScriptTypes(src, { mode: 'strip' }));
  try { return await import(pathToFileURL(tmp).href); } finally { fs.rmSync(tmp, { force: true }); }
}

/** FBXLoader.loadAsync would fetch(); serve `/anim/x.fbx` from public/ instead. */
export function patchFbxLoaderToDisk() {
  FBXLoader.prototype.loadAsync = async function (url) {
    const b = fs.readFileSync(uiPath('public', url));
    return this.parse(b.buffer.slice(b.byteOffset, b.byteOffset + b.byteLength), '');
  };
}
