# Phase 10 — Local LLM Integration

**Goal:** local-only LLM augmentation for grammar breakdowns, example
sentence generation, definition rewriting, and kanji explanations.

**Time:** one weekend.

## Scope

**Local only.** No cloud providers, no API keys, no auth flows. The
network surface stays bounded to localhost.

The user picks any model they want. Small models for snappy lookups,
larger models for deep grammar analysis. The app suggests sensible
defaults but doesn't restrict.

## Supported backends

All speak HTTP (mostly OpenAI-compatible `/v1/chat/completions`):

| Backend | Default port | Setup |
|---|---|---|
| Ollama | 11434 | `brew install ollama && ollama pull qwen2.5:7b` |
| LM Studio | 1234 | GUI app; turn on local server |
| llama.cpp | 8080 | `llama-server -m model.gguf` |
| MLX | 8080 | `mlx_lm.server --model ...` |

One Provider impl covers all four.

## Crate layout

```
crates/jp-llm/
├─ Cargo.toml
├─ src/
│  ├─ lib.rs                # Provider trait + factory
│  ├─ http.rs               # OpenAI-compatible HTTP client
│  ├─ prompts/
│  │  ├─ mod.rs
│  │  ├─ grammar.rs         # grammar breakdown prompt
│  │  ├─ examples.rs        # example sentence generation
│  │  ├─ rewrite.rs         # definition rewriting
│  │  └─ kanji.rs           # kanji explanation
│  ├─ audit.rs              # log of LLM calls
│  └─ types.rs              # Prompt, Response, Provider
```

## Provider trait

```rust
// crates/jp-llm/src/lib.rs

#[async_trait::async_trait]
pub trait Provider: Send + Sync {
    async fn complete(&self, prompt: Prompt) -> Result<Response>;
    fn name(&self) -> &str;
    async fn list_models(&self) -> Result<Vec<String>>;
    async fn ping(&self) -> Result<()>;
}

pub struct Prompt {
    pub system: String,
    pub user: String,
    pub max_tokens: u32,
    pub temperature: f32,
}

pub struct Response {
    pub text: String,
    pub model: String,
    pub tokens_used: Option<u32>,
}
```

## HTTP impl (covers all backends)

```rust
// crates/jp-llm/src/http.rs

pub struct LocalHttpProvider {
    client: reqwest::Client,
    base_url: String,
    model: String,
}

#[async_trait::async_trait]
impl Provider for LocalHttpProvider {
    async fn complete(&self, prompt: Prompt) -> Result<Response> {
        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": prompt.system },
                { "role": "user", "content": prompt.user }
            ],
            "max_tokens": prompt.max_tokens,
            "temperature": prompt.temperature,
            "stream": false
        });

        let resp = self.client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .json(&body)
            .send()
            .await?
            .error_for_status()?;

        let json: serde_json::Value = resp.json().await?;
        let text = json["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let tokens_used = json["usage"]["total_tokens"].as_u64().map(|n| n as u32);

        Ok(Response { text, model: self.model.clone(), tokens_used })
    }

    fn name(&self) -> &str { "local" }

    async fn list_models(&self) -> Result<Vec<String>> {
        // GET /v1/models — supported by Ollama, LM Studio, llama.cpp,
        // mlx_lm.server. Falls back to "user-configured model only" if
        // the endpoint doesn't support it.
        let resp = self.client
            .get(format!("{}/v1/models", self.base_url))
            .send().await?;
        if !resp.status().is_success() {
            return Ok(vec![self.model.clone()]);
        }
        let json: serde_json::Value = resp.json().await?;
        Ok(json["data"]
            .as_array()
            .map(|a| a.iter()
                .filter_map(|m| m["id"].as_str().map(String::from))
                .collect())
            .unwrap_or_else(|| vec![self.model.clone()]))
    }

    async fn ping(&self) -> Result<()> {
        // Most servers respond to /v1/models even without an active
        // request. Use it as a liveness check.
        let resp = self.client
            .get(format!("{}/v1/models", self.base_url))
            .timeout(Duration::from_millis(500))
            .send().await?;
        if resp.status().is_success() { Ok(()) }
        else { anyhow::bail!("ping failed: {}", resp.status()) }
    }
}
```

## Prompt templates

These live in code, not config — they're code, not data. Users can fork
to customize. Each template has a fixed structure, takes structured
input, returns parseable output.

**Grammar breakdown:**

