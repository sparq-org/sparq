//! `sparq-server` binary: load an RDF file into the engine and expose it over HTTP per the
//! W3C SPARQL 1.1 Protocol + Graph Store HTTP Protocol (read side).
//!
//! Usage:
//!   sparq-server --health-probe [--health-probe-addr 127.0.0.1:3030]
//!   sparq-server [--addr 127.0.0.1:3030] [--allow-remote] [--format turtle]
//!                [--persist DIR]
//!                [--auth-token TOKEN] [--auth-token-read]
//!                [--query-timeout SECS] [--update-where-timeout SECS] [--max-body-bytes N] [--max-concurrent N]
//!                [--max-results N] [--max-query-rows N] [--max-decompress-ratio N]
//!                [--max-subscriptions N] [--max-subscriptions-per-conn N]
//!                [--complete]
//!                [--service-allow HOST|*.SUFFIX]... [--service-allow-file PATH]
//!                [--cors-allow-origin ORIGIN]... [--cors-allow-origin-file PATH]
//!                [--tls-cert FILE --tls-key FILE]
//!                [--http3 [--http3-addr HOST:PORT]]
//!                [--verbose] [DATA_FILE]
//!
//! DURABLE PERSISTENCE (`--persist DIR`, env `SPARQ_PERSIST_DIR`; QLever's `--persist-updates`
//! equivalent — sq-7cxr / gh-44): treat the on-disk index at DIR as the durable, rebuildable
//! source of truth. Every committed SPARQL Update (default graph AND named graphs) is appended
//! to a write-ahead log and fsync'd BEFORE the group-commit ack, so a process RESTART preserves
//! ALL updates with NO rebuild. On startup: if DIR already holds a store it is OPENED (its WAL
//! replayed; the DATA_FILE seed is then IGNORED — the persisted store wins, like QLever's index);
//! if DIR is empty/absent the DATA_FILE seed (or empty graph) is written there and opened. Without
//! `--persist` the server is purely in-memory and updates are LOST on restart (the historical
//! behaviour). See crates/sparq-server/README.md -> "Durable persistence". [OPUS-4.8]
//!
//! SERVICE federation (`service` build feature) is DENY-ALL by default: a `SERVICE <iri>`
//! clause reaches NOTHING unless its host is allowlisted via `--service-allow` (repeatable;
//! exact host or `*.suffix` wildcard), `--service-allow-file` (one entry per line) or the
//! `SPARQ_SERVICE_ALLOW` env var (comma/whitespace-separated). This is an SSRF guard: a
//! `SERVICE` clause turns attacker-controlled query text into an outbound request from this
//! host. See crates/sparq-server/README.md -> "SERVICE federation (egress allowlist)". [OPUS-4.8]
//!
//! CORS (sq-o7o0, ASVS V14.5.3): by default the server emits NO CORS headers — it is a SPARQL
//! DATA API, so a cross-origin browser `fetch` simply cannot read its responses (the safe
//! posture). For a FIRST-PARTY browser app on a different origin, allowlist that origin via
//! `--cors-allow-origin <ORIGIN>` (repeatable; exact `scheme://host[:port]`),
//! `--cors-allow-origin-file PATH` (one origin per line) or `SPARQ_CORS_ALLOW_ORIGIN`
//! (comma/whitespace-separated). Only a listed `Origin` is reflected into
//! `Access-Control-Allow-Origin` (never `*`, never with credentials) and the preflight
//! `OPTIONS` is answered for it; an un-listed origin still gets no CORS header. See
//! crates/sparq-server/README.md -> "Security posture" (CORS). [OPUS-4.8]
//!
//! SECURITY (auth): by default the server has NO authentication on any endpoint (query, the
//! `application/sparql-update` mutation path, the `/subscriptions` WebSocket). [OPUS-4.8]
//! sq-zcby (PSS gh-46): set `--auth-token TOKEN` (env `SPARQ_AUTH_TOKEN`) to require
//! `Authorization: Bearer TOKEN` on every WRITE (a SPARQL Update on `/sparql` — by
//! `application/sparql-update` Content-Type OR a `query`/`update` body that parses as an
//! update; classification keys on whether the request MUTATES, not the route — plus the GSP
//! `PUT`/`POST`/`DELETE` methods); otherwise `401` with `WWW-Authenticate: Bearer`
//! (constant-time compared, scheme-casing tolerant; mirrors QLever's `-a <token>`). Add
//! `--auth-token-read` (env `SPARQ_AUTH_TOKEN_READ=1`) to ALSO gate reads. [OPUS-4.8] sq-cxk5:
//! the subscription read surfaces — the `/subscriptions` WebSocket AND the
//! `/subscriptions/sse` Server-Sent-Events stream — are gated by `--auth-token-read` too. The
//! SSE GET takes the `Authorization: Bearer` header like any GET; the WS upgrade accepts the
//! token from that header OR (for browsers, which cannot set headers on a WS handshake) a
//! `Sec-WebSocket-Protocol: bearer.<token>` subprotocol. With no read token configured both
//! are open (back-compatible).
//!
//! SECURITY (bind): `--addr` defaults to loopback (127.0.0.1:3030), reachable only from this
//! host. A NON-loopback bind (e.g. `0.0.0.0`) is REFUSED unless `--allow-remote` (env
//! `SPARQ_ALLOW_REMOTE=1`) is set OR the whole surface is authenticated (`--auth-token` AND
//! `--auth-token-read`) — a write-token alone still leaves reads open, so it is not
//! sufficient by itself. Even an allowed remote bind logs a warning. Deliver the token over
//! TLS (terminate at a proxy); for per-user authz front it with a reverse proxy / gateway
//! (or sparq-solid). See crates/sparq-server/README.md → "Security posture".
//!
//! With no DATA_FILE the server starts with an empty default graph (still answers queries —
//! they just return no rows). The format defaults to `turtle`; pass `--format ntriples |
//! nquads | trig` to match the file.
//!
//! Hardening flags (T15 + sq-ebii) — defaults in brackets; each flag overrides the matching
//! `SPARQ_*` environment variable (see crates/sparq-server/README.md and the four-limit
//! "Server hardening" section in skills/http-server/SKILL.md):
//!   --query-timeout SECS      per-request query timeout, 0 disables      [30, env SPARQ_QUERY_TIMEOUT]
//!   --update-where-timeout S  [OPUS-4.8 sq-nulp] separate, typically-SHORTER writer-side WHERE deadline
//!                             for SPARQL UPDATE: bounds writer-queue HEAD-OF-LINE blocking from a slow
//!                             update (a slow update releases the single writer within S instead of
//!                             holding it for the full --query-timeout). 0/unset = use --query-timeout
//!                             [unset, env SPARQ_UPDATE_WHERE_TIMEOUT]
//!   --no-adaptive-commit      [OPUS-4.8 sq-p7kk5] DISABLE adaptive group-commit (restore the always-
//!                             windowed writer). Adaptive commit is ON by default: a serial interactive
//!                             client commits in engine-time (µs) instead of paying the fixed group-commit
//!                             window (ms); concurrent load still batches. FIFO/atomicity/durability
//!                             unchanged. [on, env SPARQ_ADAPTIVE_COMMIT=0 to disable]
//!   --max-body-bytes N        maximum request body in bytes              [1048576, env SPARQ_MAX_BODY_BYTES]
//!   --max-concurrent N        maximum in-flight requests (429 beyond)    [32, env SPARQ_MAX_CONCURRENT]
//!   --header-read-timeout S   [OPUS-4.8 sq-2gqr] slow-loris guard: max SECONDS a connection may take
//!                             to send its complete request-header block (closes the connection +
//!                             frees the concurrency slot if exceeded), 0 disables
//!                             [15, env SPARQ_HEADER_READ_TIMEOUT]
//!   --body-read-timeout S     [OPUS-4.8 sq-lodb] slow-BODY guard (complement to --header-read-timeout):
//!                             max SECONDS the server waits for the NEXT request-body chunk (idle deadline
//!                             between body reads, reset after each chunk — an honest large upload is not
//!                             penalised, only a stall). Closes the slow-body DoS a complete-header client
//!                             could otherwise use. 0 disables   [30, env SPARQ_BODY_READ_TIMEOUT]
//!   --max-results N           maximum SELECT rows (413 beyond), 0 off    [unlimited, env SPARQ_MAX_RESULTS]
//!   --max-query-rows N        [OPUS-4.8 sq-ebii] coarse MEMORY CAP: working-set row ceiling on EVERY
//!                             query form (413 beyond), 0 off             [unlimited, env SPARQ_MAX_QUERY_ROWS]
//!   --max-query-bytes N       [OPUS-4.8 sq-s5is] BYTE-ACCOUNTED memory cap: prices row WIDTH and
//!                             computed-literal size on EVERY form (413 beyond), 0 off
//!                                                                        [unlimited, env SPARQ_MAX_QUERY_BYTES]
//!   --max-decompress-ratio N  [OPUS-4.8 sq-ebii] DECOMPRESSION-RATIO cap for a `Content-Encoding: gzip`
//!                             request body (zip-bomb guard; 413), 0 = refuse gzip bodies
//!                                                                        [20, env SPARQ_MAX_DECOMPRESS_RATIO]
//!   --facets                  [GPT-5.6 sq-lsp7k.5.2] (feature `facets`) serve the OPT-IN grouped
//!                             facet-count endpoint `POST /facets`. Read-only. Off by default
//!                                                                        [off, env SPARQ_FACETS=1]
//!   --complete                [GPT-5.6 sq-lsp7k.9.3] (feature `complete`) serve the OPT-IN
//!                             prefix-completion endpoint `GET /complete`. Read-only. Off by default
//!                                                                        [off, env SPARQ_COMPLETE=1]
//!   --brtpf-max-bindings N    [OPUS-4.8 sq-r74h] (feature `brtpf`) DoS cap on the brTPF binding-set
//!                             MAPPING COUNT — one index scan runs per mapping, so cost is super-linear
//!                             in the count, not the bytes (413 beyond), 0 off
//!                                                                        [1024, env SPARQ_BRTPF_MAX_BINDINGS]
//!   --brtpf-max-values-bytes N [OPUS-4.8 sq-r74h] (feature `brtpf`) DoS cap on the raw brTPF `values`
//!                             PAYLOAD BYTES — bounds the GET query-string carrier that --max-body-bytes
//!                             never sees (413 beyond), 0 off            [1048576, env SPARQ_BRTPF_MAX_VALUES_BYTES]
//!   --shacl                   [OPUS-4.8 sq-r868] (feature `shacl`) serve the OPT-IN HTTP SHACL validate
//!                             endpoint `POST /shacl/validate`: POST a shapes graph, the server validates
//!                             its loaded data graph against it. Read-only. Off by default
//!                                                                        [off, env SPARQ_SHACL=1]
//!   --shacl-guard             [GPT-5.6 sq-lsp7k.2.4] reject violating UPDATE/GSP post-states
//!                                                                        [off, env SPARQ_SHACL_GUARD=1]
//!   --shacl-shapes FILE       shapes graph for guard mode [env SPARQ_SHACL_SHAPES]
//!   --n3-patch                [OPUS-4.8 sq-hj4n] (feature `n3-patch`) enable the OPT-IN Solid N3-Patch
//!                             (`text/n3`) dialect on the Graph-Store-Protocol `PATCH` method. The
//!                             always-on `application/sparql-update` PATCH dialect needs no flag.
//!                             Off by default                            [off, env SPARQ_N3_PATCH=1]
//!   --terse                   [OPUS-4.8 sq-vczh2] (feature `terse`) serve the OPT-IN, VERIFIABLE
//!                             LLM-ergonomic transpiler endpoint `POST /terse/transpile`: POST a terse
//!                             query (the `K:<name>` keyword layer over canonical SPARQL), the server
//!                             returns the CANONICAL SPARQL it expands to (it never executes it). Off
//!                             by default                                 [off, env SPARQ_TERSE=1]
//!   --templates               [FABLE-5 sq-lsp7k.10] (feature `templates`) serve the OPT-IN named
//!                             parameterized-template REST surface: `GET /templates` lists,
//!                             `PUT`/`GET`/`DELETE /templates/{name}` manage one definition,
//!                             `POST /templates/{name}` invokes it with typed JSON arguments
//!                             (fail-closed on unknown/missing/mistyped parameters). Template
//!                             writes + UPDATE-template invocations are write-gated through the
//!                             same sequenced-writer path as /sparql updates. Off by default
//!                                                                        [off, env SPARQ_TEMPLATES=1]
//!   --templates-file PATH     [FABLE-5 sq-lsp7k.10] (feature `templates`) durable template store:
//!                             definitions load from PATH at startup (fail-closed) and every
//!                             successful PUT/DELETE rewrites it atomically
//!                                                                        [in-memory, env SPARQ_TEMPLATES_FILE]
//!   --verbose                 per-request logging (TraceLayer)
//!   --log-full-requests       [OPUS-4.8 sq-toze.34] OPT OUT of request-log redaction: log the
//!                             raw request URI (incl. the full `?query=` SPARQL text) verbatim.
//!                             By DEFAULT the --verbose log redacts the URI query string to a
//!                             length+fingerprint placeholder (PII/privacy hygiene). Inert
//!                             without --verbose.                  [env SPARQ_LOG_FULL_REQUESTS=1]
//!
//! Access audit log (opt-in `audit-log` cargo feature — see skills/http-server/SKILL.md
//! "Access audit log"; CDMC CD-2 / ISO 27001 A.8.15 / EU CRA logging):
//!   --audit-log               emit a structured per-query access-audit `tracing` record under
//!                             target `sparq_server::audit` (requester-token fingerprint /
//!                             `anonymous`, op class, NON-reversible query fingerprint — never
//!                             the full query text or token, the #241 posture —, decision,
//!                             status/rows, duration). Route it with RUST_LOG. Off by default.
//!                             [env SPARQ_AUDIT_LOG=1]
//!
//! Subscription limits (T23, the /subscriptions WebSocket — see SUBSCRIPTIONS.md):
//!   --max-subscriptions N           server-wide active subscriptions [256, env SPARQ_MAX_SUBSCRIPTIONS]
//!   --max-subscriptions-per-conn N  active subscriptions per socket  [16, env SPARQ_MAX_SUBSCRIPTIONS_PER_CONN]
//!
//! Time-travel retention (opt-in `time-travel` cargo feature — see the README's
//! "Time-travel queries" section; each retained generation is a FULL graph today):
//!   --time-travel-generations N  queryable generations older than current [16, env SPARQ_TIME_TRAVEL_GENERATIONS]
//!   --time-travel-max-age SECS   age generations out after SECS, 0 off    [off, env SPARQ_TIME_TRAVEL_MAX_AGE]
//!
//! Update path (Wave A wiring — see crates/sparq-server/README.md "Update concurrency
//! model"): sparq-serve's sequenced group-commit writer over the generation ring. The
//! old `--compact-every` flag is gone — every batch commit forks a freshly folded base,
//! so overlays never accumulate across batches and there is nothing left to compact.
#![forbid(unsafe_code)] // [OPUS-4.8] sq-emay: crate has zero `unsafe`

