<!-- [OPUS-4.8] sq-toze — Privacy framework (GDPR + ISO/IEC 27701 + SOC 2 Privacy)
     intro/scope. Engineer↔auditor loop (epic sq-toze). Re-review when Fable returns. -->

# Privacy — GDPR + ISO/IEC 27701 + SOC 2 Privacy

**Frameworks folded here:** EU/UK **GDPR** (principles + data-subject rights), **ISO/IEC
27701** (PIMS extension to 27001), and the **SOC 2** *Privacy* + *Confidentiality* Trust
Services Criteria. Per `research/production-certification-plan.md`, these three personal-data
frameworks are folded into one `privacy` worktree because they apply to sparq **narrowly and
in the same way** — through the operator-vs-engine responsibility split.

## The load-bearing reality: sparq is a data *engine*, not a controller

> **sparq does not collect personal data, and makes no controller decisions.** It is a
> general-purpose RDF/SPARQL engine + HTTP query server that processes *whatever* RDF its
> operator loads. The deploying operator decides what data to ingest, for what purpose, on
> what lawful basis, for how long, and who may query it. **The operator is the GDPR
> controller (and the ISO 27701 PII controller / SOC 2 entity); sparq is the technical
> means** — at most a *processor capability* the operator configures.

This is not a disclaimer to dodge responsibility. It is the honest scope boundary, and it is
the single most important framing in this whole framework: **you cannot certify that "sparq
is GDPR compliant"** — GDPR compliance is a property of a *deployment* (a controller +
purpose + lawful basis + retention policy + DPA chain), none of which live in this source
tree. What you *can* assess, and what this framework documents, is:

1. **Which technical privacy capabilities the engine provides** (access control, deletion,
   data minimisation, confidentiality, auditability) — with repo evidence; and
2. **Which obligations are squarely the operator's** — clearly labelled, with the
   engine-side hook the operator would use.

## Status legend

- **IMPLEMENTED & VERIFIED** — a technical privacy capability in the codebase/CI with a cited
  file/line + test/CI gate the auditor re-ran or re-read.
- **AUDIT-READY** — capability + documentation in place, but the *certificate* (a 27701/SOC 2
  attestation; a GDPR Art. 35 DPIA sign-off) needs the **operator's** ISMS/PIMS, a DPA chain,
  and (for 27701/SOC 2) an accredited external auditor we cannot substitute for.
- **OPERATOR-RESPONSIBILITY** — the control is a property of the *deployment*, not the source.
  The engine provides (or does not yet provide) a hook; the operator owns the policy. Labelled,
  never silently claimed as "sparq does this".
- **GAP** — a technical capability the engine *could* provide to make an operator's privacy job
  easier, but does not yet. Recorded in [`gap-register.md`](./gap-register.md) with a `bd` bead.

## What is in scope for a library/server (and what is out)

**In scope (what the binary can touch):**
- **Loaded RDF datasets** — the personal data, if any, is whatever the operator ingests
  (files, HTTP bodies, `LOAD`). The engine treats all RDF identically; it has no notion of
  "personal data" vs other triples (see [`../data-flow.md`](../data-flow.md)).
- **Query inputs/results** — SPARQL text in, result rows out. Both can contain personal data.
- **Server-side state** — the in-memory graph, the optional `--persist DIR` write-ahead log,
  Prometheus metrics, and request logs (opt-in via `--verbose`).
- **The WASM/JS client surface** — `sparq-wasm` runs the engine *in the user's own browser
  tab*; data never leaves the client unless the embedding app sends it.
- **Access-control capability** — `sparq-solid` provides graph-level WAC/ACP authorisation
  (fail-closed) the operator can use to enforce need-to-know.

**Out of scope (operator's deployment, not sparq's source):**
- Lawful basis, purpose limitation, consent capture, privacy notices, DPIA *sign-off*, DPA
  chains, cross-border transfer mechanisms, breach *notification* to a supervisory authority,
  DPO appointment, records of processing (RoPA) — all **OPERATOR-RESPONSIBILITY**.
- TLS termination, network isolation, at-rest disk encryption, backup retention — the
  operator's environment (mapped in [`../data-flow.md`](../data-flow.md) + ISO 27001).
- **The ZK/MPC "privacy by cryptography" story** is documented honestly as **NOT YET SOUND**
  and is **excluded** from every privacy-capability claim in this framework — see the explicit
  carve-out in [`controls.md`](./controls.md) §"ZK/MPC privacy story" and bead **sq-toze.35**.

## ZK/MPC claim-honesty CI gate (sq-toze.35, ENFORCED)

The standing-truth honesty constraint — *no doc may claim a ZK/MPC privacy or soundness
property as a settled, achieved fact while the v1 verifier is pending external audit
(**sq-qhy4**) and `sparq-mpc` is semi-honest only* — is now enforced **in CI**, not just in
prose:

