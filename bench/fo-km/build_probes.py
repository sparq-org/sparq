#!/usr/bin/env python3
# [OPUS-5] sq-mztg8.3 (FO-KM epic; design research/fo-llm-bridge.md §4.2 "Metric 3" +
# §6 Phase 6). 🤖 SPARQ agent — the authoring script for the Metric-3 probe battery.
#
# WHAT THIS IS
# --------------------------------------------------------------------------------
# Metric 3 of the FO-KM programme is the **Köhler-Neuhaus ontological-commitment
# stability probe** ("The Mercurial Top-Level Ontology of LLMs", FOIS 2024,
# arXiv:2405.01581): an LLM asked the SAME top-level categorial question in separate
# sessions does not reliably give the same answer, because ontology terms have fixed
# compositional semantics while an LLM emits tokens stochastically by context. The
# original study is scoped to GPT-3.5, so — per the design record — it is RE-RUN PER
# MODEL rather than assumed to carry over.
#
# The FO-KM adaptation asks the epic's question: **does the FO choice change that
# instability?** Same battery, same model, three arms:
#
#   ungrounded  — no ontology in context (the control: the model's own implicit
#                 top-level ontology, which is what Köhler-Neuhaus measured)
#   gufo        — the gUFO top-level fragment in context (a rich FO that DOES draw
#                 the occurrent/continuant/abstract axis)
#   schema-org  — the schema.org fragment in context (the ratified fluent facade,
#                 which does NOT draw that axis — schema:Action and
#                 schema:CreativeWork are sibling branches under schema:Thing)
#
# This is deliberately a PROMPT-LEVEL scaffold, not a `pkg-query` run. Metric 1 measures
# how well an agent WIELDS an FO through the tool; Metric 3 measures whether an explicit
# FO STABILISES the model's categorial commitments. The tool is not part of that
# construct, and keeping it out removes tool-variance from the measurement.
#
# THE BATTERY (12 probes, two strata)
#   SC (scaffolded, 8) — subjects drawn from the four PKG classes the overlays type
#       (Task / Finding / Source / Technique), each as a GENERIC probe and an INSTANCE
#       probe over a real `crates/sparq-kb/ingest/pkg-instances.ttl` individual. The
#       generic/instance probes form four `pair` links: a session that puts a kind in
#       one category and a member of that kind in another has contradicted ITSELF
#       within the session (the WS measure in stability_analyze.py).
#   US (unscaffolded, 4) — subjects NO overlay types. These measure the model's own
#       implicit top-level ontology and whether an in-context FO TRANSFERS beyond the
#       entities it covers. This stratum is the interesting one: the SC stratum can be
#       partly solved by reading the scaffold, the US stratum cannot.
#
# Every probe is a FORCED CHOICE over a closed four-label set, so grading is
# deterministic with NO model in the loop (the same anti-circularity rule as Metric 1's
# analyze.py, design §5.2).
#
# `fo_label` records what each ARM's own overlay entails, so the analyzer can report
# scaffold adherence as well as stability. It is NOT a claim about which FO is right:
#   * gufo       — from overlays/gufo.ttl's asserted mapping (Task→gufo:Event;
#                  Finding→gufo:AbstractIndividual; Source/Technique→gufo:Object).
#   * schema-org — UNDECIDABLE on every probe, and that is the CORRECT entailment, not
#                  a gap in the fixture: overlays/schema-org.ttl documents in its own
#                  header that schema.org has no endurant/perdurant axis, so the
#                  fragment cannot decide an occurrent/continuant question. An arm can
#                  therefore look perfectly "stable" by declining to commit — which is
#                  exactly why stability_analyze.py reports DECISIVENESS alongside the
#                  contradiction rate.
#   * US probes  — null for every arm (no overlay types these subjects).
#
# NOT INCLUDED, and why (honest scope): the DOLCE-DUL and no-FO overlays are not run as
# Metric-3 arms in the pilot. no-FO is definitionally the same context as `ungrounded`
# for a prompt-level scaffold (its overlay asserts no FO typing), so it would be a
# duplicate control. DOLCE-DUL needs a considered mapping decision first — `dul:Description`
# (the overlay's target for pkg:Finding) is a non-physical ENDURANT in DUL, so it maps to
# neither CONTINUANT nor ABSTRACT cleanly under this label set, and guessing would put a
# fabricated gold into the fixture. See STABILITY.md § Open questions.
#
# Usage:
#   python3 bench/fo-km/build_probes.py                     # regenerate the .jsonl
#   python3 bench/fo-km/build_probes.py --check             # verify the .jsonl matches
#   python3 bench/fo-km/build_probes.py --emit-prompt gufo  # the EXACT session brief

