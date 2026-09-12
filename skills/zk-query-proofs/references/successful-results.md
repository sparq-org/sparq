# Successful-result contract (opt-in)

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
Measured member costs live in the [gate snapshot](../../../crates/sparq-zk-compose/tests/gate_count_snapshot.json).

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

[GPT-6] Successful-result preparation applies the same credential-slice cap before
signature authentication or graph cloning. The generated commitment-method gate
matrix lists every result capacity member as string-canonical only; it does not
inherit dual-leaf compatibility from the legacy lexical-handle dispatch rule.
