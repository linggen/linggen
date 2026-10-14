// sky.js — the live weather on a page, one painter for the main UI and any
// skill page that opts in (served at /shared/sky.js; the UI bundles this same
// file). Draws clear · cloudy · fog · rain · snow · storm, day and night, for
// a light or a dark page. Two looks:
//   - background (default): a sky gradient with the weather in it, behind
//     the page's content;
//   - `overlay: true`: only the weather itself over the content, no sky fill:
//     thin streaks, soft flakes, fog as slow mist bands lying low (dense at
//     the bottom, a light veil where text sits), cloudy as a few faint clouds
//     along the top edge. Clear draws nothing; no lightning unless
//     `lightning: true` (then a short, faint flash, never a white-out).
//     A still overlay (reduced motion, slow machine) keeps the mist and
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

/* The overlay's lighter plan: fewer particles, nothing for clear skies. */
const OVERLAY_PLAN = {
  clear: { fps: 12 },
  cloudy: { topClouds: 4, fps: 12 },
  fog: { mist: true, fps: 12 },
  rain: { drops: [40, 12], fps: 30 },
  storm: { drops: [62, 16], fps: 30, flash: true },
  snow: { flakes: [48, 12], fps: 30 },
};

/* Overlay mist, bottom up: band centre and height (fractions of the
   viewport), peak alpha, drift in px/s (alternating, for parallax). With
   MIST_FLOOR under them the veil peaks ~0.45 at the bottom edge, ~0.38 three
   quarters down, ~0.2 mid screen, ~0.04-0.08 in the top third. Chat text
   (above the composer) keeps >= 4.5:1 everywhere and >= 7:1 in the top half. */
const MIST_BANDS = [
  { y: 1.02, h: 0.16, a: 0.3, v: 12 },
  { y: 0.76, h: 0.28, a: 0.27, v: -18 },
  { y: 0.52, h: 0.3, a: 0.15, v: 24 },
  { y: 0.2, h: 0.26, a: 0.03, v: -14 },
];
const MIST_FLOOR = [[0, 0.03], [0.5, 0.09], [0.62, 0.12], [0.75, 0.17], [0.9, 0.2], [0.96, 0.24], [1, 0.3]];
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
    light: { front: 'rgba(70,92,120,0.32)', back: 'rgba(70,92,120,0.14)', flake: 'rgba(130,155,190,0.36)', flakeBack: 'rgba(130,155,190,0.18)', rim: null, width: 1, flash: 0.05, mist: 'rgba(172,184,200,', cloud: 0.24 },
    dark: { front: 'rgba(205,218,235,0.28)', back: 'rgba(205,218,235,0.12)', flake: 'rgba(230,236,245,0.32)', flakeBack: 'rgba(230,236,245,0.16)', rim: null, width: 1, flash: 0.05, mist: 'rgba(178,186,197,', cloud: 0.14 },
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
  const ctx = canvas.getContext('2d', { alpha: o.overlay });
  const darkMq = matchMedia('(prefers-color-scheme: dark)');
  const calmMq = matchMedia('(prefers-reduced-motion: reduce)');

  const st = { kind: o.kind ?? 'clear', day: o.is_day !== false, dark: o.dark ?? darkMq.matches };
  let W = 0, H = 0, bg = null, front = [], back = [], clouds = [], layers = [], floor = null, covers = [], coversAt = 0;
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
    front = []; back = []; clouds = []; layers = []; floor = null;
    if (o.overlay) return seedOverlay(p);
    if (p.drops) { front = make(n(p.drops, 0), drop, 1); back = make(n(p.drops, 1), drop, 0.6); }
    if (p.flakes) { front = make(n(p.flakes, 0), flake, 1); back = make(n(p.flakes, 1), flake, 0.6); }
    if (p.night && !st.day) front = make(n(p.night, 0), star, 1);
    const blobs = p.clouds ?? p.fog ?? 0;
    clouds = Array.from({ length: blobs }, cloud);
  }

  /* Overlay: the falling particles, plus the drifting layers (mist bands,
     top-edge clouds), each painted once here into a small texture. */
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
    if (p.mist) { layers = MIST_BANDS.map(mistBand); floor = mistFloor(); }
    if (p.topClouds) layers = Array.from({ length: p.topClouds }, (_, i) => topCloud(i, p.topClouds));
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

  /* A mist band: a soft stripe plus uneven puffs, tiled across the width
     (each puff also painted one tile left and right, so the seam is gone). */
  function mistBand(b, i) {
    const h = b.h * H, rgb = ink().mist;
    const tex = texture(W, h, (g, w, th) => {
      const stripe = g.createLinearGradient(0, 0, 0, th);
      stripe.addColorStop(0, `${rgb}0)`); stripe.addColorStop(0.3, `${rgb}0.35)`);
      stripe.addColorStop(0.7, `${rgb}0.35)`); stripe.addColorStop(1, `${rgb}0)`);
      g.fillStyle = stripe; g.fillRect(0, 0, w, th);
      for (let n = 0; n < 6; n++) {
        const x = rnd(0, w), y = th * rnd(0.4, 0.6), rx = w * rnd(0.07, 0.16), ry = th * rnd(0.3, 0.45), a = rnd(0.5, 1);
        for (const dx of [-w, 0, w]) puff(g, x + dx, y, rx, ry, rgb, a);
      }
    });
    return { tex, tile: true, x: rnd(0, W), y: b.y * H - h / 2, w: W, h, a: b.a, v: b.v, ph: rnd(0, 6.3), period: 6000 + i * 1900 };
  }

  // The ground haze under the bands: one cached vertical gradient.
  function mistFloor() {
    const rgb = ink().mist, grad = ctx.createLinearGradient(0, 0, 0, H);
    for (const [at, a] of MIST_FLOOR) grad.addColorStop(at, `${rgb}${a})`);
    return grad;
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
    return { tex, tile: false, x, y: -h * rnd(0.2, 0.42), w, h, a: ink().cloud, v: rnd(3, 6), ph: rnd(0, 6.3), period: 7000 + i * 1300 };
  }

  function resize() {
    const w = Math.max(1, Math.round(host.clientWidth || innerWidth));
    const h = Math.max(1, Math.round(host.clientHeight || innerHeight));
    if (w === W && h === H) return;
    W = canvas.width = w; H = canvas.height = h;
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

  // A tile wraps within the width; a cloud leaves on the right, enters left.
  function wrapX(L, x) {
    if (L.tile) return ((x % W) + W) % W;
    return x > W ? -L.w : x;
  }

  /* The overlay's drifting layers: each slides sideways at its own speed and
     breathes slowly (alpha 0.75–1 of its peak). */
  function drift(dt, t) {
    if (floor) { ctx.fillStyle = floor; ctx.fillRect(0, 0, W, H); }
    for (const L of layers) {
      L.x = wrapX(L, L.x + L.v * dt);
      ctx.globalAlpha = L.a * (0.875 + 0.125 * Math.sin(t / L.period + L.ph));
      ctx.drawImage(L.tex, L.x, L.y, L.w, L.h);
      if (L.tile) ctx.drawImage(L.tex, L.x - W, L.y, L.w, L.h);
    }
    ctx.globalAlpha = 1;
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
    if (o.overlay) ctx.clearRect(0, 0, W, H);
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
    return alive && !still() && !document.hidden && (front.length || back.length || clouds.length || layers.length || plan().flash);
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
      canvas.remove();
    },
  };

  resize();
  start();
  o.onMode?.(mode());
  if (o.auto) follow();
  return api;
}