- **Script:** [`scripts/check-privacy-claims.sh`](../../scripts/check-privacy-claims.sh) — a
  grep-only, deterministic, network-free scanner. It greps the **outward claim surface**
  (root docs, `skills/`, `site/` copy, the `compliance/` index, crate READMEs) for a fixed set
  of forbidden unqualified-claim phrases (`zero-knowledge-secure`, `provably private/secure`, <!-- privacy-claims-allow: documents the gate's own phrase list; sq-toze.35 -->
  `sound(ness) verifier/proof`, `privacy-preserving`, `malicious(ly)-secure`, <!-- privacy-claims-allow: documents the gate's own phrase list; sq-toze.35 -->
  `cryptographically private/sound/secure`). <!-- privacy-claims-allow: documents the gate's own phrase list; sq-toze.35 -->
- **CI wiring:** HARD gate job **`privacy-claims (ZK/MPC honesty gate)`** in
  [`.github/workflows/docs-quality.yml`](../../.github/workflows/docs-quality.yml). The job name
  fails on an unqualified claim on pull requests, but it is **not** a required check (only
  `ci-fast` is; the `ci-summary` aggregator is deleted) — it reports, it does not block the merge.
- **Allow-list mechanism (the audit trail).** The phrase set is deliberately coarse, so it also
  matches *legitimate* hedged / negative / "model"-qualified mentions (e.g. "**NOT** a sound
  verifier", "documented **NOT** cryptographically sound", "which *model* privacy-preserving"). A <!-- privacy-claims-allow: documents legitimate hedged-usage examples; sq-toze.35 -->
  matched line is exempted **only** if it carries the inline marker
  `privacy-claims-allow: <one-line justification>` recording *why* that exact line is a
  legitimate usage. Whole-file path exclusions are limited to the `research/` design records and
  the `*audit*.md` documents (which must be free to name the properties to argue (un)soundness)
  plus the gate's own files. Weakening the regex, or blanket-excluding a live outward doc surface,
  to "make it pass" defeats the gate and is itself an honesty defect.
- **Status:** the repo **passes the gate clean** after the sq-toze.35 fixes (the one real
  outward overclaim was the top-level `README.md` ZK/MPC feature bullets, now caveated to point
  at `SECURITY.md`).

## Deliverables in this framework

| File | What |
|---|---|
| [`controls.md`](./controls.md) | The control spine: each GDPR principle / 27701 clause / SOC 2 Privacy criterion → status → engine-capability-or-operator → evidence → owner. |
| [`evidence.md`](./evidence.md) | Per-capability file/line/test verification the control table cites. |
| [`gap-register.md`](./gap-register.md) | Open engine-side gaps + severity + remediation + `bd` bead. |
| [`retention-erasure-runbook.md`](./retention-erasure-runbook.md) | **Operator runbook** (not a cert claim) — how an operator fulfils data-subject erasure (Art. 17) + retention (Art. 5(1)(e)) on a sparq deployment: locate → export → erase → verify, with the honest WAL/backup/physical-erasure caveats (PR-G3). |
| [`data-minimisation-posture.md`](./data-minimisation-posture.md) | **Operator guidance** (not a cert claim) — what sparq actually logs default-vs-`--verbose`-vs-`--audit-log` (verified in-code), the PII vector in the GET query string, the data-minimisation posture (no request log by default; **request-log redaction ON by default under `--verbose`** — sq-toze.34/PR-G4; audit fingerprints not content; aggregate-only metrics), and guidance to keep logs PII-clean (PR-G2 / PR-G4). |
| [`../data-flow.md`](../data-flow.md) | **Shared** — every place the binary can touch data + the operator/engine responsibility split. |
| [`../dpia.md`](../dpia.md) | **Shared** — a DPIA *template/skeleton* honestly scoped to engine risks; the operator completes the deployment-specific half. |
| [`../threat-model.md`](../threat-model.md) | **Shared** — references the STRIDE model at `research/threat-model.md` (does not fork it) + the privacy-specific threats. |

## Honest one-line posture

The engine provides a **respectable set of technical privacy capabilities** — graph-level
fail-closed access control (`sparq-solid`), full SPARQL `DELETE`/`DROP`/`CLEAR` for erasure,
**data-minimisation by default** (no request logging unless `--verbose`; aggregate-only
metrics; generic error bodies that never echo query/RDF content), optional Bearer auth, and a
default-deny SSRF egress allowlist. **It does not, and structurally cannot, make sparq "GDPR
compliant"** — that is the operator's deployment property. No privacy claim here rests on the
ZK/MPC estate, which is documented as not-yet-sound.