from __future__ import annotations

import argparse
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
PROBES_PATH = os.path.join(HERE, "stability_probes.jsonl")

# The closed answer vocabulary. Four labels, one of which is an explicit "the ontology
# you were given does not decide this" — without it a scaffold that genuinely cannot
# answer would be forced into a fabricated commitment.
LABELS = ("OCCURRENT", "CONTINUANT", "ABSTRACT", "UNDECIDABLE")

LABEL_GLOSS = {
    "OCCURRENT": "something that happens or unfolds in time; it has temporal parts, and "
                 "only part of it is present at any given moment",
    "CONTINUANT": "a concrete thing that persists through time; it is wholly present "
                  "whenever it exists, and it has no temporal parts",
    "ABSTRACT": "an abstract entity outside space and time — a proposition, a "
                "truth-bearer, a piece of information content, a type or a plan",
    "UNDECIDABLE": "the categories you have been given do not decide this question",
}

# --------------------------------------------------------------------------------
# the battery
# --------------------------------------------------------------------------------
# `subject` is the exact NL text presented to the session. Instance probes name a real
# individual from crates/sparq-kb/ingest/pkg-instances.ttl (verified present, with its
# committed dcterms:title / rdfs:label / skos:prefLabel) so the probe is grounded in the
# PKG rather than invented.
PROBES: list[dict] = [
    # --- SC: pkg:Task -> gufo:Event ------------------------------------------------
    {
        "id": "p01", "kind": "SC", "level": "generic", "pkg_class": "Task", "pair": "p02",
        "subject": "a TASK in this project's knowledge base: a unit of project work that "
                   "is opened, worked on by someone, and then closed — for example "
                   "\"add a gist overlay to the FO-KM benchmark\"",
        "fo_label": {"gufo": "OCCURRENT", "schema-org": "UNDECIDABLE"},
    },
    {
        "id": "p02", "kind": "SC", "level": "instance", "pkg_class": "Task", "pair": "p01",
        "subject": "the one particular task sq-01yr, titled \"sparq-core load_dataset_serial: "
                   "TriG catch-all silent-fallback\", whose status is now closed",
        "fo_label": {"gufo": "OCCURRENT", "schema-org": "UNDECIDABLE"},
    },
    # --- SC: pkg:Finding -> gufo:AbstractIndividual --------------------------------
    {
        "id": "p03", "kind": "SC", "level": "generic", "pkg_class": "Finding", "pair": "p04",
        "subject": "a FINDING in this project's knowledge base: a recorded claim about how "
                   "the project works, carrying a verdict, a justification, and a numeric "
                   "confidence",
        "fo_label": {"gufo": "ABSTRACT", "schema-org": "UNDECIDABLE"},
    },
    {
        "id": "p04", "kind": "SC", "level": "instance", "pkg_class": "Finding", "pair": "p03",
        "subject": "the one particular finding that \"a pull request merges only when the "
                   "ci-summary check is green and every review thread is resolved\"",
        "fo_label": {"gufo": "ABSTRACT", "schema-org": "UNDECIDABLE"},
    },
    # --- SC: pkg:Source -> gufo:Object ---------------------------------------------
    {
        "id": "p05", "kind": "SC", "level": "generic", "pkg_class": "Source", "pair": "p06",
        "subject": "a SOURCE in this project's knowledge base: a documentary source of "
                   "project knowledge — a design record, a specification, or a paper",
        "fo_label": {"gufo": "CONTINUANT", "schema-org": "UNDECIDABLE"},
    },
    {
        "id": "p06", "kind": "SC", "level": "instance", "pkg_class": "Source", "pair": "p05",
        "subject": "the one particular source AGENTS.md, titled \"the sparq agent charter "
                   "(how we work)\", a markdown file in the repository",
        "fo_label": {"gufo": "CONTINUANT", "schema-org": "UNDECIDABLE"},
    },
    # --- SC: pkg:Technique -> gufo:Object ------------------------------------------
    {
        "id": "p07", "kind": "SC", "level": "generic", "pkg_class": "Technique", "pair": "p08",
        "subject": "a TECHNIQUE in this project's knowledge base: an algorithm or method "
                   "the project knows about and has written down",
        "fo_label": {"gufo": "CONTINUANT", "schema-org": "UNDECIDABLE"},
    },
    {
        "id": "p08", "kind": "SC", "level": "instance", "pkg_class": "Technique", "pair": "p07",
        "subject": "the one particular technique recorded as the \"sparql-query surface\" — "
                   "running SPARQL 1.1 queries against this project's engine",
        "fo_label": {"gufo": "CONTINUANT", "schema-org": "UNDECIDABLE"},
    },
    # --- US: subjects no overlay types ---------------------------------------------
    # Chosen as the classic hard cases on the axis: each has a defensible reading on
    # BOTH sides, which is precisely what makes a stable answer informative.
    {
        "id": "p09", "kind": "US", "level": "generic", "pkg_class": None, "pair": None,
        "subject": "a merged pull request in this repository — the change \"add a gist "
                   "overlay\", which was proposed, reviewed, and merged last week",
        "fo_label": {"gufo": None, "schema-org": None},
    },
    {
        "id": "p10", "kind": "US", "level": "generic", "pkg_class": None, "pair": None,
        "subject": "a continuous-integration gate: the named check that must be green "
                   "before any pull request is allowed to merge",
        "fo_label": {"gufo": None, "schema-org": None},
    },
    {
        "id": "p11", "kind": "US", "level": "generic", "pkg_class": None, "pair": None,
        "subject": "the SPARQL 1.1 Query Language standard itself, as distinct from any "
                   "particular copy of the specification document",
        "fo_label": {"gufo": None, "schema-org": None},
    },
    {
        "id": "p12", "kind": "US", "level": "instance", "pkg_class": None, "pair": None,
        "subject": "the benchmark run that took place on the build machine yesterday and "
                   "produced a table of results",
        "fo_label": {"gufo": None, "schema-org": None},
    },
]

