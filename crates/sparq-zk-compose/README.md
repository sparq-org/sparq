<!-- [OPUS-4.8] sq-puyy: trimmed to the concise internal-stub README template (sq-9jw5). -->
# sparq-zk-compose

ZK proof **composition** for [sparq](../../README.md) — stage 2 of the query-proof
design ([`research/zkp-query-proofs-plan.md`](../../research/zkp-query-proofs-plan.md)
v3, §S4.E). Drives the per-property Noir circuit family at
[`zk/compose/`](../../zk/compose) into a full query-result proof
(`manifest::ProofManifest` + the nargo/bb subprocess prover) and verifies one
(`verifier::verify_manifest`). `capture` packages native per-circuit `driver::ProofArtifacts` into the browser-shippable *captured* manifest the `/showcase/zk-car-hire` in-tab verify fallback consumes (sq-1s2.3 / FL1 follow-up) — research-grade, NOT externally audited (sq-qhy4).

> **Internal crate — not published** to crates.io (`publish = false`): nothing in
> the workspace depends on it, so default and wasm builds are byte-identical either way.

<!-- separate, distinct blockquotes: the internal-crate note above vs. the soundness caveat below (MD028) -->

> **NOT-yet-sound** (standing caveat — sq-qhy4 / sq-9hrn; remediation epic sq-1s2).
> No soundness, zero-knowledge, or privacy property is claimed as achieved; the
> verifier's soundness is the subject of the open external audit
> ([external](../../research/zk-soundness-audit.md); internal re-audits:
> [binding](../../research/zk-verifier-reaudit.md) +
> [membership/PoK](../../research/zk-membership-pok-reaudit.md)). Not FIPS-approved
> ([fips-posture](../../compliance/cryptoreview/fips-posture.md)). The opt-in `dual-leaf` value lane (`filter_value_dl_{int,f64,decimal}` datatype-class members + the `dispatch` fail-closed `(method × circuit)` matrix; sq-xojl/sq-cfmv/sq-2ezsx — plus the `xsd:boolean` lane, which adds NO member and rides `filter_value_dl_int` via its public `datatype_const`, sq-5xdlk, and `filter_value_dl_datetime`, ONE member serving BOTH the `xsd:dateTime` and `xsd:date` classes on a signed scaled-epoch value handle and separated by their public lane constants alone, sq-wz99x — whose `Z`-only hookable domain is enforced fail-closed HOST-side) carries the #769-accepted **INV-VL downgrade**, an open audit obligation (**CR-G8** / `sq-qhy4`) — no soundness/privacy claim (API in the SKILL). The OFF-by-default `extended-fragment` wave-1 lane (`CircuitId::PathReach` bounded-depth property-path member + the fail-closed `verifier::dispatch_fragment` routing gate over a `FragmentManifest`, routed end-to-end by `verifier::verify_fragment_manifest`; sq-3kd2g.6 / sq-h732x) runs an accepted extended query's sub-proofs through bb verification (routing stage-1 through `fragment_query`), and the disclosed-solution term binding — `verifier::bind_fragment_solution` (sq-1zf94) for the path predicate/endpoints (`PathReach` `pred_enc`/`src_enc`/`dst_enc`) + `VALUES` cells, and `verifier::bind_fragment_scans` (sq-qyfth) for the per-solution BGP scan-slot row selection (`BranchWitness::scan_rows`) — binds each disclosed solution variable to the terms the verifier re-derives from the disclosed solution + query text, with join coherence across atoms sharing a variable (fail-closed). The per-branch cross-graph Q6 non-bnode obligation AND the existential scan↔path / path↔path join coherence are now enforced by `verifier::bind_fragment_join_coherence` (sq-ygk6x) — every existential variable shared between a scan slot and a `PathReach` endpoint is bound by encoding-equality, and a multi-graph path (whose interior-chain non-bnode obligation the verifier cannot discharge) is refused fail-closed. Since `sq-nlulr`, `verifier::bind_issuer_attestations` records PATH-referenced committed graphs in the audit-#9 salt-uniqueness gate too (each path commitment now carries the same issuer-attestation requirement + distinct-salt record as a scan commitment), so a cross-graph scan↔single-graph-path join carries the SAME distinct-salt non-bnode discipline as a scan↔scan join, and an unattested / salt-colliding path commitment is refused fail-closed. The only residual now is BY DESIGN (not a non-bnode gap): existential (non-projected) path endpoint values stay hidden — so an accepted extended-fragment proof carries the same attestation + salt discipline as the flat path, still asserting no soundness/privacy property. Opus 4.8 — re-review when Fable returns. <!-- [OPUS-4.8] privacy-claims-allow: opt-in dual-leaf value lane + fail-closed dispatch matrix + OFF-by-default extended-fragment routing gate (verify_fragment_manifest, sq-h732x) with the disclosed-solution term binding landed for path endpoints/predicate + VALUES cells (bind_fragment_solution, sq-1zf94), BGP scan-slot row selection (bind_fragment_scans, sq-qyfth), AND the per-branch cross-graph Q6 non-bnode + scan↔path/path↔path join coherence (bind_fragment_join_coherence, sq-ygk6x, multi-graph path fail-closed); sq-nlulr CLOSED the #1684 path-graph-salt residual (bind_issuer_attestations now attests + salt-records PathReach commitments), leaving only the by-design hidden existential path endpoint value; INV-VL downgrade framed as an OPEN audit obligation; every lane explicitly asserts no soundness/privacy property; sq-qhy4 / CR-G8 -->

