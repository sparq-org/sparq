<!-- [OPUS-4.8] sq-toze / sq-v48f — ISO/IEC 27001 operator deployment-security
     responsibilities (gap GAP-ISO-2 / boundary B3). This documents what the ORGANISATION
     DEPLOYING sparq must do. It is NOT a certificate and NOT a guarantee of any sparq
     built-in control. Authored while Fable unavailable — re-review when Fable returns.
     NON-CANONICAL timing (EC2 work box). -->

# ISO/IEC 27001:2022 — operator deployment-security responsibilities (boundary B3)

> **THIS IS AN OPERATOR-RESPONSIBILITY DOC, NOT A CERTIFICATION CLAIM.** It enumerates the
> security controls that fall on the **organisation deploying `sparq-server`** — the
> shared-responsibility boundary the threat model calls **B3**. Nothing here certifies sparq,
> and nothing here implies sparq provides a control it does not. For every responsibility we
> state plainly **what sparq ships built-in** (citing the real flag/feature) versus **what the
> operator MUST supply**. Where sparq does not provide a control, we say so — the onus is on
> the operator. Remediates gap **GAP-ISO-2**.

Read [`README.md`](./README.md) (scope + status-label semantics) and [`controls.md`](./controls.md)
(the Annex A spine) first. The 42 `N/A(operator)` controls in `controls.md` are asserted there
but not made *actionable*; this doc is where they become a concrete operator checklist.

## The shared-responsibility boundary in one paragraph

`sparq-server` (`crates/sparq-server`) is a **W3C SPARQL 1.1 Protocol HTTP endpoint** intended
to be **consumed as a dependency / fronted by the operator's infrastructure**, not exposed
raw to the public internet. By **documented design** (`research/threat-model.md` boundary
**B3** — "front with a gateway / `sparq-solid`"), the server provides **DoS/abuse limits,
hardening response headers, an *optional coarse* Bearer token, and a secure-by-default bind
posture** — but it provides **no per-user authentication, no authorisation model, no TLS
termination, and no identity store.** Those, plus the host/network/operations layer, are the
**operator's** responsibility. This is not a gap to "fix" in sparq; it is the architectural
split. The operator's job is to close it.

> **Crypto honesty gate.** Where this doc mentions cryptography (TLS, tokens, signing), it
> concerns only the crypto sparq **relies on operationally** (operator-terminated TLS, the
> constant-time token compare, Sigstore/SLSA release signing). The **`sparq-zk` /
> `sparq-zk-compose` / `sparq-mpc` estate makes NO production cryptographic guarantee** — the v1
> ZK verifier was **originally found NOT sound** (`research/zk-soundness-audit.md`), but `sq-1s2`
> landed the verifier-side binding layer and an **internal** re-audit
> (`research/zk-verifier-reaudit.md`, `sq-gbp4`) found all findings closed → "sound as landed for
> the assumed threat model"; **external accredited-cryptographer sign-off is STILL PENDING**
> (`sq-qhy4`, P0) and there is **NO production guarantee** (MPC semi-honest-only) — `SECURITY.md`,
> assessed in [`../cryptoreview/`](../cryptoreview/) [OPUS-4.8]. An operator MUST NOT treat any
> ZK/MPC feature as a privacy or integrity control.
> <!-- [OPUS-4.8] reconciled with post-remediation re-audit (sq-gbp4); see ZK-verdict cross-ref sweep -->

## How to read each responsibility

Each section below states:

- **Built-in (sparq):** the concrete flag / env var / feature sparq actually ships, cited to
  the SKILL/source so it is checkable. **Do not infer more than is written.**
- **Operator MUST supply:** the control sparq does **not** provide, which the operator is
  responsible for. Where sparq provides *nothing*, this is the whole control.
- **Annex A:** the ISO/IEC 27001:2022 control(s) this responsibility maps to (the `N/A(op)` /
  `AUDIT-READY` rows in `controls.md`).

---

## 1. Network exposure, TLS termination & reverse proxy

**Annex A:** A.5.14 (information transfer), A.8.20 (networks security), A.8.21 (security of
network services), A.8.22 (segregation of networks), A.8.23 (web filtering).

**Built-in (sparq):**

