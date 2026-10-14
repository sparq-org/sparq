<!-- [OPUS-4.8] sq-toze.10 — ASVS evidence pack: re-runnable verification for each
     PASS claim in controls/asvs.md. Line numbers drift; the grep/test anchors don't. -->

# OWASP ASVS — evidence pack

Per-claim verification for the **PASS** rows in [`controls/asvs.md`](./controls/asvs.md).
Each item gives a **re-runnable anchor** (a `grep`, a test name, or a CI job) rather than a
bare line number, because line numbers drift; the anchors are stable. Run all commands from
the **worktree root**. NON-CANONICAL timing — no measured numbers are baked here.

## How to re-verify the whole slice in one pass

```sh
# 1. The ASVS regression suites all run in the gating `test` job:
cargo nextest run -p sparq-server -p sparq-engine \
  --test auth --test hardening --test service_allowlist \
  --test subscriptions_auth --test subscriptions_sse
cargo test -p sparq-server --lib            # in-module http.rs unit tests
cargo test -p sparq-engine service          # is_forbidden_ip / egress policy tests

# 2. The lint + SAST + supply-chain gates:
cargo clippy --workspace --all-targets -- -D warnings
cargo deny check advisories bans sources licenses
```

---

## V1 — Architecture & secure-coding standard

