#!/usr/bin/env python3
# [OPUS-5] Orchestration automation — the `needs:area` BACKLOG CLASSIFIER.
#
# triage-area.py   (dry-run by DEFAULT; `--apply` is the only mutating mode)
#
# WHY THIS EXISTS
# ---------------
# `scripts/triage.py` refuses to promote an issue that carries no `area:<crate>`
# label: a no-area issue reserves the readiness engine's serializing `__global__`
# partition, so at backlog scale every no-area `status:ready` issue collapses the
# dispatch frontier to one worker. Such an issue is parked `needs:area` instead —
# correct, but terminal: `retriage.py` only emits PROMOTING deltas, so nothing ever
# clears the park on its own. The 2026-07-25 bd->issues migration parked 257 open
# issues that way (20 of them P1), i.e. 257 items that cannot be dispatched at all.
#
# `bd-to-issues.derive_areas()` is the migration-time deriver, and these 257 are
# exactly its residue: it only reads a conventional `verb(scope):` title prefix, a
# crate token in the TITLE, a surface keyword in the title's scope region, and a
# single-crate description. sparq's real backlog does not talk that way — it names
# `rl.rs`, `zk/xpath`, `DL L4`, `noir upstream`, `perf-gate`, `nightly sweep`. So
# this script adds the evidence tiers that residue actually carries.
#
# EVIDENCE TIERS (most authoritative first; FIRST match wins)
#   T0  an EXPLICIT author declaration in the body — the `crate_or_surface:` /
#       `crates:` / `Crate:` field the decomposition templates emit. Nothing beats
#       the author naming the package.
#   T1  an ordered table of REPO-PATH + TOPIC rules (RULES below). Each rule is a
#       narrow regex plus the areas it implies plus the evidence that justifies it.
#       Narrow on purpose: a rule that could fire on two unrelated families is a
#       misroute waiting to happen.
#   T2  `bd-to-issues.derive_areas()` — the migration deriver, reused verbatim so a
#       freshly-migrated issue is classified the same way twice.
#
# FAIL CLOSED. When no tier produces an area the issue KEEPS `needs:area`. That is
# the correct outcome, not a failure: a wrong `area:` routes a worker at the wrong
# crate and can put two workers on one conflict partition, whereas the park is
# maintainer-visible and `retriage.py` promotes the moment an area lands.
#
# NEVER INVENT A LABEL. Every produced area is checked against the repo's LIVE
# label list before anything is written; an unknown area is a hard error, not a
# `gh label create`.
#
# IDEMPOTENT + RE-RUNNABLE. An issue that already carries an `area:` is skipped
# (its area is the maintainer's/author's, not ours). `needs:area` is removed ONLY
# in the same call that adds >=1 real `area:` label, so the two never drift.
#
# REACH: EVERY AREA-LESS OPEN ISSUE, NOT JUST THE PARKED ONES (#5003). This used to
# fetch `--label needs:area`, i.e. only the issues `triage.py` / `bd-to-issues.py`
# had parked. But the park is one SOURCE of area-less issues, not the class: an
# issue the event triager never ran on carries no `status:*` label and no park at
# all, and is just as area-less and just as undispatchable. #3816 measured 832 open
# issues with no `area:` label against a park population an order of magnitude
# smaller. A label query cannot return the class the sweep exists to rescue — the
# same structural hole `retriage.py::_fetch_candidates` was widened to close — so
# the queue is now ONE unlabelled open-issue snapshot filtered by the absence of an
# `area:` label. Widening the REACH does not widen what is WRITTEN: `classify()` is
# still fail-closed, and an issue that was never parked can only ever GAIN `area:`
# labels — `apply_row(unpark=...)` never removes a park it did not have.
#
#   python3 scripts/triage-area.py --self-test    # offline rule unit tests
#   python3 scripts/triage-area.py                # dry run: full proposed mapping
#   python3 scripts/triage-area.py --apply        # apply (add area:*; drop needs:area
#                                                 #  only where it was actually set)
#   python3 scripts/triage-area.py --census       # read-only: count the unattributed class
#
# Maintainer authorisation for the automated pass: sparq-org/sparq#1135 (2026-07-26).
#
# WHO RUNS IT. `.github/workflows/triage-area.yml` runs `--apply` on a half-hourly cron
# (#3816). Until that lane existed this script was hand-invoked only, which made the
# `needs:area` park TERMINAL in automation: `retriage.py` emits PROMOTING deltas and so
# structurally cannot lift the park, and every other consumer reads the missing `area:` as
# a reason to skip the issue. The cron re-runs the self-test AND
# scripts/tests/test_triage_area.py before it writes anything, and always reports the
# left-parked residue to the run summary — the issues no rule can attribute are a visible
# count for a human, not silence.
#
# THE PARK IS NOT THE WHOLE CLASS (#5004). That residue report is built on the
# `--label needs:area` query, so it describes only issues that already carry the park. Per
# #5004 — a claim about the private registry, not checkable from here — its
# `curate-frontier.py` skips on a WIDER predicate, any issue whose area it cannot derive,
# and a no-area issue that was never parked belongs to neither report.
# `--census` computes that wider predicate here and the cron always prints it; see the
# block above `open_issues()` for why the two sets diverge and why the census only counts.
import argparse
import json
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

REPO = os.environ.get("SPARQ_REPO", "sparq-org/sparq")
PARK_LABEL = "needs:area"