- **Secure-by-default bind.** The server **binds loopback (`127.0.0.1:3030`) by default and
  *refuses* a non-loopback bind** (e.g. `0.0.0.0`) unless the operator sets `--allow-remote`
  (`SPARQ_ALLOW_REMOTE=1`) **or** the whole surface is fully authenticated (`--auth-token`
  **and** `--auth-token-read`). A write-token alone still requires `--allow-remote` because
  reads stay open. The bind decision is `bind_posture()` / `AuthPosture::from_config()` in
  `crates/sparq-server`.
- **Hardening response headers (always on).** Every response carries
  `Content-Security-Policy: default-src 'none'; frame-ancestors 'none'`,
  `X-Content-Type-Options: nosniff`, and `X-Frame-Options: DENY`, stamped by the `harden()`
  `map_response` layer.
- The published container image (`ghcr.io/sparq-org/sparq-server`) sets `SPARQ_ALLOW_REMOTE=1`
  because a port-mapped `0.0.0.0` bind is needed for Docker — so **running the container is
  the operator's explicit choice to publish a surface, and that surface has NO auth by
  default** (see §2).

**Operator MUST supply:**

- **TLS / transport encryption.** sparq speaks **plaintext HTTP only**; it does **not**
  terminate TLS and **deliberately does not emit `Strict-Transport-Security`** (HSTS belongs
  on the fronting TLS proxy). Bearer tokens and SPARQL payloads are **sniffable on the wire**
  unless the operator terminates TLS at a reverse proxy / load balancer / service mesh in
  front of the server.
- **A reverse proxy / API gateway** for the public edge (recommended deployment per B3). Bind
  sparq to loopback or a private network and let the proxy be the only ingress.
- **Network segmentation / firewalling.** Restrict who can reach the port; place the server on
  a private subnet. sparq has **no** firewall, IP allow-list, or rate-limit-by-source.
- **CORS, if a browser app calls it.** sparq is a no-CORS data API by design (it emits **no**
  `Access-Control-*` headers); a browser origin needing cross-origin access must obtain it at
  the operator's gateway.

---

## 2. Authentication & authorisation (the core B3 control — read carefully)

**Annex A:** A.5.15 (access control), A.5.16 (identity management), A.5.17 (authentication
information), A.5.18 (access rights), A.8.2 (privileged access), A.8.3 (information access
restriction), A.8.5 (secure authentication).

**Built-in (sparq) — and its honest limits:**

- sparq ships **one coarse, optional, static Bearer token**, not an authentication system.
  `--auth-token <TOKEN>` (`SPARQ_AUTH_TOKEN`) requires `Authorization: Bearer <TOKEN>` on
  every **write** (SPARQL Update + Graph Store `PUT`/`POST`/`DELETE`); otherwise `401` +
  `WWW-Authenticate: Bearer`. The token is **constant-time compared**, classified on whether
  the request **mutates** (an Update smuggled through the query path is still gated), and
  **never logged**. `--auth-token-read` (`SPARQ_AUTH_TOKEN_READ=1`) extends the same single
  token to **reads**, including the `/subscriptions` WS and `/subscriptions/sse` SSE
  transports (bead `sq-cxk5` closed the prior read-auth bypass).
- **With no token set, every endpoint is unauthenticated** — the back-compat default. The
  binary logs a loud **no-auth WARNING at startup** (`crates/sparq-server/src/main.rs`).

**The limits, stated plainly (do NOT over-read the above):**

- It is **one shared secret for the whole server**. There is **NO per-user identity, NO
  per-graph / per-dataset / per-query authorisation, NO roles, NO scopes, NO sessions, NO
  MFA, NO token rotation/expiry, NO OAuth/OIDC**. Anyone holding the token has the full
  granted surface (write, or read+write).
- "Missing" vs "wrong" token are **deliberately indistinguishable** (both `401`); this is not
  a user-feedback channel.
- **This is not a substitute for an IdP or an authorisation policy engine.**

**Operator MUST supply:**

- **All per-user / per-role / per-resource authorisation.** Front the server with a reverse
  proxy / API gateway / `sparq-solid` that performs real authentication (OIDC/OAuth/mTLS) and
  enforces an authorisation policy, then talks to a loopback-bound sparq. This is the
  load-bearing operator control under boundary **B3**.
- **Token lifecycle** if relying on the built-in token: generation of a high-entropy secret,
  secure delivery **over TLS** (see §1, §3), rotation, and revocation.