use std::net::SocketAddr;
use std::time::Duration;

#[cfg(any(feature = "http2", feature = "http3"))]
use std::{path::PathBuf, sync::Arc};

use sparq_core::Graph;
// [OPUS-4.8] sq-o4qf: bind_posture / BindPosture gate the non-loopback bind; sq-zcby:
// AuthPosture folds the --auth-token Bearer gate into that bind decision.
use sparq_server::{bind_posture, router, AppState, AuthPosture, BindPosture, ServerConfig};

// Same allocator as the CLI (T1.0a, measured ~1.29x on the parallel join): the system allocator's
// arena locks contend under rayon's per-row allocation, and the server is the long-running,
// concurrent-query process where that matters most.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // [OPUS-4.8] sq-toze.36 (cert gap GX-13): the `--health-probe` subcommand is the
    // Dockerfile HEALTHCHECK's check. The runtime image is distroless (no shell, no curl/wget),
    // so the server probes its OWN loopback /health endpoint and exits 0 (healthy) / non-zero
    // (unhealthy). It must short-circuit BEFORE any env/config/data-load work — it never binds a
    // listener, it only connects to an already-running server. Scanned directly off argv (not the
    // main flag loop below) so the probe path stays self-contained.
    if let Some(probe) = parse_health_probe(std::env::args().skip(1))? {
        return match sparq_server::health_probe::run_probe(&probe).await {
            Ok(()) => Ok(()),
            Err(e) => {
                eprintln!("health probe failed: {e}");
                // Non-zero exit so the container runtime marks the container unhealthy.
                std::process::exit(1);
            }
        };
    }

    let mut addr = "127.0.0.1:3030".to_string();
    // [GPT-5.6] sq-oprna.3: HTTP/3 is runtime-off even in a feature-on binary. Its UDP
    // address defaults to the TCP address (same numeric port, different transport).
    #[cfg(feature = "http3")]
    let mut http3 = false;
    #[cfg(feature = "http3")]
    let mut http3_addr: Option<String> = None;
    #[cfg(any(feature = "http2", feature = "http3"))]
    let mut tls_cert: Option<PathBuf> = None;
    #[cfg(any(feature = "http2", feature = "http3"))]
    let mut tls_key: Option<PathBuf> = None;
    let mut format = "turtle".to_string();
    let mut data_file: Option<String> = None;
    // Env first, flags override. [OPUS-4.8] sq-4w18: a malformed SPARQ_SERVICE_ALLOW
    // surfaces here as a clean config error (the `?` turns the String into the boxed
    // error main returns -> a one-line message to stderr + non-zero exit), not a panic.
    let mut config = ServerConfig::from_env()?;
    #[cfg(feature = "shacl")]
    let mut shacl_shapes_path = std::env::var_os("SPARQ_SHACL_SHAPES").map(std::path::PathBuf::from);
    // [OPUS-4.8] sq-4w18: collect SERVICE egress allowlist entries from the CLI; they
    // are UNIONed with the SPARQ_SERVICE_ALLOW env baseline (already in `config`) + an
    // optional --service-allow-file, after the arg loop. An allowlist is additive, so
    // the CLI only ever widens what env granted (never silently narrows it).
    let mut service_allow_cli: Vec<String> = Vec::new();
    let mut service_allow_file: Option<String> = None;
    // [OPUS-4.8] sq-bu1a: track whether --restore-delta was given on the CLI so the FIRST flag
    // OVERRIDES any SPARQ_RESTORE_DELTA env list (matching --restore's flag-overrides-env contract)
    // rather than appending to it.
    #[cfg(feature = "backup")]
    let mut restore_delta_from_cli = false;
    // [OPUS-4.8] sq-o7o0: collect first-party CORS origin allowlist entries from the CLI;
    // UNIONed with the SPARQ_CORS_ALLOW_ORIGIN env baseline + an optional
    // --cors-allow-origin-file, after the arg loop (additive — CLI only widens).
    let mut cors_allow_cli: Vec<String> = Vec::new();
    let mut cors_allow_file: Option<String> = None;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--addr" => addr = args.next().ok_or("--addr requires a value")?,
            #[cfg(feature = "http3")]
            "--http3" => http3 = true,
            #[cfg(feature = "http3")]
            "--http3-addr" => {
                http3_addr = Some(
                    args.next()
                        .ok_or("--http3-addr requires a HOST:PORT value")?,
                );
            }
            #[cfg(any(feature = "http2", feature = "http3"))]
            "--tls-cert" => {
                tls_cert = Some(PathBuf::from(
                    args.next().ok_or("--tls-cert requires a file path")?,
                ));
            }
            #[cfg(any(feature = "http2", feature = "http3"))]
            "--tls-key" => {
                tls_key = Some(PathBuf::from(
                    args.next().ok_or("--tls-key requires a file path")?,
                ));
            }
            "--format" => format = args.next().ok_or("--format requires a value")?,
            "--query-timeout" => {
                let secs: u64 = parse_flag(&mut args, "--query-timeout")?;
                config.query_timeout = (secs > 0).then(|| Duration::from_secs(secs));
            }
            // [OPUS-4.8] sq-nulp: separate, typically-shorter writer-side WHERE deadline that
            // bounds writer-queue head-of-line blocking from a slow UPDATE; 0 disables it (the
            // update WHERE budget is then the plain --query-timeout, exactly as before).
            "--update-where-timeout" => {
                let secs: u64 = parse_flag(&mut args, "--update-where-timeout")?;
                config.update_where_timeout = (secs > 0).then(|| Duration::from_secs(secs));
            }
            // [OPUS-4.8] sq-p7kk5: restore the always-windowed writer (disable adaptive
            // group-commit). Adaptive commit is ON by default — a serial interactive client
            // then commits in engine-time instead of paying the fixed group-commit window.
            "--no-adaptive-commit" => config.adaptive_commit = false,
            "--max-body-bytes" => {
                config.max_body_bytes = parse_flag(&mut args, "--max-body-bytes")?
            }
            "--max-concurrent" => {
                let n: usize = parse_flag(&mut args, "--max-concurrent")?;
                config.max_concurrent = n.max(1);
            }
            // [OPUS-4.8] sq-2gqr: slow-loris connection header-read deadline (seconds); 0 disables
            // it (unbounded header read — the pre-fix behaviour). Default 15s. See
            // ServerConfig::header_read_timeout.
            "--header-read-timeout" => {
                let secs: u64 = parse_flag(&mut args, "--header-read-timeout")?;
                config.header_read_timeout = (secs > 0).then(|| Duration::from_secs(secs));
            }
            // [OPUS-4.8] sq-lodb: slow-body request-body read/idle deadline (seconds); 0 disables
            // it (unbounded body read — a slow body can hold the slot forever). Default 30s. The
            // complement to --header-read-timeout. See ServerConfig::body_read_timeout.
            "--body-read-timeout" => {
                let secs: u64 = parse_flag(&mut args, "--body-read-timeout")?;
                config.body_read_timeout = (secs > 0).then(|| Duration::from_secs(secs));
            }
            "--max-results" => {
                let n: usize = parse_flag(&mut args, "--max-results")?;
                config.max_results = (n > 0).then_some(n);
            }
            // [OPUS-4.8] sq-ebii: memory cap (coarse working-set row ceiling, all forms);
            // 0 disables it.
            "--max-query-rows" => {
                let n: usize = parse_flag(&mut args, "--max-query-rows")?;
                config.max_query_rows = (n > 0).then_some(n);
            }
            // [OPUS-4.8] sq-s5is: byte-accounted memory cap (prices row width + computed
            // literals, all forms); 0 disables it.
            "--max-query-bytes" => {
                let n: usize = parse_flag(&mut args, "--max-query-bytes")?;
                config.max_query_bytes = (n > 0).then_some(n);
            }
            // [OPUS-4.8] sq-ebii: decompression-ratio cap (zip-bomb guard); 0 disables
            // decompression of Content-Encoding: gzip request bodies (they are then refused).
            "--max-decompress-ratio" => {
                config.max_decompress_ratio = parse_flag(&mut args, "--max-decompress-ratio")?;
            }
            "--max-subscriptions" => {
                config.max_subscriptions = parse_flag(&mut args, "--max-subscriptions")?;
            }
            "--max-subscriptions-per-conn" => {
                config.max_subscriptions_per_conn =
                    parse_flag(&mut args, "--max-subscriptions-per-conn")?;
            }
            #[cfg(feature = "time-travel")]
            "--time-travel-generations" => {
                config.time_travel_generations =
                    parse_flag(&mut args, "--time-travel-generations")?;
            }
            #[cfg(feature = "time-travel")]
            "--time-travel-max-age" => {
                let secs: u64 = parse_flag(&mut args, "--time-travel-max-age")?;
                config.time_travel_max_age = (secs > 0).then(|| Duration::from_secs(secs));
            }
            "--verbose" => config.verbose = true,
            // [OPUS-4.8] sq-toze.34 (epic sq-toze): OPT OUT of request-log redaction. By default
            // the --verbose request log redacts the URI query string (where GET /sparql?query=…
            // carries the full SPARQL text, possibly PII) to a length+fingerprint placeholder.
            // This flag logs the raw URI verbatim — the deliberate debug escape hatch. Overrides
            // SPARQ_LOG_FULL_REQUESTS. Inert without --verbose.
            "--log-full-requests" => config.redact_logs = false,
            // [OPUS-4.8] sq-0bxp: opt in to the per-query access audit log (CDMC CD-2). Emits a
            // structured `tracing` record per request under target `sparq_server::audit` (route
            // it with RUST_LOG). Only present with the `audit-log` cargo feature.
            #[cfg(feature = "audit-log")]
            "--audit-log" => config.audit_log = true,
            // [OPUS-4.8] sq-gos8 (epic sq-toze): install the RICHER STRUCTURED access-audit sink
            // — `--access-audit <file|stderr>`. Records every enforced access decision as a
            // typed JSON-Lines record (actor / action / resource / decision + policy-basis /
            // timestamp / request fingerprint). The literal `stderr` writes to stderr; any other
            // value is a file path. Overrides SPARQ_ACCESS_AUDIT. Only with the `access-audit`
            // cargo feature. PRIVACY: logs identities + resources by design; query content stays
            // fingerprinted (never raw). See skills/http-server/SKILL.md.
            #[cfg(feature = "access-audit")]
            "--access-audit" => {
                let target = args
                    .next()
                    .ok_or("--access-audit requires a target ('stderr' or a file path)")?;
                if target.is_empty() {
                    return Err("--access-audit target must not be empty".into());
                }
                config.access_audit = Some(sparq_server::access_audit::SinkTarget::parse(&target));
            }
            // [OPUS-4.8] sq-ljfz (sq-gos8 follow-up): trust a fronting auth layer's forwarded
            // WebID header so the access-audit actor is the real authenticated subject
            // (Actor::WebId) rather than the coarse Bearer-token fingerprint. NAME the header a
            // trusted front (sparq-solid / a Solid-WAC proxy / gateway) sets to the user's
            // WebID. SECURITY: only set this when the server runs BEHIND that trusted front —
            // a forwarded header is client-spoofable otherwise. Overrides SPARQ_AUDIT_WEBID_HEADER.
            #[cfg(feature = "access-audit")]
            "--audit-webid-header" => {
                let name = args
                    .next()
                    .ok_or("--audit-webid-header requires a header name")?;
                let name = name.trim().to_string();
                if name.is_empty() {
                    return Err("--audit-webid-header must not be empty".into());
                }
                config.audit_webid_header = Some(name);
            }
            // [OPUS-4.8] sq-7cxr (PSS gh-44): durable persistence directory (QLever
            // --persist-updates). Updates are WAL-durable to DIR before ack; a restart on the
            // same DIR restores all of them with no rebuild. Overrides SPARQ_PERSIST_DIR.
            "--persist" => {
                let dir = args.next().ok_or("--persist requires a directory path")?;
                if dir.is_empty() {
                    return Err("--persist must not be empty".into());
                }
                config.persist_dir = Some(std::path::PathBuf::from(dir));
            }
            // [OPUS-4.8] sq-o5bi: RESTORE-ON-START from a backup artifact (the bootstrap
            // primitive for horizontal-scaling stage-2 + the base of PITR). Off by default
            // (env SPARQ_RESTORE=<file>). Fail-closed: a corrupt artifact aborts startup.
            #[cfg(feature = "backup")]
            "--restore" => {
                let file = args.next().ok_or("--restore requires an artifact file path")?;
                if file.is_empty() {
                    return Err("--restore must not be empty".into());
                }
                config.restore = Some(std::path::PathBuf::from(file));
            }
            // [OPUS-4.8] sq-bu1a: POINT-IN-TIME-RECOVERY — replay an incremental DELTA artifact
            // forward onto the --restore base. Repeatable (oldest first); each must be a
            // sparq delta artifact. Requires --restore. Fail-closed: a corrupt / out-of-order /
            // gapped delta aborts startup. Off by default (env SPARQ_RESTORE_DELTA=<files>).
            #[cfg(feature = "backup")]
            "--restore-delta" => {
                let file = args.next().ok_or("--restore-delta requires an artifact file path")?;
                if file.is_empty() {
                    return Err("--restore-delta must not be empty".into());
                }
                // First CLI flag clears any env-provided list (flag overrides env).
                if !restore_delta_from_cli {
                    config.restore_delta.clear();
                    restore_delta_from_cli = true;
                }
                config.restore_delta.push(std::path::PathBuf::from(file));
            }
            // [OPUS-4.8] sq-ft7u: RESTORE-INTO-DURABLE on start. Combined with --persist DIR +
            // --restore FILE, write the artifact THROUGH to the durable dir crash-safely, so the
            // restored dataset survives a restart (instead of refusing the --restore/--persist
            // combination). Off by default (env SPARQ_RESTORE_PERSIST=1). See README.
            #[cfg(feature = "backup")]
            "--restore-persist" => config.restore_persist = true,
            // [OPUS-4.8] sq-o4qf: opt in to binding a non-loopback address. Without this (and
            // without SPARQ_ALLOW_REMOTE), a non-loopback --addr is refused unless the whole
            // surface is authenticated (--auth-token AND --auth-token-read) — see bind_posture.
            "--allow-remote" => config.allow_remote = true,
            // [OPUS-4.8] sq-zcby (PSS gh-46): require a Bearer token on the WRITE surface.
            // An empty token is rejected (an empty shared secret is a footgun, never valid).
            "--auth-token" => {
                let tok = args.next().ok_or("--auth-token requires a token value")?;
                if tok.is_empty() {
                    return Err("--auth-token must not be empty".into());
                }
                config.auth_token = Some(tok);
            }
            // [OPUS-4.8] sq-zcby: ALSO gate reads with the same token (QLever-style; only
            // meaningful alongside --auth-token).
            "--auth-token-read" => config.auth_token_read = true,
            // [OPUS-4.8] sq-d3d8 (epic sq-3183): OPT-IN federation discovery descriptors —
            // serve a W3C VoID at /.well-known/void and a SPARQL Service Description for a
            // GET /sparql with no query. Off by default (env SPARQ_FEDERATION_DESCRIPTORS=1).
            #[cfg(feature = "federation-descriptors")]
            "--federation-descriptors" => config.federation_descriptors = true,
            // [GPT-5.6] sq-lsp7k.5.2: OPT-IN grouped facet counts over a pinned snapshot.
            // Read-only. Off by default (env SPARQ_FACETS=1).
            #[cfg(feature = "facets")]
            "--facets" => config.facets = true,
            // [GPT-5.6] sq-lsp7k.9.3: OPT-IN prefix completion backed by a generation-keyed
            // AppState cache. Read-only. Off by default (env SPARQ_COMPLETE=1).
            #[cfg(feature = "complete")]
            "--complete" => config.complete = true,
            // [OPUS-4.8] sq-bzh1 (epic sq-3183): OPT-IN Triple Pattern Fragments / LDF source
            // endpoint — serve a paged RDF fragment of a triple pattern at GET /tpf with Hydra
            // controls. Read-only. Off by default (env SPARQ_TPF=1).
            #[cfg(feature = "tpf")]
            "--tpf" => config.tpf = true,
            // [OPUS-4.8] sq-r74h (follow-up to sq-dxhb): brTPF binding-set DoS caps. A brTPF
            // request runs ONE index scan per attached solution mapping, so the cost is
            // super-linear in the mapping COUNT — which `--max-body-bytes` bounds only loosely,
            // and not at all for the `values` query-string carrier. These cap the mapping count
            // and the raw `values` payload bytes (413 on breach); 0 disables that cap.
            #[cfg(feature = "brtpf")]
            "--brtpf-max-bindings" => {
                config.brtpf_max_bindings = parse_flag(&mut args, "--brtpf-max-bindings")?;
            }
            #[cfg(feature = "brtpf")]
            "--brtpf-max-values-bytes" => {
                config.brtpf_max_values_bytes = parse_flag(&mut args, "--brtpf-max-values-bytes")?;
            }
            // [OPUS-4.8] sq-r868 (from-pss gh-162 follow-up (c)): OPT-IN HTTP SHACL validate
            // endpoint — POST a shapes graph to /shacl/validate and the server validates its
            // loaded data graph against it. Read-only. Off by default (env SPARQ_SHACL=1).
            #[cfg(feature = "shacl")]
            "--shacl" => config.shacl = true,
            // [GPT-5.6] sq-lsp7k.2.4: post-state SHACL transaction guard + shapes source.
            #[cfg(feature = "shacl")]
            "--shacl-guard" => config.shacl_guard = true,
            #[cfg(feature = "shacl")]
            "--shacl-shapes" => {
                let path = args.next().ok_or("--shacl-shapes requires a file path")?;
                if path.is_empty() { return Err("--shacl-shapes must not be empty".into()); }
                shacl_shapes_path = Some(std::path::PathBuf::from(path));
            }
            // [OPUS-4.8] sq-hj4n (gh-916): OPT-IN Solid N3-Patch (text/n3) PATCH dialect on the
            // Graph-Store-Protocol graph route. The always-on application/sparql-update PATCH
            // dialect needs no flag; this enables the OPTIONAL text/n3 one. Off by default
            // (env SPARQ_N3_PATCH=1).
            #[cfg(feature = "n3-patch")]
            "--n3-patch" => config.n3_patch = true,
            // [OPUS-4.8] sq-vczh2: OPT-IN verifiable terse-transpiler endpoint — POST a terse
            // query to /terse/transpile and the server returns the canonical SPARQL it expands to
            // (it never executes it). Off by default (env SPARQ_TERSE=1).
            #[cfg(feature = "terse")]
            "--terse" => config.terse = true,
            // [FABLE-5] sq-lsp7k.10: OPT-IN named-parameterized-template REST surface — the
            // /templates store (list / PUT-GET-DELETE one definition / POST-invoke with typed
            // JSON arguments). Query invocations are read-gated; template writes and UPDATE
            // invocations are write-gated through the same sequenced-writer path as /sparql
            // updates. Off by default (env SPARQ_TEMPLATES=1); --templates-file PATH (env
            // SPARQ_TEMPLATES_FILE) makes the store durable (fail-closed load at startup).
            #[cfg(feature = "templates")]
            "--templates" => config.templates = true,
            #[cfg(feature = "templates")]
            "--templates-file" => {
                let v: String = parse_flag(&mut args, "--templates-file")?;
                config.templates_file = (!v.is_empty()).then(|| std::path::PathBuf::from(v));
            }
            // [FABLE-5] sq-snopa.6 (issue #992 FR-4): OPT-IN Solid WAC/ACP HTTP authorization
            // surface — POST /authz/decide, /authz/wac-allow, /authz/query, a thin HTTP shell over
            // the sparq-solid library authoriser. Fail-closed on every error path. Off by default
            // (env SPARQ_SOLID_AUTHZ=1).
            #[cfg(feature = "solid-authz")]
            "--solid-authz" => config.solid_authz = true,
            // [GPT-5.6] (sq-kqofk, gh-906): OPT-IN durable CDC change-stream at DIR. Records each
            // published group-commit generation on the writer thread to the segmented, fsync'd log
            // and serves the Neptune-GetRecords-shaped GET /streams endpoint. Resumes gaplessly
            // when DIR already holds segments. Off by default (env SPARQ_CHANGE_STREAM=DIR).
            #[cfg(feature = "change-stream")]
            "--change-stream" => {
                let dir = args.next().ok_or("--change-stream requires a directory path")?;
                if dir.is_empty() {
                    return Err("--change-stream must not be empty".into());
                }
                config.change_stream_dir = Some(std::path::PathBuf::from(dir));
            }
            // [OPUS-4.8] sq-4w18: SERVICE egress allowlist. Repeatable: each value adds one
            // host (`sparql.example.org`) or suffix wildcard (`*.example.org`). With NO
            // allowlist (the default) every SERVICE clause is refused (default-DENY-all).
            "--service-allow" => {
                service_allow_cli.push(
                    args.next()
                        .ok_or("--service-allow requires a host/pattern")?,
                );
            }
            "--service-allow-file" => {
                service_allow_file =
                    Some(args.next().ok_or("--service-allow-file requires a path")?);
            }
            // [OPUS-4.8] sq-o7o0 (ASVS V14.5.3): first-party CORS origin allowlist. Repeatable:
            // each value adds one exact origin (`https://app.example.org`). With NO allowlist
            // (the default) NO CORS headers are emitted (a cross-origin browser read is blocked).
            "--cors-allow-origin" => {
                cors_allow_cli.push(
                    args.next()
                        .ok_or("--cors-allow-origin requires an origin (scheme://host[:port])")?,
                );
            }
            "--cors-allow-origin-file" => {
                cors_allow_file =
                    Some(args.next().ok_or("--cors-allow-origin-file requires a path")?);
            }
            "-h" | "--help" => {
                let time_travel = if cfg!(feature = "time-travel") {
                    " [--time-travel-generations N] [--time-travel-max-age SECS]"
                } else {
                    ""
                };
                let service = if cfg!(feature = "service") {
                    " [--service-allow HOST|*.SUFFIX]... [--service-allow-file PATH]"
                } else {
                    ""
                };
                // [GPT-5.6] sq-lsp7k.5.2: surface the OPT-IN facet-count endpoint flag.
                let facets = if cfg!(feature = "facets") {
                    " [--facets]"
                } else {
                    ""
                };
                // [GPT-5.6] sq-lsp7k.9.3: surface the OPT-IN prefix-completion endpoint flag.
                let complete = if cfg!(feature = "complete") {
                    " [--complete]"
                } else {
                    ""
                };
                // [OPUS-4.8] sq-r74h: surface the brTPF binding-set DoS caps in --help (feature brtpf).
                let brtpf = if cfg!(feature = "brtpf") {
                    " [--brtpf-max-bindings N] [--brtpf-max-values-bytes N]"
                } else {
                    ""
                };
                // [OPUS-4.8] sq-r868: surface the SHACL validate endpoint flag (feature shacl).
                let shacl = if cfg!(feature = "shacl") {
                    " [--shacl] [--shacl-guard --shacl-shapes FILE]"
                } else {
                    ""
                };
                // [OPUS-4.8] sq-hj4n: surface the OPT-IN N3-Patch PATCH dialect flag (feature n3-patch).
                let n3_patch = if cfg!(feature = "n3-patch") {
                    " [--n3-patch]"
                } else {
                    ""
                };
                // [OPUS-4.8] sq-vczh2: surface the OPT-IN terse-transpiler endpoint flag (feature terse).
                let terse = if cfg!(feature = "terse") {
                    " [--terse]"
                } else {
                    ""
                };
                // [FABLE-5] sq-lsp7k.10: surface the OPT-IN template-surface flags (feature templates).
                let templates = if cfg!(feature = "templates") {
                    " [--templates] [--templates-file PATH]"
                } else {
                    ""
                };
                // [OPUS-4.8] sq-o5bi / sq-ft7u: surface the OPT-IN restore flags (feature backup).
                let backup = if cfg!(feature = "backup") {
                    " [--restore FILE] [--restore-persist]"
                } else {
                    ""
                };
                // [OPUS-4.8] sq-2999l: surface the OPT-IN CDC change-stream flag (feature change-stream).
                let change_stream = if cfg!(feature = "change-stream") {
                    " [--change-stream DIR]"
                } else {
                    ""
                };
                // [GPT-5.6] sq-oprna.3: these flags exist only in an `http3` build.
                let http3 = if cfg!(feature = "http3") {
                    " [--http3 [--http3-addr HOST:PORT] --tls-cert FILE --tls-key FILE]"
                } else {
                    ""
                };
                eprintln!(
                    "usage: sparq-server [--addr HOST:PORT] [--allow-remote] [--persist DIR] \
                     [--auth-token TOKEN] [--auth-token-read] [--format FMT] \
                     [--query-timeout SECS] [--update-where-timeout SECS] [--no-adaptive-commit] [--header-read-timeout SECS] \
                     [--max-body-bytes N] [--max-concurrent N] \
                     [--max-results N] [--max-query-rows N] [--max-query-bytes N] [--max-decompress-ratio N] \
                     [--max-subscriptions N] \
                     [--max-subscriptions-per-conn N]{time_travel}{service}{facets}{complete}{brtpf}{shacl}{n3_patch}{terse}{templates}{backup}{change_stream} \
                     [--cors-allow-origin ORIGIN]... [--cors-allow-origin-file PATH] [--verbose] \
                     [--log-full-requests]{http3} [DATA_FILE]\n  \
                     or: sparq-server --health-probe [--health-probe-addr HOST:PORT]\n\n  \
                     HEALTH-PROBE: --health-probe (the container HEALTHCHECK) connects to an \
                     already-running server's /health on 127.0.0.1:3030 (override with \
                     --health-probe-addr or SPARQ_HEALTH_PROBE_ADDR) and exits 0 (healthy) / \
                     non-zero (unhealthy). For distroless images with no curl/wget; it does NOT \
                     start a server.\n\n  \
                     PERSIST: --persist DIR (env SPARQ_PERSIST_DIR) makes the on-disk index at \
                     DIR the durable source of truth (QLever --persist-updates): every committed \
                     UPDATE is WAL-fsync'd before ack, so a RESTART on the same DIR restores ALL \
                     updates with no rebuild. An existing DIR is opened (DATA_FILE ignored); an \
                     empty DIR is seeded from DATA_FILE. Without it the server is in-memory \
                     (updates lost on restart).\n  \
                     AUTH: --auth-token TOKEN (env SPARQ_AUTH_TOKEN) requires \
                     'Authorization: Bearer TOKEN' on every WRITE (SPARQL Update + GSP \
                     PUT/POST/DELETE) -> 401 + 'WWW-Authenticate: Bearer' otherwise \
                     (constant-time compared; QLever's -a). Add --auth-token-read (env \
                     SPARQ_AUTH_TOKEN_READ=1) to ALSO gate reads. Unset = no auth.\n  \
                     SECURITY (bind): --addr defaults to loopback (127.0.0.1); a non-loopback \
                     bind (e.g. 0.0.0.0) is REFUSED unless --allow-remote (or \
                     SPARQ_ALLOW_REMOTE=1) is set OR the whole surface is authenticated \
                     (--auth-token AND --auth-token-read) — a write-token alone still leaves \
                     reads open. Put it behind a reverse proxy / gateway that enforces auth.\n  \
                     SERVICE federation (the `service` build feature) is DENY-ALL by default: \
                     SERVICE <iri> reaches NOTHING unless the host is allowlisted via \
                     --service-allow / --service-allow-file / SPARQ_SERVICE_ALLOW \
                     (exact host or *.suffix wildcard) — an SSRF guard.\n  \
                     RESTORE (the `backup` build feature): --restore FILE (env SPARQ_RESTORE) \
                     rehydrates the store from a backup artifact on start. Without --persist it is \
                     in-memory only. With --persist DIR, ADD --restore-persist (env \
                     SPARQ_RESTORE_PERSIST=1) to write the restore THROUGH to the durable dir \
                     crash-safely so it survives a restart; without it the --restore/--persist \
                     combination is refused. The online route POST /admin/restore mirrors this: \
                     on a --persist server it needs ?persist=true (else 409).\n  \
                     CHANGE-STREAM (the `change-stream` build feature): --change-stream DIR (env \
                     SPARQ_CHANGE_STREAM) records every committed UPDATE as one ordered, durable \
                     change record to a segmented fsync'd log at DIR, and serves the \
                     Neptune-GetRecords-shaped poll endpoint GET /streams over it \
                     (?iteratorType=TRIM_HORIZON|AT_SEQUENCE_NUMBER|AFTER_SEQUENCE_NUMBER|LATEST, \
                     ?at=N / ?after=N, ?limit=N) returning the records + a nextSequenceNumber \
                     continuation token. Resumes the same stream gaplessly across a restart. Off by \
                     default; reading is gated by the read auth.\n  \
                     HTTP/2 (the `http2` build feature): the TCP listener accepts h1 and h2c. \
                     Supply both --tls-cert and --tls-key PEM files to enable TLS with ALPN \
                     h2/http/1.1; omitting both keeps cleartext compatibility. WebSocket \
                     subscriptions use HTTP/1.1 upgrade.\n  \
                     HTTP/3 (the `http3` build feature): --http3 starts an encrypted QUIC/UDP \
                     listener beside TCP. It defaults to the same address and numeric port; \
                     override UDP with --http3-addr. Both PEM flags are required.\n  \
                     CORS: NO CORS headers by default (a cross-origin browser fetch cannot read \
                     responses). For a FIRST-PARTY browser app on another origin, allowlist its \
                     exact origin(s) via --cors-allow-origin ORIGIN (repeatable) / \
                     --cors-allow-origin-file PATH / SPARQ_CORS_ALLOW_ORIGIN \
                     (scheme://host[:port]); only a listed Origin is reflected (never '*', never \
                     with credentials), and preflight OPTIONS is answered for it."
                );
                return Ok(());
            }
            other => data_file = Some(other.to_string()),
        }
    }

    // [GPT-5.6] sq-oprna.6: TCP TLS is selected by a complete credential pair. Reject a lone
    // path rather than silently serving cleartext; `http3` retains its stricter runtime rule.
    #[cfg(feature = "http2")]
    if tls_cert.is_some() != tls_key.is_some() {
        return Err("--tls-cert and --tls-key must be provided together".into());
    }

    // [GPT-5.6] sq-oprna.3: QUIC cannot run without TLS. Reject incomplete configuration
    // before loading a potentially large dataset or binding either listener.
    #[cfg(feature = "http3")]
    if http3 && (tls_cert.is_none() || tls_key.is_none()) {
        return Err("--http3 requires both --tls-cert FILE and --tls-key FILE".into());
    }

    // [GPT-5.6] sq-lsp7k.2.4: load shapes once at startup, failing closed on bad input.
    #[cfg(feature = "shacl")]
    if let Some(path) = shacl_shapes_path {
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("--shacl-shapes: cannot read {}: {e}", path.display()))?;
        let format = match path.extension().and_then(|s| s.to_str()).unwrap_or("") {
            "nt" => "ntriples", "nq" => "nquads", "trig" => "trig", _ => "turtle",
        };
        let shapes = Graph::load_str(&text, format)
            .map_err(|e| format!("--shacl-shapes: failed to load {} as {format}: {e}", path.display()))?;
        config.shacl_shapes = Some(sparq_server::ShaclShapes::new(shapes));
    }

    // [OPUS-4.8] sq-4w18: merge the --service-allow-file + --service-allow CLI entries
    // INTO the env baseline already in config.service_allow (union — additive). A
    // malformed entry or unreadable file is a hard startup error: better to refuse to
    // boot than to silently run with an allowlist narrower than the operator wrote.
    if let Some(path) = &service_allow_file {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("--service-allow-file {path}: {e}"))?;
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            config.service_allow.add(line)?;
        }
    }
    for entry in &service_allow_cli {
        config.service_allow.add(entry)?;
    }

    // [OPUS-4.8] sq-o7o0: merge the --cors-allow-origin-file + --cors-allow-origin CLI
    // entries INTO the SPARQ_CORS_ALLOW_ORIGIN env baseline already in config.cors_allow
    // (union — additive). A malformed origin or unreadable file is a hard startup error:
    // refuse to boot rather than silently run with a narrower-than-written allowlist.
    if let Some(path) = &cors_allow_file {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("--cors-allow-origin-file {path}: {e}"))?;
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            config.cors_allow.add(line)?;
        }
    }
    for entry in &cors_allow_cli {
        config.cors_allow.add(entry)?;
    }

    // [OPUS-4.8] sq-0bxp: the per-query access audit log emits via `tracing`, so it needs a
    // subscriber too — install one when --verbose OR --audit-log is set. The default filter
    // when audit is on (and not verbose) routes the dedicated `sparq_server::audit` target at
    // info so the audit trail surfaces without the noisier request log; RUST_LOG still wins.
    #[cfg(feature = "audit-log")]
    let want_subscriber = config.verbose || config.audit_log;
    #[cfg(not(feature = "audit-log"))]
    let want_subscriber = config.verbose;
    if want_subscriber {
        // Default to request-level tracing; RUST_LOG still wins if set.
        #[cfg(feature = "audit-log")]
        let default_filter = if config.verbose {
            "tower_http=debug,sparq_server=debug"
        } else {
            // Audit-only: surface the audit target (and warnings) without the debug request log.
            "sparq_server::audit=info,warn"
        };
        #[cfg(not(feature = "audit-log"))]
        let default_filter = "tower_http=debug,sparq_server=debug";
        tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| default_filter.into()),
            )
            .init();
    }

    // [OPUS-4.8] sq-7cxr: when --persist points at a directory that ALREADY holds a store,
    // the on-disk index is the source of truth and the DATA_FILE seed is ignored — so don't
    // even read it (loading a large file only to discard it is wasteful and surprising). The
    // seed graph is then empty; `AppState::try_with_config` opens the existing store regardless.
    let persist_has_store = config
        .persist_dir
        .as_ref()
        .is_some_and(|d| d.join("dict-meta.bin").exists() || d.join("dict.bin").exists());
    let graph = match (&data_file, persist_has_store) {
        (_, true) => {
            let dir = config
                .persist_dir
                .as_ref()
                .expect("persist_has_store implies a dir");
            eprintln!(
                "--persist {}: opening existing durable store (replaying WAL; DATA_FILE, if any, ignored)",
                dir.display()
            );
            Graph::load_str("", "turtle").map_err(|e| format!("init: {e}"))?
        }
        (Some(path), false) => {
            let text = std::fs::read_to_string(path)?;
            eprintln!("loading {path} ({format}) ...");
            Graph::load_str(&text, &format).map_err(|e| format!("failed to load {path}: {e}"))?
        }
        (None, false) => {
            eprintln!("no data file given — starting with an empty default graph");
            Graph::load_str("", "turtle").map_err(|e| format!("init: {e}"))?
        }
    };
    // [OPUS-4.8] sq-o5bi / sq-ft7u: RESTORE-ON-START. When --restore <FILE> / SPARQ_RESTORE is set,
    // the backup artifact REPLACES the seed graph (fail-closed: a corrupt/mismatched artifact aborts
    // startup with a clean error).
    //
    // WITHOUT --restore-persist (the historical default): in-memory only — the combination with
    // --persist is REFUSED (an in-memory-only restore on a durable server would be silently lost on
    // the next restart).
    //
    // WITH --restore-persist (sq-ft7u) AND --persist DIR: the artifact is written THROUGH to the
    // durable dir crash-safely (`Graph::restore_into_durable`) BEFORE the store is opened, so the
    // restored dataset survives a restart (the on-disk base IS the restored image). `try_with_config`
    // then opens the already-restored durable dir.
    #[cfg(feature = "backup")]
    let graph = match (&config.restore, &config.persist_dir, config.restore_persist) {
        // --restore + --persist + --restore-persist: write the artifact through to the durable dir.
        (Some(file), Some(dir), true) => {
            // [OPUS-4.8] sq-bu1a + sq-ft7u: PITR delta replay INTO a durable store is out of scope
            // in v1 (the durable write-through swaps the base image only; replaying a delta chain
            // into a --persist dir needs its own crash-safe path). Refuse the combination loudly
            // rather than silently dropping the operator's --restore-delta chain.
            if !config.restore_delta.is_empty() {
                return Err(
                    "--restore-delta (point-in-time recovery) cannot be combined with \
                     --restore-persist in v1: replaying a delta chain into the durable store is \
                     not yet supported; restore the base only, or use an in-memory --restore"
                        .into(),
                );
            }
            eprintln!(
                "restoring durable store at {} from backup artifact {} (write-through) ...",
                dir.display(),
                file.display()
            );
            let f = std::fs::File::open(file)
                .map_err(|e| format!("--restore: cannot open {}: {e}", file.display()))?;
            let (fresh, meta) = sparq_serve::backup_import(std::io::BufReader::new(f))
                .map_err(|e| format!("--restore: rejected {} ({e})", file.display()))?;
            // Crash-safe two-rename swap of the durable dir to the restored image (fail-closed:
            // a corrupt artifact already aborted above; an interrupted swap is healed on open).
            Graph::restore_into_durable(dir, fresh)
                .map_err(|e| format!("--restore-persist: durable restore into {} failed: {e}", dir.display()))?;
            eprintln!(
                "restored durable store from artifact taken at generation {} ({} triples) — survives restart",
                meta.generation, meta.triples
            );
            // `try_with_config` opens the now-restored durable dir; the seed graph is ignored.
            Graph::load_str("", "turtle").map_err(|e| format!("init: {e}"))?
        }
        // --restore WITHOUT --persist: the historical in-memory restore (RAM-only).
        (Some(file), None, _) => {
            eprintln!("restoring in-memory store from backup artifact {} ...", file.display());
            let f = std::fs::File::open(file)
                .map_err(|e| format!("--restore: cannot open {}: {e}", file.display()))?;
            let (mut g, meta) = sparq_serve::backup_import(std::io::BufReader::new(f))
                .map_err(|e| format!("--restore: rejected {} ({e})", file.display()))?;
            eprintln!(
                "restored from artifact taken at generation {} ({} triples)",
                meta.generation, meta.triples
            );
            // [OPUS-4.8] sq-bu1a: POINT-IN-TIME RECOVERY — replay the --restore-delta chain forward
            // onto the base. Fail-closed: a corrupt / out-of-order / gapped delta aborts startup.
            if !config.restore_delta.is_empty() {
                let mut decoded = Vec::with_capacity(config.restore_delta.len());
                for d in &config.restore_delta {
                    let df = std::fs::File::open(d)
                        .map_err(|e| format!("--restore-delta: cannot open {}: {e}", d.display()))?;
                    let delta = sparq_serve::backup_import_delta(std::io::BufReader::new(df))
                        .map_err(|e| format!("--restore-delta: rejected {} ({e})", d.display()))?;
                    decoded.push(delta);
                }
                let recovered = sparq_serve::backup_replay(&mut g, &meta, decoded.iter())
                    .map_err(|e| format!("--restore-delta: replay failed ({e})"))?;
                eprintln!(
                    "replayed {} delta artifact(s) — recovered to generation {} ({} triples)",
                    config.restore_delta.len(),
                    recovered.generation,
                    recovered.triples
                );
            }
            g
        }
        // --restore + --persist WITHOUT --restore-persist: refused (footgun, see above).
        (Some(_), Some(_), false) => {
            return Err(
                "--restore and --persist cannot be combined unless --restore-persist is set \
                 (an in-memory-only restore on a durable server would be silently lost on \
                 restart; --restore-persist writes the restore through to the durable dir)"
                    .into(),
            );
        }
        // No --restore: the seed graph as built above.
        (None, _, _) => {
            // [OPUS-4.8] sq-bu1a: --restore-delta is meaningless without a --restore base; refuse
            // it loudly rather than silently ignoring an operator's PITR intent.
            if !config.restore_delta.is_empty() {
                return Err("--restore-delta requires --restore (a base artifact to replay onto)".into());
            }
            graph
        }
    };
    eprintln!("loaded {} triples", graph.len());
    eprintln!(
        "guards: query-timeout={} update-where-timeout={} header-read-timeout={} body-read-timeout={} max-body-bytes={} max-concurrent={} max-results={} \
         max-query-rows={} max-query-bytes={} max-decompress-ratio={}x max-subscriptions={} \
         max-subscriptions-per-conn={}",
        config
            .query_timeout
            .map_or("off".into(), |t| format!("{}s", t.as_secs())),
        // [OPUS-4.8] sq-nulp: writer-side WHERE deadline bounding head-of-line blocking.
        config
            .update_where_timeout
            .map_or("off".into(), |t| format!("{}s", t.as_secs())),
        // [OPUS-4.8] sq-2gqr: connection-layer header-read deadline (slow-loris guard).
        config
            .header_read_timeout
            .map_or("off".into(), |t| format!("{}s", t.as_secs())),
        // [OPUS-4.8] sq-lodb: connection-layer body read/idle deadline (slow-body guard).
        config
            .body_read_timeout
            .map_or("off".into(), |t| format!("{}s", t.as_secs())),
        config.max_body_bytes,
        config.max_concurrent,
        config.max_results.map_or("off".into(), |n| n.to_string()),
        // [OPUS-4.8] sq-ebii/sq-s5is: memory caps (row + byte) + decompression-ratio cap.
        config
            .max_query_rows
            .map_or("off".into(), |n| n.to_string()),
        config
            .max_query_bytes
            .map_or("off".into(), |n| n.to_string()),
        config.max_decompress_ratio,
        config.max_subscriptions,
        config.max_subscriptions_per_conn,
    );
    // [OPUS-4.8] sq-p7kk5: surface the write-path commit mode at startup.
    eprintln!(
        "write path: adaptive-group-commit={}",
        if config.adaptive_commit { "on" } else { "off" }
    );
    // [OPUS-4.8] sq-r74h: surface the brTPF binding-set DoS caps at startup (feature `brtpf`).
    #[cfg(feature = "brtpf")]
    {
        let fmt0 = |n: usize| {
            if n == 0 {
                "off".to_string()
            } else {
                n.to_string()
            }
        };
        eprintln!(
            "guards (brtpf): brtpf-max-bindings={} brtpf-max-values-bytes={}",
            fmt0(config.brtpf_max_bindings),
            fmt0(config.brtpf_max_values_bytes),
        );
    }
    // [OPUS-4.8] sq-7cxr: surface the durable-persistence posture at startup.
    match &config.persist_dir {
        Some(dir) => eprintln!(
            "persistence: ON — durable store at {} (updates WAL-fsync'd before ack; survive restart, QLever --persist-updates)",
            dir.display()
        ),
        None => eprintln!("persistence: OFF — in-memory only (updates are LOST on restart; pass --persist DIR to enable)"),
    }
    // [OPUS-4.8] sq-o5bi / sq-ft7u: surface the online backup/restore posture at startup, including
    // whether the restore (on-start or via /admin/restore?persist=true) writes THROUGH to the
    // durable store so it survives a restart.
    #[cfg(feature = "backup")]
    eprintln!(
        "backup/restore: ON (admin routes POST /admin/backup + /admin/backup/delta + \
         /admin/restore, write-gated; online snapshot, no stop-the-world){}{}{}",
        match &config.restore {
            Some(f) => format!(" — restored on start from {}", f.display()),
            None => String::new(),
        },
        // [OPUS-4.8] sq-bu1a: surface a replayed PITR delta chain in the posture line.
        if config.restore_delta.is_empty() {
            String::new()
        } else {
            format!(" + {} delta(s) replayed (PITR)", config.restore_delta.len())
        },
        // [OPUS-4.8] sq-ft7u: surface the restore-into-durable posture (write-through vs refusal).
        if config.persist_dir.is_some() {
            if config.restore_persist {
                " — restore-into-durable ENABLED (--restore-persist / ?persist=true writes through to the persist dir; survives restart)"
            } else {
                " — restore-into-durable OFF (a /admin/restore needs ?persist=true on this --persist server, else 409)"
            }
        } else {
            ""
        }
    );
    // [OPUS-4.8] sq-4w18: surface the SERVICE egress posture at startup so an operator
    // sees whether (and where) federation can reach. Only meaningful with the `service`
    // build feature; without it a SERVICE clause errors at execution regardless.
    #[cfg(feature = "service")]
    eprintln!(
        "SERVICE egress allowlist: {}",
        config.service_allow.display()
    );
    // [OPUS-4.8] sq-o7o0: surface the CORS posture at startup. Empty (the default) = no CORS
    // headers; configured = the exact first-party origins a browser may read responses from.
    eprintln!("CORS first-party origin allowlist: {}", config.cors_allow.display());
    #[cfg(feature = "time-travel")]
    eprintln!(
        "time travel: ?generation=N enabled — generations={} max-age={} (each retained generation is a full graph)",
        config.time_travel_generations,
        config.time_travel_max_age.map_or("off".into(), |t| format!("{}s", t.as_secs())),
    );

    // [OPUS-4.8] sq-zcby: surface the auth posture at startup. A write-only token (no
    // --auth-token-read) leaves reads OPEN — warn so the operator is not surprised; a fully
    // gated surface is reported too. No token = silent (the bind posture already warns on a
    // non-loopback bind without auth).
    let auth = AuthPosture::from_config(&config);
    match auth {
        AuthPosture::None => {}
        AuthPosture::WriteOnly => eprintln!(
            "auth: --auth-token set — WRITES require 'Authorization: Bearer <token>'; READS \
             remain OPEN (add --auth-token-read to gate reads too). The /subscriptions \
             WebSocket and /subscriptions/sse stream are read surfaces and are NOT gated \
             without --auth-token-read."
        ),
        // [OPUS-4.8] sq-cxk5: with --auth-token-read the subscription read surfaces (the
        // /subscriptions WebSocket and the /subscriptions/sse stream) are gated by the read
        // token too — no longer an open read bypass.
        AuthPosture::ReadAndWrite => eprintln!(
            "auth: --auth-token + --auth-token-read set — the whole surface (reads AND writes), \
             including the /subscriptions WebSocket and /subscriptions/sse stream, requires the \
             Bearer token. WS browser clients pass it as a 'Sec-WebSocket-Protocol: bearer.<token>' \
             subprotocol (no 'Authorization' header on a WS handshake); deliver the token over TLS."
        ),
    }

    let addr: SocketAddr = addr.parse()?;
    #[cfg(feature = "http3")]
    let http3_addr = if http3 {
        Some(match http3_addr {
            Some(value) => value
                .parse::<SocketAddr>()
                .map_err(|e| format!("--http3-addr: invalid HOST:PORT '{value}': {e}"))?,
            None => addr,
        })
    } else {
        None
    };
    // [OPUS-4.8] sq-o4qf / sq-zcby: enforce the bind posture BEFORE binding. A non-loopback
    // address is refused unless explicitly opted into (--allow-remote / SPARQ_ALLOW_REMOTE)
    // OR the whole surface is authenticated (--auth-token AND --auth-token-read). A
    // write-token alone still leaves reads open, so it still requires --allow-remote.
    match bind_posture(&addr, config.allow_remote, auth) {
        BindPosture::Loopback => {}
        BindPosture::RemoteAllowed { warning } => eprintln!("{warning}"),
        BindPosture::RemoteRefused { message } => return Err(message.into()),
    }
    // [GPT-5.6] sq-oprna.3: the independently configured UDP listener obeys the same
    // loopback/auth/--allow-remote exposure policy as the TCP listener.
    #[cfg(feature = "http3")]
    if let Some(http3_addr) = http3_addr {
        match bind_posture(&http3_addr, config.allow_remote, auth) {
            BindPosture::Loopback => {}
            BindPosture::RemoteAllowed { warning } => eprintln!("HTTP/3: {warning}"),
            BindPosture::RemoteRefused { message } => {
                return Err(format!("HTTP/3: {message}").into());
            }
        }
    }

    // [OPUS-4.8] sq-2gqr / sq-lodb: capture the connection-layer slow-client deadlines BEFORE
    // `config` is moved into `AppState` — `sparq_server::serve` (not `axum::serve`) installs the
    // header-read deadline (slow-loris HEADER guard, sq-2gqr) and the body read/idle deadline
    // (slow-BODY guard, sq-lodb). See ServerConfig::header_read_timeout / ::body_read_timeout.
    let header_read_timeout = config.header_read_timeout;
    let body_read_timeout = config.body_read_timeout;
    // [OPUS-4.8] sq-7cxr: `try_with_config` surfaces a durable-open error (corrupt persist dir,
    // unwritable path) as a clean startup error + non-zero exit, rather than panicking.
    let state = AppState::try_with_config(graph, config)?;
    let app = router(state);

    // [GPT-5.6] sq-oprna.6: a complete PEM pair opts the HTTP/2-capable TCP listener into TLS;
    // no pair preserves the cleartext h1+h2c path. Parse before binding so invalid material fails
    // startup without briefly exposing a listening socket.
    #[cfg(feature = "http2")]
    let tcp_tls_config = match (tls_cert.as_deref(), tls_key.as_deref()) {
        (Some(cert), Some(key)) => Some(load_tcp_tls_config(cert, key)?),
        (None, None) => None,
        _ => unreachable!("the credential-pair invariant was validated above"),
    };

    let listener = tokio::net::TcpListener::bind(addr).await?;
    #[cfg(not(feature = "http2"))]
    eprintln!("sparq-server listening on http://{addr}  (SPARQL endpoint: /sparql, subscriptions: ws://{addr}/subscriptions)");
    #[cfg(feature = "http2")]
    if tcp_tls_config.is_some() {
        eprintln!("sparq-server listening on https://{addr} (HTTP/1.1 + HTTP/2; SPARQL endpoint: /sparql, subscriptions: wss://{addr}/subscriptions)");
    } else {
        eprintln!("sparq-server listening on http://{addr} (HTTP/1.1 + h2c; SPARQL endpoint: /sparql, subscriptions: ws://{addr}/subscriptions)");
    }
    // [GPT-5.6] sq-oprna.3: feature-on/runtime-on adds a separate encrypted UDP listener,
    // but both transports dispatch through clones of the one router built above. The TCP
    // future is still the exact bespoke `sparq_server::serve` path with both slow-client
    // timers and WebSocket upgrades intact.
    #[cfg(feature = "http3")]
    if let Some(http3_addr) = http3_addr {
        let cert = tls_cert.as_deref().expect("validated with --http3");
        let key = tls_key.as_deref().expect("validated with --http3");
        let endpoint = bind_http3_endpoint(http3_addr, cert, key)?;
        let bound_addr = endpoint.local_addr()?;
        // [GPT-5.6] sq-oprna.4: derive the advertisement from the successfully bound endpoint, not
        // from requested configuration. Keep the h3 router unlayered: Alt-Svc is discovery for the
        // TCP response path, and both routers still share the same application handlers.
        let tcp_app = app
            .clone()
            .layer(sparq_http3::alt_svc_layer(bound_addr.port()));
        eprintln!("sparq-server HTTP/3 listening on https://{bound_addr} (QUIC/UDP; WebSocket subscriptions remain HTTP/1.1-only)");

        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        let shutdown_task = tokio::spawn(async move {
            shutdown_signal().await;
            let _ = shutdown_tx.send(true);
        });
        let tcp_shutdown = wait_for_shutdown(shutdown_rx.clone());
        let h3_shutdown = wait_for_shutdown(shutdown_rx);
        #[cfg(not(feature = "http2"))]
        let tcp = sparq_server::serve(
            listener,
            tcp_app,
            header_read_timeout,
            body_read_timeout,
            tcp_shutdown,
        );
        #[cfg(feature = "http2")]
        let tcp = {
            let tcp_tls_config = tcp_tls_config.clone();
            async move {
                if let Some(tls_config) = tcp_tls_config {
                    sparq_server::serve_tls(
                        listener,
                        tcp_app,
                        tls_config,
                        header_read_timeout,
                        body_read_timeout,
                        tcp_shutdown,
                    )
                    .await
                } else {
                    sparq_server::serve(
                        listener,
                        tcp_app,
                        header_read_timeout,
                        body_read_timeout,
                        tcp_shutdown,
                    )
                    .await
                }
            }
        };
        let h3 = sparq_http3::serve_h3(endpoint, app, h3_shutdown);
        let result = tokio::try_join!(tcp, h3);
        shutdown_task.abort();
        result?;
        return Ok(());
    }
    // [OPUS-4.8] sq-2gqr / sq-lodb: NOT `axum::serve` — it never installs a timer, so hyper's
    // header-read deadline is inert there and a slow-loris client holds a connection (and
    // concurrency slot) open forever. `sparq_server::serve` is the same accept + graceful-drain
    // loop WITH the header-read deadline (slow-loris HEADER guard) and the body read/idle deadline
    // (slow-BODY guard) wired in.
    #[cfg(not(feature = "http2"))]
    sparq_server::serve(
        listener,
        app,
        header_read_timeout,
        body_read_timeout,
        shutdown_signal(),
    )
    .await?;
    #[cfg(feature = "http2")]
    if let Some(tls_config) = tcp_tls_config {
        sparq_server::serve_tls(
            listener,
            app,
            tls_config,
            header_read_timeout,
            body_read_timeout,
            shutdown_signal(),
        )
        .await?;
    } else {
        sparq_server::serve(
            listener,
            app,
            header_read_timeout,
            body_read_timeout,
            shutdown_signal(),
        )
        .await?;
    }
    Ok(())
}

