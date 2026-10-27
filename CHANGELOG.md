# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **sparq-core / sparq-engine:** durable UPDATE journals now record named-graph creation and
  removal alongside triple changes, with ordered replay and support for the previous journal
  format. Hardens named-graph layout recovery alongside directory ownership support:
  stable manifest slots, checked allocation and cleanup, complete durability barriers, and
  shared write refusal after indeterminate log writes. Removing a child from the attached
  in-memory set no longer removes its on-disk manifest entry; durable drop methods change the
  persisted set. Read-only opens leave undeclared slots alone; writable cleanup failures warn
  with the affected path. Cleanup follows opening the complete declared tree and preserves
  separate stores and held descendant locks. Journal format upgrades use durable file
  publication. Invalid declared layouts leave slots intact, and linked named-graph
  containers are refused before cleanup or batch-drop publication.
- **sparq-core (`mmap`): one writer per store directory.** A store is locked while a graph is
  open from it: `<dir>.lock` beside the directory, taken when it exists or can be created, then
  `sparq.lock` inside it, always, in that order, by every opener. A second writable open, save,
  build or compaction of the same store fails at once (`WouldBlock`) instead of corrupting it;
  `Graph::open_read_only` opens share the lock with each other. A store whose parent directory
  is not writable (a mount root, say) still opens, writable or read-only, held by its inside
  lock alone; a compaction, vacuum or restore there (they rename the directory) is refused with
  an error naming the parent. Compaction and restore swaps stage the new base under a recorded
  staging name, recover an interrupted swap at once, and roll back on a failure; a swap that
  can neither complete nor roll back freezes the graph: `Graph::is_frozen` reports it, every
  write is refused, and the embedded LWS store and the Python `Graph` (`frozen` property) report
  it as an error instead of serving it. New: `Graph::open_read_only`, `Graph::restore_in_place`,
  `Graph::is_backed_by`, `Graph::check_writable`, `Graph::is_frozen`, `Graph::into_dict`, and
  `dir_lock::{store_root, is_inside_store, is_reserved_store_path}`.
- **sparq-cli:** output files (`--out`, `reason`, tabular conversions) are written under a fresh
  name beside the target and renamed into place; an output path that names a store, a path
  inside one or a reserved name (lock files, in any letter case) is refused (exit 2), and a
  symbolic link is followed to the file it names, which is the file replaced.
- **sparq-zk-compose (`dual-leaf`):** complete dispatch coverage for the V2 component-witnessed
  scan `(1,16,4)` and join `(16,16)` members with dedicated `CircuitId` variants.
  V2 ids are legal only under `DualLeafV2`. Checked constructors, serde and package-name
  parsing refuse uncompiled V2 shapes. Canonical VK selection uses the dedicated packages.
  Extended-fragment and value-FILTER refusals remain closed. Experimental and unaudited.

- **sparq-zk / sparq-zk-compose (opt-in `dual-leaf`):** `DualLeafV2` routes all six value lanes,
  and the host builds every lane's value-lane sub-proof inputs from typed operands.
  `dual_leaf_v2::is_hookable_datatype` and `encode_literal` now route `xsd:boolean`,
  `xsd:dateTime` and `xsd:date` as well as `xsd:integer`, `xsd:decimal` and `xsd:double`, so a
  graph committed under `DualLeafV2` carries a real value hook, under that datatype's own lane
  constant, for every literal on them — which is what a value-lane FILTER member needs the
  committed leaf to be. Two deliberate consequences. The three newly routed lanes inherit the
  fail-closed ingest rule, so any lexical outside their §13.2 value domain (non-canonical spellings such as
  `"1"^^xsd:boolean` or a `24:00:00` instant, and also XSD-canonical forms the domain excludes:
  an untimezoned or non-`Z` `xsd:dateTime`/`xsd:date`, or more than three fraction digits) now fails the WHOLE commitment instead
  of taking the degenerate string lane. And **a V2 commitment over a graph that CONTAINS a
  literal on one of those three lanes changes**; every leaf on the three already-routed lanes,
  every degenerate string-lane leaf, every IRI and blank-node leaf, and the whole-graph
  commitment over a graph of only those are BYTE-IDENTICAL, pinned by a fixed vector captured
  before the change. `DualLeafV1`'s routing is unchanged, so an already-issued V1 graph is
  unaffected. Beside this, `build::build_filter_value_dl_{int, f64, decimal}(&Literal, FilterOp,
  bound: &Literal)` complete the V2 builder set next to the existing `_bool` / `_datetime` /
  `_date` entries, over ONE shared assembly: both operands go through the lane's own
  `dual_leaf_v2` encoder, the COMMITTED literal's lane constant is carried, and the disclosed
  verdict is COMPUTED by that member's host verdict mirror (`build::integer_verdict`,
  `build::double_bits_verdict`, the shared `build::signed_epoch_verdict`) rather than taken from
  the caller. The decimal lane rescales the FILTER constant to the committed literal's scale
  EXACTLY — never rounded — refusing an inexact or overflowing rescale with
  `BuildValueDlV2Error::DecimalScaleNotRepresentable`, through the same two functions
  `value_pin` pins with. `BuiltFilterValueDlNumeric::lane_proof` feeds the checked typed prove
  entry, so the `Prover.toml` is rendered from validated typed fields and never from caller TOML.
  Every lane's builder output satisfies `value_pin::check_value_lane_pin` for the matching typed
  FILTER, and changing any single pinned field of it — member, lane constant, op, bound, verdict
  — is refused with that field's own `PinError`. **The value-lane verifier gates are UNCHANGED
  and still CLOSED:** `verifier::prefilter_manifest_structure` refuses every value-lane sub-proof
  at stage 0 with `CheckError::ValueLaneSubProofNotAccepted`, the stage-2 operand match,
  `filter_edge_true` and `encode_pattern_slot` are untouched, and a test asserts per lane that a
  manifest assembled entirely by these builders is still refused. Host-side Rust only; no `.nr`
  source changed. Research grade, NOT externally audited (`sq-qhy4`); the value lanes carry the
  #769-accepted INV-VL downgrade (CR-G8). No soundness or privacy claim.
  <!-- privacy-claims-allow: opt-in host routing + build helpers; the INV-VL downgrade and the external audit are framed as OPEN obligations; the verifier gates are stated as unchanged and closed; asserts no soundness/privacy property; sq-qhy4 / CR-G8 -->

- **sparq-zk-compose:** verifier-side **authenticated commitment method**. The verifier no longer has
  to assume which commitment method a committed graph was made under, and it does not read one from
  the manifest either — it RECOVERS it from the issuer signature. `method_auth` builds one candidate
  signed message per `CommitmentMethod::recognised()` method from values the verifier already
  established (the commitment, the issuer-attested salt, the issuer-bound status reference, and the
  attested holder digest when the attestation is holder-bound) plus the method-UNBOUND message, and
  `classify` reports which one the presented signature verifies over: `AuthenticatedMethod::Bound(m)`
  or `AuthenticatedMethod::Unbound`. `MethodPolicy` is the relying party's decision over the answer,
  carried on the external trust anchor as `KeySet::with_method_policy` (the
  `with_hidden_issuer_depth` precedent, so no public verifier signature changes):
  `MethodPolicy::current()` is the default, `require_method_bound()` refuses an attestation that binds
  no method (`CheckError::MethodNotAuthenticated`), and `admit_legacy_dual_leaf_v1()` is the
  written-down opt-in for an attested superseded method (refused otherwise with
  `CheckError::MethodNotAdmitted`); more than one method's message verifying is
  `CheckError::AmbiguousMethod`. For a HIDDEN-ONLY commitment the method is selected by the hidden
  entry's declared `message`, which must EQUAL one of the candidates — a value matching none of them
  is refused with `CheckError::HiddenIssuerMessageMismatch`, and the policy applies to the method the
  hidden-issuer proof is then required to bind. Every entry over a hidden-only commitment is checked,
  and they must all select the same candidate, so the message check does not depend on the order
  the entries are listed in. New PRESENTATION message shapes and signers in `sparq-zk`, each under its own
  domain tag: `sig::presentation_message_with_status_and_scheme` (`ZKSIG_M5`, for a bearer
  attestation) and `sig::commitment_message_with_holder_and_scheme`
  (`ZKSIG_M4`, the one shape binding both a holder key and a method), signed with
  `SecretKey::{sign_presentation_with_status_and_scheme, sign_commitment_with_holder_and_scheme}`. A
  REGISTRY entry's own `ZKSIG_M3` signature is deliberately NOT a presentation attestation — recording
  a commitment in an issuer's registry is a separate statement from authorising its presentation, so
  presenting one as a clear attestation reports `CheckError::InvalidIssuerSignature`. Back-compatible
  with ONE qualification, about the declared `message` of hidden-issuer entries: a hidden-issuer entry's declared `message` was previously not read at all,
  and over a hidden-only commitment it is now required to equal a verifier-built candidate (for every
  entry over that commitment), so a hidden-only manifest whose declared message is empty,
  not decodable as a field element, or unmatched is now refused with `CheckError::HiddenIssuerMessageMismatch`;
  a refusal of a malformed hidden-only manifest may also be reported earlier, in the pre-filter, than
  it was before. The hidden-issuer stage likewise requires EVERY entry's declared `message`, including
  one over a commitment that also has a clear attestation, to equal the message its proof is checked
  against, and that check may now be reported ahead of an entry's depth or proof refusal. Otherwise
  every manifest that verified before still verifies under the default
  policy — an attestation over the method-unbound `ZKSIG_C3`/`ZKSIG_C4` message authenticates as
  `Unbound`, which the default policy accepts, and a bad signature still reports
  `CheckError::InvalidIssuerSignature`. The newly accepted manifests are only those carrying an
  issuer signature over a method-bound PRESENTATION message (`ZKSIG_M5`/`ZKSIG_M4`). Not externally
  audited (sq-qhy4); no soundness or
  privacy property is claimed.
- **sparq-zk:** `verify::fragment_typed_filters` re-derives a query's FILTER obligations keeping each
  constant as the typed `oxrdf::Literal` the query spells (`verify::TypedQueryFilter { variable, op,
  constant }`), for a relying party that needs the constant's datatype and lexical form rather than a
  `u64`. It shares one private walker with `fragment_filters`, whose behaviour and error strings are
  unchanged (a regression test asserts the messages verbatim), and widens the datatype axis only: the
  admitted comparison shape is still `?var op <literal>` after the `const op ?var` flip, with
  everything else fail-closed. Also `commit::CommitmentMethod::recognised()`, the enumeration
  counterpart of `from_scheme_iri`.
- **sparq-zk-compose:** `value_pin` holds the relying party's **pinned value-lane inputs** for the
  dual-leaf lanes (OFF-by-default `dual-leaf` feature): given an authenticated `DualLeafV2` method and
  a typed query FILTER, `expected_value_lane` / `check_value_lane_pin` state what a value-lane
  sub-proof's member, lane constant (for the decimal lane, the scale it names), op, bound and verdict
  would have to be, each equality failing closed with its own `PinError`; `scan_method` requires every
  commitment a scan draws from to have authenticated the same method, and `check_lane_routed` refuses
  a lane the whole-graph commitment builder does not route. **The value-lane verifier gates stay
  CLOSED:** the unconditional stage-0 `CheckError::ValueLaneSubProofNotAccepted` refusal is unchanged
  and still first, the stage-2 binding-edge operand match and `filter_edge_true` are untouched, and a
  manifest every one of whose pins holds is still refused (a test asserts exactly that). Computing a
  pin is not enforcing one; the gate-opening change is separate. Research-grade, not externally
  audited (sq-qhy4); the value lanes carry the #769-accepted INV-VL downgrade (CR-G8).
- **sparq-zk:** `dual_leaf_v2` is a second dual-leaf commitment method (`CommitmentMethod::DualLeafV2`,
  scheme IRI `https://sparq.dev/ns/zk#poseidon2-dualleaf-v2`) behind the OFF-by-default `dual-leaf`
  feature. It keeps V1's leaf shapes and re-derives the constants: lane constants in BLAKE3's
  key-derivation mode over the length-framed `(datatype IRI, lane params)` pair, the degenerate fold
  left in the plain mode, and a `value_none()` re-derived until it falls outside the reachable
  value-handle band, which `is_in_handle_band` decides and a test asserts — the checkable invariant
  `research/zk-field-native-encoding.md` §14.1 asks for, discharging that reserved-tag requirement on
  the HOST side. The in-circuit half landed with the component-witnessed members in the entry below: the
  `zk/dual-leaf-v2` Noir library mirrors the sentinel and its band check, and
  `compose_core::dual_leaf_v2_leaf` applies it at the `scan_v2dl_*` / `join_eq_v2dl_*` witnessed
  component sites, which no `CircuitId` names. On the value-lane FILTER members a sentinel is
  still refused only because their `u64` range decomposition cannot represent it. V1's constants, vectors and public API are
  unchanged and V1 stays recognised and verifiable, but it is no longer production-selectable. The
  §14.1 `LANG_NONE` separation is asserted on the host side only (see the hardening entry below). Both versions carry the same #769-accepted INV-VL
  downgrade; not externally audited (CR-G8 / sq-qhy4).
- **sparq-zk-compose:** `filter_value_dl_bool` is the `DualLeafV2` `xsd:boolean` lane's dedicated
  member — `filter_value_dl_int`'s public layout field-for-field, plus in-circuit constraints that the
  witnessed `VALUE_HOOK` and the public bound lie in `{0, 1}`, so a handle the integer member's `u64`
  range decomposition admits is refused by the relation. Wired through `CircuitId::FilterValueDlBool`,
  `ProofInputs::FilterValueDlBool`, `manifest::boolean_lane_const_v2`,
  `build::build_filter_value_dl_bool`, `toml::filter_value_dl_bool_prover_toml`, the verifier's
  public-input reconstruction and the dispatch matrix. The `DualLeafV1` boolean lane keeps sharing the
  integer member, unchanged. The boolean, dateTime and date lanes have V2 builders; the integer,
  decimal and double lanes have V2 host encoders and V2 renderers but no V2 `build` helper yet. No
  soundness or privacy claim.

- **sparq-zk-compose (opt-in `successful-results`):** METHOD-BOUND successful-result issuance,
  added beside the retained contract rather than replacing it. Two new members,
  `result_v5_k1_n16_p3_r4_f0_d10` and `result_v5_k2_n16_p3_r4_f0_d10`, compile a new
  `compose_core::result_m5::result_check_m5` relation that folds the METHOD-BOUND presentation
  message `sig::presentation_message_with_status_and_scheme(root, salt, status_ref,
  scheme_iri_to_field(CommitmentMethod::StringCanonicalV1.scheme_iri()))` (`ZKSIG_M5`), so the
  issuer attests the string-canonical commitment METHOD and not merely the commitment. The scheme
  digest is a compile-time constant of the relation, so the public ABI is the version-1 f=0 one
  unchanged and the verifier's public-input reconstruction does not move. `prepare_result` selects
  these members by default at one or two credentials with no private predicate and status depth 10,
  and refuses any other bucket by name instead of silently issuing a method-unbound proof.
  `compose_core::result` and `compose_core::result_public` are byte-for-byte as they were, so every
  shipped version-1 to version-4 member keeps its compiled circuit, ABI and verification-key bytes
  and the fixed-foundation rebuild check passes unchanged.
  `crates/sparq-zk-compose/tests/result_scheme_pin.rs` pins the new relation as the legacy one with
  only the signed message substituted, with a control test that the substitution does change it.
  Gate-count baselines and the constraint-count pack cover the two members. Research grade, NOT
  externally audited (`sq-qhy4`). No soundness or privacy claim.
  <!-- privacy-claims-allow: describes an added method-bound relation and its retained legacy contract; the external audit is named as an OPEN obligation and the downgrade is written down; asserts no soundness/privacy property; sq-qhy4 -->

### Changed

- **Breaking — sparq-lws-core:** LWS resource DELETE requires Read for `If-None-Match: *`.
  A non-reader's data DELETE with `If-Match: *` is treated as unconditional: settled present
  and absent targets answer `204`, subject to the same full-budget and set-aside-parent checks.
  Other `If-Match` values on DELETE, including malformed or empty lists, require Read too.
  POST with either bare `*` works without Read. Concrete tags and
  applicable dates require Read on every method. LDP exempts bare `*`, and LWS service
  containers retain their existence-only handling. Conditional POST to the storage root
  needs no Read when `Accept` selects its public description.
- **Breaking — sparq-lws-core:** `Store::lifecycle` is required and returns `Arc<Lifecycle>`;
  `CompositeStore` clones share it. `SparqClient::reset_marker` requires a lifecycle guard;
  wrappers must forward it, or a marker reset is refused by the trait default.
- **sparq-core (BREAKING API):** `Graph::dict` and `Graph::store` are no longer public fields;
  read them with `Graph::dict()` / `Graph::store()`, and take the dictionary out with
  `Graph::into_dict()` (rebuild with `Graph::from_parts`). These openers and writers of a
  store's files became crate-internal, so a store's files are reachable only through the
  locked `Graph`: `Dict::{save, save_mmap, open_mmap, fork}`, `TripleStore::{open, save,
  save_compressed, save_pred_stats, fork}`, `compress::CompressedPermWriter::{create, create_with,
  finish}`, and `extsort::{spill_run, kway_merge, kway_merge_compressed, external_sort,
  external_sort_compressed, map_perm}`. Migration: open, save and build through `Graph`
  (`Graph::open`, `Graph::save`, `Graph::build_external*`), fork through `Graph::fork`, and restore an
  open store with `Graph::restore_in_place` (`Graph::restore_into_durable` refuses a directory a
  graph is open from).