- **The identity store** (A.5.16) and **access-rights provisioning/review** (A.5.18) — sparq
  has none.

---

## 3. Secrets management

**Annex A:** A.5.17 (authentication information), A.8.24 (use of cryptography — operational).

**Built-in (sparq):** the Bearer token is read from the environment (`SPARQ_AUTH_TOKEN` /
`SPARQ_AUTH_TOKEN_READ`) and is **never written to logs or to any HTTP response** (the
audit-log records only an `fnv1a` fingerprint of the token, never the raw value — see §6).

**Operator MUST supply:**

- **Secure provisioning of the token.** Inject it via a secrets manager / orchestrator secret
  (e.g. Kubernetes `Secret`, Docker secret, Vault), **not** a committed file or a shell
  history. Avoid passing it as a process argument where it is visible in `ps`; prefer the env
  var sourced from a secret.
- **TLS in transit** so the token is not sniffable (see §1).
- **Rotation & revocation** policy and machinery — sparq has no rotation primitive.
- **Any other deployment secrets** (proxy certs, downstream credentials) are entirely the
  operator's.

---

## 4. OS / container hardening

**Annex A:** A.8.1 (endpoint devices), A.8.7 (protection against malware), A.8.18 (privileged
utility programs), A.8.19 (installation of software on operational systems).

**Built-in (sparq) — the *image* is hardened; the *runtime flags* are the operator's:**

- The published image is **distroless, non-root, digest-pinned**: runtime stage
  `gcr.io/distroless/cc-debian12:nonroot` runs as **UID 65532**, with **no shell, no package
  manager, no setuid utilities**, only the single server binary (`Dockerfile`; verified in
  [`../cis/controls.md`](../cis/controls.md) D-4.1/4.2/4.3/4.8).

**Operator MUST supply (set at `docker run` / in the orchestrator, NOT in the image —
[`../cis/controls.md`](../cis/controls.md) row D-5.x):**

- `--read-only` root filesystem (mount only what must be writable, e.g. a `--persist` volume).
- `--cap-drop=ALL` (the server needs no Linux capabilities).
- `--security-opt=no-new-privileges`.
- A non-root `--user` (the image already defaults to 65532; do not override with `USER root`).
- `--pids-limit`, memory/CPU `--memory`/`--cpus` quotas, and a seccomp profile.
- **Mount loaded datasets read-only** (`-v "$PWD/data:/data:ro"`) unless using `--persist`.
- Patch/rebuild the host and base image on the operator's cadence (see §7).

---

## 5. Resource & DoS limits (operator-tuned)

**Annex A:** A.8.6 (capacity management), A.8.14 (redundancy / availability).

**Built-in (sparq) — primitives the operator MUST tune for their dataset/exposure:**

sparq ships four hardening limits, but **two are unbounded by default** and several are
*cooperative/coarse* — an operator exposing a surface **must** set them. From the hardening
flag table (`crates/sparq-server`, `ServerConfig`):

- `--query-timeout` (`SPARQ_QUERY_TIMEOUT`, default `30s`) — per-request wall-clock cap →
  `503`; **cooperative** (next coarse check + a `timeout + 2s` hard grace), applies to all
  forms including Update.
- `--max-body-bytes` (`SPARQ_MAX_BODY_BYTES`, default `1 MiB`) → `413`; byte-hard.
- `--max-concurrent` (`SPARQ_MAX_CONCURRENT`, default `32`) — in-flight cap, load-shed →
  `429`.
- `--max-decompress-ratio` (`SPARQ_MAX_DECOMPRESS_RATIO`, default `20`) — **zip-bomb guard**
  on `Content-Encoding: gzip` bodies → `413`; byte-hard.
- `--max-results` / `--max-query-rows` — **default OFF (unlimited).** `--max-query-rows` is a
  **coarse cardinality (row-count) OOM circuit-breaker**, *not* an RSS quota: peak heap ≈
  `rows × per-row term cost`, so wide rows / huge literals / dictionary growth can still
  exceed the implied memory. An operator exposing the surface **should set these**.
- `--max-subscriptions` / `--max-subscriptions-per-conn` (defaults `256` / `16`) for the
  WS/SSE surface.

**Operator MUST supply:**