/// [GPT-5.6] sq-oprna.6: load the TCP listener's TLS-1.3 certificate and key and advertise both
/// supported application protocols. The auto builder selects the matching h1/h2 connection.
#[cfg(feature = "http2")]
fn load_tcp_tls_config(
    cert_path: &std::path::Path,
    key_path: &std::path::Path,
) -> Result<Arc<rustls::ServerConfig>, Box<dyn std::error::Error>> {
    // [OPUS-5] sq-5ah3p: PEM decoding via `rustls-pki-types`' `PemObject`, reached through rustls'
    // own re-export, retiring the archived `rustls-pemfile` shim (RUSTSEC-2025-0134).
    use rustls::pki_types::pem::PemObject as _;

    let cert_file = std::fs::File::open(cert_path)
        .map_err(|e| format!("--tls-cert: cannot read {}: {e}", cert_path.display()))?;
    let certs = rustls::pki_types::CertificateDer::pem_reader_iter(cert_file)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("--tls-cert: malformed PEM in {}: {e}", cert_path.display()))?;
    if certs.is_empty() {
        return Err(format!(
            "--tls-cert: {} contains no certificates",
            cert_path.display()
        )
        .into());
    }

    let key_file = std::fs::File::open(key_path)
        .map_err(|e| format!("--tls-key: cannot read {}: {e}", key_path.display()))?;
    // `from_pem_reader` folds "no key section here" into `NoItemsFound` where the retired
    // `rustls_pemfile::private_key` returned `Ok(None)`; mapping that one variant back keeps both
    // operator-facing diagnostics (empty vs corrupt) exactly as they were.
    let key = rustls::pki_types::PrivateKeyDer::from_pem_reader(key_file).map_err(|e| match e {
        rustls::pki_types::pem::Error::NoItemsFound => {
            format!("--tls-key: {} contains no private key", key_path.display())
        }
        other => format!(
            "--tls-key: malformed PEM in {}: {other}",
            key_path.display()
        ),
    })?;

    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let mut tls = rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| format!("TCP TLS certificate/key configuration is invalid: {e}"))?;
    tls.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(Arc::new(tls))
}

