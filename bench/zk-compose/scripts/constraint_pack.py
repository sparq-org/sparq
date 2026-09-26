#!/usr/bin/env python3
# [OPUS-5] sq-gum8.5 (parent sq-gum8, design record research/paper-selection.md §3.1 + §5-P1).
#
# THE CONSTRAINT-COUNT EVALUATION PACK — the reproducible, submission-support evaluation
# artefact for the zkSPARQL ISWC 2026 submission (zksparql.org) and for the spec draft
# site/specs/zksparql.typ §16.2.
#
# WHAT IT IS. A deterministic, machine-independent view of the circuit-family's CONSTRAINT
# COUNTS, organised the way a reviewer actually asks about them:
#
#   1. per FAMILY (scan / join / bounded path / the four FILTER lanes / the credential-layer
#      members), with each member's family PARAMETERS parsed out of its name;
#   2. per-parameter SCALING — every pair of members inside a family that differs in EXACTLY
#      ONE parameter, with the gate delta and ratio, so "how does the circuit grow in k / n /
#      r / d / na / nb" is answered by data rather than by prose;
#   3. INVARIANCES — parameter axes along which the count provably does not move in the
#      committed snapshot (the blake3-token FILTER lanes are the load-bearing case: the
#      canonical token fits one 64-byte blake3 block for every supported digit count, so `d`
#      does not move the circuit at all);
#   4. the string-lane vs value-lane comparison, both sides JOINED from the snapshot;
#   5. a RELATED-WORK block that records, per cited system, that NO figure of it is
#      transcribed here — and why a cross-system constraint ratio would not be a measurement.
#
# WHERE THE NUMBERS COME FROM. Every integer is JOINED from the regression-gated snapshot
# crates/sparq-zk-compose/tests/gate_count_snapshot.json — the same single source of truth
# that gate_counts.sh re-baselines and that sparql_catalog.py and bb_gates_matrix.py join.
# NOTHING is hand-typed here. The generator needs NO nargo and NO bb, so it is deterministic
# and re-runs byte-identically (that is the acceptance criterion of the bead), and it can
# never fabricate a gate count: if a member is missing from the snapshot it simply is not in
# the pack, and if the snapshot grows a member this script does not recognise it FAILS
# (see classify(), below) rather than silently dropping it.
#
# HONESTY — load-bearing, and the reason this file is long-winded about it:
#
#   * A gate count is the SIZE of a compiled circuit under the pinned toolchain. It is NOT a
#     running time. This pack reports NO wall-clock number, by design: prove/verify timings
#     are machine-dependent and the repo's canonical-measurement rule puts them behind the
#     canonical EC2 runner, not a work box. bench/zk-compose/family_cost_curve.json holds the
#     (non-canonical, work-box) timing curve separately; it is deliberately NOT joined here.
#   * A gate count says NOTHING about whether a circuit proves the right statement. Coverage
#     is site/specs/zksparql.typ §7.1 + sparql_feature_catalog.json, never a circuit size. In
#     particular the `path_reach` family proves a BOUNDED-EXISTENCE statement (there exists a
#     chain of <= D committed triples, D public) — strictly weaker than the SPARQL property
#     path — and no gate number changes that.
#   * The whole ZK estate is internally re-audited but NOT externally audited (open gate
#     sq-qhy4); the forge suite is toolchain-gated (sq-1gir); the value lane carries the
#     documented INV-VL / CR-G8 downgrade (value<->lexical agreement is trusted-issuer
#     honesty, not machine-enforced). This pack asserts NO soundness, privacy, or
#     zero-knowledge property whatsoever — it counts gates.
#   * NO third-party figure is reproduced. Constraint counts are incomparable across proof
#     systems, arithmetizations, and circuit granularities; a ratio between this family and a
#     differently-arithmetized published system would not measure anything. Related work is
#     CITED, never re-measured (the related_work block records exactly this, per system).
#
# Usage:
#   bench/zk-compose/scripts/constraint_pack.py                  # JSON to stdout
#   bench/zk-compose/scripts/constraint_pack.py --format markdown # the reviewer-facing table
#   bench/zk-compose/scripts/constraint_pack.py --check           # committed copies current?
#   bench/zk-compose/scripts/constraint_pack.py --write           # regenerate both artefacts
#
# `--check` is the reproducibility gate: it fails unless BOTH committed artefacts are
# byte-identical to a fresh generation AND every number in the committed JSON still equals
# the snapshot's. Run it after any intentional circuit change, together with gate_counts.sh.

from __future__ import annotations

