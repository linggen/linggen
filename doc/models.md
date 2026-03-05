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

- `agents.md`: per-agent model override.
- `product-spec.md`: multi-model design principle.
- `storage.md`: credentials file location.

## Providers

Linggen supports multiple model providers:

| Provider | Type | Config key |
|:---------|:-----|:-----------|
| Ollama | Local | `provider = "ollama"` |
| OpenAI-compatible | Cloud/Local | `provider = "openai"` |
| Claude API | Cloud | `provider = "openai"` (OpenAI-compatible endpoint) |
| AWS Bedrock | Cloud | `provider = "openai"` (via proxy) |

Models are configured in `linggen.toml`:

```toml
[[models]]
id = "local-qwen"
provider = "ollama"
url = "http://127.0.0.1:11434"
model = "qwen3:32b"

[[models]]
id = "gemini-flash"
provider = "openai"
url = "https://generativelanguage.googleapis.com/v1beta/openai"
model = "gemini-2.0-flash"
```

Optional `context_window` override for models where provider API doesn't report size.

## Credentials

API keys are stored in `~/.linggen/credentials.json` — **not** in the TOML config (which may be committed to git).

```json
{
  "gemini-flash": { "api_key": "AIza..." },
  "groq-llama": { "api_key": "gsk_..." }
}
```

**Resolution priority** (per model):
1. TOML `api_key` field (backward compatible, not recommended)
2. `~/.linggen/credentials.json` keyed by model `id`
3. Environment variable `LINGGEN_API_KEY_{ID}` (hyphens → underscores, uppercase)

**Web UI**: Settings → Models tab shows each model's API key field. Keys are saved via `PUT /api/credentials` to `credentials.json`, never to the TOML file.

**Implementation**: `credentials.rs` (store), `server/config_api.rs` (API endpoints).

## Free API providers

Several providers offer free tiers with OpenAI-compatible APIs:

| Provider | Free Tier | Models | Limits |
|----------|-----------|--------|--------|
| Google AI Studio | Free | Gemini 2.0 Flash, Gemini 2.5 Pro | 15 RPM, 1M TPD |
| Groq | Free | Llama 3.3 70B, Mixtral | 30 RPM, 14.4K TPD |
| DeepSeek | ~Free | DeepSeek-V3, DeepSeek-R1 | $0.14/M input tokens |
| OpenRouter | Free tier | Various (free-tagged) | Varies by model |
| GitHub Models | Free | GPT-4o-mini, Llama, Mistral | 15 RPM, 150K TPD |

All use `provider = "openai"`. Get a free API key from the provider's website.

## Auto-fallback

When a model returns a rate limit (HTTP 429) or context limit (HTTP 400) error, the engine automatically tries the next configured model. If that also fails, it tries the next, and so on until all models are exhausted.

On successful fallback, the engine switches to the fallback model for subsequent iterations in the same run.

**Implementation**: `engine/mod.rs` → `stream_with_fallback()`, `agent_manager/models.rs` → `is_fallback_worthy_error()`.

## Routing policies

Named policies control which model handles each request.

**Built-in policies**:
- `local-first` — prefer local models, fall back to cloud.
- `cloud-first` — prefer cloud models, fall back to local.

**Custom policies**:

```toml
[routing]
default_policy = "balanced"

[[routing.policies]]
name = "balanced"
rules = [
  { model = "qwen3:32b", priority = 1, max_complexity = "medium" },
  { model = "claude-sonnet-4-6", priority = 2 },
  { model = "claude-opus-4-6", priority = 3, min_complexity = "high" },
]
```

## Complexity signal

Estimated from:
- Prompt length.
- Tool call depth.
- Skill metadata (`model` hint in frontmatter).

Skills can declare `model: cloud` or `model: local` to influence routing.

## Per-agent model

Agents can specify `model` in frontmatter to override the routing policy:

```yaml
---
name: coder
model: claude-sonnet-4-6
---
```

## Implementation

- `credentials.rs`: credential store, resolution, API endpoints.
- `config.rs`: `ModelConfig`, `RoutingConfig`, routing policy definitions.
- `agent_manager/models.rs`: multi-provider dispatch, streaming, fallback error classification.
- `agent_manager/routing.rs`: model routing resolution.
- `engine/mod.rs`: `stream_with_fallback()` — auto-retry with model fallback.
- `ollama.rs`: Ollama API client.
- `openai.rs`: OpenAI-compatible API client.
