//! Generation-pinning integration tests, both feature states (compile the suite
//! twice: `cargo test -p sparq-server` and `… --features time-travel`):
//!
//! * feature ON (`with_feature`): `?generation=N` end-to-end through the real
//!   server — a pinned query returns OLD data after subsequent updates; the
//!   `Sparq-Generation` header exposes the produced-against generation on query
//!   responses and the read-your-writes token on update 204s; aged-out → 410;
//!   never-published → 400; unparsable → 400; pinning an update → 400; and the
//!   retention-bound composition (configured time-travel window SMALLER than the
//!   ring's K — the K floor wins, documented in sparq-serve). The `time-travel`
//!   feature only WIDENS the retention window; these tests are unchanged by
//!   sq-ci2d6 (byte-identical header/pin behaviour when enabled).
//! * feature OFF (`default_build`, sq-ci2d6): the SAME `?generation=N` pin +
//!   `Sparq-Generation` header, bounded to the ring's EXISTING concurrency-
//!   retention window (the last K generations older than current, always kept —
//!   `sparq_serve::ring::DEFAULT_RETAIN` = 4). A pin within the window is a
//!   snapshot-consistent read (the load-bearing property for multi-request
//!   `LIMIT`/`OFFSET` pagination across an interleaved write); a pin aged out of
//!   the window → 410 Gone (never a silent fallback); never-published /
//!   unparsable / pinning an update → 400.
//!
//! Aging out is driven by the deterministic COUNT bound (publish past the
//! window), never by wall-clock `max_age` — the clock-sensitive age semantics
//! are unit-tested in sparq-serve with an injected clock (the recorded
//! determinism concern). 🤖 SPARQ agent.
//!
//! (sq-1b390) Gate the whole suite on the `server` feature. It spins the real axum
//! server and uses the `server`-gated `sparq_server::router` / `AppState` API, so under
//! `--no-default-features --all-targets` (the pure-serialiser-library build) this file must
//! compile OUT — otherwise `clippy --no-default-features --all-targets` breaks on the
//! unresolved axum / serde_json / router imports. 🤖 SPARQ agent.
#![cfg(feature = "server")]

use sparq_core::Graph;
use sparq_server::{router, AppState, ServerConfig};
use tokio::net::TcpListener;

