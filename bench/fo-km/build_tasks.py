#!/usr/bin/env python3
"""Author bench/fo-km/tasks.jsonl — the FO-EXERCISING KM task set (Metric 1).

[OPUS-4.8] FO-KM benchmark (epic sq-mztg8). Design record:
research/foundational-ontology-km-benchmark.md (PR #1106). SPARQ agent.

Each task is a knowledge-management question that genuinely REQUIRES foundational typing:
the no-FO incumbent arm (which imports no upper ontology) cannot answer it without
hand-enumerating the domain classes, while an FO arm (overlay + --close owl-rl) answers
it via a single typed query over the FO category. The three strata mirror the design §5:
  TH  type-hierarchy   — "which entities are processes/perdurants vs objects/endurants?"
  ER  entailment-dep   — answer exists only after closure (inverse / subClassOf rollup)
  CC  cross-category   — relate across the FO top split (occurrent <-> information artifact)

CONTRACT (per the brief): every record has {id, kind, question, gold_keys}.
Augmented (runnable + self-documenting for the orchestrator's A/B):
  gold_count   expected distinct row count of the FO arm's answer
  select       per-arm SPARQL (keys: gufo / dolce-dul / schema-org) — the FO-typed query
  no_fo        the SPARQL the no-FO arm would attempt over the same question
  discriminates why the no-FO arm genuinely cannot answer (the FO-win construction)

The `select` queries are SELECT statements that return the entities in `gold_keys`
(rendered as short local names by pkg-query). `gold_keys` lists the discriminating local
names that MUST appear in the FO arm's answer and that the no-FO arm cannot produce.

GOLD ANSWERS ARE MEASURED, NOT HARD-CODED. Every entity list and every `gold_count` below
is PROBED from the live PKG at build time (`_rows`/`_count`, via the same `pkg-query`
binary the benchmark drives). The first cut of this file hard-coded them as Python
literals — 11 Findings / 6 Sources / 5 Techniques / 1255 Tasks — and the fixture then
ROTTED SILENTLY as the PKG grew: by the time this was caught the real graph held 15
Findings and 71 Sources, and `validate_tasks.py` reported 24 discrimination failures
because the committed gold counts no longer described the graph the arms query. Deriving
them makes `tasks.jsonl` a regenerable projection of the PKG rather than a snapshot that
silently decays. Re-run this script whenever `pkg-instances.ttl` changes.

The probes are written in **pkg: domain terms over the no-FO arm**, deliberately NOT in
any FO's vocabulary: the gold truth must be independent of the FO overlay under test.
`validate_tasks.py` then independently checks that each FO arm reproduces that same count,
so a wrong reference query shows up as cross-arm disagreement rather than a silent pass.
"""
import json
import subprocess
import sys

GUFO = "http://purl.org/nemo/gufo#"
DUL = "http://www.ontologydesignpatterns.org/ont/dul/DUL.owl#"
SCHEMA = "http://schema.org/"
PKG = "https://sparq.dev/ns/pkg#"
DCT = "http://purl.org/dc/terms/"

NO_FO_OVERLAY = "bench/fo-km/overlays/no-fo.ttl"
PROBE_PREFIXES = f"PREFIX pkg: <{PKG}>\nPREFIX dct: <{DCT}>\n"


def _probe(sparql):
    """Run one ground-truth probe against the no-FO arm under closure; return data rows.

    Same invocation shape as validate_tasks.py: pkg-query writes the header line, a `---`
    separator, then the data rows to stdout (banners and the row-count footer go to
    stderr), so the rows are everything from index 2 on.
    """
    cmd = [
        "cargo", "run", "-q", "-p", "sparq-kb", "--features", "close",
        "--bin", "pkg-query", "--",
        "--extra-graph", NO_FO_OVERLAY, "--close", "owl-rl",
        "--sparql", PROBE_PREFIXES + sparql,
    ]
    out = subprocess.run(cmd, capture_output=True, text=True)
    if out.returncode != 0:
        raise SystemExit(
            f"gold-answer probe FAILED (needs the `close` feature; run from the repo "
            f"root):\n{out.stderr}\nQUERY:\n{sparql}")
    lines = out.stdout.splitlines()
    return [l.strip() for l in lines[2:] if l.strip()]


