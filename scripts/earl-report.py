#!/usr/bin/env python3
"""Turn per-test outcome lines into an EARL implementation report (Turtle).

Conformance runners write one `<test IRI>\\t<passed|failed>` line per manifest
entry (for SHACL: `SPARQ_EARL_OUT`, see crates/sparq-shacl/earl-report.sh). This
script rewrites the local test IRIs to the IRIs the W3C test suite publishes
(`--map LOCAL_PREFIX=PUBLISHED_PREFIX`), and emits an EARL 1.0 report with a DOAP
description of sparq, in the shape W3C working groups collect under their
`reports/` folders.

It fails on an IRI that no `--map` prefix covers, on a test reported twice with
different outcomes, and on an empty input, so a broken run cannot produce a
plausible-looking report.
"""

import argparse
import datetime
import sys

PREFIXES = """@prefix dc: <http://purl.org/dc/terms/> .
@prefix doap: <http://usefulinc.com/ns/doap#> .
@prefix earl: <http://www.w3.org/ns/earl#> .
@prefix foaf: <http://xmlns.com/foaf/0.1/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
"""


def literal(text):
    escaped = text.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n")
    return f'"{escaped}"'


def iri(text):
    if any(c in text for c in '<>"{}|^`\\ \n'):
        sys.exit(f"earl-report: not an IRI: {text!r}")
    return f"<{text}>"


def read_outcomes(paths, maps):
    outcomes = {}
    for path in paths:
        with open(path, encoding="utf-8") as f:
            for n, line in enumerate(f, 1):
                line = line.rstrip("\n")
                if not line:
                    continue
                test, _, outcome = line.partition("\t")
                if outcome not in ("passed", "failed"):
                    sys.exit(f"earl-report: {path}:{n}: bad outcome {outcome!r}")
                published = next(
                    (to + test[len(frm):] for frm, to in maps if test.startswith(frm)),
                    None,
                )
                if published is None:
                    sys.exit(f"earl-report: {path}:{n}: no --map prefix covers {test}")
                if outcomes.get(published, outcome) != outcome:
                    sys.exit(f"earl-report: {published} reported both passed and failed")
                outcomes[published] = outcome
    if not outcomes:
        sys.exit("earl-report: no outcomes in the input")
    return outcomes


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("inputs", nargs="+", help="outcome files (test IRI TAB outcome)")
    ap.add_argument("--map", action="append", default=[], required=True,
                    metavar="LOCAL=PUBLISHED", help="test IRI prefix rewrite")
    ap.add_argument("--subject", default="https://github.com/sparq-org/sparq")
    ap.add_argument("--name", required=True, help="doap:name, with the version")
    ap.add_argument("--revision", required=True, help="doap:revision of the release")
    ap.add_argument("--description", required=True)
    ap.add_argument("--assertor", default="https://github.com/sparq-org")
    ap.add_argument("--assertor-name", default="sparq contributors")
    ap.add_argument("--output", required=True)
    args = ap.parse_args()

    maps = []
    for m in args.map:
        frm, sep, to = m.partition("=")
        if not sep:
            sys.exit(f"earl-report: --map needs LOCAL=PUBLISHED, got {m!r}")
        maps.append((frm, to))
    outcomes = read_outcomes(args.inputs, maps)

    now = datetime.datetime.now(datetime.timezone.utc).replace(microsecond=0).isoformat()
    date = f'"{now}"^^xsd:dateTime'
    subject, assertor = iri(args.subject), iri(args.assertor)
    out = [PREFIXES]
    out.append(f"""<> foaf:primaryTopic {subject} ;
  dc:issued {date} ;
  foaf:maker {assertor} .

{subject} a doap:Project, earl:TestSubject, earl:Software ;
  doap:name {literal(args.name)} ;
  doap:description {literal(args.description)} ;
  doap:release [ doap:name {literal(args.name)} ; doap:revision {literal(args.revision)} ] ;
  doap:homepage <https://sparq-org.github.io/sparq/> ;
  doap:repository <https://github.com/sparq-org/sparq> ;
  doap:bug-database <https://github.com/sparq-org/sparq/issues> ;
  doap:license <https://spdx.org/licenses/MIT> ;
  doap:programming-language "Rust" ;
  doap:developer {assertor} .

{assertor} a earl:Assertor, foaf:Organization ;
  foaf:name {literal(args.assertor_name)} ;
  foaf:homepage {assertor} .
""")
    for test in sorted(outcomes):
        out.append(f"""[ a earl:Assertion ;
  earl:assertedBy {assertor} ;
  earl:subject {subject} ;
  earl:test {iri(test)} ;
  earl:mode earl:automatic ;
  earl:result [ a earl:TestResult ; earl:outcome earl:{outcomes[test]} ; dc:date {date} ]
] .
""")
    with open(args.output, "w", encoding="utf-8") as f:
        f.write("\n".join(out))
    passed = sum(1 for o in outcomes.values() if o == "passed")
    print(f"earl-report: {args.output}: {passed} passed, {len(outcomes) - passed} failed")


if __name__ == "__main__":
    main()
