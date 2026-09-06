#!/usr/bin/env python3
# [OPUS-4.8] written while Fable 5 unavailable — re-review when Fable returns.
#
# sq-ot3x: per-config bb-gates benchmark MATRIX (each commitment method x circuit).
#
# Builds bench/zk-compose/bb_gates_matrix.json — the maintainer-requested (#769)
# comparison surface: for every (commitment-method, circuit) CONFIGURATION, is the
# pair LEGAL (dispatch-compatible under the unwired resolver rule — NOT a claim the
# config is wired end-to-end and provable today; see the caveat below), and — for
# the circuit — what is its ultra_honk bb-gates
# `circuit_size`. So the maintainer can compare, e.g., the string-canonical
# blake3-token FILTER lane (filter_int = 17,416 gates) against the dual-leaf
# value lane (filter_value_dl_int = 3,033 gates) it unlocks, and see exactly which
# configs each method admits.
#
# TWO axes, joined from two regression-gated sources of truth — NOTHING is
# hand-typed here that could drift:
#
#   * The GATE COUNT axis (circuit_size) is JOINED from the regression-gated
#     snapshot (crates/sparq-zk-compose/tests/gate_count_snapshot.json) — the SAME
#     source of truth gate_counts.sh, the in-crate `gate_count_regression` test,
#     and sparql_catalog.py use. A gate number is NEVER written here.
#   * The LEGALITY axis ((method, circuit) legal/illegal) mirrors the FAIL-CLOSED
#     dispatch rule of crates/sparq-zk-compose/src/dispatch.rs (`resolve_circuit`,
#     sq-cfmv) — the single source of truth for which pairs the resolver
#     structurally admits. The
#     committed JSON is cross-checked against BOTH the snapshot AND (under the
#     `dual-leaf` feature) the real `resolve_circuit` resolver by the Rust gate
#     crates/sparq-zk-compose/tests/bb_gates_matrix.rs, so a divergence is a hard
#     CI failure.
#
# The legality rule (design §3.1, research/zk-configurable-commitment-design.md;
# dispatch.rs):
#   * VALUE-LANE members (the `filter_value_dl_*` family — those that bind the
#     operand via the committed `value_component`) are LEGAL only against a method
#     that committed a value handle: `dual-leaf` and the `value-only` research
#     dial. ILLEGAL against `string-canonical` (no value handle — unprovable).
#   * STRING-LANE members (everything else — scan, join, path, revoke, issuer,
#     holder, and the blake3-token `filter_*` FILTER lanes) read the
#     string-canonical / lexical leaf, so they are LEGAL against `string-canonical`
#     and `dual-leaf` (whose `lexical_component` preserves term identity), ILLEGAL
#     against `value-only` (which dropped the lexical handle).
#
# HONESTY is load-bearing:
#   * `legal: true` means DISPATCH-COMPATIBLE, NOT end-to-end provable today. A
#     legal cell records that the (method, circuit) pair is *admitted* by the
#     resolver rule — design-level (structural) legality — and nothing more. It
#     does NOT assert the config can currently be committed, proved, and verified
#     end to end: the `dual-leaf` host-leaf encoding (sq-j506) is NOT yet
#     implemented, and `resolve_circuit` is NOT yet wired into `verify_manifest`.
#     Only `string-canonical` is implemented end-to-end today. This is the SAME
#     structural, unwired meaning the README honesty note and the Rust gate's
#     doc-comment (crates/sparq-zk-compose/tests/bb_gates_matrix.rs) use.
#   * NON-CANONICAL. bb-gates are measured on the work-box; they are a comparison
#     tool, NOT a canonical performance number (AGENTS.md; the bead: "NON-canonical
#     on work-box"). The circuit_size values here are exactly the snapshot's.
#   * The whole ZK estate is remediated + internally re-audited but NOT externally
#     audited (sq-qhy4, P0). `dual-leaf` and `value-only` carry the documented
#     INV-VL downgrade — value<->lexical agreement on the value-FILTER lane is
#     TRUSTED-ISSUER-HONESTY, not machine-enforced (#769 accepted at research grade;
#     gap CR-G8). `value-only` is a research/benchmark dial, never production.
#     This matrix asserts NO soundness or privacy property — only which
#     configurations are structurally legal and how many gates each circuit costs.
#   * An ILLEGAL cell carries `legal: false` + the fail-closed `reason` and NO
#     per-cell gate number (the config does not exist, so it has no cost). The
#     circuit's intrinsic `circuit_size` is recorded ONCE on the circuit row.
#
# It does NOT require live `bb`/`nargo`: every number comes off the committed
# snapshot, so the harness is deterministic and non-flaky (CI-safe). To refresh
# the underlying gate counts, re-run bench/zk-compose/scripts/gate_counts.sh and
# re-baseline the snapshot; this generator then re-joins the new numbers.
#
# Usage:
#   bench/zk-compose/scripts/bb_gates_matrix.py > bench/zk-compose/bb_gates_matrix.json