How-to + the covered/deferred matrix: [`skills/zk-query-proofs/SKILL.md`](../../skills/zk-query-proofs/SKILL.md).
<!-- [GPT-6] zkp-3: typed host planning with private successful-row attribution. -->
The `planner` module derives disclosure obligations and selects private successful
result witnesses for strict SELECT DISTINCT / true ASK. It shares selected
memberships and credential authentication work and retains authentication when a
triple or FILTER operand is public. Its output is prover-local preparation, not a
proof or verification verdict. See [disclosure planning](../../skills/zk-query-proofs/references/disclosure-planner.md).
Benchmarks (gate counts, timing): [`bench/zk-compose/`](../../bench/zk-compose).
Contributing: [`AGENTS.md`](../../AGENTS.md).

## License

[MIT](../../LICENSE).

## Successful-result contract (opt-in)

[GPT-6] Enable `successful-results` for `result::{prepare_result, verify_result}`.
This research-stage addition is **not externally audited** and leaves the legacy
complete-scan verifier intact. It accepts nonempty `SELECT DISTINCT` answers for
positive BGPs with canonical nonnegative integer FILTERs. The verifier takes its
expected query, trusted issuer keys, authoritative status snapshots and fresh nonce
independently. It returns released mappings after checking the canonical circuit
key, public-input reconstruction and the proof. It does not assert answer
completeness, wallet completeness, absence or holder identity.

The circuit family admits one or two credential slots, sixteen triples per credential, three
patterns, four released rows, six variables and two private predicates.
`prepare_result` chooses the smallest credential bucket. This removes a signature
check for one-credential answers and reveals the smaller supporting capacity.
`prepare_result_with_options` with `CredentialCapacity::HideInTwo` keeps two
slots, privately repeating a single credential when needed; this does not assert
two distinct credentials. Private integers are bounded by `MAX_PRIVATE_INTEGER`; public
predicates use the shared planner semantics. Selected blank nodes are rejected
in-circuit. Public predicates choose the `f0` member without numeric circuitry;
the `f2` member binds private numeric values back to canonical literal encodings.
Measured member costs live in `tests/gate_count_snapshot.json`.

The presentation contains only version, query, released RDF terms, challenge, one or two
issuer key slots and proof bytes. Its reconstructed public inputs additionally
contain the accepted status-policy root and query-derived layout. Graph roots,
sizes, salts, credential status references, signatures, selected sources and hidden
term encodings stay in the private witness. Issuer identities, fixed capacities
and result size remain observable. Public issuer slots are ordered canonically;
individual list references are not disclosed. Each signature binds its own
private status index, list and version, and that list/version/root must belong to
the relying party's freshness-curated policy. Only existing clear-index
status-bound Schnorr credentials are accepted in this first wire contract.

`PreparedResult` has no Debug or serialization implementation. Its local work
counts identify selected credentials, shared leaves, witness uses and residual
predicates without entering the presentation. Input files use owner-only access
on Unix; successful-result proving cleans input/witness files after success or
error, although process termination can leave local files. The pinned backend is
Noir beta.21 and bb 5.0.0-nightly.20260324, explicitly `noir-recursive` (the installed
CLI identifies this as the ZK target; `noir-recursive-no-zk` is a different target).

Use a durable `SeenNonces` implementation. Authentication of external status
snapshots and selecting acceptable issuers remain the relying party's job.

[GPT-6] Backend builders can use `planner::plan_disclosure_admitted` to restrict
candidate eligibility without changing committed graphs or query semantics. The
callback receives the pattern index, original wallet/leaf reference and triple;
a rejection only removes that candidate, and all ordinary planner checks remain.

[GPT-6] `prepare_result` excludes untrusted, unauthenticated, revoked, stale or
over-capacity credentials before witness selection. Backend-ineligible selected
terms are skipped through candidate admission, so an early unsupported candidate
does not hide a later usable witness. Public query checks remain independent.

Successful-result input names are unique internally even when a caller repeats a
tag. Witnesses are created inside per-call owner-only directories, have owner-only
file permissions on Unix, and are cleaned by their scope owner. Both canonical-key
generation and verification now allocate independent subdirectories under the
caller's scratch root, so concurrent calls cannot exchange proof/input/key files.
Cleanup after abnormal process termination remains best effort.

[GPT-6] Optional `planner::optimize_disclosure[_admitted]` jointly chooses witnesses
across fixed released rows, minimizing authentication count and then shared
membership count within explicit resource and credential-capacity bounds. Its
report separates established structural optimality from budget exhaustion. This
does not change the baseline selection policy or assert a measured runtime gain.

[GPT-6] `prepare_result` now uses bounded joint optimization by default, enforcing
credential capacity during selection. `ResultOptions::witness_selection` can
select `FirstSuccess` for ablations; `max_search_steps` controls either search.
An exhausted search uses a complete feasible incumbent if available, records
`BudgetExhausted` in prover-local `work().optimization`, and otherwise returns
`ResultError::SearchExhausted`. Results and authentication obligations are never
truncated. Capacity selection applies to the chosen plan, so fewer selected
credentials can choose the smaller signature circuit. Structural search metrics
are local diagnostics; measured circuit costs are in the generated gate snapshot.

[GPT-6] The native subprocess driver serializes nargo compile and execute through
an OS advisory lock on the local workspace's target cache, including execute's
implicit compilation. Canonical-key generation and proving copy ACIR into their
own job directories while holding the lock. The public `compile` method returns
a reusable immutable content-addressed snapshot and rejects conflicting cache
contents. Its file lock uses the existing Unix `libc` dependency to preserve the
Rust 1.88 minimum; lock and I/O errors fail closed. This coordinates cooperating
driver processes on a local filesystem. External nargo writes, network filesystem
locking semantics, and deleting the cache during a job are outside that contract.

[GPT-6] Both planner paths reject oversized credential slices before traversal,
including empty or ineligible graphs, via `MAX_DISCLOSURE_CREDENTIALS`.
