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
two distinct credentials. Private canonical nonnegative integers cover the complete
`u64` range (`MAX_PRIVATE_INTEGER`); public
predicates use the shared planner semantics. Selected blank nodes are rejected
in-circuit. Public predicates choose the `f0` member without numeric circuitry;
the `f2` member binds private numeric values back to canonical literal encodings.
Measured member costs live in the [gate snapshot](../../../crates/sparq-zk-compose/tests/gate_count_snapshot.json); expanded-profile measurement scope, source hashes and ACIR hashes are in [result capacity evidence](../../../bench/zk-compose/result_capacity_gates.json).

The presentation contains only version, query, released RDF terms, challenge, one or two
issuer key slots, an optional public integer-capacity selector and proof bytes. Its reconstructed public inputs additionally
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
The largest freshness-accepted status snapshot in the verifier policy selects
status depth 10, 17 or 20. Each snapshot must fit that tree in full. The shared
root/witness guard rejects oversized snapshots; no prefix truncation is accepted.
This choice depends on all accepted lists, including lists unused by the witness.
Missing trailing bits remain revoked padding. The policy retains its existing
fixed accepted-list capacity.

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
All proving and witness entry points accept only ASCII filename labels: letters,
digits, underscores, hyphens and dots, excluding `.` and `..`. Successful-result
labels must be nonempty; legacy untagged calls retain their shared names. Dots
are encoded injectively as `%2E` in internal filenames; literal percent signs are
not admitted. Names have fixed prefixes before caller labels. Input
writes reject final-component symlinks on Unix. The caller controls the workspace
and its parent directories.

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

[GPT-6] The successful-result public ABI sorts variable names lexicographically;
this is separate from the planner’s first-occurrence variable ordering. FILTER
bounds retain their complete `u64` public field encoding. Witness TOML represents
values above `i64::MAX` as decimal strings accepted by Noir.


[GPT-6] `ResultOptions::integer_capacity` defaults to
`IntegerCapacityPolicy::Smallest`: selected private values through 99 use
`PrivateIntegerCapacity::TwoDigits`, and larger canonical values select
`FullU64`. `HideInU64` uses the full-width member for any private predicate,
including small values. Public predicates still select `f0`. The public capacity
choice reveals a range bucket; the full-width relation keeps the exact decimal
length private. No public digit-count field is introduced. The joint planner
continues to minimize authentication and membership counts, not the gate cost
of competing numeric profiles.

The full-width relation reconstructs the canonical unsigned decimal lexical form
from a private `u64`, selects among fixed-length BLAKE3 token openings inside the
circuit, and checks the original signed string commitment. Checked accumulation
rejects overflow; leading zeros, signs, non-integer datatypes and values beyond
`u64` are outside this profile. No issuer re-attestation or host-only
value-to-lexical bridge replaces the commitment check. Nineteen- and twenty-digit
tokens cross the BLAKE3 block boundary, and all private-length branches contribute
to compiled cost. Gate snapshots report that cost; this is not a runtime speedup
claim. Signed `i64`, arbitrary precision and other numeric datatypes remain
separate extensions or exact-evaluator work.

Depth-ten small profiles retain version-one members and the legacy serialized
shape. Expanded profiles use version two. The verifier derives the member from
the issuer-slot count, query-derived private-filter count, integer capacity and
its own status depth, rejecting a version mismatch. The presentation carries no
prover-supplied key or status-depth override. Member suffix `i64` denotes the
unsigned integer width in bits, not signed `i64` support. With the pinned
Noir/bb toolchain, all original version-one members retain the foundation
circuit, ABI and verification-key bytes. The [compatibility evidence](../../../bench/zk-compose/result_v1_compatibility.json)
records actual comparisons against the baseline archive. A required toolchain
test regenerates each legacy key and compares it with those independent baseline
bytes; compatibility is not inferred from matching gate counts. The generic
shared relation keeps version-one private values as `u8` throughout, while the
expanded `u64` tiny profiles retain their pre-cast range guard.

Expanded-member witness executions are distinct from genuine proof checks. The
toolchain suite additionally proves representative wide, tiny and predicate-free
version-two members and checks their reconstructed public ABI through verification.
Exact gate measurements remain circuit-size evidence only. The planner's objective
still counts authentications and memberships, and the driver still regenerates
canonical keys during verification; neither a tiny-first search policy nor a
verification-key cache is provided by this expansion.
