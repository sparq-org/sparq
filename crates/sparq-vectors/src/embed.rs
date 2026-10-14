//! Embedding providers. The engine never runs a model in-process (design decision:
//! embeddings are produced out-of-process); this module defines the provider-agnostic
//! [`Embedder`] trait, a deterministic **test-only** [`HashEmbedder`], and — behind the
//! non-default `provider` feature — the API shape for a live OpenAI-compatible
//! `/v1/embeddings` provider whose HTTP transport is supplied by the caller.

/// A batch text-embedding provider. Implementations must be deterministic per input
/// within one store build (vectors written under different provider states do not
/// compare meaningfully).
pub trait Embedder {
    /// The dimension of every vector this embedder produces.
    fn dim(&self) -> usize;
    /// Embeds each text; returns one `dim()`-length vector per input, in input order.
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, String>;
}

/// **Test-only** deterministic embedder: a random sign projection of character-trigram
/// and word hashes. Texts sharing n-grams get correlated vectors, so lexical similarity
/// becomes vector similarity — enough to exercise the store, the ANN indexes and the
/// label pipeline **without any model or network**. It captures NO semantics ("car" and
/// "automobile" are unrelated); do not ship it as a retrieval feature.
///
/// Per token `t`: a 64-bit FNV-1a hash seeds a SplitMix64 stream that contributes
/// `±1` to each of the `dim` components; the accumulated vector is L2-normalized.
/// Deterministic across platforms and runs for a fixed `(dim, seed)`.
pub struct HashEmbedder {
    dim: usize,
    seed: u64,
}

impl HashEmbedder {
    pub fn new(dim: usize) -> HashEmbedder {
        HashEmbedder::with_seed(dim, 0x5354_5156_5645_4354) // "SPQV"-flavored default
    }

    pub fn with_seed(dim: usize, seed: u64) -> HashEmbedder {
        assert!(dim > 0, "HashEmbedder dim must be > 0");
        HashEmbedder { dim, seed }
    }

    fn embed_one(&self, text: &str) -> Vec<f32> {
        let mut acc = vec![0f32; self.dim];
        let lower = text.to_lowercase();
        // Words…
        for word in lower.split_whitespace() {
            self.add_token(&mut acc, word.as_bytes());
        }
        // …and character trigrams over the whitespace-normalized, padded text, so
        // near-matches ("Athina" / "Athens 1896") still share mass.
        let padded: Vec<char> = std::iter::once(' ')
            .chain(lower.split_whitespace().collect::<Vec<_>>().join(" ").chars())
            .chain(std::iter::once(' '))
            .collect();
        for window in padded.windows(3) {
            let s: String = window.iter().collect();
            self.add_token(&mut acc, s.as_bytes());
        }
        let norm = acc.iter().map(|v| v * v).sum::<f32>().sqrt();
        if norm > 0.0 {
            for v in &mut acc {
                *v /= norm;
            }
        }
        acc
    }

    fn add_token(&self, acc: &mut [f32], token: &[u8]) {
        let mut state = fnv1a64(token) ^ self.seed;
        for slot in acc.iter_mut() {
            *slot += if splitmix64(&mut state) & 1 == 1 { 1.0 } else { -1.0 };
        }
    }
}

impl Embedder for HashEmbedder {
    fn dim(&self) -> usize {
        self.dim
    }

    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, String> {
        Ok(texts.iter().map(|t| self.embed_one(t)).collect())
    }
}

#[inline]
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

#[inline]
pub(crate) fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E3779B97F4A7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}

/// Live-provider API shape (non-default `provider` feature): an OpenAI-compatible
/// `/v1/embeddings` client where the **HTTP transport is supplied by the caller** —
/// this crate builds the request body and parses the response, but never opens a
/// socket (no HTTP client dependency; tests stay offline).
#[cfg(feature = "provider")]
pub mod provider {
    use super::Embedder;

    /// One HTTP POST with a JSON body, supplied by the caller over their HTTP client
    /// of choice (reqwest, ureq, a recorded cassette in tests, …). Returns the
    /// response body on 2xx, `Err` otherwise.
    pub trait Transport {
        fn post_json(
            &self,
            url: &str,
            headers: &[(String, String)],
            body: &str,
        ) -> Result<String, String>;
    }

