#!/usr/bin/env python3
# Generate bench/dashboard/metric-labels.json — a map from raw benchmark metric
# STEM (the github-action-benchmark `name` minus its mode/unit suffix) to a readable
# {label, suite, dataset, query, unit} record, so the dashboard can render
# "WatDiv S1 — star query, count" instead of the cryptic stem "watdiv_S1_count_us".
#
# Implements
# bead sq-ocuf (parent design sq-i0nm, plan A): labels derived from the SAME ground truth the
# CI harness uses — the per-suite *.rq files (filenames + header comments) and expected-rows
# tables — NOT invented. The naming convention is fixed by scripts/ci-bench.sh:
#
#   load_s, rdfs_infer_s, parse_ns_per_byte, store_bytes_per_triple[_small],
#   dict_bytes_per_term, wasm_bundle_bytes                 -> pipeline / memory metrics
#   <q-stem>_<mode>_us       (mode in count|materialize|json) from bench/qlever-synthetic/queries
#   op_<q-stem>_<mode>_us                                   from bench/operators/queries
#   sp2b_<q>_<mode>_us, watdiv_<q>_<mode>_us, bsbm_<q>_<mode>_us, dbpsb_<q>_<mode>_us
#   lubm_<q>_count_us                                       (count-only; reasoning suite)
#
# where <q-stem> is the *.rq file_stem() (sparq-cli `bench` uses path.file_stem()).
#
# Usage:
#   scripts/gen-metric-labels.py                 # (re)write bench/dashboard/metric-labels.json
#   scripts/gen-metric-labels.py --check         # drift gate: exit 1 if the file is out of date
#   scripts/gen-metric-labels.py --stdout        # print JSON to stdout, don't write
#
# DRIFT GATE: `--check` regenerates in memory and diffs against the committed file. Wire it into
# CI so adding/renaming a query without regenerating labels fails fast (the committed JSON is the
# served artifact — generation at serve time would need Python in the Pages job).
import argparse
import json
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BENCH = os.path.join(ROOT, "bench")
OUT = os.path.join(BENCH, "dashboard", "metric-labels.json")

# The three result-extraction modes ci-bench iterates (per scripts/ci-bench.sh). Each query
# metric is emitted once per mode it ran in; the dashboard shows one chart per (stem,mode).
MODES = {
    "count": "count (solution count)",
    "materialize": "materialize (build all rows)",
    "json": "json (serialize results)",
}
# LUBM is count-only (reasoning suite asserts solution counts; see ci-bench lubm hook).

# --- non-query (pipeline / memory) metrics: fixed, hand-curated from ci-bench.sh -------------
# These are NOT per-query, so they carry no <mode> suffix; keyed by their exact metric name.
FIXED = {
    "load_s": {
        "label": "Load — parse + index build (synthetic)",
        "suite": "Pipeline", "dataset": "synthetic (sparq-bench dump, SCALE entities)",
        "query": "end-to-end load: N-Triples parse + store/dict build", "unit": "s"},
    "rdfs_infer_s": {
        "label": "RDFS inference — depth-20 class closure",
        "suite": "Pipeline", "dataset": "synthetic (SCALE individuals, depth-20 subClassOf chain)",
        "query": "materialize the RDFS closure (rdfs:subClassOf)", "unit": "s"},
    "parse_ns_per_byte": {
        "label": "Parse cost — ns per input byte (fixed corpus)",
        "suite": "Memory / Size", "dataset": "synthetic, fixed 50k-entity corpus (deterministic)",
        "query": "N-Triples parse throughput as ns/byte (= 1000 / MB/s)", "unit": "ns/byte"},
    "store_bytes_per_triple": {
        "label": "Store size — bytes per triple",
        "suite": "Memory / Size", "dataset": "synthetic (primary SCALE)",
        "query": "triple-store memory layout footprint", "unit": "bytes"},
    "store_bytes_per_triple_small": {
        "label": "Store size — bytes per triple (fixed small corpus)",
        "suite": "Memory / Size", "dataset": "synthetic, fixed 50k-entity corpus (deterministic)",
        "query": "per-triple overhead on a second, fixed scale", "unit": "bytes"},
    "dict_bytes_per_term": {
        "label": "Dictionary size — bytes per term",
        "suite": "Memory / Size", "dataset": "synthetic (primary SCALE)",
        "query": "term-dictionary memory layout footprint", "unit": "bytes"},
    "wasm_bundle_bytes": {
        "label": "WASM bundle size",
        "suite": "Memory / Size", "dataset": "n/a (build artifact)",
        "query": "compiled sparq-wasm browser bundle size", "unit": "bytes"},
    # (sq-7d3dj.14) TREND-ONLY shipped size after wasm-opt -Oz. Emitted alongside
    # wasm_bundle_bytes (the hard-gated raw ratchet) to make real shipped-size wins visible.
    # Not in scripts/perf-gate.py. The wasm-opt version is logged per CI run (see ci-bench.sh).
    "wasm_opt_bundle_bytes": {
        "label": "WASM shipped size (wasm-opt -Oz)",
        "suite": "Memory / Size",
        "dataset": "n/a (build artifact, post wasm-opt -Oz; binaryen — version logged per CI run)",
        "query": "sparq-wasm bundle size after wasm-opt -Oz (trend-only; raw gate is wasm_bundle_bytes)",
        "unit": "bytes"},
}

# --- qlever-synthetic: descriptive filenames (qNN_<slug>); slug words are the description -----
# The filename slug IS the human description (e.g. q03_star3 -> "star3", q10_optional_age).
SYN_DESC = {
    "q02_type_person": "type lookup — ?s a ex:Person",
    "q03_star3": "3-way star join (name/age/city on one subject)",
    "q04_follows_name": "2-hop chain — follows then name",
    "q06_filter_age": "FILTER on numeric age (> 90)",
    "q09_count_edges": "COUNT(*) over follows edges",
    "q10_optional_age": "OPTIONAL left-join — name with optional age",
}

