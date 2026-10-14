//! Configurable cheap-model endpoint client (non-default `nlq-endpoint` feature) — the
//! PRODUCT flavor of the PKG natural-language tool (`sq-2m6zm.6`).
//!
//! [`EndpointLlm`] is a thin, **provider-agnostic** [`Llm`] backed by an
//! OpenAI-compatible *chat-completions* endpoint: one blocking
//! `POST {base_url}/chat/completions` per [`Llm::complete`] call, with the prompt sent
//! as a single user message. It is the productization of the measured agent-flavor win
//! (`sq-zbyo7`: a cheap-model NL→SPARQL→run→NL round-trip is far cheaper than an
//! expensive model reading docs) for **external** users who bring their OWN endpoint.
//!
//! HONEST framing — this is a *configurable client*, not a bundled model:
//! - No provider key is baked in. The base URL, model id, and API key are **entirely
//!   user-supplied** (constructor args or environment variables). The crate ships no
//!   default endpoint and never phones home.
//! - Answer quality, latency, and cost depend **entirely on the user-chosen model and
//!   endpoint**. This crate makes no accuracy claim about any particular model — the
//!   offline exec-accuracy harness ([`crate::eval`]) is the only measured surface.
//! - The endpoint speaks the de-facto OpenAI chat-completions wire format (the format
//!   most local servers — Ollama, llama.cpp's `server`, vLLM, LM Studio, OpenRouter,
//!   and OpenAI itself — expose). It is NOT specific to any vendor.
//!
//! Like [`live`](crate::live), no test makes a live network call: the unit + integration
//! tests drive [`EndpointLlm`] against a loopback `TcpListener` stub returning a canned
//! chat-completion response, so the REAL request/response/parse path is exercised
//! offline.

use crate::Llm;

/// Default chat-completions output cap. Generated SPARQL is short; 2048 is generous.
const DEFAULT_MAX_TOKENS: u32 = 2048;

/// Default wall-clock cap on one endpoint round trip — [`Llm::complete`] must never hang
/// the loop on a stalled connection. Overridable via [`EndpointConfig::timeout`].
const DEFAULT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// Environment variable carrying the endpoint **base URL** (e.g.
/// `http://localhost:11434/v1` for a local Ollama server, or
/// `https://api.openai.com/v1`). Read by [`EndpointConfig::from_env`].
pub const ENV_BASE_URL: &str = "SPARQ_NLQ_ENDPOINT_URL";
/// Environment variable carrying the **model id** (the exact string the endpoint
/// expects, e.g. `llama3.1` or `gpt-4o-mini`). Read by [`EndpointConfig::from_env`].
pub const ENV_MODEL: &str = "SPARQ_NLQ_ENDPOINT_MODEL";
/// Environment variable carrying the **API key** sent as `Authorization: Bearer …`.
/// OPTIONAL: many local endpoints need no key, so an unset/empty value simply omits the
/// header. Read by [`EndpointConfig::from_env`].
pub const ENV_API_KEY: &str = "SPARQ_NLQ_ENDPOINT_KEY";

/// User-supplied configuration for [`EndpointLlm`]. Nothing here has a hard-coded
/// default endpoint or key — the URL and model are **required** and entirely
/// user-chosen; the key is optional (local endpoints often need none).
#[derive(Debug, Clone)]
pub struct EndpointConfig {
    /// Endpoint base URL. The client POSTs to `{base_url}/chat/completions`, so pass the
    /// OpenAI-style base (e.g. `https://api.openai.com/v1`), WITHOUT the trailing
    /// `/chat/completions`. A trailing `/` is tolerated.
    pub base_url: String,
    /// Model id, passed verbatim as the request's `model` field.
    pub model: String,
    /// Bearer token. `None` (or empty) omits the `Authorization` header — for local
    /// endpoints that need no key.
    pub api_key: Option<String>,
    /// Per-completion output token cap (`max_tokens`). Default `2048`.
    pub max_tokens: u32,
    /// Hard wall-clock cap on one round trip. Default `120s`.
    pub timeout: std::time::Duration,
}

