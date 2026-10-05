#!/usr/bin/env python3
# sq-7d3dj.34 — canonical same-box competitor envelope emitter (the committed,
# extended version of the 2026-07-07 ad-hoc emitter that produced
# bench/canonical-competitor-results/2026-07-07/). Assembles ONE envelope per suite from
# the per-engine TSVs + a meta.json the bash orchestrator wrote (engine versions/modes,
# load timings, statuses, env, expected-rows, and optional wave/tsv_format/connection
# metadata for the HTTP/TTFB panel).
#
# Usage: emit_envelope.py <work-dir> <out.json>
#   <work-dir> contains: sparq.tsv oxigraph.tsv fuseki.tsv virtuoso.tsv qlever.tsv meta.json
#   meta.json = {suite, scale, iters, git_commit, canonical, engines{}, statuses{},
#                load{}, env{}, expected{query:rows} [, wave, tsv_format, connection,
#                canonical_note]}
#
# TSV line formats (both accepted; the count crosscheck only reads cols 1-2):
#   3-col: `<name>\t<rows|ERROR>\t<best_us|engine>`
#   6-col: `<name>\t<rows|ERROR>\t<ka_best_us>\t<ka_ttfb_us>\t<fresh_best_us>\t<fresh_ttfb_us>`
import json
import os
import sys

ENGINE_ORDER = ["sparq", "oxigraph", "fuseki", "virtuoso", "qlever"]

work, out = sys.argv[1], sys.argv[2]
meta = json.load(open(os.path.join(work, "meta.json")))


def read_tsv(engine):
    p = os.path.join(work, f"{engine}.tsv")
    raw = ""
    rows = {}
    if os.path.exists(p):
        raw = open(p, encoding="utf-8").read()
        for line in raw.splitlines():
            if not line.strip():
                continue
            parts = line.split("\t")
            if len(parts) < 3:
                continue
            rows[parts[0]] = (parts[1], parts[2])
    return raw, rows


tsvs = {}
parsed = {}
for e in ENGINE_ORDER:
    raw, rows = read_tsv(e)
    tsvs[e] = raw
    parsed[e] = rows

# union of query names, sorted
qnames = sorted({q for e in ENGINE_ORDER for q in parsed[e].keys()})

expected = meta.get("expected", {})
crosscheck = {}
for q in qnames:
    entry = {}
    counts = []
    for e in ENGINE_ORDER:
        if q in parsed[e]:
            rc = parsed[e][q][0]
            entry[e] = rc
            if rc != "ERROR":
                counts.append(rc)
        else:
            entry[e] = "n/a"
    # all non-ERROR engine counts equal?
    entry["all_agree"] = (len(set(counts)) <= 1) and len(counts) >= 2
    if q in expected:
        entry["expected"] = expected[q]
        entry["matches_expected"] = (
            all(c == str(expected[q]) for c in counts) if counts else False
        )
    crosscheck[q] = entry

env = dict(meta.get("env", {}))

envelope = {
    "gather": "sparql-same-box-comparison",
    "wave": meta.get("wave", "canonical-competitor-matrix (sq-1sa9r)"),
    "canonical": meta.get("canonical", True),
    "canonical_note": meta.get(
        "canonical_note",
        "CANONICAL: dedicated QUIET c6i.4xlarge (16 vCPU / 32 GiB, x86_64) in eu-west-2, "
        "single-tenant, one engine active at a time on the SAME generated corpus + SAME "
        "query files. min-of-{} on a loaded store; per-query timeout recorded as honest "
        "'timeout'/ERROR, never fabricated. Counts cross-checked engine-vs-engine AND vs "
        "bench/<suite>/expected-rows.tsv where present.".format(meta.get("iters")),
    ),
    "git_commit": meta.get("git_commit"),
    "suite": meta.get("suite"),
    "scale": meta.get("scale"),
    "iters": meta.get("iters"),
    "tsv_format": meta.get("tsv_format", "<query>\\t<rows|ERROR>\\t<best_us|engine>"),
    "engines": meta.get("engines", {}),
    "load": meta.get("load", {}),
    "statuses": meta.get("statuses", {}),
    "count_crosscheck": crosscheck,
    "count_crosscheck_note": (
        "all_agree = every engine that produced a (non-ERROR) count agrees. matches_expected "
        "= those counts equal bench/<suite>/expected-rows.tsv. A disagreement is recorded "
        "HONESTLY (never adjusted); this is why COUNT is checked before trusting any timing."
    ),
    "env": env,
    "sparq_tsv": tsvs["sparq"],
    "oxigraph_tsv": tsvs["oxigraph"],
    "fuseki_tsv": tsvs["fuseki"],
    "virtuoso_tsv": tsvs["virtuoso"],
    "qlever_tsv": tsvs["qlever"],
}
# HTTP/TTFB-panel metadata (present only when the orchestrator recorded it): the explicit
# keep-alive-vs-fresh-connect note the D9 gap row asked for.
if "connection" in meta:
    envelope["connection"] = meta["connection"]

with open(out, "w", encoding="utf-8") as f:
    json.dump(envelope, f, indent=2)
    f.write("\n")
print(f"wrote {out}")
