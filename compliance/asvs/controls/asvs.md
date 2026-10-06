<!-- [OPUS-4.8] sq-toze.10 — OWASP ASVS control table (the auditor spine).
     Re-review when Fable returns. Honesty contract: no overclaim; ZK/MPC excluded. -->

# OWASP ASVS L1/L2 — control table

One row per **applicable** requirement, scoped to `sparq-server` + the parsers (see
[`README.md`](./README.md) for chapter applicability and the B3 boundary). Status:
**PASS** (implemented & verified) / **AUDIT-READY** / **GAP** / **N/A (reason)**.
Evidence paths are repo-relative; `crates/<crate>/src/…` enforces, `crates/<crate>/tests/…`
or an in-module `#[test]` regresses, `.github/workflows/<wf>.yml` gates. Full re-run commands
+ line ranges are in [`evidence.md`](./evidence.md); open items are in
[`gap-register.md`](./gap-register.md). Owner = **sparq** unless marked **operator**.

> ASVS requirement IDs reference v4.0.3. Where a chapter is largely N/A, the table records
> the *applicable residual* requirements and the N/A rationale for the rest.

## V1 — Architecture, Design & Threat Modeling

| ASVS | Control | Status | Evidence | Owner |
|---|---|---|---|---|
| V1.1.x | Documented SDLC with security baked in (secure-coding standard, review gates) | **PASS** | `CONTRIBUTING.md#secure-coding` (unsafe policy, input-validation, error/log hygiene, SSDF touchpoints); gate stack in `AGENTS.md`; `.github/workflows/ci.yml` (`lint` clippy `-D warnings`, `test`), `codeql.yml`, `fuzz.yml`, `miri.yml` | sparq |
| V1.1.3 | Threat model exists, covers the boundaries | **PASS** | `research/threat-model.md` (STRIDE, boundaries B1–B5; B3 = no-auth server boundary, B5 = mmap) | sparq |
| V1.2.x | Authentication architecture documented | **PASS (as B3 decision)** | `research/threat-model.md` B3; `crates/sparq-server/src/main.rs` usage banner; `README.md` (this dir) "B3 boundary" | sparq + operator |
| V1.5.x | Input/output trust boundaries defined; untrusted input identified | **PASS** | `CONTRIBUTING.md#secure-coding` ("hardened surfaces": parser/engine/server); threat-model T-PARSE-FUZZ, T-MMAP-FUZZ | sparq |
| V1.14.x | Segregation of components; the unsafe/crypto surfaces isolated | **PASS** | `#![forbid(unsafe_code)]` in 31 crates; ZK/MPC scaffolds isolated + caveated (`SECURITY.md`) | sparq |
| V1.10 | Source control + signed commits / provenance | **AUDIT-READY** | SLSA `attest-build-provenance` on release (`release.yml`); commit-signing is a maintainer/org control | sparq + operator |

## V2 — Authentication (mostly N/A — operator; thin residual is the optional token gate)

| ASVS | Control | Status | Evidence | Owner |
|---|---|---|---|---|
| V2.1–V2.9 (password policy, MFA, recovery, lookup secrets, OOB, OTP, crypto-auth) | Full authentication lifecycle | **N/A (B3: operator gateway)** | sparq has no user accounts / login UI / credential store; identity is fronted by a gateway (`research/threat-model.md` B3). Sparq does **not** implement these. | operator |
| V2.10.1 | Service/secret auth: secrets not in source; transmitted only over a secure channel | **PASS** | Token read from `--auth-token` / `SPARQ_AUTH_TOKEN` (env/flag, never hardcoded): `crates/sparq-server/src/http.rs` env ingestion (`from_env`-style, ~`:339`); bind posture warns to deliver token over TLS (`:427`). No secret literals in source. | sparq |
| V2.10.x | Constant-time verification of the shared secret | **PASS** | `constant_time_eq()` (`crates/sparq-server/src/http.rs:543`); length-check + XOR-accumulate through `core::hint::black_box`. Test `constant_time_eq_matches_plain_eq` (`http.rs:~2979`). | sparq |

