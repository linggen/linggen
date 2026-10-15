// sky.js — the live weather on a page, one painter for the main UI and any
// skill page that opts in (served at /shared/sky.js; the UI bundles this same
// file). Draws clear · cloudy · fog · rain · snow · storm, day and night, for
// a light or a dark page. Two looks:
//   - background (default): a sky gradient with the weather in it, behind
//     the page's content;
//   - `overlay: true`: only the weather itself over the content, no sky fill:
//     thin streaks, soft flakes, fog as a wide flat sheet of ground mist
//     rising from the bottom edge with thin wisps sliding through it,
//     cloudy as a few faint clouds along the top edge. Fog and clouds blend
//     with the page (multiply on light, screen on dark) so they show on
//     white and keep text dark; on light, a second pass lifts thin white
//     streaks over the grey. Clear shows a sun (or a moon and a few stars)
//     in the top-right corner; no lightning unless
//     `lightning: true` (then a short, faint flash, never a white-out).
//     A still overlay (reduced motion, slow machine) keeps the fog and
//     clouds as one frozen frame and drops what falls.
//     Wind (`wind: {force 0–1, dir}`) leans the rain, carries snow and the
//     season's things across, speeds clouds and mist, and a strong one draws
//     a few gust strokes. `season` adds its things (petals, fireflies at
//     night, leaves, frost and a bare twig) and `festival` its particle kind
//     (sparks, glow, stars, leaves, petals, snow) — few and faint.
//
//   import { mountSky } from '/shared/sky.js';
//   const sky = mountSky(el, { auto: true });   // follows /api/senses/weather
//   sky.set({ kind: 'rain', is_day: true });     // or drive it yourself
//   sky.destroy();
//
// Cheap by design: one canvas at CSS-pixel resolution, the sky's gradient
// painted once per resize, the particles batched into one path per layer,
// 30 fps for falling weather and ~12 for drifting, ~6 when the window isn't
// focused, nothing while the tab is hidden. Reduced motion, `still: true`, or
// a frame budget the machine can't keep turn it into one still frame.
//
// `cover` (a function returning DOMRects) names the page's reading panels:
// behind them only a slower, sparser back layer moves, and lightning never
// lights them — full weather shows only in the uncovered space.

export const KINDS = ['clear', 'cloudy', 'fog', 'rain', 'snow', 'storm'];
/* `none`: no weather read — only the season's or a festival's things draw (overlay). */
const ALL_KINDS = [...KINDS, 'none'];

/* Sky gradients, top → bottom, by page theme, then day / night. The light
   theme never goes darker than a rain sky (#7f8d9b), so frosted light panels
   keep their contrast; the dark theme stays near-black. */
const SKIES = {
  light: {
    day: {
      clear: ['#8fc3e8', '#dcecf7'], cloudy: ['#aab7c3', '#e1e7ec'], fog: ['#c3c9cd', '#e9ebea'],
      rain: ['#8c99a6', '#c2cbd3'], snow: ['#a9b7c4', '#dde3e9'], storm: ['#7f8d9b', '#aeb8c1'],
    },
    night: {
      clear: ['#7d8fae', '#b9c4d6'], cloudy: ['#8c96a3', '#bcc3cc'], fog: ['#9da3a9', '#c8ccce'],
      rain: ['#808c99', '#b1bac3'], snow: ['#97a3b0', '#cdd4db'], storm: ['#7f8b98', '#a3adb7'],
    },
  },
  dark: {
    day: {
      clear: ['#1d3045', '#0f161e'], cloudy: ['#232a31', '#111518'], fog: ['#2a2e31', '#16191b'],
      rain: ['#1b232c', '#0d1115'], snow: ['#2b333c', '#161b21'], storm: ['#141a21', '#08090b'],
    },
    night: {
      clear: ['#0c1424', '#05070b'], cloudy: ['#161a20', '#08090b'], fog: ['#1c1f22', '#0b0c0d'],
      rain: ['#121820', '#07090b'], snow: ['#1d232a', '#0c0f12'], storm: ['#0c1015', '#040506'],
    },
  },
};

/* Per kind: particles per 1440×900 of viewport (front, back), fps. */
const PLAN = {
  clear: { day: [0, 0], night: [40, 0], fps: 12 },
  cloudy: { clouds: 5, fps: 12 },
  fog: { fog: 4, fps: 12 },
  rain: { drops: [70, 16], fps: 30 },
  storm: { drops: [110, 24], clouds: 4, fps: 30, flash: true },
  snow: { flakes: [60, 16], fps: 30 },
};

/* The overlay's lighter plan: fewer particles; clear skies get a sun or moon. */
const OVERLAY_PLAN = {
  none: { fps: 12 },
  clear: { orb: true, fps: 12 },
  cloudy: { cumuli: 4, fps: 12 },
  fog: { mist: true, fps: 12 },
  rain: { drops: [40, 12], fps: 30 },
  storm: { drops: [62, 16], fps: 30, flash: true },
  snow: { flakes: [48, 12], fps: 30 },
};

/* Overlay fog: ground mist. A sheet rising from the bottom edge to `top`
   (fraction of the viewport from the top), thickest at the bottom and
   feathering out upward, sliding back and forth (`sweep` of the width, a
   sine over `period`, so it eases at each end) while its top edge rises and
   falls by `rise` of the viewport. Through it, long soft wisps
   (centre and height as fractions, speed in px/s) slide faster and trail
   off; on light pages each wisp carries a thin white lift (kept faint:
   screen lifts the text under it too). */
const FOG = {
  top: 0.4, sweep: 0.22, period: 19000, breath: 7000, rise: 0.05, risePeriod: 11000,
  wisps: [{ y: 0.62, h: 0.26, v: 24 }, { y: 0.76, h: 0.26, v: 16 }, { y: 0.9, h: 0.22, v: -12 }],
};
const TEX_SCALE = 4; // textures paint at 1/4 size; the upscale is the blur

/* The overlay's season and festival things (2026-10-15), per 1440×900: what
   each season draws (by day, by night) and what a festival's particle kind
   is. Faint, ink-like and few; a still overlay keeps only the frost and the
   twig. Falling weather (rain, snow) takes the season's moving things away. */