# --- T1: the rule table ---------------------------------------------------------
# (rule_id, scope, regex, areas, evidence).  `scope` is "title" or "text"
# (title+body).  Prefer "title" — a body mentions neighbouring crates constantly and
# matching it is how `derive_areas` mislabeled a zkSPARQL spec bead `area:site` off
# one incidental word.  FIRST MATCH WINS, so order is load-bearing: the narrow,
# family-specific rules come before the broad surface rules.
RULES = [
    # -- workflow lanes that merely *mention* another surface ---------------------
    # These must precede the surface rules they name, else the named surface wins.
    ("zk-toolchain-lane", "title", r"into the zk-toolchain\.yml lane",
     ["ci"], "adds a step to .github/workflows/zk-toolchain.yml"),

    # -- external repositories (work does not land in this tree) -----------------
    ("noir-upstream", "title", r"\bnoir upstream:",
     ["upstream"], "PR against noir-lang/noir"),
    ("noir-paper-p1", "title", r"noir-opt paper p1\b",
     ["bench"], "commits a reproducible measurement harness"),
    ("noir-paper-p3", "title", r"noir-opt paper p3\b",
     ["upstream"], "code-level pass over unexamined noir compiler stages"),
    ("noir-paper", "title", r"noir-opt paper p[245]\b|paper: optimising noir",
     ["site-papers"], "paper-factory deliverable"),
    ("rdf-shuttle-upstream", "title",
     r"upstream rdf-shuttle|gen-rs backend gaps|shuttle generate-mode",
     ["upstream"], "jeswr/rdf-shuttle"),
    ("shaclcjs-upstream", "title", r"upstream hygiene: shaclcjs",
     ["upstream"], "jeswr/shaclcjs + jeswr/shaclc-1.2"),
    ("hdt-upstream", "title", r"upstream hdt pr",
     ["upstream"], "KonradHoeffner/hdt"),
    ("n3-cg-upstream", "title", r"open w3c/n3 issue",
     ["upstream"], "w3c/N3 community group"),
    ("rdfjs-upstream", "title", r"propose @rdfjs-test/conformance",
     ["upstream"], "contribution to the rdfjs/ org"),
    ("spec-upstream", "title",
     r"spec publish:|spec tests: contribute|contribute the solid\+sparql spec drafts",
     ["upstream"], "publishes into a maintainer/w3id-controlled namespace"),

    # -- documentation-only work -------------------------------------------------
    # EARLY on purpose: a doc-sync bead names the surface it is documenting, so a
    # later surface rule would claim it. `Doc-sync: AGENTS.md noir_XPath version
    # stale` edits AGENTS.md, not zk/xpath.
    ("docs", "title",
     r"^sq-\w+(\.\w+)*: doc-sync:|doc honesty|^docs:|docs website|"
     r"docs-site jscpd|^sq-\w+: research: append|reconcile research/|"
     r"migrate docs/adrs|compliance/sbom/controls|^sq-\w+: codify ",
     ["docs"], "documentation-only change"),

    # -- surfaces that are frequently NAME-DROPPED by neighbouring beads ---------
    # Ordered ahead of the spec/paper/zk rules: a paper's Limitations section names
    # the crate it limits, and an ODRL bead names the paper that surfaced it.
    # sq-wvne is a trust-graph REQUIREMENT whose three pieces are all circuits:
    # its body names sparq-zk-compose/src/issuer.rs, verifier.rs and
    # tests/holder_pok_binding.rs. Labelling it trust-only would send a worker to
    # a crate that holds none of the code it must change.
    ("trust-zk-composite", "title", r"zkaps-grade unlinkable presentation",
     ["sparq-trust", "sparq-zk-compose"],
     "the composite is built in sparq-zk-compose; the requirement is the trust graph's"),
    ("trust-graph", "title",
     r"solid-authz-trust|trust-graph|pfae\.|live-status gate",
     ["sparq-trust"], "the solid trust-graph / admission surface"),
    ("acbench", "title", r"acbench",
     ["sparq-acbench"], "crates/sparq-acbench"),
    ("pkg-fo-bench", "title", r"fo benchmark round|gufo closure-prior",
     ["knowledge-graph", "bench"], "FO-KM measurement under bench/fo-km"),
    ("pkg", "title",
     r"\bpkg\b|foundational-ontology|fo-km metric|provenance-driven genai kb",
     ["knowledge-graph"], "the project-knowledge-graph surface"),
    # sq-aiut is an MPC bead whose DRIVER is ODRL — the crate is sparq-mpc, so it
    # must be decided before the ODRL rule claims every title containing "ODRL".
    ("mpc-odrl-driven", "title", r"privacy-preserving federated join",
     ["sparq-mpc"], "crates/sparq-mpc (ODRL is the disclosure driver, not the crate)"),
    ("odrl", "title", r"\bodrl\b|odrl-bridge",
     ["sparq-policy"], "ODRL evaluation lives in crates/sparq-policy"),

    # -- house spec + paper surfaces (site/specs, site/papers) -------------------
    # Before the zk rules: a .typ spec that *names* zk/xpath is spec work, not zk work.
    ("site-specs", "text",
     r"site/specs/|zksparql spec|zksparql\.typ|\bspec upd:|spec fold-in:|"
     r"extended shacl compact syntax",
     ["site-specs"], "authors/edits a Typst UPD under site/specs"),
    ("site-papers", "text",
     r"site/papers/|paper-evidence|paper factory|papers rewrite|"
     r"papers/fo-km-agent|logic-bug paper|\bpapers: wire\b|paper open items",
     ["site-papers"], "paper-factory artifact"),

    ("e2ee", "title", r"\be2ee\b",
     ["e2ee"], "the end-to-end-encryption program"),

    # -- the externalised noir libraries under zk/ -------------------------------
    ("zk-proof-program", "title", r"ieee754 & noir_xpath|sparq_ieee754 & noir_xpath",
     ["zk", "zk-xpath"], "spans BOTH zk value-semantics libraries"),
    # TITLE-scoped and ahead of zk-xpath: sq-3x7dl.10's body literally reads
    # "FILE: zk/xpath NO -- zk/ieee754/src/ops/kernels.nr", so a text match on
    # zk/xpath sends an ieee754 bead to the wrong library.
    ("zk-ieee754-title", "title", r"ieee754",
     ["zk"], "zk/ieee754 (sparq_ieee754) named in the title"),
    ("zk-xpath", "text",
     r"zk/xpath|noir_xpath|\bxpath opt:|xpath: regenerate|fn:avg\(\(\)\)",
     ["zk-xpath"], "zk/xpath (noir_XPath) sources"),
    ("zk-compose", "text",
     r"zk/compose|sparq-zk-compose|reconstruct_public_inputs|bind_joins|"
     r"hiddenissuerattestation|resolve_commitment_salt|join_eq_r",
     ["sparq-zk-compose"], "zk-compose circuit/verifier surface"),
    ("zk-schnorr", "title", r"ct schnorr scalar-mul",
     ["sparq-zk"], "Baby-JubJub Schnorr lives in crates/sparq-zk"),

    # -- reasoner family ---------------------------------------------------------
    # A DL bead that moves a measured floor also edits the floor: every
    # DL_*_FLOOR / DL_PROFILE_NEGATIVE_* constant and tests/dl_suite.rs live in
    # crates/sparq-conformance, not in the reasoner.
    ("reason-dl-floor", "text", r"dl_[a-z_]*floor|dl_profile_negative|dl_suite\.rs",
     ["sparq-reason-dl", "sparq-conformance"],
     "the DL lane plus the conformance floor constants it ratchets"),
    ("reason-dl", "title",
     r"\bdl l[0-9]\b|tableau|tableux|pbz04\.4|owl profile dispatch",
     ["sparq-reason-dl"], "the DL (ALCH tableau / profile-dispatch) lane"),
    ("reason-el", "title", r"\bel abox|\bel top|pbz04\.2",
     ["sparq-reason-el"], "the EL profile lane"),
    ("reason-diff", "title", r"reason-diff",
     ["sparq-reason-diff"], "crates/sparq-reason-diff"),
    ("eyereasoner-bundle", "title", r"eyereasoner-compat.*(iife|<script>|bundle)",
     ["sparq-reason-wasm"], "ships a browser bundle"),
    ("eyereasoner", "title", r"eyereasoner-compat",
     ["sparq-reason"], "the eyereasoner-compat surface in crates/sparq-reason"),
    ("rif-conformance", "title", r"rif wg core (suite|floor)|re-measure rif",
     ["sparq-conformance", "sparq-reason"], "RIF_WG_CORE_FLOOR lives in sparq-conformance"),
    ("reason-topic", "title",
     r"^sq-\w+(\.\w+)*: (reasoner|reason\(owl\)|rif-xml|compiled-rules):|"
     r"incremental reasoning|beyond-rl|stratified datalog|reasoning profiler",
     ["sparq-reason"], "crates/sparq-reason"),

    # -- engine / core / substrate ----------------------------------------------
    ("core-memory", "title",
     r"perf\(memory\)|bulk-load|memory-accounting|dictionary raw-slot",
     ["sparq-core"], "store.rs / Dict memory + bulk-load path"),
    ("core-nt", "title", r"native nt\.rs",
     ["sparq-core", "sparq-conformance"], "crates/sparq-core/src/nt.rs + the syntax ratchet floors"),
    ("core-durable", "title", r"shared durable backend",
     ["sparq-core"], "the store/persistence layer"),
    # The trace is engine-side but the only sink today is sparq-server's
    # per-request one (crates/sparq-server/src/http.rs), so both crates move.
    ("access-audit", "title", r"access-audit: engine-internal",
     ["sparq-engine", "sparq-server"], "engine-side trace wired to the server sink"),
    ("engine-topic", "title",
     r"dpccp|dp::plan|join order|cardinality estimator|anti-join|order by|"
     r"explain_json|sp2bench complex-shape|q08/q12b|select-json path",
     ["sparq-engine"], "crates/sparq-engine planner/executor"),
    ("substrate-num", "title", r"num::lexical",
     ["sparq-substrate"], "Num lives in crates/sparq-substrate"),
    ("canon-fuzz", "title", r"canonicalize_nquads",
     ["sparq-canon"], "RDFC-1.0 canonicalization in crates/sparq-canon"),

    # -- other crate families ----------------------------------------------------
    # The sq-qcnn diff-test DAG spans two crates and the split is not the obvious
    # one. crates/sparq-difftest is node A ONLY — the value normaliser, which by
    # design has NO sparq dependency. The differential HARNESS it feeds (fuzz.rs,
    # the oracle adapters, check_ordered, the generator) lives in crates/sparq-bench.
    # A whole-family `area:sparq-difftest` therefore points every one of these
    # beads at a crate most of them never edit; caught by the post-apply sample
    # re-derivation, which found check_ordered in sparq-bench (crates/sparq-bench/
    # src/fuzz.rs:861 — the earlier :307 / :668 line cites were both wrong, though
    # the crate attribution they were used for was right).
    ("difftest-normaliser", "title",
     r"^sq-\w+(\.\w+)*: diff-test:.*(isomorphism|multiset)",
     ["sparq-difftest", "sparq-bench"],
     "extends the independent normaliser AND wires it into the harness"),
    ("difftest-harness", "title", r"^sq-\w+(\.\w+)*: diff-test:",
     ["sparq-bench"], "the differential harness (fuzz.rs / oracle adapters)"),
    ("mpc-seam-plan", "title", r"mpc seam phase 6",
     ["sparq-fedplan-mpc"], "source-selection/combination lives in sparq-fedplan-mpc"),
    ("mpc-seam", "title", r"mpc seam phase",
     ["sparq-fedplan-mpc", "sparq-mpc"], "the untrusted-planner seam + the proof pipeline"),
    ("mpc", "title", r"\bmpc m[0-9]|privacy-preserving federated join",
     ["sparq-mpc"], "crates/sparq-mpc"),
    ("shacl-12", "title", r"shacl 1\.2:",
     ["sparq-shacl"], "crates/sparq-shacl"),
    ("jsonld", "title", r"json-ld conformance",
     ["sparq-jsonld"], "crates/sparq-jsonld"),
    ("hdt-migration", "title", r"hdt 0\.4",
     ["sparq-hdt", "deps"], "crates/sparq-hdt + a supply-chain review"),
    ("lws", "title", r"^sq-\w+(\.\w+)*: (chore\(lws\)|embeddedsparqclient)|"
     r"solid cth conformance harness|port.*housekeeping/security branches",
     ["sparq-lws-core"], "the ported Solid/LWS server crate"),
    ("solid-authz-server", "title", r"stateful /authz",
     ["sparq-server"], "the sparq-server http.rs authz path"),
    ("mcp", "title", r"mcp tool-surface",
     ["sparq-mcp"], "crates/sparq-mcp"),
    ("vectors-spqv", "title", r"\.spqv\b|vec:hybrid",
     ["sparq-vectors"], "the .spqv / fusion surface in crates/sparq-vectors"),
    ("genai-planner", "title", r"gnce-style planner",
     ["sparq-engine"], "a planner-only cardinality estimator"),
    ("kani-harness", "title", r"^sq-\w+: kani:",
     ["sparq-core", "sparq-vectors"], "the two crates owning the timing-out harnesses"),

    # -- javascript / wasm client -----------------------------------------------
    ("js", "text",
     r"js/src/|js/package\.json|@jeswr/sparq|rdf/js conformance|\bjs gate\b",
     ["js"], "the js/ RDF-JS client"),

    # -- website + desktop GUI ---------------------------------------------------
    ("gui-tauri", "text", r"gui/src-tauri",
     ["gui", "deps"], "the Tauri desktop shell's dependency stack"),
    ("site-a11y-vr", "title",
     r"^sq-\w+(\.\w+)*: (a11y|nightly[- ]sweeps?|website)\b|"
     r"visual-regression|axe ratchet",
     ["site"], "site/e2e + site/src surfaces"),
    ("site-page", "title", r"^sq-\w+: page: /surface/",
     ["site"], "a site/ route"),
    ("site-app", "text", r"sparq\.jeswr\.org/app|site/src/app/",
     ["site"], "a site/ route"),
    ("deploy-demo", "title", r"^sq-\w+: deploy\(demo\)",
     ["deploy-demo"], "the hosted demo deployment"),

    # -- benchmarks --------------------------------------------------------------
    ("bench", "text",
     r"bench/dashboard|bench/lubm|bench/serve-throughput|bench/fo-km|bench/ harness|"
     r"gather-competitors|virtuoso-same-box|http-sparql|perf-gate:|benchmark-data|"
     r"in-site benchmarks|ann gather|competitor engines|competitor numbers|"
     r"diskann-ref",
     ["bench"], "the bench/ harness + competitor gather"),
    ("bench-generic", "title", r"\bbenchmark",
     ["bench"], "a benchmarking deliverable"),

    # -- CI / release / dependencies / workspace ---------------------------------
    # `workspace` before `release`: sq-qxq5m is a `git rm --cached` of a repo-root
    # tracked file that merely MENTIONS release-plz cleanliness as its motivation.
    ("fmt", "title",
     r"cargo fmt|rustfmt|proptest-regressions|untrack \.beads/interactions",
     ["workspace"], "a workspace-wide mechanical/hygiene change"),
    ("security-txt", "title", r"security\.txt",
     ["workspace"], "the repo-root .well-known/ metadata"),
    ("release", "title",
     r"release-plz|release sbom job|slsa build l3|v0\.1\.0 release",
     ["release"], "the release pipeline"),
    ("deps", "title",
     r"dependabot bumps|cyclonedx-npm|supply-chain:",
     ["deps"], "dependency management"),
    ("ci", "text",
     r"\.github/workflows/|feature-matrix\.d|merge-queue|ci-summary|nextest|"
     r"cargo-mutants|scorecard\.yml|fuzz\.yml|\.beads/issues\.jsonl|"
     r"autonomous-scheduler|\.claude/workflows|bd-to-issues|skipping tests in prs|"
     r"follow up tasks / tickets|production-readiness",
     ["ci"], "CI / orchestration infrastructure"),

    ("website", "title", r"\bwebsite\b",
     ["site"], "the marketing/docs website"),
]


