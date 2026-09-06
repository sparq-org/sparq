#!/usr/bin/env python3
# [OPUS-4.8] written while Fable 5 unavailable — re-review when Fable returns.
#
# sq-1s2.1.2: Comprehensive SPARQL ZK gate-cost benchmark CATALOG generator.
#
# Builds bench/zk-compose/sparql_feature_catalog.json — a coverage map AND an
# optimisation-target list for the ZK-SPARQL engine, faithfully implementing the
# §6 catalog design of research/zk-field-native-encoding.md (PR #765).
#
# What it does (and, just as importantly, what it does NOT do):
#
#   * The QUERY catalog (the SPARQL 1.1 feature span, the representative query
#     shapes, and the member-NAME mapping) is authored here as CATALOG, below —
#     the design's §6.2 table, faithfully transcribed.
#   * The `circuit_size` for every COVERED query is NOT written here. It is
#     JOINED from the regression-gated snapshot
#     (crates/sparq-zk-compose/tests/gate_count_snapshot.json) — the SAME source
#     of truth bench/zk-compose/scripts/gate_counts.sh and the in-crate
#     `gate_count_regression` test (tests/gate_count.rs) use. So a member's gate
#     count is NEVER hand-copied into two files that can disagree, and the
#     benchmark can never drift from the regression-gated members.
#   * For a feature with NO ZK circuit today (zero-or-one paths, aggregates,
#     BIND expression eval, string predicates, dateTime compare, negation,
#     OPTIONAL) we record `circuit_size: null` and a `status` carrying the
#     "NO ZK CIRCUIT YET" marker. We NEVER fabricate a gate number.
#   * A feature EXCLUDED BY DESIGN (OPTIONAL, aggregates, negation — non-monotone
#     / closed-world, research/zksparql-fragment-extension.md §3.2) is a gap that
#     says so: it is not "not built yet", it is "not admissible", and calling it
#     merely verifier-side would OVERCLAIM.
#   * A feature with a BOUNDED in-circuit statement (the `path_reach` family for
#     `p+`/`p*`) is `partial`, NOT `covered`: it carries its real joined member
#     sizes AND a status that spells out what is not proved (longer paths,
#     completeness). Under-stating and over-stating are both dishonest.
#
# Honesty is load-bearing (§6.1): the benchmark is a coverage map, not a
# cherry-picked win list. The `gap` rows are the larger story; surfacing them is
# the point. The high-gate covered members (the blake3-bound `filter_*` family at
# 17,416) are auto-flagged HIGH_GATE_* as the value-hook reduction target (§2.6).
#
# sq-3kd2g.11 refresh — status now reflects ACCEPTED SYNTAX at the verifier
# fragment gate (crates/sparq-zk/src/verify.rs::fragment_query), not merely
# whether a desugared form compiles to some circuit. The fragment-gate extension
# (sq-3kd2g.3 gate + sq-3kd2g.6 builder/dispatch, research/zksparql-fragment-
# extension.md §§1,3,4,7) made property paths, UNION, VALUES and subqueries
# ACCEPTED, while OPTIONAL/MINUS/BIND/aggregates/EXISTS stay REJECTED fail-closed.
# So a row is `covered` only when the gate accepts the syntax AND a dedicated
# circuit member proves the feature AS STATED; `partial` when accepted but either
# composed verifier-side (no dedicated member) or proved only under a BOUNDED
# statement; `gap` when the gate rejects it fail-closed, OR when it is accepted but
# no compiled member can be bound (p?, which pins depth to exactly 1 — see Q16).
# This fixes the §1 Q13/Q14 drift where `covered` had meant only "a desugared form
# compiles" while the path syntax was still rejected. The closures p+/p* map to
# the path_reach family under an EXISTENCE-ONLY bounded statement (§4) and carry
# a `bounded` honesty flag alongside STATUS_BOUNDED; the depth bound is a public
# input, never a claim of unbounded closure.
#
# It does NOT require live `bb`/`nargo`: every number comes off the committed
# snapshot, so the harness is deterministic and non-flaky (CI-safe). To refresh
# the underlying gate counts, re-run bench/zk-compose/scripts/gate_counts.sh and
# re-baseline the snapshot; this generator then re-joins the new numbers.
#
# NOTHING here is a soundness or privacy guarantee: the ZK estate is internally
# re-audited but NOT externally audited (sq-qhy4), the composition verifier is
# NOT-yet-sound, and the dual-leaf value lane carries the accepted INV-VL
# trusted-issuer-honesty downgrade (#769 / CR-G8). Coverage in this catalog means
# "a circuit member exists and dispatch binds it", never "proved secure".
#
# Usage:
#   bench/zk-compose/scripts/sparql_catalog.py \
#       > bench/zk-compose/sparql_feature_catalog.json
#   bench/zk-compose/scripts/sparql_catalog.py --check
#
# `--check` re-generates in memory and fails (exit 1) unless the committed JSON is
# BYTE-IDENTICAL to it and self-consistent with the snapshot (the same invariants
# the Rust guard enforces), so regeneration is provably idempotent in CI.
#
# The committed JSON is regression-gated by the Rust test
# `crates/sparq-zk-compose/tests/sparql_catalog.rs`, which fails if any covered
# query names a member absent from the snapshot, if any covered `circuit_size`
# disagrees with the snapshot, or if any gap carries a non-null gate number.

from __future__ import annotations

import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
SNAPSHOT_JSON = os.path.normpath(
    os.path.join(HERE, "..", "..", "..", "crates", "sparq-zk-compose", "tests", "gate_count_snapshot.json")
)
# The committed artefact `--check` compares against.
CATALOG_JSON = os.path.normpath(os.path.join(HERE, "..", "sparql_feature_catalog.json"))

# The blake3-bound numeric-FILTER members all land on this gate count; any covered
# query whose joined circuit_size meets or exceeds it is the value-hook reduction
# target (research/zk-field-native-encoding.md §2.6). Sourced as a member's joined
# value, not asserted: see HIGH_GATE_THRESHOLD use below.
HIGH_GATE_THRESHOLD = 17416