```rust
// crates/jp-llm/src/prompts/grammar.rs

pub struct GrammarBreakdownInput {
    pub sentence: String,
    pub target_word: Option<String>,    // word the user is currently looking at
}

pub fn build(input: &GrammarBreakdownInput) -> Prompt {
    Prompt {
        system: "You are a Japanese grammar tutor. \
                 Break down sentences into their grammatical components. \
                 Be concise. Use plain text, not markdown. \
                 If a target word is specified, focus your analysis on its role."
            .into(),
        user: match &input.target_word {
            Some(w) => format!(
                "Sentence: {}\n\nFocus on: {}\n\nProvide:\n\
                1. A natural English translation\n\
                2. Word-by-word breakdown with grammatical roles\n\
                3. Why the focus word is in this form",
                input.sentence, w
            ),
            None => format!(
                "Sentence: {}\n\nProvide:\n\
                1. A natural English translation\n\
                2. Word-by-word breakdown with grammatical roles",
                input.sentence
            ),
        },
        max_tokens: 600,
        temperature: 0.3,
    }
}
```

**Example sentence generation:**

```rust
pub struct ExamplesInput {
    pub word: String,
    pub reading: String,
    pub gloss: String,
    pub level: ExampleLevel,    // N5/N4/N3/N2/N1
    pub count: usize,
}
```

**Definition rewriting:**

```rust
pub struct RewriteInput {
    pub word: String,
    pub original_gloss: String,    // from JMdict, often terse
    pub style: RewriteStyle,       // Plain, Formal, Casual, ELI5
}
```

**Kanji explanation:**

```rust
pub struct KanjiInput {
    pub character: char,
    pub include_radicals: bool,
    pub include_etymology: bool,
}
```

## Output sanitization

LLM output is treated as untrusted input before display or storage:

```rust
pub fn sanitize_output(text: &str) -> String {
    // Strip control characters (except common whitespace).
    // Strip any HTML tags — output is plain text only.
    // Truncate at the configured max_tokens equivalent in chars.
    text.chars()
        .filter(|c| c.is_ascii_whitespace() || !c.is_ascii_control())
        .collect::<String>()
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
```

If LLM output ends up in an Anki card field, it's already
`html_escape`'d by the handlebars template engine. But if a user pastes
LLM output into a vocab `notes` field, we treat it as plain text only.

## Settings UI

```rust
pub struct LlmSettings {
    pub enabled: bool,
    pub base_url: String,                     // default: http://127.0.0.1:11434
    pub model: String,                        // default: depends on detected models
    pub features: LlmFeatures,
}

pub struct LlmFeatures {
    pub grammar_breakdown: bool,
    pub example_sentences: bool,
    pub definition_rewrite: bool,
    pub kanji_explanation: bool,
}
```

UI flow:

1. Master toggle: "Enable LLM features"
2. Endpoint URL field with validation: "must be localhost or 127.0.0.1"
3. Model dropdown populated from `list_models`
4. "Test connection" button — calls `ping`, shows status
5. Per-feature toggles
6. Audit log link: "View LLM activity (last 100 calls)"

If the user turns LLM on but no provider is reachable, show a setup
helper:

```
LLM features need a local model server. Recommended setup:

  brew install ollama
  ollama pull qwen2.5:7b
  ollama serve

You can use other models or backends — see the docs.
Note: model performance is bounded by your hardware. Larger models
(13B+) need ~12GB+ RAM.

[Try connection]
```

The setup helper deliberately doesn't push specific models. It suggests
one as an example; users with stronger hardware can pick larger ones,
users with weaker hardware can pick smaller ones, users with strong
opinions can pick whatever.

## Capability allowlist

```json
{
  "identifier": "llm",
  "windows": ["main", "reader-*"],
  "permissions": [
    {
      "identifier": "http:default",
      "allow": [
        { "url": "http://localhost:11434/*" },
        { "url": "http://127.0.0.1:11434/*" },
        { "url": "http://localhost:1234/*" },
        { "url": "http://127.0.0.1:1234/*" },
        { "url": "http://localhost:8080/*" },
        { "url": "http://127.0.0.1:8080/*" }
      ]
    }
  ]
}
```

If a user wants a non-default port, they edit the allowlist via settings,
which validates `URL parses as http/https + host is localhost or 127.0.0.1`
before adding. No frontend-injectable URL.

## Audit log

