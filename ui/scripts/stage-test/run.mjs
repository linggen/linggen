// Photograph PetStage in headless Chromium at fixed moments of fixed clips, or compare two shot folders.
//   [COLD=1] node scripts/stage-test/run.mjs shoot <builtDir> <outDir> <label>   (COLD: load the clip with her rig at rest)
//   node scripts/stage-test/run.mjs compare <dirA> <labelA> <dirB> <labelB>
// Playwright and pixel tools come from the lingjing skill's tool folder (absolute path, not a ui dependency).
import fs from 'node:fs';
import http from 'node:http';
import path from 'node:path';
import { createRequire } from 'node:module';

const tools = createRequire('/Users/lianghuang/workspace/linggen/skills/lingjing/tools/package.json');
const CASES = [
  ['idle', '/anim/idle2.fbx'],
  ['greeting', '/anim/greeting.fbx'],
  ['happy', '/anim/happy.fbx'],
  ['clap', '/anim/clap.fbx#1,3.2'],
  ['vrma01', '/anim/VRMA_01.vrma'],
];
const MOMENTS = [['t0.8', 24], ['t1.9', 33]]; // seconds into the clip, as 30 fps steps (24 then +33 more)
const TYPES = { '.html': 'text/html', '.js': 'text/javascript', '.json': 'application/json', '.css': 'text/css', '.vrm': 'model/gltf-binary', '.bin': 'application/octet-stream', '.fbx': 'application/octet-stream', '.vrma': 'application/octet-stream' };

function serve(root) {
  const server = http.createServer((req, res) => {
    const file = path.join(root, decodeURIComponent(new URL(req.url, 'http://x').pathname.replace(/\/$/, '/index.html')));
    if (!file.startsWith(root) || !fs.existsSync(file) || fs.statSync(file).isDirectory()) { res.writeHead(404); return res.end(); }
    res.writeHead(200, { 'content-type': TYPES[path.extname(file)] || 'application/octet-stream' });
    fs.createReadStream(file).pipe(res);
  });
  return new Promise((ok) => server.listen(0, '127.0.0.1', () => ok(server)));
}

async function shoot(dist, out, label) {
  const { chromium } = tools('playwright');
  fs.mkdirSync(out, { recursive: true });
  const server = await serve(path.resolve(dist));
  const url = `http://127.0.0.1:${server.address().port}/`;
  const browser = await chromium.launch({ args: ['--use-angle=swiftshader', '--enable-unsafe-swiftshader', '--ignore-gpu-blocklist', '--enable-webgl'] });
  for (const [name, clip] of CASES) {
    const page = await browser.newPage({ viewport: { width: 480, height: 640 }, deviceScaleFactor: 1 });
    const logs = [];
    page.on('console', (m) => logs.push(m.text()));
    page.on('pageerror', (e) => logs.push('PAGEERROR ' + e.message));
    await page.goto(url);
    await page.waitForFunction(() => window.stageTest);
    await page.evaluate(() => window.stageTest.setup());
    if (!process.env.COLD) {
      // warm: let her idle clip run first, so the clip under test loads while her hips are mid-animation (as in the app)
      await page.evaluate(() => window.stageTest.step(3));
      await page.waitForTimeout(1500); // the default idle clip loads in real time
      await page.evaluate(() => window.stageTest.step(3));
    }
    const dur = await page.evaluate((c) => window.stageTest.play(c, true), clip);
    for (const [m, n] of MOMENTS) {
      await page.evaluate((k) => window.stageTest.step(k), n);
      await page.waitForTimeout(150);
      // a clip, not the element: Playwright's stability wait needs the page's requestAnimationFrame, which the page replaces
      await page.screenshot({ path: `${out}/${label}-${name}-${m}.png`, clip: { x: 0, y: 0, width: 480, height: 640 } });
    }
    console.log(`${label} ${name}: clip ${clip} dur ${dur.toFixed(2)}s`, logs.filter((l) => /error|fail|warn/i.test(l)).slice(0, 3).join(' | '));
    await page.close();
  }
  await browser.close();
  server.close();
}

function compare(dirA, la, dirB, lb) {
  const { PNG } = tools('pngjs');
  const pm = tools('pixelmatch');
  const pixelmatch = pm.default ?? pm;
  const load = (f) => PNG.sync.read(fs.readFileSync(f));
  for (const [name] of CASES) for (const [m] of MOMENTS) {
    const a = load(`${dirA}/${la}-${name}-${m}.png`), b = load(`${dirB}/${lb}-${name}-${m}.png`);
    const diff = new PNG({ width: a.width, height: a.height });
    const n = pixelmatch(a.data, b.data, diff.data, a.width, a.height, { threshold: 0.1 });
    let maxCh = 0;
    for (let i = 0; i < a.data.length; i++) maxCh = Math.max(maxCh, Math.abs(a.data[i] - b.data[i]));
    fs.writeFileSync(`${dirB}/diff-${name}-${m}.png`, PNG.sync.write(diff));
    console.log(`${name.padEnd(9)} ${m.padEnd(5)} differing px ${String(n).padStart(6)} of ${a.width * a.height} (${(100 * n / (a.width * a.height)).toFixed(3)}%), max channel delta ${maxCh}`);
  }
}

const [cmd, ...args] = process.argv.slice(2);
if (cmd === 'shoot') await shoot(...args);
else if (cmd === 'compare') compare(...args);
else console.error('usage: shoot <builtDir> <outDir> <label> | compare <dirA> <labelA> <dirB> <labelB>');
