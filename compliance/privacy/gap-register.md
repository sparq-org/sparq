<!-- [OPUS-4.8] sq-toze — Privacy gap register. Honest engine-side gaps + remediation beads.
     Re-review when Fable returns. -->

# Privacy — gap register

Open **engine-side** gaps for the privacy framework — i.e. technical capabilities the engine
*could* provide to make an operator's GDPR/27701/SOC 2 job easier, but does not yet. Each has a
severity, a remediation, and the `bd` bead that tracks it (created under epic **sq-toze** via the
main checkout — never hand-edited into `.beads/`).

> **Scope discipline.** Most privacy "gaps" are **NOT engine gaps** — they are operator
> deployment responsibilities (lawful basis, RoPA, DPA, consent, breach notification, at-rest
> encryption, TLS). Those are *not* listed here as gaps; they are labelled **OPERATOR** in
> [`controls.md`](./controls.md) and detailed in [`../data-flow.md`](../data-flow.md). Listing
> them here would be papering an operator duty as a sparq defect. Only genuine engine-side
> capability gaps appear below.

## OPEN gaps (engine-side)

| ID | Gap | Sev | Remediation | Bead |
|---|---|---|---|---|
| **PR-G1** | **Error bodies echo caller input — including loaded RDF data — to an unauthenticated caller** (corrected per audit F-1/F-2; was wrongly scoped to "the UPDATE path only" and "query text, not loaded data"). Multiple parse/validation 400/500 bodies wrap the underlying parser diagnostic verbatim: `"malformed query: {msg}"` (`http.rs:1812`, **SPARQL query text**); `"malformed RDF body: {e}"` / `"malformed RDF/XML body: {e}"` (`:2293`/`:2302`, **fragments of the loaded RDF data** via `Graph::load_str`→`e.to_string()`, `crates/sparq-core/src/lib.rs:632`); `"update failed: {e}"` (`:2116`); `"query execution error: {msg}"` (`:2860`). Because the bare server has **no auth (boundary B3)**, the RDF-body sites can leak loaded-data fragments to an unauthenticated caller (P-12). | **Medium** (raised from Low — was mis-scoped) | Return a **generic** body by default at all five sites (parity with the panic/budget path), and gate the verbose diagnostic behind `--verbose` (opt-in, like request logging). Add a regression test (PR-G5) asserting the default-mode body contains no echoed query/RDF fragment. | **sq-cz89** (code fix) |
| **PR-G2** | **No built-in *richer* structured, queryable access/audit log.** GDPR Art. 5(2) accountability + SOC 2 monitoring want a demonstrable access trail. The engine emits **no** audit log by default (good for minimisation); `--verbose` produces *unstructured* tower-http trace lines. `sparq-solid` computes allow/deny verdicts but does not persist them. | **Low** | **(a) ADDRESSED (doc + opt-in mechanism):** the **query-log / PII data-minimisation posture** is documented in [`data-minimisation-posture.md`](./data-minimisation-posture.md) (what sparq logs default-vs-`--verbose`-vs-`--audit-log`, verified in-code; the PII vector in the GET query string; operator guidance to keep logs PII-clean). The opt-in **`--audit-log`** structured access record — op class, **non-reversible** query + requester **fingerprint** (FNV-1a, never raw query text/token), decision, status, duration; off by default even when the `audit-log` feature is compiled in — exists (`crates/sparq-server/src/audit.rs`, bead **sq-0bxp**). **(b) DEFERRED (code):** a richer sink capturing `(actor/WebID, target graphs, result-row-count)` and a queryable JSON-lines persistence remain open. | **sq-toze.32** (doc — ADDRESSED) · **sq-0bxp** (opt-in `--audit-log`, landed) · **sq-gos8** (richer sink, deferred) |
| **PR-G5** | **No regression test pins the error-body no-echo property** (audit F-3). P-12 was marked "verified" with no test asserting that default-mode error bodies contain no echoed query/RDF fragment, so a regression could silently re-introduce the leak. | **Low** | Add an HTTP-level regression test: POST a malformed query and a malformed RDF body, assert the default-mode 400 bodies are generic (no echoed input). Pairs with the PR-G1 code fix. | **sq-zg0u** (test) |
| **PR-G3** | **`--persist` WAL is not erasure-complete.** A triple removed by `DELETE`/`DROP` may persist in earlier append-only WAL segments until the operator compacts/rotates the persist directory, so a naive `DELETE` is **not** a complete Art. 17 erasure of the persisted store (P-9). The engine provides no WAL-purge/compaction command, and this caveat is not surfaced to the operator at the point of deletion. | **Medium** | **(a) ADDRESSED (doc, this PR):** the WAL-erasure caveat **and** the manual re-seed/rotate purge procedure are now documented in the [`retention-erasure-runbook.md`](./retention-erasure-runbook.md) (§7a, plus the §9 checklist), alongside the existing caveat in [`../data-flow.md`](../data-flow.md). The *mechanism* exists (`DROP`/`DELETE` + manual re-seed) and the operator is told how + when to use it. **(b) DEFERRED (code):** an optional `compact`/`vacuum` admin command that rewrites the WAL dropping superseded segments remains open (bead **sq-x32t**). A separate opt-in crypto-erase investigation is **sq-du24**. | **sq-toze.33** (doc — ADDRESSED) · **sq-x32t** (compact/vacuum, deferred) · **sq-du24** (crypto-erase, deferred) |
| **PR-G4** | **No request-log redaction control.** When an operator enables `--verbose`, `TraceLayer` logs full request lines including SPARQL query text — which can embed personal data (e.g. `FILTER(?email = "alice@…")`). There is no option to redact/hash query bodies in the log. | **Low** | **ADDRESSED ([OPUS-4.8] sq-toze.34):** request-log redaction is now **ON by default**. With `--verbose`, the logged URI keeps the path but replaces the query string (the GET query-text vector) with a `<redacted len=N fp=…>` length + non-reversible FNV-1a fingerprint; verbatim logging is the deliberate opt-out `--log-full-requests` / `SPARQ_LOG_FULL_REQUESTS=1`. POST bodies were never logged by `TraceLayer`. **Honest boundary:** log-content redaction, not anonymity — method/endpoint/status/size/timing metadata remains. `crates/sparq-server/src/redact.rs`; regression test `crates/sparq-server/tests/log_redaction.rs` (query text absent when redaction on, present when opted out). | **sq-toze.34** (landed) |