# --- operators: one-line description per family (from the .rq header comments) ----------------
OP_DESC = {
    "q01_bgp": "BGP — single triple pattern",
    "q02_star3": "star join — 3 patterns sharing the subject",
    "q03_chain": "chain join — 2-hop follows path",
    "q04_triangle": "cyclic join — directed triangle",
    "q05_union": "UNION of two subjects",
    "q06_optional": "OPTIONAL (left join) with optional age",
    "q07_optional_notbound": "negation via OPTIONAL + !BOUND",
    "q08_minus": "MINUS — set difference",
    "q09_filter_numeric": "FILTER — numeric range",
    "q10_filter_string": "FILTER — regex / CONTAINS on a literal",
    "q11_filter_in": "FILTER IN — membership in a fixed set",
    "q12_filter_exists": "FILTER EXISTS",
    "q13_bind": "BIND — computed value",
    "q14_values": "VALUES — inline data table join",
    "q15_agg_group_having": "aggregates + GROUP BY + HAVING",
    "q16_distinct": "DISTINCT",
    "q17_orderby_limit_offset": "ORDER BY + LIMIT + OFFSET",
    "q18_path_plus": "property path + (one-or-more), ASK",
    "q19_path_star": "property path * (zero-or-more)",
    "q20_path_opt": "property path ? (zero-or-one)",
    "q21_path_seq": "property path sequence (a/b)",
    "q22_path_alt": "property path alternative (a|b)",
    "q23_path_inverse": "property path inverse (^a)",
    "q24_path_negated_pset": "property path negated set !(...)",
    "q25_subquery": "subquery (nested SELECT aggregate)",
    "q26_ask": "ASK form",
    "q27_construct": "CONSTRUCT form (produced triples)",
    "q28_describe": "DESCRIBE form (produced triples)",
}

# --- well-known suites: per-query short descriptions, derived from the .rq files + readmes -----
SP2B_DESC = {
    "q01": "single journal's issue year (selective lookup)",
    "q02": "all inproceedings with bibliographic detail (large star + ORDER)",
    "q03a": "articles with a property (FILTER property = pages)",
    "q03b": "articles with a property (FILTER property = month)",
    "q03c": "articles with a property (FILTER property = ISBN; empty)",
    "q04": "DISTINCT co-author name pairs (large self-join)",
    "q05a": "author identity by name (implicit join, heavy)",
    "q05b": "author of both an article and an inproceedings (DISTINCT)",
    "q06": "OPTIONAL + !bound negation (heavy)",
    "q07": "nested OPTIONAL over document references (DISTINCT)",
    "q08": "Erdős co-author DISTINCT names (UNION + nested joins)",
    "q09": "DISTINCT incoming/outgoing predicates of persons (UNION)",
    "q10": "all statements about a fixed person",
    "q11": "seeAlso links — ORDER BY + LIMIT/OFFSET",
    "q12a": "Q8-as-ASK boolean (heavy)",
    "q12b": "ASK — does an Erdős co-author chain exist? (true)",
    "q12c": "ASK — is a fixed person typed foaf:Person? (false)",
}
SP2B_OP = {
    "q01": "BGP, selective lookup", "q02": "large star + ORDER BY", "q03a": "FILTER on property",
    "q03b": "FILTER on property", "q03c": "FILTER on property (empty)", "q04": "DISTINCT self-join",
    "q05a": "implicit join", "q05b": "DISTINCT join", "q06": "OPTIONAL + !bound",
    "q07": "nested OPTIONAL", "q08": "UNION + nested joins", "q09": "UNION over predicates",
    "q10": "statements about a resource", "q11": "ORDER + LIMIT/OFFSET", "q12a": "ASK (heavy)",
    "q12b": "ASK (true)", "q12c": "ASK (false)",
}

# WatDiv Basic-Testing families (README: Linear L, Star S, Snowflake F, Complex C).
WATDIV_FAMILY = {"L": "linear", "S": "star", "F": "snowflake", "C": "complex"}

DBPSB_DESC = {
    "q01": "BGP — people born in New York City",
    "q02": "chain join — person → birthPlace → country",
    "q03": "star join — birthPlace and deathPlace on one person",
    "q04": "OPTIONAL — occupation with optional spouse",
    "q05": "UNION of two predicates onto one variable",
    "q06": "DISTINCT + FILTER regex over IRI",
    "q07": "FILTER ≠ — people not born in the USA",
    "q08": "DISTINCT + ORDER BY + LIMIT (top-K)",
    "q09": "GROUP BY + COUNT + HAVING + ORDER + LIMIT",
    "q10": "scalar aggregate — COUNT(*)",
    "q11": "negation by OPTIONAL + !bound (no deathPlace)",
    "q12": "place-anchored BGP join (born/died at one place)",
    "q13": "subquery + aggregate — top occupations by holders",
    "h01": "HEAVY — co-birthplace pairs (unselective self-join)",
    "h02": "HEAVY — unbounded 3-pattern chain",
    "h03": "HEAVY — unbounded place self-join (~3M solutions)",
}
DBPSB_OP = {
    "q01": "BGP", "q02": "chain join", "q03": "star join", "q04": "OPTIONAL", "q05": "UNION",
    "q06": "DISTINCT + FILTER regex", "q07": "FILTER ≠", "q08": "DISTINCT + ORDER + LIMIT",
    "q09": "GROUP BY + HAVING", "q10": "scalar aggregate", "q11": "OPTIONAL + !bound",
    "q12": "anchored BGP join", "q13": "subquery aggregate", "h01": "heavy self-join",
    "h02": "heavy chain", "h03": "heavy self-join",
}