## V3 — Session Management

| ASVS | Control | Status | Evidence | Owner |
|---|---|---|---|---|
| V3.* | Session tokens, cookies, fixation, timeout, logout | **N/A (stateless API)** | sparq-server is a stateless HTTP SPARQL-protocol endpoint: no sessions, no cookies, no `Set-Cookie` anywhere in `crates/`. Bearer-token auth (when enabled) is per-request, not a session. | operator |

## V4 — Access Control (mostly N/A; residual = read/write operation classification)

| ASVS | Control | Status | Evidence | Owner |
|---|---|---|---|---|
| V4.1–V4.3 (per-user/RBAC/ABAC, deny-by-default per principal) | Per-user authorization | **N/A (B3: operator gateway)** | sparq has no user identities to authorize. WAC/ABAC is the gateway's job. | operator |
| V4.1.1 (residual) | Access decisions enforced server-side, not client-trusted; mutating ops gated even via a query path | **PASS** | `Operation::{Read,Write}` classified by *mutation*, not route (`crates/sparq-server/src/http.rs:521`): an UPDATE smuggled through the query path is treated as a write and gated. Test `auth.rs` mutation-classification-not-route (`tests/auth.rs:~341`). | sparq |
| V4.1.3 | Principle of least privilege: reads can stay open while writes are gated | **PASS** | Write always gated when a token is set; reads gated only under `--auth-token-read` (`http.rs:605` `auth_check`). Tests in `tests/auth.rs` (write-only token: writes 401, reads pass). | sparq |

## V5 — Validation, Sanitization & Encoding (core)

| ASVS | Control | Status | Evidence | Owner |
|---|---|---|---|---|
| V5.1.3 / V5.1.4 | All input validated; positive validation at the trust boundary | **PASS** | SPARQL parsed by vendored `spargebra` (`crates/sparq-engine/src/lib.rs:379` `PreparedQuery::parse`); RDF by `sparq-core` (`nt.rs`, `Graph::load_str`/`load_reader`). Malformed input is rejected with a typed error, never executed. | sparq |
| V5.1.x | Input-size bound to prevent oversized-payload abuse | **PASS** | `--max-body-bytes` (default 1 MiB) enforced by `DefaultBodyLimit::max` layer (`http.rs:~1325`); oversized → 413. Test `hardening.rs` oversized-body→413 (`tests/hardening.rs:~96`). | sparq |
| V5.2.x | Untrusted data sanitized for the interpreter it reaches | **PASS** | SPARQL is parsed to a typed algebra, never string-concatenated into another query; no SQL/shell/template interpreter is fed user input. Error bodies JSON-escape control chars (`http.rs` `json_error` `:2671`). | sparq |
| V5.2.4 | No eval/dynamic-code on untrusted input | **PASS** | No `eval`/dynamic codegen path; SPARQL executes via a typed planner/executor, all `#![forbid(unsafe_code)]` (parser/planner/executor). | sparq |
| V5.3.x | Output encoding contextual to the sink | **PASS** | Error responses are `application/json` with explicit escaping (`json_error` `http.rs:2688`); result serializers (JSON/XML/CSV) encode per the SPARQL-results spec. No HTML rendered (no XSS sink). | sparq |
| V5.5.x | Deserialization of untrusted data is safe (no unsafe deserialization) | **PASS** | On-disk index byte-parsers are length-validated and fuzzed (`fuzz/fuzz_targets/graph_open.rs`); the mmap reinterpret boundary (B5) is covered by the corruption oracle (`crates/sparq-core/tests/mmap_corruption_oracle.rs`) and `compliance/memsafety/`. | sparq |
| V5.5.2 | Parser resists deeply-nested / recursive input (XXE-class, billion-laughs-class) | **PASS-with-caveat** | Decompression-bomb cap: decompressed output bounded by `min(ratio×len, max_body_bytes)` (`http.rs` `decode_gzip_bounded` `:2546`). SHACL has explicit cycle + recursion guards (`crates/sparq-shacl/src/path.rs:30`, `eval.rs:43`). **Caveat:** SPARQL parse-depth relies on `spargebra` recursive descent + the body-size cap, with no explicit depth constant — tracked **GAP ASVS-G4**. | sparq |

