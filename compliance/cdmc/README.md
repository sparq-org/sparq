<!-- [OPUS-4.8] CDMC framework slice index — epic sq-toze, branch cert-cdmc. -->
# CDMC — Cloud Data Management Capabilities (sparq slice)

> 🤖 SPARQ agent. sparq's self-assessment against the **EDM Council CDMC** framework
> (6 components · 14 capabilities · 37 sub-capabilities · 14 key controls), scored as a
> **data engine**. Part of the certification estate (epic `sq-toze`); see
> `research/production-certification-plan.md` §"Final target set" for why CDMC is in scope.

## Files

| File | What it is |
|---|---|
| **[`scorecard.md`](./scorecard.md)** | **The headline** — per-capability maturity rating (1–5) with rationale + component summary + top recommendations. |
| [`controls.md`](./controls.md) | Per-capability control → status → maturity → how-sparq-addresses-it → repo evidence. The auditor's spine. |
| [`evidence.md`](./evidence.md) | The concrete, checkable artefacts (file/test/CI paths) behind each rating. |
| [`gap-register.md`](./gap-register.md) | Open CDMC gaps, severity, remediation, target maturity, bead intents. |
| `README.md` | This file. |

## Scope — CDMC for a data *engine*, not a data *platform operator*

CDMC assumes an organisation running a cloud data platform over **its own** data. sparq is the
**engine** an operator embeds; the operator remains the **accountable data owner** for nearly every
governance/cataloguing/classification *decision*. This slice therefore scores sparq on **the
technical capability it provides** (the catalogue surface, the classification/validation engine, the
entitlement enforcement, the lifecycle mechanism, the architecture) and **explicitly marks
operator-owned axes** (ownership assignment, sensitivity taxonomy, retention policy, data residency,
encryption-at-rest enforcement) — capped at the maturity of sparq's *enabling hook*, never the
governed outcome. Inflating a score by crediting sparq for an operator decision is overclaiming.

## Out of scope for the engine (operator-owned)

- Assigning **data owners** and authoritative-source registration (1.2).
- Defining the **sensitivity/classification taxonomy** (2.2 — sparq supplies SHACL/reasoning to
  *enforce* one).
- **Encryption at rest / in transit** (4.3 — TLS-terminating gateway + disk/KMS; sparq files and the
  HTTP surface are plaintext by design, fronted per threat-model B3).
- **Retention policy** content (5.1 — sparq supplies the UPDATE/DROP + ring age-out mechanism).
- **Data residency / sovereignty** — entirely a deployment-topology decision.

## The headline (one line per component)

| Component | Maturity | Verdict |
|---|---|---|
| 1 Governance & Accountability | ~2.7 | Engine governance strong; data-ownership is operator-accountable. |
| 2 Cataloguing & Classification | ~3.0 | VoID/SD catalogue + SHACL classification hooks are real; catalogue (2.1) is regression-tested **nightly** (`feature-matrix.yml`, #244 / bead `sq-kzfi`) but no longer merge-gated → level 3. |
| 3 Accessibility & Usage | ~2.5 | Access control real (WAC/ACP + token); access audit + ODRL are gaps. |
| 4 Protection & Privacy | ~2.7 | Security **excellent (4)**; privacy/crypto deliberately **low (2)** — ZK/MPC remediated but NOT externally audited (no production guarantee). |
| 5 Data Lifecycle | ~3.0 | Solid lifecycle mechanism (UPDATE/WAL/retention); policy operator-owned. |
| 6 Technical Architecture | ~3.0 | Architecture **strength (4)**; lineage the weak axis (2). |

## Honesty contract (binding)

<!-- [OPUS-4.8] reconciled with post-remediation re-audit (sq-gbp4); see ZK-verdict cross-ref sweep -->
- **ZK/MPC is excluded** from every protection-by-cryptography maturity. `research/zk-soundness-audit.md`
  originally found the v1 verifier unsound; the `sq-1s2` binding layer has since landed and the
  internal re-audit (`research/zk-verifier-reaudit.md`, `sq-gbp4`) found it "sound as landed for the
  assumed threat model" — **but** external accredited-cryptographer sign-off is still PENDING
  (`sq-qhy4`, P0) and there is **no production guarantee**, so the estate stays excluded from any
  crypto-protection credit; `SECURITY.md` codifies this (§"`sparq-zk` and `sparq-zk-compose` — ZK
  verifier: remediated, but NOT externally audited"). No CDMC rating may credit it as a production
  privacy/crypto control. [OPUS-4.8]
- Operator-owned capabilities are **not** scored as sparq strengths; they are scored on sparq's hook.
- Status of the codebase at the head of `cert-cdmc`; **re-scored at consolidation** if material
  `crates/` changes land (notably the lineage/access-audit recommendations).

## Tracking

Gaps → beads under epic `sq-toze` (intents in `gap-register.md`; the lead files them in the main
checkout — `bd` is Dolt-backed and not available in the cert worktree).