import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
PACK_DIR = os.path.normpath(os.path.join(HERE, ".."))
REPO_ROOT = os.path.normpath(os.path.join(PACK_DIR, "..", ".."))

SNAPSHOT_JSON = os.path.join(
    REPO_ROOT, "crates", "sparq-zk-compose", "tests", "gate_count_snapshot.json"
)
GATE_COUNTS_JSON = os.path.join(PACK_DIR, "gate_counts_latest.json")
PACK_JSON = os.path.join(PACK_DIR, "constraint_counts.json")
PACK_MD = os.path.join(PACK_DIR, "CONSTRAINT_COUNTS.md")

# Repo-relative names, for the artefacts' own provenance fields (never absolute paths, which
# would differ per machine and break byte-identity).
REL_SNAPSHOT = "crates/sparq-zk-compose/tests/gate_count_snapshot.json"
REL_GATE_COUNTS = "bench/zk-compose/gate_counts_latest.json"
REL_GENERATOR = "bench/zk-compose/scripts/constraint_pack.py"

METRIC = "circuit_size"
TOOL = "bb gates -s ultra_honk"

# ---------------------------------------------------------------------------------------
# Family classification.
#
# Each entry: (family key, compiled name regex, ordered parameter names, layer, role).
# The regexes are FULL-MATCH and mutually exclusive, and classify() FAILS on any snapshot
# member none of them matches — so adding a circuit family to the estate without describing
# it here is a hard error, never a silent omission from the evaluation.
#
# `layer` separates the proof contracts represented by the compiled members:
#   query      — the SPARQL-algebra members (section 7.3 of the spec draft)
#   credential — the credential-layer members (possession, revocation, issuer attestation)
#   result     — integrated support for released mappings, without result completeness
# ---------------------------------------------------------------------------------------
FAMILIES: list[dict] = [
    # [GPT-6] Keep the selected-result contract separate from complete-scan members.
    {
        "key": "result_v1",
        "pattern": r"result_v1_k(?P<k>\d+)_n(?P<n>\d+)_p(?P<p>\d+)_r(?P<r>\d+)_f(?P<f>\d+)",
        "params": ["k", "n", "p", "r", "f"],
        "layer": "result",
        "role": "Support for released SELECT DISTINCT mappings: issuer authentication, "
        "credential status, selected triple membership, BGP joins and residual private "
        "integer FILTERs. Capacity parameters: k credentials, n triples per credential, "
        "p patterns, r released rows, f private FILTERs per row. Does not establish "
        "result completeness or holder identity.",
    },
    {
        "key": "scan",
        "pattern": r"scan_k(?P<k>\d+)_n(?P<n>\d+)_r(?P<r>\d+)",
        "params": ["k", "n", "r"],
        "layer": "query",
        "role": "BGP row membership + per-scan completeness over k committed graphs, "
        "n slot bucket, r disclosed-row bucket.",
    },
    {
        "key": "join_eq",
        "pattern": r"join_eq_na(?P<na>\d+)_nb(?P<nb>\d+)",
        "params": ["na", "nb"],
        "layer": "query",
        "role": "Hidden equality join between two committed sides of bucket sizes na, nb.",
    },
    {
        "key": "path_reach",
        "pattern": r"path_reach_d(?P<d>\d+)_k(?P<k>\d+)_n(?P<n>\d+)",
        "params": ["d", "k", "n"],
        "layer": "query",
        "role": "BOUNDED-EXISTENCE property path: there exists a chain of at most d "
        "committed triples (d is a PUBLIC input). Strictly weaker than the SPARQL path "
        "operator — never completeness, never a 'no path exists' claim.",
    },
    {
        "key": "filter_int",
        "pattern": r"filter_int_d(?P<d>\d+)",
        "params": ["d"],
        "layer": "query",
        "role": "xsd:integer FILTER, string lane: operand bound via the canonical "
        "decimal-digit token under a blake3 blackbox (d = digit count).",
    },
    {
        "key": "filter_signed_int",
        "pattern": r"filter_signed_int_d(?P<d>\d+)",
        "params": ["d"],
        "layer": "query",
        "role": "Signed-integer FILTER, string lane (sign handled above the token).",
    },
    {
        "key": "filter_decimal",
        "pattern": r"filter_decimal_i(?P<i>\d+)_f(?P<f>\d+)",
        "params": ["i", "f"],
        "layer": "query",
        "role": "xsd:decimal FILTER, string lane (i integer digits, f fraction digits).",
    },
    {
        "key": "filter_f64_composable",
        "pattern": r"filter_f64_d(?P<d>\d+)",
        "params": ["d"],
        "layer": "query",
        "role": "xsd:double FILTER, string lane: manifest-composable member binding the "
        "operand via the same canonical decimal-digit token as filter_int.",
    },
    {
        "key": "filter_f64_raw",
        "pattern": r"filter_f64",
        "params": [],
        "layer": "query",
        "role": "The non-composable xsd:double building block: a bare IEEE-754 comparison "
        "with no token binding. Kept for reference; it is NOT manifest-composable, so its "
        "count is not comparable with the composable members above.",
    },
    {
        "key": "filter_value_dl",
        "pattern": r"filter_value_dl_(?P<datatype>[a-z0-9]+)",
        "params": ["datatype"],
        "layer": "query",
        "role": "VALUE lane: FILTER against the committed value handle of the dual-leaf "
        "commitment method — no in-circuit token hashing. Carries the documented INV-VL / "
        "CR-G8 downgrade and requires a method that committed a value handle.",
    },
    {
        "key": "holder_pok",
        "pattern": r"holder_pok",
        "params": [],
        "layer": "credential",
        "role": "Holder proof of possession.",
    },
    {
        "key": "holder_set",
        "pattern": r"holder_set_d(?P<d>\d+)",
        "params": ["d"],
        "layer": "credential",
        "role": "Holder membership in a registry set of depth d.",
    },
    {
        "key": "revoke_unset",
        "pattern": r"revoke_unset_d(?P<d>\d+)",
        "params": ["d"],
        "layer": "credential",
        "role": "Revocation non-membership against a depth-d status structure.",
    },
    {
        "key": "revoke_hidden_ref",
        "pattern": r"revoke_hidden_ref_d(?P<d>\d+)_a(?P<a>\d+)",
        "params": ["d", "a"],
        "layer": "credential",
        "role": "Revocation non-membership with a hidden status-list reference "
        "(depth d, a authority slots).",
    },
    {
        "key": "hidden_issuer",
        "pattern": r"hidden_issuer_d(?P<d>\d+)",
        "params": ["d"],
        "layer": "credential",
        "role": "Issuer attestation against a hidden issuer drawn from a depth-d trusted set.",
    },
]

