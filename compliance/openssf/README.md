<!-- [OPUS-4.8] OpenSSF framework intro (epic sq-toze, bead sq-toze.15). Authored while
     Fable unavailable — re-review when Fable returns. NON-CANONICAL timing. -->

# OpenSSF — Scorecard + Best-Practices (CII) Badge

This is the **OpenSSF** slice of sparq's certification readiness pack. It covers two
distinct OpenSSF programmes that consumers of an open-source dependency check:

1. **OpenSSF Scorecard** — an *automated* assessment of the repository's security
   posture (branch protection, code review, pinned dependencies, SAST, fuzzing, token
   permissions, signed releases, …). sparq runs it in CI and publishes the result to the
   public OpenSSF dashboard. See [`controls.md`](./controls/openssf.md) §A.
2. **OpenSSF Best-Practices Badge** (bestpractices.dev, formerly the *CII Best Practices
   Badge*) — a *self-certified* questionnaire across six families
   (basics / change-control / reporting / quality / security / analysis). The badge is
   **eligible but not yet filed** (gap GX-4). The drafted answers — each grounded in real
   repo evidence — live in [`evidence.md`](./evidence.md) §Badge, ready to be transcribed
   into the bestpractices.dev form when the maintainer files it.

## What this slice asserts (honesty contract)

Every row in [`controls.md`](./controls/openssf.md) is labelled one of:

- **Implemented & verified** — a technical control in the repo / CI, with a cited file
  path + the CI job that enforces it.
- **Audit-ready** — control + evidence in place, but the *external* step (the badge being
  *filed and accepted* on bestpractices.dev; the *published* Scorecard score being
  recomputed by OpenSSF infrastructure) is owned by the maintainer / OpenSSF, not by this
  agent. Labelled so explicitly.
- **Gap** — not met; recorded in [`gap-register.md`](./gap-register.md) with a remediation
  plan and the tracking `bd` bead (epic `sq-toze`).

We do **not** re-claim the strong base posture as new work — Scorecard itself
(`scorecard.yml`), SHA-pinned actions, the `ci-fast` required branch-protection check (core crates), CodeQL
SAST, cargo-fuzz, cargo-deny, the CycloneDX SBOM, the SLSA build-provenance attestation,
`SECURITY.md`, and `.well-known/security.txt` already exist. They are cited as **evidence**,
not presented as additions.

## Scope — what is in / out for a library + server

sparq is a Rust **library + binaries + container + WASM/JS/Py bindings** consumed as a
dependency, plus an HTTP query server (`sparq-server`). Both OpenSSF programmes are
**repository-level** — they assess the *project and its development process*, not a
deployed service — so almost all of their criteria apply directly to sparq. The handful
that do not, and why, are recorded inline:

- **Signed-Releases (Scorecard) / cryptographic release signing (Badge `crypto_*`).** sparq
  publishes GitHub releases with `SHA256SUMS` **and** a Sigstore-signed SLSA
  **build-provenance attestation** over every asset (`release.yml`). This is the modern,
  keyless equivalent of a detached GPG signature and is what Scorecard's `Signed-Releases`
  check now credits. The crates.io / npm / PyPI publishes are **manual** (not automated in
  `release.yml`); registry-side signing (crates.io has no first-party artifact signature
  scheme; npm provenance is possible) is recorded as a gap nuance, not a silent pass.
<!-- [OPUS-4.8] reconciled with post-remediation re-audit (sq-gbp4); see ZK-verdict cross-ref sweep -->
- **The ZK/MPC crypto estate is out of scope for *any* security claim here.** [OPUS-4.8] The v1
  ZK verifier was **originally found unsound** (`research/zk-soundness-audit.md`, kept on record),
  then `sq-1s2` landed the verifier-side binding layer and an **internal post-remediation
  re-audit** (`research/zk-verifier-reaudit.md`, `sq-gbp4`) found the prior findings closed →
  **"sound as landed for the assumed threat model"** — but that verdict is **internal /
  single-model self-review only, with external accredited-cryptographer sign-off still PENDING
  (`sq-qhy4`, P0) and NO production guarantee**; `sparq-mpc` is semi-honest-only with no
  guarantee (`SECURITY.md`). No OpenSSF control row implies a production crypto guarantee; the
  Best-Practices `crypto_*` criteria are answered about sparq's *own* use of cryptography
  (release attestation, TLS at the operator boundary), explicitly **not** about the
  research scaffolds.
- **Defect/bug-tracking criteria** (Badge `report_tracker`, `vulnerabilities_fixed_60_days`)
  map to the **beads** tracker + GitHub Security Advisories, not a public issue DB of
  security bugs (disclosure is private per `SECURITY.md`).

## Files

| File | Purpose |
|---|---|
| [`controls/openssf.md`](./controls/openssf.md) | The spine: every Scorecard check + every Badge criterion family → status → evidence (file / test / CI job) → owner. |
| [`evidence.md`](./evidence.md) | The drafted Best-Practices badge **self-certification answers** (one per criterion), each grounded in a cited artifact; plus the Scorecard per-check evidence narrative. |
| [`best-practices-self-cert.json`](./best-practices-self-cert.json) | The **machine-readable, import-ready** self-certification — the structured form of `evidence.md` §Badge (one object per bestpractices.dev criterion: status + justification + repo-relative evidence path(s)). CI-gated by [`scripts/check-bestpractices-evidence.py`](../../scripts/check-bestpractices-evidence.py) (`supply-chain.yml#openssf-selfcert`): every cited evidence path must resolve and every token must be legal, so the self-cert cannot silently drift. `project.filed` stays `false` until a maintainer files the badge (GX-4). |
| [`gap-register.md`](./gap-register.md) | Open gaps (the badge filing; the registry-publish signing nuance), severity, remediation, target, `bd` bead. |