/// [GPT-5.6] sq-oprna.3: load an operator-supplied PEM chain/key into an aws-lc-rs-backed,
/// TLS-1.3-only Quinn endpoint. This helper is absent from feature-off binaries.
#[cfg(feature = "http3")]
fn bind_http3_endpoint(
    addr: SocketAddr,
    cert_path: &std::path::Path,
    key_path: &std::path::Path,
) -> Result<quinn::Endpoint, Box<dyn std::error::Error>> {
    use rustls::pki_types::pem::PemObject as _;

    let cert_file = std::fs::File::open(cert_path)
        .map_err(|e| format!("--tls-cert: cannot read {}: {e}", cert_path.display()))?;
    let certs = rustls::pki_types::CertificateDer::pem_reader_iter(cert_file)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("--tls-cert: malformed PEM in {}: {e}", cert_path.display()))?;
    if certs.is_empty() {
        return Err(format!(
            "--tls-cert: {} contains no certificates",
            cert_path.display()
        )
        .into());
    }

    let key_file = std::fs::File::open(key_path)
        .map_err(|e| format!("--tls-key: cannot read {}: {e}", key_path.display()))?;
    // `from_pem_reader` folds "no key section here" into `NoItemsFound` where the retired
    // `rustls_pemfile::private_key` returned `Ok(None)`; mapping that one variant back keeps both
    // operator-facing diagnostics (empty vs corrupt) exactly as they were.
    let key = rustls::pki_types::PrivateKeyDer::from_pem_reader(key_file).map_err(|e| match e {
        rustls::pki_types::pem::Error::NoItemsFound => {
            format!("--tls-key: {} contains no private key", key_path.display())
        }
        other => format!(
            "--tls-key: malformed PEM in {}: {other}",
            key_path.display()
        ),
    })?;

    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let mut tls = rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| format!("HTTP/3 TLS certificate/key configuration is invalid: {e}"))?;
    tls.alpn_protocols = vec![b"h3".to_vec()];
    let quic = sparq_http3::quic_server_config(tls)?;
    quinn::Endpoint::server(quic, addr).map_err(Into::into)
}