## GATED-on-external (honesty contract — not engine gaps to "fix" here)

| ID | Item | Why it is not closeable in this framework | Bead |
|---|---|---|---|
| **PR-X1** | **ZK/MPC "privacy by cryptography" is remediated but NOT externally audited — no production guarantee yet.** <!-- [OPUS-4.8] reconciled with post-remediation re-audit (sq-gbp4); enforcement detail from sq-toze.35 --> The estate models private query proofs / MPC; the ZK verifier was originally found unsound, the `sq-1s2` binding layer has landed, and the internal re-audit (`sq-gbp4`) found it "sound as landed for the assumed threat model" — **but** external accredited-cryptographer sign-off is still PENDING (`sq-qhy4`, P0), `sparq-mpc` provides **no** guarantee, and `SECURITY.md` directs a relying party not to present a "verified" result as a production-grade guarantee until that sign-off completes (`SECURITY.md`; `research/zk-soundness-audit.md`; `research/zk-verifier-reaudit.md`). Any privacy claim resting on it today would be a high-severity overclaim. | The external sign-off + any remaining soundness/privacy work is **cryptographic work** tracked by the ZK remediation beads (epic sq-1s2) + `sq-qhy4` + the `cryptoreview` framework — **not** this privacy framework. This register only records that privacy claims remain **gated** on the external sign-off; we do **not** mark it "closed" or claim any privacy benefit from it. **A CI gate now enforces this honesty in code** (sq-toze.35, ENFORCED): `scripts/check-privacy-claims.sh` (HARD job `privacy-claims` in `.github/workflows/docs-quality.yml`, runs on PRs; reports but is not a required check) fails the build on any unqualified ZK/MPC privacy/soundness claim across the outward doc/README/SKILL/site/compliance surface unless the line carries an inline `privacy-claims-allow: <justification>` marker. | **sq-toze.35** (gate — **ENFORCED in CI**) + **sq-qhy4** (external audit, P0) + epic **sq-1s2** (the crypto fixes) |
| **PR-X2** | **27701 / SOC 2 Privacy certificate** needs the operator's PIMS + an accredited external auditor. | The engine provides the technical levers (controls.md P-17/P-18); the *attestation* is a deployment property an agent cannot issue. Labelled **AUDIT-READY**, not a gap. | n/a (external) |

## NOT gaps (operator responsibilities, recorded so the auditor does not re-flag)

These are **OPERATOR** rows in `controls.md`, NOT engine defects — listed here only to pre-empt
re-flagging:

- **At-rest encryption** of the `--persist` WAL (P-9) — operator's full-disk/volume encryption.
- **TLS / transport encryption** — operator terminates TLS (gateway / reverse proxy).
- **Lawful basis, purpose limitation, consent, privacy notice, RoPA, DPIA sign-off, DPA chain,
  cross-border transfer, breach notification, DPO** — all controller/operator duties (P-1, P-16,
  P-18).
- **Per-user authentication in the core server** — documented boundary **B3** ("front with a
  gateway / `sparq-solid`"), an explicit architectural decision, not a silent gap. The engine
  *does* provide optional Bearer auth (P-10) + `sparq-solid` graph-level authz.

## Honest overall posture

The privacy framework is **substantively AUDIT-READY** with a **respectable engine-side
capability set**: data-minimisation by default (P-2), full SPARQL erasure/rectification/export
(P-3/4/5/6), fail-closed graph-level access control (P-10/11), default-deny SSRF guard (P-13),
and in-browser client-side processing (P-15). **One control was overclaimed and is now
corrected:** error/log hygiene (P-12) is **PARTIAL**, not clean — several parse/validation error
bodies echo caller input, and on the RDF-body path leak *loaded-data fragments* to an
unauthenticated caller (audit F-1 HIGH / F-2). This is the most material open item, now tracked as
**PR-G1 (Medium)** with a code-fix bead (**sq-cz89**) + a regression-test bead (**sq-zg0u**,
PR-G5, closing audit F-3). The remaining gaps are **lower-severity engine conveniences** (PR-G2
audit sink, PR-G3 WAL erasure-completeness, PR-G4 log redaction). None changes the headline
verdict: **the engine *can* support a GDPR-compliant deployment** — but P-12 must be fixed (or its
diagnostics gated behind `--verbose`) before the error-hygiene control can be claimed clean. The
two non-engine items (PR-X1 ZK/MPC remediated-but-externally-unaudited, PR-X2 external certificate) are **gated by design**
and explicitly **not** claimed as privacy guarantees. The framework never asserts "sparq is GDPR
compliant" (a deployment property) — only the technical capabilities, each cited; the one
overclaim (P-12) has been corrected in `controls.md` + `evidence.md`.