def _rows(sparql):
    """Probed list of rendered local names (one column)."""
    rows = _probe(sparql)
    if not rows:
        raise SystemExit(f"gold-answer probe returned NO rows — the PKG is empty or the "
                         f"query is wrong:\n{sparql}")
    return rows


def _count(sparql):
    """Probed scalar COUNT."""
    rows = _probe(sparql)
    if len(rows) != 1 or not rows[0].lstrip("-").isdigit():
        raise SystemExit(f"gold-answer probe was not a scalar COUNT: {rows!r}\n{sparql}")
    return int(rows[0])


def _members(cls):
    return _rows(f"SELECT ?x WHERE {{ ?x a pkg:{cls} }} ORDER BY ?x")


# --- the measured gold truth (probed from the live PKG; see the module docstring) -------
print("probing the PKG for gold answers...", file=sys.stderr)

FINDINGS = _members("Finding")
SOURCES = _members("Source")
TECHNIQUES = _members("Technique")
INFO_ARTIFACTS = sorted(FINDINGS + SOURCES + TECHNIQUES)

N_FINDING, N_SOURCE, N_TECHNIQUE = len(FINDINGS), len(SOURCES), len(TECHNIQUES)
N_ARTIFACT = len(INFO_ARTIFACTS)
N_TASK = _count("SELECT (COUNT(DISTINCT ?x) AS ?n) WHERE { ?x a pkg:Task }")
# The FO top category rolls up all four domain classes (rdfs9 + rdfs11 through the
# overlay's intermediate categories).
N_TOP = N_TASK + N_ARTIFACT
# gufo:Endurant covers the CONCRETE artifacts only — Source + Technique, not the abstract
# Findings (gufo:AbstractIndividual) nor the Tasks (gufo:Event).
N_ENDURANT = N_SOURCE + N_TECHNIQUE

# Documents whose dct:subject is a Technique — the cc01 typed cross-category join.
SKILL_SURFACE_PAIRS = _rows(
    "SELECT DISTINCT ?src WHERE { ?src a pkg:Source ; dct:subject ?m . ?m a pkg:Technique } "
    "ORDER BY ?src")
# Distinct (finding, topic) pairs — more than the distinct Findings, as some cover >1 topic.
N_FINDING_TOPIC_PAIRS = _count(
    "SELECT (COUNT(*) AS ?n) WHERE { SELECT DISTINCT ?f ?topic "
    "WHERE { ?f a pkg:Finding ; dct:subject ?topic } }")
# Entities carrying ANY of the three incompatible state axes (Task status /
# Source exploredStatus / Finding assurance).
N_STATE_UNION = _count(
    "SELECT (COUNT(DISTINCT ?x) AS ?n) WHERE { { ?x pkg:status ?s } UNION "
    "{ ?x pkg:exploredStatus ?e } UNION { ?x pkg:assurance ?a } }")
# Downstream impact of the er01 closure-lever probe task (only pkg:dependsOn is asserted;
# pkg:blockedBy exists solely by owl:inverseOf entailment).
ER01_BEAD = "sq-8thu"
N_BLOCKED_BY_ER01 = _count(
    f"SELECT (COUNT(DISTINCT ?d) AS ?n) WHERE {{ ?b dct:identifier \"{ER01_BEAD}\" . "
    f"?b pkg:blockedBy ?d }}")

print(f"  probed: Task={N_TASK} Finding={N_FINDING} Source={N_SOURCE} "
      f"Technique={N_TECHNIQUE} artifacts={N_ARTIFACT} top={N_TOP}", file=sys.stderr)


def sel(prefix_iri, body):
    return f"PREFIX fo: <{prefix_iri}>\nPREFIX pkg: <{PKG}>\nPREFIX dct: <{DCT}>\nSELECT DISTINCT {body}"


def count(prefix_iri, body):
    return f"PREFIX fo: <{prefix_iri}>\nPREFIX pkg: <{PKG}>\nPREFIX dct: <{DCT}>\nSELECT (COUNT(DISTINCT {body}"


