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
  clear: { orb: true, fps: 12 },
  cloudy: { topClouds: 4, fps: 12 },
  fog: { mist: true, fps: 12 },
  rain: { drops: [40, 12], fps: 30 },
  storm: { drops: [62, 16], fps: 30, flash: true },
  snow: { flakes: [48, 12], fps: 30 },
};

/* Overlay fog: ground mist. A sheet rising from the bottom edge to `top`
   (fraction of the viewport from the top), thickest at the bottom and
   feathering out upward, sliding back and forth (`sweep` of the width, a
   sine over `period`, so it eases at each end). Through it, long thin wisps
   (centre and height as fractions, speed in px/s) slide faster and trail
   off; on light pages each wisp carries a thin white lift (kept faint:
   screen lifts the text under it too). */
const FOG = {
  top: 0.4, sweep: 0.22, period: 30000, breath: 7000,
  wisps: [{ y: 0.62, h: 0.2, v: 38 }, { y: 0.76, h: 0.2, v: 26 }, { y: 0.9, h: 0.18, v: -20 }],
};
const TEX_SCALE = 4; // textures paint at 1/4 size; the upscale is the blur

/* Inks by look and page theme. Background: the front reads at a glance in
   the open sky, the back (behind panels) stays faint. Overlay: everything
   low (≤ 0.4) and thin, over the page's own colours. */
const INKS = {
  background: {
    light: { front: 'rgba(40,55,75,0.6)', back: 'rgba(58,76,96,0.16)', flake: 'rgba(255,255,255,0.95)', flakeBack: 'rgba(255,255,255,0.55)', rim: 'rgba(80,100,122,0.55)', width: 1.4, flash: 0.3 },
    dark: { front: 'rgba(200,215,235,0.5)', back: 'rgba(185,200,218,0.13)', flake: 'rgba(225,232,242,0.8)', flakeBack: 'rgba(225,232,242,0.45)', rim: null, width: 1.4, flash: 0.14 },
  },
  overlay: {
    light: { front: 'rgba(70,92,120,0.32)', back: 'rgba(70,92,120,0.14)', flake: 'rgba(130,155,190,0.36)', flakeBack: 'rgba(130,155,190,0.18)', rim: null, width: 1, flash: 0.05, mist: 'rgba(160,172,192,', sheet: 0.9, wisp: 0.5, lift: 'rgba(255,255,255,', liftA: 0.12, cloud: 0.45, blend: 'multiply',
      sun: { glow: 'rgba(255,206,120,', glowA: 0.3, disc: 'rgba(255,220,140,', ray: 0.08 },
      moon: { halo: 'rgba(170,196,236,', haloA: 0.22, fill: '#f4f7fc', rim: 'rgba(140,162,198,0.9)', glow: null, star: 'rgba(130,165,220,', sparkle: true } },
    dark: { front: 'rgba(205,218,235,0.28)', back: 'rgba(205,218,235,0.12)', flake: 'rgba(230,236,245,0.32)', flakeBack: 'rgba(230,236,245,0.16)', rim: null, width: 1, flash: 0.05, mist: 'rgba(200,206,215,', sheet: 0.24, wisp: 0.12, lift: null, liftA: 0, cloud: 0.12, blend: 'screen',
      sun: { glow: 'rgba(255,210,140,', glowA: 0.14, disc: 'rgba(255,232,190,', ray: 0.05 },
      moon: { halo: 'rgba(255,236,196,', haloA: 0.1, fill: '#f6ecd2', rim: null, glow: 'rgba(255,236,196,0.8)', star: 'rgba(240,244,255,', sparkle: false } },
  },
};

const BLUR_FPS = 6;
const SLOW_MS = 7; // a draw slower than this on average → one still frame
const AREA = 1440 * 900;