# --- T0: an explicit author declaration in the body -----------------------------
_DECL = re.compile(
    r"(?:crate_or_surface|crates?|surface)\s*[:=]\s*(.+?)(?:\||\n|\.\s|$)", re.I)
_SURFACE_TOKENS = {"site": "site", "gui": "gui", "docs": "docs", "bench": "bench",
                   "ci": "ci", "js": "js"}
# Path fragments a declaration may use instead of a bare token —
# `surface=.github/workflows/ci.yml` names CI, but "ci" there is preceded by "/".
_DECL_PATHS = ((".github/workflows", "ci"), ("bench/", "bench"), ("site/", "site"),
               ("gui/", "gui"), ("js/", "js"), ("docs/", "docs"),
               ("zk/xpath", "zk-xpath"), ("zk/ieee754", "zk"))


def declared_areas(body, crates):
    """Areas the AUTHOR declared in a `crate_or_surface:` / `crates:` / `Crate:`
    field. The decomposition templates emit this field verbatim, and an author
    naming the package beats every inference below it. Returns [] when the field is
    absent or names something that is not a real crate/surface (e.g. sq-lsp7k.12's
    "NEW opt-in crate" — that issue is genuinely unclassifiable and stays parked).

    The span ends at the first `|`, newline, or SENTENCE BREAK: sq-p4zci's field
    reads "crates: sparq-reason + sparq-cli. CLI flag + ... docs/SKILL examples",
    and running to end-of-line would derive a spurious `area:docs` from the prose
    that follows the declaration."""
    out = []
    for m in _DECL.finditer(body or ""):
        span = m.group(1).lower()
        # Ordered by FIRST OCCURRENCE in the declaration, longest crate name
        # winning over any name it contains (sparq-reason-el over sparq-reason) —
        # the same discipline bd-to-issues._token_hits uses, and the reason the
        # output is stable enough to assert on.
        hits = []
        for name in sorted(crates, key=len, reverse=True):
            mm = re.search(r"(?<![a-z0-9\-])" + re.escape(name) + r"(?![a-z0-9\-])", span)
            if mm and not any(name in n for _, n in hits):
                hits.append((mm.start(), name))
        for frag, area in _DECL_PATHS:
            if frag in span:
                hits.append((span.index(frag), area))
        for tok, area in _SURFACE_TOKENS.items():
            mm = re.search(r"(?<![a-z0-9\-/])" + tok + r"(?![a-z0-9\-])", span)
            if mm:
                hits.append((mm.start(), area))
        for _, area in sorted(hits):
            if area not in out:
                out.append(area)
    return out