TASKS = []


def add(tid, kind, question, gold_keys, gold_count, select, no_fo, discriminates):
    TASKS.append({
        "id": tid,
        "kind": kind,
        "question": question,
        "gold_keys": gold_keys,
        "gold_count": gold_count,
        "select": select,
        "no_fo": no_fo,
        "discriminates": discriminates,
    })


# ---------------------------------------------------------------------------
# TH — type-hierarchy queries (need entailed rdf:type up the FO tree)
# ---------------------------------------------------------------------------

add(
    "th01", "TH",
    "List every information artifact the knowledge base tracks (the documents, findings, "
    "and methods — everything that is information content rather than a unit of work).",
    INFO_ARTIFACTS, N_ARTIFACT,
    {
        "gufo": sel(GUFO, "?x WHERE { { ?x a fo:Object } UNION { ?x a fo:AbstractIndividual } } ORDER BY ?x"),
        "dolce-dul": sel(DUL, "?x WHERE { ?x a fo:SocialObject } ORDER BY ?x"),
        "schema-org": sel(SCHEMA, "?x WHERE { ?x a fo:CreativeWork } ORDER BY ?x"),
    },
    sel(PKG, "?x WHERE { ?x a pkg:Information_Content_Entity } ORDER BY ?x"),
    "The PKG has NO common superclass over Source/Finding/Technique — they sit in three "
    "unrelated domain vocabularies (fabio:Expression, sig-impl:Assertion, skos:Concept). "
    "The no-FO arm must hand-enumerate three classes (and silently miss any future artifact "
    "class); an FO arm returns the union via a single typed query under closure. The canonical "
    "FO win: open-world extensibility (design §5 TH1).",
)

add(
    "th02", "TH",
    "Which entities in the project are occurrents/processes (things that happen over time), "
    "as opposed to continuants/objects that endure?",
    ["task-sq"], N_TASK,
    {
        "gufo": count(GUFO, "?x) AS ?n) WHERE { ?x a fo:Event }"),
        "dolce-dul": count(DUL, "?x) AS ?n) WHERE { ?x a fo:Event }"),
        "schema-org": None,
    },
    count(PKG, "?x) AS ?n) WHERE { ?x a pkg:Perdurant }"),
    "The occurrent/continuant partition is un-queryable in the no-FO PKG — nothing says a "
    "Task is process-like. gUFO (gufo:Event) and DUL (dul:Action SubClassOf dul:Event) draw "
    "it; schema.org CANNOT (schema:Action sits under schema:Thing parallel to CreativeWork, "
    "with no occurrent axis) — so this task discriminates BOTH no-FO and schema.org from the "
    "metaphysical FOs (design §5 TH2).",
)

add(
    "th03", "TH",
    "Which entities are truth-bearers (propositions/claims) versus information-bearers "
    "(documents/files)? Distinguish the findings from the sources.",
    {"truth_bearers": FINDINGS, "info_bearers": SOURCES},
    {"truth_bearers": N_FINDING, "info_bearers": N_SOURCE},
    {
        "gufo": {
            "truth_bearers": sel(GUFO, "?x WHERE { ?x a fo:AbstractIndividual } ORDER BY ?x"),
            "info_bearers": sel(GUFO, "?x WHERE { ?x a fo:Object FILTER NOT EXISTS { ?x a pkg:Technique } } ORDER BY ?x"),
        },
        "dolce-dul": {
            "truth_bearers": sel(DUL, "?x WHERE { ?x a fo:Description FILTER NOT EXISTS { ?x a fo:Method } } ORDER BY ?x"),
            "info_bearers": sel(DUL, "?x WHERE { ?x a fo:InformationObject } ORDER BY ?x"),
        },
        "schema-org": {
            "truth_bearers": sel(SCHEMA, "?x WHERE { ?x a fo:Claim } ORDER BY ?x"),
            "info_bearers": sel(SCHEMA, "?x WHERE { ?x a fo:DigitalDocument } ORDER BY ?x"),
        },
    },
    sel(PKG, "?x WHERE { ?x a pkg:Proposition } ORDER BY ?x"),
    "The proposition (Finding) vs information-bearer (Source) distinction is collapsed in the "
    "status quo (both are just 'things with provenance'). Every FO arm draws it; the no-FO arm "
    "has no category to query (design §5 TH3).",
)