- **Secure-coding standard exists.** `CONTRIBUTING.md`, section "Secure coding"
  (`grep -n "## Secure coding" CONTRIBUTING.md`) — covers the `unsafe` policy, input
  validation on the hardened surfaces, error/log hygiene (explicitly "do not leak RDF/SPARQL
  content, internal paths, or stack detail … (ASVS V7)"), supply chain, and the SSDF
  touchpoints. Closes GX-6 (bead `sq-toze.7` ✓).
- **Threat model.** `research/threat-model.md` (STRIDE; boundaries B1–B5). B3 = the
  no-auth-by-default server boundary; B5 = the mmap unsafe surface.
- **Component isolation.** `grep -rl "forbid(unsafe_code)" crates/ | wc -l` → the unsafe
  surface is confined to a handful of crates; the rest are `#![forbid(unsafe_code)]` (exact
  count is the live grep; the per-site register is `compliance/memsafety/unsafe-register.md`).

## V2.10 / V6.2 — Constant-time secret comparison + secrets not in source

- **Constant-time compare.** `grep -n "fn constant_time_eq" crates/sparq-server/src/http.rs`
  — length-check + XOR-accumulate routed through `core::hint::black_box`. Test:
  `cargo test -p sparq-server --lib constant_time_eq_matches_plain_eq`
  (`grep -n "fn constant_time_eq_matches_plain_eq" crates/sparq-server/src/http.rs`).
- **Token from env/flag, never source.** `grep -n "SPARQ_AUTH_TOKEN" crates/sparq-server/src/http.rs`
  (env ingestion) and `crates/sparq-server/src/main.rs` (`--auth-token` flag). No token
  literal in source: `grep -rn "auth_token" crates/sparq-server/src | grep -i '"'` returns no
  hardcoded secret.

## V4 — Access-control residual (operation classification, least privilege)

- **Mutation classified by effect, not route.**
  `grep -n "enum Operation\|Operation::Write\|Operation::Read" crates/sparq-server/src/http.rs`
  — an UPDATE on the query path is a write. Test: the mutation-classification-not-route case
  in `crates/sparq-server/tests/auth.rs`
  (`grep -n "classif\|mutation\|not.*route\|update" crates/sparq-server/tests/auth.rs`).
- **Least privilege (reads open, writes gated).** `grep -n "fn auth_check" crates/sparq-server/src/http.rs`;
  integration coverage in `tests/auth.rs` (write-only token → writes 401, reads pass; read-gate
  flag → reads gated too).
- **401 parity (no differential info-leak).** `grep -n "fn unauthorized" crates/sparq-server/src/http.rs`
  (identical 401 + `WWW-Authenticate: Bearer`). Test: the wrong-token-indistinguishable case
  in `tests/auth.rs`.

## V5 — Input validation & parser robustness

- **SPARQL parse seam.** `grep -n "PreparedQuery::parse\|fn parse" crates/sparq-engine/src/lib.rs`
  — vendored `spargebra` (`vendor/spargebra`); malformed input → typed error, never executed.
- **RDF parse seams.** `crates/sparq-core/src/nt.rs` (N-Triples/N-Quads, byte-level) and
  `Graph::load_str` / `load_reader` / `load_reader_parallel`
  (`grep -n "pub fn load_str\|pub fn load_reader" crates/sparq-core/src`).
- **Body-size cap (1 MiB default).** `grep -n "max_body_bytes\|DefaultBodyLimit" crates/sparq-server/src/http.rs`.
  Test: oversized-body→413 in `crates/sparq-server/tests/hardening.rs`
  (`grep -n "413\|body" crates/sparq-server/tests/hardening.rs`).
- **Decompression-bomb cap.** `grep -n "decode_gzip_bounded\|max_decompress" crates/sparq-server/src/http.rs`
  — decompressed output bounded by `min(ratio×len, max_body_bytes)`, refuses 413.
- **Error-body escaping (V5.3).** `grep -n "fn json_error" crates/sparq-server/src/http.rs`
  — control chars `\u00xx`-escaped. Test asserting control-char escape:
  `cargo test -p sparq-server --lib` (the JSON-escape unit test in `http.rs`).
- **SHACL cycle/recursion guards.** `grep -n "seen\|recursion\|cycle" crates/sparq-shacl/src/path.rs crates/sparq-shacl/src/eval.rs`.
- **Parser fuzzing (hostile bytes never UB/panic).** `ls fuzz/fuzz_targets/` →
  `parse_sparql.rs`, `parse_rdf_str.rs`, `load_reader_parallel.rs`, `graph_open.rs`,
  `validate_shacl.rs`. CI: `.github/workflows/fuzz.yml` (PR smoke + nightly;
  `cargo fuzz list` auto-enumerates).

## V7 — Error handling & logging

- **Structured, escaped error envelope.** `grep -n "fn json_error\b" crates/sparq-server/src/http.rs`
  (fixed `{"error":...}`); `json_error_bodies` middleware rewrites plain-text errors into the
  same shape (`grep -n "json_error_bodies" crates/sparq-server/src/http.rs`).
- **Panic isolation.** `grep -n "panic" crates/sparq-server/src/http.rs` → catch-panic
  middleware returns a generic 500, never the panic payload.
- **Logging is opt-in and body-free.** `grep -n "TraceLayer\|verbose" crates/sparq-server/src/http.rs crates/sparq-server/src/main.rs`
  — `tower_http::trace::TraceLayer` behind `--verbose`; logs method/path/status, not body or
  query text. No `info!/warn!/error!` of query text or PII: `grep -rn "info!\|warn!\|error!" crates/sparq-server/src | grep -i "query\|token"` returns nothing leaking.
- **No internal/sensitive detail in error bodies (ASVS-G3 — Resolved, [OPUS-4.8] `sq-kfel`,
  building on `sq-cz89`/`sq-j9zs`).** Every error path that could carry caller input, a
  loaded-data fragment, a filesystem path or a `Debug` of an internal type routes through
  `sanitized_error` — `grep -n "fn sanitized_error" crates/sparq-server/src/http.rs` (full
  detail to the server log under `target:"sparq_server"`, only a stable generic class message
  in the body). sq-kfel closed the residual raw-echo paths (GSP-write minted-update rejection,
  TPF term parse, descriptor-serialize 500, `--persist` compaction failure, the unreachable
  middleware-error fallback). **Verified** by `cargo test -p sparq-server --test hardening`
  (`no_echo_*`, `no_echo_gsp_write_rejection_stays_clean`,
  `unauthorized_error_is_clean_and_actionable`, `not_found_error_is_clean`,
  `main_failure_classes_are_clean` — `FORBIDDEN_INTERNALS` asserts no `/home/`,`/Users/`,
  `/tmp/`… path, no `WriteError::`/`Os { code:` Debug, no secret) and
  `--test tpf malformed_term_does_not_echo_input`.

## V8 — Data protection

- **No PII collection / no result side-channel.** sparq-server holds the loaded dataset for
  serving; it does not persist query bodies/results elsewhere. Dataset classification is the
  operator's — `compliance/data-flow.md` (privacy framework).

## V9 — Communication (operator-terminated TLS)

- **No in-process TLS; bind posture forces an explicit choice.**
  `grep -n "fn bind_posture\|allow.remote\|terminate.*proxy\|TLS" crates/sparq-server/src/http.rs crates/sparq-server/src/main.rs`
  — non-loopback bind is **refused** unless `--allow-remote`/`SPARQ_ALLOW_REMOTE` or full auth.
  Test: the remote-refusal message names the gate (`grep -n "allow-remote\|auth-token-read" crates/sparq-server/src/http.rs` in-module tests).

## V10 — Malicious code & dependency integrity

- **Gating cargo-deny (advisories included).**
  `grep -n "GATING\|cargo deny" .github/workflows/supply-chain.yml` — `cargo deny check
  advisories bans sources licenses`, no `continue-on-error`. (GX-1 resolved.)
- **CodeQL SAST.** `.github/workflows/codeql.yml` — `queries: security-and-quality`, `rust`
  language, push/PR/schedule.
- **Daily advisory watchdog + Dependabot.** `.github/workflows/dependency-monitoring.yml`;
  Dependabot config (4 ecosystems).
- **Parser safe-only; executor unsafe confined to cancellation.** `compliance/memsafety/` MS-13;
  `sparq-engine` has four registered atomic-cancellation pointer sites whose lifetime is bound by
  the query-budget guard, while `sparq-parse` and `sparq-shacl` remain `forbid(unsafe_code)`.

## V11 — DoS / anti-automation limits

- **All caps in one place.** `grep -n "max_concurrent\|query_timeout\|max_results\|max_query_rows\|max_subscriptions" crates/sparq-server/src/http.rs crates/sparq-server/src/main.rs`.
- **Tests.** `crates/sparq-server/tests/hardening.rs` — timeout→503, load-shed→429,
  row-cap→413 (`grep -n "503\|429\|413" crates/sparq-server/tests/hardening.rs`).
- **Subscription caps.** `grep -n "max_subscriptions\|per_conn\|frame" crates/sparq-server/src/subscriptions.rs`.

## V12 — Files & resources (SSRF)

- **Default-deny federation allowlist.** `crates/sparq-server/src/service_config.rs`
  (`grep -n "is_empty\|deny\|allow" crates/sparq-server/src/service_config.rs`) — empty
  allowlist = deny-all; sources `--service-allow` / `SPARQ_SERVICE_ALLOW` / `--service-allow-file`.
- **Resolved-IP filter (loopback/private/link-local/metadata).**
  `grep -n "fn is_forbidden_ip\|169.254.169.254\|EgressFilterResolver" crates/sparq-core/src/net.rs crates/sparq-engine-service/src/service.rs`
  — enforced on the **resolved** address (no DNS-rebinding TOCTOU).
- **Tests.** `crates/sparq-server/tests/service_allowlist.rs` (empty allowlist refuses SERVICE
  **before any dial**); `is_forbidden_ip` unit tests in `crates/sparq-core/src/net.rs`.
- **No static-file serving sink.** `grep -rn "ServeDir\|ServeFile\|fs::read.*req\|\.\./" crates/sparq-server/src` returns no path-traversal sink; routes are SPARQL + Graph-Store protocol only.

## V13 — API & web service

- **Method restriction (405 + `Allow`).** `grep -n "fn method_not_allowed\|405\|Allow" crates/sparq-server/src/http.rs`.
- **Error content-type.** `grep -n "application/json" crates/sparq-server/src/http.rs`.
- **WS auth via subprotocol.** `grep -n "Sec-WebSocket-Protocol\|bearer" crates/sparq-server/src/http.rs`.

## V14 — Configuration

- **Secure defaults.** Loopback bind `127.0.0.1:3030` (`grep -n "127.0.0.1:3030" crates/sparq-server/src/main.rs`),
  deny-all federation, 1 MiB body cap, DoS caps on.
- **Distroless non-root image + smoke gate.** `Dockerfile`; `.github/workflows/ci.yml`
  `docker-smoke` job (CIS-Docker depth in `compliance/cis/`).
- **`.well-known/security.txt` (RFC 9116).** `cat .well-known/security.txt` — Contact/Expires/
  Policy/Canonical; referenced from `CONTRIBUTING.md` + `SECURITY.md`. (GX-3 ✓.)
- **No CORS / no security headers (the two GAPs).**
  `grep -rn "CorsLayer\|Access-Control-Allow\|nosniff\|Content-Security-Policy\|X-Frame-Options" crates/`
  returns **nothing** — V14.4 (ASVS-G1, bead `sq-cmvh`) and V14.5.3 (ASVS-G2, bead `sq-o7o0`).
  (Note: the header-specific `Access-Control-Allow` is the load-bearing token — a bare
  `Access-Control` over-matches the Solid WAC English phrase "access control" with ~18
  non-header hits in `sparq-solid`, none of them an emitted HTTP response header.)

---

## Honesty notes for the auditor

1. **Line numbers are approximate** in `controls/asvs.md` (marked with `~`); the **grep
   anchors here are the canonical pointer**. Verified live: `constant_time_eq` and
   `is_forbidden_ip` exist; `CorsLayer`/`nosniff`/`Set-Cookie`/`Access-Control-Allow` (the
   header-specific token — *not* bare `Access-Control`, which over-matches the Solid WAC
   English phrase) grep to **zero** hits in `crates/` (the basis for the V3 N/A, V14.4 GAP,
   and V14.5.3 documented-decision).
2. **The ZK/MPC estate is excluded** from every claim. No V6/V8 row leans on it. <!-- [OPUS-4.8] reconciled with post-remediation re-audit (sq-gbp4); see ZK-verdict cross-ref sweep --> Per
   `SECURITY.md` + `research/zk-soundness-audit.md` + `research/zk-verifier-reaudit.md`, the ZK
   verifier is **remediated but NOT externally audited** (originally unsound; `sq-1s2` landed;
   internal re-audit "sound as landed for the assumed threat model"; external sign-off `sq-qhy4`
   PENDING; no production guarantee). [OPUS-4.8]
3. **`sparq-serve/tests/tokens.rs` is NOT an auth test** — it covers read-your-writes
   *generation* tokens (consistency). It is deliberately **not** cited for V2/V13 auth.
4. **External attestation residual:** an accredited ASVS L2 verification / pen-test of a
   deployed instance is **AUDIT-READY**, not self-issued.