def classify(title, body, crates):
    """(areas, evidence) for one issue. `areas` == [] means LEAVE IT PARKED."""
    title = title or ""
    body = body or ""
    text = (title + "\n" + body).lower()
    low_title = title.lower()

    d = declared_areas(body, crates)
    if d:
        return d, "T0 author-declared crate_or_surface/crates field"

    for rid, scope, rx, areas, why in RULES:
        hay = low_title if scope == "title" else text
        if re.search(rx, hay, re.I):
            return list(areas), f"T1 {rid}: {why}"

    fallback = _bd_derive(title, body, crates)
    if fallback:
        return fallback, "T2 bd-to-issues.derive_areas (title scope/crate token)"
    return [], ""


def _bd_derive(title, body, crates):
    """Reuse the migration-time deriver verbatim so a freshly-migrated issue is
    classified identically by both code paths. Imported lazily + defensively: this
    script must still self-test if bd-to-issues.py is unavailable."""
    try:
        import importlib.util
        spec = importlib.util.spec_from_file_location(
            "bd_to_issues", os.path.join(HERE, "bd-to-issues.py"))
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        return mod.derive_areas({"title": title, "description": body}, crates)
    except Exception:
        return []


# --- github I/O -----------------------------------------------------------------
def _gh(args):
    return subprocess.run(["gh"] + args, capture_output=True, text=True, check=True).stdout