add(
    "th04", "TH",
    "List the methods/techniques the project knows about, typed as such by the foundational "
    "category (so a method authored under a different domain vocabulary would still be found).",
    TECHNIQUES, N_TECHNIQUE,
    {
        "gufo": sel(GUFO, "?x WHERE { ?x a fo:Object . ?x a pkg:Technique } ORDER BY ?x"),
        "dolce-dul": sel(DUL, "?x WHERE { ?x a fo:Method } ORDER BY ?x"),
        "schema-org": sel(SCHEMA, "?x WHERE { ?x a fo:HowTo } ORDER BY ?x"),
    },
    sel(PKG, "?x WHERE { ?x a fo:Method } ORDER BY ?x"),
    "DUL (dul:Method) and schema.org (schema:HowTo) give a native method category that an FO-"
    "typed query targets directly; the no-FO arm has only the domain class pkg:Technique, with "
    "no foundational method category — so a method introduced under another vocabulary would be "
    "missed. (gUFO has no native method category, so its arm falls back to the domain class — an "
    "honest partial: this task discriminates DUL/schema.org from no-FO, not gUFO.)",
)

# ---------------------------------------------------------------------------
# ER — entailment/reasoning-dependent (answer exists only after closure)
# ---------------------------------------------------------------------------

add(
    "er01", "ER",
    "What work items are blocked by task sq-8thu (its downstream impact), asked directly via "
    "the blocked-by relation?",
    [f"blockedBy-count-{N_BLOCKED_BY_ER01}"], N_BLOCKED_BY_ER01,
    {
        "gufo": count(PKG, "?d) AS ?n) WHERE { ?b dct:identifier \"sq-8thu\" . ?b pkg:blockedBy ?d }"),
        "dolce-dul": count(PKG, "?d) AS ?n) WHERE { ?b dct:identifier \"sq-8thu\" . ?b pkg:blockedBy ?d }"),
        "schema-org": count(PKG, "?d) AS ?n) WHERE { ?b dct:identifier \"sq-8thu\" . ?b pkg:blockedBy ?d }"),
    },
    count(PKG, "?d) AS ?n) WHERE { ?b dct:identifier \"sq-8thu\" . ?b pkg:blockedBy ?d }"),
    "Only pkg:dependsOn is asserted in the data; pkg:blockedBy (its owl:inverseOf) is NEVER "
    "asserted. WITHOUT closure the direct blockedBy query returns 0 rows; the FO arms run "
    "--close owl-rl, which materialises the inverse, so the same question is answerable. This "
    "isolates the REASONING contribution cleanly — the existing canned task-blocks query's "
    "hand-rewrite to dependsOn is itself proof that without inverse entailment you must rewrite "
    "the query (design §5 ER1). It discriminates closed-vs-open, the FO arms' shared lever.",
)

add(
    "er02", "ER",
    "How many distinct entities roll up to the foundational TOP category (everything the "
    "project tracks, regardless of which of the four domain classes it belongs to)?",
    [f"top-{N_TOP}"], N_TOP,
    {
        "gufo": count(GUFO, "?x) AS ?n) WHERE { ?x a fo:Individual }"),
        "dolce-dul": count(DUL, "?x) AS ?n) WHERE { ?x a fo:Entity }"),
        "schema-org": count(SCHEMA, "?x) AS ?n) WHERE { ?x a fo:Thing }"),
    },
    count(PKG, "?x) AS ?n) WHERE { ?x a fo:Top }"),
    "Requires TRANSITIVE subClassOf rollup (rdfs11) through the FO's intermediate categories "
    "(e.g. gufo:Object -> gufo:Endurant -> gufo:ConcreteIndividual -> gufo:Individual) and rdfs9 "
    f"type propagation, so all {N_TOP} entities ({N_TASK} Tasks + {N_FINDING} Findings + "
    f"{N_SOURCE} Sources + {N_TECHNIQUE} Techniques) "
    "land under one top class. The no-FO PKG has no top category at all; without the overlay + "
    "closure the count is 0 (design §5 ER, transitive-closure dependent).",
)

