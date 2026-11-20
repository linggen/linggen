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

New installs default to `deepseek-flash` (DeepSeek V4.1 Flash) via Linggen Cloud (the built-in
proxy model, metered against the linggen.dev account). No API key needed —
just sign in:

```bash
ling account login  # Opens browser → sign in to linggen.dev
```

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
| `price` | `{ input, output, cache_read, cache_write }` in USD per million tokens. Wins over the built-in table (§ What a session cost). `cache_read` and `cache_write` default to `input`. Flat: it holds for every prompt size |
| `max_output_tokens` | Output limit (`max_tokens`) sent to an Anthropic model; clamped to the model's cap when the Models API gives it. Other providers ignore it |

### Web UI setup

The easiest way to add models: open **Settings → Models** in the browser, click **Add Model**, pick a provider, paste your API key. The health indicator turns green when connected.

## Prompt cache

A provider bills a repeated beginning of a request at a fraction of the price, so the engine keeps each request's beginning the same from turn to turn:

- The system prompt holds nothing that changes per turn. A session's "Right now" (clock, presence) goes beside the turn as a system message, like memory recall.
- Only the system messages a request opens with are its system prompt. A system message inside the conversation — a turn's memory recall, a reminder — stays where it is: a `developer` item on the Responses API, a `<system-reminder>` user block on Anthropic, in place on Chat Completions.
- The ChatGPT backend caches nothing without a key: the engine sends `prompt_cache_key` and the `session_id` header (as Codex CLI does), from the system prompt's SHA-256.
- Anthropic caches only what a request marks: the engine sends the top-level `cache_control` (automatic caching) to api.anthropic.com only; a compatible gateway may not take it.
- A capped session (Yinyue) trims its history in chunks, so the beginning holds between cuts.
- OpenAI API keys, Gemini, DeepSeek and Ollama cache a repeated beginning on their own.

## What a session cost

The engine records what every model call used, per session and per model, in one place: `usage.json` in the session's folder (`storage-spec.md`), written by `AgentEngine::note_call` after each call (and by the next-prompt fork, which is a paid call too). A record is four counts: `input` (the prompt not served from the provider's cache and not written to it), `output`, `cache_read`, `cache_write`; the prompt is the first and the last two together. Anthropic reports all four (`usage.input_tokens`, `output_tokens`, `cache_read_input_tokens`, `cache_creation_input_tokens`); the OpenAI wires, Gemini and Ollama report the prompt, the reply and, where they have it, the cached part (no write); a call whose provider reports nothing counts as a call with `unreported`. A call is recorded however it ends — finished, stopped, failed half-way, dropped (the next-prompt fork), or a summary (`/compact` too): the provider bills what it counted, so what it counted is kept; a call that opened and ended before any usage is one `unreported` call, one refused before it opened is none. What a call cost is fixed when it is made: the ledger keeps one entry per model, billing class and whether a price was known, and adds the estimate into it at record time, at the price then in effect (the class and the amount come from the model's config as it is at that moment). Showing a session's spend only sums what is stored — a later change of config, price or the built-in table re-prices nothing already made, and a model that leaves the config keeps its figure. A ledger written before classes and amounts were stored shows its tokens and claims nothing. Calls with a prompt over 100,000 tokens are also kept apart (`long`), because one model prices them differently.

A dollar figure is worked out for each call when it is made, and only for a model with a price:

- **The built-in table** (`provider/models/pricing.rs`) lists models of Anthropic's own API (provider `anthropic`, `api.anthropic.com`, a key — not a gateway, not a Claude subscription), each with the day the price was checked. USD per million tokens, input / output / cache read / cache write (the 5-minute write, 1.25x input — the engine sets no 1-hour ttl), as of 2026-11-03: claude-opus-5-5 4 / 20 / 0.20 / 5; claude-sonnet-5-5 2 / 10 / 0.10 / 2.50; claude-haiku-5-5 0.10 / 0.50 / 0.01 / 0.125 for a prompt up to 100,000 tokens and 0.50 / 2.50 / 0.05 / 0.625 above it, each request on its own; claude-fable-5-2 8 / 40 / 0.20 / 10.
- **`price` in the model's config** replaces the table's row, and prices a model the table does not list. A model with a `price` is metered whatever its host (a gateway on the LAN is not "local"). Every number must be finite and not negative, and a model that bills an account cannot have one. Strict when writing, lenient when reading: a save (the Models tab, `POST /api/config`) with a bad price is refused, the error naming the model and the field; a file already holding one loads anyway — that model alone loses its price (tokens, no dollars), the rest of the config is untouched, and the problem is an error line in the log and a `[FAIL] Config` line in `ling status`.
- **No price, no figure.** The tokens are shown and nothing is guessed. The figure is an estimate at list price; the bill can differ.
- **An account or local model has no per-token price** (the ChatGPT sign-in, Linggen Cloud, a room's model, a Claude subscription, a model on this machine's network): it shows its allowance meter or nothing, and a session's spend leaves its tokens out. The Claude SDK runtime's own cost figure (`claude_usage`) is the host's and stays as it is.

One answer serves both readers (`server/session_cost.rs`): `GET /api/workspace/state` (for the session's top bar — `ui/src/lib/spend.mts` only words it) and `agent_status` (for a caller telling the person what a delegated job cost) both carry `usage`, the engine's ledger, and for a Claude SDK session `claude_usage`, the host's own figure. `usage` lists each model's counts and `cost_usd` (only when priced), the tokens of the models that count as spend (each count block also carries its `prompt`), the part of them that was priced (`priced_tokens`) and the part that was not (`unpriced_tokens`), `spent` (any of them used at all), a total `cost_usd` over the priced ones, and `unpriced`, the models whose tokens are not in that total.

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

Settings → Models sets one default per kind of session:

- **Star** — the default for Linggen sessions (`routing.default_models`).
- **Moon** — the default for Claude sessions (`routing.claude_default_model`). Only Anthropic API-key models can take it. Unset, or the model no longer usable, Claude sessions fall back to the star, then to the first Claude API-key model in the list.

## Which model a turn runs on

One rule, `engine/model_choice::resolve(asked, standing, rule)`, for a chat turn, the companion's turn, a mission's run and a delegated run:

- **Asked** — the picker's model, the one a session keeps for a member, the companion's Pet setting (`auto`: Linggen Cloud when signed in, else nothing), or a mission's `model:`. A retired built-in id runs, and is kept, as its successor.
- **Nothing asked** — the place's standing model: the agent's own, from its chain (config override, frontmatter, the star, the first configured model).
- **Asked but cannot run** (not on this machine, or not shared with a room's guest) — the standing model runs instead, the kept pin is cleared, and the same `ModelFallback` notice as a mid-call fallback says which model answers and why.
- **Switching off** (`model_fallback: false`; a delegated `agent_run` session by default) — the asked model is kept and the turn fails on it, named. A guest's shared list is an access rule, not a failure: it holds either way.
- **A room's guest** runs only the models the room shares — before the turn and in the fallback below. A room that shares none refuses the turn.

`AgentEngine::run_on` is the one writer of a turn's model; a lock test fails on any other.

## Auto-fallback

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
