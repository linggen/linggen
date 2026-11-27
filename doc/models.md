---
type: spec
reader: Coding agent and users
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
---

# Models

Hardware abstraction: model providers, default-model routing, credentials, and auto-fallback.

## Related docs

- `agent-spec.md`: per-agent model override.
- `product-spec.md`: multi-model design principle.
- `storage-spec.md`: credentials file location.

## Providers

Linggen supports multiple model providers. Each has a dedicated `provider` value:

| Provider | Type | `provider` value | Auth |
|:---------|:-----|:-----------------|:-----|
| ChatGPT (Subscription) | Cloud | `chatgpt` | OAuth (`ling auth login`) |
| Ollama | Local | `ollama` | None |
| Google Gemini | Cloud | `gemini` | API key |
| OpenAI | Cloud | `openai` | API key |
| Groq | Cloud | `groq` | API key |
| DeepSeek | Cloud | `deepseek` | API key |
| OpenRouter | Cloud | `openrouter` | API key |
| GitHub Models | Cloud | `github` | API key |

All cloud providers (except ChatGPT) use the OpenAI-compatible chat completions API. You can also use `provider = "openai"` with any OpenAI-compatible endpoint (vLLM, LM Studio, etc.).

### Default: Linggen Cloud

New installs default to `qwen3.8-flash` via Linggen Cloud, the list's starter, ticked once when the
config is first read. Linggen Cloud's chat models are the site's: the engine reads them from linggen.dev
`GET /api/cloud/models` (`{"updated","starter","models":[{"id","label"}]}`, built from the site's price table),
keeps the last good answer in `~/.linggen/cache/cloud-models.json` for offline, and before it has ever read one
knows only the first-run starter (`provider/models/cloud_models.rs`). They are ticked in Settings → Models;
only ticked ones reach a session's picker; a tick is an ordinary row (`provider/models/cloud_tick.rs`). They run through the linggen.dev relay on the account, usage shown as a percentage. A saved
`deepseek-flash` or `deepseek-v4-flash` becomes the ticked `deepseek-v4.1-flash`. No API key needed — just sign in:

```bash
ling account login  # Opens browser → sign in to linggen.dev
```

Linggen Cloud means who pays (Linggen's keys, the subscription), not the path the traffic
takes, and offers exactly three uses, each one the engine has a wire for:

1. **Chat** — the models the site lists, through linggen.dev.
2. **Live voice** — Qwen audio realtime, for Lingyu lessons. Linggen Cloud only: a subscriber is given a short-lived key per lesson by linggen.dev and the device connects straight to the provider (`live::find_source`, `account::cloud_voice`). The person's own key never pays for it; no key provider row shows a live line, and each says under its model input that models added there are for chat. Gemini Live stays for development behind `LINGGEN_LIVE_DEV`.
   Metered per lesson, with a length cap the page keeps.
3. **Read-aloud** — Yinyue's voice (Serena) through linggen.dev, the default for subscribers
   (`tts::Source::Cloud`, first in the order); the local voice stays the free and offline choice,
   and takes over when the Cloud voice is refused or unreachable.

Not in Linggen Cloud: pictures, and dictation (Apple's, on the device).

The ChatGPT built-ins — one per family, Sol, Terra and Luna (Luna is the default) — are also present in every install and
need only a ChatGPT sign-in (Settings → Models → Sign in with ChatGPT).
They update by themselves (`provider/models/chatgpt_catalog`): about once a
day and at sign-in the engine reads the latest released Codex client version
from the public listing (never below the compiled one), reads the backend's
model list under it, and moves a family to its newest listed model once one
tiny request on that model has answered. Defaults, the companion's model and
paired phones follow the family; a session or mission pinned to an older id
keeps it while the backend lists it. The Models tab says when a family moved
and can stay on the previous model. Offline and signed out, the compiled
models stand (`gpt-6.2-sol`, `gpt-6.1-terra`, `gpt-6-luna`).
The same sign-in draws pictures: `GenerateImage` asks Luna with the
Responses API's `image_generation` tool (about 30 s a picture, PNG). The
plan's picture allowance is small and separate from chat; a refusal is
reported to the model in one line (tool-spec § GenerateImage).

The model is auto-configured. Tokens are stored in `~/.linggen/codex_auth.json` and auto-refresh. To sign out: `ling auth logout`.

### Configuration

Models are configured in `linggen.runtime.toml`:

```toml
[[models]]
id = "local-qwen"
provider = "ollama"
url = "http://127.0.0.1:11434"
model = "qwen3.5:35b"

[[models]]
id = "gemini-flash"
provider = "gemini"
url = "https://generativelanguage.googleapis.com/v1beta/openai"
model = "gemini-2.5-flash"
```

Optional fields:

| Field | Purpose |
|:------|:--------|
| `context_window` | Override context size when provider API doesn't report it |
| `supports_tools` | Force enable/disable native function calling (`true`/`false`) |
| `reasoning_effort` | Reasoning effort hint (provider-specific) |
| `max_output_tokens` | Output limit (`max_tokens`) sent to an Anthropic model; clamped to the model's cap when the Models API gives it. Other providers ignore it |

### Web UI setup

Nobody at Linggen maintains a model or provider list. Open **Settings → Models**, click **Add provider**, pick a preset — one table in the engine (`provider/models/presets.rs`: Anthropic, OpenAI, Gemini, DeepSeek, Qwen Cloud, OpenRouter, Groq, Ollama) — or **其他…** (a name, an address, the API style: OpenAI-compatible or Claude, and the key). Paste the key once. Nothing is ticked for you: under the provider is one model input; typing suggests ids from the provider's own list (`GET {base}/models`, Anthropic's `/v1/models` with `display_name`, Ollama's tags; asked when the row opens and again when its key or address changes; a provider that answers 401/403 gives `why: key_rejected`; nothing hidden — `provider/models/suggest.rs`). A click adds one; a full id + Enter adds it even when the list cannot be read. Added models show with ✕ to remove. A wrong pick (a tts or embedding id) fails on its first use with the provider's error. A preset's address is not editable (`PATCH` is refused); a typed provider's is, and changing it asks once "地址改了，密钥还用吗？". The key line says only 已设置密钥 · 更换; the row's ⋯ menu has 移除这个服务商, confirmed with the default and fallbacks it holds named (`DELETE /api/providers/{id}` takes its rows and key; the chain goes on without them).

**Kind.** A row is live voice when the engine declares its model at its host (`live::PROVIDERS`) or its id contains `realtime`; every other row is chat (`provider/kinds.rs`).

**Upgrade.** A config from the listed days loads: a ticked family row already holds the exact model it ran, so it is kept as an exact row; `family_binding`, `price`, `price_lookup`, `spend_caps`, `[model_catalog]` are ignored on read and dropped on the next save; `family_starts` naming Linggen Cloud becomes `cloud_started = true`.

The page draws what `GET /api/providers` answers and decides nothing; each button calls its route (`PUT /api/providers/{id}/key`, `GET …/suggestions`, `POST …/models` with just `{model}` — the engine makes the row's id — `PUT /api/providers/cloud` to tick a Linggen Cloud model).

### Defaults by use

Settings → Models owns the chat chain, effort, reading voice, and drawing. Dictation is Apple's on-device speech recognition on a Mac (no choice, nothing to download but a language's model, free); anywhere else, local Whisper. Apps follow the chat default unless an existing override applies, including a model declared in skill frontmatter. Lingyu offers only the Cloud-backed live provider (Qwen); Gemini and own-key lessons are reachable only under `LINGGEN_LIVE_DEV`.

The `/api/uses` snapshot supplies each option's readable `name` and `source`, and the `effective` choice used for Automatic. Repository IDs are tooltip metadata, not option labels.

**Effort (思考强度).** One resolver (`provider/models/effort.rs`) decides a call's effort after the actual model is known: the internal turn's own intent, the session's choice (cleared when its model changes), the agent spec's, the actual model's remembered choice in `[uses.model_effort]`, the row's `reasoning_effort`, then the provider's default (no field). `auto` is the provider's default and stops the walk. API-key models offer what their wire carries — no list is read: OpenAI-compatible sends low, medium, high; Anthropic sends `output_config.effort` (low, medium, high, xhigh, max). When a provider refuses the field (a bad request naming it), the call is asked once more without it and the model offers no effort for the rest of the engine's run. ChatGPT sign-in uses the Codex model list instead: Responses carries low, medium, high, xhigh, max and ultra, and only low, medium and high are offered without listed levels. Claude sign-in, Ollama and room proxies offer none. A chosen value the model lacks sends nothing for that call, stays chosen, and the chat is told once. The chat's model menu shows the chip only where the model offers values.

New preferences live in `[uses]`; the established chat chain and Yinyue’s speaker keep their original storage. Version 2.6.1 ignores `[uses]` on load and drops it on save; those uses then return to their built-in defaults.

## Prompt cache

A provider bills a repeated beginning of a request at a fraction of the price, so the engine keeps each request's beginning the same from turn to turn:

- The system prompt holds nothing that changes per turn. A session's "Right now" (clock, presence) goes beside the turn as a system message, like memory recall.
- Only the system messages a request opens with are its system prompt. A system message inside the conversation — a turn's memory recall, a reminder — stays where it is: a `developer` item on the Responses API, a `<system-reminder>` user block on Anthropic, in place on Chat Completions.
- The ChatGPT backend caches nothing without a key: the engine sends `prompt_cache_key` and the `session_id` header (as Codex CLI does), from the system prompt's SHA-256.
- Anthropic caches only what a request marks: the engine sends the top-level `cache_control` (automatic caching) to api.anthropic.com only; a compatible gateway may not take it.
- A capped session (Yinyue) trims its history in chunks, so the beginning holds between cuts.
- OpenAI API keys, Gemini, DeepSeek and Ollama cache a repeated beginning on their own.

## What a session cost

The engine records what every model call used, per session and per model, in one place: `usage.json` in the session's folder (`storage-spec.md`), written by `AgentEngine::note_call` after each call (and by the next-prompt fork, which is a paid call too). A record is four counts: `input` (the prompt not served from the provider's cache and not written to it), `output`, `cache_read`, `cache_write`; the prompt is the first and the last two together. Anthropic reports all four (`usage.input_tokens`, `output_tokens`, `cache_read_input_tokens`, `cache_creation_input_tokens`); the OpenAI wires, Gemini and Ollama report the prompt, the reply and, where they have it, the cached part (no write); a call whose provider reports nothing counts as a call with `unreported`. A call is recorded however it ends — finished, stopped, failed half-way, dropped (the next-prompt fork), or a summary (`/compact` too): the provider bills what it counted, so what it counted is kept; a call that opened and ended before any usage is one `unreported` call, one refused before it opened is none. Each call is kept with its billing class, fixed from the model's config at the moment it is made; a later change of config reclassifies nothing already made. A ledger written before classes and amounts were stored shows its tokens and claims nothing.

No price is looked up, configured or estimated for a key provider (Hanli, 2026-11-26: 「不设置BYOK的价格上限了」): a session shows tokens, and the person sets budgets in the provider's own console. A model that bills an account (the ChatGPT sign-in, Linggen Cloud, a room's model, a Claude subscription) or runs on this machine shows its allowance meter or nothing, and a session's spend leaves its tokens out. A ledger written while prices were kept still holds its amounts and is summed as stored. Linggen Cloud is billed by the site from each reply's usage × its price table; the page shows the percentage used.

One answer serves both readers (`server/session_cost.rs`): `GET /api/workspace/state` (for the session's top bar — `ui/src/lib/spend.mts` only words it) and `agent_status` (for a caller telling the person what a delegated job cost) both carry `usage`, the engine's ledger. `usage` lists each model's counts and `cost_usd` (only when priced), the tokens of the models that count as spend (each count block also carries its `prompt`), the part of them that was priced (`priced_tokens`) and the part that was not (`unpriced_tokens`), `spent` (any of them used at all), a total `cost_usd` over the priced ones, and `unpriced`, the models whose tokens are not in that total.

The Linggen Cloud meter (a skill's `cloud.meter`) counts a call from the same four counts: `input + cache_write + output` — everything but the cache read. A report without both its prompt and its output meters its `total_tokens`.

## Output limit (Anthropic)

Anthropic requires an output limit on every request, and a coding run is cut when one reply must write more than it. The engine learns each model's cap once, from the Models API (`GET /v1/models/{id}`, `max_tokens`), and sends:

- **A request with tools** (an agent's turn): the cap, or 64,000 when the cap is higher. The limit counts against the context window and toward the output rate limit when a request begins, so the full 128,000 is not asked for by every turn; 64,000 is eight times the old limit.
- **A text-only request** (a summary, a title, the companion's one line): 8,192, the number used before, which it never outgrew.
- **A model the host cannot describe** (another host behind the same wire, an unknown id, no network): 8,192. A lookup that failed is asked again on a later request; a final "not found" is not.
- **`max_output_tokens` in the model's config** replaces both numbers, still clamped to the cap.

The window counts input and limit together, and a context is compacted only at 95% of it, so a request can overflow it. The refusal names the input it counted; the engine sends the request once more with the room left (when at least 1,024 tokens), and any other refusal is the caller's. The limit does not change the cached prefix.

When a reply is cut at the limit the model is told what was cut (`agentic-loop.md`). Other providers set no output limit of their own, except the 65,536 `max_completion_tokens` Chat Completions asks for a reasoning model; the one cut note and the one three-cuts rule serve them all.

## Credentials

API keys are stored in `~/.linggen/credentials.json` — **not** in the TOML config (which may be committed to git).

```json
{
  "gemini-flash": { "api_key": "AIza..." },
  "groq-llama": { "api_key": "gsk_..." }
}
```

**Resolution priority** (per model):
1. `~/.linggen/credentials.json` keyed by model `id` (recommended)
2. TOML `api_key` field (backward compatible, not recommended)
3. Environment variable `LINGGEN_API_KEY_{ID}` (hyphens → underscores, uppercase)

## Free API providers

Some providers offer free tiers:

| Provider | Free tier | Recommended models | Limits |
|:---------|:----------|:-------------------|:-------|
| Google AI Studio | Free | Gemini 2.5 Flash, Gemini 2.5 Pro | 15 RPM, 1M TPD |
| Groq | Free | Llama 4 Scout, Qwen 3 | 30 RPM, 14.4K TPD |
| DeepSeek | Near-free | DeepSeek-V3, DeepSeek-R1 | $0.14/M input tokens |
| OpenRouter | Free tier | Various (free-tagged) | Varies by model |
| GitHub Models | Free | GPT-4.1 mini, Llama, Mistral | 15 RPM, 150K TPD |

All use their respective `provider` value (or `provider = "openai"` with their endpoint URL). Get a free API key from the provider's website.

## Default model

Settings → Models sets the default:

- **The default-model card** — the default for new sessions, then the hand-set fallbacks in order (`routing.default_models`). Changing the default keeps the fallback list, except for the new default itself. An unavailable fallback stays listed until removed. The UI writes these choices through `/api/uses`; `Config::make_default` decides how to replace the first entry.

A Claude model runs natively, with an API key or the Claude subscription
sign-in. The 2.5.1 key for the retired Claude sessions' default,
`routing.claude_default_model`, is read once on load: when the config names
no other default it becomes the default, so a person who ran only Claude keeps
it (`Config::migrate_retired_claude_default`); it is never written again.

**A sign-in makes work possible.** When a person signs in (ChatGPT, or the
Linggen account) and the default cannot run now, the first model that sign-in
makes ready becomes the default — one line says so where they signed in. A
default that can run is never changed. One rule,
`model_choice::after_sign_in`, applied by `AgentManager::adopt_default_after_sign_in`
at each sign-in's end in the engine. A run that cannot start for want of a
sign-in says so and names a ready model (`ModelManager::next_step`, the same
readiness `agent_status op=models` shows), or that none is ready.

## Which model a turn runs on

One rule, `engine/model_choice::resolve(asked, standing, rule)`, for a chat turn, the companion's turn, a mission's run and a delegated run:

- **Asked** — the picker's model, the one a session keeps for a member, the companion's Pet setting (`auto`: Linggen Cloud when signed in, else nothing), or a mission's `model:`. A retired built-in id runs, and is kept, as its successor.
- **Nothing asked** — the place's standing model: the agent's own, from its chain (config override, frontmatter, the star, the first configured model).
- **Asked but cannot run** (not on this machine, or not shared with a room's guest) — the standing model runs instead, the kept pin is cleared, and the same `ModelFallback` notice as a mid-call fallback says which model answers and why.
- **Switching off** (`model_fallback: false`; a delegated `agent_run` session by default) — the asked model is kept and the turn fails on it, named. A guest's shared list is an access rule, not a failure: it holds either way.
- **A room's guest** runs only the models the room shares — before the turn and in the fallback below. A room that shares none refuses the turn.

`AgentEngine::run_on` is the one writer of a turn's model; a lock test fails on any other.

The chat bar's "Default (…)" is that rule with nothing asked, for this session: the app's declared `model:` (`members::app_standing_model`, read by the turn too), else the agent's chain. The page asks `GET /api/sessions/default-model?session_id=` and words the answer; it never reads the routing list itself.

## Auto-fallback

Fallback is on by default. Use the chat row’s switch in Settings → Models or set `model_fallback = false` under `[routing]` to turn it off. The retired `auto_fallback` key is ignored and never saved.

When the active model refuses for a reason about itself — a rate limit or usage window, an empty wallet (402, `insufficient_quota`), a context overflow, a server error or overload, an unreachable backend, an empty answer — the engine tries the next available model. A bad request or a missing key is shown instead: another model would only hide it. On successful fallback, the engine switches to the fallback model for the rest of that run.

Each client reads a refusal once, where it arrives, into a typed `ProviderError` (`provider/error.rs`). A refusing model is benched on one process-wide board (`provider/models/health.rs`) until the time the provider named, else 60 s (30 min for an empty wallet), capped at an hour; a success clears it. The chain steps around benched models and **Settings → Models** (`/api/models/health`) shows the same board.

## Per-agent model

Agents can specify `model` in frontmatter to override the default model:

```yaml
---
name: my-agent
model: gpt-5.4
---
```

## Implementation

- `credentials.rs`: credential store, resolution, API endpoints.
- `config.rs`: `ModelConfig`, `RoutingConfig` (default models + auto-fallback).
- `provider/models.rs`: multi-provider dispatch, streaming, fallback error classification.
- `engine/mod.rs`: `stream_with_fallback()` — auto-retry with model fallback.
- `provider/ollama.rs`: Ollama API client.
- `provider/output_limit.rs`: the output limit an Anthropic request carries.
- `provider/openai.rs`: OpenAI-compatible API client (used by all cloud providers).
- `provider/codex_auth.rs`: ChatGPT OAuth token management.