```sql
-- Migration 004: LLM audit
CREATE TABLE llm_call (
    id INTEGER PRIMARY KEY,
    timestamp INTEGER NOT NULL,
    feature TEXT NOT NULL,            -- 'grammar' | 'examples' | 'rewrite' | 'kanji'
    model TEXT NOT NULL,
    prompt_tokens INTEGER,
    completion_tokens INTEGER,
    duration_ms INTEGER NOT NULL,
    succeeded INTEGER NOT NULL,
    error TEXT
);
CREATE INDEX idx_llm_time ON llm_call(timestamp DESC);
```

Every LLM call gets a row. The settings UI's "View LLM activity" reads
the last 100 rows. Helps users debug and gives them visibility into
what the app is doing locally.

We do **not** log prompts or completions — those could contain content
the user hasn't agreed to log (e.g., copyrighted manga panel they OCR'd).

## Wiring into existing UI

Lookup popup gains an LLM section if enabled:

- "Explain in context" button — runs grammar breakdown on the
  encounter sentence
- "More examples" button — runs example generation
- "Rewrite definition" toggle — replaces JMdict glosses with rewritten
  ones inline

Card mining: if grammar breakdown is enabled, an LLM-generated
breakdown can be auto-appended to the sentence field via a handlebars
helper:

```
{{sentence}}<br><small>{{llm-breakdown sentence target=expression}}</small>
```

The helper is registered in the handlebars engine and calls
`provider.complete()` synchronously during card render. Long card
renders aren't great UX; document the latency tradeoff.

## Hardware reality

Worth surfacing in the setup screen:

> Local model performance depends on your hardware. As a rough guide:
>
> - 8 GB RAM, integrated GPU: small models only (3-4B params, quantized).
>   Workable for definition rewriting and short queries.
> - 16 GB RAM, Apple Silicon: 7-8B models work well. Good for grammar
>   breakdowns.
> - 32 GB+ RAM: 13-30B models. Better quality but slower.
>
> Quality is bounded by what runs on your machine. If you need
> frontier-quality grammar analysis on hard literary Japanese,
> consider running tasks through a separate workflow rather than
> in-app.

This is honest framing — local LLMs trade off against cloud frontier
models, and it's better to say so than to overpromise.

## Acceptance criteria

- [ ] `ping` correctly reports backend availability
- [ ] `list_models` populates dropdown for Ollama, LM Studio, llama.cpp
- [ ] Grammar breakdown returns coherent output for a typical Aozora
      sentence
- [ ] Example sentences generated at requested JLPT level
- [ ] Definition rewrite preserves the meaning of JMdict gloss
- [ ] Kanji explanation includes radicals and meanings
- [ ] Output sanitization strips HTML tags and control chars
- [ ] Audit log records every call with timing
- [ ] Capability allowlist rejects non-localhost URLs
- [ ] Settings UI gracefully handles "no provider reachable" state
- [ ] Per-feature toggles work — disabling a feature stops calls

## Common Phase 10 problems

**Streaming responses:** Ollama and LM Studio support SSE streaming.
Phase 10 uses non-streaming for simplicity. Add streaming later if
latency matters for your use cases — most local lookups complete in
1-3s, which is fine without streaming.

**Model returns wrong language:** small models sometimes ignore the
"answer in English" instruction and respond in Japanese. Adjust the
system prompt to be more emphatic, or pick a model with better
instruction-following.

**Long card render times:** if a card template includes
`{{llm-breakdown ...}}`, mining waits for the LLM call. For users with
slow models, this is annoying. Mitigation: render with a placeholder,
add the LLM result to the card via AnkiConnect's `updateNote` after
the fact.

**Endpoint changes between Ollama versions:** Ollama's API has
stabilized but can shift. Pin behavior to v1-style chat completions
(OpenAI-compatible). If a user runs into incompatibilities, they pick
a different backend.

**Hardware mismatches:** users on weak machines pick a 13B model and
the app feels broken. Surface latency in the audit log; if average
latency > 10s for any feature, suggest in settings: "Consider a
smaller model — your average latency is N seconds."

## What this enables but doesn't include

LLM features open up plausible future work:

- Auto-suggesting words to track from a reading session
- Auto-categorizing tracked words into themes
- Generating personalized review summaries
- Translating user-provided notes between Japanese and English

All of these are downstream of having a working LLM provider and
vocab DB. Add them as features when you find yourself wanting them.

## Final phase

Phase 10 is the last documented phase. The app is complete. Future
work is feature additions, not new phases.

Update `JOURNAL.md`. Commit. Use the app daily and iterate based on
real usage.