## V6 — Stored Cryptography (narrow)

| ASVS | Control | Status | Evidence | Owner |
|---|---|---|---|---|
| V6.2.x | Constant-time comparison for secret material | **PASS** | `constant_time_eq()` (`http.rs:543`) for the auth token. | sparq |
| V6.* (key management, approved algorithms, at-rest encryption) | Application cryptography | **N/A / operator** | sparq-server stores no secrets at rest and manages no key material; at-rest encryption of the loaded dataset is the operator's storage concern. **ZK/MPC crypto is EXCLUDED** — not a delivered control: remediated but NOT externally audited, external sign-off `sq-qhy4` PENDING, no production guarantee (`SECURITY.md` §"`sparq-zk` and `sparq-zk-compose` — ZK verifier: remediated, but NOT externally audited"; `research/zk-soundness-audit.md`; `research/zk-verifier-reaudit.md`). <!-- [OPUS-4.8] reconciled with post-remediation re-audit (sq-gbp4); see ZK-verdict cross-ref sweep --> | operator |

## V7 — Error Handling & Logging (core)

| ASVS | Control | Status | Evidence | Owner |
|---|---|---|---|---|
| V7.1.1 | No sensitive/internal detail (paths, stack traces) in error responses | **PASS** | `json_error` returns a fixed `{"error":"<msg>"}` with control-char escaping; panics are isolated to a generic "internal server error (panic)". Every error path that could carry caller input / loaded-data fragments / a filesystem path / a `Debug` of an internal type routes through `http.rs::sanitized_error` — full detail to the server log (`target:"sparq_server"`), only a STABLE generic class message in the body (sq-cz89/sq-j9zs guard; [OPUS-4.8] sq-kfel closed the residual GSP-write/TPF/descriptor/compaction/middleware echo paths). **Verified:** `tests/hardening.rs::no_echo_*` + the sq-kfel regression guards (`no_echo_gsp_write_rejection_stays_clean`, `unauthorized_error_is_clean_and_actionable`, `main_failure_classes_are_clean`, `FORBIDDEN_INTERNALS` — asserts no `/home/`,`/Users/`,`/tmp/`… path, no `WriteError::`/`Os { code:` Debug, no secret) + `tests/tpf.rs::malformed_term_does_not_echo_input`. Was **GAP ASVS-G3**, now **Resolved**. | sparq |
| V7.1.2 | Generic error message; no info-leak via differential errors | **PASS** | Auth returns an **identical** 401 for missing vs wrong token (`unauthorized` `http.rs:644`); test `wrong-token 401 indistinguishable` (`tests/auth.rs:~142`). | sparq |
| V7.3.x | Logs do not contain sensitive data (PII, secrets, full query bodies) | **PASS** | Request logging is **opt-in** via `--verbose` `tower_http::TraceLayer` (`http.rs:171`, `:1328`); it logs HTTP method/path/status, **not** request body or full query text. No `info!/warn!/error!` of query text or PII found in server src. Auth tokens are never logged. | sparq |
| V7.4.x | Error handling does not crash the process / leak via unhandled panic | **PASS** | Catch-panic middleware converts a worker panic to a generic 500 without propagating the payload (`http.rs:1309`, `:2240`). Reachable panic on untrusted input is an in-scope DoS bug per `SECURITY.md`; fuzz lane hunts them (`fuzz.yml`). | sparq |

## V8 — Data Protection (narrow)