- **Breaking — sparq-zk-compose:** `CircuitId::package()` now returns
  `Result<String, CircuitIdError>`. `ProofManifest::to_json()` and
  `FragmentManifest::to_json()` now return `Result<String, serde_json::Error>`;
  hand-built uncompiled V2 ids return an error instead of panicking.
  With `dual-leaf`, opaque scan/join ids are refused under `DualLeafV2`;
  use the dedicated V2 component-witnessed ids instead.
- **sparq-zk-compose (`dual-leaf`):** restore the explicit `admit_unbound()` downgrade:
  all-Unbound sub-proofs resolve as `StringCanonicalV1`, matching pattern encoding.
  Opaque scans/joins remain legal; mixed Unbound + Bound sub-proofs remain refused.
  Under `admit_unbound` the commitment is ASSUMED string-canonical; nothing checks it.
  The downgrade is sound only if the issuer signed unbound messages over
  string-canonical commitments alone. A dual-leaf commitment signed unbound cannot
  use the V2 members, and its opaque presentations are not sound.
  Orphan sub-proofs still require method-independent members.

- **sparq-zk-compose:** the verifier now selects each sub-proof's circuit member — and therefore
  its canonical verification key — from the AUTHENTICATED commitment method of the commitments
  that sub-proof is bound to, instead of from the manifest's declared `CircuitId` alone. The
  method is derived per sub-proof: a `Scan` (and an `extended-fragment` `PathReach`) resolves
  every commitment in its own `commitments` vector, a `JoinEq` resolves the ENDPOINT commitments
  of every declared `JoinEdge` that covers it (`sub_proofs[edge.scan_a].commitments[edge.graph_a]`
  and `sub_proofs[edge.scan_b].commitments[edge.graph_b]`) rather than its own prover-declared
  `commit_a`/`commit_b`, and a `Filter*` member inherits the method of the source scan of every
  validated binding edge that binds it; all of them must agree. The declared id is resolved through the
  `(commitment-method x circuit)` dispatch matrix (`dispatch::resolve_circuit`, opt-in behind
  `dual-leaf`) and only the resolved member is used. Five new fail-closed refusals:
  `CheckError::MemberNotLegalForMethod` (the member is not legal for that method),
  `CheckError::SubProofMethodUnresolved` (no single method could be derived — differing methods
  across bound commitments, a mixture of attested and unattested ones, an unparseable bound
  commitment, or a sub-proof with no derivable method whose member is not legal under every
  admissible method), `CheckError::SubProofCommitmentNotAuthenticated` (a bound commitment is
  ABSENT from the authenticated map, so nothing was authenticated for it at all),
  `CheckError::AuxiliaryMemberIdInSubProof` (a manifest sub-proof declares an id that belongs to
  one of the auxiliary bb roles below) and `CheckError::MemberNotForFixedRole`. Absence from the
  authenticated map is NOT read as `Unbound`: it is refused by name. Under the explicit
  `admit_unbound()` downgrade, a sub-proof all of whose
  bound commitments authenticate as Unbound resolves as `StringCanonicalV1`:
  opaque scan/join members remain legal. Under `admit_unbound` the commitment is
  ASSUMED string-canonical; nothing checks it. The downgrade is sound only if the
  issuer signed unbound messages over string-canonical commitments alone. A dual-leaf
  commitment signed unbound cannot use the V2 members, and its opaque presentations
  are not sound. Under the default policy,
  unbound attestations are refused upstream. Under `dual-leaf`, orphan sub-proofs require
  method-independent members; without it, they retain their declared member.
  A sub-proof bound to a commitment covered only by a hidden-issuer
  proof defers its resolution until after `bind_hidden_issuer_attestations`, so a candidate
  method is never read as authenticated. The five auxiliary bb gates (hidden and fully-hidden
  revocation, hidden issuer, holder PoK, holder set) are method-independent by construction —
  their member is fixed by the relying party's own policy or registry and declared nowhere in the
  manifest — and each now selects it through a named per-role fixed-member path that refuses any
  id outside its own role. This adds refusals and no acceptance. **Behaviour change:** a manifest
  whose sub-proof binds commitments that authenticated different methods, or a mixture of
  attested and unattested ones, used to be accepted and is now refused. For a `Scan` that is
  narrower than it sounds: `CheckError::ScanPatternMethodsMixed` already refused commitments
  whose methods disagree on the pattern-constant ENCODER, so what is newly refused there is the
  pairing of a method-UNBOUND commitment with a `StringCanonicalV1` one, which share one encoder.
  A `Filter*` and a `JoinEq` had no such check. An edge-less FILTER over a member legal under
  every admissible method still verifies. **Scope:** method folding and both resolution
  passes now run in every build. The dispatch matrix and `MemberNotLegalForMethod` remain
  behind the OFF-by-default `dual-leaf` feature. Without it, a successful fold retains the
  declared member under the existing method-policy and encoder checks. The other four
  refusals above, the stage 2g′ endpoint gate, `CheckError::JoinProofNotBoundToAnEdge`,
  `FixedMemberRole` and its per-role selection path are unconditional.
  Not externally audited (sq-qhy4); no soundness or privacy property is claimed as
  achieved.