_COMPILED = [(f, re.compile(f["pattern"] + r"\Z")) for f in FAMILIES]

# The string-lane / value-lane pairs the #769 comparison is about. Emitted ONLY when BOTH
# members exist in the snapshot, so the comparison can never be half-fabricated.
LANE_PAIRS = [
    ("integer", "filter_int_d2", "filter_value_dl_int"),
    ("double", "filter_f64_d2", "filter_value_dl_f64"),
    ("decimal", "filter_decimal_i3_f2", "filter_value_dl_decimal"),
]

# The related-work block. NO figure of any of these systems is transcribed — the point of the
# block is to record that refusal, per system, with the reason. `metric_reported` describes
# what the cited paper counts, which is exactly why the counts are not comparable.
RELATED_WORK = [
    {
        "system": "ZKSQL",
        "citation": "Li, Weng, Xu, Wang, Rogers. PVLDB 16(8), 2023.",
        "domain": "SQL over a committed database",
        "proof_system": "interactive, VOLE-based",
        "metric_reported": "communication and running time of an interactive protocol",
        "figures_transcribed": False,
    },
    {
        "system": "PoneglyphDB",
        "citation": "Gu, Fang, Nawab. ACM SIGMOD / PACMMOD, 2025. arXiv:2411.15031.",
        "domain": "SQL over a committed database",
        "proof_system": "non-interactive, PLONKish",
        "metric_reported": "prover/verifier time for whole SQL workloads",
        "figures_transcribed": False,
    },
    {
        "system": "ZKGraph",
        "citation": "arXiv:2507.00427, July 2025.",
        "domain": "property-graph queries (no RDF/SPARQL surface)",
        "proof_system": "non-interactive, PLONKish",
        "metric_reported": "proving cost for graph-query workloads",
        "figures_transcribed": False,
    },
    {
        "system": "VeriDKG",
        "citation": "Zhou et al. PVLDB 17(5), 2024.",
        "domain": "SPARQL over decentralised knowledge graphs",
        "proof_system": "authenticated data structure — integrity, NOT hiding",
        "metric_reported": "proof size and verification time of an ADS, not circuit gates",
        "figures_transcribed": False,
    },
    {
        "system": "ZKLP",
        "citation": "Ernstberger et al. IEEE S&P 2025.",
        "domain": "IEEE-754 floating-point arithmetic in zero knowledge",
        "proof_system": "lookup-optimised circuits",
        "metric_reported": "constraint counts for individual IEEE-754 primitive operations",
        "figures_transcribed": False,
    },
]

