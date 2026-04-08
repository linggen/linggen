---
type: spec
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
---

# Models

Hardware abstraction: model providers, routing policies, credentials, and auto-fallback.

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

### Default: ChatGPT via subscription

New installs default to GPT-5.4 via ChatGPT subscription. No API key needed — just sign in:

```bash
ling auth login     # Opens browser → sign in with OpenAI account
```

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

### Web UI setup

The easiest way to add models: open **Settings → Models** in the browser, click **Add Model**, pick a provider, paste your API key. The health indicator turns green when connected.

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

Star a model in **Settings → Models** to set it as the default. All new sessions use this model.

## Auto-fallback

When the active model returns a rate limit (HTTP 429) or context limit (HTTP 400) error, the engine automatically tries the next available model. On successful fallback, the engine switches to the fallback model for the rest of that run.

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
- `config.rs`: `ModelConfig`, `RoutingConfig`, routing policy definitions.
- `agent_manager/models.rs`: multi-provider dispatch, streaming, fallback error classification.
- `agent_manager/routing.rs`: model routing, complexity signal, policy resolution.
- `engine/mod.rs`: `stream_with_fallback()` — auto-retry with model fallback.
- `ollama.rs`: Ollama API client.
- `openai.rs`: OpenAI-compatible API client (used by all cloud providers).
- `codex_auth.rs`: ChatGPT OAuth token management.