    /// Configuration for an OpenAI-compatible `/v1/embeddings` endpoint.
    pub struct ProviderConfig {
        /// Full endpoint URL, e.g. `https://api.openai.com/v1/embeddings`.
        pub endpoint: String,
        /// Model name passed in the request body, e.g. `text-embedding-3-small`.
        pub model: String,
        /// Sent as `Authorization: Bearer …` when present.
        pub api_key: Option<String>,
        /// Expected vector dimension; responses with other dimensions are an error.
        pub dim: usize,
    }

    /// An [`Embedder`] over a remote OpenAI-compatible embeddings API.
    pub struct RemoteEmbedder<T: Transport> {
        cfg: ProviderConfig,
        transport: T,
    }

    impl<T: Transport> RemoteEmbedder<T> {
        pub fn new(cfg: ProviderConfig, transport: T) -> RemoteEmbedder<T> {
            RemoteEmbedder { cfg, transport }
        }
    }

    // Concrete reqwest blocking transport + env-driven constructor
    // (non-default `embeddings` feature, sq-fg9y). This is the piece that makes the
    // crate usable for real retrieval WITHOUT caller glue: with the feature on, a
    // `RemoteEmbedder` can be built straight from environment variables and embeds
    // against a live OpenAI-compatible `/v1/embeddings` endpoint. Mirrors
    // `sparq-nlq`'s `live` feature (`AnthropicLlm::from_env`): blocking reqwest, key
    // from env, a hard per-request timeout, status->error mapping. The DEFAULT build
    // pulls none of this (no reqwest), so it stays socket-free and wasm-safe.
    #[cfg(feature = "embeddings")]
    pub use live::{ReqwestTransport, DEFAULT_ENDPOINT, DEFAULT_MODEL};

    #[cfg(feature = "embeddings")]
    mod live {
        use super::{ProviderConfig, RemoteEmbedder, Transport};

        /// Default endpoint when `SPARQ_EMBEDDINGS_BASE_URL` is unset — OpenAI's
        /// `/v1/embeddings`. Any OpenAI-compatible server works (point the base URL
        /// at it).
        pub const DEFAULT_ENDPOINT: &str = "https://api.openai.com/v1/embeddings";
        /// Default model when `SPARQ_EMBEDDINGS_MODEL` is unset.
        pub const DEFAULT_MODEL: &str = "text-embedding-3-small";

        /// Hard wall-clock cap on one `/v1/embeddings` round trip — `Embedder::embed`
        /// must never hang on a stalled connection. Matches `sparq-nlq`'s live client.
        const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

        /// A blocking [`reqwest`] [`Transport`]: one `POST` with the JSON body the
        /// caller built, returning the response body on 2xx and an error string
        /// (status + provider message when JSON) otherwise. Blocking by design — the
        /// embed loop is synchronous and this crate must not drag in an async runtime.
        pub struct ReqwestTransport {
            client: reqwest::blocking::Client,
        }

        impl ReqwestTransport {
            /// Builds the blocking client with the standard request timeout.
            pub fn new() -> Result<ReqwestTransport, String> {
                let client = reqwest::blocking::Client::builder()
                    .timeout(REQUEST_TIMEOUT)
                    .build()
                    .map_err(|e| format!("cannot build HTTP client: {e}"))?;
                Ok(ReqwestTransport { client })
            }
        }

        impl Transport for ReqwestTransport {
            fn post_json(
                &self,
                url: &str,
                headers: &[(String, String)],
                body: &str,
            ) -> Result<String, String> {
                let mut req = self.client.post(url).body(body.to_owned());
                for (k, v) in headers {
                    req = req.header(k, v);
                }
                let resp = req
                    .send()
                    .map_err(|e| format!("embeddings request failed: {e}"))?;
                let status = resp.status();
                let text = resp
                    .text()
                    .map_err(|e| format!("cannot read embeddings response body: {e}"))?;
                if !status.is_success() {
                    // Surface the provider's `error.message` when the body is JSON,
                    // else a truncated raw body — same shape as the nlq live client.
                    let message = serde_json::from_str::<serde_json::Value>(&text)
                        .ok()
                        .and_then(|v| v["error"]["message"].as_str().map(str::to_owned))
                        .unwrap_or_else(|| text.chars().take(200).collect());
                    return Err(format!("embeddings API error ({status}): {message}"));
                }
                Ok(text)
            }
        }

