# Value FILTER obligations

With `dual-leaf`, the flat verifier opens the dual-leaf V2 value-FILTER lane,
research-grade, not externally audited. `verify_manifest` accepts a value member
only when it discharges a query-derived obligation and its challenge-bound bb
proof verifies. `prefilter_manifest_structure` establishes the structural checks
only; it does not verify proofs.

An obligation is one typed FILTER conjunct, answering source scan, active row
and variable slot. Every slot across every constant-compatible answering pattern
is covered; duplicate occurrences of the same slot are deduplicated.
`pattern_scans` cannot narrow coverage. A FILTER must still have an answering
scan (`any_scan_answered`); an empty scan has no active-row obligations.

`bind_query_correctness` parses `fragment_typed_filters` once. The shared
`verify::query_filter_u64` reduction supplies exactly the canonical non-negative
`xsd:integer` view used by `fragment_filters`. A conjunct without that view can
only use the value lane. Each obligation requires either a true `filter_int`
edge with matching comparison and integer bound, or exactly one value edge.
A string edge plus a value edge on one obligation is refused, as are duplicate
value edges. Existing string-only coverage retains its previous behavior.

`BindingEdge::filter` is `Some(query_conjunct_index)` for V2 value edges and
`None` for string-lane edges. Absent indices deserialize as `None` and serialize
without the field; canonical ordering includes this field after `to_proof`.
Stage 2 refuses an indexed edge into an ordinary proof with
`EdgeFilterIndexOnNonValueProof`, and an unindexed value edge with
`ValueEdgeWithoutConjunct`. `EdgeFilterIndexOutOfRange` refuses nonexistent
conjuncts. Both lanes require operand equality with the named scan cell.

The shared per-edge predicate in `value_obligation` checks the source scan's own
authenticated method is `DualLeafV2`, the `expected_value_lane` member matches,
`check_value_lane_pin` passes and the operand equals the exact active row/slot.
The pin derives the lane, comparison and bound from the typed literal and requires
`expected=true`. Every value edge must discharge a real obligation, and every
value member must have an edge. Invalid edges are refused even when all real
obligations already have coverage. `CheckError::ValueObligation` carries named
refusals including `DischargedTwice`, `OrphanValueProof` and `EdgeMismatch`.

`value_obligation::derive(&manifest, &authenticated_methods)` returns the complete
value-obligation set. `discharge(&manifest, &authenticated_methods, &obligations)`
checks exact value-only coverage using the same per-edge predicate as flat
verification. It does not allow string-lane substitution or run bb. The methods
must come from issuer authentication; missing authentication is refused.

Only `!=` and `!(=)` normalize to `Ne`. General NOT remains outside the fragment;
a false verdict never discharges an obligation. Integers are non-negative,
sign-split temporal lanes do not admit signed integers, negative zero decimals
are refused, decimal scales rescale exactly, doubles canonicalize NaN/zero and
use IEEE comparisons, and booleans use the dedicated bit relation. Private
operand-domain checks remain circuit constraints.

V1 integer, boolean, double, decimal (all admitted scales), date and dateTime
value members remain refused at stage 0 with `LegacyV1ValueMember`.
`admit_legacy_dual_leaf_v1` preserves V1 string-lane verification only. The INV-VL
issuer-honesty assumption applies to V1 only. V2 lanes remain research-grade,
not externally audited; this change makes no external assurance claim.

Extended verification (`verify_fragment_manifest` / `skip_query_binding`) keeps
refusing every value member by name with `ValueLaneSubProofNotAccepted` (or
`LegacyV1ValueMember` for V1). It also refuses V2 scan/join members through its
existing encoding-policy refusal.