BSBM_DESC = {
    "query01": "find products by type + two features (ORDER/LIMIT)",
    "query02": "product detail — labels + 3 OPTIONALs",
    "query03": "products by type/feature with negation (OPTIONAL+!bound)",
    "query04": "two-branch UNION + OFFSET/LIMIT",
    "query05": "similar products — self-join + numeric FILTERs",
    "query06": "HEAVY — unanchored regex over product labels (full scan)",
    "query07": "product offers + reviews (nested OPTIONALs)",
    "query08": "reviews in English — langMatches + ratings + ORDER",
    "query09": "DESCRIBE a reviewer (produced triples)",
    "query10": "cheapest offers for a product (FILTER + ORDER/LIMIT)",
    "query11": "all in/out triples of an offer (UNION)",
    "query12": "CONSTRUCT — export an offer (produced triples)",
}
BSBM_OP = {
    "query01": "type + feature filter", "query02": "detail + OPTIONALs", "query03": "OPTIONAL + !bound",
    "query04": "UNION + OFFSET/LIMIT", "query05": "self-join + FILTER", "query06": "heavy regex scan",
    "query07": "nested OPTIONALs", "query08": "langMatches + ORDER", "query09": "DESCRIBE",
    "query10": "FILTER + ORDER/LIMIT", "query11": "UNION", "query12": "CONSTRUCT",
}

# LUBM: split extensional (raw ABox) vs entailed (OWL-RL closure) — directory tells us which.
LUBM_DESC = {
    "q01": ("extensional", "GraduateStudent taking a course (leaf type)"),
    "q02": ("extensional", "triangular join over 3 explicit leaf types"),
    "q03": ("extensional", "publications by an AssistantProfessor"),
    "q14": ("extensional", "all UndergraduateStudent instances"),
    "q04": ("entailed", "Professors (rdfs:subClassOf superclass)"),
    "q05": ("entailed", "Persons (top-level subClassOf)"),
    "q06": ("entailed", "Students (OWL restriction classification)"),
    "q07": ("entailed", "Students taking a professor's course (Student closure)"),
    "q08": ("entailed", "Students in a department, with email (Student closure)"),
    "q09": ("entailed", "Student–Faculty–Course triangle (heaviest)"),
    "q10": ("entailed", "Students taking GraduateCourse0 (grad closure)"),
    "q11": ("entailed", "ResearchGroups under University0 (transitive)"),
    "q12": ("entailed", "Chairs of a sub-department (defined-class + transitive)"),
    "q13": ("entailed", "Alumni of University0 (owl:inverseOf)"),
}

# (sq-7iai) SHACL validation workloads -> per-shape-graph human descriptions. The
# shape-file stems under bench/shacl/shapes/*.ttl drive the metric names shacl_<workload>_validate.
SHACL_DESC = {
    "cardinality": "sh:minCount/sh:maxCount over ub:FullProfessor (focus enumeration + path counting)",
    "datatype_range": "sh:datatype/sh:pattern over ub:GraduateStudent literals (XSD lexical)",
    "class_nodekind": "sh:class (subclass-closure target) + sh:nodeKind over ub:worksFor",
    "node_paths": "sh:node + sequence/inverse paths over ub:GraduateStudent (inter-shape recursion)",
    "sparql_constraint": "sh:sparql (SHACL §5.2) routed through sparq-engine",
}

# (sq-tf8n) GeoSPARQL workloads -> human descriptions. The ci-bench hook emits
# `geo_<name>_us` for each bench_geo workload (advisory) + the HARD-gated geo_compliance_deficit.
GEO_DESC = {
    "within10km": "#entities within 10 km great-circle of (0, 51) (geof:within result-set size)",
    "within50km": "#entities within 50 km great-circle of (0, 51)",
    "nearest_k10": "k=10 nearest entities to (0, 51) (nearest-k; count invariant = k)",
    "nearest_k100": "k=100 nearest entities to (0, 51)",
    "geof_within": "#corpus points satisfying geof:sfWithin(point, 1°x1° box) — the geof: filter path",
    "geo_compliance_pass": "#OGC topology fixtures (sf/eh/rcc8) matching the spec truth value",
}

# (sq-ustq) Full-text-search workloads -> human descriptions. These are FIXED
# workload names emitted by examples/bench_text (the FTS corpus is synthetic, generated
# in-process — there are no *.rq / *.ttl input files to glob), driving the ADVISORY metric
# names text_<workload>_us (per-query latency) + text_build_s (index build). The deterministic
# gate (hit counts + fts_bytes_per_doc) lives in bench/fts/run.sh's self-assertion + the
# fts_bytes_per_doc mode:auto ratchet (bench/perf-baseline.json), not on the dashboard.
FTS_DESC = {
    "and_terms": "text:matches — docs containing BOTH terms (AND)",
    "or_terms": "text:matchesAny — docs containing at least one term (OR)",
    "prefix4": "text:matches 'abcd*' — 4-char prefix (autocomplete)",
    "phrase": "text:phrase — two adjacent in-order terms",
    "near_slop2": "text:near (slop 2) — two in-order terms within gap 2",
}


def stems(subdir, ext=".rq"):
    """Sorted file stems under bench/<subdir> with the given extension (default *.rq, matching
    sparq-cli `bench` ordering). (sq-7iai, gap G2) `ext` is parameterised so non-query
    suites can enumerate their inputs — SHACL's shape graphs are `*.ttl`, not `*.rq`."""
    d = os.path.join(BENCH, subdir)
    if not os.path.isdir(d):
        return []
    n = len(ext)
    return sorted(f[:-n] for f in os.listdir(d) if f.endswith(ext))


def deeptax_depths():
    """The Deep Taxonomy depth tiers, read from bench/deep-taxonomy/expected.tsv (the ground
    truth the suite asserts against). Each line is `depth<TAB>closure_triples<TAB>query_rows`;
    `#`/blank lines are skipped. Sorted ascending. So adding a tier to expected.tsv + the run
    set automatically labels its metrics here (and `--check` flags the resulting drift)."""
    path = os.path.join(BENCH, "deep-taxonomy", "expected.tsv")
    if not os.path.isfile(path):
        return []
    out = []
    with open(path, encoding="utf-8") as fh:
        for line in fh:
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            parts = line.split("\t")
            if parts and parts[0].isdigit():
                out.append(int(parts[0]))
    return sorted(out)