        impl RemoteEmbedder<ReqwestTransport> {
            /// Build a live embedder entirely from the environment, mirroring
            /// `sparq-nlq`'s `AnthropicLlm::from_env`:
            ///
            /// - `SPARQ_EMBEDDINGS_API_KEY` (**required**) → `Authorization: Bearer …`;
            /// - `SPARQ_EMBEDDINGS_BASE_URL` (optional) → endpoint, default
            ///   [`DEFAULT_ENDPOINT`];
            /// - `SPARQ_EMBEDDINGS_MODEL` (optional) → model, default [`DEFAULT_MODEL`].
            ///
            /// `dim` is passed explicitly because it must match the
            /// [`VectorStore`](crate::VectorStore) it will write into — every response
            /// vector is checked against it (a mismatch is an error, never silently
            /// truncated). Returns `Err` if the key is unset or the HTTP client cannot
            /// be built.
            pub fn from_env(dim: usize) -> Result<RemoteEmbedder<ReqwestTransport>, String> {
                let api_key = std::env::var("SPARQ_EMBEDDINGS_API_KEY")
                    .map_err(|_| "SPARQ_EMBEDDINGS_API_KEY is not set".to_string())?;
                let endpoint = std::env::var("SPARQ_EMBEDDINGS_BASE_URL")
                    .unwrap_or_else(|_| DEFAULT_ENDPOINT.to_string());
                let model = std::env::var("SPARQ_EMBEDDINGS_MODEL")
                    .unwrap_or_else(|_| DEFAULT_MODEL.to_string());
                let cfg = ProviderConfig {
                    endpoint,
                    model,
                    api_key: Some(api_key),
                    dim,
                };
                Ok(RemoteEmbedder::new(cfg, ReqwestTransport::new()?))
            }
        }

        #[cfg(test)]
        mod tests {
            use super::super::Embedder; // RemoteEmbedder::dim is from the Embedder trait
            use super::*;

            // Env vars are process-global; serialize every test that reads OR mutates
            // them so they don't race when the binary runs them in parallel.
            static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

            // The full set of SPARQ_EMBEDDINGS_* vars any test in this
            // module reads or writes — the guard snapshots/restores exactly these.
            const ENV_VARS: [&str; 4] = [
                "SPARQ_EMBEDDINGS_API_KEY",
                "SPARQ_EMBEDDINGS_BASE_URL",
                "SPARQ_EMBEDDINGS_MODEL",
                "SPARQ_EMBEDDINGS_DIM",
            ];

            /// RAII env guard: on construction it takes [`ENV_LOCK`] (so all
            /// env-touching tests are serialized — including the live round-trip, which
            /// only *reads* the vars) and snapshots the current value of every
            /// [`ENV_VARS`] entry; on drop it restores each to exactly what it was —
            /// re-`set_var` if it had a value, `remove_var` if it was unset. Restore
            /// runs on the unwinding path too, so a panicking test never leaks mutated
            /// (or removed) `SPARQ_EMBEDDINGS_*` state into the rest of the process or a
            /// developer's shell. Replaces the old destructive `clear_env()` helper that
            /// wiped pre-existing values without restoring them.
            struct EnvGuard {
                _lock: std::sync::MutexGuard<'static, ()>,
                saved: Vec<(&'static str, Option<String>)>,
            }

            impl EnvGuard {
                /// Acquire the lock + snapshot. Does NOT mutate the environment — call
                /// [`EnvGuard::clear`] for the cleared-state unit tests; the live test
                /// keeps the user-set opt-in vars and just relies on the lock + restore.
                fn acquire() -> EnvGuard {
                    // `.unwrap_or_else(|e| e.into_inner())` so a panic in a prior test
                    // (which poisons the mutex) doesn't cascade-fail every other one;
                    // we still serialize and the restore-on-drop keeps state clean.
                    let lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
                    let saved =
                        ENV_VARS.iter().map(|&k| (k, std::env::var(k).ok())).collect();
                    EnvGuard { _lock: lock, saved }
                }