# Status strings. EVERY gap label MUST contain this marker so the Rust test and any
# downstream surface can detect a gap by SUBSTRING — that is the honesty hook, and a
# new gap flavour must keep it rather than invent a friendlier word.
GAP_MARKER = "NO ZK CIRCUIT YET"
STATUS_COVERED = "covered"
STATUS_PARTIAL = "partial (verifier-side / desugars to a covered primitive)"
# A BOUNDED in-circuit statement: real members, real joined gate counts, but the
# proved statement is strictly weaker than the SPARQL feature. Deliberately in the
# `partial` bucket (it is not `covered`) and deliberately NOT a gap (it is not null).
STATUS_BOUNDED = (
    "partial (BOUNDED in-circuit statement — the bound is a PUBLIC input; "
    "longer witnesses and completeness are NOT proved)"
)
STATUS_GAP = f"{GAP_MARKER} (gap)"
# Not "not built yet" but "not admissible": a non-monotone / closed-world construct
# the monotone proof model cannot carry (research/zksparql-fragment-extension.md §3.2).
STATUS_OUT_OF_FRAGMENT = (
    f"{GAP_MARKER} (gap — EXCLUDED BY DESIGN: non-monotone / closed-world, "
    "not merely unbuilt)"
)


def is_gap(status: str) -> bool:
    """A gap is detected by SUBSTRING, in lockstep with the Rust guard's GAP_MARKER."""
    return GAP_MARKER in status

# The value-hook reduction target reference, shared by every flagged high-gate
# numeric-FILTER member.
VALUE_HOOK_TARGET = "value-hook (research/zk-field-native-encoding.md §2.6)"
# An ESTIMATE, explicitly NOT a measured number — it MUST be re-measured with
# `bb gates` once the value-hook lands (§6.3). Carried as prose with the ESTIMATE
# qualifier so it can never be mistaken for a measurement. Used ONLY for a lane with
# no compiled value-lane member: where the lane HAS landed, the row carries
# `value_lane_member` and its MEASURED joined size instead, because keeping a
# projection next to a measurement is exactly the drift this catalog exists to stop.
VALUE_HOOK_PROJECTED = "ESTIMATE ~3200 — MUST be re-measured with bb gates (NOT a measurement)"

# The compiled bounded-depth path-reachability family (the sq-3kd2g.2 circuits,
# dispatched by sq-3kd2g.6's builder under the opt-in `extended-fragment` feature).
# These members carry the property-path closures p?/p+/p* under the BOUNDED,
# EXISTENCE-ONLY statement of research/zksparql-fragment-extension.md §4 — the depth
# bound is a PUBLIC input; a bounded proof never asserts absence or completeness.
# circuit_size is joined from the snapshot like every other member (never fabricated).
PATH_REACH_MEMBERS = [
    "path_reach_d2_k1_n16",
    "path_reach_d4_k1_n16",
    "path_reach_d4_k2_n16",
    "path_reach_d8_k1_n16",
]