def deeptax_tier_name(depth):
    """Human "dtNk"/"dtN" tier label for a depth (matches bench/inference/eye-comparison.md +
    the EYE reference scales: 1000->dt1k, 10000->dt10k, 100000->dt100k)."""
    if depth and depth % 1000 == 0:
        return "dt%dk" % (depth // 1000)
    return "dt%d" % depth


def mode_records(stem_key, base_label, suite, dataset, query_desc, modes=MODES):
    """Emit one {label,...} record per mode for a query stem, keyed `<stem>_<mode>`.

    The visible label is "<base_label> — <desc>, <mode>"; if <desc> is empty (the
    base_label already says everything, e.g. a descriptive op family) we drop the dash."""
    out = {}
    for mode in modes:
        head = "%s — %s" % (base_label, query_desc) if query_desc else base_label
        out["%s_%s" % (stem_key, mode)] = {
            "label": "%s, %s" % (head, mode),
            "suite": suite, "dataset": dataset, "query": query_desc or base_label,
            "mode": mode, "unit": "µs",
        }
    return out


def build():
    labels = {}

    # 1) fixed pipeline / memory metrics (exact names; no mode suffix).
    for name, rec in FIXED.items():
        labels[name] = dict(rec)

    # 2) qlever-synthetic engine queries -> "Synthetic: <desc>, <mode>".
    for s in stems("qlever-synthetic/queries"):
        desc = SYN_DESC.get(s, s.replace("_", " "))
        labels.update(mode_records(
            s, "Synthetic", "Synthetic (qlever-style)",
            "synthetic social graph (sparq-bench dump)", desc))

    # 3) operator-coverage suite -> "Operator: <family desc>, <mode>". The descriptive slug
    # already names the operator, so the label leads with the human description, not the stem.
    for s in stems("operators/queries"):
        desc = OP_DESC.get(s, s.replace("_", " "))
        labels.update(mode_records(
            "op_" + s, "Operator", "Operators",
            "synthetic social graph (sparq-bench dump)", desc))

    # 4) SP2Bench (per-commit + heavy) -> "SP2Bench <q> — <desc>, <mode>".
    for sub, heavy in (("sp2b/queries", False), ("sp2b/queries-heavy", True)):
        for s in stems(sub):
            desc = SP2B_DESC.get(s, SP2B_OP.get(s, s))
            tag = " (heavy)" if heavy else ""
            labels.update(mode_records(
                "sp2b_" + s, "SP2Bench " + s, "SP2Bench",
                "SP2Bench DBLP-in-RDF, 250k triples" + tag, desc))

    # 5) WatDiv (family from the leading letter L/S/F/C) -> "WatDiv S1 — star, <mode>".
    # (sq-1wrw) The stem carries the scale factor as an `_sf<SF>` token, matching the
    # ci-bench emitter (`watdiv_sf<SF>_<query>_<mode>_us`): SF=1 for the per-commit tier and the
    # documented SF=1000 full reference scale for the heavy nightly tier (bench/watdiv/README.md
    # Tiering table). Distinct tokens keep the two SF tiers as SEPARATE github-action-benchmark
    # series so the dashboard's engine-vs-scale-factor scaling chart (sq-viby) can plot them on one
    # axis (bench/dashboard/dashboard.js sizeAxisOf parses `_sf<digits>`).
    for sub, heavy in (("watdiv/queries", False), ("watdiv/queries-heavy", True)):
        sf = 1000 if heavy else 1
        for s in stems(sub):
            fam = WATDIV_FAMILY.get(s[0], "query")
            # Per-commit runs at SF=1 (~106k triples); the heavy queries are empty at SF=1
            # and only populate at the EC2/nightly scale (SF≥10, up to SF=1000 ≈ 100M+),
            # per bench/watdiv/README.md — so do NOT claim SF=1 for a heavy metric.
            dataset = ("WatDiv SF=1000 full-scale (nightly) ≈ 100M+ triples"
                       if heavy else "WatDiv SF=1, ~106k triples")
            desc = "%s query" % fam
            labels.update(mode_records(
                "watdiv_sf%d_%s" % (sf, s), "WatDiv %s (SF=%d)" % (s, sf), "WatDiv",
                dataset, desc))

    # 6) BSBM Explore mix -> "BSBM query01 — <desc>, <mode>".
    for sub, heavy in (("bsbm/queries", False), ("bsbm/queries-heavy", True)):
        for s in stems(sub):
            desc = BSBM_DESC.get(s, BSBM_OP.get(s, s))
            # Per-commit is -pc 300 (~116k triples); the heavy query (query06) runs at the
            # EC2/nightly full reference scale (-pc ~284,826 ≈ 100M triples), per
            # bench/bsbm/README.md — so do NOT claim the small -pc 300 count for it.
            dataset = ("BSBM Explore, full-scale (nightly), -pc ~284,826 ≈ 100M triples (heavy)"
                       if heavy else "BSBM Explore, -pc 300, ~116k triples")
            labels.update(mode_records(
                "bsbm_" + s, "BSBM " + s, "BSBM",
                dataset, desc))

    # 7) DBPSB / FEASIBLE -> "DBPSB q01 — <desc>, <mode>".
    for sub, heavy in (("dbpsb/queries", False), ("dbpsb/queries-heavy", True)):
        for s in stems(sub):
            desc = DBPSB_DESC.get(s, DBPSB_OP.get(s, s))
            # Per-commit runs against the 750k-triple head cut; the heavy queries run against
            # the FULL pinned artifact (~11.8M object-property triples) on the EC2/nightly
            # tier, per bench/dbpsb/README.md — so do NOT claim the 750k cut for them.
            dataset = ("DBpedia full artifact, ~11.8M triples (heavy, nightly)"
                       if heavy else "DBpedia slice, 750k-triple cut")
            labels.update(mode_records(
                "dbpsb_" + s, "DBPSB " + s, "DBPSB",
                dataset, desc))

    # 8) LUBM reasoning suite (COUNT ONLY) -> "LUBM Q06 — Students …, count [entailed]".
    for sub in ("lubm/queries-extensional", "lubm/queries-entailed"):
        for s in stems(sub):
            regime, desc = LUBM_DESC.get(s, ("extensional", s))
            labels["lubm_%s_count" % s] = {
                "label": "LUBM %s — %s, count [%s]" % (s.upper(), desc, regime),
                "suite": "LUBM (reasoning)",
                "dataset": "LUBM(1) %s" % (
                    "raw ABox" if regime == "extensional" else "OWL-RL closure"),
                "query": desc, "mode": "count", "regime": regime, "unit": "µs",
            }

    # 9) Deep Taxonomy reasoning suite (sq-1hgz) -> three metrics per depth tier. The metric
    # NAMES carry a numeric `d<DEPTH>` token (so the dashboard derives the depth axis for its
    # scaling chart, and `deeptax` routes it to the featured "Deep Taxonomy" suite). One depth
    # tier per row of bench/deep-taxonomy/expected.tsv. Keys match the dashboard's lookup (stem =
    # name minus `_us`, else the raw name), so the `_s`/`_triples` metrics are keyed by raw name
    # and the `_us` latency by its `_query` stem. (See bench/deep-taxonomy/README.md.)
    for depth in deeptax_depths():
        tier = deeptax_tier_name(depth)
        dataset = ("DeepTaxonomy %s — 1 instance, %d-deep :sc chain, 1 transitivity meta-rule "
                   "(reuses bench/inference/gen_deeptaxonomy.py); N3 forward closure = %d triples"
                   % (tier, depth, 2 * depth + 1))
        labels["deeptax_d%d_closure_s" % depth] = {
            "label": "Deep Taxonomy %s — N3 forward-closure time" % tier,
            "suite": "Deep Taxonomy", "dataset": dataset,
            "query": "materialize the N3 forward closure (transitivity meta-rule)",
            "mode": "closure", "unit": "s",
        }
        labels["deeptax_d%d_query" % depth] = {
            "label": "Deep Taxonomy %s — class-membership query over the closure" % tier,
            "suite": "Deep Taxonomy", "dataset": dataset,
            "query": "SELECT ?c WHERE { :i a ?c } (returns depth+1 = %d rows)" % (depth + 1),
            "mode": "count", "unit": "µs",
        }
        labels["deeptax_d%d_closure_triples" % depth] = {
            "label": "Deep Taxonomy %s — materialized closure size" % tier,
            "suite": "Deep Taxonomy", "dataset": dataset,
            "query": "deterministic closure triple-count (self-asserting gate; = 2·depth+1 = %d)"
                     % (2 * depth + 1),
            "mode": "closure", "unit": "triples",
        }

    # 10) SHACL validation suite (sq-7iai, gap G2 — ext=".ttl") -> one ADVISORY validate-time
    # metric per committed shape graph. The hook emits `shacl_<workload>_validate_us`; the
    # dashboard strips the `_us` for the label key (like LUBM's `lubm_<q>_count`). The deterministic
    # gates (violations/conforms/focus_nodes) live in bench/shacl/run.sh's self-assertion + the
    # W3C `BASELINE_PASS` ratchet (crates/sparq-shacl/tests/w3c_core.rs), not on the dashboard.
    for s in stems("shacl/shapes", ext=".ttl"):
        desc = SHACL_DESC.get(s, s.replace("_", " "))
        labels["shacl_%s_validate" % s] = {
            "label": "SHACL %s — %s, validate" % (s, desc),
            "suite": "SHACL validation",
            "dataset": "LUBM(1) ABox, ~103k triples x hand-authored shape graph",
            "query": desc, "mode": "validate", "unit": "µs",
        }

    # 11) Full-text-search suite (sq-ustq, design §3.4) -> one ADVISORY per-query-latency metric
    # per workload (text_<workload>_us; dashboard strips the _us for the label key) + the index
    # build time (text_build_s). The corpus is synthetic (in-process), so the workload names are
    # FIXED (not file stems). The deterministic gates (hit counts + fts_bytes_per_doc) live in
    # bench/fts/run.sh's self-assertion + the fts_bytes_per_doc mode:auto ratchet, not here.
    FTS_DATASET = ("synthetic FTS corpus — 100k 8-word literals, ~10k-term Zipf vocab, "
                   "BM25 positions-enabled index (bench/fts/gen.sh)")
    for s, desc in FTS_DESC.items():
        labels["text_%s" % s] = {
            "label": "Full-Text %s — %s, query" % (s, desc),
            "suite": "Full-Text", "dataset": FTS_DATASET,
            "query": desc, "mode": "query", "unit": "µs",
        }
    labels["text_build_s"] = {
        "label": "Full-Text — BM25 index build time",
        "suite": "Full-Text", "dataset": FTS_DATASET,
        "query": "build the positions-enabled BM25 inverted index over the corpus",
        "mode": "build", "unit": "s",
    }
    labels["fts_bytes_per_doc"] = {
        "label": "Full-Text — index bytes per document",
        "suite": "Full-Text", "dataset": FTS_DATASET,
        "query": "deterministic index heap_bytes()/len() (mode:auto ratchet gate)",
        "mode": "bytes", "unit": "bytes",
    }

    # 12) Vector / ANN suite (sq-v02y, design §3.3) -> per-searcher recall-DEFICIT gate metrics
    # (vectors_<searcher>_recall_at10) + advisory per-query latency (vectors_<searcher>_query_us;
    # the dashboard strips the _us for the label key) + the HNSW index build time (vectors_build_s).
    # The corpus is synthetic (in-process), so the searcher names are FIXED (not file stems). Recall
    # is emitted as a DEFICIT (round((1-recall)*1000)) so it slots into the smaller-is-better
    # mode:auto ratchet (gap G4). diskann/pq are EXACT-gated + mode:auto ratcheted; hnsw is
    # floor-gated in bench/vector/run.sh (rayon-parallel build jitter).
    VEC_DATASET = ("synthetic ANN corpus — 50k 32-d vectors from a seeded splitmix64 PRNG "
                   "(PQ on a fixed 10k clustered set); recall@10 vs nearest_exact (bench/vector/gen.sh)")
    VEC_DESC = {
        "hnsw": ("in-RAM HNSW (instant-distance), recall@10 floor 0.95 (floor-gated)",
                 "in-RAM HNSW approximate kNN"),
        "diskann": ("on-disk Vamana (.spqg), recall@10 floor 0.90 (exact-gated + ratchet)",
                    "on-disk Vamana (DiskANN) kNN"),
        "pq": ("PQ codes + full-precision re-rank, recall@10 floor 0.95 (exact-gated + ratchet)",
               "product-quantized kNN + re-rank"),
    }
    for s, (gate_desc, q_desc) in VEC_DESC.items():
        labels["vectors_%s_recall_at10" % s] = {
            "label": "Vector / ANN %s — recall@10 deficit, gate" % s,
            "suite": "Vector / ANN", "dataset": VEC_DATASET,
            "query": gate_desc, "mode": "recall_deficit", "unit": "milli",
        }
        labels["vectors_%s_query" % s] = {
            "label": "Vector / ANN %s — query latency" % s,
            "suite": "Vector / ANN", "dataset": VEC_DATASET,
            "query": q_desc, "mode": "query", "unit": "µs",
        }
    labels["vectors_build_s"] = {
        "label": "Vector / ANN — HNSW index build time",
        "suite": "Vector / ANN", "dataset": VEC_DATASET,
        "query": "build the in-RAM HNSW index over the corpus (advisory)",
        "mode": "build", "unit": "s",
    }

    # 13) GeoSPARQL suite (sq-tf8n, design §3.5). The ci-bench hook emits `geo_<name>_us`
    # (ADVISORY trend timing) per bench_geo workload — the dashboard strips `_us` for the
    # label key. The deterministic gates (result-set sizes) live in bench/geo/run.sh's
    # self-assertion; the compliance ratchet is the HARD-gated DEFICIT geo_compliance_deficit
    # (= 25 - passed, mode:auto in bench/perf-baseline.json), labelled separately below.
    GEO_DATASET = "fixed CRS84 point corpus, ~100k seeded POINT literals (8°x8° window)"
    # (sq-tf8n) These geo_<name> dashboard series are the ADVISORY per-workload QUERY
    # TIMES (harvested from geo_<name>_us, stripped of the _us suffix), NOT counts — the result-set
    # SIZES are the deterministic gate in bench/geo/run.sh's self-assertion, never plotted here. So
    # the mode/unit/label must read as a TIME (mode "query", unit µs, ", query" suffix), exactly like
    # the FTS text_<workload> labels — the `desc` still says WHAT the query computes (a count), but
    # the plotted value is its latency. (Earlier `mode:count` contradicted the µs unit.)
    for name, desc in GEO_DESC.items():
        labels["geo_%s" % name] = {
            "label": "GeoSPARQL %s — %s, query" % (name, desc),
            "suite": "GeoSPARQL", "dataset": GEO_DATASET,
            "query": desc, "mode": "query", "unit": "µs",
        }
    labels["geo_compliance_deficit"] = {
        "label": "GeoSPARQL compliance deficit — (25 OGC fixtures − passing); smaller is better",
        "suite": "GeoSPARQL", "dataset": "OGC GeoSPARQL topology fixtures (sf/eh/rcc8)",
        "query": "deterministic compliance ratchet (mode:auto; coverage only tightens)",
        "mode": "deficit", "unit": "fixtures",
    }

    # 13.5) HDT load-and-decode suite (sq-lrp9, design §3.7). The ci-bench hook emits the
    # DETERMINISTIC load-and-decode counts (snikmeta_* , unit count — the HARD gate in
    # bench/hdt/run.sh's self-assertion) plus two ADVISORY rows: hdt_load_s (wall-clock, trend)
    # and hdt_vs_ntgz_load_s (a RATIO that survives box contention; trend, NEVER asserted).
    # Load-and-decode-to-native is the ONLY like-for-like axis vs hdt-cpp/java (query-over-HDT is
    # a NON-goal); the deterministic counts ARE the gate, not the dashboard. The `hdt` alias in the
    # dashboard's FEATURED_SUITES routes these metrics to the HDT card via name-token match.
    HDT_DATASET = ("vendored real-world snikmeta.hdt (hdt-cpp/java-shaped FourSectDict+BitmapTriples; "
                   "328 triples) for the gate; deterministic ~250k-triple synthetic archive for the "
                   "advisory timing (HDT_BENCH_N)")
    HDT_GATE = {
        "snikmeta_triples": "stored triples of the decoded graph (load-and-decode-to-native)",
        "snikmeta_terms": "distinct dictionary terms interned into sparq's Dict (id-translation)",
        "snikmeta_distinct_predicates": "distinct predicates via a full SPO scan (triple-pattern resolution)",
        "snikmeta_rdf_type_triples": "bound-predicate triple-pattern resolution (rdf:type triples)",
        "snikmeta_direct_eq_upstream": "1 iff direct decoder == upstream-backed path on the same bytes (oracle)",
    }
    for name, desc in HDT_GATE.items():
        labels[name] = {
            "label": "HDT %s — load-and-decode gate" % name.replace("snikmeta_", ""),
            "suite": "HDT", "dataset": HDT_DATASET,
            "query": desc, "mode": "count", "unit": "count",
        }
    labels["hdt_load_s"] = {
        "label": "HDT — load+decode .hdt to a native Graph (advisory)",
        "suite": "HDT", "dataset": HDT_DATASET,
        "query": "wall-clock load+decode of the synthetic .hdt (trend-only; machine-sensitive)",
        "mode": "load", "unit": "s",
    }
    labels["hdt_vs_ntgz_load_s"] = {
        "label": "HDT — gzipped-N-Triples / HDT load-time ratio (advisory)",
        "suite": "HDT", "dataset": HDT_DATASET,
        "query": "ntgz_load_s / hdt_load_s on the same triples (ratio survives contention; trend-only)",
        "mode": "ratio", "unit": "ratio",
    }

    # 13.6) SOLID WAC/ACP auth-view suite (sq-k0km). The ci-bench `solid` hook (bench/solid/run.sh)
    # emits DETERMINISTIC STRUCTURAL counts — a pure function of the in-crate WAC + ACP fixtures
    # (sparq_solid::fixture), byte-stable across machines (NOT wall-clock). These are the gate (the
    # run.sh self-assertion vs bench/solid/expected.tsv) and the dashboard's Solid/access-control
    # family card data. The `suite` lowercases to "solid", which the dashboard's `solid` FAMILY
    # (familyOf) matches, so every metric routes to the Solid card. The metric STEMS here MUST match
    # what scripts/ci-bench.sh emits so the gen-metric-labels --check drift gate stays green.
    SOLID_DATASET = ("in-crate ~1.1k-graph WAC fixture (+ ACP variant), built deterministically by "
                     "sparq_solid::fixture::{wac_fixture, acp_fixture}")
    SOLID_GATE = {
        "solid_wac_named_graphs": "named graphs in the WAC fixture",
        "solid_wac_quads": "total quads across the WAC fixture's named graphs",
        "solid_wac_auth_triples": "triples in the materialised WAC auth view (materialize_wac)",
        "solid_acp_auth_triples": "triples in the materialised ACP auth view (materialize_acp, 3 strata)",
        "solid_alice_readable_graphs": "graphs visible to ALICE in Mode::Read (AuthIndex walk)",
        "solid_full_dataset_rows": "rows of the title query over the FULL dataset (GRAPH ?g)",
        "solid_authorized_rows": "rows of the title query restricted to alice's authorized subset",
    }
    for name, desc in SOLID_GATE.items():
        labels[name] = {
            "label": "Solid %s — WAC/ACP auth-view count" % name.replace("solid_", ""),
            "suite": "Solid", "dataset": SOLID_DATASET,
            "query": desc, "mode": "count", "unit": "count",
        }

    # 13.7) GenAI — sparq-nlq OFFLINE NL->SPARQL suite (sq-k0km). The ci-bench `nlq` hook
    # (bench/nlq/run.sh) emits DETERMINISTIC counts at a PINNED N=2000 — a pure function of N + the
    # synthetic typed schema (no external data, no network, offline fixed-completion stub LLM), so
    # byte-stable across machines (NOT wall-clock). These are the gate (run.sh self-assertion vs
    # bench/nlq/expected.tsv) and the dashboard's GenAI family card data. The `nlq` token / "NL→SPARQL"
    # suite both route the metrics to the dashboard's `genai` FAMILY (familyOf). HONESTY: offline
    # deterministic core only — LIVE exec-accuracy on a canonical host is a SEPARATE concern (sq-qidj/
    # sq-g0lw, EC2-blocked) and the sibling sim/introspect GenAI suites need the gitignored olympics.nt
    # (NOT in CI); both are EC2/dataset-gathered, not in this feed.
    NLQ_DATASET = ("synthetic typed graph generated in-example at N=2000 entities "
                   "(rdf:type/rdfs:label/ex:age per entity); deterministic, no external data, "
                   "offline stub LLM (no `live` feature, no network)")
    labels["nlq_synth_triples"] = {
        "label": "NL→SPARQL synthetic graph — triple count",
        "suite": "NL→SPARQL", "dataset": NLQ_DATASET,
        "query": "triples in the synthetic typed graph (3 per entity)", "mode": "count", "unit": "count"}
    labels["nlq_prompt_chars"] = {
        "label": "NL→SPARQL grounded prompt — char length (token-budget proxy)",
        "suite": "NL→SPARQL", "dataset": NLQ_DATASET,
        "query": "length of the schema-grounded prompt prompt_for(question) builds; smaller = leaner",
        "mode": "count", "unit": "chars"}
    labels["nlq_ask_result_rows"] = {
        "label": "NL→SPARQL ask loop — result rows",
        "suite": "NL→SPARQL", "dataset": NLQ_DATASET,
        "query": "rows the full ask loop returns (GROUP BY ?type -> one row per type)",
        "mode": "count", "unit": "count"}
    labels["nlq_ask_repairs"] = {
        "label": "NL→SPARQL ask loop — repair rounds",
        "suite": "NL→SPARQL", "dataset": NLQ_DATASET,
        "query": "repair rounds the ask loop needed (0 = stub's first SPARQL was valid + executable)",
        "mode": "count", "unit": "count"}

    # 14) ZERO-KNOWLEDGE family (sq dashboard-data-feed; ci-bench ZK hooks). EVERY ZK metric
    # name leads with the token `zk` so the dashboard's Zero-knowledge family prefix routing
    # buckets it. Three feeds, each grounded in an EXISTING bench (bench/benchmarks.toml zk-*):
    #   - zk_compose_<member>_gates : DETERMINISTIC ultra_honk circuit gate counts harvested
    #     from the committed bench/zk-compose/gate_counts_latest.json (members FIXED in
    #     bench/zk-compose/scripts/gate_counts.sh). The headline ZK measurement.
    #   - zk_canon_/zk_commit_<shape>_<n> : sparq-zk commitment-pipeline criterion MEANS (µs)
    #     from bench/zk/benches/zk_throughput.rs (canon + end-to-end commit, shapes iri/bnode
    #     at n=64/256/1024; commit also has leaves+fold/<n>; plus poseidon2 permutation/hash40).
    #   - zk_trace_<n>entities_<shape>_<arm> : zk-trace per-operator capture overhead criterion
    #     MEANS (µs) from bench/zk-trace/benches/trace_overhead.rs (8 plan shapes x traced/
    #     untraced at n=100/1000 entities). Wall-clock; trend-only (advisory), never a gate.
    # The metric STEMS here MUST match what scripts/ci-bench.sh emits (criterion path tokens)
    # so the gen-metric-labels --check drift gate stays green.
    ZK_GATE_MEMBERS = [
        "scan_k1_n16_r4", "scan_k2_n16_r8", "scan_k2_n64_r8",
        "filter_int_d1", "filter_int_d2", "filter_int_d4", "filter_f64",
        "join_eq_na16_nb16",
    ]
    ZK_GATES_DS = "compiled Noir circuits (zk/compose, nargo compile --workspace); bb gates -s ultra_honk"
    for m in ZK_GATE_MEMBERS:
        labels["zk_compose_%s_gates" % m] = {
            "label": "zk/compose %s — ultra_honk circuit gate count" % m,
            "suite": "Zero-knowledge", "dataset": ZK_GATES_DS,
            "query": "deterministic circuit_size (bb gates); smaller is better",
            "mode": "gates", "unit": "gates",
        }
    ZK_COMMIT_DS = ("synthetic graph shapes generated in-bench (iri = all-ground; "
                    "bnode = bnode chain), credential scale")
    for n in (64, 256, 1024):
        for shape in ("iri", "bnode"):
            labels["zk_canon_%s_%d" % (shape, n)] = {
                "label": "sparq-zk RDFC10 canon — %s graph, %d triples" % (shape, n),
                "suite": "Zero-knowledge", "dataset": ZK_COMMIT_DS,
                "query": "RDFC-1.0 canonicalisation (criterion mean)", "mode": "canon", "unit": "µs"}
            labels["zk_commit_%s_%d" % (shape, n)] = {
                "label": "sparq-zk end-to-end commit — %s graph, %d triples" % (shape, n),
                "suite": "Zero-knowledge", "dataset": ZK_COMMIT_DS,
                "query": "RDFC10 + leaf encode + Poseidon2 fold (criterion mean)",
                "mode": "commit", "unit": "µs"}
        labels["zk_commit_leaves_fold_%d" % n] = {
            "label": "sparq-zk recommit (leaves+fold) — precomputed canon, %d triples" % n,
            "suite": "Zero-knowledge", "dataset": ZK_COMMIT_DS,
            "query": "leaf encode + Poseidon2 fold over precomputed canonical form (criterion mean)",
            "mode": "commit", "unit": "µs"}
    labels["zk_poseidon2_permutation"] = {
        "label": "sparq-zk Poseidon2-BN254 permutation",
        "suite": "Zero-knowledge", "dataset": "fixed 4-element BN254 state",
        "query": "one Poseidon2 permutation (criterion mean)", "mode": "perm", "unit": "µs"}
    labels["zk_poseidon2_hash40"] = {
        "label": "sparq-zk Poseidon2-BN254 hash — 40 elements",
        "suite": "Zero-knowledge", "dataset": "fixed 40-element input",
        "query": "Poseidon2 sponge over 40 field elements (criterion mean)", "mode": "hash", "unit": "µs"}
    ZK_TRACE_DS = ("synthetic social graph (WatDiv-flavoured, ~9 triples/entity), "
                   "generated in-bench")
    ZK_TRACE_SHAPES = {
        "bgp_star": "BGP star join (age/name/city on ?s)",
        "bgp_chain": "BGP chain join (follows then age)",
        "bgp_triangle": "BGP triangle (3-way follows cycle)",
        "filter": "BGP + numeric FILTER(?a > 50)",
        "optional": "left join (OPTIONAL follows)",
        "union": "UNION of two single-pattern arms",
        "distinct": "DISTINCT projection over city",
        "count": "COUNT aggregate over age",
    }
    for n in (100, 1000):
        for shape, desc in ZK_TRACE_SHAPES.items():
            for arm, arm_desc in (("traced", "recorder armed + drained (proving path)"),
                                  ("untraced", "default execution (no recorder)")):
                labels["zk_trace_%dentities_%s_%s_%d" % (n, shape, arm, n)] = {
                    "label": "zk-trace %s — %s, %s, %d entities" % (shape, desc, arm, n),
                    "suite": "Zero-knowledge", "dataset": ZK_TRACE_DS,
                    "query": "%s; %s (criterion mean)" % (desc, arm_desc),
                    "mode": arm, "unit": "µs"}

    return labels


def serialize(labels):
    # Stable key order so `--check` is deterministic and diffs are minimal.
    ordered = {k: labels[k] for k in sorted(labels)}
    return json.dumps({
        "_comment": ("GENERATED by scripts/gen-metric-labels.py from the per-suite "
                     "*.rq files + ci-bench naming — DO NOT edit by hand; run the generator "
                     "(or `--check` in CI gates drift). Maps metric STEM (name minus _us) to a "
                     "readable record. bead sq-ocuf / design sq-i0nm."),
        "_schema": {"key": "metric stem (github-action-benchmark name without the _us suffix)",
                    "value": "{label, suite, dataset, query, mode?, regime?, unit}"},
        "labels": ordered,
    }, indent=2, ensure_ascii=False) + "\n"


def main():
    ap = argparse.ArgumentParser(description="Generate the dashboard metric-labels.json")
    ap.add_argument("--check", action="store_true",
                    help="exit 1 if the committed file is out of date (drift gate)")
    ap.add_argument("--stdout", action="store_true", help="print JSON to stdout, do not write")
    args = ap.parse_args()

    # Build the label map ONCE and reuse it for both the serialized output and the count, so
    # the filesystem scan is not duplicated and the reported count can never diverge from the
    # content actually written.
    labels = build()
    text = serialize(labels)

    if args.stdout:
        sys.stdout.write(text)
        return 0

    if args.check:
        try:
            with open(OUT, encoding="utf-8") as fh:
                current = fh.read()
        except FileNotFoundError:
            current = None
        if current != text:
            sys.stderr.write(
                "ERROR: %s is out of date — run scripts/gen-metric-labels.py "
                "(a query was added/renamed without regenerating labels).\n" % OUT)
            return 1
        print("metric-labels.json is up to date.")
        return 0

    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, "w", encoding="utf-8") as fh:
        fh.write(text)
    print("wrote %s (%d metrics labeled)" % (OUT, len(labels)))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