/// [GPT-5.6] sq-oprna.3: adapt one shared signal notification into each listener's
/// independently owned shutdown future.
#[cfg(feature = "http3")]
async fn wait_for_shutdown(mut shutdown: tokio::sync::watch::Receiver<bool>) {
    while !*shutdown.borrow_and_update() {
        if shutdown.changed().await.is_err() {
            break;
        }
    }
}

/// [OPUS-4.8] sq-toze.36 (cert gap GX-13): detect the `--health-probe` subcommand and resolve
/// the address it should probe. Returns `Ok(Some(addr))` when `--health-probe` is present (the
/// caller then runs the probe and exits), `Ok(None)` when it is absent (normal server start),
/// or `Err` on a malformed `--health-probe-addr`.
///
/// Address precedence: `--health-probe-addr HOST:PORT` (highest) > `SPARQ_HEALTH_PROBE_ADDR`
/// env > the [`health_probe::DEFAULT_PROBE_ADDR`] loopback default. Kept separate from the main
/// flag loop so the probe path never touches data-loading / bind / config.
fn parse_health_probe(args: impl Iterator<Item = String>) -> Result<Option<String>, String> {
    use sparq_server::health_probe::DEFAULT_PROBE_ADDR;
    let mut requested = false;
    let mut addr_override: Option<String> = None;
    let mut args = args;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--health-probe" => requested = true,
            "--health-probe-addr" => {
                let v = args
                    .next()
                    .ok_or("--health-probe-addr requires a HOST:PORT value")?;
                if v.is_empty() {
                    return Err("--health-probe-addr must not be empty".into());
                }
                addr_override = Some(v);
            }
            _ => {}
        }
    }
    if !requested {
        return Ok(None);
    }
    let addr = addr_override
        .or_else(|| std::env::var("SPARQ_HEALTH_PROBE_ADDR").ok())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_PROBE_ADDR.to_string());
    Ok(Some(addr))
}

fn parse_flag<T: std::str::FromStr>(
    args: &mut impl Iterator<Item = String>,
    flag: &str,
) -> Result<T, String> {
    let v = args
        .next()
        .ok_or_else(|| format!("{flag} requires a value"))?;
    v.parse()
        .map_err(|_| format!("{flag}: invalid value '{v}'"))
}

/// Resolves on SIGINT (Ctrl-C) or SIGTERM (the signal init systems / container runtimes
/// send), letting `axum::serve` drain in-flight requests before the process exits.
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
    eprintln!("shutting down");
}