const SEASON_THINGS = {
  spring: { day: ['petal'], night: ['petal'] },
  summer: { day: [], night: ['firefly'] },
  autumn: { day: ['leaf'], night: ['leaf'] },
  winter: { day: ['frost', 'twig'], night: ['frost', 'twig'] },
};
const FESTIVAL_THINGS = { sparks: 'spark', glow: 'lantern', stars: 'star', leaves: 'leaf', petals: 'petal', snow: 'flake' };
const THING_COUNT = { petal: 16, leaf: 16, firefly: 14, spark: 22, lantern: 12, star: 14, flake: 30 };
const STILL_THINGS = new Set(['frost', 'twig']);
/* The wind reads in what it carries (Hanli, 2026-10-15: 「落叶的走向可以代表风」):
   leaves, petals, snow and rain drift with its direction, lean and tumble
   with its force, and from BURST_FROM it comes in bursts — every few seconds
   everything sweeps across faster for a moment. Only a gale adds a few faint
   gust strokes. */
const BURST_FROM = 0.5;
const GUSTS_FROM = 0.9;
const BURST_MS = 1800;
/* Inks for the season's and festival's things: muted, ink-like colours drawn
   on their own canvas, multiplied into a light page and screened onto a dark
   one, so they show on white without a light haze over the text. */
const THING_INK = {
  light: { petal: 'rgba(196,112,132,', leaf: 'rgba(160,92,40,', firefly: 'rgba(140,160,40,', spark: 'rgba(200,84,36,', lantern: 'rgba(210,128,52,', star: 'rgba(176,136,60,', flake: 'rgba(100,122,156,', gust: 'rgba(84,98,118,', frost: 'rgba(150,175,200,', twig: 'rgba(40,36,30,', a: 0.62, blend: 'multiply' },
  dark: { petal: 'rgba(240,186,198,', leaf: 'rgba(226,158,92,', firefly: 'rgba(220,240,120,', spark: 'rgba(255,170,90,', lantern: 'rgba(255,190,110,', star: 'rgba(250,236,190,', flake: 'rgba(230,236,245,', gust: 'rgba(196,208,224,', frost: 'rgba(220,232,245,', twig: 'rgba(220,214,200,', a: 0.5, blend: 'screen' },
};

/* Inks by look and page theme. Background: the front reads at a glance in
   the open sky, the back (behind panels) stays faint. Overlay: everything
   low (≤ 0.4) and thin, over the page's own colours. */
const INKS = {
  background: {
    light: { front: 'rgba(40,55,75,0.6)', back: 'rgba(58,76,96,0.16)', flake: 'rgba(255,255,255,0.95)', flakeBack: 'rgba(255,255,255,0.55)', rim: 'rgba(80,100,122,0.55)', width: 1.4, flash: 0.3 },
    dark: { front: 'rgba(200,215,235,0.5)', back: 'rgba(185,200,218,0.13)', flake: 'rgba(225,232,242,0.8)', flakeBack: 'rgba(225,232,242,0.45)', rim: null, width: 1.4, flash: 0.14 },
  },
  overlay: {
    light: { front: 'rgba(70,92,120,0.32)', back: 'rgba(70,92,120,0.14)', flake: 'rgba(130,155,190,0.36)', flakeBack: 'rgba(130,155,190,0.18)', rim: null, width: 1, flash: 0.05, mist: 'rgba(160,172,192,', sheet: 0.9, wisp: 0.5, lift: 'rgba(255,255,255,', liftA: 0.12, blend: 'multiply',
      cumulus: { top: '#e4eaf3', belly: '#b4c0d4', a: 0.9, lift: 0.6 },
      sun: { glow: 'rgba(255,206,120,', glowA: 0.3, disc: 'rgba(255,220,140,', ray: 0.08 },
      moon: { halo: 'rgba(236,206,126,', haloA: 0.2, fill: ['#f1de9c', '#e9cf7a'], glow: 'rgba(233,200,110,0.55)', star: 'rgba(214,172,84,', sparkle: true } },
    dark: { front: 'rgba(205,218,235,0.28)', back: 'rgba(205,218,235,0.12)', flake: 'rgba(230,236,245,0.32)', flakeBack: 'rgba(230,236,245,0.16)', rim: null, width: 1, flash: 0.05, mist: 'rgba(200,206,215,', sheet: 0.24, wisp: 0.12, lift: null, liftA: 0, blend: 'screen',
      cumulus: { top: '#cdd2de', belly: '#9096a6', a: 0.45, lift: 0 },
      sun: { glow: 'rgba(255,210,140,', glowA: 0.14, disc: 'rgba(255,232,190,', ray: 0.05 },
      moon: { halo: 'rgba(255,236,196,', haloA: 0.1, fill: '#f6ecd2', glow: 'rgba(255,236,196,0.8)', star: 'rgba(240,244,255,', sparkle: false } },
  },
};

const BLUR_FPS = 6;
const SLOW_MS = 7; // a draw slower than this on average → one still frame
const AREA = 1440 * 900;

const rnd = (a, b) => a + Math.random() * (b - a);
const cssRect = (r) => ({ x: r.left ?? r.x, y: r.top ?? r.y, w: r.width, h: r.height });

/* The wind as the painter uses it: force 0–1, which way across (+1 = to the
   right; `dir` is where it blows FROM), and the rain's lean (dx per dy).
   None given: a light one leaning left, the old look. */
function windOf(w) {
  const force = Math.min(1, Math.max(0, Number.isFinite(w?.force) ? w.force : 0.15));
  const toward = Number.isFinite(w?.dir) ? -Math.sin((w.dir * Math.PI) / 180) : -1;
  const sign = toward >= 0 ? 1 : -1;
  return { force, sign, lean: sign * Math.tan(((6 + 30 * force) * Math.PI) / 180) };
}

/** Weather view (`/api/senses/weather`) → `{kind, is_day}` or null. */
export function skyOf(view) {
  const r = view?.reading;
  if (!r || !KINDS.includes(r.kind)) return null;
  return { kind: r.kind, is_day: r.is_day !== false };
}