| ASVS | Control | Status | Evidence | Owner |
|---|---|---|---|---|
| V8.1.x | Sensitive data is not cached/retained beyond need | **PASS / operator** | sparq-server holds the loaded dataset in memory/mmap for serving and does not persist query bodies or results to a side channel; what RDF is loaded (and whether it is personal data) is the **operator's** choice — see `compliance/data-flow.md` (privacy framework). | sparq + operator |
| V8.2.x / V8.3.x | Client-side caching headers; no sensitive data in URLs | **PASS** | Errors set no caching of sensitive content; SPARQL POST carries the query in the body (not the URL); the GET form is the SPARQL-protocol standard query param. | sparq |
| V8.* (field-level encryption, data classification) | Stored data protection | **N/A / operator** | The dataset's classification + encryption is the operator's (the GDPR controller's) responsibility; sparq is the technical means (`compliance/data-flow.md`). | operator |

## V9 — Communication

| ASVS | Control | Status | Evidence | Owner |
|---|---|---|---|---|
| V9.1.x | TLS for all client connections; strong config | **AUDIT-READY (operator)** | sparq-server terminates **no TLS in-process** by design — TLS is terminated at a reverse proxy (`http.rs:427` bind warnings; `main.rs:48`). The non-loopback bind refusal (`bind_posture`) forces the operator to make an explicit choice. The TLS *config* is the operator's; sparq documents the requirement. | operator |
| V9.2.x | Outbound (server-to-server) connections authenticated/restricted | **PASS** | Federation egress is default-deny + IP-filtered (see V12). | sparq |

## V10 — Malicious Code

| ASVS | Control | Status | Evidence | Owner |
|---|---|---|---|---|
| V10.2.x | No malicious/back-doored code; dependency integrity | **PASS** | `cargo deny check advisories bans sources licenses` — all failing steps in `.github/workflows/supply-chain.yml` (push to `main` + nightly; **not** a PR gate — post-merge detection); cargo-vet step present; daily `dependency-monitoring.yml` advisory watchdog; SHA-pinned actions; CodeQL SAST (`codeql.yml`, `security-and-quality`). | sparq |
| V10.3.x | Integrity of the build/deployment pipeline | **PASS / AUDIT-READY** | SLSA build provenance + SHA256SUMS on release (`release.yml`); buildkit `provenance: mode=max`. SLSA *level* is honestly ≈L2 — assessed in `compliance/slsa/`. | sparq |
| V10.3.2 | Application does not execute unverified code / has no time-bomb | **PASS** | No dynamic code loading; no `unsafe` in the parser/executor (`#![forbid(unsafe_code)]`, MS-13 in `compliance/memsafety/`); fuzz + Miri lanes. | sparq |

## V11 — Business Logic (anti-automation / DoS)

| ASVS | Control | Status | Evidence | Owner |
|---|---|---|---|---|
| V11.1.5 | Anti-automation / resource-exhaustion limits | **PASS** | `--max-concurrent` load-shed → 429 (`http.rs:~1315`); `--query-timeout` budget → 503 (`timeout_response` `http.rs:1988`); `--max-results` / `--max-query-rows` → 413 (`engine_error_response` `http.rs:1945`). Tests in `tests/hardening.rs` (timeout→503, load-shed→429, row-cap→413). | sparq |
| V11.1.x | Per-connection resource bounds | **PASS** | `--max-subscriptions` (global) + `--max-subscriptions-per-conn` enforced (`crates/sparq-server/src/subscriptions.rs:104`, `:362`); WS frame cap = `max_body_bytes` (`subscriptions.rs:163`); time-travel age cap `--time-travel-max-age`. | sparq |

## V12 — Files & Resources (SSRF, decompression, file serving)

| ASVS | Control | Status | Evidence | Owner |
|---|---|---|---|---|
| V12.3.x | No untrusted file-path / file-serving sink | **PASS** | sparq-server serves **no static files** (no path-traversal sink); routes are the SPARQL protocol + Graph-Store protocol only (`http.rs` routing, 405 on unlisted methods). | sparq |
| V12.4.x | Files/archives from untrusted sources size-bounded (no zip/decompression bomb) | **PASS** | Decompression cap: decompressed output bounded by `min(ratio×compressed_len, max_body_bytes)`, refuses 413 (`http.rs` `decode_gzip_bounded` `:2546`); `--max-decompress-ratio` knob (`main.rs:145`). | sparq |
| V12.6.1 | **SSRF defense** — server-side requests to untrusted URLs are restricted | **PASS** | SPARQL `SERVICE` federation is **default DENY-ALL**; operator opts hosts in via `--service-allow` / `SPARQ_SERVICE_ALLOW` (`crates/sparq-server/src/service_config.rs`). The engine **also** blocks loopback/private/link-local + cloud-metadata `169.254.169.254` on the **resolved** IP (`crates/sparq-engine/src/service.rs:203` `is_forbidden_ip`; `EgressFilterResolver` enforces on the resolved address — no DNS-rebinding TOCTOU). *Trust model:* an explicitly **allowlisted** host deliberately bypasses `is_forbidden_ip` (an allowlist entry = the operator vouching for that host, even one resolving to a private/metadata IP), so the resolved-IP filter hardens the **not-explicitly-allowlisted** path; the empty-allowlist default denies every host before any dial, so default-deny stays sound. Tests: empty allowlist refuses SERVICE **before any dial** (`tests/service_allowlist.rs:81`); `is_forbidden_ip` unit tests (`service.rs`). | sparq |

## V13 — API & Web Service (core)

| ASVS | Control | Status | Evidence | Owner |
|---|---|---|---|---|
| V13.1.3 | API URLs/methods restricted; unsupported methods rejected | **PASS** | Explicit method routing; non-listed methods → 405 with an `Allow` header (`method_not_allowed` `http.rs:2732`; `/sparql` GET/HEAD/POST; GSP GET/HEAD/PUT/POST/DELETE). | sparq |
| V13.1.x | Content-type enforced; correct on responses | **PASS** | Errors are `Content-Type: application/json` (`http.rs:2688`); result content-negotiation per the SPARQL-protocol Accept header. | sparq |
| V13.2.x | RESTful service authn/authz where applicable | **PASS (optional gate) / operator** | Optional bearer-token gate (V2.10 above); identity-based authz is the operator gateway (B3). WS auth via `Sec-WebSocket-Protocol: bearer.<token>` (`http.rs:587`). | sparq + operator |
| V13.2.3 | Anti-CSRF for state-changing API requests | **N/A (token-based, no ambient cookie auth)** | No cookie/session ambient credential exists, so CSRF does not apply; mutating requests require the bearer token (when enabled), which a cross-site form cannot supply. | sparq |
| V13.4.x | GraphQL/other query-language DoS controls | **PASS (SPARQL analogue)** | The query DoS controls are timeout + row/result caps + concurrency shed (V11). | sparq |

## V14 — Configuration (core)

| ASVS | Control | Status | Evidence | Owner |
|---|---|---|---|---|
| V14.1.x | Secure build; no debug/secret artifacts shipped | **PASS** | Release build via `dist.yml`/`release.yml`; distroless non-root SHA-pinned `Dockerfile` + `docker-smoke` gate; no secrets in image (CIS-Docker assessed in `compliance/cis/`). | sparq |
| V14.2.x | Dependencies current; known-vuln dependencies gated | **PASS** | Dependabot (4 ecosystems); gating cargo-deny advisories (`supply-chain.yml`); daily advisory watchdog. | sparq |
| V14.3.x | No secrets/credentials in source or config; secure defaults | **PASS** | Auth token + service allowlist come from env/flags, never source. **Secure defaults:** loopback bind, deny-all federation, 1 MiB body cap, all DoS caps on. | sparq |
| V14.4.x | Security headers on HTTP responses (nosniff, frame-options, CSP) | **GAP** | No `X-Content-Type-Options: nosniff` / `X-Frame-Options` / CSP / HSTS set anywhere in `crates/`. For a JSON API the residual risk is low (no HTML sink), but ASVS V14.4 expects at least `nosniff`. Tracked **GAP ASVS-G1** + bead. | sparq |
| V14.5.1 | HTTP method/verb tampering rejected | **PASS** | 405 on unlisted methods (V13.1.3). | sparq |
| V14.5.3 | CORS configured to a trusted origin allowlist (not `*`) | **PASS (by absence) / operator** | **No CORS layer** is present (no `Access-Control-Allow-Origin` emitted), so the browser same-origin policy applies and no cross-origin reads are granted. If the operator needs browser cross-origin access, CORS is configured at the fronting gateway. Documented as an explicit decision, **GAP ASVS-G2** if a first-party allowlist is later wanted. | sparq + operator |
| V14.* (machine-discoverable disclosure pointer) | RFC 9116 `security.txt` | **PASS** | `.well-known/security.txt` (RFC 9116; Contact/Expires/Policy/Canonical) — `/.well-known/security.txt`, also referenced from `CONTRIBUTING.md` + `SECURITY.md`. | sparq |

## ASVS-control regression coverage (the "ASVS test job")

The ASVS input-validation / DoS-limit / auth / SSRF / error-hygiene controls above are
**regression-tested** and run in the gating `test` job of `.github/workflows/ci.yml`
(via `cargo nextest`, sharded). The load-bearing suites:

| Control family | Test file | CI |
|---|---|---|
| Auth gating (token, constant-time, 401 parity, op-classification) | `crates/sparq-server/tests/auth.rs`, `tests/subscriptions_auth.rs`, in-module tests `http.rs:~2979` | `ci.yml` `test` (gating) |
| DoS limits (timeout→503, body→413, load-shed→429, row-cap→413) | `crates/sparq-server/tests/hardening.rs` | `ci.yml` `test` (gating) |
| SSRF egress allowlist (deny-before-dial, IP filter) | `crates/sparq-server/tests/service_allowlist.rs`, `crates/sparq-engine/src/service.rs` tests | `ci.yml` `test` (gating) |
| Error hygiene (JSON-escape, structured bodies, SSE error mapping) | `tests/hardening.rs`, `tests/subscriptions_sse.rs`, `http.rs:~3311` | `ci.yml` `test` (gating) |
| Bind posture (remote-refusal names the gate) | in-module tests `http.rs:~2928` | `ci.yml` `test` (gating) |
| Parser robustness (hostile bytes never UB/panic) | `fuzz/fuzz_targets/{parse_sparql,parse_rdf_str,load_reader_parallel,graph_open,validate_shacl}.rs` | `fuzz.yml` (PR smoke + nightly) |

## Summary

- **PASS (implemented & verified):** the V5 input-validation core, V7 error/log hygiene, V11
  DoS limits, V12 SSRF + decompression caps, V13 method/content-type + optional token gate,
  V14 secure defaults + supply-chain + `security.txt`, and the V2.10/V4/V6.2 residual
  controls (constant-time token compare, op-classification gate).
- **AUDIT-READY:** V9 TLS (operator-terminated, documented), V10.3 SLSA-level (assessed in
  `slsa`), V1.10 commit provenance. An external **ASVS L2 verification / penetration test** is
  the residual attestation we cannot self-issue.
- **N/A (operator gateway, B3):** the V2/V3/V4 authentication-session-access-control
  lifecycle, V6/V8 stored-crypto/data-classification — assigned to the operator with
  rationale.
- **GAP:** V14.5.3 first-party CORS allowlist (ASVS-G2, a documented safe-default decision).
  ASVS-G1 (V14.4 security headers, sq-cmvh), ASVS-G3 (V7.1.1 engine-error-string leakage
  verification — [OPUS-4.8] sq-kfel) and ASVS-G4 (V5.5.2 parse-depth bound, sq-53s1) are now
  **Resolved**. All in [`gap-register.md`](./gap-register.md) with beads.
- **EXCLUDED (not a control):** the ZK/MPC estate — **remediated but NOT externally audited**, no production guarantee until external sign-off (`sq-qhy4`), per `SECURITY.md`. <!-- [OPUS-4.8] reconciled with post-remediation re-audit (sq-gbp4); see ZK-verdict cross-ref sweep -->