add(
    "er03", "ER",
    "Across all foundational categories, how many entities are information content of any kind "
    "(documents, claims, or methods) — the entailed union, not a hand-listed one?",
    [f"info-union-{N_ARTIFACT}"], N_ARTIFACT,
    {
        "gufo": count(GUFO, "?x) AS ?n) WHERE { { ?x a fo:Object } UNION { ?x a fo:AbstractIndividual } }"),
        "dolce-dul": count(DUL, "?x) AS ?n) WHERE { ?x a fo:SocialObject }"),
        "schema-org": count(SCHEMA, "?x) AS ?n) WHERE { ?x a fo:CreativeWork }"),
    },
    count(PKG, "?x) AS ?n) WHERE { ?x a fo:InformationContentEntity }"),
    "The count form of TH1: a single entailed union over the information-artifact category. "
    f"no-FO returns 0 (no such category); DUL/schema return {N_ARTIFACT} via one entailed class, gUFO via "
    "the Object+AbstractIndividual union it draws (design §5 ER/TH1).",
)

# ---------------------------------------------------------------------------
# CC — cross-category queries (relate across the FO top split)
# ---------------------------------------------------------------------------

add(
    "cc01", "CC",
    "For each information-bearing document that is about a method/technique, list the "
    "(document, method) pair — joining across the information-artifact categories.",
    SKILL_SURFACE_PAIRS, len(SKILL_SURFACE_PAIRS),
    {
        "gufo": sel(GUFO, "?src WHERE { ?src a fo:Object ; dct:subject ?m . ?m a pkg:Technique } ORDER BY ?src"),
        "dolce-dul": sel(DUL, "?src WHERE { ?src a fo:InformationObject ; dct:subject ?m . ?m a fo:Method } ORDER BY ?src"),
        "schema-org": sel(SCHEMA, "?src WHERE { ?src a fo:DigitalDocument ; dct:subject ?m . ?m a fo:HowTo } ORDER BY ?src"),
    },
    sel(PKG, "?src WHERE { ?src a fo:InformationObject ; dct:subject ?m . ?m a fo:Method } ORDER BY ?src"),
    "A type-checked cross-category join: the document side (Source) and the method side "
    "(Technique) must both carry their FO category. The no-FO arm has neither foundational "
    "category, so the typed join is unexpressible without naming the raw domain classes "
    "(design §5 CC1).",
)

add(
    "cc02", "CC",
    "For each occurrent (a tracked unit of work), is there any information artifact (a finding "
    "or document) — i.e. do the process side and the content side coexist in one foundational "
    "frame so a cross-category query can relate them?",
    ["occurrents-and-artifacts-coexist"], 2,
    {
        "gufo": sel(GUFO, "?cat (COUNT(DISTINCT ?x) AS ?n) WHERE { { ?x a fo:Event BIND(\"occurrent\" AS ?cat) } UNION { { ?x a fo:Object } UNION { ?x a fo:AbstractIndividual } BIND(\"artifact\" AS ?cat) } } GROUP BY ?cat ORDER BY ?cat"),
        "dolce-dul": sel(DUL, "?cat (COUNT(DISTINCT ?x) AS ?n) WHERE { { ?x a fo:Event BIND(\"occurrent\" AS ?cat) } UNION { ?x a fo:SocialObject BIND(\"artifact\" AS ?cat) } } GROUP BY ?cat ORDER BY ?cat"),
        "schema-org": None,
    },
    sel(PKG, "?cat ?x WHERE { ?x a fo:Occurrent BIND(\"occurrent\" AS ?cat) } ORDER BY ?cat"),
    "Unifies the process side (Task -> occurrent) and the content side (Finding/Source -> "
    "information artifact) under ONE FO frame so a single query partitions the whole KB by the "
    "top split. no-FO has three unrelated vocab silos with no top join; schema.org lacks the "
    "occurrent axis (only the metaphysical FOs answer the occurrent half) — design §5 CC1.",
)