                /// Remove every `SPARQ_EMBEDDINGS_*` var, for tests that need a known
                /// empty starting point. The originals are still restored on drop.
                fn clear(&self) {
                    for &k in &ENV_VARS {
                        std::env::remove_var(k);
                    }
                }
            }

            impl Drop for EnvGuard {
                fn drop(&mut self) {
                    for (k, v) in &self.saved {
                        match v {
                            Some(val) => std::env::set_var(k, val),
                            None => std::env::remove_var(k),
                        }
                    }
                }
            }

            /// No key set → a clear error, and crucially NO network/socket touched
            /// (the client is never even built). This is the default-CI path.
            #[test]
            fn from_env_errors_without_key() {
                let g = EnvGuard::acquire();
                g.clear();
                // RemoteEmbedder isn't Debug, so unwrap the Result by hand rather than
                // `expect_err`.
                let err = match RemoteEmbedder::from_env(8) {
                    Ok(_) => panic!("missing key must be an error"),
                    Err(e) => e,
                };
                assert!(
                    err.contains("SPARQ_EMBEDDINGS_API_KEY"),
                    "error names the missing var, got: {err}"
                );
            }

            /// Key only → defaults for endpoint + model; `dim` is honored.
            #[test]
            fn from_env_uses_defaults_for_base_url_and_model() {
                let g = EnvGuard::acquire();
                g.clear();
                std::env::set_var("SPARQ_EMBEDDINGS_API_KEY", "sk-test");
                let e = RemoteEmbedder::from_env(1536).expect("builds with just a key");
                assert_eq!(e.dim(), 1536);
                assert_eq!(e.cfg.endpoint, DEFAULT_ENDPOINT);
                assert_eq!(e.cfg.model, DEFAULT_MODEL);
                assert_eq!(e.cfg.api_key.as_deref(), Some("sk-test"));
            }

            /// All three env vars set → all three are read (point at a self-hosted
            /// OpenAI-compatible server).
            #[test]
            fn from_env_reads_base_url_and_model_overrides() {
                let g = EnvGuard::acquire();
                g.clear();
                std::env::set_var("SPARQ_EMBEDDINGS_API_KEY", "sk-x");
                std::env::set_var("SPARQ_EMBEDDINGS_BASE_URL", "http://localhost:8080/v1/embeddings");
                std::env::set_var("SPARQ_EMBEDDINGS_MODEL", "bge-small");
                let e = RemoteEmbedder::from_env(384).expect("builds with overrides");
                assert_eq!(e.cfg.endpoint, "http://localhost:8080/v1/embeddings");
                assert_eq!(e.cfg.model, "bge-small");
                assert_eq!(e.dim(), 384);
            }

            /// LIVE network test — opt-in only. `#[ignore]` by default (mirrors
            /// `sparq-nlq`'s live-model test), and additionally a no-op unless
            /// `SPARQ_EMBEDDINGS_API_KEY` is set, so it never hits the network in CI
            /// even if explicitly un-ignored. Run with:
            ///   `SPARQ_EMBEDDINGS_API_KEY=… cargo test -p sparq-vectors \
            ///       --features embeddings -- --ignored embeddings_live`
            #[test]
            #[ignore = "hits a live /v1/embeddings endpoint; needs SPARQ_EMBEDDINGS_API_KEY"]
            fn embeddings_live_round_trip() {
                // Take ENV_LOCK (via the guard) so this serializes against
                // the env-parsing unit tests when run together under `--ignored`; do NOT
                // `clear()`, the user-set SPARQ_EMBEDDINGS_* opt-in vars are the input.
                let _g = EnvGuard::acquire();
                if std::env::var("SPARQ_EMBEDDINGS_API_KEY").is_err() {
                    eprintln!("skipping: SPARQ_EMBEDDINGS_API_KEY not set");
                    return;
                }
                // Dimension of OpenAI text-embedding-3-small; override the model/dim
                // via env if you point at a different server.
                let dim = std::env::var("SPARQ_EMBEDDINGS_DIM")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(1536usize);
                let e = RemoteEmbedder::from_env(dim).expect("from_env");
                let vecs = e
                    .embed(&["the quick brown fox", "a slow green turtle"])
                    .expect("live embed call");
                assert_eq!(vecs.len(), 2);
                assert!(vecs.iter().all(|v| v.len() == dim));
                assert!(vecs.iter().all(|v| v.iter().any(|x| *x != 0.0)));
            }
        }
    }