ARMS = ("ungrounded", "gufo", "schema-org")

# --------------------------------------------------------------------------------
# the per-arm scaffold text
# --------------------------------------------------------------------------------
# The scaffold gives the FO's TOP-LEVEL TAXONOMY and definitions but deliberately does
# NOT give the PKG-class -> FO-category mapping the overlay asserts. If it did, the SC
# stratum would collapse into a lookup and would measure reading comprehension rather
# than ontological commitment. Withholding the mapping means the model must ground each
# subject onto the fragment itself — the same introspect->ground step Metric 1 measures —
# and `fo_label` (the overlay's own asserted mapping) becomes a meaningful adherence
# target rather than a restatement of the prompt.
#
# Both fragments are transcribed from the committed overlays in overlays/, so the arms
# are the SAME ontologies Metric 1 ran, not paraphrases.
SCAFFOLDS = {
    "ungrounded": "",
    "gufo": """You must answer using the categories of the following foundational ontology
(gUFO — "A Gentle Foundational Ontology for Semantic Web Knowledge Graphs";
namespace http://purl.org/nemo/gufo#). This is its top-level taxonomy:

  gufo:Individual
    |- gufo:ConcreteIndividual        an individual that exists in space and time
    |    |- gufo:Endurant             persists in time, wholly present whenever it exists
    |    |    |- gufo:Object          an endurant that exists independently
    |    |- gufo:Event                unfolds in time, accumulating temporal parts
    |- gufo:AbstractIndividual        an individual outside space and time (a
                                      proposition, a truth-bearer, information content)

Map the gUFO category you choose onto the answer labels as:
  gufo:Event -> OCCURRENT | gufo:Object or gufo:Endurant -> CONTINUANT |
  gufo:AbstractIndividual -> ABSTRACT
If this taxonomy does not decide a question, answer UNDECIDABLE.
""",
    "schema-org": """You must answer using the categories of the following vocabulary
(schema.org; namespace http://schema.org/). This is the relevant fragment of its
class taxonomy:

  schema:Thing
    |- schema:Action                  an action performed by a direct agent
    |- schema:CreativeWork            the most generic kind of creative work
         |- schema:Claim              a specific, factually-oriented claim
         |- schema:DigitalDocument    an electronic file or document
         |- schema:HowTo              instructions that explain how to achieve a result

Map the schema.org category you choose onto the answer labels as:
  a category this vocabulary places among things that happen or unfold in time
    -> OCCURRENT
  a category it places among concrete things that persist through time -> CONTINUANT
  a category it places among abstract truth-bearers or information content -> ABSTRACT
If this taxonomy does not decide a question, answer UNDECIDABLE.
""",
}