export function mountSky(host, opts = {}) {
  const o = { lightning: !opts.overlay, density: 1, cover: null, still: false, auto: false, onMode: null, overlay: false, ...opts };
  const look = o.overlay ? 'overlay' : 'background';
  const canvas = document.createElement('canvas');
  canvas.setAttribute('aria-hidden', 'true');
  const page = host === document.body || host === document.documentElement;
  Object.assign(canvas.style, {
    position: page ? 'fixed' : 'absolute', inset: '0', width: '100%', height: '100%',
    pointerEvents: 'none', zIndex: o.overlay ? '2147483000' : page ? '-1' : '0', display: 'block',
  });
  if (!page && getComputedStyle(host).position === 'static') host.style.position = 'relative';
  host.prepend(canvas);
  // Mist blends with the page (multiply on light, screen on dark): set on the
  // element whose stacking context sits over the page — the host, unless the
  // canvas is the page's own child.
  const blendEl = page ? canvas : host;
  const lift = o.overlay ? liftCanvas() : null;
  const inkC = o.overlay ? liftCanvas() : null; // the season's and festival's things, blended on their own
  const inkCtx = inkC?.getContext('2d');
  const liftCtx = lift?.getContext('2d');
  const ctx = canvas.getContext('2d', { alpha: o.overlay });
  const darkMq = matchMedia('(prefers-color-scheme: dark)');
  const calmMq = matchMedia('(prefers-reduced-motion: reduce)');

  const st = { kind: o.kind ?? 'clear', day: o.is_day !== false, dark: o.dark ?? darkMq.matches, wind: windOf(o.wind), season: o.season ?? null, festival: o.festival ?? null, blow: 1 };
  let W = 0, H = 0, bg = null, front = [], back = [], clouds = [], cumuli = [], fog = null, orb = null, covers = [], coversAt = 0;
  let things = [], gusts = [], deco = null, burst = { at: -1e9, next: 0 };
  let raf = 0, timer = 0, last = 0, flash = 0, nextFlash = 0, slow = false, alive = true;
  const cost = [];

  const theme = () => SKIES[st.dark ? 'dark' : 'light'][st.day ? 'day' : 'night'];
  const plan = () => (o.overlay ? OVERLAY_PLAN : PLAN)[st.kind] ?? PLAN.clear;
  const still = () => o.still || slow || calmMq.matches;
  const mode = () => (still() ? 'still' : 'motion');

  function paintBackground() {
    if (o.overlay) return;
    bg = document.createElement('canvas');
    bg.width = W; bg.height = H;
    const g = bg.getContext('2d');
    const [top, bottom] = theme()[st.kind] ?? theme().clear;
    const grad = g.createLinearGradient(0, 0, 0, H);
    grad.addColorStop(0, top); grad.addColorStop(1, bottom);
    g.fillStyle = grad; g.fillRect(0, 0, W, H);
    if (st.kind === 'clear') glow(g, st.day ? 'rgba(255,244,214,' : 'rgba(225,232,255,', st.dark ? 0.1 : st.day ? 0.55 : 0.35);
  }

  function glow(g, rgb, a) {
    const x = W * 0.84, y = H * 0.1, r = Math.max(W, H) * 0.35;
    const grad = g.createRadialGradient(x, y, 0, x, y, r);
    grad.addColorStop(0, `${rgb}${a})`); grad.addColorStop(1, `${rgb}0)`);
    g.fillStyle = grad; g.fillRect(0, 0, W, H);
  }

  function seed() {
    const p = plan(), k = Math.min(1.5, Math.max(0.4, (W * H) / AREA)) * o.density;
    const n = (pair, i) => Math.round((pair?.[i] ?? 0) * k);
    const make = (count, maker, depth) => Array.from({ length: count }, () => maker(depth));
    const drop = (d) => ({ x: rnd(0, W), y: rnd(-H, H), v: rnd(520, 780) * d, len: rnd(14, 28) * d });
    const flake = (d) => ({ x: rnd(0, W), y: rnd(-H, H), v: rnd(26, 60) * d, r: rnd(1.6, 3.4) * (0.6 + d * 0.4), ph: rnd(0, 6.3) });
    const star = () => ({ x: rnd(0, W), y: rnd(0, H * 0.6), r: rnd(0.6, 1.4), ph: rnd(0, 6.3) });
    const cloud = () => ({ x: rnd(-0.3, 1) * W, y: rnd(0, 0.45) * H, s: rnd(0.7, 1.4), v: rnd(4, 9) });
    front = []; back = []; clouds = []; cumuli = []; fog = null; orb = null;
    if (o.overlay) return seedOverlay(p);
    if (p.drops) { front = make(n(p.drops, 0), drop, 1); back = make(n(p.drops, 1), drop, 0.6); }
    if (p.flakes) { front = make(n(p.flakes, 0), flake, 1); back = make(n(p.flakes, 1), flake, 0.6); }
    if (p.night && !st.day) front = make(n(p.night, 0), star, 1);
    const blobs = p.clouds ?? p.fog ?? 0;
    clouds = Array.from({ length: blobs }, cloud);
  }

  /* Overlay: the falling particles, the ground fog, the sun or moon, or the
     clouds
     — each painted once here into small textures. */
  function seedOverlay(p) {
    const k = Math.min(1.5, Math.max(0.4, (W * H) / AREA)) * o.density;
    const drop = (d) => ({ x: rnd(0, W), y: rnd(-H, H), v: rnd(520, 780) * d, len: rnd(14, 28) * d });
    const flake = (d) => ({ x: rnd(0, W), y: rnd(-H, H), v: rnd(26, 60) * d, r: rnd(1.6, 3.4) * (0.6 + d * 0.4), ph: rnd(0, 6.3) });
    const make = (pair, maker) => [
      Array.from({ length: Math.round(pair[0] * k) }, () => maker(1)),
      Array.from({ length: Math.round(pair[1] * k) }, () => maker(0.6)),
    ];
    if (p.drops) [front, back] = make(p.drops, drop);
    if (p.flakes) [front, back] = make(p.flakes, flake);
    if (p.mist) fog = groundFog();
    if (p.orb) orb = st.day ? sun() : moon();
    if (p.cumuli) cumuli = Array.from({ length: p.cumuli }, (_, i) => cumulus(i, p.cumuli));
    seedThings(k);
    blendEl.style.mixBlendMode = fog || cumuli.length ? ink().blend : '';
    lift.style.mixBlendMode = 'screen';
  }

  // A small canvas at 1/`scale` of the drawn size, filled by `paint`.
  function texture(w, h, paint, scale = TEX_SCALE) {
    const c = document.createElement('canvas');
    c.width = Math.max(1, Math.ceil(w / scale)); c.height = Math.max(1, Math.ceil(h / scale));
    paint(c.getContext('2d'), c.width, c.height);
    return c;
  }

  // One soft elliptical puff of `rgb` at alpha `a`.
  function puff(g, x, y, rx, ry, rgb, a) {
    const grad = g.createRadialGradient(0, 0, 0, 0, 0, 1);
    grad.addColorStop(0, `${rgb}${a})`); grad.addColorStop(0.55, `${rgb}${a * 0.45})`); grad.addColorStop(1, `${rgb}0)`);
    g.save(); g.translate(x, y); g.scale(rx, ry);
    g.fillStyle = grad; g.fillRect(-1, -1, 2, 2);
    g.restore();
  }

  /* A second canvas over the overlay's host, for the light theme's white
     lift (screen): one element can only blend one way. */
  function liftCanvas() {
    const c = document.createElement('canvas');
    c.setAttribute('aria-hidden', 'true');
    Object.assign(c.style, { position: 'fixed', inset: '0', width: '100%', height: '100%', pointerEvents: 'none', zIndex: '2147483001', display: 'block' });
    if (page) host.append(c); else host.after(c);
    return c;
  }

  /* A texture that tiles across the width: painted with a few pixels of
     the wrap on each side, so the upscale has no seam where tiles meet. */
  function tileTexture(h, paint) {
    const pad = 3, tw = Math.ceil(W / TEX_SCALE);
    const c = texture((tw + 2 * pad) * TEX_SCALE, h, (g, _w, th) => {
      g.translate(pad, 0);
      paint(g, tw, th);
    });
    c.pad = pad; c.tw = tw;
    return c;
  }

  /* Flat, soft ellipses tiled across `w` — streaks, never lumps. `rys` is
     their half-height as a fraction of the tile; with `ys` it keeps every
     smear well inside, so the tile's top and bottom rows stay clear. */
  function smears(g, w, th, n, rgb, ys, a, rys) {
    for (let k = 0; k < n; k++) {
      const x = rnd(0, w), y = th * rnd(ys[0], ys[1]), rx = w * rnd(0.1, 0.24), ry = th * rnd(rys[0], rys[1]);
      for (const dx of [-w, 0, w]) puff(g, x + dx, y, rx, ry, rgb, rnd(a[0], a[1]));
    }
  }

  /* A clear sky: a sun (day) or a moon with a few stars (night) in the
     top-right corner, beside the chat column, never over it. The disc and
     its glow are one sprite; the sun's faint rays another, turned slowly. */
  function orbSprite(k, r, R) {
    const size = Math.ceil(R * 2), c = size / 2, sprite = document.createElement('canvas');
    sprite.width = sprite.height = size;
    const g = sprite.getContext('2d');
    const glow = g.createRadialGradient(c, c, r * 0.8, c, c, R);
    glow.addColorStop(0, `${k.glow}${k.glowA})`); glow.addColorStop(0.35, `${k.glow}${k.glowA * 0.35})`); glow.addColorStop(1, `${k.glow}0)`);
    g.fillStyle = glow; g.fillRect(0, 0, size, size);
    const disc = g.createRadialGradient(c, c, 0, c, c, r);
    disc.addColorStop(0, `${k.disc}0.95)`); disc.addColorStop(0.8, `${k.disc}0.85)`); disc.addColorStop(1, `${k.disc}0)`);
    g.fillStyle = disc; g.beginPath(); g.arc(c, c, r, 0, 6.2832); g.fill();
    return sprite;
  }

  function rays(k, r, R) {
    const size = Math.ceil(R * 2), c = size / 2, sprite = document.createElement('canvas');
    sprite.width = sprite.height = size;
    const g = sprite.getContext('2d');
    const fade = g.createRadialGradient(c, c, r, c, c, R * 0.95);
    fade.addColorStop(0, `${k.disc}${k.ray})`); fade.addColorStop(1, `${k.disc}0)`);
    g.fillStyle = fade;
    for (let i = 0; i < 9; i++) {
      const a = (i / 9) * 6.2832, w = 0.05;
      g.beginPath(); g.moveTo(c, c);
      g.lineTo(c + Math.cos(a - w) * R, c + Math.sin(a - w) * R);
      g.lineTo(c + Math.cos(a + w) * R, c + Math.sin(a + w) * R);
      g.fill();
    }
    return sprite;
  }

  const orbAt = (R) => ({ x: W - Math.min(120, W * 0.1), y: Math.min(64, H * 0.08), R });

  function sun() {
    const k = ink().sun, r = 22, R = Math.min(W, H) * 0.24;
    return { ...orbAt(R), body: orbSprite(k, r, R), rays: rays(k, r, R), stars: [] };
  }

  /* A waxing crescent (~35% lit: the disc minus one offset by 0.55 r),
     tilted, lit on the right; a wide faint halo around it. */
  function crescentSprite(k, r, R) {
    const size = Math.ceil(R * 2), c = size / 2, sprite = document.createElement('canvas');
    sprite.width = sprite.height = size;
    const g = sprite.getContext('2d');
    const halo = g.createRadialGradient(c, c, r * 0.6, c, c, R);
    halo.addColorStop(0, `${k.halo}${k.haloA})`); halo.addColorStop(0.4, `${k.halo}${k.haloA * 0.3})`); halo.addColorStop(1, `${k.halo}0)`);
    g.fillStyle = halo; g.fillRect(0, 0, size, size);
    const d = r * 0.55, h = Math.sqrt(r * r - (d * d) / 4), a = Math.atan2(h, d / 2);
    g.translate(c, c); g.rotate(-0.45); g.scale(-1, 1);
    g.beginPath();
    g.arc(0, 0, r, a, 2 * Math.PI - a);
    g.arc(d, 0, r, Math.atan2(-h, -d / 2), Math.atan2(h, -d / 2), true);
    g.closePath();
    if (k.glow) { g.shadowColor = k.glow; g.shadowBlur = r * 0.8; }
    g.fillStyle = moonFill(g, k.fill, r, d);
    g.fill();
    g.shadowBlur = 0;
    return sprite;
  }

  // A plain colour, or [lit edge, inner] as a gradient across the crescent.
  function moonFill(g, fill, r, d) {
    if (typeof fill === 'string') return fill;
    const grad = g.createLinearGradient(-r, 0, d - r, 0);
    grad.addColorStop(0, fill[0]); grad.addColorStop(1, fill[1]);
    return grad;
  }

  function moon() {
    const k = ink().moon, R = Math.min(W, H) * 0.16, at = orbAt(R);
    const near = () => ({ x: at.x + rnd(-150, 40), y: rnd(10, at.y + 50), r: rnd(1.6, 2.6), ph: rnd(0, 6.3) });
    const edge = () => ({ x: rnd(W * 0.55, W), y: rnd(6, H * 0.09), r: rnd(0.7, 1.3), ph: rnd(0, 6.3) });
    const stars = k.sparkle ? Array.from({ length: 4 }, near) : Array.from({ length: 12 }, edge);
    return { ...at, body: crescentSprite(k, 20, R), rays: null, stars, star: k.star, sparkle: k.sparkle };
  }

  // A tiny four-point sparkle: two thin crossed diamonds.
  function sparkle(x, y, r) {
    ctx.beginPath();
    ctx.moveTo(x, y - r * 2); ctx.lineTo(x + r * 0.35, y); ctx.lineTo(x, y + r * 2); ctx.lineTo(x - r * 0.35, y); ctx.closePath();
    ctx.moveTo(x - r * 2, y); ctx.lineTo(x, y + r * 0.35); ctx.lineTo(x + r * 2, y); ctx.lineTo(x, y - r * 0.35); ctx.closePath();
    ctx.fill();
  }

  function drawOrb(t) {
    if (orb.rays) {
      ctx.save(); ctx.translate(orb.x, orb.y); ctx.rotate((t / 90000) * 6.2832);
      ctx.drawImage(orb.rays, -orb.R, -orb.R, orb.R * 2, orb.R * 2);
      ctx.restore();
    }
    ctx.drawImage(orb.body, orb.x - orb.R, orb.y - orb.R, orb.R * 2, orb.R * 2);
    for (const s of orb.stars) {
      ctx.fillStyle = `${orb.star}${0.2 + 0.55 * Math.sin(t / 1600 + s.ph) ** 2})`;
      if (orb.sparkle) { sparkle(s.x, s.y, s.r); continue; }
      ctx.beginPath(); ctx.arc(s.x, s.y, s.r, 0, 6.2832); ctx.fill();
    }
  }

  /* The ground fog: the sheet (a tall feathered gradient whose top edge is
     smeared, not lumpy) and the wisps, each one tiled texture. */
  function groundFog() {
    const k = ink(), rgb = k.mist, h = (1 - FOG.top) * H;
    const sheet = tileTexture(h, (g, w, th) => {
      const grad = g.createLinearGradient(0, 0, 0, th);
      grad.addColorStop(0, `${rgb}0)`); grad.addColorStop(0.35, `${rgb}0.22)`);
      grad.addColorStop(0.7, `${rgb}0.62)`); grad.addColorStop(1, `${rgb}1)`);
      g.fillStyle = grad; g.fillRect(-4, 0, w + 8, th); // the pads too
      smears(g, w, th, 12, rgb, [0.3, 0.55], [0.15, 0.4], [0.06, 0.14]);
      g.globalCompositeOperation = 'destination-out';
      smears(g, w, th, 5, 'rgba(0,0,0,', [0.25, 0.45], [0.3, 0.6], [0.06, 0.12]);
      g.globalCompositeOperation = 'source-over';
    });
    const wisp = (wd, tint) => tileTexture(wd.h * H, (g, w, th) => smears(g, w, th, 9, tint, [0.42, 0.58], [0.4, 0.9], [0.14, 0.26]));
    const wisps = FOG.wisps.map((wd) => ({ ...wd, x: rnd(0, W), tex: wisp(wd, rgb), lift: k.lift ? wisp(wd, k.lift) : null }));
    return { sheet, h, wisps, ph: rnd(0, 6.3) };
  }

  /* A cumulus in the top ~30%: a flat row of puffs at the base, bigger
     puffs rising above it, 300–600 px wide; the bigger ones are nearer and
     drift faster. Painted once: the puffs' union as one smooth mask, shaded
     from a pale top to a cool-grey belly (multiply on light, screen on
     dark); on light, a white lift on the upper half. It changes shape a
     little as it goes (a slow, uneven stretch). */
  function cumulus(i, count) {
    const k = ink().cumulus, w = rnd(300, 600), h = w * 0.5;
    const puffs = [
      ...[0.22, 0.36, 0.5, 0.64, 0.78].map((x) => [x + rnd(-0.02, 0.02), 0.68, rnd(0.12, 0.15), 0.6]),
      ...[0.36, 0.52, 0.66].map((x) => [x + rnd(-0.03, 0.03), rnd(0.5, 0.56), rnd(0.15, 0.2), 0.95]),
    ];
    // Lobes: solid to 60% of each puff's radius, then a soft fall-off.
    const shape = (g, tw, th) => {
      for (const [x, y, r, sq] of puffs) {
        const grad = g.createRadialGradient(0, 0, 0, 0, 0, 1);
        grad.addColorStop(0, '#000'); grad.addColorStop(0.6, '#000'); grad.addColorStop(1, 'rgba(0,0,0,0)');
        g.save(); g.translate(x * tw, y * th); g.scale(r * tw, r * tw * sq);
        g.fillStyle = grad; g.fillRect(-1, -1, 2, 2);
        g.restore();
      }
      g.globalCompositeOperation = 'source-in';
    };
    const body = texture(w, h, (g, tw, th) => {
      shape(g, tw, th);
      const grad = g.createLinearGradient(0, th * 0.25, 0, th * 0.85);
      grad.addColorStop(0, k.top); grad.addColorStop(1, k.belly);
      g.fillStyle = grad; g.fillRect(0, 0, tw, th);
    }, 2);
    const lift = k.lift ? texture(w, h, (g, tw, th) => {
      shape(g, tw, th);
      const grad = g.createLinearGradient(0, th * 0.2, 0, th * 0.6);
      grad.addColorStop(0, 'rgba(255,255,255,0.45)'); grad.addColorStop(1, 'rgba(255,255,255,0)');
      g.fillStyle = grad; g.fillRect(0, 0, tw, th);
    }, 2) : null;
    const x = ((i + rnd(0, 0.5)) / count) * (W + w) - w;
    return { k, body, lift, w, h, x, y: H * rnd(0.02, 0.16), v: 8 + (w / 600) * 7, ph: rnd(0, 6.3), T: rnd(7000, 11000) };
  }

  function drawCumulus(c, dt, t) {
    c.x += c.v * dt * st.wind.sign * (0.6 + 3 * st.wind.force);
    if (c.x > W) c.x = -c.w;
    else if (c.x < -c.w) c.x = W;
    const sx = 1 + 0.04 * Math.sin(t / c.T + c.ph), sy = 1 + 0.05 * Math.sin(t / (c.T * 1.4) + c.ph + 1);
    const w = c.w * sx, h = c.h * sy, y = c.y + c.h - h;
    ctx.globalAlpha = c.k.a;
    ctx.drawImage(c.body, c.x, y, w, h);
    ctx.globalAlpha = 1;
    if (!c.lift) return;
    liftCtx.globalAlpha = c.k.lift;
    liftCtx.drawImage(c.lift, c.x, y - h * 0.04, w, h);
    liftCtx.globalAlpha = 1;
  }

  /* The season's and the festival's things, and the wind's gusts. */
  function seedThings(k) {
    const falls = st.kind === 'rain' || st.kind === 'storm' || st.kind === 'snow';
    const season = SEASON_THINGS[st.season]?.[st.day ? 'day' : 'night'] ?? [];
    const fest = FESTIVAL_THINGS[st.festival] ?? null;
    const kinds = [...new Set([...season.filter((t) => STILL_THINGS.has(t) || (!falls && !fest)), fest].filter(Boolean))];
    things = kinds.filter((t) => !STILL_THINGS.has(t)).flatMap((t) => Array.from({ length: Math.round((THING_COUNT[t] ?? 10) * k) }, () => thing(t)));
    deco = kinds.some((t) => STILL_THINGS.has(t)) ? decoTexture(kinds) : null;
    gusts = st.wind.force >= GUSTS_FROM ? Array.from({ length: Math.round(3 * Math.min(1.2, k)) }, () => gust(true)) : [];
    inkC.style.mixBlendMode = THING_INK[st.dark ? 'dark' : 'light'].blend;
  }

  /* How hard the wind blows this instant: 1, rising to ~1 + 2·force in a
     burst that swells and fades over BURST_MS, every few seconds. */
  function blowAt(t) {
    if (st.wind.force < BURST_FROM) return 1;
    if (t > burst.next) { burst.at = t; burst.next = t + rnd(3000, 7000) / st.wind.force; }
    const u = (t - burst.at) / BURST_MS;
    return u >= 0 && u < 1 ? 1 + 2 * st.wind.force * Math.sin(Math.PI * u) : 1;
  }

  function thing(t) {
    const base = { t, x: rnd(0, W), y: rnd(-H, H), ph: rnd(0, 6.3), r: rnd(0.7, 1.3) };
    if (t === 'firefly') return { ...base, y: rnd(H * 0.35, H), v: rnd(6, 14) };
    if (t === 'star') return { ...base, y: rnd(0, H * 0.35), v: 0 };
    if (t === 'spark' || t === 'lantern') return { ...base, y: rnd(0, H * 1.2), v: -rnd(t === 'spark' ? 40 : 14, t === 'spark' ? 90 : 30) };
    if (t === 'flake') return { ...base, v: rnd(26, 52) };
    return { ...base, v: rnd(30, 60), spin: rnd(-2, 2) }; // petal, leaf
  }

  function gust(first = false) {
    const len = rnd(0.18, 0.34) * W;
    return { y: rnd(0.08, 0.7) * H, len, x: first ? rnd(-W, W) : -len - rnd(0, W * 0.8), v: rnd(700, 1100) * (0.6 + st.wind.force * 0.6), bow: rnd(-14, 14), w: rnd(1.3, 2.8) };
  }

  /* Frost breathing in from the corners and one bare ink twig at the top-left
     corner (the sun and moon keep the right): painted once. */
  function decoTexture(kinds) {
    const c = document.createElement('canvas');
    c.width = W; c.height = H;
    const g = c.getContext('2d'), k = THING_INK[st.dark ? 'dark' : 'light'];
    if (kinds.includes('frost')) {
      for (const [x, y, r] of [[0, 0, 0.32], [W, 0, 0.26], [0, H, 0.22], [W, H, 0.22]]) {
        const R = Math.max(W, H) * r, grad = g.createRadialGradient(x, y, 0, x, y, R);
        grad.addColorStop(0, `${k.frost}${k.a * 0.55})`); grad.addColorStop(1, `${k.frost}0)`);
        g.fillStyle = grad; g.fillRect(0, 0, W, H);
      }
    }
    if (kinds.includes('twig')) twig(g, k);
    return c;
  }

  function twig(g, k) {
    const s = Math.min(1, W / 1440) * 1.1;
    g.save(); g.scale(s, s);
    g.strokeStyle = `${k.twig}${k.a})`; g.lineCap = 'round';
    const branch = (pts, w) => { g.lineWidth = w; g.beginPath(); g.moveTo(pts[0], pts[1]); g.bezierCurveTo(...pts.slice(2)); g.stroke(); };
    branch([0, 8, 60, 20, 120, 46, 200, 70], 4.5);
    branch([70, 26, 82, 46, 84, 64, 78, 84], 2.2);
    branch([120, 46, 140, 36, 156, 30, 170, 30], 2);
    branch([160, 62, 166, 80, 180, 92, 196, 98], 1.6);
    branch([200, 70, 226, 72, 246, 80, 262, 92], 1.4);
    g.restore();
  }

  /* Moving things: each carried by the wind — its way across, its force,
     its bursts — the leaves and petals tumbling faster as it blows. */
  function drawThings(dt, t) {
    const k = THING_INK[st.dark ? 'dark' : 'light'], blow = blowAt(t);
    const drift = st.wind.sign * (20 + 220 * st.wind.force) * blow;
    for (const p of things) {
      stepThing(p, dt, t, drift, blow);
      drawThing(p, t, k);
    }
  }

  function stepThing(p, dt, t, drift, blow) {
    if (p.t === 'star') return;
    if (p.t === 'firefly') { p.x += (Math.sin(t / 1700 + p.ph) * 10 + drift * 0.1) * dt; p.y += Math.cos(t / 2100 + p.ph) * p.v * dt; return; }
    p.y += p.v * dt;
    p.tumble = (p.tumble ?? 0) + (p.spin ?? 0) * dt * (1 + 2 * st.wind.force) * blow;
    p.x += (drift * (p.t === 'spark' || p.t === 'lantern' ? 0.2 : 1) + Math.sin(t / 1300 + p.ph) * 14) * dt;
    if (p.y > H + 10) { p.y = -10; p.x = rnd(0, W); }
    if (p.y < -12) { p.y = H + 10; p.x = rnd(0, W); }
    if (p.x > W + 20) p.x = -20; else if (p.x < -20) p.x = W + 20;
  }

  function drawThing(p, t, k) {
    const g = inkCtx, tw = 0.5 + 0.5 * Math.sin(t / 900 + p.ph);
    if (p.t === 'firefly' || p.t === 'star' || p.t === 'spark' || p.t === 'lantern') {
      const a = (p.t === 'spark' ? 0.9 : p.t === 'lantern' ? 0.7 : 0.4 + 0.6 * tw) * k.a * 1.4;
      g.fillStyle = `${k[p.t]}${Math.min(0.9, a)})`;
      g.beginPath(); g.arc(p.x, p.y, (p.t === 'lantern' ? 3.2 : 1.6) * p.r, 0, 6.2832); g.fill();
      return;
    }
    // A leaf or petal tumbles: it turns, and its face narrows as it flips.
    const turn = p.ph + (p.tumble ?? 0);
    g.save(); g.translate(p.x, p.y); g.rotate(turn);
    if (p.t !== 'flake') g.scale(1, 0.35 + 0.65 * Math.abs(Math.cos(turn * 1.7)));
    g.fillStyle = `${k[p.t]}${k.a * (p.t === 'flake' ? 0.8 : 1)})`;
    g.beginPath();
    if (p.t === 'flake') g.arc(0, 0, 2.2 * p.r, 0, 6.2832);
    else g.ellipse(0, 0, (p.t === 'leaf' ? 7.5 : 5) * p.r, (p.t === 'leaf' ? 3.2 : 3.4) * p.r, 0, 0, 6.2832);
    g.fill(); g.restore();
  }

  /* Gusts: long dry-brush strokes sweeping across the way the wind blows. */
  function drawGusts(dt) {
    const k = THING_INK[st.dark ? 'dark' : 'light'], ctx = inkCtx;
    for (let i = 0; i < gusts.length; i++) {
      const gs = gusts[i];
      gs.x += gs.v * dt;
      if (gs.x > W + gs.len) { gusts[i] = gust(); continue; }
      const x0 = st.wind.sign > 0 ? gs.x : W - gs.x, x1 = x0 + st.wind.sign * gs.len;
      const grad = ctx.createLinearGradient(x0, 0, x1, 0);
      grad.addColorStop(0, `${k.gust}0)`); grad.addColorStop(0.3, `${k.gust}${k.a * 0.4})`); grad.addColorStop(1, `${k.gust}0)`);
      ctx.strokeStyle = grad; ctx.lineWidth = gs.w;
      ctx.beginPath(); ctx.moveTo(x0, gs.y); ctx.quadraticCurveTo((x0 + x1) / 2, gs.y + gs.bow, x1, gs.y + gs.bow * 0.4); ctx.stroke();
    }
  }

  function resize() {
    const w = Math.max(1, Math.round(host.clientWidth || innerWidth));
    const h = Math.max(1, Math.round(host.clientHeight || innerHeight));
    if (w === W && h === H) return;
    W = canvas.width = w; H = canvas.height = h;
    if (lift) { lift.width = w; lift.height = h; inkC.width = w; inkC.height = h; }
    paintBackground(); seed(); coversAt = 0; draw(0);
  }

  function readCovers(now) {
    if (!o.cover || now - coversAt < 500) return;
    coversAt = now;
    const off = canvas.getBoundingClientRect();
    covers = (o.cover() || []).map(cssRect).filter((r) => r.w > 0 && r.h > 0)
      .map((r) => ({ ...r, x: r.x - off.left, y: r.y - off.top }));
  }

  // Everything but the panels: the full rect plus each cover, even-odd.
  function clipOutCovers() {
    if (!covers.length) return;
    ctx.beginPath();
    ctx.rect(0, 0, W, H);
    for (const r of covers) ctx.rect(r.x, r.y, r.w, r.h);
    ctx.clip('evenodd');
  }

  /* Inks by layer: the front reads at a glance in the open sky (dark streaks
     on a light page, pale on a dark one); the back, behind panels, stays
     faint — the panels' contrast is measured against it. */
  const ink = () => INKS[look][st.dark ? 'dark' : 'light'];

  function rain(list, dt, color, width) {
    ctx.beginPath();
    for (const d of list) {
      d.y += d.v * dt; d.x += d.v * dt * st.wind.lean * (1 + 0.3 * (st.blow - 1));
      if (d.y > H) { d.y = rnd(-40, -10); d.x = rnd(-W * 0.3, W * 1.3); }
      ctx.moveTo(d.x, d.y); ctx.lineTo(d.x + d.len * st.wind.lean * (1 + 0.3 * (st.blow - 1)), d.y + d.len);
    }
    ctx.strokeStyle = color; ctx.lineWidth = width; ctx.stroke();
  }

  function snow(list, dt, t, color, rim) {
    ctx.beginPath();
    for (const f of list) {
      f.y += f.v * dt; f.x += st.wind.sign * 140 * st.wind.force * st.blow * dt;
      if (f.y > H + 4) { f.y = -4; f.x = rnd(0, W); }
      if (f.x > W + 8) f.x = -8; else if (f.x < -8) f.x = W + 8;
      const x = f.x + Math.sin(t / 1400 + f.ph) * 12;
      ctx.moveTo(x + f.r, f.y); ctx.arc(x, f.y, f.r, 0, 6.2832);
    }
    ctx.fillStyle = color; ctx.fill();
    if (rim) { ctx.strokeStyle = rim; ctx.lineWidth = 0.8; ctx.stroke(); }
  }

  function stars(list, t) {
    for (const s of list) {
      const a = 0.35 + 0.45 * Math.sin(t / 900 + s.ph) ** 2;
      ctx.fillStyle = st.dark ? `rgba(230,236,255,${a * 0.7})` : `rgba(255,255,255,${a})`;
      ctx.fillRect(s.x, s.y, s.r * 1.6, s.r * 1.6);
    }
  }

  const wrapW = (x) => ((x % W) + W) % W;
  // One tile across the width, drawn twice so it wraps without a seam.
  function tile(g, tex, x, y, h) {
    g.drawImage(tex, tex.pad, 0, tex.tw, tex.height, x, y, W, h);
    g.drawImage(tex, tex.pad, 0, tex.tw, tex.height, x - W, y, W, h);
  }

  /* The overlay's drifting things: the ground fog's sheet sweeping back and
     forth and breathing, its wisps sliding through; the sun or moon; the
     clouds drifting across the top. */
  function drift(dt, t) {
    if (deco) inkCtx.drawImage(deco, 0, 0);
    if (fog) drawFog(dt * (1 + 2 * st.wind.force), t);
    if (orb) drawOrb(t);
    for (const c of cumuli) drawCumulus(c, dt, t);
  }

  function drawFog(dt, t) {
    const k = ink(), breath = 0.92 + 0.08 * Math.sin(t / FOG.breath + fog.ph);
    const sx = wrapW(FOG.sweep * W * Math.sin((t / FOG.period) * 6.2832 + fog.ph));
    const top = H - fog.h - FOG.rise * H * Math.sin((t / FOG.risePeriod) * 6.2832 + fog.ph);
    ctx.globalAlpha = k.sheet * breath;
    tile(ctx, fog.sheet, sx, top, H - top); // stretched so its bottom stays on the edge
    for (const w of fog.wisps) {
      w.x = wrapW(w.x + w.v * dt);
      const y = w.y * H - (w.h * H) / 2;
      ctx.globalAlpha = k.wisp * breath;
      tile(ctx, w.tex, w.x, y, w.h * H);
      if (!w.lift) continue;
      liftCtx.globalAlpha = k.liftA * breath;
      tile(liftCtx, w.lift, wrapW(w.x + W * 0.07), y - w.h * H * 0.12, w.h * H);
    }
    liftCtx.globalAlpha = 1;
  }

  function blobs(dt) {
    const fog = st.kind === 'fog';
    const shade = st.dark ? (fog ? 'rgba(150,158,165,' : 'rgba(110,120,132,') : (st.kind === 'storm' ? 'rgba(95,105,118,' : 'rgba(255,255,255,');
    const a = st.dark ? 0.1 : fog ? 0.5 : 0.42;
    for (const c of clouds) {
      c.x += c.v * dt * (fog ? 0.6 : 1);
      const rw = (fog ? W * 0.6 : 260) * c.s, rh = (fog ? H * 0.22 : 70) * c.s;
      if (c.x - rw > W) c.x = -rw;
      const y = fog ? H * 0.25 + c.y : c.y;
      const g = ctx.createRadialGradient(c.x, y, 0, c.x, y, rw);
      g.addColorStop(0, `${shade}${a})`); g.addColorStop(1, `${shade}0)`);
      ctx.save(); ctx.translate(c.x, y); ctx.scale(1, rh / rw); ctx.translate(-c.x, -y);
      ctx.fillStyle = g; ctx.fillRect(c.x - rw, y - rw, rw * 2, rw * 2);
      ctx.restore();
    }
  }

  function lightning(t) {
    if (!plan().flash || !o.lightning || still()) return;
    if (!nextFlash) nextFlash = t + rnd(6000, 16000);
    if (t > nextFlash) { flash = 1; nextFlash = t + rnd(7000, 18000); }
    if (flash <= 0.02) return;
    ctx.fillStyle = `rgba(235,240,255,${flash * ink().flash})`;
    ctx.fillRect(0, 0, W, H);
    flash *= o.overlay ? 0.6 : 0.8;
  }

  function draw(dt, t = performance.now()) {
    st.blow = o.overlay ? blowAt(t) : 1;
    if (o.overlay) { ctx.clearRect(0, 0, W, H); liftCtx.clearRect(0, 0, W, H); inkCtx.clearRect(0, 0, W, H); }
    else if (bg) ctx.drawImage(bg, 0, 0);
    else return;
    if (o.overlay) drift(dt, t); else blobs(dt);
    if (o.overlay && still()) return; // a still overlay: the mist, frost and twig stay, nothing falls
    if (o.overlay) { drawThings(dt, t); drawGusts(dt); }
    readCovers(t);
    const kind = st.kind;
    const k = ink();
    if (kind === 'rain' || kind === 'storm') rain(back, dt, k.back, 1);
    if (kind === 'snow') snow(back, dt, t, k.flakeBack, null);
    ctx.save();
    clipOutCovers();
    if (kind === 'rain' || kind === 'storm') rain(front, dt, k.front, k.width);
    if (kind === 'snow') snow(front, dt, t, k.flake, k.rim);
    if (kind === 'clear') stars(front, t);
    lightning(t);
    ctx.restore();
  }

  function moving() {
    return alive && !still() && !document.hidden && (front.length || back.length || clouds.length || cumuli.length || fog || orb || things.length || gusts.length || plan().flash);
  }

  function watch(ms) {
    cost.push(ms);
    if (cost.length < 90) return;
    const avg = cost.reduce((a, b) => a + b, 0) / cost.length;
    cost.length = 0;
    if (avg <= SLOW_MS) return;
    slow = true;
    draw(0); // the still frame: no half-fallen particles left frozen
    o.onMode?.('still');
  }

  function frame(t) {
    raf = 0;
    if (!moving()) return;
    const dt = last ? Math.min(t - last, 120) / 1000 : 0;
    last = t;
    const t0 = performance.now();
    draw(dt, t);
    watch(performance.now() - t0);
    const fps = document.hasFocus() ? plan().fps : BLUR_FPS;
    timer = setTimeout(() => { timer = 0; raf = requestAnimationFrame(frame); }, 1000 / fps);
  }

  function start() {
    stop();
    last = 0;
    draw(0);
    if (moving()) raf = requestAnimationFrame(frame);
  }

  function stop() {
    if (raf) cancelAnimationFrame(raf);
    if (timer) clearTimeout(timer);
    raf = 0; timer = 0;
  }

  const onVisible = () => (document.hidden ? stop() : start());
  const onTheme = () => { if (o.dark === undefined) { st.dark = darkMq.matches; paintBackground(); seed(); start(); } };
  const onCalm = () => { o.onMode?.(mode()); start(); };
  const onFocus = () => { if (moving() && timer) { clearTimeout(timer); timer = 0; raf = requestAnimationFrame(frame); } };
  const ro = typeof ResizeObserver !== 'undefined' ? new ResizeObserver(() => resize()) : null;
  ro?.observe(host);
  addEventListener('resize', resize);
  addEventListener('focus', onFocus);
  document.addEventListener('visibilitychange', onVisible);
  darkMq.addEventListener('change', onTheme);
  calmMq.addEventListener('change', onCalm);

  let poll = 0;
  async function follow() {
    if (!alive) return;
    if (!document.hidden) {
      try {
        const res = await fetch('/api/senses/weather');
        const sky = res.ok ? skyOf(await res.json()) : null;
        if (sky) api.set(sky);
      } catch { /* keep the last sky */ }
    }
    poll = setTimeout(follow, 10 * 60 * 1000);
  }

  const api = {
    canvas,
    get mode() { return mode(); },
    /** `{kind, is_day, wind?, season?, festival?}` — a reading from the weather
        sense, or your own. `wind` {force 0–1, dir: degrees it blows from}. */
    set(next) {
      const kind = ALL_KINDS.includes(next?.kind) ? next.kind : st.kind;
      const day = next?.is_day !== false;
      const wind = windOf(next?.wind), season = next?.season ?? null, festival = next?.festival ?? null;
      const same = kind === st.kind && day === st.day && season === st.season && festival === st.festival && wind.force === st.wind.force && wind.sign === st.wind.sign;
      if (same && W) return;
      Object.assign(st, { kind, day, wind, season, festival }); nextFlash = 0;
      paintBackground(); seed(); start();
    },
    destroy() {
      alive = false; stop(); clearTimeout(poll);
      ro?.disconnect();
      removeEventListener('resize', resize);
      removeEventListener('focus', onFocus);
      document.removeEventListener('visibilitychange', onVisible);
      darkMq.removeEventListener('change', onTheme);
      calmMq.removeEventListener('change', onCalm);
      blendEl.style.mixBlendMode = '';
      canvas.remove();
      lift?.remove();
      inkC?.remove();
    },
  };

  resize();
  start();
  o.onMode?.(mode());
  if (o.auto) follow();
  return api;
}