    impl<T: Transport> Embedder for RemoteEmbedder<T> {
        fn dim(&self) -> usize {
            self.cfg.dim
        }

        fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, String> {
            if texts.is_empty() {
                return Ok(Vec::new());
            }
            let body = serde_json::json!({ "model": self.cfg.model, "input": texts });
            let mut headers = vec![("Content-Type".to_string(), "application/json".to_string())];
            if let Some(key) = &self.cfg.api_key {
                headers.push(("Authorization".to_string(), format!("Bearer {key}")));
            }
            let response =
                self.transport.post_json(&self.cfg.endpoint, &headers, &body.to_string())?;
            let parsed: serde_json::Value =
                serde_json::from_str(&response).map_err(|e| format!("bad response JSON: {e}"))?;
            let data = parsed["data"]
                .as_array()
                .ok_or_else(|| "response has no `data` array".to_string())?;
            if data.len() != texts.len() {
                return Err(format!("asked for {} embeddings, got {}", texts.len(), data.len()));
            }
            // The API may return out of order; honor the `index` field. Each input
            // index must appear exactly once — a duplicate or missing index is a
            // protocol violation (the Embedder contract is one dim()-length vector
            // per input, in input order).
            let mut out: Vec<Option<Vec<f32>>> = vec![None; texts.len()];
            for item in data {
                let i = item["index"].as_u64().ok_or("embedding without `index`")? as usize;
                let emb = item["embedding"].as_array().ok_or("embedding without `embedding`")?;
                let v: Vec<f32> =
                    emb.iter().map(|x| x.as_f64().unwrap_or(f64::NAN) as f32).collect();
                if v.len() != self.cfg.dim || v.iter().any(|x| !x.is_finite()) {
                    return Err(format!(
                        "embedding {i} has dim {} (expected {}) or non-finite values",
                        v.len(),
                        self.cfg.dim
                    ));
                }
                let slot = out.get_mut(i).ok_or(format!("embedding index {i} out of range"))?;
                if slot.is_some() {
                    return Err(format!("duplicate embedding index {i} in response"));
                }
                *slot = Some(v);
            }
            out.into_iter()
                .enumerate()
                .map(|(i, v)| v.ok_or(format!("response is missing embedding index {i}")))
                .collect()
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        struct Fake;
        impl Transport for Fake {
            fn post_json(&self, url: &str, headers: &[(String, String)], body: &str) -> Result<String, String> {
                assert_eq!(url, "https://example.test/v1/embeddings");
                assert!(headers.iter().any(|(k, v)| k == "Authorization" && v == "Bearer k"));
                assert!(body.contains("\"model\":\"m\""));
                // Out-of-order response to exercise `index` handling.
                Ok(r#"{"data":[
                    {"index":1,"embedding":[0.5,0.5]},
                    {"index":0,"embedding":[1.0,0.0]}
                ]}"#
                .to_string())
            }
        }

        #[test]
        fn remote_embedder_shapes_request_and_orders_response() {
            let e = RemoteEmbedder::new(
                ProviderConfig {
                    endpoint: "https://example.test/v1/embeddings".into(),
                    model: "m".into(),
                    api_key: Some("k".into()),
                    dim: 2,
                },
                Fake,
            );
            let got = e.embed(&["a", "b"]).unwrap();
            assert_eq!(got, vec![vec![1.0, 0.0], vec![0.5, 0.5]]);
        }

