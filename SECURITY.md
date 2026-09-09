<!-- [OPUS-4.8] Governance: security policy (bead sq-3pq5). -->
# Security Policy

`sparq` is an experimental research RDF triplestore and SPARQL 1.1 engine. We take
security reports seriously and welcome responsible disclosure. This document explains
how to report a vulnerability, what to expect, and — importantly — which parts of the
project carry **no security guarantee** and must not be relied on in production.

## Reporting a vulnerability

**Please report security issues privately. Do not open a public GitHub issue, PR, or
discussion for a suspected vulnerability.**

Two private channels, either is fine:

1. **GitHub Security Advisories (preferred).** Open a private report via the repository's
   **Security → Advisories → "Report a vulnerability"** page
   (<https://github.com/sparq-org/sparq/security/advisories/new>). This keeps the report,
   discussion, and fix coordination private until a coordinated disclosure.
2. **Email.** Write to **jesse@jeswr.org** with a clear subject line (e.g.
   `[sparq security] <short summary>`). If you want to encrypt, say so in a first
   contact email and we will arrange a key.

Please include enough to reproduce: affected crate/surface and version (or commit), a
description of the issue and its impact, and a minimal proof-of-concept or steps where
possible.

## What to expect (response expectation)

`sparq` is maintained on a best-effort basis by a small team. We aim to:

- **Acknowledge** your report within **5 business days**.
- Provide an **initial assessment** (severity, whether we can reproduce, likely
  remediation path) within **10 business days**.
- Keep you updated as we work on a fix and **coordinate a disclosure timeline** with you.

We will credit reporters in the advisory unless you ask us not to. These are targets,
not contractual SLAs — this is a volunteer research project.

## Supported versions

`sparq` is **pre-1.0 and experimental; the API is unstable** (see [`AGENTS.md`](./AGENTS.md)).
We do not maintain long-term support branches. Security fixes are made against the
**`main`** branch and shipped in the **next release** of the affected published artifact
(the `sparq-*` crates on crates.io, `@sparq-org/sparq` on npm, and `sparq` on PyPI).

| Version line | Supported for security fixes |
|---|---|
| `main` (latest) | Yes — fixes land here first |
| Latest published release | Yes — via the next patch/minor release |
| Older releases | No — please upgrade |

If you depend on a specific release and cannot upgrade, say so in your report and we will
discuss options, but the default remediation is "upgrade to the fixed release."

## Scope and a critical caveat: research scaffolds with NO security guarantee

`sparq` includes cryptography-adjacent research crates. **These are research scaffolds.
They do not yet provide the security property they model, and a relying party MUST NOT
depend on them for any production security, privacy, or integrity guarantee.** This is a
deliberate, documented limitation, not an undisclosed weakness — so reports that simply
restate the gaps below are already known (but reports of *new* classes of failure, or of
a gap in a part we claim *is* sound, are very welcome).

<!-- [OPUS-4.8] Reconciled with the post-remediation re-audit (bead sq-zrt5); see
     research/zk-verifier-reaudit.md (sq-gbp4). Do NOT relax the no-production-guarantee
     posture or the external-pending (sq-qhy4) caveat. -->
### `sparq-zk` and `sparq-zk-compose` — ZK verifier: remediated, but NOT externally audited

The zero-knowledge query-proof estate (`sparq-zk`, `sparq-zk-compose`, and the Noir
circuits under `zk/`) is a research scaffold of the **in-circuit relations** (the scan
completeness/soundness relation and the integer/float filter-comparison relations are
correctly constrained in-circuit).

**Status — what changed.** The **original** adversarial soundness audit
([`research/zk-soundness-audit.md`](./research/zk-soundness-audit.md), 2026-06-13) found
the **v1 verifier BROKEN**: the verifier-side binding layer that ties the in-circuit
relations to a relying party did not exist, so `verify_manifest` proved essentially
nothing and a prover controlling its own side could make it return success over
arbitrary, false results. **That audit predated the remediation.** The `sq-1s2`
verifier-soundness epic (17 remediation commits) has since **landed** the binding layer:
`verify_manifest` now reconstructs the bb public inputs from the *declared* statement
under the *verifier's own nonce* and byte-compares them against each proof
(`PublicInputMismatch`); recomputes the canonical verification key verifier-side rather
than trusting the prover's; verifies real issuer Schnorr signatures against an external
trusted key set; enforces single-use replay/freshness binding; and binds the query's
FILTER operator/bound/verdict and BGP constants into the proof.

A post-remediation **re-audit** ([`research/zk-verifier-reaudit.md`](./research/zk-verifier-reaudit.md),
bead `sq-gbp4`) re-ran the same 12-finding adversarial pass against the verifier **as
landed** and found every prior finding (including all five CRITICALs) **CLOSED with code
evidence**. Its bottom-line verdict, in its own qualified words, is that the verifier is
**"SOUND as landed for the threat model the prior audit assumed"** — a prover that fully
controls its own side, presenting a manifest to a relying party that supplies the
external trust anchors (trusted key set, fresh nonce, authoritative revocation snapshot).

**The load-bearing caveats — read these.** "Sound as landed under the re-audit's stated
threat model" is **NOT** a production security guarantee, and this estate remains a
research scaffold a relying party MUST NOT depend on for any production security, privacy,
or integrity guarantee. Specifically:

- **External sign-off is still PENDING.** No independent accredited cryptographer has
  audited the verifier or the Noir circuits. That review (bead `sq-qhy4`, P0) is
  **REQUIRED before any production ZK security claim** and has not happened. The internal
  re-audit was run by an LLM agent and is itself pending external re-review.
- **The re-audit's closure rests partly on code-reading, not on tests running in CI.**
  The cryptographic-chain forge tests and the real `bb` prove/verify end-to-end cases are
  `#[ignore]`d (slow; require the nargo/bb toolchain) and do not run in default CI; only
  two empirical bb-output anchors are pinned. A toolchain change could silently shift the
  public-input serialization with no failing test (re-audit NEW-1).
- **Documented residual deferrals remain** (re-audit NEW-2): among trusted holders the
  proof-of-possession is not yet bound to the *specific* credential at every tier, and the
  revocation-list IRI / snapshot version (and the clear-index path's index) are disclosed
  — privacy/linkability residuals, not soundness holes.

**Do not present a "verified" result from this estate as a production-grade guarantee to a
relying party** until the external cryptographer audit (`sq-qhy4`) completes. The re-audit
is the design-of-record for the closed findings; the original audit is preserved for the
forge-and-verify regression map (`sq-1gir`).

### `sparq-mpc` — cryptography deferred

The multi-party-computation crate (`sparq-mpc`) is an early research scaffold. The MPC
cryptography (secret sharing / garbled circuits / authenticated-input malicious security)
is **deferred and not implemented** to a production standard. It provides **no
confidentiality, correctness, attestation, or malicious-security guarantee** today and
must not be used to protect real data across distrusting parties.

### Other capability crates

The opt-in capability crates (`sparq-geo`, `sparq-text`, `sparq-vectors`, `sparq-rsp`,
`sparq-shacl`, `sparq-reason`, `sparq-solid`, `sparq-hdt`, …) are functional features, not
security boundaries. As with the whole engine at this stage, run untrusted input through
`sparq` only inside your own sandboxing/resource limits.

<!-- [OPUS-4.8] sq-d7kh / CR-G4 (epic sq-toze) — FIPS posture cross-ref into the
     governance-owned SECURITY.md, mirroring how the ZK posture cross-ref was handled.
     Honest negative only: NO FIPS 140-3/CMVP module, NO FIPS claim, NO CMVP claim.
     Authored by Opus 4.8 while Fable 5 unavailable — re-review when Fable returns. -->
### FIPS / CMVP posture — no validated module, no claim

`sparq` incorporates **no FIPS 140-2 / 140-3 (CMVP-validated) cryptographic module** and
makes **no FIPS claim and no CMVP claim**. Its bespoke, research-only ZK/MPC cryptography is
deliberately built on ZK-friendly, **non-FIPS-approved** primitives (BN254, Poseidon2,
Schnorr over Baby-JubJub) and must be treated as **out of FIPS scope**; computing a
FIPS 180-4 algorithm such as the SHA-256 release digests is *not* the same as running it
inside a validated module. An operator under a FIPS deployment constraint must supply their
own FIPS-validated module. This is an honest negative posture, not a deficiency to be
remediated into a positive claim within this project. The full statement — the per-primitive
breakdown and FIPS-constrained-operator guidance — lives in
[`compliance/cryptoreview/fips-posture.md`](./compliance/cryptoreview/fips-posture.md) (CR-G4).

## Hardened-input expectations for the core engine

The parser, query engine, and HTTP server (`sparq-core`, `sparq-engine`, `sparq-server`)
are the surfaces most likely to process untrusted input. We treat memory-safety issues,
panics reachable from untrusted input that escalate to denial of service, and
authentication/authorization gaps in the server as in-scope security bugs — please report
them through the private channels above.