def crate_names():
    base = os.path.join(HERE, os.pardir, "crates")
    try:
        return tuple(sorted(d for d in os.listdir(base)
                            if os.path.isdir(os.path.join(base, d)) and not d.startswith(".")))
    except OSError:
        return ()


def live_area_labels():
    """The area labels that ALREADY EXIST. Never `gh label create` — an invented
    partition is invisible to every consumer (ready-issues.py, push-frontier.sh)
    and silently mis-partitions the frontier."""
    data = json.loads(_gh(["label", "list", "--repo", REPO, "--limit", "500", "--json", "name"]))
    return {d["name"] for d in data if d["name"].startswith("area:")}


FETCH_CEILING = 10000


def label_names(issue):
    """The label names on a `gh`/REST issue payload. Both shapes spell a label as
    `{"name": ...}`, so one reader serves the snapshot fetch and the plan."""
    return {lb.get("name") for lb in (issue.get("labels") or []) if isinstance(lb, dict)}


def open_issues(ceiling=FETCH_CEILING):
    """Every OPEN issue, via REAL cursor pagination.

    [OPUS-5] (#5003) This was `gh issue list --limit 1000`, which SILENTLY
    TRUNCATES: the CLI stops at the limit and reports nothing — no warning, no
    non-zero exit. On a half-hourly cron a silently-dropped tail is never noticed,
    and this is a work-queue sweep, so a dropped issue is not classified late, it is
    never classified on any tick. `gh api --paginate` follows the Link headers to
    exhaustion instead; the explicit ceiling still fails CLOSED on a runaway
    snapshot rather than half-reporting (same shape as `retriage.py::_fetch_label`
    and `ready-issues.py::_fetch`).

    `gh search` is deliberately not used: it reads a lagging index and has caused
    three wrong conclusions in this repo.

    The issues endpoint returns PRs too; they are dropped here (a PR carries no
    dispatch partition, and labelling one would be pure noise on the board)."""
    pages = json.loads(_gh(["api", "--paginate", "--slurp",
                            f"repos/{REPO}/issues?state=open&per_page=100"]) or "[]")
    rows = [i for page in pages if isinstance(page, list) for i in page
            if isinstance(i, dict) and "pull_request" not in i]
    if len(rows) >= ceiling:
        raise SystemExit(
            f"triage-area: fetched {len(rows)} open issues >= ceiling {ceiling} — the "
            "snapshot looks runaway (fail-closed). Raise the ceiling deliberately.")
    return rows