const rnd = (a, b) => a + Math.random() * (b - a);
const cssRect = (r) => ({ x: r.left ?? r.x, y: r.top ?? r.y, w: r.width, h: r.height });

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
  const liftCtx = lift?.getContext('2d');
  const ctx = canvas.getContext('2d', { alpha: o.overlay });
  const darkMq = matchMedia('(prefers-color-scheme: dark)');
  const calmMq = matchMedia('(prefers-reduced-motion: reduce)');

  const st = { kind: o.kind ?? 'clear', day: o.is_day !== false, dark: o.dark ?? darkMq.matches };
  let W = 0, H = 0, bg = null, front = [], back = [], clouds = [], layers = [], fog = null, orb = null, covers = [], coversAt = 0;
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
    const [top, bottom] = theme()[st.kind];
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
    front = []; back = []; clouds = []; layers = []; fog = null; orb = null;
    if (o.overlay) return seedOverlay(p);
    if (p.drops) { front = make(n(p.drops, 0), drop, 1); back = make(n(p.drops, 1), drop, 0.6); }
    if (p.flakes) { front = make(n(p.flakes, 0), flake, 1); back = make(n(p.flakes, 1), flake, 0.6); }
    if (p.night && !st.day) front = make(n(p.night, 0), star, 1);
    const blobs = p.clouds ?? p.fog ?? 0;
    clouds = Array.from({ length: blobs }, cloud);
  }

  /* Overlay: the falling particles, the ground fog, or the top-edge clouds
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
    if (p.topClouds) layers = Array.from({ length: p.topClouds }, (_, i) => topCloud(i, p.topClouds));
    blendEl.style.mixBlendMode = fog || layers.length ? ink().blend : '';
    lift.style.mixBlendMode = 'screen';
  }

  // A small canvas at 1/TEX_SCALE of the drawn size, filled by `paint`.
  function texture(w, h, paint) {
    const c = document.createElement('canvas');
    c.width = Math.max(1, Math.ceil(w / TEX_SCALE)); c.height = Math.max(1, Math.ceil(h / TEX_SCALE));
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
    g.fillStyle = k.fill; g.fill();
    g.shadowBlur = 0;
    if (k.rim) { g.strokeStyle = k.rim; g.lineWidth = 1.2; g.stroke(); }
    return sprite;
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

  /* A faint cloud hugging the top edge: a row of puffs, a lumpy top. */
  function topCloud(i, count) {
    const w = rnd(280, 460), h = w * rnd(0.32, 0.42), rgb = ink().mist;
    const tex = texture(w, h, (g, tw, th) => {
      for (let n = 0; n < 6; n++) {
        const x = tw * (0.18 + n * 0.13 + rnd(-0.04, 0.04)), y = th * rnd(0.45, 0.62);
        puff(g, x, y, tw * rnd(0.14, 0.22), th * rnd(0.3, 0.45), rgb, rnd(0.55, 0.85));
      }
    });
    const x = ((i + rnd(0, 0.6)) / count) * (W + w) - w;
    return { tex, x, y: -h * rnd(0.2, 0.42), w, h, a: ink().cloud, v: rnd(3, 6), ph: rnd(0, 6.3), period: 7000 + i * 1300 };
  }

  function resize() {
    const w = Math.max(1, Math.round(host.clientWidth || innerWidth));
    const h = Math.max(1, Math.round(host.clientHeight || innerHeight));
    if (w === W && h === H) return;
    W = canvas.width = w; H = canvas.height = h;
    if (lift) { lift.width = w; lift.height = h; }
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
      d.y += d.v * dt; d.x -= d.v * dt * 0.12;
      if (d.y > H) { d.y = rnd(-40, -10); d.x = rnd(0, W * 1.1); }
      ctx.moveTo(d.x, d.y); ctx.lineTo(d.x - d.len * 0.12, d.y + d.len);
    }
    ctx.strokeStyle = color; ctx.lineWidth = width; ctx.stroke();
  }

  function snow(list, dt, t, color, rim) {
    ctx.beginPath();
    for (const f of list) {
      f.y += f.v * dt;
      if (f.y > H + 4) { f.y = -4; f.x = rnd(0, W); }
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

  // A cloud leaves on the right and comes back in on the left.
  const wrapX = (L, x) => (x > W ? -L.w : x);

  /* The overlay's drifting things: the ground fog's sheet sweeping back and
     forth and breathing, its wisps sliding through; clouds along the top. */
  function drift(dt, t) {
    if (fog) drawFog(dt, t);
    if (orb) drawOrb(t);
    for (const L of layers) {
      L.x = wrapX(L, L.x + L.v * dt);
      ctx.globalAlpha = L.a * (0.875 + 0.125 * Math.sin(t / L.period + L.ph));
      ctx.drawImage(L.tex, L.x, L.y, L.w, L.h);
    }
    ctx.globalAlpha = 1;
  }

  function drawFog(dt, t) {
    const k = ink(), breath = 0.92 + 0.08 * Math.sin(t / FOG.breath + fog.ph);
    const sx = wrapW(FOG.sweep * W * Math.sin((t / FOG.period) * 6.2832 + fog.ph));
    ctx.globalAlpha = k.sheet * breath;
    tile(ctx, fog.sheet, sx, H - fog.h, fog.h);
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
    if (o.overlay) { ctx.clearRect(0, 0, W, H); liftCtx.clearRect(0, 0, W, H); }
    else if (bg) ctx.drawImage(bg, 0, 0);
    else return;
    if (o.overlay) drift(dt, t); else blobs(dt);
    if (o.overlay && still()) return; // a still overlay: the mist stays, nothing falls
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
    return alive && !still() && !document.hidden && (front.length || back.length || clouds.length || layers.length || fog || orb || plan().flash);
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
    /** `{kind, is_day}` — a reading from the weather sense, or your own. */
    set(next) {
      const kind = KINDS.includes(next?.kind) ? next.kind : st.kind;
      const day = next?.is_day !== false;
      if (kind === st.kind && day === st.day && W) return;
      st.kind = kind; st.day = day; nextFlash = 0;
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
    },
  };

  resize();
  start();
  o.onMode?.(mode());
  if (o.auto) follow();
  return api;
}