/// Boots the real server over `graph` on an ephemeral port; returns its base URL.
async fn spawn_with(graph: Graph, config: ServerConfig) -> String {
    let app = router(AppState::with_config(graph, config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

async fn spawn_empty(config: ServerConfig) -> String {
    spawn_with(Graph::load_str("", "turtle").unwrap(), config).await
}

async fn post_update(cl: &reqwest::Client, base: &str, update: &str) -> reqwest::Response {
    cl.post(format!("{base}/sparql"))
        .header("content-type", "application/sparql-update")
        .body(update.to_string())
        .send()
        .await
        .unwrap()
}

const PROBE: &str = "SELECT ?s WHERE { ?s <http://ex/seen> ?o }";

/// GETs `query` (optionally pinned to `generation`), returning the response.
async fn query_response(cl: &reqwest::Client, base: &str, query: &str, generation: Option<&str>) -> reqwest::Response {
    let mut params = vec![("query", query)];
    if let Some(g) = generation {
        params.push(("generation", g));
    }
    cl.get(format!("{base}/sparql"))
        .header("accept", "application/sparql-results+json")
        .query(&params)
        .send()
        .await
        .unwrap()
}

/// Solution count of a 200 JSON response.
async fn rows(resp: reqwest::Response) -> usize {
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    body["results"]["bindings"].as_array().unwrap().len()
}

fn generation_header(resp: &reqwest::Response) -> Option<u64> {
    resp.headers()
        .get("sparq-generation")
        .map(|v| v.to_str().unwrap().parse().unwrap())
}

#[cfg(feature = "time-travel")]
mod with_feature {
    use super::*;

    fn insert(i: usize) -> String {
        format!("INSERT DATA {{ <http://ex/u{i}> <http://ex/seen> <http://ex/yes> }}")
    }

    /// The headline property: a query pinned to a captured generation returns the
    /// OLD data after subsequent updates — end to end through the real server —
    /// while unpinned queries see the new state. Headers carry the tokens: each
    /// update 204 exposes the generation containing it (read-your-writes), each
    /// query response the generation it was produced against.
    #[tokio::test]
    async fn pinned_query_returns_old_data_after_updates() {
        let base = spawn_empty(ServerConfig::default()).await;
        let cl = reqwest::Client::new();

        // Generation 0: empty. The response header exposes the current generation.
        let r0 = query_response(&cl, &base, PROBE, None).await;
        assert_eq!(generation_header(&r0), Some(0), "query response exposes its generation");
        assert_eq!(rows(r0).await, 0);

        // Three sequential updates → generations 1, 2, 3; each 204 carries its token.
        for i in 1..=3usize {
            let resp = post_update(&cl, &base, &insert(i)).await;
            assert_eq!(resp.status(), 204);
            assert_eq!(
                generation_header(&resp),
                Some(i as u64),
                "update ack exposes the generation containing the update"
            );
        }

        // Unpinned: current state (3 rows), header says generation 3.
        let now = query_response(&cl, &base, PROBE, None).await;
        assert_eq!(generation_header(&now), Some(3));
        assert_eq!(rows(now).await, 3);

        // Pinned to each retained generation: exactly the state as of that point.
        for (g, expect) in [(0u64, 0usize), (1, 1), (2, 2), (3, 3)] {
            let resp = query_response(&cl, &base, PROBE, Some(&g.to_string())).await;
            assert_eq!(generation_header(&resp), Some(g), "pinned response confirms its pin");
            assert_eq!(rows(resp).await, expect, "generation {g} must serve its own state");
        }
    }

    /// The pin also flows through the url-encoded POST body (same precedence
    /// contract as `explain`: body wins).
    #[tokio::test]
    async fn generation_in_urlencoded_post_body_pins() {
        let base = spawn_empty(ServerConfig::default()).await;
        let cl = reqwest::Client::new();
        assert_eq!(post_update(&cl, &base, &insert(1)).await.status(), 204);

        let resp = cl
            .post(format!("{base}/sparql"))
            .header("content-type", "application/x-www-form-urlencoded")
            .header("accept", "application/sparql-results+json")
            .body(format!("query={}&generation=0", urlencoded(PROBE)))
            .send()
            .await
            .unwrap();
        assert_eq!(generation_header(&resp), Some(0));
        assert_eq!(rows(resp).await, 0, "POST-form pin must serve generation 0's empty state");
    }

    fn urlencoded(s: &str) -> String {
        s.bytes()
            .map(|b| match b {
                b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' => (b as char).to_string(),
                b' ' => "+".into(),
                _ => format!("%{b:02X}"),
            })
            .collect()
    }

    /// Aged-out generations are a 410 Gone (not a silent fallback) — and the
    /// retention bounds compose as documented: configure a time-travel window
    /// (2) SMALLER than the ring's concurrency bound K (4) and the K floor wins,
    /// so exactly the K newest older-than-current generations are servable.
    #[tokio::test]
    async fn aged_out_generation_is_410_and_k_floor_wins() {
        let config = ServerConfig { time_travel_generations: 2, ..ServerConfig::default() };
        let base = spawn_empty(config).await;
        let cl = reqwest::Client::new();
        for i in 1..=6usize {
            assert_eq!(post_update(&cl, &base, &insert(i)).await.status(), 204);
        }
        // Current = 6; retained older = max(K = 4, configured 2) = 4 → gens 2..=6.
        let ok = query_response(&cl, &base, PROBE, Some("2")).await;
        assert_eq!(rows(ok).await, 2, "oldest retained generation still serves its state");

        let gone = query_response(&cl, &base, PROBE, Some("1")).await;
        assert_eq!(gone.status(), 410, "an aged-out generation is Gone, never substituted");
        let body: serde_json::Value = gone.json().await.unwrap();
        let msg = body["error"].as_str().unwrap();
        assert!(msg.contains("aged out"), "error explains the 410: {msg}");
        assert!(msg.contains("oldest retained: 2"), "error names the oldest retained: {msg}");
    }

    /// Token misuse is a 400 with a precise message: a generation that was never
    /// published, an unparsable number, and pinning an update.
    #[tokio::test]
    async fn bad_generation_tokens_are_400() {
        let base = spawn_empty(ServerConfig::default()).await;
        let cl = reqwest::Client::new();

        let future = query_response(&cl, &base, PROBE, Some("999")).await;
        assert_eq!(future.status(), 400);
        let body: serde_json::Value = future.json().await.unwrap();
        assert!(body["error"].as_str().unwrap().contains("not been published"));

        let junk = query_response(&cl, &base, PROBE, Some("yesterday")).await;
        assert_eq!(junk.status(), 400);
        let body: serde_json::Value = junk.json().await.unwrap();
        assert!(body["error"].as_str().unwrap().contains("invalid 'generation' parameter"));

        // Updates always apply to the current generation: pinning one is refused,
        // not silently ignored.
        let resp = cl
            .post(format!("{base}/sparql?generation=0"))
            .header("content-type", "application/sparql-update")
            .body(insert(1))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 400);
        let body: serde_json::Value = resp.json().await.unwrap();
        assert!(body["error"].as_str().unwrap().contains("updates always apply to the current generation"));
        // …and nothing was applied.
        assert_eq!(rows(query_response(&cl, &base, PROBE, None).await).await, 0);
    }
}

#[cfg(not(feature = "time-travel"))]
mod default_build {
    //! (sq-ci2d6) The DEFAULT build (no `time-travel` feature) exposes the SAME
    //! `Sparq-Generation` header + `?generation=N` pin, bounded to the generation ring's EXISTING
    //! concurrency-retention window (`sparq_serve::ring::DEFAULT_RETAIN` = 4 generations older than
    //! current, always kept). Unblocks solid-server-rs snapshot-consistent multi-request pagination.
    use super::*;
    use std::collections::BTreeSet;

    fn insert(i: usize) -> String {
        format!("INSERT DATA {{ <http://ex/u{i}> <http://ex/seen> <http://ex/yes> }}")
    }

    /// ORDER BY ?s makes LIMIT/OFFSET pages deterministic over an immutable snapshot — the
    /// ordering guarantee multi-request pagination relies on (single-digit IRIs sort lexically).
    const ORDERED: &str = "SELECT ?s WHERE { ?s <http://ex/seen> ?o } ORDER BY ?s";

    /// The `?s` bindings of a 200 JSON response (one page's rows, in order).
    async fn page_subjects(resp: reqwest::Response) -> Vec<String> {
        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = resp.json().await.unwrap();
        body["results"]["bindings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["s"]["value"].as_str().unwrap().to_string())
            .collect()
    }

    /// GETs one `LIMIT limit OFFSET offset` page of `ORDERED`, pinned to generation `gen`.
    async fn paged(
        cl: &reqwest::Client,
        base: &str,
        gen: u64,
        limit: usize,
        offset: usize,
    ) -> reqwest::Response {
        let q = format!("{ORDERED} LIMIT {limit} OFFSET {offset}");
        let g = gen.to_string();
        cl.get(format!("{base}/sparql"))
            .header("accept", "application/sparql-results+json")
            .query(&[("query", q.as_str()), ("generation", g.as_str())])
            .send()
            .await
            .unwrap()
    }

    /// The `error` string of a non-200 JSON response.
    async fn error_of(resp: reqwest::Response) -> String {
        let body: serde_json::Value = resp.json().await.unwrap();
        body["error"].as_str().unwrap().to_string()
    }

    /// The headline default-build property (the PSS #1572 acceptance test): pin one generation,
    /// page it with `LIMIT`/`OFFSET` across an INTERLEAVED write, and the pages union to exactly
    /// the single-shot result AT that generation — no overlap, no skip, the interleaved row
    /// invisible. This is snapshot-consistent pagination in the DEFAULT build (no feature).
    #[tokio::test]
    async fn pinned_pagination_is_snapshot_consistent_across_an_interleaved_write() {
        let base = spawn_empty(ServerConfig::default()).await;
        let cl = reqwest::Client::new();

        // Three rows → generation 3. The header hands back the pin token in the DEFAULT build.
        for i in 1..=3usize {
            assert_eq!(post_update(&cl, &base, &insert(i)).await.status(), 204);
        }
        let probe = query_response(&cl, &base, PROBE, None).await;
        let pin = generation_header(&probe).expect("default build stamps Sparq-Generation");
        assert_eq!(pin, 3, "header exposes the current generation in the default build");
        assert_eq!(rows(probe).await, 3);

        // Page 1 (LIMIT 2 OFFSET 0), pinned.
        let p1 = paged(&cl, &base, pin, 2, 0).await;
        assert_eq!(generation_header(&p1), Some(pin), "page confirms its pin");
        let page1 = page_subjects(p1).await;

        // Interleave a write BETWEEN the two page reads → generation 4.
        assert_eq!(post_update(&cl, &base, &insert(4)).await.status(), 204);

        // Page 2 (LIMIT 2 OFFSET 2), STILL pinned to the same generation.
        let p2 = paged(&cl, &base, pin, 2, 2).await;
        assert_eq!(generation_header(&p2), Some(pin));
        let page2 = page_subjects(p2).await;

        // Union = single-shot at the pin (3 rows): no overlap, no skip, u4 invisible.
        let mut union: Vec<String> = page1.clone();
        union.extend(page2.clone());
        let distinct: BTreeSet<&String> = union.iter().collect();
        assert_eq!(distinct.len(), union.len(), "no row appears on both pages: {page1:?} + {page2:?}");
        assert!(
            union.iter().all(|s| s != "http://ex/u4"),
            "the interleaved write is invisible to the pinned snapshot: {union:?}"
        );
        let single = page_subjects(paged(&cl, &base, pin, 100, 0).await).await;
        assert_eq!(union, single, "paged union == single-shot at the pinned generation");

        // The pin ISOLATED the pages, it did not freeze the store: unpinned now sees all 4.
        assert_eq!(rows(query_response(&cl, &base, PROBE, None).await).await, 4);
    }

    /// A pin that has aged out of the concurrency-retention window is `410 Gone` — never a silent
    /// fallback to a different generation (the honesty boundary) — and the default-build 410 cites
    /// the concurrency window, NOT the time-travel flags (retention is NOT extended here).
    #[tokio::test]
    async fn pin_beyond_concurrency_window_is_410_gone() {
        let base = spawn_empty(ServerConfig::default()).await;
        let cl = reqwest::Client::new();
        // Publish past the K = 4 concurrency floor: 6 updates → current 6, oldest retained 2.
        for i in 1..=6usize {
            assert_eq!(post_update(&cl, &base, &insert(i)).await.status(), 204);
        }
        // Generation 2 is the oldest still retained → still serves its own snapshot (2 rows).
        let ok = query_response(&cl, &base, PROBE, Some("2")).await;
        assert_eq!(rows(ok).await, 2, "oldest retained generation still serves its snapshot");

        // Generation 1 aged out of the window → 410 Gone, never substituted.
        let gone = query_response(&cl, &base, PROBE, Some("1")).await;
        assert_eq!(gone.status(), 410, "a pin past the concurrency window is Gone, not a silent fallback");
        let msg = error_of(gone).await;
        assert!(msg.contains("aged out"), "410 explains itself: {msg}");
        assert!(msg.contains("oldest retained: 2"), "410 names the oldest retained: {msg}");
        assert!(
            msg.contains("concurrency-retention window"),
            "default-build 410 cites the concurrency window (not the time-travel flags): {msg}"
        );
    }

    /// Token misuse is a precise `400` (no longer a silent ignore): a never-published generation,
    /// an unparsable number, and pinning an UPDATE (an update always applies to current).
    #[tokio::test]
    async fn bad_generation_tokens_are_400_in_the_default_build() {
        let base = spawn_empty(ServerConfig::default()).await;
        let cl = reqwest::Client::new();

        let future = query_response(&cl, &base, PROBE, Some("999")).await;
        assert_eq!(future.status(), 400);
        assert!(error_of(future).await.contains("not been published"));

        let junk = query_response(&cl, &base, PROBE, Some("yesterday")).await;
        assert_eq!(junk.status(), 400);
        assert!(error_of(junk).await.contains("invalid 'generation' parameter"));

        // Pinning an update is an honest refusal, not a silent ignore.
        let resp = cl
            .post(format!("{base}/sparql?generation=0"))
            .header("content-type", "application/sparql-update")
            .body(insert(1))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 400);
        assert!(error_of(resp).await.contains("updates always apply to the current generation"));
        // …and nothing was applied.
        assert_eq!(rows(query_response(&cl, &base, PROBE, None).await).await, 0);
    }
}
