<!-- [OPUS-4.8] CDMC gap register — epic sq-toze, branch cert-cdmc. -->
# CDMC gap register — sparq

> 🤖 SPARQ agent. Open CDMC capability gaps with severity, remediation, target maturity, and the
> tracking **bead intent** under epic `sq-toze`. **Bead-creation note:** the `bd` CLI (Dolt-backed)
> is **not on PATH in the cert worktree**, so these beads could not be created in-session. The lead
> must run `cd <main checkout> && bd create … --epic sq-toze` for each, then re-export. Each row
> below is a ready-to-file bead spec. (Matches the orchestration contract: subagents capture
> discovered work as beads via the main checkout; orchestrator re-exports.)

Severity — **P0** unblocks a CDMC key-control theme · **P1** raises a capability one level · **P2**
polish. None of these are *security* gaps (sparq's security posture is 4 and covered by the other
framework slices); they are **data-management-maturity** gaps specific to CDMC.

| ID | Gap | Cap (cur→tgt) | Sev | Remediation (see `recommendations.md`) | Bead intent (epic sq-toze) |
|---|---|---|---|---|---|
| CD-1 | **No first-class data lineage** — _**PARTIALLY ADDRESSED**_ (sq-ntcg, branch `feat-prov-lineage`). The opt-in `sparq-prov` crate now emits **W3C PROV-O** for the **CONSTRUCT/DESCRIBE** derivation path: a time-stamped `prov:Activity` (`prov:startedAtTime`/`endedAtTime`), a result `prov:Entity`, and `prov:wasGeneratedBy`/`prov:used`/`prov:wasDerivedFrom` edges to the configured inputs (`Derivation::prov_graph()` / `prov_ntriples()`). Opt-in (publish=false standalone member; zero core overhead when absent). **Still open:** SPARQL UPDATE (`INSERT…WHERE`) lineage + reasoner-materialization PROV (the latter would reuse `sparq-reason`'s `why()` proof trees, already finer-grained); VoID/SD surfacing. | 6.2 (2→**~3**) | P0 | ✅ CONSTRUCT path landed (`crates/sparq-prov`, skill `prov-lineage`). Follow-ups: UPDATE-path + reasoner-materialization PROV (deferred beads), then VoID/SD surfacing. | `CDMC: W3C-PROV data-lineage capture (ingest→graph→result)` — CONSTRUCT slice **done** (sq-ntcg) |
| CD-2 | **ADDRESSED (`sq-0bxp`, branch `feat-access-audit-log`).** ~~No per-query access audit trail.~~ An **opt-in per-query access audit log** now ships on `sparq-server` (cargo feature `audit-log` + runtime `--audit-log` / `SPARQ_AUDIT_LOG=1`): one structured `tracing` event per query/update/Graph-Store request under target `sparq_server::audit` — requester identity (a Bearer-token fingerprint or `anonymous`, never the secret), operation class, a non-reversible query fingerprint (not the full query text — the #241 info-leak posture), the access decision (allowed/denied + reason), the HTTP status / row count, and duration. Off ⇒ zero overhead. **EXTENDED (`sq-gos8`, branch `feat-access-audit-sink`, epic sq-toze):** a RICHER STRUCTURED sink (cargo feature `access-audit` + runtime `--access-audit <file\|stderr>` / `SPARQ_ACCESS_AUDIT`) now emits each ENFORCED decision as a TYPED JSON-Lines record through a pluggable `AuditSink` trait — **actor** (WebID / agent IRI when known, else a token fingerprint), **action**, **resource** (the named-graph IRI / dataset touched, the "graphs-touched" the rec asked for), **decision + policy-basis**, timestamp + a non-reversible request fingerprint (the same FNV-1a construction). The default sink writes a file / stderr; heavy/external sinks (SIEM, OTel) stay out of core (an embedder implements the trait). See `crates/sparq-server/src/access_audit.rs` + `tests/access_audit.rs` + `skills/http-server/SKILL.md` → "Structured access-audit sink". **Re-score 3.2 (2→3).** | 3.2 (2→**3**) | P0 | ~~Opt-in structured access log~~ — **landed (`sq-0bxp` flat trail + `sq-gos8` structured sink incl. actor/resource/decision-basis).** Remaining follow-up: WAC-session WebID enrichment once `sparq-solid` fronts the server (the `Actor::WebId` variant exists for that seam); engine-internal per-graph READ trace. | `CDMC: opt-in structured access audit log (identity + graphs touched)` — **done (sq-0bxp, sq-gos8)** |
| CD-3 | **Classification taxonomy + encryption-at-rest are silently operator-owned.** No documented handoff; mmap files + HTTP are plaintext. | 2.2/4.3 (2/3) | P1 | Deployment guide: SHACL sensitivity shapes → WAC/ACP wiring + operator KMS/TLS-gateway pattern; explicit boundary doc. | `CDMC: classification + encryption-at-rest operator deployment guide` |
| CD-4 | **No usage control (ODRL).** Entitlements are access-control only; no purpose/recipient/duty enforcement. | 3.1 (3→4) | P1 | Land the single-node ODRL gate over `sparq-solid` (the buildable-today slice from the ODRL research record). | `CDMC: ODRL single-node usage-control gate over sparq-solid` |
| CD-5 | **Retention is possible but not policy-driven.** Generation-ring ages out by age/count, not by declarative per-graph retention rule. | 5.1 (3→4) | P1 | Bind `TimeTravelConfig`/`time-travel-max-age` to declarative per-graph TTL → automatic DROP/age-out; document operator retention policy. | `CDMC: declarative per-graph retention policy mechanism` |
| CD-6 | **No machine-readable dataset-ownership convention.** Ownership decision has no recorded home in the loaded graph. | 1.2 (2→3) | P2 | VoID/DCAT dataset header convention (`dcterms:publisher`, `dcat:contactPoint`, sensitivity) surfaced by introspect/descriptors. | `CDMC: VoID/DCAT dataset-ownership header convention + surfacing` |
| CD-7 | **No published CDMC operator-responsibility split.** Deploying teams may assume the engine satisfies governance controls it delegates. | 1.1 / cross-cutting | P2 | Short deployment doc condensing this scorecard's operator-owned column. | `CDMC: publish operator-responsibility split deployment doc` |

## Resolved gaps

| ID | Gap | Cap (cur→tgt) | Sev | Resolution | Bead |
|---|---|---|---|---|---|
| CD-8 | **Catalogue capability was not CI-gated.** The VoID + Service-Description catalogue (`crates/sparq-server/src/descriptors.rs`, #219) is real with 6 `#[test]`s but lived behind the **default-OFF** `federation-descriptors` cargo feature with no CI lane compiling or running it, so a regression in the module would pass all gates silently. This held 2.1 at maturity 3. | 2.1 (3→**4**) | P1 | **RESOLVED — PR #244 (merged); partially regressed by the lean-CI change** (the lane now runs nightly, not per-PR, and the `ci-summary` aggregator is deleted, so 2.1 is re-scored 4 → 3). `feature-matrix.yml` adds a GATING `opt-in sparq-server (federation-descriptors, service, time-travel, geo, test-seams)` leg that runs `cargo build` + `cargo test` + `cargo clippy --all-targets -D warnings` with `--features federation-descriptors,…` on every PR/merge_group/push; the leg is auto-discovered as a required check by the `ci-summary / gate` aggregator (its name carries no advisory/informational marker). The 6 descriptor tests now gate every change → **2.1 re-scored 3 → 4**, Component 2 average → 3.5. | bead **sq-kzfi** (epic `sq-toze`) |

## Explicitly NOT gaps (auditor anchors — do not re-open)

- **ZK/MPC not contributing to 4.2/4.3.** This is **correct by the honesty contract**, not a gap to
  close by crediting the scaffold. <!-- [OPUS-4.8] reconciled with post-remediation re-audit (sq-gbp4); see ZK-verdict cross-ref sweep --> The estate is **remediated but NOT externally audited**
  (originally unsound; `sq-1s2` landed; internal re-audit "sound as landed for the assumed threat
  model"; external sign-off `sq-qhy4` PENDING; no production guarantee — `research/zk-soundness-audit.md`,
  `research/zk-verifier-reaudit.md`); this is binding. The remaining crypto remediation + the external
  sign-off are tracked by the ZK remediation beads + `sq-qhy4`, **not** by CDMC.
- **No per-user authentication in `sparq-server`.** Documented architectural decision (threat-model
  **B3**: "front with a gateway / sparq-solid"). The optional bearer-token write gate (sq-zcby) +
  `sparq-solid` WAC/ACP are the in-scope entitlement controls; full user auth is operator-owned.
- **Plaintext mmap / plaintext HTTP.** Operator-deployment (TLS gateway + disk/KMS encryption),
  recorded as operator-owned in `controls.md` 4.3, with the handoff doc tracked as CD-3 — not a
  sparq-source defect.

## Re-score triggers

Per the orchestration runbook, CDMC scores the **current** codebase state. If **CD-1** (lineage) or
**CD-2** (access audit) — the two most likely to drive a `crates/` change — land before
consolidation, **re-score capabilities 6.2 and 3.2** and update `scorecard.md` accordingly.
**CD-8** (bead `sq-kzfi`) **has landed** — PR #244 added the `feature-matrix.yml` lane that compiles,
tests and clippies `federation-descriptors`, so capability **2.1 is re-scored 3 → 4** and Component 2's
average is back to **~3.5** (see the Resolved gaps table above).