add(
    "cc03", "CC",
    "Partition the entire knowledge base by the foundational top split: how many entities are "
    "occurrents/events versus information artifacts? (the FO's whole point — one query, two "
    "categories spanning four domain classes)",
    {"event": N_TASK, "artifact": N_ARTIFACT},
    N_TOP,
    {
        "gufo": sel(GUFO, "(COUNT(DISTINCT ?ev) AS ?events) (COUNT(DISTINCT ?art) AS ?artifacts) WHERE { OPTIONAL { ?ev a fo:Event } OPTIONAL { { ?art a fo:Object } UNION { ?art a fo:AbstractIndividual } } }"),
        "dolce-dul": sel(DUL, "(COUNT(DISTINCT ?ev) AS ?events) (COUNT(DISTINCT ?art) AS ?artifacts) WHERE { OPTIONAL { ?ev a fo:Event } OPTIONAL { ?art a fo:SocialObject } }"),
        "schema-org": None,
    },
    sel(PKG, "(COUNT(DISTINCT ?ev) AS ?events) WHERE { ?ev a fo:Occurrent }"),
    "The canonical cross-category partition. Needs BOTH the occurrent category (Tasks) and the "
    "information-artifact category (Finding/Source/Technique) in one frame. no-FO cannot express "
    "either; schema.org can count the artifact union (CreativeWork) but NOT the occurrent half "
    "(no event/occurrent category over Action) — so it is a partial, discriminating against the "
    "metaphysical FOs (design §5 CC).",
)

# ---------------------------------------------------------------------------
# extra TH/CC strata (the corpus ships 16: TH 7, ER 4, CC 5)
# ---------------------------------------------------------------------------

add(
    "th05", "TH",
    "How many concrete endurants (objects that persist through time, as opposed to events or "
    "abstract propositions) does the project track? (the gUFO realist endurant axis)",
    [f"concrete-endurants-{N_ENDURANT}"], N_ENDURANT,
    {
        # gUFO draws a CONCRETE-endurant axis: gufo:Endurant excludes the abstract Findings
        # (gufo:AbstractIndividual) and the Tasks (gufo:Event) — exactly the 11 Source+Technique
        # concrete objects.
        "gufo": count(GUFO, "?x) AS ?n) WHERE { ?x a fo:Endurant }"),
        # HONEST DIVERGENCE: DUL has no class that is exactly "the concrete continuants" — its
        # dul:Object subsumes dul:SocialObject, so ALL information artifacts are dul:Object.
        # schema.org has no endurant axis at all. So this task discriminates the gUFO endurant
        # axis from no-FO; the other FOs draw the line differently (recorded, not forced).
        "dolce-dul": None,
        "schema-org": None,
    },
    count(PKG, "?x) AS ?n) WHERE { ?x a fo:Endurant }"),
    "The concrete-endurant category (Source+Technique) is a gUFO realist distinction: "
    "gufo:Endurant rolls up the concrete objects but not the abstract truth-bearers (Findings) "
    "or the events (Tasks). no-FO has no endurant category. Faithfully gUFO-specific — DUL's "
    "dul:Object and schema.org draw no equivalent concrete/abstract endurant line (design §5 TH2).",
)

add(
    "cc04", "CC",
    "List every finding (claim) together with the topic it is about, where the answer set is "
    "scoped by the foundational claim/proposition category rather than the domain class. "
    "(some findings cover more than one topic, so the (finding, topic) pairs number more than "
    "the distinct findings)",
    FINDINGS, N_FINDING_TOPIC_PAIRS,
    {
        "gufo": sel(GUFO, "?f ?topic WHERE { ?f a fo:AbstractIndividual ; dct:subject ?topic } ORDER BY ?f"),
        "dolce-dul": sel(DUL, "?f ?topic WHERE { ?f a fo:Description ; dct:subject ?topic . FILTER NOT EXISTS { ?f a fo:Method } } ORDER BY ?f"),
        "schema-org": sel(SCHEMA, "?f ?topic WHERE { ?f a fo:Claim ; dct:subject ?topic } ORDER BY ?f"),
    },
    sel(PKG, "?f ?topic WHERE { ?f a fo:Claim ; dct:subject ?topic } ORDER BY ?f"),
    "Scopes by the foundational claim category (gufo:AbstractIndividual / dul:Description-not-"
    "Method / schema:Claim) instead of pkg:Finding, so a claim authored under a different domain "
    "vocabulary would still be found. The no-FO arm has only pkg:Finding; no foundational claim "
    "category exists for it to target — design §5 TH3/CC.",
)