PREAMBLE = """You are being asked a series of top-level ontology questions. For each
subject below, decide which ONE of these four categories it falls into:

{labels}

{scaffold}Answer from your own judgement in a single pass. Do NOT use any tool, do NOT
read any file, and do NOT search. There is no repository to consult.

The subjects:

{subjects}

OUTPUT FORMAT — this is strict. Reply with exactly {n} lines and nothing else. Each line
must be the probe id, a colon, a space, and one label:

{example}

No preamble, no explanation, no blank lines, no markdown. Just the {n} lines."""


def render_session_prompt(arm: str, probes: list[dict] | None = None) -> str:
    """The EXACT brief handed to one fresh session of one (model, arm) cell.

    Identical for every session of a cell — probe ORDER is held fixed on purpose, so
    that any cross-session disagreement is attributable to sampling rather than to a
    permuted presentation order (see STABILITY.md § Method).
    """
    if arm not in SCAFFOLDS:
        raise SystemExit(f"unknown arm {arm!r}; expected one of {', '.join(ARMS)}")
    probes = probes if probes is not None else PROBES
    labels = "\n".join(f"  {lab} - {LABEL_GLOSS[lab]}" for lab in LABELS)
    subjects = "\n\n".join(f"  {p['id']}: {p['subject']}" for p in probes)
    scaffold = (SCAFFOLDS[arm] + "\n") if SCAFFOLDS[arm] else ""
    example = "\n".join(f"{p['id']}: <LABEL>" for p in probes[:2]) + "\n..."
    return PREAMBLE.format(labels=labels, scaffold=scaffold, subjects=subjects,
                           n=len(probes), example=example)


def serialise(probes: list[dict]) -> str:
    return "".join(json.dumps(p, ensure_ascii=False) + "\n" for p in probes)


def validate(probes: list[dict]) -> None:
    """Structural invariants of the battery — cheap, and they catch fixture rot."""
    ids = [p["id"] for p in probes]
    if len(set(ids)) != len(ids):
        raise SystemExit("duplicate probe id")
    by_id = {p["id"]: p for p in probes}
    for p in probes:
        for arm, lab in p["fo_label"].items():
            if arm not in ARMS:
                raise SystemExit(f"{p['id']}: fo_label names unknown arm {arm!r}")
            if lab is not None and lab not in LABELS:
                raise SystemExit(f"{p['id']}: fo_label {lab!r} outside the label set")
        # A pair link must be symmetric, cross-level, and within one PKG class —
        # otherwise the within-session consistency check is comparing unlike things.
        if p["pair"] is not None:
            other = by_id.get(p["pair"])
            if other is None or other["pair"] != p["id"]:
                raise SystemExit(f"{p['id']}: pair link is not symmetric")
            if other["pkg_class"] != p["pkg_class"]:
                raise SystemExit(f"{p['id']}: pair crosses PKG classes")
            if other["level"] == p["level"]:
                raise SystemExit(f"{p['id']}: pair is not generic<->instance")
            # A generic/instance pair must AGREE per arm, or the WS measure would flag
            # the fixture's own disagreement as a model contradiction.
            if other["fo_label"] != p["fo_label"]:
                raise SystemExit(f"{p['id']}: paired probes disagree on fo_label")
        if p["kind"] == "US" and any(v is not None for v in p["fo_label"].values()):
            raise SystemExit(f"{p['id']}: US probe must have no FO-entailed label")


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description="FO-KM Metric-3 probe-battery author")
    ap.add_argument("--check", action="store_true",
                    help="verify stability_probes.jsonl matches this script (no write)")
    ap.add_argument("--emit-prompt", metavar="ARM", choices=ARMS,
                    help="print the exact per-session brief for one arm and exit")
    args = ap.parse_args(argv[1:])

    validate(PROBES)

    if args.emit_prompt:
        print(render_session_prompt(args.emit_prompt))
        return 0

    text = serialise(PROBES)
    if args.check:
        on_disk = open(PROBES_PATH, encoding="utf-8").read() if os.path.exists(PROBES_PATH) else ""
        if on_disk != text:
            print(f"DRIFT: {PROBES_PATH} does not match build_probes.py — rerun without --check")
            return 1
        print(f"ok: {PROBES_PATH} matches ({len(PROBES)} probes)")
        return 0

    with open(PROBES_PATH, "w", encoding="utf-8") as fh:
        fh.write(text)
    kinds: dict[str, int] = {}
    for p in PROBES:
        kinds[p["kind"]] = kinds.get(p["kind"], 0) + 1
    pairs = sum(1 for p in PROBES if p["pair"]) // 2
    print(f"wrote {PROBES_PATH}: {len(PROBES)} probes {kinds}, {pairs} generic/instance pairs")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