# ---------------------------------------------------------------------------
# The query catalog — §6.2 of research/zk-field-native-encoding.md, transcribed.
#
# Each entry is the AUTHORITATIVE design row: an id, the SPARQL 1.1 feature it
# exercises, a representative query, the ZK circuit member(s) it compiles to (by
# NAME — circuit_size is joined from the snapshot), a coverage status, and, for
# covered high-gate members, a reduction target. NO circuit_size literal lives
# here; that is the anti-drift contract.
#
# `zk_members`: member names that MUST exist in the snapshot (joined → circuit_size).
#               Empty list ⇒ gap or verifier-side-only (circuit_size = null).
# `representative_members`: for a covered query that SPANS a family (e.g. a BGP scan
#               over the (k,n,r) lattice), the subset whose joined sizes are reported
#               as the cost RANGE. Defaults to zk_members when omitted.
# `value_lane_member`: the DUAL-LEAF value-lane sibling that proves the SAME feature at
#               a lower cost when the commitment method committed a value handle
#               (`filter_value_dl_*`, sq-2ezsx). Its size is JOINED too — it is a
#               measurement, which is why a row that has one drops `projected_after`.
# ---------------------------------------------------------------------------
CATALOG: list[dict] = [
    {
        "id": "Q01_bgp_single_pattern",
        "feature": "BGP (1 triple pattern)",
        "sparql": "SELECT ?s ?o WHERE { ?s :p ?o }",
        "zk_members": ["scan_k1_n16_r4", "scan_k1_n16_r8", "scan_k1_n64_r4", "scan_k1_n64_r8"],
        "status": STATUS_COVERED,
        "note": "Maps to the disclosed-row scan family over the (n,r) lattice (k=1, one graph).",
    },
    {
        "id": "Q02_bgp_multi_pattern",
        "feature": "BGP (multi-pattern, single graph)",
        "sparql": "SELECT ?s ?v WHERE { ?s :p ?o . ?s :q ?v }",
        "zk_members": ["scan_k2_n16_r4", "scan_k2_n16_r8", "scan_k2_n64_r4", "scan_k2_n64_r8"],
        "status": STATUS_COVERED,
        "note": "Multiple shared-subject patterns map to the k=2 scan lattice (k = committed-graph / pattern width).",
    },
    {
        "id": "Q03_filter_integer_ge",
        "feature": "FILTER xsd:integer (>=, <, =, !=)",
        "sparql": "SELECT ?p WHERE { ?p :age ?age FILTER(?age >= 18) }",
        "zk_members": ["filter_int_d1", "filter_int_d2", "filter_int_d3", "filter_int_d4"],
        "status": STATUS_COVERED,
        "reduction_target": VALUE_HOOK_TARGET,
        "value_lane_member": "filter_value_dl_int",
        "note": (
            "The complex numeric FILTER family. Gate-identical across d (the blake3 token fits "
            "one 64-byte block for d<=19). The value hook has LANDED for this lane: "
            "`filter_value_dl_int` is the MEASURED dual-leaf sibling (see value_lane_circuit_size, "
            "joined — not a projection). It is legal ONLY against a commitment method that "
            "committed a value handle (`dual-leaf` / the `value-only` research dial, "
            "crates/sparq-zk-compose/src/dispatch.rs), so the blake3 digit-byte members remain the "
            "cost under `string-canonical`. The value lane carries the accepted INV-VL "
            "trusted-issuer-honesty downgrade (#769 / CR-G8, sq-qhy4)."
        ),
    },
    {
        "id": "Q04_filter_signed_integer_ge",
        "feature": "FILTER signed xsd:integer (negative operands)",
        "sparql": "SELECT ?a WHERE { ?a :balance ?bal FILTER(?bal >= -100) }",
        "zk_members": ["filter_signed_int_d2", "filter_signed_int_d4"],
        "status": STATUS_COVERED,
        "reduction_target": VALUE_HOOK_TARGET,
        "projected_after": VALUE_HOOK_PROJECTED,
        "note": "Signed-int lane; same blake3-binding cost driver as filter_int.",
    },
    {
        "id": "Q05_filter_decimal_le",
        "feature": "FILTER xsd:decimal",
        "sparql": "SELECT ?o WHERE { ?o :amount ?amt FILTER(?amt <= 199.99) }",
        "zk_members": ["filter_decimal_i3_f2"],
        "status": STATUS_COVERED,
        "reduction_target": VALUE_HOOK_TARGET,
        "value_lane_member": "filter_value_dl_decimal",
        "note": (
            "Scaled-decimal lane (i integer digits, f fractional digits). The value hook has "
            "LANDED for this lane: `filter_value_dl_decimal` is the MEASURED dual-leaf sibling "
            "(joined, not projected), legal only under a value-committing method — same "
            "dispatch rule and same INV-VL downgrade as Q03."
        ),
    },
    {
        "id": "Q06_filter_double_gt",
        "feature": "FILTER xsd:double (IEEE-754 compare)",
        "sparql": "SELECT ?x WHERE { ?x :reading ?d FILTER(?d > 42.0) }",
        "zk_members": ["filter_f64_d1", "filter_f64_d2", "filter_f64_d3", "filter_f64_d4"],
        # The raw-compare floor (a pure sparq_ieee754 comparison, no string hash) is
        # reported alongside as the lower bound the value-hook is reaching toward.
        "floor_member": "filter_f64",
        "status": STATUS_COVERED,
        "reduction_target": VALUE_HOOK_TARGET,
        "value_lane_member": "filter_value_dl_f64",
        "note": (
            "Manifest-composable double FILTER, blake3-bound like filter_int; the raw-compare "
            "floor `filter_f64` shows the achievable lower bound. The value hook has LANDED for "
            "this lane: `filter_value_dl_f64` is the MEASURED dual-leaf sibling (joined, not "
            "projected) — it sits above the raw-compare floor because it still binds the operand "
            "through the dual-leaf Poseidon2 components. Same dispatch rule and INV-VL downgrade "
            "as Q03."
        ),
    },
    {
        "id": "Q07_filter_boolean_eq",
        "feature": "FILTER xsd:boolean (=, !=, and the degenerate order false < true)",
        "sparql": "SELECT ?x WHERE { ?x :active ?flag FILTER(?flag = true) }",
        "zk_members": ["filter_value_dl_int"],
        "status": STATUS_COVERED,
        "note": (
            "Boolean value lane (sq-5xdlk, #3973) — it adds NO new Noir member: it REUSES "
            "`filter_value_dl_int` with the PUBLIC boolean `datatype_const` = "
            "blake3_field(xsd:boolean) and the value hooks {0 = false, 1 = true} inside that "
            "member's u64 comparison domain, so XSD's false<true order (and the degenerate "
            "LT/LE/GT/GE verdicts) falls out of the integer comparison unchanged. Lane separation "
            "is the public datatype_const and only that: an integer leaf recomputed under the "
            "boolean constant fails the member's leaf binding — a BINDING argument resting on "
            "Poseidon2 preimage resistance, NOT an audited soundness claim. Opt-in (`dual-leaf`) "
            "and only against a value-committing method; the blake3 digit-byte lane has no "
            "boolean member. The encoder is fail-closed on non-canonical lexicals (\"1\"/\"0\" "
            "are rejected). Inherits the value lane's INV-VL downgrade (#769 / CR-G8); NOT "
            "externally audited (sq-qhy4)."
        ),
    },
    {
        "id": "Q08_filter_string_ops",
        "feature": "FILTER string ops (STR, STRLEN, CONTAINS, STRSTARTS, STRENDS, SUBSTR, REGEX)",
        "sparql": 'SELECT ?x WHERE { ?x :name ?n FILTER(CONTAINS(STR(?n), "a")) }',
        "zk_members": [],
        "status": STATUS_GAP,
        "reduction_target": (
            "expression fragment, phase 3 (research/zksparql-fragment-extension.md §5.1) — needs "
            "a string VALUE_HOOK plus the EBV + error lane; REGEX would be admitted for the "
            "BOUNDED subset only (literal / anchored / char-class, sq-y73), full fn:matches stays "
            "a documented gap"
        ),
        "note": (
            "No in-circuit string predicate: the COMPOSITION layer commits the opaque lexical "
            "component of a literal and binds no function over it, so a string FILTER has no "
            "provable member and the fragment gate fail-closes. The string FUNCTION estate exists "
            "but OUT OF TREE (externalized to sparq-org/noir_XPath, #1599) and is not wired to a "
            "committed leaf. Locale collation is out of scope even after phase 3 (codepoint order "
            "only)."
        ),
    },
    {
        "id": "Q09_filter_datetime_compare",
        "feature": "FILTER xsd:dateTime / date / time / duration compare",
        "sparql": 'SELECT ?e WHERE { ?e :when ?d FILTER(?d >= "2020-01-01T00:00:00Z"^^xsd:dateTime) }',
        "zk_members": [],
        "status": STATUS_GAP,
        "reduction_target": (
            "the dateTime VALUE_HOOK has LANDED for the xsd:dateTime / xsd:date HALF of this row "
            "(`filter_value_dl_datetime`, sq-wz99x, research/zk-field-native-encoding.md §13); "
            "xsd:time / xsd:duration remain design-only — expression fragment phase 3 "
            "(research/zksparql-fragment-extension.md §5.1)"
        ),
        "note": (
            "STILL A GAP AS STATED, but no longer for the reason it was: the "
            "`filter_value_dl_datetime` member IS now compiled (sq-wz99x — the epoch-based value "
            "handle on the dual-leaf value lane, ONE member serving BOTH the xsd:dateTime and "
            "xsd:date lanes via the public datatype_const), and it DOES bind a committed dateTime "
            "literal to an ordering claim. Three things keep this ROW a gap, and no gate number "
            "is joined onto it because a gap carries none: (1) `xsd:time` and `xsd:duration` — "
            "named in this row's feature — have NO value handle and no member; (2) the member's "
            "hookable domain is strict XSD-canonical `Z`-timezoned lexicals ONLY, so a bare "
            "(un-timezoned) or non-`Z`-offset operand is REJECTED fail-closed rather than "
            "compared — the §13.2 indeterminacy rule, matching the engine's own residual partial "
            "order (sq-2k5py); (3) the composition path is the value lane's usual audit-gated "
            "one (host commit-pipeline integration sq-j506, verify_manifest dispatch design bead "
            "6). Timezone normalisation and the indeterminate-comparison case, previously "
            "'unspecified for the ZK lane', are now PINNED by §13.2/§13.4 — the two documented "
            "widenings (offset normalisation; bare lexicals as a disjoint sub-lane) are recorded "
            "in §13.6 and deliberately NOT in this slice. Splitting this row so the landed "
            "dateTime/date half can be scored separately from time/duration is follow-up work. "
            "Inherits the value lane's INV-VL downgrade (#769 / CR-G8); the §13 rule set is "
            "itself an OPEN external-audit obligation — NOT externally audited (sq-qhy4)."
        ),
    },
    {
        "id": "Q10_optional",
        "feature": "OPTIONAL (LeftJoin)",
        "sparql": "SELECT ?s ?v WHERE { ?s :p ?o OPTIONAL { ?s :q ?v } }",
        "zk_members": [],
        "status": STATUS_OUT_OF_FRAGMENT,
        "note": (
            "EXCLUDED BY DESIGN, not merely unbuilt (research/zksparql-fragment-extension.md "
            "§3.2): a solution that leaves the optional side UNBOUND asserts that no compatible "
            "extension exists — a closed-world, non-monotone claim a membership proof cannot "
            "carry. crates/sparq-zk/src/verify.rs fail-closes on LeftJoin "
            "(VerifyError::UnsupportedFragment) in BOTH the base and "
            "the extended fragment gate. The BOUND case is semantically a plain Join (Q12), so "
            "admitting OPTIONAL would add no sound expressiveness; authors rewrite to a Join or "
            "to a UNION of the explicit cases. (This row previously read `partial — the left-join "
            "is verifier-side`, which OVERCLAIMED: there is no verifier-side construction that "
            "makes the unbound case sound.)"
        ),
    },
    {
        "id": "Q11_union",
        "feature": "UNION",
        "sparql": "SELECT ?s WHERE { { ?s :p ?o } UNION { ?s :q ?o } }",
        "zk_members": [],
        "status": STATUS_PARTIAL,
        "note": (
            "Monotone (eval is the set union of the branch evals) and ADMITTED by the extended "
            "fragment gate (crates/sparq-zk/src/verify.rs, sq-3kd2g.3) with PER-SOLUTION BRANCH "
            "ATTRIBUTION: each disclosed solution names the branch it witnesses, and the verifier "
            "re-derives that branch's pattern list from the query text and checks the branch's "
            "own sub-proofs. No new circuit primitive and no dedicated member — the cost is the "
            "witnessing branch's scan/join/path members (Q01/Q02/Q12/Q17), which is why "
            "circuit_size stays null here rather than double-counting them. Branch count is "
            "capped (MAX_FRAGMENT_BRANCHES) so a query cannot fan out unboundedly."
        ),
    },
    {
        "id": "Q12_join_shared_var_hidden",
        "feature": "JOIN on a shared variable (hidden equi-join)",
        "sparql": "SELECT ?p ?s WHERE { ?p :worksFor :ACME . ?p :hasSalary ?s }",
        "zk_members": ["join_eq_na16_nb16", "join_eq_na16_nb64", "join_eq_na64_nb16", "join_eq_na64_nb64"],
        "status": STATUS_COVERED,
        "note": "Hidden equi-join over the (na,nb) bucket lattice.",
    },
    {
        "id": "Q13_path_sequence",
        "feature": "Property path / (sequence)",
        "sparql": "SELECT ?o WHERE { ?s :p/:q ?o }",
        "zk_members": ["scan_k2_n16_r4", "scan_k2_n16_r8", "scan_k2_n64_r4", "scan_k2_n64_r8"],
        "status": STATUS_COVERED,
        "note": "Path syntax now ACCEPTED by the fragment gate (rewritten to a fresh-intermediate BGP — the SPARQL 1.1 translation, design §4); the desugared multi-pattern BGP maps to the k=2 scan lattice. (Pre-extension this row was 'covered' only because the desugared form compiled while the path syntax was still rejected — §1 drift, now resolved.)",
    },
    {
        "id": "Q14_path_inverse",
        "feature": "Property path ^ (inverse)",
        "sparql": "SELECT ?s WHERE { ?o ^:p ?s }",
        "zk_members": ["scan_k1_n16_r4", "scan_k1_n16_r8", "scan_k1_n64_r4", "scan_k1_n64_r8"],
        "status": STATUS_COVERED,
        "note": "Path syntax now ACCEPTED by the fragment gate (endpoint swap, design §4); maps to the k=1 scan family (no extra cost over a forward pattern). (Pre-extension this row was 'covered' only because the desugared form compiled while the path syntax was still rejected — §1 drift, now resolved.)",
    },
    {
        "id": "Q15_path_alternative",
        "feature": "Property path | (alternative)",
        "sparql": "SELECT ?o WHERE { ?s (:p|:q) ?o }",
        "zk_members": [],
        "status": STATUS_PARTIAL,
        "note": (
            "ACCEPTED by the fragment gate; desugars to a UNION of per-predicate branches and "
            "lands with exactly the Q11 branch "
            "attribution (research/zksparql-fragment-extension.md §4 table): the verifier "
            "re-derives the branches from the query text and checks the witnessing branch's "
            "sub-proofs. No dedicated member — the cost is that branch's own scan (Q01/Q02) or "
            "bounded-path (Q17/Q18) member."
        ),
    },
    {
        "id": "Q16_path_zero_or_one",
        "feature": "Property path ? (zero-or-one)",
        "sparql": "SELECT ?o WHERE { ?s :p? ?o }",
        "zk_members": [],
        "status": STATUS_GAP,
        "reduction_target": (
            "compile a `path_reach_d1_k{k}_n{n}` member — the `?` closure pins the depth bound to "
            "EXACTLY 1 and the family's smallest compiled depth is d=2; the relation itself "
            "already carries the zero-length case through the public `allow_zero` input, so this "
            "is a lattice extension, not new circuit logic"
        ),
        "note": (
            "NOT provable today. The extended fragment gate admits `?` and routes it to the "
            "path_reach family with allow_zero=true and a FIXED depth bound of 1 "
            "(PathClosure::ZeroOrOne.fixed_k() == 1), but no d=1 "
            "member is compiled — every compiled member is d>=2 — so dispatch fail-closes "
            "(PathDepthExceedsClosure, "
            "crates/sparq-zk-compose/src/verifier.rs). A DEEPER member is deliberately NOT "
            "accepted as a substitute: proofs at different depth bounds are DIFFERENT statements "
            "(research/zksparql-fragment-extension.md §4 req 1), and a d=2 member would prove a "
            "two-step path the query never asked for. p+/p* (Q17/Q18) are unaffected because "
            "their closures have no fixed depth (fixed_k() == None), so any compiled bound "
            "satisfies the dispatcher."
        ),
    },
    {
        "id": "Q17_path_one_or_more",
        "feature": "Property path + (one-or-more, transitive closure) — BOUNDED depth only",
        "sparql": "SELECT ?o WHERE { :s :p+ ?o }",
        "zk_members": PATH_REACH_MEMBERS,
        "status": STATUS_BOUNDED,
        "bounded": True,
        "reduction_target": (
            "UNBOUNDED closure stays OUT (not circuit-realizable); reaching further is a "
            "compile-time lattice extension (larger d / n), and each d is a DISTINCT verification "
            "key by design so the bound cannot be silently widened"
        ),
        "note": (
            "ACCEPTED by the fragment gate and mapped to the bounded path_reach family "
            "(sq-3kd2g.2 #1591 / sq-3kd2g.6; opt-in `extended-fragment`). The "
            "member proves EXACTLY: there exists a chain of committed triples (t_1..t_l) with "
            "1<=l<=D, each a member of a committed graph in the disclosed attribution set with "
            "predicate p, chained object-to-subject, connecting the endpoints — where D is a "
            "PUBLIC input (`depth_bound`, constant-constrained to the member's compiled d) and "
            "unused steps are constrained pass-throughs. EXISTENCE ONLY and MONOTONE: it never "
            "asserts that no longer path exists nor that the reachable set is complete, so "
            "eval_D is a SUBSET of eval and failing to prove at depth D proves NOTHING. Compiled "
            "lattice today: (d,k,n) in {(2,1,16),(4,1,16),(4,2,16),(8,1,16)}, EXACT-match dispatch "
            "(no wrong-bucket fallback). Reported as `partial`, not `covered`, because the proved "
            "statement is strictly weaker than SPARQL `p+` — the `bounded` honesty flag marks "
            "that explicitly. Padding soundness is forge-tested but "
            "NOT externally audited (sq-qhy4)."
        ),
    },
    {
        "id": "Q18_path_zero_or_more",
        "feature": "Property path * (zero-or-more, reflexive transitive closure) — BOUNDED depth only",
        "sparql": "SELECT ?o WHERE { :s :p* ?o }",
        "zk_members": PATH_REACH_MEMBERS,
        "status": STATUS_BOUNDED,
        "bounded": True,
        "reduction_target": (
            "as Q17 — the unbounded closure stays OUT; only the compiled (d,k,n) lattice grows"
        ),
        "note": (
            "ACCEPTED by the fragment gate; the SAME path_reach members as Q17 — the closure is "
            "a PUBLIC input (`allow_zero` = "
            "true), not a separate member, and the verifier rejects a proof whose allow_zero "
            "disagrees with the closure re-derived from the query. Adds the zero-length case "
            "(0<=l<=D): it holds when the endpoints are the same term AND that term OCCURS in the "
            "committed union, so the circuit requires an occurrence witness rather than bare "
            "equality (research/zksparql-fragment-extension.md §4 req 5). Same bounded "
            "EXISTENCE-ONLY statement, same public depth bound, same `bounded` honesty flag and "
            "same not-externally-audited caveat (sq-qhy4) as Q17."
        ),
    },
    {
        "id": "Q19_aggregate_group_by",
        "feature": "Aggregation (GROUP BY / HAVING / COUNT / SUM / AVG / MIN / MAX / GROUP_CONCAT / SAMPLE)",
        "sparql": "SELECT ?g (COUNT(?x) AS ?n) WHERE { ?x :inGroup ?g } GROUP BY ?g",
        "zk_members": [],
        "status": STATUS_OUT_OF_FRAGMENT,
        "reduction_target": (
            "re-entry requires a COMPOSED-PATTERN completeness obligation (a separate future "
            "program, research/zksparql-fragment-extension.md §3.2) — not a circuit-size problem, "
            "so there is nothing here to optimise toward"
        ),
        "note": (
            "EXCLUDED BY DESIGN: an aggregate VALUE is a completeness claim over the WHOLE pattern "
            "(\"exactly these solutions and no more\") — closed-world at pattern level. Only "
            "PER-BGP-SCAN completeness is proved today, never composed-pattern completeness, and "
            "double-counting across credentials is forgeable. The COUNT-forgery guard in scan.nr "
            "is anti-forgery over DISCLOSED rows, NOT a proven aggregate — reading it as one is "
            "the exact overclaim this row exists to prevent. crates/sparq-zk/src/verify.rs "
            "fail-closes on Group and on Extend-over-Group (the parse shape of "
            "`SELECT (COUNT(?x) AS ?n)`), so an aggregate is reported as aggregation, not BIND."
        ),
    },
    {
        "id": "Q20_subquery",
        "feature": "Subquery (nested SELECT, no aggregates)",
        "sparql": "SELECT ?p WHERE { ?p :a ?x { SELECT ?p WHERE { ?p :b ?y } } }",
        "zk_members": [],
        "status": STATUS_PARTIAL,
        "note": (
            "Monotone when composed of in-fragment operators, and ADMITTED by the extended gate: a "
            "Project below the outer modifier stack is read as a subquery and its NON-projected "
            "variables are renamed apart, so an identically-named outer variable is never "
            "conflated (inner projection = existential quantification). No dedicated member — the "
            "cost is the inner branches' own members. Two carve-outs stay fail-closed: LIMIT / "
            "OFFSET INSIDE a subquery (a nondeterministic subset selection — a witness for the "
            "unrestricted inner evaluation is not a witness for every legal sliced evaluation), "
            "and subquery + aggregates (Q19)."
        ),
    },
    {
        "id": "Q21_bind_expression",
        "feature": "BIND (Extend — deterministic expression evaluation)",
        "sparql": "SELECT ?c WHERE { ?s :a ?a . ?s :b ?b BIND(?a + ?b AS ?c) }",
        "zk_members": [],
        "status": STATUS_GAP,
        "reduction_target": (
            "expression fragment, phase 3 (research/zksparql-fragment-extension.md §5) — the same "
            "in-circuit evaluator as FILTER with its result bound into the disclosed solution; "
            "blocked on the EBV + error lane"
        ),
        "note": (
            "No in-circuit expression evaluation: the composition layer binds only "
            "single-comparison numeric lanes, so a DERIVED binding cannot be proved and the gate "
            "fail-closes on Extend. Monotone in principle (it only adds a derived binding), so it "
            "is a buildable gap rather than a design exclusion — unlike Q10/Q19/Q23. The "
            "non-deterministic builtins (RAND, UUID, STRUUID, BNODE, NOW) stay OUT permanently: "
            "they have no deterministic in-circuit meaning."
        ),
    },
    {
        "id": "Q22_values",
        "feature": "VALUES (inline data, including UNDEF cells)",
        "sparql": "SELECT ?x WHERE { VALUES ?x { :a :b } ?x :p ?o }",
        "zk_members": [],
        "status": STATUS_PARTIAL,
        "note": (
            "Monotone — the rows are PUBLIC constants in the query text — and admitted by the "
            "extended gate: the verifier re-derives the VALUES block from the query and checks "
            "each disclosed solution against it, with UNDEF cells treated as wildcards and a row "
            "index outside the block's range rejected. No new circuit primitive and no dedicated "
            "member; the cost is the accompanying BGP's scan member (Q01/Q02)."
        ),
    },
    {
        "id": "Q23_negation_minus_notexists",
        "feature": "Negation (FILTER NOT EXISTS / MINUS)",
        "sparql": "SELECT ?s WHERE { ?s :p ?o FILTER NOT EXISTS { ?s :q ?v } }",
        "zk_members": [],
        "status": STATUS_OUT_OF_FRAGMENT,
        "reduction_target": (
            "no optimisation target: absence is not provable in a monotone membership model at "
            "any circuit size. The monotone NEIGHBOURS are what can land — the negated property "
            "set !(p1|…|pn) (a per-triple inequality against public constants) and positive "
            "FILTER EXISTS, both DEFERRED (research/zksparql-fragment-extension.md §3.2, §4)"
        ),
        "note": (
            "EXCLUDED BY DESIGN: closed-world negation / set difference is non-monotone — it "
            "asserts ABSENCE, which a membership proof cannot carry (more committed data could "
            "falsify it). crates/sparq-zk/src/verify.rs fail-closes on Minus and on any "
            "(NOT) EXISTS inside a FILTER, in both gate stages. Per-BGP scan completeness "
            "(scan.nr) is the closest primitive and is NOT a negation proof: it is completeness "
            "over ONE committed graph's disclosed rows, not over the world."
        ),
    },
    {
        "id": "Q24_revocation_status",
        "feature": "Revocation / status (hidden status-list index)",
        "sparql": "ASK { ?cred :revocationStatus ?bit FILTER(?bit = 0) }",
        "zk_members": ["revoke_unset_d10"],
        "status": STATUS_COVERED,
        "note": "Hidden status-list index proving the credential's status bit is unset.",
    },
    {
        "id": "Q25_issuer_attestation_hidden",
        "feature": "Issuer attestation (hidden tier, Schnorr)",
        "sparql": "ASK { ?cred :issuer ?iss FILTER(?iss IN (:tierA, :tierB)) }",
        "zk_members": ["hidden_issuer_d4"],
        "status": STATUS_COVERED,
        "reduction_target": "scalar-mul projective coords (research/zk-age-gatecount-reduction.md §3.3; bead sq-hb75)",
        "note": "Hidden-issuer Schnorr verification; the scalar-mul is the dominant cost (reduction target).",
    },
    {
        "id": "Q26_holder_possession_hidden",
        "feature": "Holder possession (hidden binding)",
        "sparql": "ASK { ?cred :holder ?h FILTER(?h = :me) }",
        "zk_members": ["holder_pok", "holder_set_d4"],
        "status": STATUS_COVERED,
        "note": "Holder proof-of-possession (holder_pok) and set-membership variant (holder_set_d4).",
    },
]


