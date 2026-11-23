// Usage: node scripts/generate-model-catalog.mjs api.json YYYY-MM-DD output.json
// File-to-file only. The output is both the engine seed and the static site asset.
import { readFileSync, writeFileSync, renameSync, rmSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

const MAX_BYTES = 512 * 1024;
const order = (a, b) => a < b ? -1 : a > b ? 1 : 0;
const need = (ok, message) => { if (!ok) throw new Error(message); };
const record = x => x !== null && typeof x === 'object' && !Array.isArray(x);
const word = x => typeof x === 'string' && x.trim().length > 0;
const number = x => Number.isFinite(x) && x >= 0;
const positive = x => Number.isSafeInteger(x) && x > 0 && x <= 0xffffffff;
const words = x => Array.isArray(x) && x.length > 0 && x.every(word) && new Set(x).size === x.length;
function date(x) {
  return /^\d{4}-\d{2}-\d{2}$/.test(x) && !x.startsWith('0000') &&
    Number.isFinite(Date.parse(x)) && new Date(x).toISOString().slice(0, 10) === x;
}

// Presets declare the upstream ids; no second list of bound providers.
const presets = new URL('../crates/linggen-engine/src/config/providers.rs', import.meta.url);
export const sourceIds = () => [...new Set(Array.from(readFileSync(presets, 'utf8')
  .matchAll(/catalog: Some\(CatalogBinding\s*\{\s*source_id: "([\w-]+)"/g), m => m[1]))].sort(order);

function stability(m) {
  const markers = `${m.id} ${m.status ?? ''}`.toLowerCase().split(/[^a-z0-9]+/);
  if (markers.includes('latest') || m.status === 'alias') return 'alias';
  if (m.experimental === true || markers.some(x => ['exp', 'experimental'].includes(x))) return 'experimental';
  if (markers.some(x => ['preview', 'beta', 'alpha', 'rc'].includes(x))) return 'preview';
  if (m.status === 'deprecated') return 'deprecated';
  return m.status === undefined || m.status === 'stable' ? 'stable' : 'unknown';
}

const rateKeys = ['input', 'output', 'cache_read', 'cache_write'];
function rates(raw) {
  need(record(raw), 'invalid rates');
  return Object.fromEntries(rateKeys.filter(k => raw[k] !== undefined).map(k => {
    need(number(raw[k]), `invalid ${k} rate`);
    return [k, raw[k]];
  }));
}
function cost(raw) {
  const base = rates(raw);
  const tiers = [];
  const extra = Object.keys(raw).filter(k => !rateKeys.includes(k) && k !== 'tiers' && k !== 'context_over_200k');
  for (const k of extra) need(number(raw[k]), `invalid extra rate ${k}`);
  if (raw.tiers !== undefined) {
    need(Array.isArray(raw.tiers), 'invalid tiers');
    for (const item of raw.tiers) {
      const rate = rates(item);
      need(record(item.tier) && word(item.tier.type), 'invalid tier');
      if (item.tier.type !== 'context' || Object.keys(item).some(k => k !== 'tier' && !rateKeys.includes(k))) {
        extra.push('unsupported_tier');
        continue;
      }
      need(positive(item.tier.size) && !tiers.some(t => t.over === item.tier.size), 'invalid or duplicate context threshold');
      tiers.push({ over: item.tier.size, ...rate });
    }
  }
  if (raw.context_over_200k !== undefined) {
    const legacy = rates(raw.context_over_200k);
    if (Object.keys(raw.context_over_200k).some(k => !rateKeys.includes(k))) extra.push('unsupported_tier');
    // The legacy field can duplicate an explicitly sized tier. Its label is
    // less precise (e.g. 272k), so use the explicit threshold in that case.
    if (!tiers.some(({ over, ...rates }) => JSON.stringify(rates) === JSON.stringify(legacy))) {
      if (tiers.length) extra.push('ambiguous_context_tier');
      else tiers.push({ over: 200000, ...legacy });
    }
  }
  tiers.sort((a, b) => a.over - b.over);
  const unestimated = [...new Set(extra)].sort(order);
  return { ...base, ...(tiers.length && { tiers }), ...(unestimated.length && { unestimated }) };
}

function reasoning(options) {
  need(Array.isArray(options), 'invalid reasoning options');
  return options.map(o => {
    need(record(o), 'invalid reasoning option');
    switch (o.type) {
      case 'toggle': return { type: o.type };
      case 'effort': need(words(o.values), 'invalid effort values'); return { type: o.type, values: o.values };
      case 'budget_tokens': {
        for (const k of ['min', 'max']) need(o[k] === undefined || (Number.isInteger(o[k]) && o[k] >= 0 && o[k] <= 0xffffffff), `invalid ${k} budget`);
        need(o.min === undefined || o.max === undefined || o.min <= o.max, 'reversed budget');
        return { type: o.type, min: o.min, max: o.max };
      }
      default: throw new Error('unknown reasoning option');
    }
  });
}

function model(id, m, observed) {
  need(record(m) && word(id) && m.id === id && word(m.name), `invalid model ${id}`);
  for (const k of ['family', 'status', 'task', 'type']) need(m[k] === undefined || word(m[k]), `invalid ${id}.${k}`);
  const day = m.release_date;
  const month = typeof day === 'string' && /^\d{4}-\d{2}$/.test(day) && date(`${day}-01`);
  need(day === undefined || (typeof day === 'string' && (month || date(day))), `invalid release date for ${id}`);
  need(m.experimental === undefined || typeof m.experimental === 'boolean' || record(m.experimental), `invalid experimental flag for ${id}`);
  need(record(m.modalities) && words(m.modalities.input) && words(m.modalities.output), `invalid modalities for ${id}`);
  need(m.limit === undefined || record(m.limit), `invalid limits for ${id}`);
  for (const k of ['context', 'output']) need(m.limit?.[k] === undefined || m.limit[k] === 0 || positive(m.limit[k]), `invalid ${k} limit for ${id}`);
  need(m.tool_call === undefined || typeof m.tool_call === 'boolean', `invalid tool flag for ${id}`);
  if (m.cost?.as_of !== undefined) need(date(m.cost.as_of) && m.cost.as_of <= observed, `invalid price date for ${id}`);
  const rawCost = record(m.cost) ? Object.fromEntries(Object.entries(m.cost).filter(([k]) => k !== 'as_of')) : m.cost;
  return {
    id, family: m.family, name: m.name, release_date: day, status: m.status, stability: stability(m), task: m.task ?? m.type,
    modalities: { input: [...m.modalities.input].sort(order), output: [...m.modalities.output].sort(order) },
    context: m.limit?.context || undefined, output_limit: m.limit?.output || undefined, tool_call: m.tool_call,
    reasoning_options: m.reasoning_options === undefined ? undefined : reasoning(m.reasoning_options),
    cost: m.cost === undefined ? undefined : { ...cost(rawCost), as_of: m.cost.as_of },
  };
}

export function generate(input, observed, ids = sourceIds()) {
  need(record(input) && date(observed) && ids.length > 0 && new Set(ids).size === ids.length, 'invalid source or observation date');
  const providers = [...ids].sort(order).map(source_id => {
    const p = input[source_id];
    need(record(p) && p.id === source_id && word(p.name) && record(p.models), `missing provider ${source_id}`);
    if (p.api != null) {
      need(typeof p.api === 'string', `invalid descriptive API for ${source_id}`);
      const url = new URL(p.api);
      need(url.protocol === 'https:' && !url.username && !url.password && !url.search && !url.hash, `invalid descriptive API for ${source_id}`);
    }
    const models = Object.keys(p.models).sort(order).map(id => model(id, p.models[id], observed));
    need(models.length > 0, `empty provider ${source_id}`);
    return { source_id, name: p.name, api: p.api ?? undefined, models };
  });
  return { schema: 1, source: 'models.dev', observed_at: observed, providers };
}

function main([input, observed, output, ...rest]) {
  need(input && observed && output && !rest.length, 'usage: generate-model-catalog.mjs api.json YYYY-MM-DD output.json');
  const next = generate(JSON.parse(readFileSync(input, 'utf8')), observed);
  let old;
  try { old = JSON.parse(readFileSync(output, 'utf8')); } catch (err) { if (err.code !== 'ENOENT') throw err; }
  if (old) {
    need(old.schema === 1 && old.source === next.source && old.observed_at <= observed, 'invalid or newer baseline');
    for (const p of old.providers) {
      const rows = new Set(next.providers.find(n => n.source_id === p.source_id)?.models.map(m => m.id));
      need(p.models.filter(m => rows.has(m.id)).length >= Math.ceil(p.models.length * 0.8), `abnormal row loss in ${p.source_id}`);
    }
  }
  const bytes = `${JSON.stringify(next)}\n`;
  need(Buffer.byteLength(bytes) <= MAX_BYTES, 'catalog exceeds 512 KiB');
  const tmp = `${output}.${process.pid}.tmp`;
  try {
    writeFileSync(tmp, bytes, { flag: 'wx' });
    renameSync(tmp, output);
  } finally { rmSync(tmp, { force: true }); }
  console.log(`${Buffer.byteLength(bytes)} bytes; ${next.providers.length} providers; ${next.providers.reduce((n, p) => n + p.models.length, 0)} models`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) main(process.argv.slice(2));