from __future__ import annotations

import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
SNAPSHOT_JSON = os.path.normpath(
    os.path.join(
        HERE, "..", "..", "..", "crates", "sparq-zk-compose", "tests", "gate_count_snapshot.json"
    )
)

# The commitment methods (crates/sparq-zk/src/commit.rs `CommitmentMethod` +
# registry.rs `ZK_SCHEME_*`). `value-only` is the feature-gated research dial
# (never production); it is included so the matrix is the COMPLETE (method x
# circuit) surface the maintainer compares, exactly as dispatch.rs's exhaustive
# legality sweep covers it under the `commitment-value-only` feature.
#
# `production_selectable` is narrowly ENUM/REGISTRY selectability: whether the
# method is a non-research member of the `CommitmentMethod` enum / `ZK_SCHEME_*`
# registry that a production config is allowed to name — NOT a claim it is
# operationally usable end-to-end today. `dual-leaf` is registry-selectable but its
# host-leaf encoding (sq-j506) is unimplemented and `resolve_circuit` is not wired
# into `verify_manifest`, so it is not yet an operational path (see the header
# HONESTY note); `value-only` is a research dial and never selectable in production.
METHODS = [
    {
        "key": "string-canonical",
        "iri": "https://sparq.dev/ns/zk#poseidon2-rdfc10-v1",
        "production_selectable": True,
        "removes_inv_vl": False,
        "note": "Conservative default + back-compat anchor; the only method "
        "implemented end-to-end today. Retains the in-circuit INV-VL invariant "
        "for the value-FILTER lane.",
    },
    {
        "key": "dual-leaf",
        "iri": "https://sparq.dev/ns/zk#poseidon2-dualleaf-v1",
        "production_selectable": True,
        "removes_inv_vl": True,
        "note": "The #769 value-optimised method: unlocks the value lane AND keeps "
        "term identity (via lexical_component). REMOVES INV-VL (value<->lexical "
        "agreement becomes trusted-issuer-honesty; CR-G8 / sq-qhy4). Opt-in "
        "(`dual-leaf` feature); host leaf encoding (sq-j506) not yet implemented.",
    },
    {
        "key": "value-only",
        "iri": "https://sparq.dev/ns/zk#poseidon2-valuehook-v1",
        "production_selectable": False,
        "removes_inv_vl": True,
        "note": "Benchmark / research dial ONLY, never production (§10 default (2)). "
        "Drops the lexical handle, losing term identity entirely — so every "
        "string-lane / identity member is illegal against it. Gated behind the "
        "OFF-by-default `commitment-value-only` feature.",
    },
]

# Mirrors dispatch.rs `is_value_lane_member`: the value-lane members are exactly
# the dual-leaf `filter_value_dl_*` family (they bind the operand via the
# committed `value_component`). Classifying by this prefix is faithful AND
# forward-compatible with future `filter_value_dl_*` datatype-class siblings.
VALUE_LANE_PREFIX = "filter_value_dl_"

# The fail-closed reject reason names for illegal cells — kept in lockstep with
# dispatch.rs `DispatchError`. Today no member is BOTH value-lane and an identity
# op, so only IllegalPair arises among the committed members.
REASON_ILLEGAL_PAIR = "IllegalPair"


def lane_of(member: str) -> str:
    return "value" if member.startswith(VALUE_LANE_PREFIX) else "string"


def legality(method_key: str, lane: str) -> dict:
    """The (method, lane) -> legal/illegal cell, mirroring dispatch::resolve_circuit."""
    if lane == "value":
        # value-lane: legal only where a value handle was committed.
        legal = method_key in ("dual-leaf", "value-only")
    else:
        # string-lane: legal where the string-canonical / lexical leaf is present.
        legal = method_key in ("string-canonical", "dual-leaf")
    if legal:
        return {"legal": True}
    return {"legal": False, "reason": REASON_ILLEGAL_PAIR}