def candidate_issues():
    """Every OPEN issue carrying NO `area:` label — parked `needs:area` or not.

    [OPUS-5] THE REACH DEFECT (#5003). This used to be a `--label needs:area` query.
    That returns the issues `triage.py` / `bd-to-issues.py` PARKED, but the park is
    one source of area-less issues, not the class: an issue the event triager never
    ran on carries no `status:*` label and no park either, and is equally area-less
    and equally undispatchable: `ready-issues.py::packages_of` resolves a label-set
    with no `area:` to the `GLOBAL` partition, whose `()` path is a prefix of every
    other, so it serializes against everything. #3816 counted 832 open issues with
    no `area:` against a park an order of magnitude smaller.

    A label query is structurally unable to return the class defined by a MISSING
    label — the same hole `retriage.py::_fetch_candidates` was widened to close, and
    for the same reason: an issue that is never FETCHED is never tested.

    Reach is not permission. Every gate below is unchanged — `classify()` still
    fail-closes to no label, `live_area_labels()` still rejects an invented one, and
    `apply_row(unpark=...)` still refuses to remove a park the issue never had. The
    only new write this widening can produce is an `area:` label on an area-less
    issue, which is exactly what the tool is for."""
    return [it for it in open_issues()
            if not any((n or "").startswith("area:") for n in label_names(it))]


# --- the unattributed census (#5004) --------------------------------------------
# [OPUS-5] THE CLASS THE PARK QUERY CANNOT SEE.
#
# The classifier's own residue report (#3816) is assembled from the rows the sweep LEFT
# unattributed and is written up in `triage-area.yml` against the `--label needs:area`
# population, so every report built that way — including that run-summary one — describes
# only the issues that ALREADY carry the park. (Since #5003 the sweep's own QUEUE is wider
# than the park; the census below is about what gets COUNTED and reported, not fetched.)
#
# PER sparq#5004 — a statement about the PRIVATE `jeswr/agent-account-registry`, not
# something this repository can check — its `curate-frontier.py` skips on a WIDER
# predicate: any issue whose area it cannot derive, logged `skip #N: no confident area`,
# whether or not that issue is parked. Only the sparq half is verifiable from here, and
# that half is enough on its own: the two label sets genuinely differ, and the gap between
# them is structural rather than incidental:
#
#   * `triage.py` applies the park only to an issue that is otherwise triage-complete —
#     it deliberately withholds it from an epic, from an externally-gated issue, and from
#     anything still missing a role or a priority;
#   * `retriage.py` COMPUTES that park delta and then discards it, because it emits
#     PROMOTING deltas only. Its own docstring records the leftover population and hands
#     it on: "clearing that population is `needs:area` work, not this layer's."
#
# Nothing then picks it up. `triage-area.py` is that layer, and it was querying the park —
# so a no-area issue that was never parked is invisible to BOTH ends: sparq reports on the
# park it is not in, and the registry re-skips it every half hour with no label, no count
# and no route back to a human. That is #5004, and it is the half of it that sparq owns:
# the curator lives in another repository, but the board it reads is this one.
#
# This census is READ-ONLY and deliberately so. It computes the curator's predicate on
# sparq's side of the fence and hands the count to `triage-area.yml`, which always prints
# it — the always-printed-count shape #5004 asks for. It does NOT write `needs:area` onto
# the unparked half: that would be a mass relabel of the live board under park rules
# `triage.py` deliberately withholds from epics and gated issues, and it is a separate
# decision from making the number visible.
#
# This asks for the WHOLE board, not just the park, and the open-issue count is already the
# larger figure (`retriage.py`'s docstring carries the live measurement; no number is
# restated here, because one pinned in a comment did not survive the week last time). A
# silently short fetch would UNDERCOUNT the very residue the census exists to surface — the
# exact failure mode #5004 is about, reintroduced by the fix. Detected, never assumed.
#
# So the census does NOT carry a fetch of its own: it reuses `open_issues()` above, the one
# cursor-paginated snapshot #5003 introduced. That is the same requirement arrived at from
# the other end — #5003 needed the classifier to stop truncating its queue, #5004 needs the
# census to stop undercounting its class, and a `--limit` fetch fails both the same way.
# One fetch means the two reports cannot drift into disagreeing about the same board, and
# `FETCH_CEILING` gives the census its fail-closed guarantee: pagination runs to exhaustion,
# so a short count is impossible, and an implausible snapshot raises instead of half-
# reporting. `open_issues()` also drops PRs, which the census must not count either.
def census(issues):
    """Split the OPEN issues into (parked, unparked) halves of the unattributed class.

    `unattributed` = carries no `area:` label = the registry curator's skip predicate.
    `parked` are the ones the existing residue report already describes; `unparked` are
    the ones no report on either side of the fence has ever counted.
    """
    parked, unparked = [], []
    for it in sorted(issues, key=lambda i: i["number"]):
        # `label_names()` is the shared tolerant reader: since the census took over the
        # classifier's REST fetch (#5003) both callers see the same payload shape, so both
        # must read it the same way — and it is the predicate that decides the count.
        names = label_names(it)
        if any((n or "").startswith("area:") for n in names):
            continue
        (parked if PARK_LABEL in names else unparked).append(it)
    return parked, unparked