impl EndpointConfig {
    /// A config from an explicit base URL + model, no key, with default caps. Chainable
    /// with [`with_api_key`](Self::with_api_key) / [`with_max_tokens`](Self::with_max_tokens)
    /// / [`with_timeout`](Self::with_timeout).
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            model: model.into(),
            api_key: None,
            max_tokens: DEFAULT_MAX_TOKENS,
            timeout: DEFAULT_TIMEOUT,
        }
    }

    /// Sets the bearer token (sent as `Authorization: Bearer <key>`).
    pub fn with_api_key(mut self, key: impl Into<String>) -> Self {
        self.api_key = Some(key.into());
        self
    }

    /// Overrides the per-completion output token cap.
    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = max_tokens;
        self
    }

    /// Overrides the per-round-trip wall-clock timeout.
    pub fn with_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Builds a config from the environment: [`ENV_BASE_URL`] and [`ENV_MODEL`] are
    /// REQUIRED (an unset/empty either is an error — the crate ships no default
    /// endpoint); [`ENV_API_KEY`] is optional (unset/empty omits the auth header). The
    /// token caps take their defaults. Returns `Err` describing the first missing
    /// variable so a CLI can print an actionable message.
    pub fn from_env() -> Result<Self, String> {
        let base_url = non_empty_env(ENV_BASE_URL)?;
        let model = non_empty_env(ENV_MODEL)?;
        let api_key = std::env::var(ENV_API_KEY).ok().filter(|s| !s.is_empty());
        Ok(Self {
            base_url,
            model,
            api_key,
            max_tokens: DEFAULT_MAX_TOKENS,
            timeout: DEFAULT_TIMEOUT,
        })
    }
}

/// Reads a REQUIRED non-empty environment variable, mapping unset/empty to a uniform
/// actionable error.
fn non_empty_env(var: &str) -> Result<String, String> {
    match std::env::var(var) {
        Ok(v) if !v.is_empty() => Ok(v),
        _ => Err(format!(
            "{var} is not set (it is required; the endpoint is user-configured — sparq-nlq \
             ships no default endpoint)"
        )),
    }
}

/// A configurable OpenAI-compatible chat-completions [`Llm`]. Construct from an
/// [`EndpointConfig`]; one blocking `POST {base_url}/chat/completions` per
/// [`Llm::complete`]. Provider-agnostic — works against any endpoint speaking the
/// OpenAI chat-completions wire format. See the [module docs](self) for the honest
/// framing (quality depends on the user-chosen model; nothing is baked in).
pub struct EndpointLlm {
    config: EndpointConfig,
    /// Pre-joined `{base_url}/chat/completions` request URL.
    request_url: String,
    client: reqwest::blocking::Client,
}

impl EndpointLlm {
    /// Builds the client from a [`EndpointConfig`]. Fails only if the underlying HTTP
    /// client cannot be constructed (e.g. the TLS backend is unavailable).
    pub fn new(config: EndpointConfig) -> Result<Self, String> {
        let client = reqwest::blocking::Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(|e| format!("cannot build HTTP client: {e}"))?;
        let request_url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));
        Ok(Self {
            config,
            request_url,
            client,
        })
    }

    /// Convenience: build from the environment (see [`EndpointConfig::from_env`]).
    pub fn from_env() -> Result<Self, String> {
        Self::new(EndpointConfig::from_env()?)
    }

    /// The model id this client targets (for reporting / logging).
    pub fn model(&self) -> &str {
        &self.config.model
    }
}