WHY_NOT_TRANSCRIBED = (
    "This pack reproduces NO figure from any cited system. Constraint counts are not "
    "comparable across proof systems, arithmetizations, or circuit granularities: the "
    "numbers below are Barretenberg ultra_honk `circuit_size` for WHOLE SPARQL-fragment "
    "and credential-layer circuits, whereas the cited works count different objects under "
    "different backends (and several report time, not constraints). A ratio between them "
    "would not be a measurement of anything, so none is computed. Related work is cited — "
    "see site/specs/zksparql.typ section 3 — never re-measured."
)

HONESTY = (
    "GATE COUNTS ONLY. A circuit_size is the SIZE of a compiled circuit under the pinned "
    "toolchain, not a running time; this pack reports NO wall-clock figure, because timings "
    "are machine-dependent and only canonical runs may carry them. A gate count also says "
    "nothing about WHAT a circuit proves: coverage lives in site/specs/zksparql.typ section "
    "7.1 and in sparql_feature_catalog.json, and the path_reach family proves a strictly "
    "weaker BOUNDED-EXISTENCE statement regardless of its size. The ZK estate is internally "
    "re-audited but NOT externally audited (open gate sq-qhy4); the forge suite is "
    "toolchain-gated (sq-1gir); the value lane carries the documented INV-VL / CR-G8 "
    "downgrade (value<->lexical agreement is trusted-issuer honesty, not machine-enforced). "
    "NO soundness, privacy, or zero-knowledge property is asserted here."
)


# ---------------------------------------------------------------------------------------
# Classification + derivation
# ---------------------------------------------------------------------------------------
def classify(member: str) -> tuple[dict, dict]:
    """(family, parsed params) for a snapshot member, or raise on an unknown shape.

    Failing loudly is the anti-drift property: a new circuit family cannot enter the estate
    and silently vanish from the evaluation pack.
    """
    for family, rx in _COMPILED:
        m = rx.match(member)
        if m:
            raw = m.groupdict()
            params = {
                p: (int(raw[p]) if raw[p].isdigit() else raw[p]) for p in family["params"]
            }
            return family, params
    raise KeyError(
        f"member {member!r} matches no family in FAMILIES — describe it in "
        f"{REL_GENERATOR} (layer, role, parameters) before re-running the pack"
    )


def _sort_key(row: dict, family: dict) -> tuple:
    """Deterministic member order: by parameter values (numeric first), then by name."""
    vals = []
    for p in family["params"]:
        v = row["params"][p]
        vals.append((0, v, "") if isinstance(v, int) else (1, 0, v))
    return (tuple(vals), row["member"])


def build_families(members: dict[str, int]) -> dict:
    grouped: dict[str, list[dict]] = {f["key"]: [] for f in FAMILIES}
    for member in sorted(members):
        family, params = classify(member)
        grouped[family["key"]].append(
            {"member": member, "params": params, METRIC: members[member]}
        )

    out: dict[str, dict] = {}
    for family in FAMILIES:
        rows = grouped[family["key"]]
        if not rows:
            # A described family with no compiled member in the snapshot is simply absent —
            # emitting an empty row would invite a reader to infer a zero cost.
            continue
        rows.sort(key=lambda r: _sort_key(r, family))
        sizes = [r[METRIC] for r in rows]
        out[family["key"]] = {
            "layer": family["layer"],
            "role": family["role"],
            "parameters": family["params"],
            "members": rows,
            "min_circuit_size": min(sizes),
            "max_circuit_size": max(sizes),
            "scaling": scaling_pairs(rows, family),
            "invariances": invariances(rows, family),
        }
    return out


def _is_ordinal(rows: list[dict], param: str) -> bool:
    """A parameter is swept only if every compiled member gives it a numeric value."""
    return all(isinstance(r["params"][param], int) for r in rows)