def print_census(issues):
    """The always-printed count, in the shape `triage-area.yml` reads back out.

    `== ` is the tally line and `   UNPARKED ` the per-issue lines, deliberately distinct
    from the classifier's own `-- `/`   LEFT `. The two report steps read SEPARATE logs
    today, so that distinctness is defence in depth rather than the thing keeping them
    apart: it is what stops a later merge of the two reports — or a `--census` run teed
    into the classifier's log — from having one step silently absorb the other's lines.
    """
    parked, unparked = census(issues)
    print(f"== {len(parked) + len(unparked)} open issue(s) carry no area: label "
          f"({len(parked)} parked {PARK_LABEL}, {len(unparked)} with no board state at all), "
          f"{len(issues)} open")
    for it in unparked:
        print(f"   UNPARKED #{it['number']} {it['title'][:120]}")


def plan(issues, crates):
    """The proposed mapping. Deterministic, and a no-op for an issue that already
    carries an area (idempotence: re-running never re-decides settled work).

    The already-has-an-area check is also `candidate_issues()`'s fetch filter. Kept
    here as the second line of defence: `plan()` is the only caller of `classify()`,
    so a future fetch that widens again cannot silently start re-deciding work a
    maintainer already settled."""
    rows = []
    for it in sorted(issues, key=lambda i: i["number"]):
        names = [lb["name"] for lb in it.get("labels", [])]
        if any(n.startswith("area:") for n in names):
            rows.append((it, [], "SKIP already carries an area: label"))
            continue
        areas, why = classify(it["title"], it.get("body") or "", crates)
        rows.append((it, [f"area:{a}" for a in areas], why))
    return rows


def apply_row(number, add, unpark=True):
    """Add the area labels AND — when the issue is actually parked — drop the park,
    in ONE call. They must not drift: an issue with an area but still parked stays
    undispatchable, and a cleared park with no area silently reserves the
    serializing __global__ partition.

    `unpark=False` is the never-parked half of the widened reach (#5003). Since the
    queue is every AREA-LESS open issue rather than every PARKED one, most rows now
    carry no `needs:area` at all. Such an issue must only ever GAIN `area:` labels:
    emitting `--remove-label needs:area` for it would be a write the sweep has no
    business making, and it would report a park-clearing it did not perform.

    FAIL CLOSED on an empty `add`. `main()` already filters unclassified rows out
    before it gets here, but that filter is one edit away from the apply loop and
    a BARE UNPARK is the worst outcome this tool can produce: the issue looks
    triaged, `retriage.py` promotes it, and it then reserves the serializing
    `__global__` partition — which collapses the dispatch frontier to a single
    worker. The invariant therefore lives in the ONLY function that can emit the
    unpark, not in the caller that happens to protect it today. It is asserted for
    BOTH values of `unpark`: with no areas there is nothing legitimate to write
    either way."""
    if not add:
        raise ValueError(
            f"refusing a bare unpark of #{number}: `{PARK_LABEL}` may only be "
            "removed in the same call that adds >=1 area: label")
    args = ["issue", "edit", str(number), "--repo", REPO]
    if unpark:
        args += ["--remove-label", PARK_LABEL]
    for a in add:
        args += ["--add-label", a]
    _gh(args)


# --- self-test ------------------------------------------------------------------
def _chk(name, got, want):
    if got != want:
        print(f"FAIL {name}: got {got!r} want {want!r}")
        return 1
    print(f"ok   {name}")
    return 0