        /// A duplicate `index` in the response (which also leaves another index
        /// missing) must be rejected, not silently yield an empty vector.
        struct DupIndex;
        impl Transport for DupIndex {
            fn post_json(&self, _: &str, _: &[(String, String)], _: &str) -> Result<String, String> {
                Ok(r#"{"data":[
                    {"index":1,"embedding":[0.5,0.5]},
                    {"index":1,"embedding":[1.0,0.0]}
                ]}"#
                .to_string())
            }
        }

        #[test]
        fn remote_embedder_rejects_duplicate_response_indexes() {
            let e = RemoteEmbedder::new(
                ProviderConfig {
                    endpoint: "https://example.test/v1/embeddings".into(),
                    model: "m".into(),
                    api_key: None,
                    dim: 2,
                },
                DupIndex,
            );
            let err = e.embed(&["a", "b"]).unwrap_err();
            assert!(err.contains("duplicate embedding index 1"), "got: {err}");
        }

        // The rest of the `RemoteEmbedder::embed` response-validation contract — the
        // fail-closed protocol-violation branches. Each is a *distinct* error a buggy or hostile
        // provider could trigger; the Embedder contract is "one finite, dim()-length vector per
        // input, in input order", and a violation must be a descriptive `Err`, never a silently
        // wrong vector. A regression that drops any of these checks would let a malformed response
        // poison the store, so each branch gets a transport that produces exactly that violation.

        /// A config with `dim: 2` and a transport returning `body` verbatim — the harness for the
        /// response-validation tests. `api_key: None` so no auth header is required.
        fn embed_with_body(body: &'static str) -> Result<Vec<Vec<f32>>, String> {
            struct Body(&'static str);
            impl Transport for Body {
                fn post_json(&self, _: &str, _: &[(String, String)], _: &str) -> Result<String, String> {
                    Ok(self.0.to_string())
                }
            }
            let e = RemoteEmbedder::new(
                ProviderConfig {
                    endpoint: "https://example.test/v1/embeddings".into(),
                    model: "m".into(),
                    api_key: None,
                    dim: 2,
                },
                Body(body),
            );
            e.embed(&["a", "b"])
        }

        #[test]
        fn remote_embedder_empty_input_short_circuits_without_a_request() {
            // An empty `texts` returns `Ok([])` WITHOUT calling the transport at all — a transport
            // that panics if hit proves no request is sent.
            struct NeverCalled;
            impl Transport for NeverCalled {
                fn post_json(&self, _: &str, _: &[(String, String)], _: &str) -> Result<String, String> {
                    panic!("empty input must not issue a request");
                }
            }
            let e = RemoteEmbedder::new(
                ProviderConfig {
                    endpoint: "https://example.test/v1/embeddings".into(),
                    model: "m".into(),
                    api_key: None,
                    dim: 2,
                },
                NeverCalled,
            );
            assert_eq!(e.embed(&[]).unwrap(), Vec::<Vec<f32>>::new());
        }

        #[test]
        fn remote_embedder_propagates_a_transport_error() {
            // A transport-level failure (network/HTTP/status error) surfaces verbatim, not swallowed.
            struct Failing;
            impl Transport for Failing {
                fn post_json(&self, _: &str, _: &[(String, String)], _: &str) -> Result<String, String> {
                    Err("embeddings API error (503): upstream down".into())
                }
            }
            let e = RemoteEmbedder::new(
                ProviderConfig {
                    endpoint: "https://example.test/v1/embeddings".into(),
                    model: "m".into(),
                    api_key: None,
                    dim: 2,
                },
                Failing,
            );
            let err = e.embed(&["a", "b"]).unwrap_err();
            assert!(err.contains("503") && err.contains("upstream down"), "got: {err}");
        }

        #[test]
        fn remote_embedder_rejects_non_json_response() {
            let err = embed_with_body("this is not json").unwrap_err();
            assert!(err.contains("bad response JSON"), "got: {err}");
        }

        #[test]
        fn remote_embedder_rejects_response_without_a_data_array() {
            let err = embed_with_body(r#"{"object":"list"}"#).unwrap_err();
            assert!(err.contains("no `data` array"), "got: {err}");
        }

        #[test]
        fn remote_embedder_rejects_wrong_embedding_count() {
            // Asked for 2 embeddings, the provider returned 1 — a count mismatch, not silently padded.
            let err = embed_with_body(r#"{"data":[{"index":0,"embedding":[1.0,0.0]}]}"#).unwrap_err();
            assert!(err.contains("asked for 2 embeddings, got 1"), "got: {err}");
        }

        #[test]
        fn remote_embedder_rejects_wrong_dimension() {
            // A vector whose length != cfg.dim (here a 3-d vector for a dim-2 store) is rejected, never
            // truncated/padded to fit the store.
            let err = embed_with_body(
                r#"{"data":[{"index":0,"embedding":[1.0,0.0,9.0]},{"index":1,"embedding":[0.0,1.0,9.0]}]}"#,
            )
            .unwrap_err();
            assert!(err.contains("dim 3 (expected 2)"), "got: {err}");
        }

        #[test]
        fn remote_embedder_rejects_non_finite_components() {
            // A NaN component (here from a JSON null, which `as_f64` turns into NaN) is a non-finite
            // value the store would reject — caught at the embedder boundary with the dim/finite check.
            let err = embed_with_body(
                r#"{"data":[{"index":0,"embedding":[1.0,null]},{"index":1,"embedding":[0.0,1.0]}]}"#,
            )
            .unwrap_err();
            assert!(err.contains("non-finite"), "got: {err}");
        }

        #[test]
        fn remote_embedder_rejects_an_out_of_range_index() {
            // An `index` past the requested count would write out of bounds; it must be a clean error.
            let err = embed_with_body(
                r#"{"data":[{"index":0,"embedding":[1.0,0.0]},{"index":7,"embedding":[0.0,1.0]}]}"#,
            )
            .unwrap_err();
            assert!(err.contains("index 7 out of range"), "got: {err}");
        }

        #[test]
        fn remote_embedder_rejects_a_missing_index() {
            // Two items but both claim index 0 — index 1 is never filled. The "one vector per input"
            // contract is violated, so the assemble step reports the missing index rather than
            // returning a vector with a hole (which a `vec![None; n]` of the wrong shape would).
            let err = embed_with_body(
                r#"{"data":[{"index":0,"embedding":[1.0,0.0]},{"index":0,"embedding":[0.5,0.5]}]}"#,
            )
            .unwrap_err();
            // The duplicate index 0 trips the duplicate guard before the missing-index assemble — both
            // are "this response is malformed" errors; assert it is one of those two specific messages.
            assert!(
                err.contains("duplicate embedding index 0") || err.contains("missing embedding index 1"),
                "got: {err}"
            );
        }

        #[test]
        fn remote_embedder_rejects_an_item_without_index_or_embedding() {
            // A data item missing its `index` field, and (separately) one missing `embedding`.
            let no_index = embed_with_body(
                r#"{"data":[{"embedding":[1.0,0.0]},{"index":1,"embedding":[0.0,1.0]}]}"#,
            )
            .unwrap_err();
            assert!(no_index.contains("without `index`"), "got: {no_index}");
            let no_emb = embed_with_body(
                r#"{"data":[{"index":0},{"index":1,"embedding":[0.0,1.0]}]}"#,
            )
            .unwrap_err();
            assert!(no_emb.contains("without `embedding`"), "got: {no_emb}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_embedder_is_deterministic_and_normalized() {
        let e = HashEmbedder::new(32);
        let a = e.embed(&["Usain Bolt"]).unwrap();
        let b = e.embed(&["Usain Bolt"]).unwrap();
        assert_eq!(a, b);
        let norm: f32 = a[0].iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5, "norm = {norm}");
    }

    #[test]
    fn hash_embedder_correlates_lexically_similar_texts() {
        let e = HashEmbedder::new(64);
        let v = e.embed(&["Usain Bolt", "Usain Bolt Jr", "Pierre de Coubertin"]).unwrap();
        let cos = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
        let near = cos(&v[0], &v[1]);
        let far = cos(&v[0], &v[2]);
        assert!(near > far, "near {near} should beat far {far}");
        assert!(near > 0.4, "shared n-grams should correlate strongly, got {near}");
    }

    #[test]
    fn hash_embedder_empty_text_is_zero_vector() {
        let e = HashEmbedder::new(16);
        let v = e.embed(&[""]).unwrap();
        assert!(v[0].iter().all(|&x| x == 0.0));
    }
}