def scaling_pairs(rows: list[dict], family: dict) -> list[dict]:
    """Every ordered pair inside the family differing in EXACTLY ONE ORDINAL parameter.

    This is the honest form of a scaling claim: it does not fit a curve or extrapolate, it
    reports the measured pairs the compiled family actually contains. Only NUMERIC family
    parameters are swept — a categorical parameter (the value lane's `datatype`) has no
    ordering, so a "delta" along it would describe two unrelated circuits, not a trend.
    """
    ordinal = [p for p in family["params"] if _is_ordinal(rows, p)]
    pairs: list[dict] = []
    for i, a in enumerate(rows):
        for b in rows[i + 1 :]:
            differing = [
                p for p in family["params"] if a["params"][p] != b["params"][p]
            ]
            if len(differing) != 1 or differing[0] not in ordinal:
                continue
            p = differing[0]
            lo, hi = (a, b)
            if isinstance(lo["params"][p], int) and lo["params"][p] > hi["params"][p]:
                lo, hi = hi, lo
            pairs.append(
                {
                    "parameter": p,
                    "from_member": lo["member"],
                    "from_value": lo["params"][p],
                    "from_circuit_size": lo[METRIC],
                    "to_member": hi["member"],
                    "to_value": hi["params"][p],
                    "to_circuit_size": hi[METRIC],
                    "delta_gates": hi[METRIC] - lo[METRIC],
                    "ratio": round(hi[METRIC] / lo[METRIC], 4),
                    "held_fixed": {
                        q: lo["params"][q] for q in family["params"] if q != p
                    },
                }
            )
    pairs.sort(key=lambda d: (d["parameter"], d["from_member"], d["to_member"]))
    return pairs


def invariances(rows: list[dict], family: dict) -> list[dict]:
    """Parameter axes along which the count does NOT move, over >= 2 compiled members.

    Reported as a fact about the committed snapshot only — not as a general theorem about
    the construction. The blake3-token FILTER lanes are the interesting case.
    """
    facts: list[dict] = []
    for p in family["params"]:
        if not _is_ordinal(rows, p):
            continue
        buckets: dict[tuple, list[dict]] = {}
        for r in rows:
            key = tuple(str(r["params"][q]) for q in family["params"] if q != p)
            buckets.setdefault(key, []).append(r)
        for key in sorted(buckets):
            group = buckets[key]
            if len(group) < 2:
                continue
            sizes = {r[METRIC] for r in group}
            if len(sizes) != 1:
                continue
            facts.append(
                {
                    "parameter": p,
                    "constant_circuit_size": sizes.pop(),
                    "over_values": [r["params"][p] for r in group],
                    "members": [r["member"] for r in group],
                    "held_fixed": {
                        q: group[0]["params"][q] for q in family["params"] if q != p
                    },
                }
            )
    facts.sort(key=lambda d: (d["parameter"], d["members"][0]))
    return facts


def lane_comparison(members: dict[str, int]) -> dict:
    out: dict[str, dict] = {}
    for datatype, string_member, value_member in LANE_PAIRS:
        if string_member not in members or value_member not in members:
            continue
        out[datatype] = {
            "string_lane_member": string_member,
            "string_lane_circuit_size": members[string_member],
            "value_lane_member": value_member,
            "value_lane_circuit_size": members[value_member],
            "gate_reduction_x": round(
                members[string_member] / members[value_member], 4
            ),
            "caveat": "The value lane requires a commitment method that committed a value "
            "handle (dual-leaf), whose host-leaf encoding is NOT yet implemented and which "
            "carries the INV-VL / CR-G8 downgrade. This is a circuit-size comparison of two "
            "designs, not a claim that the value lane is available end-to-end today.",
        }
    return out


def build_pack(snapshot: dict, gate_counts: dict) -> dict:
    members: dict[str, int] = snapshot["members"]
    families = build_families(members)

    by_layer: dict[str, dict] = {}
    for key, fam in families.items():
        bucket = by_layer.setdefault(fam["layer"], {"families": [], "members": 0})
        bucket["families"].append(key)
        bucket["members"] += len(fam["members"])

    return {
        "_comment": "[OPUS-5] sq-gum8.5 constraint-count evaluation pack for the zkSPARQL "
        "submission. GENERATED by " + REL_GENERATOR + " — do NOT hand-edit. Every "
        "circuit_size is JOINED from " + REL_SNAPSHOT + " (the regression-gated source of "
        "truth); no number is written by this script. Regenerate with `--write`, verify "
        "with `--check` (byte-identical re-run is the bead's acceptance criterion).",
        "_honesty": HONESTY,
        "metric": {
            "name": METRIC,
            "tool": TOOL,
            "meaning": "Compiled circuit size (gate count) under the pinned proving "
            "toolchain. A SIZE, not a time.",
            "is_wall_clock": False,
            "is_canonical_performance_number": False,
        },
        "provenance": {
            "source_snapshot": REL_SNAPSHOT,
            "gate_count_snapshot_companion": REL_GATE_COUNTS,
            "generator": REL_GENERATOR,
            "requires_nargo_or_bb": False,
            "deterministic": True,
        },
        "toolchain": {
            "bb": snapshot.get("bb_version_baselined"),
            "nargo": snapshot.get("nargo_version_baselined"),
            "regression_tolerance_pct": snapshot.get("tolerance_pct"),
            "bb_at_last_measurement": gate_counts.get("bb_version"),
            "nargo_at_last_measurement": gate_counts.get("nargo_version"),
        },
        "layers": by_layer,
        "families": families,
        "lane_comparison": lane_comparison(members),
        "related_work": {
            "why_no_figures_transcribed": WHY_NOT_TRANSCRIBED,
            "systems": RELATED_WORK,
        },
        "summary": {
            "families": len(families),
            "members": sum(len(f["members"]) for f in families.values()),
            "snapshot_members": len(members),
            "scaling_pairs": sum(len(f["scaling"]) for f in families.values()),
            "invariance_facts": sum(len(f["invariances"]) for f in families.values()),
        },
    }