- **Tuning** of the above to the operator's dataset size, query mix, and exposure — the
  defaults are a starting point, not a guarantee. In particular **set `--max-query-rows`**
  (and an OS/cgroup memory limit, §4) because the row cap is a cardinality ceiling, not a hard
  memory bound.
- **OS-level enforcement** of memory/CPU (cgroups, the `--memory`/`--cpus` of §4) as the hard
  backstop the cooperative caps cannot give.
- **Availability / redundancy / autoscaling / rate-limiting-by-source** — sparq is a single
  process with no built-in HA or per-client rate limit.
- **SERVICE (federation) egress allow-listing**, if the `service` feature is compiled in:
  `--service-allow` (`SPARQ_SERVICE_ALLOW`) defaults to **deny ALL** SERVICE egress; the
  operator must explicitly allow-list each outbound host (SSRF control).

---

## 6. Logging, monitoring & PII / redaction posture

**Annex A:** A.8.12 (data leakage prevention), A.8.15 (logging), A.8.16 (monitoring
activities), A.5.34 (privacy & PII).

**Built-in (sparq):**

- **Error-body sanitisation** at the HTTP boundary: caller input, **loaded RDF data**, and
  filesystem paths are **not echoed** into error responses (PR #241 — the
  unauthenticated error-body information-leak fix, with no-echo regression tests). This is the
  one no-info-leak control sparq enforces itself.
- **Optional request log:** `--verbose` enables a `TraceLayer` request log (respects
  `RUST_LOG`).
- **Optional per-query access audit log** (doubly opt-in: `audit-log` cargo feature **and**
  `--audit-log` / `SPARQ_AUDIT_LOG=1`; bead `sq-0bxp`). It emits one structured `tracing`
  event per request under `target: "sparq_server::audit"` and deliberately logs **no raw
  query text and no raw token** — only a non-reversible **fingerprint** (FNV-1a) of each, plus
  `requester` / `op` / `decision` / `reason` / `status` / `rows` / `duration_us`. It is a
  **server-side** log under the operator's control and is **never** written to the HTTP
  response. This supports a per-subject/per-query trail (ISO 27001 A.8.15, EU CRA logging,
  CDMC CD-2).
- **Aggregate `/metrics`** (Prometheus text exposition) — counts/histograms only, no
  per-query content.

**Operator MUST supply:**

- **Log collection, retention, rotation, integrity, and access control.** sparq emits events;
  it does **not** ship a log store, retention policy, or tamper-evidence. Route
  `sparq_server::audit` / `--verbose` output to the operator's sink (`RUST_LOG`).
- **PII governance of loaded RDF.** sparq processes **no PII of its own** — the operator is
  the **data controller** for whatever RDF it loads, and is responsible for classification,
  minimisation, masking, retention, and subject-rights handling. Even though the audit log
  fingerprints query text, **raw SPARQL the operator chooses to log elsewhere (e.g. a
  permissive `--verbose` + custom layer) can disclose loaded-data fragments or caller PII** —
  this is the #241 lesson; the operator must not re-introduce the leak in its own logging.
  See [`../data-flow.md`](../data-flow.md) and [`../dpia.md`](../dpia.md) (privacy worktree).
- **Runtime monitoring & alerting** (scraping `/metrics`, alerting on `429`/`503`/`413`
  spikes, intrusion detection) — sparq exposes the signal; the operator runs the monitoring
  estate (A.8.16 runtime facet).

---

## 7. Backup / restore, durability & WAL operational practice

**Annex A:** A.8.13 (information backup), A.5.33 (protection of records), A.5.29 (security
during disruption), A.5.30 (ICT readiness for BC).

**Built-in (sparq):**

- **Durable persistence** with `--persist DIR`: updates are **WAL-fsync'd** and survive a
  restart with no rebuild. In-memory is the default (no on-disk state, lost on exit).

**Operator MUST supply:**

- **Backup and restore of the `--persist` directory** (and any source dataset), on the
  operator's schedule, with off-host copies and **periodically tested restores**. sparq has
  **no** backup scheduler, snapshot tooling, or off-host replication.
- **Durability validation in the operator's environment** — `fsync` guarantees depend on the
  underlying storage honouring it (beware write-back caches / network filesystems).
- **Business-continuity / disaster-recovery** of the running service — out of sparq's scope
  entirely.