def load_snapshot() -> dict:
    with open(SNAPSHOT_JSON, encoding="utf-8") as fh:
        return json.load(fh)


def build_catalog(snapshot: dict) -> dict:
    members: dict[str, int] = snapshot["members"]

    # Fail loudly on a typo in CATALOG rather than silently emitting a gap: every
    # named member MUST exist in the regression-gated snapshot (the anti-drift
    # contract, §6.3). This is the generator-side half; the Rust test re-checks the
    # committed JSON.
    for entry in CATALOG:
        for key in ("zk_members", "floor_member", "value_lane_member"):
            named = entry.get(key, [] if key == "zk_members" else None)
            for m in named if isinstance(named, list) else ([named] if named else []):
                if m not in members:
                    raise SystemExit(
                        f"catalog error: query {entry['id']} names member '{m}' ({key}) "
                        f"absent from {SNAPSHOT_JSON} — fix the member name or re-baseline the snapshot"
                    )
        # Anti-fabrication, generator side: a GAP may not reach ANY member, so there
        # is no path by which a joined number could land on a gap row.
        if is_gap(entry["status"]) and (
            entry.get("zk_members") or entry.get("floor_member") or entry.get("value_lane_member")
        ):
            raise SystemExit(
                f"catalog error: query {entry['id']} is a GAP but names circuit members — "
                "a gap has no circuit and must never carry a gate number"
            )

    queries: dict[str, dict] = {}
    covered = partial = gap = 0
    high_gate: list[str] = []

    for entry in CATALOG:
        member_names = entry.get("zk_members", [])
        rep = entry.get("representative_members", member_names)

        # Join circuit_size from the snapshot — NEVER from CATALOG. Gaps and
        # verifier-side-only entries (no member) get a null circuit_size and an
        # empty per-member map. A real number is impossible to fabricate here:
        # it can only come from the snapshot's regression-gated value.
        per_member = {m: members[m] for m in member_names}
        rep_sizes = [members[m] for m in rep]
        circuit_size = max(rep_sizes) if rep_sizes else None
        circuit_size_min = min(rep_sizes) if rep_sizes else None

        # Bucketing MUST match the Rust guard's: gap wins by substring, then exact
        # "covered", everything else is partial. So a new status flavour can never
        # quietly re-bucket itself as covered.
        status = entry["status"]
        if is_gap(status):
            gap += 1
        elif status == STATUS_COVERED:
            covered += 1
        else:
            partial += 1

        # Flag high-gate covered members as reduction targets (§6.3). The flag is
        # derived from the JOINED snapshot value, never asserted. Two distinct
        # high-gate causes, kept apart so the value-hook target is not conflated
        # with a merely-large lattice corner:
        #   * HIGH_GATE_blake3_binding — the numeric-FILTER family (filter_*) whose
        #     17,416 cost is the blake3 token-binding the value-hook removes (§2.6).
        #     Identified by the presence of the value-hook reduction_target, NOT by
        #     size alone.
        #   * HIGH_GATE_lattice — a scan/join member whose JOINED size exceeds the
        #     threshold because of the (k,n,r)/(na,nb) lattice corner, not blake3.
        #     This is honest about WHY it is large (it is not the value-hook target).
        #   * BOUNDED_DEPTH_EXISTENCE_ONLY — a REAL in-circuit member proving a
        #     strictly weaker statement than the SPARQL feature (the path_reach
        #     family). Kept apart from VERIFIER_SIDE_OR_DESUGARED because the cost
        #     is real and in-circuit, and apart from `covered` because the statement
        #     is bounded.
        flag = None
        if is_gap(status):
            flag = "NO_ZK_CIRCUIT_YET"
        elif status == STATUS_BOUNDED:
            flag = "BOUNDED_DEPTH_EXISTENCE_ONLY"
        elif status != STATUS_COVERED:
            flag = "VERIFIER_SIDE_OR_DESUGARED"
        elif entry.get("reduction_target") == VALUE_HOOK_TARGET:
            # The blake3-bound numeric-FILTER family: the value-hook target.
            flag = "HIGH_GATE_blake3_binding"
            high_gate.append(entry["id"])
        elif circuit_size is not None and circuit_size >= HIGH_GATE_THRESHOLD:
            flag = "HIGH_GATE_lattice"
            high_gate.append(entry["id"])

        out: dict = {
            "feature": entry["feature"],
            "sparql": entry["sparql"],
            "zk_members": member_names,
            "circuit_size": circuit_size,
            "status": status,
            "flag": flag,
        }
        # Honesty flag for the bounded property-path closures (p?/p+/p*): they are
        # covered ONLY under the EXISTENCE-ONLY bounded statement (§4), never the
        # unbounded SPARQL closure. Surfaced so a consumer can never read a bounded
        # path proof as an unbounded-reachability claim.
        if entry.get("bounded"):
            out["bounded"] = True
        # Report the per-member sizes and the covered range so a site surface can
        # render either the headline (max) or the spread.
        if per_member:
            out["circuit_size_per_member"] = per_member
            if circuit_size_min != circuit_size:
                out["circuit_size_range"] = [circuit_size_min, circuit_size]
        if "floor_member" in entry:
            out["floor_member"] = entry["floor_member"]
            out["floor_circuit_size"] = members[entry["floor_member"]]
        # The LANDED dual-leaf value-lane sibling: a MEASUREMENT, joined like every
        # other number here — which is why such a row carries no `projected_after`.
        if "value_lane_member" in entry:
            out["value_lane_member"] = entry["value_lane_member"]
            out["value_lane_circuit_size"] = members[entry["value_lane_member"]]
        if "reduction_target" in entry:
            out["reduction_target"] = entry["reduction_target"]
        if "projected_after" in entry:
            out["projected_after"] = entry["projected_after"]
        if "note" in entry:
            out["note"] = entry["note"]

        queries[entry["id"]] = out

    return {
        "_comment": (
            "[OPUS-4.8] sq-1s2.1.2 comprehensive SPARQL ZK gate-cost catalog. A "
            "COVERAGE MAP (which circuit member each SPARQL 1.1 feature compiles to, "
            "or null for a gap) AND an OPTIMISATION-TARGET LIST (per-member bb-gates "
            "circuit_size, with the blake3-bound filter_* family flagged HIGH_GATE_* "
            "as the value-hook reduction target). circuit_size is JOINED from "
            "crates/sparq-zk-compose/tests/gate_count_snapshot.json (the "
            "regression-gated source of truth) so it can never drift; gaps carry "
            "null and are NEVER given a fabricated number. STATUS derives from the "
            "verifier fragment gate (crates/sparq-zk/src/verify.rs::fragment_query): "
            "accepted-with-a-dedicated-member = covered, accepted-but-composed-"
            "verifier-side or accepted-under-a-BOUNDED-statement = partial, "
            "rejected-fail-closed = gap — NOT whether a "
            "desugared form merely compiles (sq-3kd2g.11 fixes the §1 Q13/Q14 drift). "
            "Regenerate with bench/zk-compose/scripts/sparql_catalog.py (--check "
            "asserts the committed copy is byte-identical); gated by "
            "crates/sparq-zk-compose/tests/sparql_catalog.rs. Design: "
            "research/zk-field-native-encoding.md §6 (PR #765) + "
            "research/zksparql-fragment-extension.md §§1,3-5 (epic sq-3kd2g / #1591). "
            "NOT a soundness or privacy claim: the ZK estate is internally "
            "re-audited but NOT externally audited (sq-qhy4), the composition "
            "verifier is NOT-yet-sound, and 'covered' means a member exists and "
            "dispatch binds it — never 'proved secure'."
        ),
        "status_vocabulary": {
            STATUS_COVERED: (
                "a real circuit member proves the feature as stated; circuit_size is joined"
            ),
            STATUS_PARTIAL: (
                "no dedicated member — composed verifier-side or desugared into a covered "
                "primitive, so circuit_size is null and the cost is the underlying member's"
            ),
            STATUS_BOUNDED: (
                "a real member proving a STRICTLY WEAKER, explicitly bounded statement; the "
                "joined circuit_size is real but the row is NOT 'covered'"
            ),
            STATUS_GAP: "no circuit today; buildable in principle",
            STATUS_OUT_OF_FRAGMENT: (
                "non-monotone / closed-world — excluded by design, not awaiting implementation"
            ),
        },
        "tool": snapshot.get("tool", "bb gates -s ultra_honk"),
        "bb_version": snapshot.get("bb_version_baselined"),
        "nargo_version": snapshot.get("nargo_version_baselined"),
        "snapshot_source": "crates/sparq-zk-compose/tests/gate_count_snapshot.json",
        "high_gate_threshold": HIGH_GATE_THRESHOLD,
        "summary": {
            "total_queries": len(CATALOG),
            "covered": covered,
            "partial": partial,
            "gaps": gap,
            "high_gate_flagged": high_gate,
        },
        "queries": queries,
    }