# ---------------------------------------------------------------------------------------
# Rendering
# ---------------------------------------------------------------------------------------
def render_json(pack: dict) -> str:
    return json.dumps(pack, indent=2) + "\n"


def _md_params(params: dict) -> str:
    return ", ".join(f"{k}={v}" for k, v in params.items()) if params else "—"


def render_markdown(pack: dict) -> str:
    L: list[str] = []
    add = L.append
    add("# zkSPARQL constraint-count evaluation pack")
    add("")
    add(
        f"<!-- GENERATED by `{REL_GENERATOR}` — do NOT hand-edit. "
        "Regenerate with `--write`; verify with `--check`. -->"
    )
    add("")
    add(
        "Deterministic circuit-size evidence for the zkSPARQL submission "
        "(bead `sq-gum8.5`; design record `research/paper-selection.md` §3.1)."
    )
    add("")
    add("## What this is, and what it is not")
    add("")
    add(
        f"Every number below is the `{METRIC}` reported by `{TOOL}`, **joined** from the "
        f"regression-gated snapshot `{REL_SNAPSHOT}`. Nothing is hand-typed, the generator "
        "needs neither `nargo` nor `bb`, and a re-run is byte-identical."
    )
    add("")
    add(
        "- **A size, not a time.** No wall-clock figure appears here. Prove/verify timings "
        "are machine-dependent; only canonical runs may carry them."
    )
    add(
        "- **A size, not a coverage claim.** What each circuit *proves* is "
        "`site/specs/zksparql.typ` §7.1 and `sparql_feature_catalog.json`. The `path_reach` "
        "family proves a **bounded-existence** statement — there exists a chain of at most "
        "`d` committed triples, `d` public — which is strictly weaker than the SPARQL "
        "property path, whatever its gate count."
    )
    add(
        "- **A size, not a security claim.** The estate is internally re-audited but **not "
        "externally audited** (open gate `sq-qhy4`); the forge suite is toolchain-gated "
        "(`sq-1gir`); the value lane carries the documented INV-VL / CR-G8 downgrade. "
        "Nothing here asserts a soundness, privacy, or zero-knowledge property."
    )
    add(
        "- **No third-party figure is reproduced.** See "
        "[Related work](#related-work-cited-never-re-measured)."
    )
    add("")
    tc = pack["toolchain"]
    add(
        f"Toolchain baselined for these counts: `bb {tc['bb']}`, `nargo {tc['nargo']}` "
        f"(regression tolerance {tc['regression_tolerance_pct']}%)."
    )
    add("")

    for layer_name, layer_title in (
        ("result", "Successful-result circuits (released mapping support)"),
        ("query", "Query-layer circuits (SPARQL algebra fragment)"),
        ("credential", "Credential-layer circuits"),
    ):
        add(f"## {layer_title}")
        add("")
        for key, fam in pack["families"].items():
            if fam["layer"] != layer_name:
                continue
            add(f"### `{key}`")
            add("")
            add(fam["role"])
            add("")
            add(f"| member | parameters | `{METRIC}` |")
            add("| --- | --- | --- |")
            for row in fam["members"]:
                add(
                    f"| `{row['member']}` | {_md_params(row['params'])} "
                    f"| {row[METRIC]:,} |"
                )
            add("")
            if fam["scaling"]:
                add("Scaling — pairs differing in exactly one parameter:")
                add("")
                add("| parameter | from | to | held fixed | Δ gates | ratio |")
                add("| --- | --- | --- | --- | --- | --- |")
                for s in fam["scaling"]:
                    held = _md_params(s["held_fixed"])
                    add(
                        f"| `{s['parameter']}` "
                        f"| {s['from_value']} ({s['from_circuit_size']:,}) "
                        f"| {s['to_value']} ({s['to_circuit_size']:,}) "
                        f"| {held} | {s['delta_gates']:+,} | {s['ratio']:.4f} |"
                    )
                add("")
            if fam["invariances"]:
                add("Invariances observed in this snapshot:")
                add("")
                for inv in fam["invariances"]:
                    vals = ", ".join(str(v) for v in inv["over_values"])
                    held = _md_params(inv["held_fixed"])
                    add(
                        f"- `{inv['parameter']}` ∈ {{{vals}}} (held fixed: {held}) — "
                        f"constant at {inv['constant_circuit_size']:,} gates "
                        f"({', '.join('`' + m + '`' for m in inv['members'])})."
                    )
                add("")

    if pack["lane_comparison"]:
        add("## String lane vs value lane")
        add("")
        add(
            "The value lane binds the FILTER operand through the committed value handle of "
            "the dual-leaf commitment method instead of hashing a canonical token in "
            "circuit. Both sides are joined from the snapshot."
        )
        add("")
        add("| datatype | string lane | gates | value lane | gates | reduction |")
        add("| --- | --- | --- | --- | --- | --- |")
        for datatype, row in pack["lane_comparison"].items():
            add(
                f"| {datatype} | `{row['string_lane_member']}` "
                f"| {row['string_lane_circuit_size']:,} "
                f"| `{row['value_lane_member']}` "
                f"| {row['value_lane_circuit_size']:,} "
                f"| {row['gate_reduction_x']:.4f}× |"
            )
        add("")
        add(
            "> The dual-leaf host-leaf encoding is **not yet implemented** and the resolver "
            "is not wired into `verify_manifest`. This is a circuit-size comparison of two "
            "designs, not a claim that the value lane is usable end to end today."
        )
        add("")

    add("## Related work — cited, never re-measured")
    add("")
    add(pack["related_work"]["why_no_figures_transcribed"])
    add("")
    add("| system | domain | proof system | what it reports | figures reproduced here |")
    add("| --- | --- | --- | --- | --- |")
    for s in pack["related_work"]["systems"]:
        add(
            f"| {s['system']} | {s['domain']} | {s['proof_system']} "
            f"| {s['metric_reported']} | "
            f"{'yes' if s['figures_transcribed'] else '**no**'} |"
        )
    add("")
    add("Citations: `site/specs/zksparql.typ` §3 (References).")
    add("")

    add("## Reproduce")
    add("")
    add("```sh")
    add("# 1. (only after an intentional circuit change) re-measure the gate counts.")
    add("#    Needs nargo + bb; re-baseline the snapshot from the result.")
    add(
        "bench/zk-compose/scripts/gate_counts.sh > "
        "bench/zk-compose/gate_counts_latest.json"
    )
    add("")
    add("# 2. regenerate this pack from the snapshot (no nargo, no bb, deterministic).")
    add(f"{REL_GENERATOR} --write")
    add("")
    add("# 3. verify the committed artefacts are byte-identical to a fresh generation.")
    add(f"{REL_GENERATOR} --check")
    add("```")
    add("")
    s = pack["summary"]
    add(
        f"Coverage: {s['members']} of {s['snapshot_members']} snapshot members across "
        f"{s['families']} families, {s['scaling_pairs']} single-parameter scaling pairs, "
        f"{s['invariance_facts']} invariance facts. The generator **fails** on a snapshot "
        "member it cannot classify, so a new circuit family cannot silently drop out of "
        "this evaluation."
    )
    add("")
    return "\n".join(L)


