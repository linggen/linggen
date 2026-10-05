// Release gate: the VM's web UI in a real Chromium, over the SSH tunnel.
//   E2E_DIR=tests/e2e node ui-check.mjs <url> <screenshot.png>
// Uses tests/e2e's Playwright install. Pass = the React root renders and no
// uncaught page error; prints one JSON line with what the page shows.
import { createRequire } from 'node:module';

const require = createRequire(`${process.env.E2E_DIR}/package.json`);
const { chromium } = require('@playwright/test');
const [url, shot] = process.argv.slice(2);

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1400, height: 900 } });
const errors = [];
page.on('pageerror', (e) => errors.push(String(e).slice(0, 200)));
let rendered = false;
try {
  await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 30_000 });
  await page.waitForFunction(() => (document.querySelector('#root')?.children.length ?? 0) > 0, null, { timeout: 30_000 });
  rendered = true;
  await page.waitForTimeout(4_000);
} catch (e) {
  errors.push(String(e).slice(0, 200));
}
const text = (await page.innerText('body').catch(() => '')).replace(/\s+/g, ' ').slice(0, 120);
await page.screenshot({ path: shot }).catch(() => {});
await browser.close();
console.log(JSON.stringify({ rendered, errors, text }));
process.exit(rendered && errors.length === 0 ? 0 : 1);