def render(catalog: dict) -> str:
    """The single serialisation both `main` and `--check` use (no format drift)."""
    return json.dumps(catalog, indent=2) + "\n"


def self_consistency_errors(catalog: dict, snapshot: dict) -> list[str]:
    """Re-check the BUILT catalog against the snapshot, mirroring the Rust guard.

    The generator cannot fabricate a number by construction, but `--check` is also
    run against the COMMITTED file, where a hand edit could have. Kept deliberately
    independent of `build_catalog`'s internals so it is a real check, not an echo.
    """
    members: dict[str, int] = snapshot["members"]
    errors: list[str] = []
    covered = partial = gap = 0
    high_gate: list[str] = []

    for qid, q in catalog["queries"].items():
        status = q["status"]
        gap_row = is_gap(status)
        if gap_row:
            gap += 1
        elif status == STATUS_COVERED:
            covered += 1
        else:
            partial += 1
        if q.get("flag") in ("HIGH_GATE_blake3_binding", "HIGH_GATE_lattice"):
            high_gate.append(qid)

        # Anti-fabrication: a gap has no members and NO number, anywhere on the row.
        if gap_row:
            if q["circuit_size"] is not None:
                errors.append(f"{qid}: GAP carries circuit_size {q['circuit_size']} — never fabricate")
            if q["zk_members"]:
                errors.append(f"{qid}: GAP names members {q['zk_members']}")
            if q.get("flag") != "NO_ZK_CIRCUIT_YET":
                errors.append(f"{qid}: GAP must carry flag NO_ZK_CIRCUIT_YET, found {q.get('flag')!r}")
            for k in ("floor_circuit_size", "value_lane_circuit_size", "circuit_size_per_member"):
                if q.get(k):
                    errors.append(f"{qid}: GAP carries {k} — never fabricate")

        # Every number on the row is the snapshot's.
        for m, size in (q.get("circuit_size_per_member") or {}).items():
            if members.get(m) != size:
                errors.append(f"{qid}: per-member {m}={size} != snapshot {members.get(m)}")
        for name_key, size_key in (
            ("floor_member", "floor_circuit_size"),
            ("value_lane_member", "value_lane_circuit_size"),
        ):
            if name_key in q and members.get(q[name_key]) != q.get(size_key):
                errors.append(
                    f"{qid}: {name_key} {q[name_key]} size {q.get(size_key)} != "
                    f"snapshot {members.get(q[name_key])}"
                )
        if status == STATUS_COVERED:
            if not q["zk_members"]:
                errors.append(f"{qid}: covered but names no member")
            else:
                want = max(members[m] for m in q["zk_members"] if m in members)
                if q["circuit_size"] != want:
                    errors.append(
                        f"{qid}: covered circuit_size {q['circuit_size']} != max(member sizes) {want}"
                    )
        # A projection may never sit next to a measurement of the same thing.
        if "projected_after" in q and "value_lane_circuit_size" in q:
            errors.append(
                f"{qid}: carries BOTH a projected_after ESTIMATE and a MEASURED value lane — "
                "drop the projection"
            )
        if not q["sparql"].strip():
            errors.append(f"{qid}: empty SPARQL")

    s = catalog["summary"]
    for key, got, want in (
        ("total_queries", s["total_queries"], len(catalog["queries"])),
        ("covered", s["covered"], covered),
        ("partial", s["partial"], partial),
        ("gaps", s["gaps"], gap),
    ):
        if got != want:
            errors.append(f"summary.{key} = {got}, rows say {want}")
    if sorted(s["high_gate_flagged"]) != sorted(high_gate):
        errors.append(
            f"summary.high_gate_flagged {s['high_gate_flagged']} != flagged rows {high_gate}"
        )
    return errors