# ---------------------------------------------------------------------------------------
# Entry points
# ---------------------------------------------------------------------------------------
def load(path: str) -> dict:
    with open(path, encoding="utf-8") as fh:
        return json.load(fh)


def self_consistency_errors(pack: dict, snapshot: dict) -> list[str]:
    """Re-check the COMMITTED pack against the snapshot.

    Deliberately independent of build_pack's internals: `--check` runs against the file on
    disk, where a hand edit could have introduced a number the generator would never emit.
    """
    members: dict[str, int] = snapshot["members"]
    errors: list[str] = []
    seen: set[str] = set()

    for key, fam in pack["families"].items():
        for row in fam["members"]:
            name = row["member"]
            seen.add(name)
            if name not in members:
                errors.append(f"{key}: member {name} is not in the snapshot")
            elif row[METRIC] != members[name]:
                errors.append(
                    f"{key}: {name} {METRIC}={row[METRIC]} != snapshot {members[name]}"
                )
        sizes = [r[METRIC] for r in fam["members"]]
        if sizes and (fam["min_circuit_size"] != min(sizes) or fam["max_circuit_size"] != max(sizes)):
            errors.append(f"{key}: min/max disagree with its own member rows")
        for s in fam["scaling"]:
            for m_key, size_key in (
                ("from_member", "from_circuit_size"),
                ("to_member", "to_circuit_size"),
            ):
                if members.get(s[m_key]) != s[size_key]:
                    errors.append(
                        f"{key}: scaling {s[m_key]} {size_key}={s[size_key]} != "
                        f"snapshot {members.get(s[m_key])}"
                    )
            if s["delta_gates"] != s["to_circuit_size"] - s["from_circuit_size"]:
                errors.append(f"{key}: scaling {s['from_member']}->{s['to_member']} bad delta")
        for inv in fam["invariances"]:
            for m in inv["members"]:
                if members.get(m) != inv["constant_circuit_size"]:
                    errors.append(
                        f"{key}: invariance member {m} is not "
                        f"{inv['constant_circuit_size']} in the snapshot"
                    )

    missing = sorted(set(members) - seen)
    if missing:
        errors.append(f"snapshot members absent from the pack: {missing}")

    for row in pack["lane_comparison"].values():
        for m_key, size_key in (
            ("string_lane_member", "string_lane_circuit_size"),
            ("value_lane_member", "value_lane_circuit_size"),
        ):
            if members.get(row[m_key]) != row[size_key]:
                errors.append(f"lane_comparison: {row[m_key]} disagrees with the snapshot")

    for s in pack["related_work"]["systems"]:
        if s["figures_transcribed"]:
            errors.append(
                f"related_work: {s['system']} claims transcribed figures — this pack "
                "reproduces no third-party number"
            )

    if pack["metric"]["is_wall_clock"] or pack["metric"]["is_canonical_performance_number"]:
        errors.append("metric: this pack is gate counts only — never wall-clock, never canonical")

    return errors


