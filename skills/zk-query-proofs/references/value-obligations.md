# Value FILTER obligations

With `dual-leaf`, `value_obligation::{ValueObligation, derive, discharge,
DischargeMap, ObligationError}` provides the value-lane obligation model, not yet
accepting. The value-lane gate stays CLOSED. Neither accepting verifier calls
`discharge`; structural success does not verify a proof.

`derive(&manifest, &authenticated_methods)` returns one obligation per typed
FILTER conjunct, matching source scan, active row and variable slot. It reuses
the verifier's constant-membership and authenticated-method resolution. An
explicit `pattern_scans` declaration cannot narrow coverage. The methods must
come from issuer authentication; a missing map entry is refused, and an entirely
Unbound scan resolves as string-canonical with no value lane.

`discharge(&manifest, &authenticated_methods, &obligations)` checks the complete
query-derived set, exact edge identity, per-scan method, member, typed pin and
`operand_enc` equality with the source row/slot. Each obligation requires exactly
one proof; every value proof must cover an obligation. `BindingEdge` now includes
`filter: Option<usize>`: use `Some(query_conjunct_index)` for value diagnostics,
`None` for existing edges. Absent indices deserialize as `None` and serialize
without the field; canonical ordering adds this field after `to_proof`.
The predicate rejects missing conjunct indices rather than guessing from bounds.
Both accepting verifiers refuse every edge carrying `Some(_)` at their shared
stage-2 prefilter with `CheckError::EdgeFilterIndexNotAccepted { edge }`, including
out-of-range indices on ordinary FILTER members. Existing `None` edges retain
their structural checks. Diagnostic `discharge` still reads the index; the later
accepting PR replaces this refusal. The value-lane gate stays CLOSED.

Only `!=` and `!(=)` normalize to `Ne`. General NOT is `OpNotModelled`, never an
`expected=false` obligation. False verdicts fail the pin. Integer values are
non-negative, sign-split temporal lanes do not admit signed integers, negative
zero decimals are refused, decimal scales rescale exactly, doubles canonicalize
NaN/zero and use IEEE comparisons, and boolean operands use the dedicated bit
relation. Private operand-domain checks remain circuit constraints.

V1 integer, boolean, double, decimal (all admitted scales), date and dateTime
value lanes are refused at stage 0 with `CheckError::LegacyV1ValueMember`.
Only the matching dispatch error maps to that variant; other legacy-check
errors map to `CheckError::LaneMemberMismatch { proof, reason }`.
`dispatch::resolve_circuit(DualLeafV1, value_member)` returns
`DispatchError::LegacyV1ValueMember`, including for already-issued use.
`admit_legacy_dual_leaf_v1` preserves V1 string-lane verification only. Pairing
and rendering helpers retain the V1 inventory for legacy fixtures. Other value
proofs remain `ValueLaneSubProofNotAccepted`; extended verification also refuses
both V2 scan/join members by its named encoding-policy error.