add(
    "th06", "TH",
    "Considering only the foundational types, which sources are documents (information-bearing "
    "objects) — listed via the FO document category, not the domain vocabulary?",
    SOURCES, N_SOURCE,
    {
        "gufo": sel(GUFO, "?x WHERE { ?x a fo:Object . ?x a pkg:Source } ORDER BY ?x"),
        "dolce-dul": sel(DUL, "?x WHERE { ?x a fo:InformationObject } ORDER BY ?x"),
        "schema-org": sel(SCHEMA, "?x WHERE { ?x a fo:DigitalDocument } ORDER BY ?x"),
    },
    sel(PKG, "?x WHERE { ?x a fo:DigitalDocument } ORDER BY ?x"),
    "DUL (dul:InformationObject) and schema.org (schema:DigitalDocument) provide a document "
    "category an FO query targets directly. no-FO has only pkg:Source — design §5 TH3.",
)

add(
    "er04", "ER",
    "Counting via the foundational event category, how many units of work does the project "
    "track in total (the answer must come from the entailed FO type, not the asserted pkg:Task)?",
    [f"events-{N_TASK}"], N_TASK,
    {
        "gufo": count(GUFO, "?x) AS ?n) WHERE { ?x a fo:Event }"),
        "dolce-dul": count(DUL, "?x) AS ?n) WHERE { ?x a fo:Action }"),
        "schema-org": count(SCHEMA, "?x) AS ?n) WHERE { ?x a fo:Action }"),
    },
    count(GUFO, "?x) AS ?n) WHERE { ?x a fo:Event }"),
    "Answer is the entailed FO type (gufo:Event / dul:Action). NOTE: schema:Action is already "
    "asserted as a superclass of pkg:Task in pkg.ttl, so the schema.org arm does NOT discriminate "
    f"from no-FO here (both yield {N_TASK} under closure) — this is an HONEST non-discriminating arm "
    "for schema.org, recorded so the orchestrator does not over-credit it. It discriminates "
    "gUFO/DUL (whose event category is new) from no-FO.",
)

add(
    "cc05", "CC",
    "Group the information artifacts by their foundational sub-category: how many are claims/"
    "propositions, how many are documents, how many are methods?",
    {"claim": N_FINDING, "document": N_SOURCE, "method": N_TECHNIQUE},
    3,
    {
        "gufo": sel(GUFO, "?kind (COUNT(DISTINCT ?x) AS ?n) WHERE { { ?x a fo:AbstractIndividual BIND(\"claim\" AS ?kind) } UNION { ?x a fo:Object . ?x a pkg:Source BIND(\"document\" AS ?kind) } UNION { ?x a fo:Object . ?x a pkg:Technique BIND(\"method\" AS ?kind) } } GROUP BY ?kind ORDER BY ?kind"),
        "dolce-dul": sel(DUL, "?kind (COUNT(DISTINCT ?x) AS ?n) WHERE { { ?x a fo:Description FILTER NOT EXISTS { ?x a fo:Method } BIND(\"claim\" AS ?kind) } UNION { ?x a fo:InformationObject BIND(\"document\" AS ?kind) } UNION { ?x a fo:Method BIND(\"method\" AS ?kind) } } GROUP BY ?kind ORDER BY ?kind"),
        "schema-org": sel(SCHEMA, "?kind (COUNT(DISTINCT ?x) AS ?n) WHERE { { ?x a fo:Claim BIND(\"claim\" AS ?kind) } UNION { ?x a fo:DigitalDocument BIND(\"document\" AS ?kind) } UNION { ?x a fo:HowTo BIND(\"method\" AS ?kind) } } GROUP BY ?kind ORDER BY ?kind"),
    },
    sel(PKG, "?x WHERE { ?x a fo:InformationContentEntity }"),
    "Sub-partitions the information-artifact union by FO sub-category in one query — needs the "
    "whole FO content branch. no-FO has no information-artifact category to sub-partition "
    "(design §5 CC2).",
)