- **At-rest protection.** The `--persist` WAL/data and any mmap'd files are **plaintext on
  disk** (`research/threat-model.md` boundary B5 / [`../data-flow.md`](../data-flow.md)); the
  operator owns disk encryption and filesystem access control.

---

## 8. Update / patch cadence

**Annex A:** A.8.8 (technical vulnerability management), A.8.19 (installation of software),
A.8.32 (change management).

**Built-in (sparq):** sparq's own supply chain is gated and monitored — `cargo deny check
advisories` (gating), a daily RustSec advisory watchdog, Dependabot (4 ecosystems), CodeQL,
and a coordinated-disclosure intake (`SECURITY.md`, `.well-known/security.txt`). Releases are
SLSA-attested with a CycloneDX SBOM + VEX. (See `controls.md` A.8.8 / `../sbom/`, `../slsa/`,
`../cra/`.)

**Operator MUST supply:**

- **A patch cadence for the deployed artifact:** watch sparq releases / GHSA advisories,
  consume the per-release **SBOM + VEX** to triage exposure, and **rebuild/redeploy** the
  pinned image on a defined schedule. sparq ships the *evidence*; the operator owns the
  *deploy decision* and the support-window tracking (relevant to EU CRA security-update
  obligations — [`../cra/`](../cra/)).
- **Host/base-image patching** (see §4) and the operator's own change-management process.

---

## 9. Cross-reference — which Annex A controls this doc makes actionable

This doc is the concrete remediation of **GAP-ISO-2**: it turns the `N/A(operator)` and the
operator-facet `AUDIT-READY` rows of [`controls.md`](./controls.md) into an actionable
checklist. The mapping:

| This doc § | Annex A controls made actionable | Primary sparq built-in cited | Primary operator obligation |
|---|---|---|---|
| §1 Network/TLS | A.5.14, A.8.20–A.8.23 | secure-by-default bind / hardening headers | TLS termination + gateway + segmentation |
| §2 AuthN/AuthZ (B3) | A.5.15–A.5.18, A.8.2/A.8.3/A.8.5 | optional coarse Bearer token | real IdP + per-user authz at a gateway |
| §3 Secrets | A.5.17, A.8.24(op) | token from env, never logged | secrets manager + rotation + TLS delivery |
| §4 OS/container | A.8.1/A.8.7/A.8.18/A.8.19 | distroless non-root pinned image | `--read-only`/`--cap-drop`/`no-new-privileges`/quotas |
| §5 Resource/DoS | A.8.6, A.8.14 | 4 hardening limits + SERVICE deny-all | tune limits + cgroup memory + HA |
| §6 Logging/PII | A.8.12, A.8.15, A.8.16, A.5.34 | #241 error sanitisation + opt-in audit log | log store/retention + PII governance + monitoring |
| §7 Backup/durability | A.8.13, A.5.33, A.5.29/30 | `--persist` WAL-fsync | backup/restore + at-rest encryption + BC/DR |
| §8 Patch cadence | A.8.8, A.8.19, A.8.32 | gated supply chain + SBOM/VEX | rebuild/redeploy cadence + host patch |

## What this doc does NOT claim

- It does **not** claim sparq is ISO/IEC 27001 certified — a certificate is an accredited-body
  act over an operating ISMS (see [`README.md`](./README.md), `gap-register.md` GAP-ISO-1).
- It does **not** claim the built-in Bearer token is an authentication or authorisation
  *system* — it is one coarse shared secret with the limits stated in §2.
- It does **not** claim any `sparq-zk` / `sparq-zk-compose` / `sparq-mpc` feature provides a
  privacy or integrity guarantee — the v1 ZK verifier was **originally found NOT sound**
  (`research/zk-soundness-audit.md`), and although `sq-1s2` landed the binding layer and an
  **internal** re-audit (`research/zk-verifier-reaudit.md`, `sq-gbp4`) found all findings closed
  ("sound as landed for the assumed threat model"), **external sign-off is STILL PENDING**
  (`sq-qhy4`, P0) and there is **NO production guarantee** (`SECURITY.md`) [OPUS-4.8].
- It does **not** transfer the operator's data-controller responsibility for loaded RDF onto
  sparq — that stays with the deploying operator ([`../data-flow.md`](../data-flow.md),
  [`../dpia.md`](../dpia.md)).