def self_test():
    crates = crate_names() or ("sparq-core", "sparq-engine", "sparq-reason", "sparq-py",
                               "sparq-server", "sparq-zk-compose", "sparq-mpc")
    f = 0
    c = lambda t, b="": classify(t, b, crates)[0]

    # T0 beats everything: the author named the package.
    f += _chk("declared single", c("whatever", "crate_or_surface: sparq-reason | effort:M"),
              ["sparq-reason"])
    f += _chk("declared multi", c("x", "crates: sparq-reason + sparq-cli."),
              ["sparq-reason", "sparq-cli"])
    f += _chk("declared surface", c("x", "crate_or_surface: sparq-py (magic) + site + docs"),
              ["sparq-py", "site", "docs"])
    # A declaration that names no real crate must NOT invent one.
    f += _chk("declared new-crate -> parked",
              c("BI/SQL facade", "crate_or_surface: NEW opt-in crate | effort:XL"), [])

    # FAIL CLOSED is the headline behaviour: no evidence => no label.
    f += _chk("no evidence -> parked", c("do the thing", "it would be nice"), [])

    # SCOPE is the other half of the anti-mislabel design, and 60 of the 70 rules
    # rely on it: a `title`-scoped rule must NOT fire off a body mention. This is
    # the axis `derive_areas` got wrong (one incidental word in a body labelled a
    # zkSPARQL spec bead `area:site`). Collapsing the scope dispatch in classify()
    # to `hay = text` is a ONE-LINE edit, and it is the shape every rule-table
    # tweak takes, so pin it here as well as in the per-rule property test in
    # scripts/tests/test_triage_area.py.
    f += _chk("body-only mention does not fire a title rule",
              c("do the thing", "we should benchmark this before deciding"), [])
    f += _chk("body-only ieee754 does not fire the title-scoped zk rule",
              c("do the thing", "background notes: this touches ieee754 rounding"), [])

    # Ordering: a .typ spec that merely NAMES zk/xpath is spec work, not zk work.
    f += _chk("spec beats zk",
              c("zksparql.typ 7.3 stale estate sentence: still names zk/ieee754 + zk/xpath"),
              ["site-specs"])
    # ...and a workflow-lane wiring bead that names zk-compose is CI work.
    f += _chk("workflow lane beats zk-compose",
              c("Wire bench/zk-compose/scripts/eval_pack.py --check into the zk-toolchain.yml lane"),
              ["ci"])
    # ...and an upstream noir bead that names SSA files is upstream, not a crate.
    f += _chk("noir upstream",
              c("noir upstream: NOT-canonicalization to unlock IfElse merging (simplify.rs TODOs)"),
              ["upstream"])

    # Genuinely cross-cutting stays cross-cutting (the partitioner maps it to
    # __global__ deliberately; collapsing it to one crate would be a lie).
    f += _chk("dual zk libraries",
              c("[epic] Proof-of-correctness program for sparq_ieee754 & noir_XPath"),
              ["zk", "zk-xpath"])

    # Narrow families resolve to their own crate, not the parent reasoner.
    f += _chk("dl lane", c("DL L4: narrow the RL disjointWith divergence guard"),
              ["sparq-reason-dl"])
    f += _chk("el lane", c("EL abox+cdomain DataPropertyAssertion point-range rescue"),
              ["sparq-reason-el"])
    # The diff-test DAG splits across TWO crates and the split is not the obvious one,
    # so pin BOTH branches of the split — a single case cannot catch a rule that
    # collapses the family onto one crate. Node A (the engine-independent normaliser)
    # is crates/sparq-difftest; the harness it feeds (check_ordered lives at
    # crates/sparq-bench/src/fuzz.rs) is crates/sparq-bench. A bead that WIRES the
    # normaliser into the harness therefore moves both.
    f += _chk("difftest normaliser+harness",
              c("sq-qcnn.5: diff-test: wire canonical binding-MULTISET check"),
              ["sparq-difftest", "sparq-bench"])
    # ...but a harness-only diff-test bead must NOT drag the normaliser crate in.
    f += _chk("difftest harness only",
              c("sq-qcnn.7: diff-test: add an oracle adapter to the generator"),
              ["sparq-bench"])

    # T2 fallback still works (a conventional title scope prefix).
    f += _chk("t2 title scope", c("bug(sparq-solid): fail-closed ACP"), ["sparq-solid"])

    # Idempotence: an issue that already has an area is never re-decided.
    rows = plan([{"number": 1, "title": "DL L4: whatever", "body": "",
                  "labels": [{"name": "area:sparq-core"}, {"name": PARK_LABEL}]}], crates)
    f += _chk("already-area is a no-op", rows[0][1], [])

    print("SELF-TEST", "FAILED" if f else "PASSED")
    return 1 if f else 0


def main():
    ap = argparse.ArgumentParser(description="Classify the needs:area backlog.")
    ap.add_argument("--apply", action="store_true", help="write labels (default: dry run)")
    ap.add_argument("--self-test", action="store_true", help="offline rule unit tests")
    ap.add_argument("--json", action="store_true", help="emit the plan as JSON")
    ap.add_argument("--census", action="store_true",
                    help="READ-ONLY: count every open issue with no area: label (#5004)")
    a = ap.parse_args()
    if a.self_test:
        return self_test()
    if a.census:
        # Read-only and independent of the rule table: it neither classifies nor writes, so
        # it reports the same number whether or not the sweep above ran or wrote anything.
        print_census(open_issues())
        return 0

    crates = crate_names()
    known = live_area_labels()
    rows = plan(candidate_issues(), crates)

    unknown = sorted({lb for _, add, _ in rows for lb in add} - known)
    if unknown:
        print(f"ERROR: rule table produced labels that do not exist: {unknown}", file=sys.stderr)
        print("Fix the rule table — do NOT create the label.", file=sys.stderr)
        return 2

    if a.json:
        print(json.dumps([{"number": it["number"], "title": it["title"],
                           "areas": add, "evidence": why} for it, add, why in rows], indent=1))
        return 0

    classified = [r for r in rows if r[1]]
    left = [r for r in rows if not r[1] and not r[2].startswith("SKIP")]
    for it, add, why in rows:
        if not add:
            continue
        print(f"#{it['number']:<5} {','.join(a[5:] for a in add):<40} {why}")
        print(f"       {it['title'][:150]}")
    print(f"\n-- {len(classified)} classified, {len(left)} left unattributed "
          f"(no confident evidence), {len(rows)} scanned")
    for it, _, _ in left:
        print(f"   LEFT #{it['number']} {it['title'][:120]}")

    if not a.apply:
        print("\n(dry run — re-run with --apply to write labels)")
        return 0
    for it, add, _ in classified:
        # The unpark is decided PER ISSUE from its own labels: the queue is now every
        # area-less open issue, so most rows were never parked and must gain only areas.
        parked = PARK_LABEL in label_names(it)
        apply_row(it["number"], add, unpark=parked)
        print(f"applied #{it['number']} {','.join(add)}"
              f"{' -' + PARK_LABEL if parked else ''}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