def check() -> int:
    snapshot = load(SNAPSHOT_JSON)
    pack = build_pack(snapshot, load(GATE_COUNTS_JSON))
    errors: list[str] = []

    for path, want in ((PACK_JSON, render_json(pack)), (PACK_MD, render_markdown(pack))):
        rel = os.path.relpath(path, REPO_ROOT)
        try:
            with open(path, encoding="utf-8") as fh:
                got = fh.read()
        except FileNotFoundError:
            errors.append(f"{rel} is missing — regenerate with `{REL_GENERATOR} --write`")
            continue
        if got != want:
            errors.append(
                f"{rel} is NOT what the generator produces — regenerate with "
                f"`{REL_GENERATOR} --write`"
            )

    try:
        errors.extend(self_consistency_errors(load(PACK_JSON), snapshot))
    except FileNotFoundError:
        pass
    except json.JSONDecodeError as exc:
        errors.append(f"committed pack does not parse: {exc}")

    if errors:
        print("constraint_pack --check FAILED:", file=sys.stderr)
        for e in errors:
            print(f"  - {e}", file=sys.stderr)
        return 1
    print(
        "constraint_pack --check OK: both artefacts are byte-identical to a fresh "
        "generation and every number still agrees with the snapshot."
    )
    return 0


def write() -> int:
    pack = build_pack(load(SNAPSHOT_JSON), load(GATE_COUNTS_JSON))
    for path, text in ((PACK_JSON, render_json(pack)), (PACK_MD, render_markdown(pack))):
        with open(path, "w", encoding="utf-8") as fh:
            fh.write(text)
        print(f"wrote {os.path.relpath(path, REPO_ROOT)}", file=sys.stderr)
    return 0


def main(argv: list[str]) -> int:
    args = argv[1:]
    if args == ["--check"]:
        return check()
    if args == ["--write"]:
        return write()
    if args in ([], ["--format", "json"]):
        sys.stdout.write(render_json(build_pack(load(SNAPSHOT_JSON), load(GATE_COUNTS_JSON))))
        return 0
    if args == ["--format", "markdown"]:
        sys.stdout.write(render_markdown(build_pack(load(SNAPSHOT_JSON), load(GATE_COUNTS_JSON))))
        return 0
    print(f"usage: {argv[0]} [--format json|markdown | --write | --check]", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