add(
    "th07", "TH",
    "Treating status, exploration-state, and assurance uniformly as foundational categories, "
    "how many distinct entities have any explicitly tracked epistemic/lifecycle state across "
    "ALL four classes? (an FO arm can unify the three state axes; the no-FO arm needs three "
    "separate domain-specific queries)",
    ["cross-class-state-union"], N_STATE_UNION,
    {
        "gufo": count(GUFO, "?x) AS ?n) WHERE { ?x a fo:Individual . { ?x pkg:status ?s } UNION { ?x pkg:exploredStatus ?e } UNION { ?x pkg:assurance ?a } }"),
        "dolce-dul": count(DUL, "?x) AS ?n) WHERE { ?x a fo:Entity . { ?x pkg:status ?s } UNION { ?x pkg:exploredStatus ?e } UNION { ?x pkg:assurance ?a } }"),
        "schema-org": count(SCHEMA, "?x) AS ?n) WHERE { ?x a fo:Thing . { ?x pkg:status ?s } UNION { ?x pkg:exploredStatus ?e } UNION { ?x pkg:assurance ?a } }"),
    },
    count(PKG, "?x) AS ?n) WHERE { ?x a fo:Entity . { ?x pkg:status ?s } UNION { ?x pkg:exploredStatus ?e } UNION { ?x pkg:assurance ?a } }"),
    "Ranges over the FO top category so a SINGLE query spans Task-status + Source-exploredStatus "
    "+ Finding-assurance (three incompatible domain enums). The no-FO arm has no top category to "
    "range over, so it needs three separate queries and a manual union — the FO arm does it in "
    "one (design §5 CC2: rank entities by epistemic status across categories).",
)

assert len({t["id"] for t in TASKS}) == len(TASKS), "duplicate task ids"
kinds = {}
for t in TASKS:
    kinds[t["kind"]] = kinds.get(t["kind"], 0) + 1
print(f"{len(TASKS)} tasks: {kinds}", file=sys.stderr)
assert len(TASKS) >= 16, "need >=16 FO-exercising tasks"

# Cross-check the derived arithmetic against the graph, so a probe that silently changes
# meaning (e.g. a new pkg: class landing under the artifact union) fails LOUDLY here rather
# than shipping a wrong gold answer. These are the identities the FO overlays entail.
assert N_ARTIFACT == N_FINDING + N_SOURCE + N_TECHNIQUE, "artifact union != F+S+T"
assert N_TOP > N_TASK and N_ENDURANT == N_SOURCE + N_TECHNIQUE, "top/endurant identity broken"
assert N_FINDING_TOPIC_PAIRS >= N_FINDING, "(finding, topic) pairs cannot be fewer than findings"

# POWER (pre-registered KILL-A, design §7.1): the FO verdict needs >=30 FO-exercising tasks
# before any `recommend_adopt` is reachable. This corpus is 16, so every run to date is
# UNDERPOWERED BY CONSTRUCTION and may only report an ordering, never an adoption verdict.
# Growing the TH/ER/CC strata to >=30 is a separate phase (design §8 step 2).
KILL_A_MIN_TASKS = 30
if len(TASKS) < KILL_A_MIN_TASKS:
    print(f"  NOTE: {len(TASKS)} tasks < KILL-A floor of {KILL_A_MIN_TASKS} — any run over "
          f"this corpus is UNDERPOWERED; report ordering only, not an adoption verdict.",
          file=sys.stderr)

out = sys.argv[1] if len(sys.argv) > 1 else "bench/fo-km/tasks.jsonl"
with open(out, "w") as f:
    for t in TASKS:
        f.write(json.dumps(t, ensure_ascii=False) + "\n")
print(f"wrote {out}", file=sys.stderr)