impl Llm for EndpointLlm {
    fn complete(&self, prompt: &str) -> Result<String, String> {
        let body = serde_json::json!({
            "model": self.config.model,
            "max_tokens": self.config.max_tokens,
            "messages": [{ "role": "user", "content": prompt }],
        });
        let mut req = self.client.post(&self.request_url).json(&body);
        if let Some(key) = self.config.api_key.as_deref().filter(|k| !k.is_empty()) {
            req = req.bearer_auth(key);
        }
        let resp = req
            .send()
            .map_err(|e| format!("endpoint request to {} failed: {e}", self.request_url))?;
        let status = resp.status();
        let v: serde_json::Value = resp
            .json()
            .map_err(|e| format!("endpoint response was not JSON: {e}"))?;
        if !status.is_success() {
            // OpenAI-style error envelope is `{"error": {"message": ...}}`; fall back to
            // the raw status when the body is shaped differently.
            let msg = v["error"]["message"]
                .as_str()
                .or_else(|| v["error"].as_str())
                .unwrap_or("unknown");
            return Err(format!("endpoint API error ({}): {}", status, msg));
        }
        // chat-completions: the assistant text is choices[0].message.content.
        let text = v["choices"][0]["message"]["content"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_default();
        if text.is_empty() {
            return Err("endpoint response carried no assistant message content".to_string());
        }
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `EndpointConfig` builder chain sets each field and leaves the others at their
    /// defaults — the config invariant the loopback-stub integration tests assume but never
    /// assert on `with_max_tokens` / `with_timeout`.
    #[test]
    fn config_builder_chain_sets_each_field() {
        let cfg = EndpointConfig::new("http://h/v1", "m")
            .with_api_key("sk-xyz")
            .with_max_tokens(64)
            .with_timeout(std::time::Duration::from_secs(7));
        assert_eq!(cfg.base_url, "http://h/v1");
        assert_eq!(cfg.model, "m");
        assert_eq!(cfg.api_key.as_deref(), Some("sk-xyz"));
        assert_eq!(cfg.max_tokens, 64);
        assert_eq!(cfg.timeout, std::time::Duration::from_secs(7));

        // A bare `new` leaves the documented defaults in place.
        let bare = EndpointConfig::new("http://h", "m");
        assert!(bare.api_key.is_none());
        assert_eq!(bare.max_tokens, DEFAULT_MAX_TOKENS);
        assert_eq!(bare.timeout, DEFAULT_TIMEOUT);
    }

    /// The request URL is `{base_url}/chat/completions` with at most one separating slash —
    /// a trailing slash on the base is trimmed (no doubled `//chat`), an absent one is
    /// added. Guards the `trim_end_matches('/')` join.
    #[test]
    fn request_url_join_normalises_trailing_slash() {
        let no_slash = EndpointLlm::new(EndpointConfig::new("http://h/v1", "m")).unwrap();
        assert_eq!(no_slash.request_url, "http://h/v1/chat/completions");
        assert_eq!(no_slash.model(), "m");

        let with_slash = EndpointLlm::new(EndpointConfig::new("http://h/v1/", "m")).unwrap();
        assert_eq!(with_slash.request_url, "http://h/v1/chat/completions");

        // Multiple trailing slashes are all trimmed (trim_end_matches strips the run).
        let many = EndpointLlm::new(EndpointConfig::new("http://h/v1///", "m")).unwrap();
        assert_eq!(many.request_url, "http://h/v1/chat/completions");
    }

    /// `non_empty_env` distinguishes "unset" and "empty" from a real value, mapping both
    /// failure modes to the same actionable error naming the variable. Uses a unique
    /// variable name so it does not race the integration-suite env tests.
    #[test]
    fn non_empty_env_rejects_unset_and_empty() {
        // Serialize every env-mutating test in this binary against ONE process-wide lock:
        // `set_var` / `remove_var` are NOT thread-safe (they mutate process-global state
        // and race any concurrent `getenv`/`setenv` from another thread — UB even though
        // the probe key below is unique to this test), and cargo runs this binary's unit
        // tests on a thread pool. Mirrors the `ENV_LOCK` guard in `tests/endpoint_stub.rs`.
        //
        static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());

        const VAR: &str = "SPARQ_NLQ_TEST_NON_EMPTY_ENV_PROBE";
        std::env::remove_var(VAR);
        let err = non_empty_env(VAR).unwrap_err();
        assert!(err.contains(VAR), "error names the variable: {err}");

        std::env::set_var(VAR, "");
        assert!(non_empty_env(VAR).is_err(), "empty value is rejected");

        std::env::set_var(VAR, "value");
        assert_eq!(non_empty_env(VAR).unwrap(), "value");
        std::env::remove_var(VAR);
    }
}