def check() -> int:
    """Fail unless the committed catalog is exactly what this generator produces."""
    snapshot = load_snapshot()
    want = render(build_catalog(snapshot))

    try:
        with open(CATALOG_JSON, encoding="utf-8") as fh:
            got = fh.read()
    except FileNotFoundError:
        print(f"FAIL: {CATALOG_JSON} is missing — regenerate it", file=sys.stderr)
        return 1

    errors: list[str] = []
    if got != want:
        errors.append(
            f"{CATALOG_JSON} is NOT what the generator produces — regenerate with:\n"
            "  bench/zk-compose/scripts/sparql_catalog.py > bench/zk-compose/sparql_feature_catalog.json"
        )
    # Check the COMMITTED bytes, not just the freshly-built ones: that is what the
    # Rust guard and the site consume.
    try:
        errors.extend(self_consistency_errors(json.loads(got), snapshot))
    except json.JSONDecodeError as exc:
        errors.append(f"committed catalog does not parse: {exc}")

    if errors:
        print("sparql_catalog --check FAILED:", file=sys.stderr)
        for e in errors:
            print(f"  - {e}", file=sys.stderr)
        return 1
    print(
        f"sparql_catalog --check OK: {CATALOG_JSON} is byte-identical to a fresh "
        "generation and self-consistent with the snapshot."
    )
    return 0


def main(argv: list[str]) -> int:
    if argv[1:] == ["--check"]:
        return check()
    if argv[1:]:
        print(f"usage: {argv[0]} [--check]", file=sys.stderr)
        return 2
    sys.stdout.write(render(build_catalog(load_snapshot())))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