def main() -> int:
    with open(SNAPSHOT_JSON, encoding="utf-8") as fh:
        snapshot = json.load(fh)
    members: dict[str, int] = snapshot["members"]

    matrix: dict[str, dict] = {}
    for member in sorted(members):
        size = members[member]
        lane = lane_of(member)
        configs = {m["key"]: legality(m["key"], lane) for m in METHODS}
        matrix[member] = {
            "circuit_size": size,
            "lane": lane,
            "configs": configs,
        }

    legal_configs = sum(
        1 for row in matrix.values() for c in row["configs"].values() if c["legal"]
    )
    illegal_configs = sum(
        1 for row in matrix.values() for c in row["configs"].values() if not c["legal"]
    )

    # A small COMPARISON surface (#769): the headline value-lane reduction vs the
    # blake3-token string lane, per datatype class, with BOTH numbers JOINED from
    # the snapshot (never hand-typed). Only pairs whose BOTH members exist in the
    # snapshot are emitted, so this can never fabricate a comparison.
    string_lane_ref = {
        "integer": "filter_int_d2",
        "double": "filter_f64_d2",
    }
    value_lane_ref = {
        "integer": "filter_value_dl_int",
        "double": "filter_value_dl_f64",
        "decimal": "filter_value_dl_decimal",
        # [OPUS-5] sq-wz99x: the dateTime/date lane. Like decimal it has NO
        # blake3-token string-lane counterpart (there is no filter_datetime_d*
        # member), so the entry carries the value-lane cost alone — no ratio is
        # emitted, and none is fabricated.
        "datetime": "filter_value_dl_datetime",
    }
    comparison = {}
    for datatype, value_member in value_lane_ref.items():
        string_member = string_lane_ref.get(datatype)
        if value_member not in members:
            continue
        entry = {
            "value_lane_member": value_member,
            "value_lane_circuit_size": members[value_member],
            "value_lane_method": "dual-leaf",
        }
        if string_member and string_member in members:
            entry["string_lane_member"] = string_member
            entry["string_lane_circuit_size"] = members[string_member]
            entry["string_lane_method"] = "string-canonical"
            entry["gate_reduction_x"] = round(
                members[string_member] / members[value_member], 2
            )
        comparison[datatype] = entry

    out = {
        "_comment": "[OPUS-4.8] sq-ot3x per-config bb-gates benchmark matrix "
        "(commitment method x circuit). GENERATED by "
        "bench/zk-compose/scripts/bb_gates_matrix.py; gated by "
        "crates/sparq-zk-compose/tests/bb_gates_matrix.rs. circuit_size is JOINED "
        "from the regression-gated snapshot "
        "(crates/sparq-zk-compose/tests/gate_count_snapshot.json); legality mirrors "
        "the fail-closed dispatch rule (crates/sparq-zk-compose/src/dispatch.rs, "
        "sq-cfmv). Re-run gate_counts.sh + this generator after an intentional "
        "circuit change. Do NOT hand-edit.",
        "_honesty": "NON-CANONICAL bb-gates measured on the work-box — a comparison "
        "tool, not a canonical performance number. The whole ZK estate is NOT "
        "externally audited (sq-qhy4). dual-leaf / value-only carry the documented "
        "INV-VL downgrade (value<->lexical agreement is trusted-issuer-honesty, "
        "CR-G8 / #769). value-only is a research dial, never production. This "
        "matrix asserts NO soundness or privacy property — only structural "
        "(method, circuit) legality and per-circuit gate cost.",
        "tool": "bb gates -s ultra_honk",
        "source_snapshot": "crates/sparq-zk-compose/tests/gate_count_snapshot.json",
        "dispatch_source": "crates/sparq-zk-compose/src/dispatch.rs",
        "non_canonical": True,
        "methods": METHODS,
        "value_lane_prefix": VALUE_LANE_PREFIX,
        "matrix": matrix,
        "comparison": comparison,
        "summary": {
            "circuits": len(matrix),
            "methods": len(METHODS),
            "configs": len(matrix) * len(METHODS),
            "legal_configs": legal_configs,
            "illegal_configs": illegal_configs,
        },
    }

    json.dump(out, sys.stdout, indent=2, sort_keys=False)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