- **sparq-zk-compose:** the hidden-join gate's ENDPOINT half now runs in BOTH stage-1 regimes.
  `bind_joins` enforces two separable things — the endpoint resolution plus commitment-matching
  (each declared `JoinEdge`'s join proof must carry public `commit_a`/`commit_b` byte-equal to the
  two referenced scans' `commitments[graph_*]`), which reads only the manifest, and the slot
  binding, which re-derives the query's patterns. The first is now its own gate
  (`bind_join_endpoints`, stage 2g′) and is no longer deferred with the second in the
  `extended-fragment` regime; only the slot binding and the N-way chain check remain deferred
  there (sq-1zf94). It is also what the `JoinEq` member resolution above reads its endpoints from.
  **Behaviour change (`extended-fragment` only):** a declared hidden `JoinEdge` whose join proof
  names commitments other than its endpoint scans' is now refused
  (`CheckError::JoinCommitmentMismatch`) instead of standing as a prover-declared linkage, and a
  hidden join one of whose endpoints is a bounded-path (`PathReach`) sub-proof rather than a
  `Scan` is refused fail-closed (`CheckError::JoinEdgeKindMismatch`). The flat regime's ENDPOINT
  verdicts are unchanged — it already ran that half of the gate — but see the coverage rule
  below, which is new in both regimes. In BOTH regimes and in EVERY feature state, a
  `JoinEq` sub-proof that no `JoinEdge` names is now refused
  (`CheckError::JoinProofNotBoundToAnEdge`): its `commit_a`/`commit_b` were free of the manifest's
  declared linkage, and no later gate read them. Adds refusals and no acceptance.

- **sparq-zk-compose (opt-in `extended-fragment`):** the extended-fragment verifier now REFUSES
  the query/presentation shapes it does not discharge, instead of accepting them.
  (1) A query one of whose re-derived branches carries a `FILTER` is refused by the routing gate
  (`FragmentDispatchError::FilterNotDischarged { branch, filters }`): the flat FILTER obligation
  lives in `bind_query_correctness`, which this regime skips, and nothing on this path reads
  `FragmentBranch::filters`, so an accepted presentation would have answered the query as though
  the FILTER were absent. Checked over EVERY branch, attributed or not; the branch-local typed
  FILTER obligation that would lift it does not exist yet.
  (2) A `VALUES` cell now constrains its variable whether or not that variable is PROJECTED:
  `bind_fragment_join_coherence` merges every selected non-`UNDEF` cell — re-encoded by the
  verifier from the query text, never a prover encoding — into the branch coherence map before
  the atoms are read, so the scan slot or `PathReach` endpoint carrying the variable must equal
  it. `bind_fragment_solution` still binds only the PROJECTED cells, having no disclosed term for
  the rest; before the merge an unprojected `VALUES` variable entered no map at all. Two blocks
  pinning one variable to disagreeing cells is `FragmentJoinError::ValuesCellIncoherent`, a cell
  the verifier cannot re-encode is `ValuesCellUnencodable`, and an atom disagreeing with a seeded
  cell is `Incoherent` against that atom.
  (3) The branch coherence maps refuse a DISAGREEING duplicate rather than overwriting it, so a
  variable occurring twice in ONE atom is compared: a path whose subject and object are the same
  variable (`ASK { ?x <p>+ ?x }`) must carry `src_enc` equal to `dst_enc`, and the two slots of a
  self-joining BGP atom must agree in the selected row.
  (4) `verify_fragment_manifest` refuses a presentation carrying NO `BranchWitness`
  (`CheckError::FragmentNoBranchWitness`): every branch-local layer iterates the witnesses, so an
  empty vector left all four of them with nothing to check and the accept would have rested on the
  crypto stage alone. `dispatch_fragment`'s per-branch ROUTING stays a deliberate no-op over such a
  manifest, since it is also used over stage-1-style presentations; its query-hygiene, FILTER
  and id checks still run.
  (5) A disclosed binding must be BOUND BY THE ATTRIBUTED BRANCH, projected, and unique.
  `bind_fragment_solution` and `bind_fragment_scans` build the re-encoded disclosed map through one
  shared admissibility rule: a branch binds the variables of its pattern `SlotPattern::Var` slots,
  its `path_reach` subject/object positions, and the `VALUES` columns whose SELECTED row carries a
  non-`UNDEF` cell. A disclosed variable outside that set is refused
  (`FragmentSolutionError::SolutionVariableNotBoundByBranch`) — previously a solution could carry a
  variable the attributed branch binds nowhere (the other arm of a `UNION`, a branch with no atoms,
  or a `VALUES` column selected at an `UNDEF` row) and every obligation over it was simply absent.
  A variable the query does not project is refused (`SolutionVariableNotProjected`), and a second
  binding of one variable is refused (`DuplicateSolutionVariable`) instead of silently replacing the
  first, which went unchecked.
  (6) A `VALUES` cell is encoded under the commitment method the branch's scans AUTHENTICATE:
  `bind_fragment_join_coherence` now takes `&AuthenticatedMethods`, resolves one pattern-constant
  encoder across the branch's scans and encodes each seeded cell with `encode_pattern_slot` under
  it, so the cell is comparable to the slots and endpoints it constrains under the authenticated
  method rather than under string-canonical always. A branch whose scans resolve no single method,
  or disagree, is refused (`FragmentJoinError::ValuesMethodUnresolved`). This is a breaking change
  to that function's signature; it is `pub` behind the opt-in feature and has no callers outside
  the crate's own verifier path.
  (7) The lane has ONE encoding policy. `verify_fragment_manifest` reads the authenticated-method
  map BEFORE the first layer that re-encodes a disclosed term and refuses a presentation referring
  to a commitment that authenticated any method other than
  `sparq_zk::commit::CommitmentMethod::StringCanonicalV1`
  (`CheckError::FragmentUnsupportedCommitmentMethod`; a method-UNATTESTED commitment is admitted,
  its pattern encoder being the same one). The layers re-encode a disclosed term with `encode_term`
  — string-canonical — and compare the result against disclosed scan-row slots and `PathReach`
  endpoint encodings, which is coherent only under that method; under another, an HONEST
  presentation would be refused for an encoding mismatch and nothing would be admitted, so the
  combination is refused by name instead. This is a bound on the lane, not a property established
  about any proof, and one coherent policy across all four surfaces is the deferred follow-up. Two
  consequences of reading the map earlier: an unattested or revoked commitment is now reported
  before the disclosed-solution layer rather than after it (both refusals, same accept set), and
  `bind_fragment_join_coherence` stays method-aware for the `VALUES` seed even though the lane
  admits only the one method.
  All of these ADD refusals and open nothing. The feature is OFF by default, so no default-build
  verdict changes with them, and no new dependency is added. Research grade, NOT externally audited
  (`sq-qhy4`). No soundness or privacy claim.
  <!-- privacy-claims-allow: describes added fail-closed refusals on an opt-in research lane, each with the obligation it does NOT yet discharge named; the external audit is restated as an OPEN obligation; asserts no soundness/privacy property as achieved; sq-qhy4 -->

- **sparq-zk-compose (extended-fragment):** a scan attributed to a branch witness must disclose
  at least one ACTIVE row (`FragmentScanError::NoActiveRow`). The scan circuit admits an empty
  result, and a fully ground pattern binds no variable, so the ground-pattern shortcut in
  `bind_fragment_scans` previously accepted a branch over a triple the graph lacks (for example
  `ASK { <a> <p> <b> }`, or a `VALUES`-supplied answer beside a ground pattern). The check now runs
  ahead of that shortcut for every attributed scan. A standalone empty scan proof is unaffected.
  Adds a refusal and opens nothing.

- **sparq-zk:** every query extractor refuses a solution SLICE (`LIMIT` and/or `OFFSET`) at any
  depth, under one shared reason, instead of descending through it: `verify::fragment_query`'s
  top-level modifier peel and its subquery arm, and the flat `verify::fragment_patterns`,
  `verify::fragment_filters` and `verify::fragment_expr_trees` walkers. A proof of this design
  states that a disclosed solution is a MEMBER of the query's answer, and a slice makes that
  unprovable from the disclosed data in both directions: a positive `OFFSET` can empty a
  non-empty answer (`SELECT ?x WHERE { VALUES ?x { <urn:a> } } OFFSET 1` has no solutions at all),
  `LIMIT 0` always empties it, and a `LIMIT` smaller than the answer selects an
  implementation-nondeterministic subset. Peeling the modifier off therefore proved membership in
  the UNSLICED answer and accepted a solution the query text excludes. `DISTINCT` / `REDUCED` are
  still peeled. `sparq_zk_compose::planner` already refused to PLAN a sliced query, so no honest
  consumer in the estate asks for one. Adds refusals and opens nothing.

- **sparq-zk-compose:** stage 2g′ (`bind_join_endpoints`) collects the covered join-proof indices
  in the SAME pass that validates the declared edges, so the coverage rule is a set lookup per
  sub-proof rather than a re-walk of `join_edges` for each one, and the `JoinEq` arm of the
  per-sub-proof member resolution reads a per-manifest index of validated endpoints
  (`JoinEndpointIndex`, built once) rather than re-validating the matching edges for every
  sub-proof it is asked about. Both vectors are prover-controlled, so neither product is a cost a
  presentation can choose any more. No verdict changes: the same edges are validated and the same
  endpoints are read.

- **sparq-lws-core (notifications):** subscription expiry is decided against a durable
  high-water time stored in the index, so a restart with the clock stepped back does not revive
  an expired subscription. On the first start after upgrading, and after a restore that drops
  index-only records, every subscription with an `expires` is removed and its subscriber must
  subscribe again; subscriptions without one are kept. Notifications now start only over a store
  that can raise that time atomically (the in-memory and embedded indexes); `LwsState::new`
  fails over any other. While the time cannot be stored, notification endpoints answer `503`
  and nothing is delivered. A stored time ahead of the clock stands (subscriptions expiring
  before it stay expired); to recover from a clock that jumped forward, correct the clock and
  start once with `SOLID_SERVER_LWS_RESET_SUBSCRIPTION_CLOCK=1` (`LwsConfig::reset_subscription_clock`),
  which, while the state is built, removes every subscription with an `expires` and then stores
  the clock.
  An `LwsState` now holds a lease per store handle (shared by every clone of its store,
  `LwsState::store`, and by any `Tracked` wrapper of one) while it lives: a second `LwsState`
  built over a clone or a wrapper of a running state's store is refused, so a state that replaces
  another must be built after the old one is dropped. A store that keeps no lease of its own
  (`Store::lifecycle` is `None`, as for a plain `CompositeStore`) gets a new one in each
  `LwsState::new`, so two states built over clones of one such store each get their own lease;
  a store opened again separately over the same backend has a lease of its own too. `Store::reset_marker` takes that lease's guard. `Tracked::new` takes its inner
  store's lease and no longer accepts one from the caller, and the store's generic mutators refuse
  an IRI shaped like the stored time, which changes only through `raise_marker` and
  `reset_marker`.

- **sparq-reason (N3 backward chaining):** a variable substitution is cheaper to COPY, with no
  change to any name or answer. The `name -> term` map a premise evaluation and a backward proof
  carry now holds the variable's name as a reference-counted string rather than an owned one, so
  copying a substitution — which the backward path does once per candidate rule site and once per
  solution, the join path once per candidate fact, and a formula multiset match once per pairing
  it tries — no longer allocates a fresh name for every entry it carries. That mattered most where
  the substitution is largest: a backward step's renamed variables stay in the substitution its
  whole subtree inherits, so the copies grew with the depth of the proof in allocation count, not
  merely in bytes. Alongside it, the unification of a backward goal against a rule conclusion
  decides the cases it can decide — the two sides already equal, or either one an unbound variable
  — on BORROWED terms, where it used to resolve and deep-copy both sides every time; only the
  member-wise unification of two compounds still owns its two sides, and only the term actually
  bound is copied. On the FORWARD side, a premise atom's variable names are now minted at most once
  for the whole atom rather than once per candidate fact, so matching a join atom against its
  candidates no longer allocates and frees a name per bound variable per row; a name is minted on
  the first row that actually binds its position, so a position no candidate ever binds costs
  nothing at all.

  **The hasher is unchanged and the amount of hashing is unchanged.** A key is still hashed as the
  variable name's bytes under SipHash, which is what keeps a crafted document from degrading the
  map, and a lookup still computes one hash of the same bytes and lands in the same bucket — one
  lookup per lookup, as before; the saving is in what a copy costs, not in what a lookup costs. No output depends on the
  substitution's iteration order: every reader of a whole binding, including the proof-mode
  tie-break, sorts it or asks a question of every entry. Every `__bw<n>_` name, derived fact, proof, `why()` string and `sparq-prov` IRI is
  byte-identical, pinned against output captured from the parent commit, and the W3C N3 inference
  conformance report is unchanged. No run that previously answered is refused, and no refusal
  changes. (#7080)

- **sparq-zk-compose / zk (sq-j506, #7121; Noir library #7103):** the `DualLeafV2` **component-witnessed** scan and
  hidden-join circuit members, and a **method-aware** pattern-constant encoder. Two NEW Noir
  members, `scan_v2dl_k1_n16_r4` and `join_eq_v2dl_na16_nb16`, recompute each graph's per-slot
  term encodings from witnessed V2 leaf components through
  `compose_core::dual_leaf_v2_leaf::recompute_term` — applying the reserved-sentinel band check
  and the reserved language-tag check at the witnessed component sites — and then run the
  UNCHANGED `scan_check` / `join_eq_check` bodies. The shipped `scan_*` / `join_eq_*` members and
  the relation sources are byte-unchanged, so existing verification keys and all issued material
  stay valid; each new member's public-input vector is its shipped counterpart's, and the two are
  told apart by their verification keys. `verifier::encode_pattern_slot` now folds a query pattern
  constant with the encoder the scan's AUTHENTICATED commitment method selects, with no fallback,
  and `bind_scan_pattern_methods` adds two named refusals — `CheckError::ScanPatternMethodsMixed`
  (a `K > 1` scan whose commitments select different encoders: one pattern constant cannot match
  two encodings) and `CheckError::ScanPatternMethodNotEncodable`. `verifier::bind_fragment_scans`
  takes the authenticated-method map as a second argument. **Gate order:** the attestation gate
  that authenticates the methods runs AHEAD of the stage-1 query-binding gates, and
  `bind_scan_pattern_methods` immediately after it, so every comparison site reads an
  authenticated map rather than comparing string-canonically whatever the attested method was;
  in the `extended-fragment` regime `verify_fragment_manifest` runs it before
  `bind_fragment_scans`, which is that regime's only scan-slot binding. The gate SET is
  unchanged and every gate still runs, so nothing that was refused becomes acceptable; what
  changes is which refusal is reported first for a manifest that fails both an attestation check
  and a structural check, and that the pattern gates now refuse strictly more. The value-lane and
  V2-scan verifier gates stay CLOSED: nothing becomes acceptable, and every change here only
  adds refusals.
  **BEHAVIOUR CHANGE in a DEFAULT build:** a scan whose authenticated method is `DualLeafV1` or
  `DualLeafV2` is now refused when the `dual-leaf` feature is off, because the host dual-leaf
  encoders are behind that feature and the verifier must not re-check such a scan's pattern
  constants string-canonically. Such a manifest used to be accepted. Research-grade, not
  externally audited (sq-qhy4).

- **sparq-zk-compose (BREAKING for `successful-results` users):** successful-result issuance and
  verification are METHOD-BOUND by default. A method-UNBOUND (`ZKSIG_C3`) credential, which attests
  no commitment method at all, is refused with a named error, and the new
  `ResultPolicy::admit_unbound_legacy` field (a public struct field, so constructing `ResultPolicy`
  requires it) is the written-down opt-in that acts on already-issued unbound material and issues
  through the retained legacy members. It has the same shape and meaning as
  `MethodPolicy::admit_unbound`: under it the commitment method rests on trusted-issuer honesty
  rather than an attestation the circuit recomputes. The signed (`result_v3_*`) and public-pattern
  (`result_v4_*`) families have no method-bound member yet, so they need that opt-in. The
  requirement is ONE boundness check on the SELECTED member, not one arm per contract family, so a
  family added later cannot inherit the legacy behaviour by omission. Each selected
  credential is now witnessed under the message the SELECTED member's relation folds, and an
  incoherent pairing — a method-bound attestation through a legacy member, or the reverse — is
  refused by name rather than witnessed into a proof that could not succeed. Legacy proofs stay
  verifiable; nothing previously refused is admitted.
- **sparq-zk-compose (BREAKING for a relying party on default settings):** `MethodPolicy::current()`
  — the default policy on `verifier::KeySet` — now **REFUSES** a method-UNBOUND commitment
  attestation (`CheckError::MethodNotAuthenticated`) instead of accepting one. A commitment whose
  method is not issuer-attested has no authenticated leaf shape, so there is nothing for a later
  gate to pin a lane constant or a pattern-slot encoding against; refusing it by default is the
  prerequisite that makes an authenticated method something a gate could rely on (sq-j506). A
  manifest whose attestation signs only the `ZKSIG_C3`/`ZKSIG_C4` message verified before and is
  now refused: re-issue over the method-bound `ZKSIG_M5`/`ZKSIG_M4` shapes, or state the downgrade
  with `KeySet::with_method_policy(MethodPolicy::current().admit_unbound())`. The new
  `MethodPolicy::admit_unbound()` is that written-down opt-in, and `require_method_bound()` takes
  the requirement back again; the unbound dial and `admit_legacy_dual_leaf_v1()` are independent,
  so neither widens the other. **It opens no gate:** the Stage-0 `ValueLaneSubProofNotAccepted`
  value-lane refusals are untouched and stay closed. Research-grade, NOT externally audited
  (sq-qhy4) — no soundness or privacy property is claimed as achieved.

- **sparq-zk-compose (opt-in `successful-results`):** the successful-result relations now
  authenticate every credential slot over the METHOD-BOUND presentation message
  `sig::presentation_message_with_status_and_scheme` (`ZKSIG_M5`) instead of the method-UNBOUND
  `commitment_message_with_status` (`ZKSIG_C3`), with the folded scheme digest fixed to the
  STRING-CANONICAL method. The relations prove `h3(Enc(s), Enc(p), Enc(o))` leaves over the
  string-canonical term encoding and nothing else, so the digest is a compile-time constant in the
  circuit (`compose_core::result::SCHEME_STRING_CANONICAL_V1`, with `hashes::h5` for the arity the
  message needs) rather than a public input — which makes the commitment method part of the
  relation instead of a prover choice. The host refuses per cause: an attested method that is not
  string-canonical, and a method-unbound attestation, which attests no method at all. There is no
  opt-in for the unbound case here, because the circuit side has no unbound message to recompute.
  A `ResultCredential` must therefore carry a `ZKSIG_M5` signature; an already-issued `ZKSIG_C3`
  one is refused. `crates/sparq-zk-compose/tests/result_scheme_pin.rs` pins the message the circuit
  recomputes from its own Noir literals to the one the host signs, so the two sides cannot drift
  apart silently. Every `result_v1_*`/`result_v2_*`/`result_v3_*`/`result_v4_*` member routes
  through the two shared library functions, so no member source changes; the `zk/compose` source
  hashes and verification-key/gate evidence are regenerated by the dedicated-hardware measurement
  run. NOT externally audited (sq-qhy4).
- **sparq-reason (N3 backward chaining):** the per-goal cost of naming a backward renaming by
  its proof-tree fingerprint is reduced, with no change to any name or answer. A goal's
  canonical identity key is built into one pre-sized buffer with the length prefix written
  straight into it, instead of a fresh string per term and a fresh string per length; the walk
  that resolves a goal's variables to the end of their chains now BORROWS the terms it did not
  have to rebuild, and keeps its occurs-check stack by reference; a candidate rule application
  folds its node key once rather than three times, and renders its `__bw<n>_` prefix without
  allocating. `log:conclusion` on a nested formula with no `<=` rule no longer registers that
  formula's scope salt: a purely forward nested closure can mint no name, so it no longer spends
  the run's backward identity allowance nor grows the run's tables, and the salt's value is
  unchanged either way. Every `__bw<n>_` name, derived fact, proof, `why()` string and
  `sparq-prov` IRI is byte-identical, and the W3C N3 inference conformance report is unchanged.

  **This does change which runs are refused, in the direction of refusing fewer.** Nested
  closures accumulate their salts run-wide against `max_join_rows × max_rounds` while each
  closure gets a fresh join/round allowance of its own, so a document with no `<=` rule
  anywhere but a deep CHAIN of nested `log:conclusion` closures could be refused with the
  `backward proof-node limit exceeded` cut for backward naming it can never perform. Such a
  document now answers under limits that previously refused it. A document that DOES have a
  backward rule and also nests forward-only closures can still be refused under tight limits,
  but the cut it is refused with may change from `backward proof-node limit exceeded` to a
  forward-closure limit. Wherever both the old and the new behaviour answer, the answers are
  identical; no run that previously answered is now refused.

- **sparq-core (`sq-ycle`):** hardens named-graph layout recovery. Stable manifest slots,
  checked allocation and removal, complete file-sync barriers, and ordered journal replay
  integrate with exclusive directory locks and role markers. Read-only opens do no cleanup.
  Whole-tree compaction keeps the existing rollback and frozen-view recovery behavior.
  Layout-generation lookups follow path ancestors; ordinary slot records do not scan the log.
  Dictionary metadata and named manifests write version 2 while reading version 1. A legacy
  single-file dictionary must be re-saved before atomic UPDATE journaling. Preserve a backup
  if an older executable must remain usable.

- **sparq-zk-compose:** hardening — the salt a HIDDEN-ONLY commitment's issuer-signed message is
  recomputed from is now resolved by AGREEMENT over every hidden-issuer entry declared over that
  commitment, instead of from whichever entry the prover listed first. All such entries must carry
  a salt, every one must decode to a field element, and all must decode to the SAME one (the hex
  spelling may differ, since agreement is compared on the decoded element). If any is absent or
  malformed, or two disagree, the salt does not resolve and the commitment is refused as
  unreferenced — the same fail-closed outcome a single missing salt already produced. The previous
  first-entry-wins resolution made that verdict depend on entry order (`[no salt, salt]` refused
  while `[salt, no salt]` proceeded, and a second, disagreeing salt was never read). Under
  `verify_manifest` the new rule refuses strictly more: the clear-attestation-preferred path is
  unchanged, and no manifest that verified before is newly refused unless it carried a salt-less,
  malformed-salt or disagreeing hidden-issuer entry over the same commitment. The structural
  prefilter alone is not a verifier: there, an unresolved salt now skips the salt-reuse check, and
  the full verifier refuses the commitment as unreferenced instead.
- **sparq-zk:** doc correction — `RegistryEntry::issued_with_status` documented its signed message
  as the method-unbound `commitment_message_with_status` (`ZKSIG_C3`); it signs the method-bound
  `commitment_message_with_status_and_scheme` (`ZKSIG_M3`), which is what the code already did and
  what `verify_commitment_signature_with_status` recomputes. No behaviour change.
- **sparq-zk:** hardening — the dual-leaf encoders now assert the §14.1 `LANG_NONE` separation
  on the host side instead of assuming it. `dual_leaf::degenerate_value_component` and
  `dual_leaf_v2::degenerate_value_component` return `Result<Fr, DualLeafError>` and refuse a language
  tag whose language constant equals the reserved `LANG_NONE` sentinel with the new
  `DualLeafError::ReservedLanguageConstant`, so such a tag fails the literal, term, triple and
  whole-graph encoders rather than being encoded; `checked_lang_const` (in both modules) is the gate.
  A language constant is a 248-bit truncated BLAKE3 digest and `LANG_NONE` is the field element `1`,
  so only a fixed-target preimage reaches the refusal. No committed value, vector or leaf changes. The
  in-circuit half of the assert landed with the component-witnessed `scan_v2dl_*` /
  `join_eq_v2dl_*` members, which no `CircuitId` names and so no verify path selects
  (`research/zk-field-native-encoding.md` §14.1, §14.3).

- **sparq-reason (N3):** rule existentials are now skolemized in a deterministic order, so for
  the same parsed input, and as long as the binding values and firing sets are themselves
  stable, their labels (`__sk<n>_<label>`), and the statement keys and `sparq-prov` entity IRIs
  of facts carrying them, no longer depend on hash-set traversal order or hasher width (native
  vs `wasm32`). Bindings that still carry traversal order (for example a `log:collectAllIn`
  list) can still change those keys (#6844). Existing labels are renumbered, so these keys and
  IRIs differ from those minted by earlier versions; regenerate stored ones. `why()` proof
  trees can still vary with traversal order. Ranking each round's existential firings adds a
  per-firing cost to rules with existential conclusions, since reduced (#6843). (#6749)

- **sparq-reason (N3):** the per-round ranking that makes existential skolem labels
  deterministic (#6749) now orders a round's firing keys by a packed prefix of the keys rather
  than by comparing whole keys, and a binding whose firing already minted its existentials no
  longer allocates its key. The labels, the statement keys and the proof steps are unchanged,
  byte for byte; only the cost of arriving at them is lower. Measured against the pre-#6749
  path on an existential-heavy closure, a wide-firing-key closure and a proof-mode run.
  (#6843)

- **sparq-zk:** `CommitmentMethod::DualLeafV1` is no longer production-selectable
  (`is_production_selectable()` is `false` for it), and `secprop::production_method_iris()` yields the
  V2 scheme IRI in its place. V1 stays recognised, decodable and verifiable for already-issued
  graphs; the new `CommitmentMethod::checked_for_issuance` refuses it fail-closed when a NEW entry
  selects a method. **Breaking — registry:** `RegistryEntry::scheme` is no longer a public field
  (read it through `RegistryEntry::scheme()`); the method is set only at construction, by
  `RegistryEntry::new_already_issued(doc, c, salt, method)` (records any recognised method, for
  decoding and already-issued entries), `RegistryEntry::new_for_issuance(…)` and the signed
  `RegistryEntry::{issued, issued_with_status}(…, method, …, sk)`, which return
  `Result<_, MethodNotSelectable>` and refuse a non-selectable method. `with_method` is removed with
  no replacement builder, so a recorded method cannot be rewritten after construction. The issuer
  signature now covers the recorded method, for every method including `string-canonical`:
  issuance signs the new `sig::commitment_message_with_scheme` /
  `commitment_message_with_status_and_scheme`, which bind `sig::scheme_iri_to_field(scheme)`,
  and `verify_commitment_signature{,_with_status}` recompute that message from the recorded
  scheme, so an entry whose `zk:scheme` was changed after signing fails verification and an
  unrecognised scheme fails closed. **Breaking — behaviour change:** a registry entry is never verified over the
  method-independent `sig::commitment_message_with_salt` / `commitment_message_with_status`
  message, so a registry signature made before this change does not verify. No such entry
  shipped: `sparq-zk` is unpublished, and no released artifact calls the registry issuance
  code. The manifest attestation path
  (`verifier`'s issuer-signature check) is a separate message and is unchanged. Retiring V1 in a
  deployed registry stays the operator's choice: an existing entry keeps its method. These host
  entry points take a `commit::MethodUse` (`NewIssuance` is the default and is refused for a V1
  method or constant; `LegacyAlreadyIssued` is the written-down opt-in for re-deriving or
  re-verifying an already-issued commitment, and is a CALLER ASSERTION — nothing reads a
  signature, entry or timestamp to confirm it): `dual_leaf::{commit_triples_dual,
  commit_graph_dual}` (new `DualCommitError::MethodNotSelectable`),
  `dispatch::resolve_circuit_for_use` (new `DispatchError::MethodNotSelectableForIssuance`) and
  `sparq-zk-compose`'s `driver::CircuitProver::prove_value_lane` (below).
  `dispatch::resolve_circuit` itself is unchanged — it is the verification view, so an
  already-issued graph keeps resolving. Per-term encoders (`dual_leaf::encode_literal`,
  `dual_leaf_boolean::encode_boolean`, `dual_leaf_datetime::{encode_datetime, encode_date}`, …) are
  deliberately NOT gated: they produce components, not a commitment, a recorded entry, a rendered
  `Prover.toml` or a proof.
- **sparq-zk-compose:** **Breaking — V1 value-lane helpers move behind `legacy_v1`.** Everything
  that produces a `DualLeafV1` value-lane constant is reachable only by naming the new
  `legacy_v1` module: `legacy_v1::{boolean_datatype_const, datetime_datatype_const,
  date_datatype_const}` (moved from `manifest`), `legacy_v1::{build_filter_value_dl_boolean,
  build_filter_value_dl_datetime, build_filter_value_dl_date}` (returning
  `legacy_v1::BuildValueDlV1Error::{Encode, Pairing}`) and the V1 renderers
  `legacy_v1::filter_value_dl{,_boolean,_f64,_decimal,_datetime}_prover_toml`, each of which
  admits V1 constants only (`DispatchError::NotALegacyV1LaneConst`). The ordinary names produce
  V2 only: `build::{build_filter_value_dl_datetime, build_filter_value_dl_date}` now encode the
  V2 dateTime/date lanes (returning `DualLeafError`), and every ordinary value-lane
  `toml::filter_value_dl*_prover_toml` drops its `MethodUse` argument, returns
  `Result<String, DispatchError>` and refuses a V1 constant. Verification-side V1 support is
  unchanged: raw `bb` verification of a V1 proof, `dispatch::resolve_circuit`, the verifier's
  id derivation and the `LegacyAlreadyIssued` pairing view all still accept V1 (the manifest
  pre-filter refuses every value-lane sub-proof, V1 or V2, as before).
- **sparq-zk-compose:** **Breaking — one typed checked prove entry.**
  `driver::CircuitProver::prove_value_lane(method, use_, challenge, &value_lane::ValueLaneProof,
  out_dir, tag)` replaces `prove_in_for_method` (removed, with
  `DriverError::ProverTomlLaneMismatch` and `driver::top_level_datatype_const`). It takes no
  circuit id and no TOML: the new `value_lane` module's `ValueLaneProof` (one typed variant per
  lane, built with `ValueLaneProof::from_inputs(&ProofInputs, LaneWitness)` or a builder's
  `lane_proof()`) carries the public inputs and private witnesses, the member is derived from the
  lane by `ValueLane::member` (through `ValueLaneProof::circuit_id`), the table
  `value_lane::derive_value_lane_member` and the verifier's id derivation also read, and the `Prover.toml` is rendered inside the entry by
  `ValueLaneProof::checked_prover_toml` after the `(method, member, use)` dispatch, the
  canonical-`FieldHex` check over every written field (challenge and private witnesses
  included), the lane↔member allow-list and the method/constant version agreement
  (`DispatchError::MethodLaneVersionMismatch`). A refusal precedes any `nargo`/`bb` subprocess.
  The raw `CircuitProver::{prove, prove_in, gen_witness*}` entries stay ungated and are
  documented as such. Every value-lane renderer, ordinary or legacy, refuses a non-canonical
  `FieldHex` in any field it writes, using the same decoder as the dispatch check, so the
  rendered text is always the flat form.
- **sparq-zk-compose:** each value-lane `datatype_const` is now bound to its own member by an
  AFFIRMATIVE allow-list decided on DECODED field elements.
  `dispatch::check_lane_const_for_member(&CircuitId, &FieldHex, MethodUse)` and
  `check_lane_member_pairing{,_for_use}` accept a constant on a member iff a shipped lane derives
  it for that member — V1 integer and V1 boolean on `filter_value_dl_int`, V1/V2 double on
  `filter_value_dl_f64`, V1/V2 decimal at any encodable scale (`fd ∈ 1..=19`) on
  `filter_value_dl_decimal`, V1/V2 dateTime and date on `filter_value_dl_datetime`, V2 integer on
  `filter_value_dl_int`, and the V2 boolean constant on `filter_value_dl_bool` and nothing else —
  with a V1 constant accepted only under `MethodUse::LegacyAlreadyIssued`. A constant no lane
  derives is refused on every member (`DispatchError::UnknownLaneConst`), a known constant on
  another member is `LaneMemberMismatch`, a V1 constant for new issuance is
  `LegacyLaneConstForIssuance`, and a text that is not the canonical rendering of the element it
  decodes to is `NonCanonicalFieldHex` (`field_from_hex_str` also accepts uppercase, `0x`-less,
  short and at-or-above-modulus spellings, which a text comparison would let through).
  The `ProofInputs`-level `check_lane_member_pairing{,_for_use}` checks `operand_enc` and
  `datatype_const` for canonical spelling, and every value-lane renderer and the checked prove
  entry check every `FieldHex` they write (public inputs, challenge and private witnesses) with
  the same decoder. The same pairing function runs at the value-lane builders, every value-lane
  renderer, `CircuitProver::prove_value_lane` and stage 0 of
  `verifier::prefilter_manifest_structure` (`CheckError::LaneMemberMismatch`). The
  `ProofInputs`-level check additionally requires the VARIANT and the declared `id` to name the
  same member (`DispatchError::ValueLaneVariantIdMismatch`), with the member derived from the
  variant by `value_lane::derive_value_lane_member`, the function the verifier's id derivation
  shares; a typed `ValueLaneProof` carries no separate id, so the checked prove entry cannot be
  handed a crossed pair. The raw `CircuitProver::{prove, prove_in, gen_witness*}` entries stay
  method- and lane-agnostic, so a caller rendering its own `Prover.toml` is outside this
  boundary. `datatype_const` is a public input no member pins, which is why the decision lives
  here. `check_lane_member_pairing_for_use` alone does not cross-check the method version
  against the constant version under `MethodUse::LegacyAlreadyIssued`; `prove_value_lane` does
  (a `DualLeafV1` method needs a V1 constant and a `DualLeafV2` method a V2 one; the
  research-only `ValueOnlyV1`, reachable only under `LegacyAlreadyIssued`, has no dual-leaf
  version and accepts either). This is a host
  boundary plus one structural pre-filter refusal: the value-lane verifier gates stay closed
  until the verifier authenticates the method and pins member, lane, scale and bound itself
  (sq-j506).
- **sparq-zk-compose:** `verifier::prefilter_manifest_structure` refuses EVERY value-lane
  sub-proof (`CheckError::ValueLaneSubProofNotAccepted`). The refusal is unconditional on the
  sub-proof being a value-lane member and inspects no edge; it is sound to refuse them wholesale
  because `filter_edge_true` discharges a query FILTER only through a `filter_int` edge, so no
  value-lane member can discharge an obligation in this verifier (that wiring is sq-j506).
  **Behaviour change:** such a manifest was previously ACCEPTED, with the sub-proof
  reconstructed and verified while discharging nothing. When sq-j506 wires the value-lane query
  binding, this refusal is what it replaces with a real edge check.
- **sparq-reason:** `MaterializedN3Graph::with_limits` builds the incremental N3 graph under
  a `ClosureLimits` (`new` uses the default). The counting path now sizes each builtin's
  argument list and result before building it and charges a round and fact allowance; past
  a limit the graph rebuilds through the batch engine, which refuses with
  `"n3: forward closure limit exceeded …"`, so the graph fails closed instead of growing a
  term every round. (#6937)
- **sparq-reason:** `ClosureLimits::max_join_rows` is an opt-in allowance for the
  transient work of one round of an N3 forward closure: the binding rows its premise joins
  produce and the times its rules fire, counted together. The default is unbounded, so
  every closure accepted before is still accepted; set a limit to opt in. The text engine,
  the `compiled-rules` evaluator and the incremental counting engine of
  `MaterializedN3Graph` charge each before it is stored; a round past a set allowance
  returns `"n3: forward closure limit exceeded … max_join_rows …"` instead of building rows
  that multiply with each unconstrained premise pattern. `ClosureLimits` is
  `#[non_exhaustive]`, so the new field is additive. The `compiled-rules` evaluator's
  first round of each stratum is now naive, as the text engine's is, instead of seeding
  each join position with the whole store. (#6951)

- **sparq-reason (N3):** a forward rule whose premise declares its own universal
  (`{ @forAll :x. :x :p :o } => { … }`, W3C `simple_forAll_premise.n3`) now fires where
  the store proves the quantified premise, instead of being refused. A premise universal
  binds only a universal of the store: a document-level `@forAll` one, or inside a
  `log:conclusion` / `log:supports` run that formula's own (never an enclosing formula's).
  Distinct premise universals need distinct store universals, and no other variable of the
  firing may mention one, at any depth, or bind a value holding a blank node (an
  existential whose witness may depend on it). A ground fact never proves it. Supported where the
  universal is a subject or object (or in a list or triple term there) of a
  stored-predicate atom, in a document with no backward rules; every other placement (or a
  premise that also holds a blank constant, as a rule inside a closed formula can), and
  an N3 query document, is still refused with the same error. The `compiled-rules` engine
  compiles such a rule as one that never fires (its store is ground), and
  `MaterializedN3Graph` evaluates it in fallback mode. (#6882)

- **sparq-core:** `net::is_forbidden_ip` is the one outbound-address classifier behind the
  SSRF guards of SERVICE, sparq-fedclient and the LWS federation client, which re-export it
  under the names they had; its answers are unchanged (#6927).
  `Graph::is_directory_backed()` reports whether a graph or any named graph under it is
  opened from a directory; it is always `false` without the `mmap` feature (#6944).
- **sparq-solid:** an ODRL permission attached to a pod document now grants per request
  through `sparq_policy::decide`, for a target that exists and has a full static decision:
  Read on `odrl:read` (#6941); Write when one policy permits `odrl:modify` and
  `odrl:delete`, and Append on `odrl:append` or wherever Write is granted (#6942), each
  from a rule that names the action, so an `odrl:use` permission still grants Read only.
  Write and Append are never granted on a container or a referenced group document, and
  apply only in an update made of `INSERT DATA`, `DELETE DATA` and `DELETE`/`INSERT …
  WHERE`: attached policies grant data changes, not graph management (`CLEAR`, `DROP`,
  `CREATE`, `LOAD`, `COPY`, `MOVE`), and `PodStore::decide_removal` (used by the MCP
  `resource_delete` tool) ignores them. No permission grants a create or Control, and a
  prohibition or an explicit deny still vetoes.
- **Security-property vocabulary:** two ordered dimensions, `secx:AdversaryModel`
  (Malicious ⊐ Covert ⊐ SemiHonest) and `secx:CorruptionThreshold` (DishonestMajority ⊐
  HonestMajority ⊐ SuperHonestMajority), with ODRL leftOperands
  `secx:requiresAdversaryModel` / `secx:requiresCorruptionThreshold` and their order chains
  in the admissibility ruleset. sparq-mpc ships a per-protocol annotation graph behind the
  default-off `secprop-annotations` feature. The change is additive, but the serialised
  `sparq-secprop-vocab` registry, the ODRL profile Turtle and the admissibility ruleset text
  gain the new facts, so anything that pins those outputs byte for byte will see them
  (#6977).
- **sparq-reason (N3):** matching a quoted-formula pattern against a quoted-formula value
  now has a step budget for its backtracking over statement pairings. A match that never
  backtracks is not limited; past the budget the formula does not match and the run
  records a cut (`formula unification exhausted its step budget`), so a negation or
  aggregation in that run is refused. (#6953)
- **sparq-engine / sparq-fedclient:** SERVICE and federation requests no longer use proxy
  environment variables (`HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY`) and have no proxy support,
  so the address the egress resolver vets is always the address that is dialled.
- **sparq-reason (N3):** a run already refused by a `ClosureLimits` limit no longer reports
  a negation or aggregation evaluated after the refusal ("refused a negation or aggregation
  … over an incomplete set"); it fails with the refusal itself, which names the limit
  (for example `… over max_term_bytes (…)`). Both are refusals: only the error text
  changes. (#6951)
- **sparq-reason (N3):** formula-level `@forAll` declarations keep their own scope,
  including nested and sibling quoted formulas. Naive, compiled and incremental engines
  reject `@forAll` in a rule's own premise or a scoped body pattern with a specific error;
  a `log:forAllIn` / `log:collectAllIn` clause or a `log:includes` / `log:notIncludes`
  pattern bound through a variable is checked when it is evaluated and fails the run the
  same way. Templates and other quoted values keep their own universals at any nesting
  depth. A document parsed by `log:parsedAsN3` / `log:semantics` declares its top-level
  `@forAll` in the resulting formula, the same term as the inline `{ @forAll … }`.
  Quoted formula terms retain identity matching, conclusion universals keep their own
  scope, and closure-introduced universals are restored in the result formula.
  Document-level `@forAll` is unchanged. Exact writers place each formula-level
  declaration inside its formula. (#6851)
- **Breaking (sparq-reason, `compiled-rules` feature):** `n3::compiled::eval` and
  `BoundRuleSet::eval` now return `Result<Vec<[Id; 3]>, String>` and run under the default
  `ClosureLimits`; new `n3::compiled::eval_with_limits` and `BoundRuleSet::eval_with_limits`
  take a `ClosureLimits`. A compiled closure past its round, fact or term-size allowance
  returns `Err("n3: forward closure limit exceeded …")` instead of running until memory
  runs out. (#6934)
- **Breaking (sparq-reason):** `ClosureLimits` has a new field, `max_term_bytes` (default
  16 MiB), and is now `#[non_exhaustive]`, so it can no longer be built with a struct literal
  outside the crate. Start from the default and set fields:
  `let mut l = ClosureLimits::default(); l.max_term_bytes = 1 << 20;`. A forward N3 closure
  whose rules would build a term past the limit (a string or list that doubles each round)
  now returns `Err("n3: forward closure limit exceeded …")` instead of aborting on a failed
  allocation. (#6833)
- **Breaking (sparq-fedclient):** pushed-down `VALUES` cells, `ORDER BY` keys and filter
  conjuncts are typed and checked instead of inserted as text. `render_values_block` takes
  `&[Vec<Term>]` and returns `Result<String, RenderError>`, rendering each cell with the
  strict term renderer; `RenderError` has a new `Values` variant; `push_group` takes
  `&[OrderKey]` (`OrderKey::asc("x")`, `OrderKey::desc("x")`) and returns `None` for an
  invalid key. A filter conjunct is pushed only when its text parses as exactly one SPARQL
  expression over its declared variables, with no `EXISTS` and no `\u`/`\U` escape;
  otherwise it is evaluated locally. A group with any local conjunct projects all its
  variables and is sent without its `LIMIT`, so local evaluation sees every row it needs.
  (#6949)
- **sparq-reason (N3):** the blank nodes of a document parsed at run time
  (`log:parsedAsN3`, `log:semantics`) are standardized apart as `__rp{k}_{key}_<label>`,
  where the key is derived from the document's text and base with SHA-256. Before, every
  such document's `_:a` was one node, shared with the rules document's `_:a`. The label is
  the same on every build, in a materialization and in `why()`, and for a
  `MaterializedN3Graph`'s whole life; re-reasoning exported output is a fixed point. A
  text that spells a graph's caller prefix (`__bs{k}_`) is no longer refused. A run or graph
  keeps at most 100,000 distinct runtime documents; one more fails the run. Documents with
  no runtime parse evaluate as before. (#6867)
- **sparq-solid:** a multi-operation update that includes a `DELETE`/`INSERT … WHERE` with a
  `GRAPH ?var` target now runs, all-or-nothing, on a store with no directory behind it: each
  operation is authorized against what it writes, and one unwritable target refuses the
  whole request. A directory-backed store still refuses such a request, and its error names
  the reason. `CLEAR NAMED`/`DROP NAMED` now also needs Write on graphs that earlier
  operations of the same request wrote or created. (#6944)
- **SERVICE:** both results readers build terms with one shared literal and triple-term
  builder. An SRX triple term missing its subject or predicate reports `triple term without
  subject` / `predicate`, as SRJ does, and an invalid one is printed with `Display`. A
  `<binding>` whose `<variable>` is declared only after the results have started is
  ignored. (#6905)
- **Dependencies:** sparq-fedclient drops its unused direct `serde` dependency and
  serde_json's `raw_value` feature (#6911); sparq-engine-service drops `rustc-hash` (#6917).

### Fixed

- **sparq-lws-core:** access grant/request lookups hide records owned by another principal;
  subscription lookups clone only owned records. An unconditional LWS data DELETE allowed by an
  unconstrained Delete grant without Read returns `204` for settled present or absent targets,
  and for an absent prior during in-flight or set-aside creates, including anonymous callers
  with public Delete grants. Read on an absent target is decided without stored metadata,
  including partial create metadata and orphan creator records. Concurrent removals use the
  settled-absent decision, including under the delete lock; a full set-aside budget returns
  `503` with `Retry-After`, before checking whether the parent is set aside.
  A set-aside change to a previously present resource retains `503`.
  A set-aside parent container returns `503` for both present and absent children.
  An absent delete changes and announces nothing. Pending metadata on a present resource and
  type- or format-constrained Delete grants remain residual differences from absence.
  Resource-linkset OPTIONS returns fixed `204` for every caller; unsupported methods return `405`,
  both before backend reads, with `Allow`, `Accept-Patch` and the anchor `Link`. For readers too,
  OPTIONS now returns `204` where an absent target previously returned `404` and a set-aside
  target `503`. Anonymous grant denials skip metadata reads. For a non-creator, malformed
  unrelated metadata fields such as `links` or `pending` no longer make `allowed()` answer `500`;
  its fast path returns false. Raw index mutations refuse service-marker operands; marker resets
  require a lifecycle guard.

- **sparq-zk-compose:** hardening: method-fold refusals now apply in every build.
  Default builds now return `SubProofCommitmentNotAuthenticated` for a bound
  commitment absent from the authenticated-method map, and `SubProofMethodUnresolved`
  when bound commitments disagree or deferred resolution cannot complete after the
  hidden-issuer gate. The `dual-leaf` dispatch matrix remains opt-in.
- **sparq-lws-core (LWS records and notifications, follow-up to #7174):** putting a change back
  while a request waits on it tries each step up to three times, each attempt waiting for a
  fence two seconds at most (counted from the store's answer); a step whose write failed
  unsettled and whose fence has not succeeded is set aside, its fence first, and never tried
  again before that fence succeeds, in the request or in the background (the background
  put-back and the start-up settling still wait as long as the store takes). A removal whose
  fence lands with the record still stored is answered at once from the listing; one whose
  outcome cannot be read after its fence stays out of force and is settled in the background.
  A journal's commit fences an unsettled clearing of its intent before reading the intent. A service-container
  create whose precondition compares no state (a bare `If-Match: *`) takes the container shared,
  as an unconditional create does, so it no longer waits on a removal being settled or holds up
  other creates. A background fence's attempts are capped at the store's own call timeout
  (`Store::call_timeout`, new, defaulting to `STORE_CALL_TIMEOUT`; the `http` index reports the
  timeout it was built with), clamped to between two seconds and one hour per attempt.
  Forward removals and intent creates keep their unsettled fences through rollback and commit.
  A settled removal whose outcome cannot be read stays out of force until the listing answers.
  A WebSocket socket closes `1011` for the client to reconcile at the
  next point where the subscriber may read the topic after any notification was dropped
  (whether it could not be read or its check could not be made), instead of delivering later
  ones; a receive token has at most one socket open at a time (a second connect gets `401`); pings
  are answered while an access check runs. A due tick takes priority over inbound frames and
  queued notifications, pre-empts a running notification or lag check and notes it as missed:
  a reader gets `RECONCILE` for either dropped check, rather than `OVERFLOW` for a dropped lag
  check. Within two periods of losing Read the socket closes: `1008` for a confirmed denial,
  or `1011` if the timer check is undecided. Each check has at most one period; a slower check
  counts as undecided and the client must resubscribe. An LDP write also refuses a dot segment around a
  %-decoded slash (`..%2Fx`). A topic being created is decided as a URI where nothing is,
  whatever metadata the change kept.

- **sparq-lws-core (LWS notifications, type index, records):** notifications follow the
  subscriber's current access. A WebSocket notification the subscriber may no longer read is
  not sent and decides no close; an access timer (every 30 seconds, `ACCESS_RECHECK`) closes the
  socket (1008) once read access is gone; a check that cannot be made closes it `1011` (on the
  timer) or drops the notification and asks a subscriber that may read to reconcile
  (`1011`) at the next tick. The receive token gates connecting only: an open socket is not
  closed when it expires, and an expired token never connects again in the process. A webhook
  subscription decides access before asking whether its topic exists, and a topic being created
  is decided with nothing read of it. The type index decides read access before anything else is
  asked of a resource, passes over one the caller may not read as one not there (failing only
  when a grant names the caller itself), and charges linkset-record names nothing against its
  budget. A record removal waits for its fence at most as long as for a lock in the request, and
  in the background for longer each attempt up to the store's call timeout
  (`STORE_CALL_TIMEOUT`); it keeps the container held until a removal left to settle in the
  background is settled, and answers `507` when the store refuses it for want of room; a
  subscription being cancelled hears nothing until it is back in force; a subscription create
  decides its subscriber's due subscriptions before counting. An LDP write refuses a dot segment
  a URL normaliser could resolve away (%-decoded, or between backslashes) with `400`; what is
  stored under one can still be read and deleted.

- **sparq-reason (N3):** with a `{ … }` formula (or `{}`) as the scope, a `log:includes`
  / `log:notIncludes` / `log:supports` pattern given through a variable DIRECTLY bound to
  a stored formula that holds no variable at any depth is matched as that formula, as
  stored (its blank nodes are its existentials), the same as the formula written in the
  rule; its statements are charged to the round's `max_join_rows` allowance. It was
  matched as the empty pattern, so `log:includes` held whatever the formula held and
  `log:notIncludes` never fired. Every other value (a formula holding a variable, a
  non-formula value, an unbound variable) and every bound pattern with the store as the
  scope still match as the empty pattern (#6943). Each containment matches its pattern's
  blank nodes afresh: a witness found by one `log:includes` no longer constrains the next
  one in the premise. (refs #6936, #6943)

- **sparq-reason (N3):** three more N3 outputs no longer depend on hash-set traversal order,
  so the same input gives byte-identical labels, proofs and `term_key` / `statement_keys` on
  every run and every target. A `log:conclusion` / `log:supports` closure's DERIVED rows are
  sorted into a canonical term order instead of the nested closure's fact-set order (a
  formula's row order is part of its term identity). The rdf:first/rest cell labels minted
  when a list value is interned (`_l<k>_<n>`) are numbered by interning every reachable list
  value up front in that same order, instead of from a counter bumped as the rows arrived.
  And where a round derives ONE fact several ways — a rule whose conclusion is symmetric or
  has several triples firing under distinct bindings, two different rules, or the transitivity
  fast path walking several intermediate paths — the proof step records the derivation with the
  least premise vector rather than whichever one arrived first, so `why()` names the same
  supporting facts every run. (refs #6844)

- **sparq-reason (N3):** the backward chainer's freshened variable names (`__bw<n>_`) now
  address the DERIVATION they standardize apart, instead of counting applications. A rule
  conclusion can bind a goal variable to a quoted formula that still holds that application's
  renamed variables, and that value reaches a derived fact — so the name was part of the
  fact's term identity (its `term_key` / `statement_keys`, its `why()` string, its
  `sparq-prov` IRI) while the counter's value depended on how many applications had run
  before it, and so on a fact set's traversal order and the hasher's width (native vs
  `wasm32`). A name is now the FINGERPRINT of the derivation: a 64-bit fold of the evaluation
  scope, the node's place in the proof tree (the premise-atom index, then the
  `(rule, conclusion)` site index, plus a scoped operator's clause position) and the goal
  being proved — folded step by step, each step from the one above it, so neither the name nor
  the work of building it grows with the depth of the proof — rendered in the `__bw<n>_` form the readers already accept, so no writer, key
  or refusal changes shape, and a name's length no longer grows with the depth of the proof.
  Two derivations that fold alike are refused — the run fails rather than let them share one
  set of renamed variables, and the table that detects it belongs to the whole top-level run,
  so a collision between a data closure, a query's own evaluation and a nested
  `log:conclusion` closure is caught too. Every part a name is built from is registered the
  same way, the goal included: it enters the name as an INTERNED id, not as a bare fold, so
  two different goals cannot merge by folding alike, and it is identified from its FULLY
  RESOLVED form — every variable walked to the end of its binding chain, through lists,
  triple terms AND quoted formulas — so two goals that differ only past one binding (a
  repeated query variable aliasing two renamed ones), or only inside a quoted formula whose
  variable the caller bound and a `log:includes` premise reads, stay two goals instead of
  merging and losing a row. Two branches that reach the SAME resolved goal do share one
  renaming: their proofs' formula values are then one term rather than two α-variants, so a
  query projecting only the proved value can return one row where the counter returned two
  — the same collapse that makes the #6757 programs saturate. A branch the conclusion
  projects still gets its own row. Sibling nested closures at one nesting
  depth are also told apart, by the nested document's canonical key — its `term_key`s folded
  in explicit `u64` arithmetic, NOT the pointer-width-dependent hash the import-cycle guard
  uses, so a nested closure's names agree between native and `wasm32` as well. **A run can
  now be refused for this:** the nodes and identities a run registers are charged to
  `ClosureLimits::max_join_rows` × `ClosureLimits::max_rounds` — the per-round row allowance
  times the rounds the run may take, since the tables accumulate over every round — and the
  entry that would be one too many fails the run with a cut; a `(rule, conclusion)` site is
  registered only once its conclusion unifies with the goal, so the sites a search rejects
  cost nothing, and the DEFAULT `max_join_rows` is the unbounded sentinel, so a
  default-limits run is never refused for it, and an evaluation with no `<=` rule in it
  registers nothing at all, since no name it could mint would ever rename a variable. The
  resolution that identifies a goal leaves a variable it is already inside the expansion of
  as it is — a formula bound to `?x` may name `?x` itself, and that occurrence is the
  formula's own — so a self-naming formula resolves to a finite term instead of being
  refused; a goal the resolution does refuse (its depth or its term size) now FAILS the run
  with that reason, where it used to come back as an empty proof that read as "not
  provable"; and a site's fingerprint is checked against the run's table before it renames
  anything, since a fold already held by another key renames the conclusion into names the
  inherited bindings hold, which fails unification and would otherwise skip the check
  entirely. Three
  consequences: a fact carrying
  a renamed variable has a different identity than before, so regenerate stored `sparq-prov`
  IRIs for such facts; proofs of different goals keep the distinct values they had, including
  values inside a quoted formula, which substitution does not reach; and a program whose only
  round-to-round growth was that counter (the #6757 shape: a backward rule concluding a
  formula with a variable, a forward rule copying it out) now SATURATES instead of being
  refused at the closure limit, because re-proving one goal mints one name. A program that
  diverges by growing its terms is still refused. (#6844)

- **sparq-reason:** `MaterializedN3Graph::new` / `with_limits` now ignore a `base_facts`
  entry with a variable outside a formula, as `insert` already does, instead of adding it
  to the base.

- **sparq-reason (N3):** a rule whose existential conclusion contains a quoted formula with
  variables (e.g. `{ ?x :p :v } => { _:e :q { ?x :r :v } }`) now fires once per distinct
  binding of those variables. Before, it could fire only once and drop the other bindings'
  conclusions, so such closures can now be larger and may reach closure limits sooner. (#6749)

- **sparq-reason:** deleting a fact that the rules document itself asserts from a
  `MaterializedN3Graph` now sticks when the graph re-runs the batch engine (fallback mode),
  and `why()` no longer cites such a deleted fact as a premise. The re-run took the
  document's facts from its source text as well as from the graph's base.

- **sparq-policy:** `contains` and `detect_conflicts` now prove containment and certain
  overlap through `odrl:and` / `odrl:or` / `odrl:xone` compound constraints where the
  runtime semantics decide it, instead of always answering `Unknown` / `Possible`; they
  still never report `Contains` or `Certain` a compound cannot prove (#6985).
- **sparq-solid:** blank nodes in ACL/ACR documents are skolemized on the document's IRI
  instead of its position, so writing or deleting one document no longer renumbers another
  document's skolem IRIs and clears the authorization cache (#6960).
- **sparq-reason (N3):** `ClosureLimits::max_term_bytes` now also covers numeric builtin
  results, forward and reverse, which are sized from their value before the literal is
  rendered (#6929), and the parse behind `log:parsedAsN3` and `log:semantics`, which is
  charged as it builds terms, so a short source whose prefixes or copied terms expand is
  refused before the expanded formula is built (#6894). The document's own parse stays
  uncharged.
- **sparq-engine:** a `SELECT` whose deadline passes or that is cancelled while its result
  terms are materialised returns the budget error instead of a complete, late result. This
  covers `query`, `query_with_budget`, the `WHERE` step of `CONSTRUCT`/`DESCRIBE` and
  `EXPLAIN ANALYZE`. (#6946)
- **SERVICE (results XML):** elements and attributes are matched by namespace as well as
  local name, so an element in another namespace or with an undeclared prefix is ignored and
  can no longer stand in for a result, binding or term, or cut a literal short (#6932);
  `its:dir` sets a literal's direction only when `its` is declared as the ITS namespace
  (#6935); triple terms nested more than 61 deep are refused, the depth SRJ admits (#6916).
- **sparq-zk:** `ZkDataset::query_traced` refuses a query with a `FROM` or `FROM NAMED`
  clause, with the verifier's reason, instead of returning an empty trace. (#6950)
- **sparq-solid:** a graph named by a bare `scheme://` (for example `<https://>`) is no
  longer seeded as the container of every pod root, so its `.acl`/`.acr` no longer take part
  in their inheritance. (#6939)
- **LWS:** extension link relations compare case-insensitively wherever they are stored,
  matched or searched, as RFC 8288 requires (#6899); the storage owner is notified of a
  Delete whose prior state cannot be read, and every other non-open subscriber is still
  refused (#6912); a listing has no `Last-Modified` when the container's or any member's
  time is unknown, a member of unknown time lists no `modified`, and a linkset's `self`
  entry omits `modified` instead of dating it the epoch (#6919, #6924); while a journaled
  change is being put back, requests and subscription topic checks are decided on the
  resource as it was before the change, so a caller only the change would admit gets `403`
  at once instead of `503` (#6925).

## [0.1.6] - 2026-10-13

First release with the `sparq-rdf` PyPI wheels (the v0.1.5 wheel builds failed; npm 0.1.5
packages were published). Publication is not implied by this entry; verify the tagged run
and every registry and release artifact.

The entries for #6869 and #6897 were added after the v0.1.6 release commit (`66bebdb3`).
Both changes are in that commit, but its copy of this file does not list them.

### Added

- **sparq-lws-core:** the type index and `QUERY` type search, paged, listed in the storage
  description. Experimental. (#6650)
- **sparq-lws-core:** the token endpoint accepts OpenID Connect ID tokens, with DPoP proofs
  for bound ones. (#6721)
- **LWS authentication suites as packages:** `sparq-lws-authn` defines the suite interface;
  `sparq-lws-authn-saml` adds SAML 2.0 subject tokens, `sparq-lws-authn-openid` holds the
  OpenID Connect suite and `sparq-lws-authn-ssi` the did:key and controlled identifier
  suites. A deployment chooses which suites the token endpoint offers. Experimental, not
  published to crates.io. (#6819, #6820, #6821)
- **CI:** hosted previews of the site, specs and papers for pull requests that change them.
  (#6818)
- **Research prototypes** (proof-methods/, not part of the shipped runtime): a Noir opening
  written with field-constraint rotations and additions (#6815), and tamper checks for both
  Noir openings plus a gate-count script (#6816).

- **sparq-e2ee-ng:** `PublicGrant::check_epoch` enforces a grant's signed `max_epoch`
  ceiling. (#6853)
- **Research prototypes** (zk/, not part of the shipped runtime): one variable-length BLAKE3
  opening for the 64-bit integer result circuits (#6829).

### Changed

- **LWS token endpoint:** a SAML 2.0 subject token is refused unless the assertion's
  `NameID` is an http(s) URL. Every subject token's expiry is checked again just before an
  access token is issued, still with the clock-skew allowance, so a token that expires while
  its documents are fetched is refused. (#6819, #6821)
- **sparq-trust (status lists):** `decode_encoded_list` now rejects a `u` (base64url) body
  whose length is ≡ 1 mod 4 even when the leftover bits are zero (for example `ugAAAA`);
  such a body has no byte-string meaning, and every other non-canonical form was already
  refused. A list published by a conforming encoder is unaffected. (#6873)
- **Breaking (sparq-fedclient):** pushed-down SPARQL renders every pattern position strictly as
  one SPARQL term. `lower_leaf` now returns `Result<SubQuery, RenderError>`, the interpreters
  report `InterpError::Render`, and `push_group` returns `None` for a pattern holding a
  variable name that is not a SPARQL `VARNAME`, an IRI with a character `IRIREF` forbids, or a
  `"`-prefixed literal that is not exactly one SPARQL literal. Pre-rendered literals are
  re-serialised with canonical escaping. (#6877)
- **Breaking (sparq-e2ee-ng):** `encode_secret` returns `Zeroizing<Vec<u8>>`, built in one
  buffer that is wiped on drop (#6862). `Commit::decode` refuses a parent set that is not
  sorted and duplicate-free, and `Commit::encode` writes it that way (#6845).
- **Breaking (sparq-lws-core):** `parse_links` returns an iterator of `Link` values parsed on
  demand, so the header caps bound the parsing work (#6839). The notification sender takes a
  `Prepared` batch; `Notifier::dropped()` counts only notifications that were owed, and
  subscriptions past the delivery bound are counted by `Notifier::unprepared()` (#6841).
- **sparq-reason:** deleting from a materialized graph with an RDFS meta schema runs an
  incremental DRed delete instead of rebuilding the closure. (#6842)
- **sparq-solid:** a rules-free `DELETE`/`INSERT … WHERE` update commits without forking the
  store. (#6830)
- **SERVICE and sparq-fedclient** share one SPARQL results decoder and SRJ driver, one SSRF
  resolver, and one pattern and N-Triples term renderer; sparq-fedclient parses federation
  results with the engine's streaming SRJ reader. (#6840, #6872, #6879, #6884, #6895, #6896,
  #6898)
- **sparq-core:** shared hex, percent, JSON-string, civil-date, base58btc and base64url codecs
  replace per-crate copies. (#6850, #6864, #6866, #6868, #6873)
- **sparq-shacl:** one JS report renderer replaces the two wasm copies, with public
  `to_js_json` and `message_text` helpers on the report. (#6856)
- **CI:** zk-native-composition also runs when `supply-chain/config.toml` changes (#6861); the
  LWS.NET suite's own 57 test definitions run as a ratchet (#6831).

### Fixed

- **Release pipeline:** release files are staged in an empty `release-assets/` directory.
  v0.1.5 published the repository's `assets/logo.svg` as an extra asset with no provenance;
  every other v0.1.5 asset is covered by the isolated-builder bundles. (#6812)
- **Publish pipeline:** the `sparq-rdf` wheel build no longer passes `--release` with
  `--profile`, which current maturin rejects, and the npm provenance checks wait for the
  registry to serve the new version. (#6817)
- **sparq-solid / sparq-prov (date stamps):** epoch seconds before 0000-03-01 now format to the
  correct proleptic-Gregorian date. The previous conversion was one day late there (and could
  emit 29 February in a non-leap year); the only dates it already got right were
  29 February of years divisible by 400. For such instants a trust-graph grant's
  `auth:notAfter` and its deterministic grant IRI change (for example `-62167219200` now formats
  as `0000-01-01`, not `0000-01-02`), and years -1 to -999 are now written with four digits
  after the sign (`-0001`, not `-001`), as XSD requires. Instants from 0000-03-01 onwards are
  unchanged; signature inputs and PROV IRIs are unaffected. (#6864)
- **SSRF:** the egress classifier shared by SERVICE, sparq-fedclient and the LWS federation
  client refuses every non-global address range. (#6852, #6870)
- **LWS:** SAML subject confirmations are alternatives, as SAML Core defines them: one
  satisfied bearer confirmation confirms the assertion (#6827); the server checks authorization before revealing set-aside or lock state
  (#6848); a store write that times out is treated as having an unknown outcome (#6860); the
  server binary forbids unsafe code (#6876).
- **sparq-mpc:** federated plans fail closed when a participant has no privacy policy. (#6825)
- **JavaScript:** the remaining RDF and SPARQL text builders validate interpolated terms
  against the grammar. (#6826)
- **sparq-reason:** the N3 nesting limit fits a 2 MiB thread stack, so deeply nested input is
  refused instead of overflowing the stack. (#6835)
- **sparq-reason:** base-graph blank nodes stay distinct from blank labels the N3 parser
  allocates. (#6779)
- **sparq-reason:** the N3 evaluator bounds its stack use on untrusted input. Under the default
  policy, terms built past 128 levels are dropped as cuts, nested closures and proof steps
  share one stack allowance per run, and a run with cuts refuses a later negation or
  aggregation instead of reading missing facts. (#6869)
- **sparq-fedclient:** the ASK reachability probe keeps an endpoint's existing query string
  and replaces only its `query` parameter. (#6897)
- **LWS:** a type-search filter too long to page is refused as too complex (422). (#6889)
- **sparq-shacl:** `LATERAL` blocks are covered by the SHACL-SPARQL pre-binding checks and the
  `VALUES` push-down. (#6846)
- **JSON-LD conformance:** the oracles compare arrays inside `@json` literals in order. (#6857)
- **CI:** the scheduled and full-sweep runs are green again (#6824, #6837), and the nightly
  visual sweep builds its inputs first and pushes re-minted baselines to a review branch
  (#6858, #6887, #6890, #6892, #6893).

## [0.1.5] - 2026-10-10

Fix-forward release after the v0.1.4 release workflow failed before creating a GitHub
Release (no v0.1.4 assets, npm or PyPI packages were published). Publication is not
implied by this entry; verify the tagged run and every registry and release artifact.

### Fixed

- **Release pipeline:** the Windows matrix rows write their archive, binary and GUI bundle
  digests in text mode (`<hex>  <name>`). Git Bash's binary-mode `*<name>` record failed the
  archive-digest collector, which stopped the v0.1.4 run before provenance and the GitHub
  Release. (#6717)
- **sparq-policy / sparq-solid (ODRL):** hardened ODRL policy evaluation and the ODRL
  authorization bridge. Some policies accepted by v0.1.4 are now rejected at parse time.
  (#6734, #6737)
- **sparq-solid:** a SPARQL Update's WHERE clause is now evaluated against the session's read
  view, the same as a query, and graphs named in `USING` / `USING NAMED` need Read access.
  (#6763)
- **sparq-solid:** a SPARQL Update request with more than one operation can no longer include a
  `DELETE`/`INSERT … WHERE` with a variable `GRAPH` target; send that operation on its own.
  (#6763)
- **sparq-server / sparq-lws-core (ODRL):** rules that target an `odrl:AssetCollection` now
  apply to the collection's `odrl:partOf` members on the server's ODRL lane and the LWS ODRL
  gate. (#6773)
- **sparq-solid:** if rebuilding the authorization view fails after an accepted `.acl` / `.acr`
  change, the store denies every request until a rebuild succeeds, instead of continuing to
  enforce the previous rules. (#6742)
- **sparq-reason:** `@forAll` universals are named by their full IRI, so `@forAll a:x, b:x`
  are two variables and `reason_n3_pass_all` no longer prints internal `__ua_` names. Exact
  writers refuse a document that would not re-read the same way. (#6701)
- **sparq-reason:** a forward N3 closure that never reaches a fixpoint stops with an error once
  it exceeds the configurable `ClosureLimits` round or fact count. The limits count rounds and
  facts, not bytes, so rules that grow a term each round can still use unbounded memory. (#6769)
- **sparq-reason:** `math:` comparisons promote numeric types as XPath does. (#6772)
- **sparq-reason:** the RDFS closure reaches its fixpoint when schema properties have schemas
  of their own, including with `why()` and incremental updates. (#6736)
- **sparq-core / sparq-engine:** `xsd:dateTimeStamp` without a timezone is ill-formed (#6725),
  and XSD 1.1 year zero is accepted (#6747).
- **sparq-engine:** integers and decimals beyond `i128` order by value (#6697), `ROUND` keeps a
  negative zero (#6703), CONSTRUCT and DESCRIBE go through `PreparedQuery` and honour `BASE`
  (#6677), and the query budget bounds CONSTRUCT / DESCRIBE N-Triples output (#6714).
- **sparq-canon:** tied RDFC-1.0 N-degree hashes break ties independently of blank-node labels
  (#6645), and RDF 1.2 permutation enumeration is lazy and budgeted (#6702).
- **sparq-shacl:** date and dateTime comparisons use the shared exact temporal comparator.
  (#6733)
- **sparq-server:** CSV, TSV and XML SELECT results are streamed instead of rendered whole.
  (#6708)
- **sparq-lws-core:** durable intents restore changes that a process stop interrupted. (#6767)
- **sparq-client:** the shared N-Triples writer validates term parts. (#6716)
- **@sparq-org/sparq:** `@sparq-org/sparq/wasm/*` resolves through the package `exports` map,
  and the wasm copy step runs on any platform Node runs on. (#6739)
- **sparq-forms:** update terms that can't be written safely are rejected. (#6707)
- **Desktop GUI:** Graph view refuses UPDATE (#6704), full-text search terms and SHACL report
  strings are escaped (#6741), and the ODRL preview shows the policy's own per-request verdict
  (#6760).
- **Site and wasm:** horizontally scrolling tables and code blocks are keyboard-focusable
  (#6777), and the parser size optimisation applies again after the spargebra rename (#6782).
- **Dependencies:** Rust dependency updates in the root and side lockfiles (#6694), and an
  npm override for `basic-ftp` (#6751).

### Added

- **sparq-solid:** `PodStore::attach_odrl_policy` keeps an ODRL policy on the store and
  evaluates its prohibitions for the requesting session on every read, query, decision and
  update path. (#6759)
- **sparq-lws-core:** `SOLID_SERVER_PROTOCOL=lws` serves the LWS 1.0 storage surface:
  storage description, paged containers, data resources with preconditions and byte ranges,
  `POST` / `PUT` / `DELETE` and linksets. Experimental. (#6728) It also accepts JSON Patch on
  data resources (#6805) and linksets (#6806), and serves access grants and access requests
  (#6730) and webhook notifications (#6731).
- **Research prototypes** (zk/ and proof-methods/, not part of the shipped runtime): the
  stacked ZK and credential-query work (#6647), a revealed-signature mode for the zkVM
  eddsa-rdfc-2022 relation (#6787), the `eddsa-sha256-merkle-2026` Merkle-root cryptosuite and
  spec (#6789), a disclosed-credential re-evaluation method (#6788), a QuickSilver prover for
  the zk/compose circuits (#6790), an AWS Nitro Enclaves attestation method (#6794), a paper
  measurement harness and executor evidence records (#6791, #6793), and a hand-written ACIR
  prototype for the integer literal opening (#6803).
- **sparq-proved-evaluator (research):** the V5 host rejects DESCRIBE and dataset clauses
  (`FROM` / `FROM NAMED`). (#6792)
- `sparq-vectors` can open canonical little-endian `.spqv` stores on big-endian hosts by
  validating the file and byte-swapping its dense f32 region into aligned owned storage; `.spqv`
  writers and the `.spqg` reader/writer remain little-endian-host only.

### Changed

- **Breaking (sparq-reason):** an N3 document whose rules cycle through negation
  (`log:notIncludes`, `log:collectAllIn`, `log:forAllIn` over a predicate the same document
  derives) now returns an error naming the negated predicate, at every N3 entry point. Acyclic
  documents are evaluated stratum by stratum. The `*_with_cycles` variants opt in to the
  previous acceptance. (#6705)
- **Breaking (sparq-engine, JSON-LD compaction):** `graph_to_jsonld_compact`, the CLI's
  `jsonld-compact` and the wasm serializer now produce JSON-LD 1.1 compaction output. The RDF is
  unchanged, but a single top-level node has no `@graph` wrapper, nodes come out in `@id`
  order, and `@id`/`@type` maps and reverse properties follow the spec. (#6674)
- **Breaking (sparq-engine, JSON-LD framing):** `graph_to_jsonld_framed`, `write_jsonld_framed`
  and the wasm framed serializer now use the JSON-LD 1.1 framing algorithm. Frames match over
  the merged graph, so matches no longer carry named-graph wrappers, `@link` behaves as
  `@once`, and a frame that matches nothing returns `{}`. (#6727)
- **Breaking (sparq-core):** the native N-Triples / N-Quads parser follows the W3C grammar.
  IRIs must be absolute, may not contain the excluded characters and allow only `\u` / `\U`
  escapes, so input it used to accept may now fail to parse. (#6723)
- **Breaking (sparq-core):** the native N-Triples / N-Quads parser requires well-formed BCP47
  language tags and an exact `ltr` / `rtl` base direction, matching the serial parser. (#6740)
- **Breaking (sparq-mcp):** `container_list` leaves out members the session cannot read, the
  stdio transport refuses requests larger than `ServerConfig::max_request_bytes`, and a new
  `query_timeout_ms` setting bounds query time. (#6700)
- Workspace, desktop, and public npm package versions move to 0.1.5, with every
  path-dependency requirement and lockfile record following.
- **Breaking (@sparq-org/solid-server):** the optional `fastify` peer dependency is now
  `^5.12.5` (was `^4.28.0 || ^5.0.0`). Fastify 4.x is end of life and has no release that
  fixes GHSA-p68q-wchp-6fh7 (authentication bypass via malformed URLs); fastify releases
  before 5.12.5 carry further request-validation advisories.

- **Breaking (sparq-vc):** `sign`, `sign_graph`, `verify` and `verify_graph` validate
  proof options before any signing or verification work: `verificationMethod` must be
  an absolute IRI, `proofPurpose` a supported VC v2 term or an absolute IRI, and
  `created` an XSD 1.1 `dateTime`. The `authentication`, `capabilityDelegation`,
  `capabilityInvocation` and `keyAgreement` purposes now hash as their VC v2 context
  IRIs (`sec:…Method`), and absolute-IRI purposes hash verbatim. Proofs signed with
  those purposes by earlier releases **no longer verify** and must be re-signed; there
  is no fallback. `assertionMethod` proofs are unaffected.
- `FROM` graph merges standardize source blank nodes apart, including a
  single `FROM`. A blank node in that merged default graph no longer aliases its
  source node accessed with `GRAPH`, even when `FROM NAMED` names the same graph.
  Protocol dataset overrides and LWS union-default expansion use this behavior;
  blank-node labels in results may change.
- `sparq_engine::QueryBudget` adds temporal/numeric capacity controls and
  an optional EBV dialect selector. Exhaustive struct literals must supply the
  new fields or use `..Default::default()`; defaults retain ordinary native
  behavior. Additive detailed evaluation APIs distinguish row/byte capacity,
  numeric/temporal capacity, deadline, cancellation and execution failure.
- Query wrappers retain `VERSION` announcements through algebra rewrites.
  Unknown or incompatible labels reject during query preparation; proof profiles
  explicitly require REC 2013 semantics, and UPDATE remains REC-only.

## [0.1.4] - 2026-09-26

[GPT-5] Recovery candidate after the immutable v0.1.3 release workflow failed before
creating a GitHub Release. Publication is not implied by this entry; verify the
tagged run and every registry and release artifact before using the install paths.

### Fixed

- The release job reads the pinned SLSA generator's actual `provenance-name` output
  for both signed bundles and rejects missing or unexpected names before downloading.
  The v0.1.3 workflow requested an absent output, downloaded unrelated artifacts,
  and failed while generating checksums over their directories.

### Changed

- Workspace, desktop, and public npm package versions move to 0.1.4. Root lockfile
  records follow the workspace with external dependency versions unchanged; the
  standalone bench, fuzz, and desktop lockfiles are regenerated by cargo against their
  current manifests.
- The crates.io publish set is the focused core of 12 crates plus the `sparq-spargebra`
  parser fork (see [docs/release.md](docs/release.md) §4); every other workspace crate is
  `publish = false`. The three empty `sparq-wrapper-*` stub crates are removed.
- The v0.1.3 tag and already-published container artifacts remain intact. This
  candidate rebuilds and attests its own bytes under a new immutable source tag.

## [0.1.3] - 2026-09-21

[GPT-6] New complete-release candidate after the immutable v0.1.2 attempt.
Publication is not implied by this entry; use the [release runbook](docs/release.md)
and require the cadence guard and hosted release verification to pass.

### Fixed

- [GPT-6] Already-expired or cancelled SELECT-JSON requests refuse before scanning,
  emitting output, or waiting for unrelated Rayon pool work.
- GUI installer staging uses Bash 3.2-compatible glob options on macOS. The shared
  staging step is exercised by hermetic fixtures for every desktop platform; empty
  output still fails and the full release alias/provenance contracts remain enforced.
- Desktop package and installer metadata now match the release version, instead of
  retaining the old 0.1.0 value underneath versioned asset filenames.

### Changed

- Workspace packages, Python's derived version, all three public npm packages and
  the desktop application target 0.1.3. Lockfiles preserve external dependencies;
  the private LWS crate and tooling retain their independent versions.
- Release preparation and installation documentation now distinguish the incomplete
  v0.1.2 attempt from this new candidate. The existing v0.1.2 tag, containers and
  provenance remain intact; they are not rebuilt or renamed as v0.1.3 artifacts.
- A maintainer-authorized cadence exception admits only the v0.1.3 recovery with
  pinned predecessor/tag evidence and definitive crates.io absence. All other
  versions retain the 24-hour minimum, and indeterminate reads still refuse.

## [0.1.2] - 2026-09-20

[GPT-6] **Incomplete GitHub Release.** Both container publication lanes completed,
but [run 35540471918](https://github.com/sparq-org/sparq/actions/runs/35540471918)
failed on macOS GUI staging and the alias completeness gate correctly blocked the
GitHub Release. The immutable tag and published containers remain unchanged.
The fix and next complete-release candidate belong to v0.1.3.

[GPT-6] Recovery release prepared from the changes after the immutable `v0.1.1`
source tag. Publication is not implied by this version entry; verify the release
artifacts and registries using [the release runbook](docs/release.md).

### Added

- Experimental, default-off `sparq-core/overlay-deleted-projections` caches
  deletion counts by index permutation. Its retained-memory and cold-read tradeoffs
  are documented in the core README and `bench/overlay-count`; no universal speedup
  is claimed.
- The experimental private `sparq-lws-core` server resolves `acl:agentGroup`
  membership from same-pod group documents. Missing, malformed, off-origin, and
  non-member groups grant nothing; resolution performs no outbound fetch.

### Changed

- Bind joins reuse contiguous groups only after validating sorted input; capped
  seed blocks can reuse bounded right-hand scans when no query budget is armed.
- Core updates preserve the numeric memo when the dictionary is unchanged.
- Workspace packages and all three public npm packages target `0.1.2`, including
  `@sparq-org/eyereasoner-compat` (previously `0.1.0`). The private native LWS crate
  retains its independent version. PyPI `sparq-rdf` derives the workspace version.
- CI now verifies completed nightly work and reporter identity, reserves heavy
  runner capacity, and bounds benchmark dashboard publication. Site and GUI
  dependency patches update Next.js and sharp.

### Fixed

- Nested queries restore the outer budget, cancellation handle, accounting, and
  sticky error on return or unwind; child budgets remain independent.
- Predicate statistics serialize in stable predicate-ID order without changing
  the on-disk format. UPDATE differential tests preserve RDF term identity.
- Scheduler queue-occupancy tests park both workers and release them safely on
  assertion panic, preventing the feature-matrix test hang.
- Release SLSA permissions permit the reusable provenance workflow. The Cargo
  bootstrap graph excludes the path-only introspection test dependency, while
  retaining versioned dev-dependencies in dependency-first publish validation.
- Release and package workflows require matching tag, build commit, and manifest
  versions. Automatic release-plz version calculation is contained until the first
  crates.io bootstrap; the manual version PR's tag handoff remains active.

## [0.1.1] - 2026-08-31

[GPT-6] **Incomplete bootstrap, not a complete release.** The immutable source tag
points to `1a63aa7c638bd80da55f1811d5fb97e8d014f631`. The features below describe
that source snapshot, not successful registry or binary publication. As verified on
2026-09-12, its release workflow failed before starting jobs; no GitHub Release,
crates.io packages, or PyPI `sparq-rdf` distribution were published. npm
`@sparq-org/sparq@0.1.1` and `@sparq-org/solid-server@0.1.1` exist without
`dist.attestations` and cannot be republished to add provenance.

The earlier `v0.1.0` tag was also incomplete. Both tags remain fixed for auditability;
post-tag fixes and complete-release recovery belong to `0.1.2`, not this snapshot.
APIs remain experimental and unstable.

**Crates.io build caveat:** crates.io builds resolve upstream `spargebra` 0.4.6 — the vendored SPARQL-parser conformance fixes (`vendor/spargebra/SPARQ-PATCHES.md`) apply only to git builds until the upstream PRs land.

### Added

- **Storage (`sparq-core`)** — dictionary-encoded triple store with six sorted permutation
  indexes (Hexastore/RDF-3X lineage); a 3-permutation compact-index mode (~half the index
  memory) for memory-constrained targets; parallel index construction.
- **Out-of-core mode** — build the indexes on disk and query them memory-mapped: a 100M-triple
  external build at a few GB peak RAM, then fast open with ~0 committed heap (OS page cache
  only), at query speeds matching the in-memory engine (figures: `research/BENCHMARKS.md`).
- **Parsing** — N-Triples, Turtle, N-Quads, TriG (via `oxttl`); parallel and streaming, with
  transparent `.gz` / `.bz2` / `.zst` decompression. Real-Wikidata ingestion builds all six
  permutations out-of-core on a fanless laptop (rate: `research/wikidata-ingestion-benchmark.md`).
- **SPARQL query (`sparq-engine`)** — SELECT/ASK; Basic Graph Patterns, FILTER, OPTIONAL,
  UNION, MINUS, VALUES, BIND, aggregation + GROUP BY, ORDER BY, DISTINCT, LIMIT/OFFSET;
  sort-merge, hash, and worst-case-optimal joins with greedy cardinality-based planning;
  parallel scan / filter / sort / aggregation / serialization.
- **Inference (`sparq-reason`, opt-in)** — RDFS, OWL-RL, and a Notation3 subset with
  saturate-then-sweep single-pass materialization (a large speedup over a naive fixpoint;
  figures in `research/BENCHMARKS.md` / the crate README) and union-find `owl:sameAs`.
- **CLI (`sparq-cli`)** — load/query/bench subcommands, on-disk index build + `query-mmap`,
  `reason`, a per-subsystem parallel-scaling harness, and hardware-tiered release binaries
  (per-ISA prefetch tuning selected per silicon family).
- **HTTP server (`sparq-server`)** — W3C SPARQL 1.1 Protocol query endpoint (GET/POST) and
  Graph Store HTTP Protocol read side; JSON / XML / CSV / TSV results with content
  negotiation; Docker image (distroless, `ghcr.io/sparq-org/sparq-server`).
- **WebAssembly (`sparq-wasm`, unpublished)** — the core engine compiled for the browser with
  a minimal bundle (no threads, compact index); ships via npm later, not crates.io.

- **Python full-text search (`sparq-py` × `sparq-text`)** — the `TextIndex`
  lifecycle follow-up recorded in `crates/sparq-py/TODO.md` is implemented on the
  wrapper: `Graph.text_search(query, any=False, limit=None)` returns BM25-ranked
  `(Term, score)` hits over the default graph's string literals (AND of tokens by
  default, `any=True` for OR, `*`-suffix prefix tokens), and `Graph.query_text`
  runs SELECTs with the `text:` magic predicates (`text:matches` /
  `text:matchesAny` / `text:score`) through `sparq_text::query_text`. The index is
  built lazily on first use, cached on the `Graph`, and invalidated on every
  mutating swap (`update` / `reason` / `reason_n3_with` — the wrapper's mutation
  paths rebuild the graph, so lazy rebuild, not `apply_delta`, is the policy that
  matches); `build_text_index()` builds eagerly (returns the indexed-literal
  count) and `drop_text_index()` releases the cache. Opt-in stays consistent with
  native (full-text is engaged by depending on `sparq-text`; the new edge is on
  the leaf `sparq-py` crate only, so engine/CLI/wasm stay text-free; `sparq-text`
  itself is unchanged). 12 new pytest cases (38 → 50).
- **Concurrent-serving Wave A2 + A3 (`sparq-serve`)** — the single sequenced
  writer with group-commit over the A1 generation ring (`research/concurrent-serving.md`
  §6.5). One writer thread owns the sole right to publish generations; updates
  arriving within a group-commit window (default 3 ms / 256 updates) apply to one
  working copy and publish as ONE generation (one epoch tick), with per-update
  atomicity — a failed update is reported to its submitter and skipped, never
  poisoning its batch (re-fork-and-replay recovery for partial mutation). Opt-in
  `CommitGranularity::CommuteGroup` splits a window into maximal runs of
  provably-commuting pure-data updates via conservative graph-level footprint
  analysis (`Footprint::of_sparql`; same-named-graph opposite-polarity or
  unanalyzable = barrier) and publishes one conflict-free generation per run —
  committed result is byte-identical to serial (FIFO application either way).
  `GraphApplier` is the production `ApplyUpdates` (structural `Graph::fork` +
  delta-overlay apply + threshold compaction). **A3**: `Writer::submit`'s returned
  generation number doubles as a single-node read-your-writes `shard_seq` token;
  `GenerationRing::read_your_writes(token)` pins the current generation iff it
  already reflects that commit (`number >= token`), `None` otherwise (a lagging
  replica must wait/redirect). Readers never block — `current()` stays lock-free.
  Tested: group-commit + FIFO + failed-update isolation, commutativity-batch
  differential correctness (CommuteGroup == Window == serial, + 200-op fuzz),
  50k-triple bulk-update atomicity (a concurrent sampler sees 0-or-all, never a
  partial prefix; never stalls), retention/pinning under sustained writes, and
  read-your-writes token semantics. Benched in `bench/serve/writer_spike`
  (group-commit ~7.8× writer throughput at batch 16 vs 1; readers within ~1.3× of
  idle p99 under load; the too-large-batch regression recorded honestly).
  `sparq-serve` stays out of the `sparq-wasm` dependency graph (server-side only).
- **Concurrent-serving Wave B — read-side scheduler (`sparq-serve`)** — the
  cost-aware query `Scheduler` (`research/concurrent-serving.md` §6.2; goal #4
  requirements 3 + 4: cheap-query prioritisation + no head-of-line blocking). A
  bounded thread pool with two lanes split by a crude cost estimate (Wierman &
  Nuyens: SRPT tolerates crude size errors): **cheap** jobs are prioritised
  (FIFO) and always keep a reserved worker, while **heavy** jobs run under a
  bounded concurrency cap (default `cores/2`, clamped `<= workers-1` so a cheap
  worker is always free). The reserved-cheap-capacity invariant is the structural
  no-head-of-line guarantee — an expensive-query flood can never block a cheap
  query, because at least one worker is always available the instant a cheap job
  arrives. Within the heavy lane, ordering is Umbra-style SRPT-approx + unbounded
  aging (litreview-C: `p0 = 10^4` start, cost lowers priority, wait raises it), so
  the most-postponed heavy job eventually becomes the max-priority pick —
  starvation-free. Readers still pin a generation on entry *inside* the submitted
  closure (A1 snapshot consistency), so scheduling reorders *when/where* work runs
  but never *what* it computes: results are byte-identical to unscheduled
  execution. Library-first: sync, runtime-agnostic (`std::thread` + condvars, no
  HTTP/async types); the estimator and the work are both injected, so the
  scheduler never reaches into the engine itself. Tested (empirical-honesty
  suite): differential result-equality vs direct execution (closure + real-engine
  COUNT queries over the ring), bounded heavy concurrency, head-of-line cheap-p99
  containment under a concurrent expensive query (open-loop,
  coordinated-omission-safe arrival), starvation-freedom under a sustained cheap
  flood, SRPT-orders-cheaper-first + aging-overtakes-a-cheaper-fresh-heavy
  (deterministic), no-regression on an all-cheap workload vs a plain pool,
  panic-isolation, and shutdown shed-don't-hang. Honest scope: true preemption is
  out (the engine has no re-entrant suspend/resume); the cap + reserved capacity
  deliver the no-HoL property without it. Stays out of `sparq-wasm`'s graph.
- **Python bindings parity wave (`sparq-py`)** — the Python `Graph` catches up with
  the engine/reasoner surface: native `ask()` (engine early-exit instead of the
  SELECT-count rewrite; SELECT still accepted), new `construct()` / `describe()`
  (lists of `(s, p, o)` `Term` tuples; DESCRIBE is concise-bounded-description),
  named-graph parity (the stale "named graphs unsupported by update / dropped by
  reason" caveats removed: updates ride engine update v2, and `reason()` /
  `reason_n3_with()` now carry `Graph.named` across the rebuild), new
  `inconsistencies()` (OWL 2 RL clash report from `sparq_reason::inconsistencies`),
  and new `reason_n3_with(rules)` (caller-supplied N3 rules over an already-loaded
  graph, composed exactly like `MaterializedN3Graph`'s fallback: default graph
  rendered as N-Triples under the rules document, run through `reason_n3`; on error
  the graph is unchanged). 18 new pytest cases (20 → 38); README/TODO refreshed.
  Verified non-gap recorded honestly: full-text `text:` predicates do NOT work
  through plain `Graph.query` — `sparq-text` is a deliberately standalone opt-in
  crate, so exposing it needs a `TextIndex` lifecycle on the wrapper (left as a
  documented follow-up in `crates/sparq-py/TODO.md`, not wired in quietly).
- **JS/wasm parity wave (`@sparq-org/sparq` + `sparq-wasm` + `sparq-core`)** — six recorded
  parity gaps closed; wasm bundle 1,593,075 → 1,643,103 B (+50,028 B, +3.14%, all from
  three new wasm capabilities, measured per feature; JS-only features byte-identical):
  - **ASK from JS** (0 B): `queryBoolean()` now uses the engine's native ASK (boolean
    JSON form, first-solution early exit) instead of the SELECT-rewrite + count path;
    `queryJson()` serves ASK too. Plus a test pinning full-text-style matching via plain
    SPARQL string functions (and that `REGEX` stays compiled out of wasm by design —
    the engine's non-default `regex` cargo feature).
  - **Named graphs** (+16,906 B): wasm `Store.loadDataset` (N-Quads/TriG named graphs
    preserved); JS `options.dataset` on `fromString`/`fromQuads`; `GRAPH`/`FROM`/
    `FROM NAMED` queries and full-dataset SPARQL Update (already compiled in) now
    reachable; `match()`/`countQuads()` graph-aware (wildcard spans named graphs via a
    `UNION { GRAPH ?g … }`).
  - **Streaming results** (+4,575 B): wasm `Store.queryChunks` cursor over the engine's
    chunked SPARQL-JSON serialiser (~64 KiB, row-boundary splits, concatenation
    byte-identical to `query()`); JS `queryBindingsStream()` generator (`for…of` and
    `for await…of`) backed by a chunk-boundary-agnostic incremental row parser
    (`SparqlJsonRowsParser`), `queryJsonChunks()` for raw forwarding.
  - **Incremental updates from JS** (+28,547 B: `applyDelta` +14,895, `updateInPlace`
    +13,652): new core API `Graph::apply_delta_nquads` (N-Quads batch, deletes-first,
    routed per graph, named graphs auto-created, bnodes by label); wasm
    `Store.applyDelta` + `Store.updateInPlace` (engine `update_in_place`); JS
    `applyDelta`/`addQuads`/`removeQuads`, and `update()` now applies in place —
    O(batch), no index rebuild.
  - **Compressed ingest in the browser** (0 B, JS-only): `fromCompressed()` /
    `decompress()` — zstd via pure-JS `fzstd` (dynamically imported; decodes the
    multi-frame streams `CompressedSink` emits — re-verified), gzip via `node:zlib`
    (multi-member) / `DecompressionStream` (single-member, browser caveat documented
    per the D4 §3 matrix). `fzstd` is the package's single runtime dependency now.
  - **Dictionary-fetch protocol client** (0 B, JS-only): `SparqDictionaryClient`
    implements D4 §4 — `Sparq-Dictionary`/`Sparq-Dictionary-Current` negotiation,
    content-addressed (truncated-id, prefix-verified) dictionary caching with
    background warm-up from `GET /dictionary/{dict-id}`, pluggable dict-capable
    decoder hook (fzstd has no dictionary API), RFC 8878 frame-header dictID parsing.
  - JS test suite 16 → 42 (`node --test`), +1 native `sparq-core` unit test.

- **Opt-in inference proof trees (`sparq-reason`, non-default `explain` feature)** —
  `why(triple)` on `MaterializedGraph` / `MaterializedOwlGraph` / `MaterializedN3Graph`
  returns a `ProofTree` for any closure triple: which rule fired (W3C spec rule ids —
  `rdfs2..11`, `cax-sco`, `prp-trp`, `scm-*` — or documented engine rules `inv-dom`/
  `inv-rng`/`sym-dif`, `n3-rule-<i>`) from which premises, recursively to asserted facts.
  The shape is flat and ZK-witness-friendly (premises-before-conclusion node list, root
  last, shared sub-proofs deduplicated, deterministic, terms as self-contained strings —
  a planned ZKP module can consume it as a derivation witness); JSON + indented-text
  renderings; depth/size caps (`ExplainOpts`). Proofs are *a witness, not all witnesses*,
  and always reflect the CURRENT base: after a delta, retracted supports are never served
  (an alternative support is found, or the triple no longer explains).
  Design: derivations are *reconstructed* from the counting engines' closed TBox +
  deterministic emission functions (plus raw TBox edge maps and base reverse indexes kept
  under the feature) rather than stored per derivation. OWL fallback mode (recursive/
  equality features) is documented as unexplained; N3 `why()` re-runs the batch engine
  with proof recording per call. Id-level batch N3 proofs bridge via
  `explain::n3_proof_tree`. The feature is cfg'd out entirely by default: zero hot-path
  cost, zero wasm impact (sparq-reason is not in the wasm graph; bundle byte-identical).
  When ON: batch `materialize_*` is unchanged (untouched code); incremental handles pay a
  bounded constant for the base reverse indexes on initial build, and small deltas stay
  ms-scale and far faster than re-materialization (measured cost in the crate README); N3
  unchanged (no extra maintenance state). Design lever if that ever matters: build the
  indexes lazily on first `why()`.

- **Opt-in full-text search over literals (`sparq-text`, new crate)** — a small,
  owned BM25 inverted index over a graph's string literals (UAX #29 tokenizer +
  Unicode lowercasing via `unicode-segmentation`; deliberately no tantivy) where
  the dictionary term id IS the document id, so hits join back to triples through
  the ordinary permutation indexes. `TextIndex::build` scans the dict once
  (rayon-sharded under `parallel`); `apply_delta` mirrors `Graph::apply_delta`
  batches incrementally (pinned equal to a rebuild by a differential test).
  Query surface: AND (`search`), OR (`search_any`), `*`-suffix prefix tokens
  (autocomplete), BM25 scores — and, behind the default-on `engine` feature, the
  `text:` magic predicates (`http://sparq.dev/text#`): `?lit text:matches "q"` /
  `text:matchesAny` / `text:score ?s`, rewritten at the spargebra-algebra level
  into inline `VALUES` over the hits and executed through the engine's existing
  `PreparedQuery: From<spargebra::Query>` seam — zero engine changes, zero wasm
  impact (bundle byte-checked; the crate sits outside every default dependency
  graph, like `sparq-geo`/`sparq-vectors`). Build / index-size / query-latency figures
  on 1M synthetic literals: `crates/sparq-text/README.md` (run its `bench_text` example).
- **Opt-in time-travel queries (`sparq-serve`, `sparq-server`)** — a retained
  generation IS a queryable snapshot. Library: `RingConfig::time_travel`
  (`TimeTravelConfig { max_generations, max_age }`, default `None`) extends ring
  retention beyond the concurrency bound K (K stays a floor — time travel extends,
  never shrinks); `GenerationRing::at(n)` pins a retained generation,
  `Generation::published_at` (stamped by the injectable `RingConfig::clock`) +
  `GenerationRing::as_of(t)` resolve "as of T" to a generation, honest `None` once
  history ages out. Server (non-default `time-travel` cargo feature):
  `?generation=N` on `/sparql` (URL or url-encoded body) serves the store as of
  that generation; every `/sparql` response carries a `Sparq-Generation` header
  (update 204s carry the generation containing the update — the read-your-writes
  token, the horizontal-scaling ADR's shard_seq concept); aged-out → `410 Gone`,
  never-published/unparsable/pinned-update → `400`; retention via
  `--time-travel-generations` [16] / `--time-travel-max-age` [off]. Feature off:
  the parameter handling is compiled out (tests cover both compilations). Memory
  cost recorded honestly: each retained generation is a FULL `Graph` until the
  structural-fork follow-up lands (delta-chain/OSTRICH-style retention is the
  named follow-up; the API needs no change for it). Wasm bundle unchanged.
- **GeoSPARQL function completion (`sparq-geo`)** — the `geof:` registry grows from
  12 to 35 functions: `geof:getSRID` (`xsd:anyURI`), the generic DE-9IM
  `geof:relate`, the full Egenhofer (`geof:eh*`, 8) and RCC8 (`geof:rcc8*`, 8)
  relation families (GeoSPARQL 1.0 Req 25/26 matrix patterns), the set operations
  `geof:intersection`/`union`/`difference`/`symDifference` (polygonal operands via
  geo's `BooleanOps`; other operand types are a clear per-row expression error),
  and `geof:buffer` (unlocked by the geo 0.30→0.33 bump; metric radii buffer in a
  local equirectangular metre frame). All exercised through real SPARQL.
- **GeoIndex: antimeridian, named graphs, incremental updates (`sparq-geo`)** —
  ball queries crossing ±180° split into two longitude windows (merged, deduped;
  brute-force-verified near the seam); `GeoIndex::build` scans every named graph
  and entries record their origin graph + `geo:asWKT` node; new
  `GeoIndex::apply_delta` mirrors a `Graph::apply_delta` batch into the R-tree
  incrementally (rstar insert/remove, O(batch·log n) — no rebuild), including
  `geo:hasGeometry` ownership re-keying.
- **CRS reprojection (`sparq-geo`, opt-in `reproject` feature)** — pure-Rust
  proj4 (`proj4rs`; no C dependency) behind a curated EPSG table (27700 — verified
  against the Ordnance Survey worked example to ~1e-6°, 3857, 2154, 25832/25833,
  UTM 326xx/327xx): `reproject::to_crs84` brings projected literals into the
  geographic machinery (metric distance, GeoIndex).
- **HDT quality-of-life (`sparq-hdt`)** — GZipped containers (`.hdt.gz`) are
  detected by magic bytes (not file names) and decompressed on the fly in every
  entry point; new `header()`/`header_reader()` expose the HDT header (dataset
  metadata triples — VoID statistics, provenance) as a queryable sparq `Graph`.
- **CLI HDT ingestion (`sparq-cli`, opt-in `hdt` feature)** — `--features hdt`
  wires sparq-hdt into the loader dispatch: format argument `hdt` or a
  `.hdt`/`.hdt.gz` extension loads through `sparq_hdt::load`; without the feature
  the CLI exits with a rebuild hint. Off by default (MSRV 1.87 vs the CLI's 1.85
  floor + decode-stack weight; rationale in `crates/sparq-cli/Cargo.toml`).
- **Recorded engine/core API seams (audit of the opt-in crates' TODOs)** — the
  cross-crate seams the "modify no existing crate" waves recorded, now implemented:
  - `sparq-core`: `Graph::load_str_with_base` / `parse_to_triples_with_base`
    (base-IRI resolution for Turtle/TriG — removes sparq-shacl's oxttl+`from_parts`
    workaround); `Dict::iter()` (the official dense-1-based-id term iterator
    sparq-introspect leaned on by hand); `Graph::iter_ids` / `iter_ids_sorted(col)`
    (borrowing canonical-id triple iterator, overlay-merged, zero alloc per row —
    sparq-shacl's term scans, sparq-sim's distinct subject/object enumeration);
    regression test pinning `Graph::save` on a SPARSE numeric cache (the panic
    sparq-py recorded was already fixed by `dense_numerics`; now pinned).
  - `sparq-engine`: **`PreparedQuery`** — parse-once / execute-many entry points
    (`query`/`ask`/`count`/`query_json`/`construct`/`describe`
    `*_prepared(_with_budget)` twins; the string APIs are now thin wrappers), the
    seam sparq-rsp recorded for per-window evaluation without re-parsing; and
    `#[derive(Debug)]` on `QueryResult` (workspace TODO, recorded by sparq-nlq).
  - `sparq-engine` **`cs-planner` feature (opt-in, non-default)** + `sparq-introspect`
    `characteristic_set_ids()`: characteristic-set star-join cardinality estimation
    (Neumann & Moerkotte) injected via `cs::CsTable` / `with_cs_table`; the greedy
    planner scores star candidates by the conditional CS expansion and uses
    `Σ_{C⊇Q} count(C)` as the subject-variable ndv (join order only — results
    identical; zero code in the wasm/default builds). Gate (q-error through the real
    planner recurrence vs true cardinalities, synthetic correlated stars): PredStat
    median 1.83 / gmean 7.02 / max 1283 vs **CS 1.00 / 1.00 / 1.00**.

- **Incremental inference maintenance (`sparq-reason`)** — counting-based incremental
  closure maintenance under base inserts AND deletes (deletion costs the same as
  insertion: exact derivation counts, no overdelete/rederive pass), extending T18's
  RDFS `MaterializedGraph` to two new opt-in, zero-wasm-impact siblings:
  - `MaterializedOwlGraph` (OWL 2 RL): the monotone assertional rules (prp-spo1,
    cax-sco, prp-dom/rng, prp-inv1/2, prp-symp, prp-eqp1/2, cax-eqc1/2) maintained via
    a property-orientation closure mirroring the batch `PropExpand`; `prp-trp` via a
    dedicated exact transitive layer (per-property effective-edge multisets, closure
    recomputed on touch — cost proportional to the transitive subgraph — and diffed
    into the counts). TBox mutations / scm-* and the recursive/equality features
    (sameAs, Functional/InverseFunctional, chains, restrictions, cardinality, hasKey,
    oneOf, intersection/union) take documented re-materialization fallbacks
    (`OwlMode` telemetry + `full_rebuilds()`).
  - `MaterializedN3Graph` (Notation3): a counting fast path for rule sets that
    ANALYSIS proves monotone with input-stratified negation — ground IRI predicates,
    a verified-parity builtin whitelist (log:uri, log:equalTo/notEqualTo,
    string:concatenation/scrape/encodeForUri), `?UNSCOPED log:notIncludes` only over
    predicates no rule derives (guard deltas rebuild), recursion via recursive-SCC
    layers; everything else falls back to the batch engine per mutation. Fallback
    cases include: out-of-whitelist data reached at evaluation (sticky); base
    `log:implies`-family triples (rules-as-data); and rules whose premise has NO plain
    join atom (an empty `{}` premise or a builtin/guard-only premise), since counting
    seeds emissions only from plain premise atoms and would otherwise drop their
    conclusions ([OPUS-4.8] reviews 1868/1884). The sparq-solid WAC rules and ACP
    strata a/b qualify; acp-c does not (variable conclusion predicate) — asserted by test.
  Differential property tests hold every counting-*maintained* profile equal to its
  from-scratch batch closure after every randomized edit batch (RDFS/OWL/N3);
  rule shapes outside the maintained profile (above) take the always-correct batch
  fallback rather than being maintained. Benchmarks
  (`bench/inference/incremental-bench.md`, olympics scale + a 1k-doc WAC pod):
  1-triple deltas maintain in microseconds vs ~1s re-materialization (~10⁵–10⁶x);
  10k-triple deltas 16–500x; live WAC ACL edits 11–162 ms vs the 0.84 s engine re-run,
  with the incremental initial build itself beating the batch engine (anchored
  premise evaluation). Conformance and the batch paths are untouched.

- **Severity-aware conformance + conformance memo (`sparq-shacl`)** —
  `ValidationReport::conforms_violations_only()` (CI-style gating: warnings and
  infos report but don't break conformance) and `results_with_severity(iri)`;
  the spec's `sh:conforms` is untouched and the API stays infallible. Inside
  the validator, `conforms()` now memoises `(focus, shape) → bool` with a
  cycle-soundness rule that leaves the recursion guard unchanged (a result is
  cached only when no guard re-entry escaped below its frame). W3C core suite
  unchanged at 98/98; pinned by new cyclic-shape and severity tests.

- **Neighbor-sparse fallback for `most_similar` (`sparq-sim`)** — entities whose
  signature elements all point at degree-1 neighbors (every event names exactly
  one sport) generated NO candidates in v1. `SimConfig::profile_fallback`
  (default on) fills starved slots with predicate-profile (role) matches, ranked
  below every exactly-scored result; predicate blocks are scanned
  most-selective-first with a `max(4k, 64)` stop. Re-measured on the real
  olympics eval: Sport precision@10 0.000 (0/0) → 1.000 (400/400), City 0.500
  (2/4) → 0.995 (398/400), full 2400/2400 candidate coverage, ~0.3 ms mean
  latency cost. The per-element IDF hub budget was investigated and is
  measured-and-rejected (the eval is saturated).

- **Streaming store builds, weighted RRF, bytes-backed open (`sparq-vectors`)**
  — `StreamingWriter` removes the build-phase RAM ceiling (vectors append
  straight to the `.spqv` data section, the id→slot index spills to a sidecar;
  output byte-identical to the in-RAM builder — no format change; duplicates
  reported at finalize), `fuse_rrf_weighted` (Elasticsearch-style per-list
  weights, unit weights ≡ `fuse_rrf`), and `VectorStore::open_from_bytes`
  (filesystem-less environments; validation identical to `open`, all read paths
  shared). No wasm feature wired (deliberate — the crate stays out of the wasm
  graph); DiskANN-style persistent ANN, quantization and big-endian remain
  deferred with rationale in `sparq-vectors/TODO.md`.

- **Overlay window evaluation, t0, CONSTRUCT/ASK (`sparq-rsp`)** — closed
  windows are no longer rebuilt from scratch: the new `EvalMode` selects the
  R2R materialisation strategy — `PersistentDict` (one dictionary per
  continuous query, terms interned once at push time, per-window graphs from
  already-interned ids; the new default — wins every benchmark scenario,
  1.2–5.3× over the v1 rebuild, biggest on sliding windows), `Delta` (one
  live graph + set-semantic `apply_delta` per slide with churn-driven
  compaction; measured slower everywhere, kept opt-in), and `Rebuild` (the v1
  baseline, still right for unbounded-vocabulary streams). Plus: RSP-QL
  window origin `t0` (`WindowSpec::with_t0`), and continuous CONSTRUCT
  (`ContinuousConstruct` — stream-to-stream transformation with exact
  set-diff ISTREAM/DSTREAM) and ASK (`ContinuousAsk`) query forms. All three
  modes are observationally identical (pinned by tests); README throughput
  table re-measured per mode. Zero wasm-bundle impact (opt-in crate).

### Changed

- [OPUS-5] **N3 incremental maintenance: the base↔layer ownership transfer no longer
  re-materializes (`sparq-reason`, `sq-6tykl.6`)** — a fact that is both asserted and
  derivable by a recursive-SCC layer is charged to the base while asserted, so mutating its
  base copy hands ownership between the base and the layer without changing the closure. The
  sign-homogeneous delta round could not express that hand-off and recovered with a full
  (non-sticky) re-materialization. The affected layer's own local fixpoint — recomputed in
  that round — now decides the hand-off directly: the layer step runs before the counted-rule
  step, a fact the layer re-derives is put straight back and dropped from the round's delta,
  and no derivation count is disturbed. The hand-off is settled whenever the affected layer is
  recomputed, which makes it lazy in the assert direction: retracting the base copy enters the
  round and normalizes ownership immediately, whereas asserting a fact the closure already
  holds contributes nothing to the round, so the layer keeps its copy alongside the new base
  one until a later delta recomputes that layer. The interim double ownership is
  observationally inert — while the base copy exists the layer's entry can never suppress a
  seed, and nothing consults it without the base guard. `full_rebuilds()` no longer
  increments for these deltas. Behaviour-neutral for the closure (the from-scratch
  differential oracle in `tests/incremental_n3_prop.rs` is unchanged and green); the TBox and
  guard-predicate full-rebuild fallbacks are untouched and remain documented.

- [SONNET-4.6] N3 rule-existential blank labels now use a source-fresh numbered
  namespace, so closure output may mint labels such as `_:__sk0_1_e` instead of
  the previous `_:__sk1_e` shape.

- [GPT-5.6] **Breaking (`@sparq-org/sparq`)** — `SparqStore.queryBindings(sparql, context?)` now
  returns `Promise<ResultStream<Bindings>>` instead of `Bindings[]`; await it and consume
  `data` / `end` / `error` events (or use `query()` for synchronous materialisation). Unsupported
  `context.sources` overrides, including an empty array, now reject instead of being ignored.

- **Pipelined streaming N-Triples ingest (`sparq-core`)** — `Graph::load_reader_parallel`
  now fills its 32 MiB block from repeated short `read()`s and overlaps decompression
  with parallel parse (producer thread → bounded channel → rayon parse → dict merge),
  matching the `build_external_ntriples_parallel` pipeline. Streaming ingest of a gzip/zstd
  N-Triples file is now far faster than the pre-fix per-`read()` flush and matches or beats
  decompress-then-parse (figures: `research/custom-parsers-baseline.md`).
  `load_reader_parallel` requires `R: Read + Send`.

- **Inference fixpoint linearization + delta-driven evaluation (`sparq-reason`,
  fixpoint-opt thread)** — the chain-transitivity derivation storms are gone, and
  closure-only callers stop paying for proof bookkeeping:
  * OWL-RL `prp-trp` is evaluated as the LINEAR rule `R(x,y), GEN(y,z) ⊢ R(x,z)` over
    generator edges (edges not derived by prp-trp itself; `TC(GEN) = TC(R)`), with the
    per-round schema rebuild and `prp-fp`/`prp-ifp` moved into the delta sweep —
    owl-bench `owl-transitive` (2k-edge chain, ~2M-pair closure): **54 s → 0.25 s**.
  * The N3 forward chainer detects transitivity-shaped rules
    (`{?x P ?y. ?y P ?z} => {?x P ?z}`) and runs the same linearization, bypassing the
    generic binding machinery — anc500 (500-link `:ancestor` chain): **52 s → 0.18 s**
    engine-internal (closure byte-identical, 125,250).
  * `run_closure` takes a `StepMode`: premise materialization and proof-step interning
    are skipped when the caller discards them (`reason_n3`, the CLI closure path;
    conformance keeps conclusions only) — grid30 closure 1.94 s → 1.02 s (−47%),
    dt100k −18% (engine-internal min-of-5, identical closures).
  Inference conformance unchanged (1637 pass / 0 fail / 17 documented divergences);
  wasm artifact size unchanged; refreshed EYE head-to-head in
  `bench/inference/eye-comparison.md`.

- **`sparq-geo` depends on geo 0.33** (was 0.30; clean API compatibility): brings
  the `Buffer` trait that makes `geof:buffer` implementable.
- **`GeoIndex::entries()` returns an iterator** (was a slice): entry slots are
  tombstoned/reused by incremental deletes.

- **OWL 2 RL rule completion (`sparq-reason`)** — the remaining OWL 2 RL/RDF rules
  (Profiles §4.3): `cls-oo` (oneOf member typing), `cls-maxqc1/2`
  (qualified-cardinality-0 clashes in `inconsistencies()`), the schema-level
  restriction-subsumption family `scm-hv`/`scm-svf1`/`scm-svf2`/`scm-avf1`/`scm-avf2`
  (indexed/guarded joins grouped by onProperty resp. filler — never the naive
  quadratic restriction×restriction pass), and the premise-free
  `cls-thing`/`cls-nothing1` axioms (occurrence-guarded). The rule-coverage table in
  `research/inference-completeness-audit.md` is now fully implemented or explicitly
  by-design (prp-ap, eq-ref, dt-*, reflexive scm-* — each argued). Conformance scores
  and closure throughput unchanged; zero wasm-bundle impact.

- **EXPLAIN / EXPLAIN ANALYZE (`sparq-engine`, `sparq-server`, T22)** — query-plan
  introspection: engine `explain()` renders the algebra tree plus a planning-only dry
  run of the BGP planner (greedy GOO join order with cardinality estimates, per-step
  join strategy — merge/hash/bind/worst-case-optimal — and pushed-down filters) by
  replaying the executor's own planning helpers; `explain_analyze()` (SELECT/ASK)
  executes under a thread-local per-operator trace reporting output rows + wall time
  per operator (zero-cost when off: one flag read per operator entry). Served on
  `/sparql` via `?explain=` / `explain=analyze` or `Accept: text/x-sparq-explain`.
- **Prometheus metrics (`sparq-server`, T22)** — hand-rolled text exposition at
  `GET /metrics` (no new dependencies): request counts by endpoint/status, a `/sparql`
  latency histogram, active-subscription and graph-triple gauges, an update counter.
- **SPARQL CONSTRUCT / DESCRIBE (`sparq-engine`, `sparq-server`)** — RDF-graph query
  results: spec-conformant CONSTRUCT template instantiation (unbound/illegal slots
  skipped, fresh blank nodes per solution, set-deduplicated) and DESCRIBE as concise
  bounded description; engine API `construct` / `describe` / `construct_ntriples`
  (+ `_with_budget` variants); served over HTTP with `application/n-triples` /
  `text/turtle` content negotiation under the existing query budget. W3C construct
  evaluation suites pass 9/9 run (1 skip: `FROM`).
- **Streamed SELECT results (`sparq-server`)** — JSON SELECT bodies are streamed from
  an ordered chunk sequence (engine `query_json_chunks_with_budget`; concatenation
  byte-identical, same `Content-Length`) instead of one giant string: peak server RSS
  for a 1M-row SELECT (202 MB body) drops from ~750 MB to ~405–570 MB.

### Fixed

- **Persisted per-predicate stats never loaded on `open` (`sparq-core`)** —
  `load_pred_stats` read the predicate id as 8 bytes while `save_pred_stats` writes a
  4-byte `Id`, mis-framing every record: the load always failed and `open()` silently
  fell back to recomputing the stats, which paged the ENTIRE POS+PSO permutations into
  RAM (the dominant out-of-core open cost the persisted file exists to avoid). Measured
  on the 10M-triple synthetic dir: open 0.52 s → 0.019 s, RSS after open 236 MB →
  2.3 MB (research/memory-tiering.md). Regression-tested
  (`pred_stats_load_is_some_and_exact`).
- **N3 incremental maintenance: base↔layer ownership transfer (`sparq-reason`)** —
  deleting a fact that is both asserted and derivable by a recursive-SCC layer made the
  fact re-enter the layer's derived set during a delete round (it stops seeding the local
  fixpoint), tripping the sign-homogeneity `debug_assert` (panic in debug builds, a
  needless *sticky* engine fallback in release). Now recovered with a non-sticky full
  re-materialization; the graph stays on the counting fast path. Found by the `explain`
  retraction tests; regression-tested against the from-scratch oracle.

- **Domain/range typing for TBox-orphan properties on the monotone OWL route
  (`sparq-reason`)** — on the batch single-pass path (`rdfs_closure` with the
  property-orientation closure active, i.e. any `owl:inverseOf`/`owl:SymmetricProperty`
  axiom present), a property with an `rdfs:domain`/`rdfs:range` declaration but no
  subPropertyOf/inverseOf/symmetric/equivalentProperty edge emitted NO rdfs2/rdfs3
  typing for its assertions (absent from the orientation map, the emission
  short-circuited) — diverging from the full fixpoint path. Such properties now fall
  through to the plain-RDFS emission (their expansion is the trivial identity).
  Regression-tested; conformance unchanged (1637/0/17 — no suite test hits the
  combination, which is how it survived).

### Performance

Measured against native QLever on the same machine, compute-only, cold, min-of-N. The
methodology, the full per-query tables, and the speedup ranges are single-sourced in
`bench/qlever-baselines.md` (run `bench/qlever-olympics/` / `bench/qlever-synthetic/` to
reproduce):

- Synthetic 10M and 100M join/OPTIONAL queries: faster than QLever, in-memory and
  out-of-core (mmap) alike; the advantage holds from 10M to 100M.
- Real skewed data (DBpedia/Wallscope olympics, 1.78M triples): faster on **every** query.
- Honest caveats: QLever's on-disk index is *compressed* (smaller disk footprint at
  billion-scale); billion-scale real data (Wikidata/WDBench) is untested on this hardware;
  single-pattern/FILTER comparisons short-circuit via index range-counting and are excluded
  from the claims above.

[Unreleased]: https://github.com/sparq-org/sparq/compare/v0.1.6...HEAD
[0.1.6]: https://github.com/sparq-org/sparq/compare/v0.1.5...v0.1.6
[0.1.5]: https://github.com/sparq-org/sparq/compare/v0.1.4...v0.1.5
[0.1.4]: https://github.com/sparq-org/sparq/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/sparq-org/sparq/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/sparq-org/sparq/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/sparq-org/sparq/tree/v0.1.1
