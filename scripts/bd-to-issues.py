#!/usr/bin/env python3
# [OPUS-4.8] Issue-native orchestration: migrate bd beads -> GitHub issues (Phase 1).
"""bd-to-issues.py — one-time, idempotent migration of open bd beads into GitHub issues.

DEFAULT is --dry-run: it parses `bd export`, computes the issue payloads + label mapping + the
dependency edges, and prints a summary WITHOUT creating anything. `--apply` does the real
three-pass run (create issues, then stamp the `Blocked-by:`/`Parent:` body markers, then reconcile
labels — see apply_migration) and writes the `sq-… ↔ #NN` map — held for the maintainer's go-ahead
because it bulk-creates hundreds of issues.

Label mapping (bd -> issue):
  priority 0..4            -> priority:P{n}
  existing labels          -> passed through verbatim (area:<crate>=package, needs:*, kind:*, from:*)
  issue_type / kind:       -> role:<r> (feature/bug->impl, docs->docs, spike->research, chore->ci, ...)
  every migrated issue     -> MIGRATION_LABEL (authenticates the `<!-- bd-id:… -->` body marker)
  dependency edges         -> `Blocked-by: #NN` body markers ONLY (see NATIVE DEPENDENCIES below)
The bead id (`sq-…`) is preserved in the issue title so existing PR-title tokens still resolve.

NATIVE DEPENDENCIES — this script does NOT write them, in any mode. Every edge it emits is a
`Blocked-by:` / `Parent:` body marker (pass 2); nothing here calls GitHub's issue-dependency or
sub-issue API, so a migrated bd blocker edge does not appear in the native dependency UI. Dispatch
is unaffected: `ready-issues.py::open_blocker_count` holds an issue iff EITHER channel reports an
open blocker, so a marker-only edge still blocks. The gap is one-directional and is a VISIBILITY
one: a maintainer reading the native graph cannot see the bd-derived edges. Mirroring the markers
into native edges is a separate, still-undecided change (issue #4616); the only native reader
in this file is `--verify`, which uses the count-only summary as context and never as evidence that
a specific marker landed (see fetch_native_blockers / verify_migration).

`--verify` is the read-only post-migration reconcile (see verify_migration): counts, duplicate
bd-id mappings, and whether pass 2's dependency markers actually landed. It creates and edits
nothing, so it is safe to run at any time against a live board. It counts a bead as migrated only
on the same authentication `--apply` demands — MIGRATION_LABEL on the issue — so a board of
forgeable marker-only decoys fails closed instead of verifying as a finished migration.
"""
import argparse
import json
import os
import re
import subprocess
import sys
import time

ROLE_BY_TYPE = {"feature": "impl", "bug": "impl", "task": "impl", "chore": "ci",
                "spike": "research", "epic": "impl"}
ROLE_BY_KIND = {"docs": "docs", "design": "research", "research": "research", "perf": "perf",
                "test": "impl", "ci": "ci", "site": "site"}


def parse_export(lines):
    """Return (issues_by_id, edges) from `bd export` JSONL. Robust to edge records / dependencies
    arrays being expressed either as separate `_type` lines or as a `dependencies` field."""
    issues, edges = {}, []
    for line in lines:
        line = line.strip()
        if not line:
            continue
        d = json.loads(line)
        t = d.get("_type", "issue")
        if t and t != "issue":
            # a SEPARATE dependency record: {issue_id, depends_on_id, type} (or from/to variants)
            frm = d.get("issue_id") or d.get("from") or d.get("dependent") or d.get("blocked")
            to = d.get("depends_on_id") or d.get("to") or d.get("depends_on") or d.get("blocker")
            if frm and to:
                edges.append((frm, to, d.get("type", "blocks")))
            continue
        if "id" not in d:
            continue
        issues[d["id"]] = d
        for dep in d.get("dependencies") or []:
            if not isinstance(dep, dict):
                continue
            to = dep.get("depends_on_id") or dep.get("depends_on") or dep.get("to")
            etype = dep.get("type", "blocks")
            if to:
                edges.append((dep.get("issue_id", d["id"]), to, etype))
    # dedupe (an edge can appear both embedded in an issue and as a separate record)
    edges = sorted({(a, b, c) for (a, b, c) in edges})
    return issues, edges


def role_for(issue):
    labels = [lb["name"] if isinstance(lb, dict) else lb for lb in issue.get("labels", [])]
    for lb in labels:
        if lb.startswith("kind:") and lb[5:] in ROLE_BY_KIND:
            return ROLE_BY_KIND[lb[5:]]
    return ROLE_BY_TYPE.get(issue.get("issue_type", "task"), "impl")


# [OPUS-4.8] Only PIPELINE-RELEVANT labels cross into GitHub. bd carries ~200 free-form tags
# (from:*, effort:*, tier:*, roadmap, one-off topic tags) that triage/readiness/dispatch NEVER read.
# Passing them through would (a) force creating ~200 junk repo labels and (b) risk a LABEL-LESS issue:
# `gh issue create` fails on the first unknown label → issues are only created against a PRE-CREATED
# full label set (ensure_labels), failing LOUDLY instead of silently dropping labels. Keep only what
# the pipeline consumes.
_KEEP_PREFIX = ("area:", "needs:", "trust:", "kind:")
_SEC_KEYWORDS = ("zk", "mpc", "reasoner", "crypto", "auth", "e2ee")

# Migration-owned authentication label (PR #2528 review): scripts/ci-close-merged-beads.py trusts
# a `<!-- bd-id:… -->` body marker ONLY on an issue carrying this label, because issue bodies are
# forgeable by any GitHub user while attaching a label needs triage/write permission. Stamped on
# EVERY migrated issue; a resumed --apply backfills it ONLY onto issues with migration-owned
# provenance (recorded in the trusted checkpoint — see resolve_resume_map), never onto arbitrary
# body-marker matches. Keep in sync with _MIGRATION_LABEL there, and never let an issue template
# auto-apply it.
MIGRATION_LABEL = "bd-migration"
_BLOCKED_ON = re.compile(r"^blocked[-:]on[-:](.+)$")
_BLOCKED = re.compile(r"^blocked:(.+)$")


def b2i_bd_labels(bead):
    """The bead's OWN labels (post blocked:*->needs:* mapping, pre-derivation) — used by the
    summary to report how many `area:` labels the migration DERIVED rather than inherited."""
    return {_map_label(lb["name"] if isinstance(lb, dict) else lb)
            for lb in bead.get("labels", [])} - {None}


def _map_label(lb):
    """Migration gating pass (audit-2026-07-17): bd `blocked:*` / `blocked-on-*` tags become
    `needs:*` gates — the readiness engine only gates on the needs:/trust: prefixes, so an unmapped
    `blocked:ec2` bead would migrate DISPATCHABLE and burn worker attempts on an un-runnable task.
    `blocked-by-epic:*` is DROPPED: parent-child edges already model epic linkage, and mapping it to
    needs:* would reintroduce the sub-epic-leaf-invisibility bug the migration deliberately fixes."""
    if lb.startswith("blocked-by-epic:"):
        return None
    m = _BLOCKED_ON.match(lb) or _BLOCKED.match(lb)
    return f"needs:{m.group(1)}" if m else lb


def _pipeline_relevant(lb):
    """A label triage/readiness/dispatch actually reads: an area:/needs:/trust:/kind: label, or one
    carrying a security keyword so triage's soundness routing survives even without an area:/kind:."""
    return lb.startswith(_KEEP_PREFIX) or any(k in lb for k in _SEC_KEYWORDS)


# External/audit gating (audit-2026-07-17): human/credential/externally gated beads (the sq-qhy4
# external-cryptographer-audit class, token-placement, maintainer-decision beads) must migrate as
# needs:user — NOT as plain dispatchable issues. needs:user is the ONLY durable parking state: the
# dispatcher's deferred-retry re-fires any needs-free status:deferred issue, so without this gate
# sq-qhy4 (P0, no area → the __global__ partition) would be dispatched FIRST, fail, and loop —
# serializing the frontier behind an issue no worker can implement.
_EXTERNAL_IDS = {"sq-qhy4", "sq-v286.8"}
_EXTERNAL_TITLE = re.compile(
    r"\bexternal\b.*\baudit\b|accredited|cryptographer|\bneeds:user\b|"
    r"maintainer[- ](sign-?off|review|greenlit|gated)|code-signing|notariz|"
    r"token (placement|upload|provisioning)", re.I)
_EXTERNAL_DESC = re.compile(r"agent[- ]out[- ]of[- ]scope|out[- ]of[- ]scope by definition", re.I)


def externally_gated(bead):
    """True when a bead is gated on a human/credential/external dependency (curated id list +
    title patterns + an explicit description marker). Deliberately errs toward over-gating:
    needs:user is maintainer-visible and trivially reversible, while a mis-dispatched external
    gate burns worker attempts and can reserve the global partition."""
    if str(bead.get("id", "")) in _EXTERNAL_IDS:
        return True
    if _EXTERNAL_TITLE.search(bead.get("title") or ""):
        return True
    return bool(_EXTERNAL_DESC.search((bead.get("description") or "")[:400]))


# --- area (package) derivation ------------------------------------------------------------------
# Audit 2026-07-25 (sq-gj35r): 551 of the 895 open beads carry NO `area:` label, so they migrated
# into the readiness engine's serializing `__global__` partition. triage.py refuses to promote a
# no-area issue at all (it parks `needs:area`), and retriage.py only emits PROMOTING deltas — so
# such an issue is TERMINALLY `status:untriaged`: present in GitHub, invisible to the orchestrator.
# Migrating an issue the orchestrator cannot classify is not migration, it is litter. So derive the
# package from the bead's own text, and when nothing is derivable make the parked state EXPLICIT
# with `needs:area` (a gate ready-issues.py already respects + maintainer-visible) instead of
# silently reserving the global partition.
#
# [OPUS-5] SCOPE OF THAT CLAIM, corrected in review round 2. It holds for the ZERO-area case only.
# `limit=2` below means a bead CAN emit two areas, and the two consumers then disagree about what
# that means: ready-issues.py `packages_of` treats it as the SET {a, b}, while dispatch-plan.py
# collapses any non-singleton package set to `_ready.GLOBAL` — so a two-area issue lands in the
# serializing __global__ partition at PLAN time after all, which is precisely the outcome the
# needs:area park exists to avoid. MEASURED on the live board 2026-07-26: 55 of the 895 migrated
# issues carry >=2 `area:` labels. Re-running derive_areas over their reconstructed bead text
# reproduces >=2 for only 13 of those 55 (the reconstruction from the issue title/body is
# approximate, so treat 13 as +/-1); the other 42 inherited both labels from their bd record or
# from hand-labelling. So lowering this limit would NOT retro-fix the bulk of them, which is why
# the fix is a decision about which consumer is authoritative rather than a change here.
# Tracked separately (#3838); deliberately not reworded away.
_SURFACE_AREAS = {"site": "site", "gui": "gui", "bench": "bench", "ci": "ci", "docs": "docs",
                  "js": "js", "wasm": "sparq-wasm", "workflows": "ci", "release": "release",
                  "orchestration": "orchestration", "deps": "deps", "workspace": "workspace"}
# `verb(scope):` / `scope:` conventional title prefixes, e.g. "bug(sparq-solid): …", "site: …".
_TITLE_SCOPE = re.compile(r"^\s*(?:[a-z]+\(([a-z0-9\-/]+)\)|([a-z0-9\-]+))\s*:", re.I)
_CRATES_CACHE = None


def crate_names(root=None):
    """The workspace crate names (`crates/`), cached. Empty when run outside a checkout — the
    derivation then falls back to the conventional-title-prefix + surface rules only."""
    global _CRATES_CACHE
    if root is None and _CRATES_CACHE is not None:
        return _CRATES_CACHE
    base = root or os.path.join(os.path.dirname(os.path.abspath(__file__)), os.pardir, "crates")
    try:
        names = tuple(sorted(d for d in os.listdir(base)
                             if os.path.isdir(os.path.join(base, d)) and not d.startswith(".")))
    except OSError:
        names = ()
    if root is None:
        _CRATES_CACHE = names
    return names


def _scope_to_area(scope, crates):
    scope = (scope or "").lower().split("/")[0]
    for cand in (scope, f"sparq-{scope}"):
        if cand in crates:
            return cand
    return _SURFACE_AREAS.get(scope)


def _token_hits(names, text):
    """`names` occurring in `text` as whole tokens, ordered by FIRST OCCURRENCE, longest name
    winning over any name it contains (`sparq-reason-el` over `sparq-reason`). First-occurrence
    order matters: an alphabetical sort followed by a `[:limit]` truncation silently drops the
    most-specific hit — "deploy(site+docs): …" derived `ci,docs` and discarded `site`."""
    found = []
    for n in sorted(names, key=len, reverse=True):
        m = re.search(r"(?<![a-z0-9\-])" + re.escape(n) + r"(?![a-z0-9\-])", text)
        if m and not any(n in f for _, f in found):
            found.append((m.start(), n))
    return [n for _, n in sorted(found)]


def derive_areas(bead, crates=None, limit=2):
    """Derive `area:` values from a bead's own text, most-specific evidence first:

      1. the conventional `verb(scope):` / `scope:` title prefix — an explicit author declaration;
      2. crate names appearing as whole tokens in the TITLE;
      3. a surface keyword (site/gui/bench/ci/…) in the title's SCOPE REGION only — i.e. before the
         first colon. A bare "site"/"ci"/"docs" anywhere in running prose is NOT evidence: matching
         the whole title mislabeled a zkSPARQL spec bead `area:site` off one incidental word;
      4. the description, and ONLY when it names exactly ONE crate. A description that name-drops
         several neighbouring crates is a cross-cutting task, not a package assignment — that rule
         is what stopped "formalize codex-EC2 worker tooling" deriving `sparq-core,sparq-jsonld`.

    Returns [] when nothing is derivable; the caller then parks the issue `needs:area` rather than
    guessing. A wrong partition is worse than an explicit park: the park is maintainer-visible and
    the retriage cron re-promotes it once an area lands, whereas a wrong area silently routes the
    work. That readmission holds only for a park the sweep can SEE and whose author it trusts —
    the fetch is paginated for exactly this reason (retriage.py `_fetch_label`); its predecessor
    truncated at 500 and 219 of 719 parks were unreachable on every tick (issue #3831)."""
    crates = crate_names() if crates is None else crates
    title = bead.get("title") or ""
    low = title.lower()
    m = _TITLE_SCOPE.match(title)
    if m:
        a = _scope_to_area(m.group(1) or m.group(2), crates)
        if a:
            return [a]
    hits = _token_hits(crates, low)
    if hits:
        return hits[:limit]
    scope_region = low.split(":", 1)[0] if ":" in low[:60] else ""
    surf = [_SURFACE_AREAS[k] for k in _token_hits(_SURFACE_AREAS, scope_region)]
    if surf:
        return list(dict.fromkeys(surf))[:limit]
    desc_hits = _token_hits(crates, (bead.get("description") or "")[:400].lower())
    return desc_hits if len(desc_hits) == 1 else []


def issue_labels(bead, crates=None):
    labels = [lb["name"] if isinstance(lb, dict) else lb for lb in bead.get("labels", [])]
    mapped = (_map_label(lb) for lb in labels)              # blocked:* -> needs:* gating pass
    out = {lb for lb in mapped if lb and _pipeline_relevant(lb)}   # whitelist — drop free-form tags
    p = bead.get("priority")
    if isinstance(p, int) and 0 <= p <= 4:
        out.add(f"priority:P{p}")
    out.add(f"role:{role_for(bead)}")
    out.add(MIGRATION_LABEL)  # authenticates the bd-id body marker downstream — see MIGRATION_LABEL
    if externally_gated(bead):
        out.add("needs:user")
    # [OPUS-4.8] an epic is a tracking/umbrella issue, never a dispatchable work item — mark it
    # `kind:epic` so the readiness engine excludes it (else a worker would try to "implement" the
    # epic itself, producing a garbage PR). 57 of the 893 open beads are epics, 41 at P1 — they would
    # otherwise be selected FIRST by priority. Enforcement is in ready-issues.py; this is the signal.
    is_epic = str(bead.get("issue_type", "")).lower() == "epic"
    if is_epic:
        out.add("kind:epic")
    # Package partition (audit 2026-07-25) — see the derive_areas note. An epic is never
    # dispatched, so it needs no partition and is never needs:area-parked.
    if not is_epic and not any(lb.startswith("area:") for lb in out):
        derived = derive_areas(bead, crates)
        if derived:
            out |= {f"area:{a}" for a in derived}
        elif "needs:user" not in out:
            out.add("needs:area")
    return sorted(out)


def plan(issues, edges, include_closed=False):
    """The migration plan: which beads become issues, their labels, blocker edges, and parent links.
    `blocks` edges become readiness blockers; `parent-child` edges become parent (sub-issue) links —
    NOT readiness blockers (this fixes bd's sub-epic-leaf-invisibility, where a subtask showed as
    blocked merely because its epic was open)."""
    open_ids = {i: b for i, b in issues.items()
                if include_closed or str(b.get("status", "open")).lower() in ("open", "in_progress")}
    blockers, parents = {}, {}
    for edge in edges:
        frm, to = edge[0], edge[1]
        etype = edge[2] if len(edge) > 2 else "blocks"
        if frm not in open_ids or to not in open_ids:
            continue
        if etype == "parent-child":
            parents.setdefault(frm, []).append(to)
        elif etype == "blocks":
            blockers.setdefault(frm, []).append(to)
        # other edge types (related/discovered/duplicate/…) are NOT readiness blockers — skipped
    return open_ids, blockers, parents


# --- idempotent multi-pass --apply (review C2) ---------------------------------------------------
def _run(args, check=True):
    return subprocess.run(args, capture_output=True, text=True, check=check)


# Shape-constrained (audit 2026-07-25, issue #3797): the old `(\S+?)` capture matched ANY
# non-space run, so PROSE documenting the marker with an elided id ("<!-- bd-id:… -->" inside
# issues #2534/#2535/#3797) was scanned as a bead named "…" and injected into the migration map.
# Keep in sync with scripts/ci-close-merged-beads.py `_MARKER_RE`.
MARKER_RE = re.compile(r"<!--\s*bd-id:(sq-[0-9a-z]+(?:\.\d+)*)\s*-->")


def fetch_issues(repo):
    """Every issue (any state) with the fields the resume + reconcile passes need. ONE LIST call —
    the search index lags badly on this repo, so current state must come from the list API.

    `state` is carried for `--verify` only: a dependency edge whose endpoint issue is already
    CLOSED cannot hold anything, so the reconcile must not report it as a live gap."""
    out = _run(["gh", "issue", "list", "-R", repo, "--state", "all", "--limit", "10000",
                "--json", "number,id,body,labels,author,title,state"]).stdout
    return json.loads(out or "[]")


def _writer_logins(repo, logins, cache=None):
    """The subset of `logins` with write-or-better permission on `repo` — the SAME trust primitive
    triage-issue.yml uses. An unprivileged decoy author (the resolve_resume_map threat model)
    cannot be in this set, which is exactly what makes author provenance load-bearing."""
    cache = {} if cache is None else cache
    ok = set()
    for lg in sorted(set(logins)):
        if lg not in cache:
            if lg.endswith("[bot]"):
                # App bots are not collaborators; the orchestrator App is the migration's own
                # identity and no unprivileged user can author as a `[bot]` login.
                cache[lg] = lg in _MIGRATION_BOTS
            else:
                r = _run(["gh", "api", f"repos/{repo}/collaborators/{lg}/permission",
                          "--jq", ".permission"], check=False)
                cache[lg] = (r.stdout or "").strip() in ("admin", "maintain", "write")
        if cache[lg]:
            ok.add(lg)
    return ok


_MIGRATION_BOTS = {"sparq-orchestrator[bot]"}


def migration_owned(issues, writers):
    """bd-ids whose marker-only issue carries MIGRATION-OWNED provenance, i.e. all of:
      * the marker payload is a real bd-id SHAPE (MARKER_RE — kills the #3797 phantom);
      * the title is the migration's own `"<bd-id>: <bead title>"` format (a decoy that copies a
        marker to capture a bead id does not also reproduce the title contract);
      * the AUTHOR has write-or-better permission on the repo — the threat model's attacker is by
        definition unprivileged, so this closes the forgeable-body hole the label was added for;
      * exactly ONE issue in the repo carries that bd-id (ambiguity is never resolved by guessing).
    This recovers the ~750 issues of the 2026-07-17 bulk run that predate MIGRATION_LABEL: without
    it every subsequent `--apply` aborts on the fail-closed unverifiable check and the migration is
    permanently unresumable."""
    seen = {}
    for it in issues:
        mm = MARKER_RE.search(it.get("body") or "")
        if not mm:
            continue
        seen.setdefault(mm.group(1), []).append(it)
    owned = set()
    for bid, cands in seen.items():
        if len(cands) != 1:
            continue                                  # ambiguous — fail closed, never guess
        it = cands[0]
        login = (it.get("author") or {}).get("login") if isinstance(it.get("author"), dict) \
            else it.get("author")
        if login in writers and (it.get("title") or "").startswith(f"{bid}:"):
            owned.add(bid)
    return owned


def fetch_bd_map(repo, issues=None):
    """(trusted, unverified) {bd-id: issue_number} maps from existing issues carrying a
    `<!-- bd-id:... -->` marker (resume). trusted = the issue ALSO carries MIGRATION_LABEL
    (attaching one needs triage/write permission); unverified = marker-only — could be a
    pre-label legacy migration OR an attacker decoy, so never trusted on its own
    (see resolve_resume_map, PR #2528 review round 2)."""
    issues = fetch_issues(repo) if issues is None else issues
    trusted, unverified = {}, {}
    for it in issues:
        mm = MARKER_RE.search(it.get("body") or "")
        if not mm:
            continue
        labels = {lb["name"] if isinstance(lb, dict) else lb for lb in it.get("labels") or []}
        (trusted if MIGRATION_LABEL in labels else unverified)[mm.group(1)] = it["number"]
    return trusted, unverified


def resolve_resume_map(trusted, unverified, checkpoint_map, owned=frozenset()):
    """Pure resume resolution (PR #2528 review round 2). A body marker alone is FORGEABLE: an
    unprivileged user can pre-create a decoy issue carrying a target `<!-- bd-id:… -->` marker,
    and a resume path that accepted it would (a) never create the real issue and (b) stamp the
    authentication label onto attacker content — after which ci-close-merged-beads.py would
    trust and close the decoy. So an existing issue is accepted as already-migrated ONLY on
    migration-owned provenance: it already carries MIGRATION_LABEL, its exact
    (bd-id, issue_number) pair is recorded in the trusted on-disk checkpoint this migration
    wrote, or its bd-id is in `owned` (see migration_owned: unique marker + migration title
    format + a WRITE-permission author, which the unprivileged decoy author cannot satisfy).
    A marker-only issue with none of the three is returned as `unverifiable` and the caller FAILS
    CLOSED (no label, no mapping, no create) for operator review.

    Returns (id_map, backfill, unverifiable): id_map = accepted {bd-id: number}; backfill = the
    subset needing MIGRATION_LABEL stamped (checkpoint-verified but not yet labeled);
    unverifiable = marker-only {bd-id: number} matches with no provenance."""
    id_map = dict(trusted)
    backfill, unverifiable = {}, {}
    for bid, num in unverified.items():
        if bid in id_map:
            continue  # an authenticated issue exists; the marker-only copy is a decoy/dup — inert
        if checkpoint_map.get(bid) == num or bid in owned:
            id_map[bid] = num
            backfill[bid] = num
        else:
            unverifiable[bid] = num
    return id_map, backfill, unverifiable


# Agent self-identification (standing repo rule): every issue/comment an agent opens says so, so a
# maintainer scanning the tracker can tell machine-filed items from human ones at a glance.
SELF_ID = "> 🤖 SPARQ agent — migrated from the local `bd` tracker by `scripts/bd-to-issues.py`."


def _body(bead, bid):
    desc = (bead.get("description") or "").strip()
    return f"{SELF_ID}\n\n{desc}\n\n<!-- bd-id:{bid} -->\n"


def ensure_labels(repo, labels):
    """Idempotently create EVERY label the migration will use, BEFORE any issue create. Fails
    LOUDLY on any real failure: `gh issue create` errors on the first unknown label, and the old
    fallback dropped ALL labels — a silent label-less issue is permanently status:untriaged and
    loses its package partition. No --force: existing labels keep their curated colors."""
    failed = []
    for l in sorted(labels):
        r = _run(["gh", "label", "create", l, "-R", repo, "--color", "ededed",
                  "--description", "bd migration label"], check=False)
        if r.returncode != 0 and "already exists" not in (r.stderr or ""):
            failed.append(l)
    if failed:
        raise SystemExit(f"refusing --apply: {len(failed)} label(s) could not be created "
                         f"(fail-loud, no silent label drop): {failed[:10]}")


def _create_issue(repo, bid, bead, crates=None):
    args = ["gh", "issue", "create", "-R", repo, "--title", f"{bid}: {bead['title']}", "--body", _body(bead, bid)]
    for l in issue_labels(bead, crates):
        args += ["--label", l]
    r = _run(args, check=False)
    if r.returncode != 0:
        # FAIL LOUDLY (audit-2026-07-17): the old fallback retried with NO labels at all, silently
        # producing untriaged, partition-less issues. Labels are pre-created by ensure_labels, so a
        # failure here is a real problem the operator must see. The run is crash-resumable.
        raise SystemExit(f"issue create failed for {bid} (labels are pre-created; refusing the "
                         f"silent label-drop fallback): {(r.stderr or '').strip()[:300]}")
    return int(re.search(r"/issues/(\d+)", r.stdout).group(1))


def _snapshot_body(issues, num):
    """The issue body already held in the run's single LIST snapshot, or None if unknown."""
    if num is None:
        return None
    for it in issues:
        if it["number"] == num:
            return it.get("body") or ""
    return None


def _ensure_markers(repo, num, markers, body=None):
    """Append every missing edge marker to the issue body in ONE edit (idempotent). Returns True
    if it wrote.

    Two reasons this is plural rather than a per-marker call: (a) an issue with both a Blocked-by
    and a Parent edge would otherwise be read-modify-written twice, and the second write built from
    a STALE body silently drops the first marker; (b) `body` short-circuits the per-issue read using
    the run's single LIST snapshot — pass 2 walks 558 edges, so a no-op re-run used to cost 558 API
    reads just to discover it had nothing to do. A no-op re-run must be cheap, or nobody runs it."""
    if not markers:
        return False
    if body is None:
        body = json.loads(_run(["gh", "issue", "view", str(num), "-R", repo,
                                "--json", "body"]).stdout)["body"] or ""
    missing = [m for m in markers if m not in body]
    if not missing:
        return False
    _run(["gh", "issue", "edit", str(num), "-R", repo,
          "--body", body.rstrip() + "\n" + "\n".join(missing) + "\n"])
    return True


# --- ADD-only label reconciliation, batched (audit 2026-07-25) -----------------------------------
# Label families the pipeline treats as SINGLE-VALUED. `role:` — triage.py enforces a single-role
# invariant explicitly so the dispatcher's resolve() never sees an ambiguous role set; `priority:` —
# ready-issues.py's valid_priority() returns None (i.e. NOT dispatchable) when two are present;
# `area:` — a second area puts one issue in two package partitions. So for these families the
# reconcile must ADD only when the family is ABSENT, never add a second member. A live run proved
# the point before this guard existed: it put `role:impl` on issues triage had already routed
# `role:ci` / `role:soundness`, which would have made them undispatchable.
_SINGLE_VALUED = ("role:", "priority:", "area:")
# [OPUS-5] Labels the reconcile may set at CREATION but must NEVER re-apply to a live issue.
# "ADD-only, so curation survives" is false for a PARK label: re-adding `needs:user` does not
# preserve a human decision, it OVERRIDES one. `needs:user` is terminal (ready-issues.py
# PARKED_AREA_LABELS) and `externally_gated()` deliberately over-gates, so before this guard the
# next migration run would have silently re-parked every issue the authorised re-triage pass
# (scripts/retriage-needs-user.py, maintainer authorisation sparq-org/sparq#1135) demoted — 92
# issues, back behind the front door, with no record. Removing the hold IS the curation.
_NEVER_RECONCILE = frozenset({"needs:user"})


def plan_reconcile(open_ids, id_map, have_labels, crates=None):
    """{issue_number: [labels to ADD]} for already-mapped OPEN beads whose issue is missing labels
    the plan requires. ADD-only by construction — never removes, so a human's or triage's curation
    survives. A single-valued family already present on the issue is left completely alone (see
    _SINGLE_VALUED); in particular an issue that already carries an `area:` keeps it and is not
    `needs:area`-parked either, because it is already classified. `_NEVER_RECONCILE` labels are
    never re-applied at all — for a park label, "add it back" is a revert, not a reconcile."""
    out = {}
    for bid, num in sorted(id_map.items()):
        bead = open_ids.get(bid)
        if bead is None:
            continue                                   # bead is closed/absent — not our business
        have = set(have_labels.get(num) or ())
        want = set(issue_labels(bead, crates))
        for fam in _SINGLE_VALUED:
            if any(lb.startswith(fam) for lb in have):
                want = {lb for lb in want if not lb.startswith(fam)}
        if any(lb.startswith("area:") for lb in have):
            want.discard("needs:area")                 # already classified — do not park it
        want -= _NEVER_RECONCILE                       # never re-park a deliberately-demoted issue
        gap = want - have
        if gap:
            out[num] = sorted(gap)
    return out


def _chunks(seq, n):
    seq = list(seq)
    return [seq[i:i + n] for i in range(0, len(seq), n)]


def _label_node_ids(repo, names):
    """{label name: node id} via GraphQL (the REST/`gh label` surfaces do not expose node ids)."""
    owner, name = repo.split("/", 1)
    q = ('query($o:String!,$r:String!,$c:String){repository(owner:$o,name:$r){'
         'labels(first:100,after:$c){nodes{id name} pageInfo{hasNextPage endCursor}}}}')
    ids, cursor = {}, None
    while True:
        args = ["gh", "api", "graphql", "-f", f"query={q}", "-F", f"o={owner}", "-F", f"r={name}"]
        args += ["-F", f"c={cursor}"] if cursor else ["-F", "c="]
        page = json.loads(_run(args).stdout)["data"]["repository"]["labels"]
        ids.update({n["name"]: n["id"] for n in page["nodes"]})
        if not page["pageInfo"]["hasNextPage"]:
            break
        cursor = page["pageInfo"]["endCursor"]
    missing = set(names) - set(ids)
    if missing:
        raise SystemExit(f"label node ids missing (run ensure_labels first): {sorted(missing)[:10]}")
    return ids


def _split_chunk(chunk, half=None):
    """Split an over-limit chunk STRICTLY downward, or fail loud.

    [OPUS-5] The split must SHRINK. GitHub executes the leading aliases before refusing an
    oversize mutation, and these mutations are ADD-only, so a chunk re-queued at its original
    size loops forever while re-applying its prefix on every pass — the process never exits and
    never errors, it just spins. Review round 2 measured exactly that: mutating the halving
    HUNG the self-test instead of failing it, which is a weak kill (a hang reads as a stuck
    runner, not a caught defect). The invariant below turns that into a loud SystemExit.

    `half` is injectable ONLY so the guard is reachable from the self-test. A post-condition no
    input can violate is a tripwire nobody can prove still works — deleting it would red nothing
    (measured: it survived mutation until this seam existed). Production always uses the default.
    """
    half = max(1, len(chunk) // 2) if half is None else half
    halves = [chunk[:half], chunk[half:]]
    if sum(len(h) for h in halves) != len(chunk) or any(len(h) >= len(chunk) for h in halves):
        raise SystemExit(f"label reconcile: non-progressing split of {len(chunk)} -> "
                         f"{[len(h) for h in halves]} — would loop while partially applying")
    return halves


def apply_reconcile(repo, plan, node_ids, batch=8, pause=2.0, log=print):
    """Apply `plan` ({issue_number: [labels]}) with ONE GraphQL request per `batch` issues.

    Batching is not cosmetic: ~800 single `gh issue edit` mutations trip GitHub's secondary
    rate limiter, and a 403 mid-run leaves the repo half-labeled. Aliased mutations keep the
    whole reconcile inside ~100 requests, paced under the documented GraphQL mutation budget
    (5 points/mutation, 2000 points/min).

    Two distinct server refusals, two distinct responses (both learned from a real run):
      * `Resource limits for this query exceeded` — the REQUEST is too big. A batch of 20 hit
        this on the very first chunk, and GitHub had already executed the first few aliases
        before refusing, so a naive retry of the same shape would loop forever while partially
        applying. HALVE the chunk and retry; ADD-only mutations make the replay harmless.
      * a secondary-rate-limit 403 — the request is fine but too frequent. Back off, don't split.
    """
    if not plan:
        return 0
    label_ids = _label_node_ids(repo, {l for v in plan.values() for l in v})
    queue, done, total = _chunks(sorted(plan), batch), 0, len(plan)
    while queue:
        chunk = queue.pop(0)
        muts = []
        for i, num in enumerate(chunk):
            lids = ",".join(f'"{label_ids[l]}"' for l in plan[num])
            muts.append(f'm{i}: addLabelsToLabelable(input: {{labelableId: "{node_ids[num]}", '
                        f'labelIds: [{lids}]}}) {{ __typename }}')
        query = "mutation {\n  " + "\n  ".join(muts) + "\n}"
        applied = False
        for attempt in range(5):
            r = _run(["gh", "api", "graphql", "-f", f"query={query}"], check=False)
            if r.returncode == 0 and '"errors"' not in (r.stdout or ""):
                applied = True
                break
            err = ((r.stderr or "") + (r.stdout or "")).lower()
            if "resource limit" in err or "query is too large" in err:
                if len(chunk) == 1:
                    raise SystemExit(f"label reconcile: issue #{chunk[0]} is irreducibly over the "
                                     f"GraphQL resource limit: {err[:300]}")
                halves = _split_chunk(chunk)          # strictly downward, or a loud SystemExit
                log(f"  request over the GraphQL resource limit — splitting {len(chunk)} -> "
                    f"{len(halves[0])}")
                queue[:0] = halves
                break
            if "secondary rate" in err or "abuse" in err or "was submitted too quickly" in err:
                wait = 30 * (2 ** attempt)
                log(f"  secondary rate limit — backing off {wait}s")
                time.sleep(wait)
                continue
            raise SystemExit(f"label reconcile failed on issues {chunk}: "
                             f"{((r.stderr or '') + (r.stdout or '')).strip()[:400]}")
        else:
            raise SystemExit(f"label reconcile: still rate-limited after 5 backoffs at {chunk}")
        if not applied:
            continue                       # split happened; the halves are back on the queue
        done += len(chunk)
        log(f"  reconciled {done}/{total} issue(s)")
        time.sleep(pause)
    return done


def apply_migration(repo, open_ids, blockers, parents, limit=None, checkpoint="/tmp/bd-migration-map.json",
                    reconcile=True, crates=None):
    """Resumable: pass 1 upserts issues by bd-id marker (checkpointing the map); pass 2 idempotently
    adds Blocked-by:/Parent: markers; pass 3 ADD-only label reconcile over already-mapped issues.
    Re-running creates nothing new and reconciles nothing."""
    issues_snapshot = fetch_issues(repo)
    node_ids = {it["number"]: it["id"] for it in issues_snapshot}
    have_labels = {it["number"]: [lb["name"] if isinstance(lb, dict) else lb
                                  for lb in it.get("labels") or []] for it in issues_snapshot}
    trusted, unverified = fetch_bd_map(repo, issues_snapshot)
    try:
        ckpt = json.load(open(checkpoint, encoding="utf-8"))
    except (FileNotFoundError, json.JSONDecodeError):
        ckpt = {}
    authors = {((it.get("author") or {}).get("login") if isinstance(it.get("author"), dict)
                else it.get("author")) for it in issues_snapshot}
    owned = migration_owned(issues_snapshot, _writer_logins(repo, {a for a in authors if a}))
    id_map, backfill, unverifiable = resolve_resume_map(trusted, unverified, ckpt, owned)
    ids = list(open_ids)[: limit] if limit else list(open_ids)
    # Fail closed (PR #2528 review round 2): a marker-only issue with NO migration provenance
    # (not labeled, not in the trusted checkpoint) may be an attacker decoy pre-created to
    # capture a bd-id. Refuse to label it, map to it, OR create a competing issue — stop for
    # operator review instead.
    bad = {b: n for b, n in unverifiable.items() if b in set(ids)}
    if bad:
        raise SystemExit(
            f"refusing --apply: {len(bad)} existing issue(s) carry a bd-id body marker but have "
            f"NO migration provenance (no '{MIGRATION_LABEL}' label, not in the checkpoint) — "
            f"possible decoys; review + label or close them manually: "
            f"{sorted((b, '#' + str(n)) for b, n in bad.items())[:10]}")
    # Pre-flight: the FULL label set exists before the first create (fail-loud, item 10).
    ensure_labels(repo, {l for bid in ids for l in issue_labels(open_ids[bid], crates)})
    created, fresh = 0, {}
    for bid in ids:                                  # pass 1
        if bid in id_map:
            continue      # already migrated; MIGRATION_LABEL backfill is pass 3's job (batched)
        id_map[bid] = _create_issue(repo, bid, open_ids[bid], crates)
        created += 1
        fresh[bid] = _body(open_ids[bid], bid)
        have_labels[id_map[bid]] = issue_labels(open_ids[bid], crates)
        json.dump(id_map, open(checkpoint, "w"))     # checkpoint immediately (crash-resumable)
    edged = 0
    for bid in ids:                                  # pass 2 (idempotent)
        num = id_map.get(bid)
        markers = ([f"Blocked-by: #{id_map[b]}" for b in blockers.get(bid, []) if b in id_map]
                   + [f"Parent: #{id_map[p]}" for p in parents.get(bid, []) if p in id_map])
        if not markers:
            continue
        body = fresh.get(bid, _snapshot_body(issues_snapshot, num))
        edged += bool(_ensure_markers(repo, num, markers, body))
    # pass 3 (idempotent, ADD-only, batched): reconcile labels over every mapped OPEN bead. This
    # is also where `backfill` lands — MIGRATION_LABEL is part of every plan's label set, so the
    # ~750 pre-label issues of the 2026-07-17 bulk run get authenticated here in ~40 requests
    # rather than ~750 single edits (which trip the secondary rate limiter).
    reconciled = 0
    if reconcile:
        rplan = plan_reconcile({b: open_ids[b] for b in ids}, id_map, have_labels, crates)
        if rplan:
            ensure_labels(repo, {l for v in rplan.values() for l in v})
            if any(n not in node_ids for n in rplan):   # created THIS run — re-fetch their ids
                node_ids.update({it["number"]: it["id"] for it in fetch_issues(repo)})
            reconciled = apply_reconcile(repo, rplan, node_ids)
    if backfill:
        print(f"[apply] authenticated {len(backfill)} pre-label issue(s) via migration-owned "
              f"provenance (unique marker + migration title format + write-permission author)")
    return id_map, created, reconciled, edged


# --- post-migration verification, READ-ONLY (sq-gj35r / issue #3812) -----------------------------
# The bulk `--apply` was still inside pass 1 when the 2026-07-17 audit sampled the board, and pass 2
# — the `Blocked-by:` / `Parent:` body markers — runs ONLY after pass 1 finishes. A run that died
# mid pass-1 therefore leaves a board that looks migrated (the issues exist, labelled and mapped)
# and carries NO dependency edges at all. Nothing about that state is loud.
#
# `--apply` cannot answer "did it finish", because it is idempotent by design: a re-run repairs
# whatever it finds and then prints 0/0/0 — the same output it prints when nothing was ever wrong.
# So the completion question needs its own read-only pass, which is this one. It creates nothing,
# edits nothing, and reports the three things the audit asked for: counts reconcile, no bd-id maps
# to two issues, and every planned edge is actually present on the issue that carries it.
#
# The regexes below are the READ side of exactly what pass 2 writes. `_MARKER_BLOCKED_BY` in
# scripts/ready-issues.py is the OTHER reader of that same channel, and the self-test pins the two
# patterns equal: an edge the readiness engine cannot parse is not an edge, however well-formed it
# looks to the writer.
VERIFY_BLOCKED_BY_RE = re.compile(r"[Bb]locked-by:\s*#(\d+)")
VERIFY_PARENT_RE = re.compile(r"[Pp]arent:\s*#(\d+)")
_EDGE_RE = {"blocked-by": VERIFY_BLOCKED_BY_RE, "parent": VERIFY_PARENT_RE}
# The NATIVE dependency channel, as carried by GitHub's REST issue-list payload. ready-issues.py
# unions it with the marker channel, so it is what decides whether a marker gap is also a DISPATCH
# problem. It reports a COUNT, not blocker numbers — see verify_migration on why that can only ever
# downgrade the description of a gap, never erase one.
NATIVE_SUMMARY_FIELD = "issue_dependencies_summary"


def marker_index(issues):
    """(authenticated, marker_only): two {bd-id: [issue, ...]} maps over every issue carrying a
    shape-valid bd-id marker, split on MIGRATION_LABEL, duplicates KEPT in both.

    The split is the SAME trust boundary fetch_bd_map draws, and it is load-bearing here for the
    same reason: a body marker is FORGEABLE. An unprivileged user can pre-create one decoy issue
    per planned bead carrying the expected marker — and correctly-edged decoys are no harder to
    forge than bare ones. `--apply` already fails closed on exactly that shape (the `unverifiable`
    check in apply_migration refuses to map to a marker-only issue with no migration provenance),
    so `--verify` must not accept as migrated what `--apply` refuses to resume from; otherwise a
    board of decoys reports PASSED although the migration never created or authenticated anything.
    MIGRATION_LABEL is the stable POST-migration invariant to check against: pass 3 stamps it on
    every mapped bead, including the ~750 pre-label issues of the 2026-07-17 run (see `backfill`).
    A marker-only issue is therefore an unfinished migration or a decoy — never a pass.

    Duplicates are kept because "one bd-id, two issues" is a defect this reconcile looks for.
    fetch_bd_map collapses a repeated bd-id by dict assignment (last write wins). That is right for
    resume — resolve_resume_map re-derives provenance afterwards — but it is precisely the wrong
    shape for a count reconcile."""
    authenticated, marker_only = {}, {}
    for it in issues:
        mm = MARKER_RE.search(it.get("body") or "")
        if not mm:
            continue
        labels = {lb["name"] if isinstance(lb, dict) else lb for lb in it.get("labels") or []}
        (authenticated if MIGRATION_LABEL in labels else marker_only).setdefault(
            mm.group(1), []).append(it)
    return authenticated, marker_only


def _is_closed(issue):
    """True only on an explicit closed state. `gh issue list --json state` yields OPEN/CLOSED and
    REST yields open/closed; an ABSENT state reads as open, which is the fail-loud direction — an
    unknown-state endpoint is reported as a live gap rather than quietly excused as moot."""
    return str(issue.get("state") or "").lower() == "closed"


def verify_migration(open_ids, blockers, parents, issues, native_blockers=None):
    """Reconcile the migration PLAN against a board snapshot. PURE — the caller does the fetching.

    `open_ids`/`blockers`/`parents` are `plan()`'s output (both endpoints of every edge are already
    scoped to the migrated set). `issues` is a `fetch_issues` snapshot. `native_blockers` is the
    optional {issue_number: open native blocker count} map; None means the native channel was not
    consulted, which the report states rather than silently reading as "zero everywhere".

    Verdict rules, and why each is drawn where it is:

    * ONLY a MIGRATION_LABEL-carrying issue counts as a mapping (marker_index). A marker-only
      issue is a forgeable claim, not evidence the migration ran; `--apply` refuses to resume from
      one, so `--verify` must not pass on one. Such bd-ids are reported under `unauthenticated`
      and their beads stay `unmigrated`, i.e. the verdict fails closed. The remedy is the same
      re-run of `--apply`, whose pass 3 stamps the label (or stops for review if it is a decoy).
    * A bd-id found on TWO authenticated issues is NOT a mapping. It is reported as a duplicate and
      the bead counts as unmigrated, because choosing one of the two here would be guessing — the
      same fail-closed rule migration_owned already applies. A marker-only copy SHADOWING an
      authenticated issue is inert (resolve_resume_map already treats it so): the authenticated
      issue is canonical, and the copy is reported under `shadowed` for information only.
    * A missing marker is a missing marker even when the issue carries native edges. Pass 2 either
      wrote it or it did not, and the count-only native summary cannot confirm that one SPECIFIC
      edge exists. The native count is attached to the record so the operator can tell a
      cosmetic backfill from a real dispatch hole; it never launders the gap away.
    * An edge whose source or target issue is already CLOSED is recorded but does not fail the
      verdict: a closed issue is never dispatched and never holds anything, so no marker written
      there could change an outcome.
    """
    authenticated, marker_only = marker_index(issues)
    duplicates = {bid: sorted(it["number"] for it in cands)
                  for bid, cands in authenticated.items() if len(cands) > 1}
    by_bid = {bid: cands[0] for bid, cands in authenticated.items() if len(cands) == 1}
    unauthenticated = {bid: sorted(it["number"] for it in cands)
                       for bid, cands in marker_only.items() if bid not in authenticated}
    shadowed = {bid: sorted(it["number"] for it in cands)
                for bid, cands in marker_only.items() if bid in authenticated}
    unmigrated = sorted(b for b in open_ids if b not in by_bid)
    stray = sorted(b for b in by_bid if b not in open_ids)

    landed, missing, unresolvable = 0, [], []
    for kind, table in (("blocked-by", blockers), ("parent", parents)):
        for bid in sorted(table):
            for dep in sorted(set(table[bid])):
                if bid not in by_bid or dep not in by_bid:
                    unresolvable.append({"kind": kind, "from": bid, "to": dep})
                    continue
                src, dst = by_bid[bid], by_bid[dep]
                seen = {int(n) for n in _EDGE_RE[kind].findall(src.get("body") or "")}
                if dst["number"] in seen:
                    landed += 1
                    continue
                missing.append({
                    "kind": kind, "from": bid, "from_issue": src["number"],
                    "to": dep, "to_issue": dst["number"],
                    "moot": _is_closed(src) or _is_closed(dst),
                    "native_blockers": (None if native_blockers is None
                                        else native_blockers.get(src["number"], 0)),
                })
    live_missing = [m for m in missing if not m["moot"]]
    return {
        "planned": len(open_ids), "mapped": len(by_bid),
        "duplicates": duplicates, "unmigrated": unmigrated, "stray": stray,
        "unauthenticated": unauthenticated, "shadowed": shadowed,
        "edges_planned": landed + len(missing) + len(unresolvable),
        "edges_landed": landed, "edges_missing": missing, "edges_live_missing": live_missing,
        "edges_unresolvable": unresolvable,
        "native_consulted": native_blockers is not None,
        "ok": not duplicates and not unmigrated and not live_missing,
    }


def print_verify_report(rep, log=print, sample=8):
    """Human-readable rendering of a verify_migration report. Samples the long lists — the counts
    are the verdict, the samples are the starting point for the operator's own look."""
    log(f"[verify] planned (open/in_progress beads): {rep['planned']}  |  "
        f"mapped to exactly one '{MIGRATION_LABEL}'-labelled issue: {rep['mapped']}")
    log(f"[verify] planned edges: {rep['edges_planned']}  |  markers present: "
        f"{rep['edges_landed']}  |  absent: {len(rep['edges_missing'])} "
        f"({len(rep['edges_live_missing'])} on issue pairs that are both still open)")
    if rep["stray"]:
        log(f"[verify] {len(rep['stray'])} mapped bd-id(s) are not in the current plan "
            f"(bead closed or dropped since the run) — informational: {rep['stray'][:sample]}")
    if rep["duplicates"]:
        log(f"[verify] FAIL {len(rep['duplicates'])} bd-id(s) map to MORE THAN ONE issue "
            f"(never resolved by guessing — close or re-marker the extras):")
        for bid, nums in sorted(rep["duplicates"].items())[:sample]:
            log(f"          {bid} -> {['#' + str(n) for n in nums]}")
    if rep["unmigrated"]:
        log(f"[verify] FAIL {len(rep['unmigrated'])} planned bead(s) have NO unambiguous "
            f"authenticated issue — pass 1 did not finish: {rep['unmigrated'][:sample]}")
    if rep["unauthenticated"]:
        # These are NOT mappings, and saying so is the whole point: a marker without the label is
        # either an unfinished migration or a pre-created decoy, and this report cannot tell which.
        log(f"[verify] {len(rep['unauthenticated'])} bd-id(s) appear ONLY on issue(s) with no "
            f"'{MIGRATION_LABEL}' label — an unauthenticated body marker is not evidence of "
            f"migration (--apply refuses to resume from one). Any that are in the plan are counted "
            f"unmigrated above; re-run --apply to label the genuine ones or close the decoys:")
        for bid, nums in sorted(rep["unauthenticated"].items())[:sample]:
            log(f"          {bid} -> {['#' + str(n) for n in nums]}")
    if rep["shadowed"]:
        log(f"[verify] {len(rep['shadowed'])} bd-id(s) also appear on an unlabelled COPY of an "
            f"authenticated issue — inert (the labelled issue is canonical), informational: "
            f"{sorted(rep['shadowed'])[:sample]}")
    if rep["edges_unresolvable"]:
        log(f"[verify] {len(rep['edges_unresolvable'])} edge(s) have an unmigrated endpoint "
            f"(a consequence of the unmigrated beads above, not a separate defect)")
    if rep["edges_missing"]:
        log(f"[verify] {len(rep['edges_live_missing'])} live edge(s) missing their marker — "
            f"pass 2 did not run for them; re-run --apply to backfill idempotently:")
        for m in rep["edges_live_missing"][:sample]:
            nat = ("" if m["native_blockers"] is None else
                   f", issue carries {m['native_blockers']} native blocker(s)")
            log(f"          {m['kind']}: {m['from']} (#{m['from_issue']}) -> "
                f"{m['to']} (#{m['to_issue']}){nat}")
        moot = len(rep["edges_missing"]) - len(rep["edges_live_missing"])
        if moot:
            log(f"          (+{moot} on an already-closed issue — a marker there holds nothing)")
    # Both caveats are about how to read a GAP, so neither belongs on a report that has none.
    if rep["edges_missing"] and not rep["native_consulted"]:
        log("[verify] native dependency channel NOT consulted — the marker gaps above are reported "
            "as-is and may overstate the DISPATCH impact (ready-issues.py unions both channels).")
    elif any(m["native_blockers"] for m in rep["edges_live_missing"]):
        log("[verify] note: the native summary reports a COUNT, not blocker numbers, so a native "
            "edge can never confirm one SPECIFIC missing marker — only suggest the issue is held.")
    log(f"[verify] {'PASSED' if rep['ok'] else 'FAILED'}")
    return 0 if rep["ok"] else 1


def fetch_native_blockers(repo, ceiling=20000, log=print):
    """{issue_number: open native blocker count} for OPEN issues, or None when unavailable.

    Cursor-paginated (`gh api --paginate` follows Link headers) for the same reason
    ready-issues.py `_fetch` is: a single-page fetch fails closed at the page limit, and this
    backlog is past it. OPEN issues only — a closed issue is never dispatched.

    Returns None, not an all-zero map, when the fetch fails or no issue in the whole snapshot
    carries `issue_dependencies_summary`. Flattened to zeros, either state is indistinguishable
    from "nothing is blocked", and the report has to be able to say which one it is."""
    r = _run(["gh", "api", "--paginate", "--slurp",
              f"repos/{repo}/issues?state=open&per_page=100"], check=False)
    if r.returncode != 0:
        log(f"[verify] native dependency fetch failed ({(r.stderr or '').strip()[:200]})")
        return None
    rows = [it for page in json.loads(r.stdout or "[]") if isinstance(page, list)
            for it in page if isinstance(it, dict)]
    if len(rows) >= ceiling:
        raise SystemExit(f"refusing --verify: fetched {len(rows)} >= ceiling {ceiling} — the "
                         "snapshot looks runaway (fail-closed).")
    summaries = {it["number"]: it[NATIVE_SUMMARY_FIELD] for it in rows
                 if isinstance(it.get(NATIVE_SUMMARY_FIELD), dict)}
    if not summaries:
        log(f"[verify] no issue in the snapshot carries `{NATIVE_SUMMARY_FIELD}` — treating the "
            "native channel as unavailable rather than as zero blockers everywhere.")
        return None
    return {n: s.get("blocked_by") if isinstance(s.get("blocked_by"), int)
            and not isinstance(s.get("blocked_by"), bool) else 0
            for n, s in summaries.items()}


def _self_test():
    ok = True

    def chk(n, got, want):
        nonlocal ok
        good = got == want
        ok = ok and good
        print(f"  {'ok  ' if good else 'FAIL'} {n}: {got} (want {want})")

    # whitelist: keep area:/kind:/needs:/trust: + security-keyword labels; DROP free-form bd tags
    bead = {"priority": 2, "issue_type": "feature", "labels": [
        "area:sparq-core", "kind:test", "needs:user", "from:agent", "effort:M",
        "tier:fable", "roadmap", "federation", "noir"]}
    got = set(issue_labels(bead))
    chk("whitelist keeps pipeline labels", {"area:sparq-core", "kind:test", "needs:user"} <= got, True)
    chk("whitelist drops free-form tags", got & {"from:agent", "effort:M", "tier:fable", "roadmap",
                                                 "federation", "noir"}, set())
    chk("adds priority+role", {"priority:P2", "role:impl"} <= got, True)
    # every migrated issue carries the authentication label the autoclose mapping requires
    chk("stamps the migration authentication label", MIGRATION_LABEL in got, True)
    # security keyword survives the whitelist (soundness routing depends on it downstream)
    chk("keeps security keyword", "area:sparq-zk" in issue_labels(
        {"priority": 1, "issue_type": "feature", "labels": ["area:sparq-zk", "from:x"]}), True)
    # epic gets kind:epic
    chk("epic tagged", "kind:epic" in issue_labels(
        {"priority": 1, "issue_type": "epic", "labels": ["area:x"]}), True)
    # blocked:* -> needs:* gating pass (audit-2026-07-17)
    chk("blocked:ec2 maps to needs:ec2", _map_label("blocked:ec2"), "needs:ec2")
    chk("blocked-on-zk maps to needs:zk", _map_label("blocked-on-zk"), "needs:zk")
    chk("blocked-by-epic is dropped", _map_label("blocked-by-epic:sq-3kd2g"), None)
    chk("plain label passes through", _map_label("area:sparq-core"), "area:sparq-core")
    got = set(issue_labels({"priority": 1, "issue_type": "task",
                            "labels": ["blocked:ec2", "blocked-by-epic:sq-x", "area:bench"]}))
    chk("mapped gate survives whitelist", ("needs:ec2" in got, "blocked:ec2" in got,
                                           any("epic" in lb for lb in got)), (True, False, False))
    # external/audit gating: the sq-qhy4 class must land needs:user (durably parked)
    qhy4 = {"id": "sq-qhy4", "priority": 0, "issue_type": "task",
            "title": "[cert][cryptoreview] External accredited-cryptographer audit of the ZK "
                     "verifier + Noir circuits", "labels": []}
    chk("sq-qhy4 class gated needs:user", "needs:user" in issue_labels(qhy4), True)
    chk("curated id gated", externally_gated({"id": "sq-v286.8", "title": "x"}), True)
    chk("needs:user title gated", externally_gated(
        {"id": "sq-z", "title": "needs:user — flip GitHub Pages source"}), True)
    chk("agent-out-of-scope desc gated", externally_gated(
        {"id": "sq-z", "title": "plain", "description": "Agent-out-of-scope by definition."}), True)
    chk("plain feature not gated", externally_gated(
        {"id": "sq-a", "title": "feat(core): add a parser fast-path",
         "description": "speed up ingest"}), False)
    # Resume provenance (PR #2528 review round 2): an attacker-authored body-only marker that
    # PRECEDES migration must be neither label-backfilled nor accepted as the canonical mapping —
    # it surfaces as unverifiable and apply_migration fails closed for operator review.
    idm, bf, unv = resolve_resume_map({}, {"sq-target": 7}, {})
    chk("pre-migration decoy is not accepted as the mapping", "sq-target" in idm, False)
    chk("pre-migration decoy is never label-backfilled", bf, {})
    chk("pre-migration decoy surfaces as unverifiable (fail closed)", unv, {"sq-target": 7})
    # A decoy alongside the real labeled issue is inert: the labeled issue wins, no backfill.
    idm, bf, unv = resolve_resume_map({"sq-real": 5}, {"sq-real": 6}, {})
    chk("labeled issue is canonical; shadowing decoy is inert",
        (idm, bf, unv), ({"sq-real": 5}, {}, {}))
    # Checkpoint provenance: a pre-label legacy issue THIS migration created resumes + backfills…
    idm, bf, unv = resolve_resume_map({}, {"sq-legacy": 9}, {"sq-legacy": 9})
    chk("checkpoint-verified legacy issue resumes and backfills",
        (idm, bf, unv), ({"sq-legacy": 9}, {"sq-legacy": 9}, {}))
    # …but only on an EXACT (bd-id, number) match — a different issue number is unverifiable.
    idm, bf, unv = resolve_resume_map({}, {"sq-legacy": 9}, {"sq-legacy": 8})
    chk("checkpoint number mismatch stays unverifiable", (idm, bf, unv),
        ({}, {}, {"sq-legacy": 9}))

    # --- marker shape (audit 2026-07-25, #3797) --------------------------------------------------
    # Prose that DOCUMENTS the marker is not a bead mapping. The old `(\S+?)` capture scanned "…"
    # as a bead id on three real issues and injected that phantom into the migration map.
    chk("prose marker with an elided id does not scan",
        MARKER_RE.search("fetch_bd_map trusts `<!-- bd-id:… -->` markers"), None)
    chk("non-bead marker payload does not scan", MARKER_RE.search("<!-- bd-id:not-a-bead -->"), None)
    chk("dotted subtask ids still scan",
        MARKER_RE.search("<!-- bd-id:sq-7d3dj.32.2.3 -->").group(1), "sq-7d3dj.32.2.3")
    # [OPUS-5] _body was UNTESTED: reading a wrong/absent key (e.g. bead["body"] instead of
    # bead["description"]) silently ships every migrated issue with an EMPTY description —
    # content loss across a ~900-issue bulk run, with no error and a still-valid marker. The
    # fixture carries ONLY `description`, so a wrong-key read cannot pass by coincidence.
    body_out = _body({"description": "  the real bead prose  "}, "sq-body")
    chk("migrated body carries the bead description", "the real bead prose" in body_out, True)
    chk("migrated body is marker-addressable and self-identified",
        (MARKER_RE.search(body_out).group(1), body_out.startswith(SELF_ID)), ("sq-body", True))
    chk("a description-less bead still yields a valid marker body",
        MARKER_RE.search(_body({}, "sq-empty")).group(1), "sq-empty")

    # --- migration-owned provenance (audit 2026-07-25) -------------------------------------------
    # The 2026-07-17 bulk run predates MIGRATION_LABEL: ~750 marker-only issues. Without a third
    # provenance source every later --apply aborts on the fail-closed check, i.e. the migration is
    # unresumable. Provenance = unique marker + migration title format + WRITE-permission author;
    # the decoy threat model's attacker is unprivileged, so the author check is what closes the hole.
    legit = {"number": 42, "title": "sq-legit: do the thing",
             "body": "x\n<!-- bd-id:sq-legit -->\n", "author": {"login": "maintainer"}}
    # [OPUS-5] The decoy MUST satisfy the title contract. Review round 2 measured that the old
    # title ("please look at this") also violated the title check, so `chk("unprivileged decoy
    # author is NOT owned")` short-circuited there and NEVER reached `login in writers` — deleting
    # the author check outright left the whole self-test green. A decoy that copies a marker to
    # capture a bead id would obviously copy the title format too; the author check is the ONLY
    # thing standing between an unprivileged forger and a hijacked migration mapping, so it is
    # the one that has to be isolated here.
    decoy = {"number": 43, "title": "sq-victim: do the thing",
             "body": "<!-- bd-id:sq-victim -->", "author": {"login": "randomuser"}}
    wrongtitle = {"number": 44, "title": "unrelated title",
                  "body": "<!-- bd-id:sq-titleless -->", "author": {"login": "maintainer"}}
    prose_iss = {"number": 45, "title": "bug: scanner matches prose",
                 "body": "matches `<!-- bd-id:… -->`", "author": {"login": "maintainer"}}
    owned = migration_owned([legit, decoy, wrongtitle, prose_iss], {"maintainer"})
    chk("write-author + title contract is migration-owned", owned, {"sq-legit"})
    # ISOLATES the author check: the decoy now satisfies title + unique-marker, so the ONLY
    # remaining reason it is not owned is that its author lacks write permission.
    chk("title-contract-satisfying decoy from an unprivileged author is NOT owned",
        "sq-victim" in owned, False)
    chk("write author WITHOUT the title contract is NOT owned", "sq-titleless" in owned, False)

    # --- _writer_logins: the LOAD-BEARING half of the trust boundary (review round 2) ------------
    # migration_owned only consumes the `writers` set; the code that BUILDS it was untested, so
    # "every login is a writer" and "any [bot] is trusted" were both free mutations. Stub the
    # subprocess boundary (`_run`) so the permission contract is asserted without network.
    class _Resp:
        def __init__(self, stdout):
            self.stdout = stdout

    perms = {"maintainer": "admin", "triager": "write", "curator": "maintain",
             "randomuser": "read", "watcher": ""}
    api_calls = []

    def _stub_run(args, check=True):
        api_calls.append(args)
        assert args[0:2] == ["gh", "api"], args
        login = args[2].split("/")[-2]
        return _Resp(perms.get(login, "") + "\n")

    real_run = globals()["_run"]
    globals()["_run"] = _stub_run
    try:
        got = _writer_logins("o/r", ["maintainer", "triager", "curator", "randomuser", "watcher"])
        chk("only admin/maintain/write logins are writers", got,
            {"maintainer", "triager", "curator"})
        chk("a read-only collaborator is NOT a writer", "randomuser" in got, False)
        chk("an empty/absent permission is NOT a writer (fail closed)", "watcher" in got, False)
        # `[bot]` logins never hit the collaborators API — an App is not a collaborator — so the
        # allowlist is the ONLY gate. Trusting the suffix would let anyone who can create a
        # `*[bot]`-suffixed account author a decoy the migration then treats as its own.
        n_before = len(api_calls)
        bots = _writer_logins("o/r", ["sparq-orchestrator[bot]", "attacker[bot]"])
        chk("only the migration's own App bot is trusted", bots, {"sparq-orchestrator[bot]"})
        chk("an unlisted [bot] login is NOT a writer", "attacker[bot]" in bots, False)
        chk("bot logins are decided by the allowlist, not by the permissions API",
            len(api_calls) - n_before, 0)
        # the cache must not launder an untrusted login into a trusted one
        shared = {}
        _writer_logins("o/r", ["randomuser"], cache=shared)
        chk("a cached negative stays negative", _writer_logins("o/r", ["randomuser"],
                                                              cache=shared), set())
        chk("the cache records the verdict, not the raw permission", shared, {"randomuser": False})
    finally:
        globals()["_run"] = real_run

    # --- THE YAML SEAM: a self-test no workflow INVOKES is not a gate (review round 2) ----------
    # Measured on the merged tree: `bd-to-issues.py --self-test` was invoked by NO workflow, and
    # `ci-close-merged-beads.py --self-test` ran only POST-merge in bead-autoclose.yml
    # (pull_request_target: [closed], ref: main) — after the change it would have caught was
    # already on main. Every assertion above was therefore ungated on the PR that could break it.
    #
    # SCOPED ON PURPOSE. A whole-file substring search passes for the WRONG reason here, because
    # each filename appears in BOTH the `paths:` filter and the `run:` block — that exact vacuity
    # has already been measured twice on this repo's routing gate. So: count the filename inside
    # the `paths:` region only (exactly 2 — the pull_request filter AND the push filter), and
    # assert the `--self-test` invocation inside the steps region only.
    wf = os.path.join(os.path.dirname(os.path.abspath(__file__)),
                      "..", ".github", "workflows", "routing-self-tests.yml")
    wf_src = open(wf, encoding="utf-8").read()
    paths_region = wf_src[:wf_src.index("permissions:")]
    steps_region = wf_src[wf_src.index("    steps:"):]
    for script in ("scripts/bd-to-issues.py", "scripts/ci-close-merged-beads.py"):
        chk(f"{script} is a path trigger on BOTH pull_request and push",
            paths_region.count(f'"{script}"'), 2)
        chk(f"{script} --self-test is actually INVOKED, not merely path-filtered",
            f"python3 {script} --self-test" in steps_region, True)
    # [OPUS-5] GATING STATUS, guarded against the rule that is ACTUALLY live. ci-summary's
    # discovery changed on 2026-07-25 (#3773): a check is non-gating iff it is EXPLICITLY
    # DECLARED in .github/advisory-registry.json, keyed on workflow file + job id. The old
    # `\b(advisory|informational)\b` NAME rule is gone — it had silently neutralised four real
    # gates — so a rename can no longer demote anything, and asserting on the job NAME would
    # guard a rule that no longer exists. The real demotion path is a registry entry, so that
    # is what is asserted: this job must not be declared advisory.
    registry = json.load(open(os.path.join(os.path.dirname(os.path.abspath(__file__)),
                                           "..", ".github", "advisory-registry.json"),
                              encoding="utf-8"))
    declared = {(e.get("workflow"), e.get("job_id")) for e in registry.get("jobs", {}).values()}
    chk("the routing gate is NOT declared advisory (a declaration is what demotes it now)",
        ("routing-self-tests.yml", "validate") in declared, False)
    # merge_group cannot carry a paths filter; without the trigger the queue ref never exposes
    # this gating check and the merge queue would merge past it.
    chk("merge_group trigger present (the queue ref must expose the gate)",
        bool(re.search(r"(?m)^  merge_group:", wf_src)), True)
    # The two scripts share a marker regex that must not drift. Pin that they are gated
    # TOGETHER, so a change to either re-runs both.
    chk("MARKER_RE stays in sync with ci-close-merged-beads _MARKER_RE",
        MARKER_RE.pattern,
        re.search(r"_MARKER_RE = re\.compile\(r\"(.+?)\"\)",
                  open(os.path.join(os.path.dirname(os.path.abspath(__file__)),
                                    "ci-close-merged-beads.py"), encoding="utf-8").read()).group(1))
    dup = [dict(legit), dict(legit, number=46)]
    chk("two issues sharing a bd-id are never owned (fail closed)",
        migration_owned(dup, {"maintainer"}), set())
    idm, bf, unv = resolve_resume_map({}, {"sq-legit": 42}, {}, {"sq-legit"})
    chk("owned marker-only issue resumes and backfills the auth label",
        (idm, bf, unv), ({"sq-legit": 42}, {"sq-legit": 42}, {}))
    idm, bf, unv = resolve_resume_map({}, {"sq-victim": 43}, {}, {"sq-legit"})
    chk("un-owned marker-only issue still fails closed",
        (idm, bf, unv), ({}, {}, {"sq-victim": 43}))

    # --- area (package) derivation (audit 2026-07-25) --------------------------------------------
    # A no-area issue reserves the readiness engine's serializing __global__ partition and triage.py
    # refuses to promote it, so it is TERMINALLY status:untriaged — present in GitHub, invisible to
    # the orchestrator. 551 of the 895 open beads had no area label.
    cr = ("sparq-core", "sparq-jsonld", "sparq-reason", "sparq-reason-el", "sparq-solid",
          "sparq-zk")
    chk("conventional verb(scope): prefix wins",
        derive_areas({"title": "bug(sparq-solid): ODRL bridge drops a rule",
                      "description": "touches sparq-core too"}, cr), ["sparq-solid"])
    chk("bare scope: prefix maps a surface",
        derive_areas({"title": "site: honest competitive feature matrix"}, cr), ["site"])
    chk("title crate mention beats description mention",
        derive_areas({"title": "widen the sparq-zk verifier",
                      "description": "sparq-core and sparq-solid are involved"}, cr), ["sparq-zk"])
    chk("longest crate name wins over its own prefix",
        derive_areas({"title": "sparq-reason-el RBox regularity check"}, cr), ["sparq-reason-el"])
    chk("description is the last resort",
        derive_areas({"title": "no scope here at all",
                      "description": "the fix lands in sparq-core"}, cr), ["sparq-core"])
    chk("nothing derivable -> [] (never guess)",
        derive_areas({"title": "LinkedIn advert post", "description": "enumerate claims"}, cr), [])
    # precision regressions found by sampling the real backlog (audit 2026-07-25)
    chk("a multi-crate description name-drop is NOT a package assignment",
        derive_areas({"title": "formalize codex-EC2 worker tooling",
                      "description": "moves sparq-core and sparq-jsonld notes"}, cr), [])
    chk("a surface word in running prose is not evidence",
        derive_areas({"title": "zkSPARQL spec: transcribe normative cores into the draft site"},
                     cr), [])
    chk("surface hits keep first-occurrence order, not alphabetical",
        derive_areas({"title": "deploy(site+docs): a /deploy surface"}, cr), ["site", "docs"])
    chk("first-occurrence ordering, longest-name-wins",
        _token_hits(("sparq-core", "sparq-reason", "sparq-reason-el"),
                    "sparq-reason-el then sparq-core"), ["sparq-reason-el", "sparq-core"])
    got = set(issue_labels({"id": "sq-a", "priority": 2, "issue_type": "task",
                            "title": "bug(sparq-solid): x", "labels": []}, cr))
    chk("derived area lands on the issue", "area:sparq-solid" in got, True)
    chk("derived area suppresses the needs:area park", "needs:area" in got, False)
    got = set(issue_labels({"id": "sq-b", "priority": 2, "issue_type": "task",
                            "title": "undecidable title", "labels": []}, cr))
    chk("no derivable area -> explicit needs:area park (not silent __global__)",
        "needs:area" in got, True)
    chk("an epic is never needs:area-parked (never dispatched)",
        "needs:area" in issue_labels({"id": "sq-c", "priority": 1, "issue_type": "epic",
                                      "title": "EPIC: umbrella", "labels": []}, cr), False)
    chk("a bd area label is never overridden by derivation",
        [lb for lb in issue_labels({"id": "sq-d", "priority": 2, "issue_type": "task",
                                    "title": "bug(sparq-solid): x",
                                    "labels": ["area:sparq-core"]}, cr)
         if lb.startswith("area:")], ["area:sparq-core"])

    # --- ADD-only label reconcile (audit 2026-07-25) ---------------------------------------------
    beads = {"sq-a": {"id": "sq-a", "priority": 2, "issue_type": "task",
                      "title": "bug(sparq-solid): x", "labels": []}}
    want = set(issue_labels(beads["sq-a"], cr))
    chk("reconcile plans exactly the missing labels",
        set(plan_reconcile(beads, {"sq-a": 10}, {10: []}, cr)[10]), want)
    chk("reconcile is a no-op once the labels are present",
        plan_reconcile(beads, {"sq-a": 10}, {10: sorted(want)}, cr), {})
    chk("reconcile never removes a label the issue gained elsewhere",
        plan_reconcile(beads, {"sq-a": 10}, {10: sorted(want) + ["status:ready", "role:site"]},
                       cr), {})
    chk("a human-curated area is preserved (no second area, no needs:area)",
        plan_reconcile({"sq-b": {"id": "sq-b", "priority": 2, "issue_type": "task",
                                 "title": "undecidable", "labels": []}},
                       {"sq-b": 11}, {11: ["area:sparq-core", "priority:P2", "role:impl",
                                           MIGRATION_LABEL]}, cr), {})
    chk("an unmapped/closed bead is never reconciled",
        plan_reconcile(beads, {"sq-zzz": 12}, {12: []}, cr), {})
    # [OPUS-5] `needs:user` is TERMINAL (ready-issues.py PARKED_AREA_LABELS) and externally_gated()
    # over-gates on purpose, so re-adding it to a live issue reverts a human's demotion instead of
    # preserving curation. The bead below is gated (its title trips _EXTERNAL_TITLE) and its issue
    # has had the hold removed — the reconcile must plan NOTHING for it. Without the guard this
    # would silently re-park all 92 issues the authorised re-triage demoted.
    gated_bead = {"sq-g": {"id": "sq-g", "priority": 2, "issue_type": "task",
                           "title": "needs:user — flip the thing", "labels": ["area:sparq-core"]}}
    chk("the gated bead really is externally gated (else the guard test is vacuous)",
        "needs:user" in issue_labels(gated_bead["sq-g"], cr), True)
    chk("a demoted issue is never re-parked with needs:user",
        plan_reconcile(gated_bead, {"sq-g": 16},
                       {16: ["area:sparq-core", "priority:P2", "role:impl", "status:ready",
                             MIGRATION_LABEL]}, cr), {})
    # Single-valued families (found by a LIVE run, not by review): the reconcile put role:impl on
    # issues triage had already routed role:ci / role:soundness. Two role labels break triage.py's
    # single-role invariant and two priorities make valid_priority() return None — either way the
    # issue stops being dispatchable, so the "repair" would have made things worse.
    chk("never adds a SECOND role: label (triage's single-role invariant)",
        plan_reconcile(beads, {"sq-a": 13}, {13: ["role:ci"]}, cr).get(13, []),
        ["area:sparq-solid", MIGRATION_LABEL, "priority:P2"])
    chk("never adds a SECOND priority: label (valid_priority would return None)",
        [lb for lb in plan_reconcile(beads, {"sq-a": 14}, {14: ["priority:P0"]}, cr).get(14, [])
         if lb.startswith("priority:")], [])
    chk("a missing single-valued family IS still filled in",
        "role:impl" in plan_reconcile(beads, {"sq-a": 15}, {15: ["area:x"]}, cr).get(15, []), True)
    # [OPUS-5] `area:` was in _SINGLE_VALUED but UNGUARDED — dropping it from the tuple redded
    # nothing, unlike its two siblings. The "human-curated area is preserved" case above passes
    # for a DIFFERENT reason (its bead derives no area at all, so the `needs:area` discard does
    # the work). This is the case that isolates the family suppression: the bead derives
    # area:sparq-solid while the issue already carries a different area, so a second area label
    # would put one issue in two package partitions — which dispatch-plan.py then collapses to
    # the serializing __global__ partition, i.e. strictly worse than leaving it alone.
    chk("never adds a SECOND area: label when the issue already has a different one",
        [lb for lb in plan_reconcile(beads, {"sq-a": 16}, {16: ["area:sparq-core"]},
                                     cr).get(16, []) if lb.startswith("area:")], [])
    chk("batching chunks evenly", [len(c) for c in _chunks(range(45), 20)], [20, 20, 5])
    chk("empty reconcile does zero API calls", apply_reconcile("o/r", {}, {}), 0)
    # Adaptive split on the server's `Resource limits for this query exceeded` refusal (a real
    # 20-mutation batch hit it on the first chunk). Every issue must still be applied EXACTLY
    # once overall, and a single irreducible issue must fail loud rather than loop.
    # [OPUS-5] Split-invariant assertions run BEFORE the apply_reconcile block on purpose: a
    # broken split makes apply_reconcile raise, which would abort _self_test mid-way and turn
    # every later assertion into an unnamed traceback instead of a named FAIL line.
    def _shrinks(n):
        try:
            return all(0 < len(h) < n for h in _split_chunk(list(range(n))))
        except SystemExit:
            return "SystemExit"
    chk("every split strictly shrinks (a non-progressing split loops forever)",
        [n for n in range(2, 65) if _shrinks(n) is not True], [])
    chk("a split preserves every element exactly once",
        _split_chunk([1, 2, 3, 4, 5]), [[1, 2], [3, 4, 5]])
    # The guard itself, exercised through the injectable seam — otherwise it is an unreachable
    # post-condition that could be deleted with nothing going red.
    for bad, why in ((5, "no shrink"), (0, "empty head, full tail"), (9, "tail dropped")):
        try:
            _split_chunk([1, 2, 3, 4, 5], half=bad)
            chk(f"non-progressing split is rejected ({why})", "returned", "SystemExit")
        except SystemExit:
            chk(f"non-progressing split is rejected ({why})", "SystemExit", "SystemExit")

    calls = []

    def fake_run(args, check=True):
        q = args[-1]
        n = q.count("addLabelsToLabelable")
        calls.append(n)
        big = n > 2
        return type("R", (), {"returncode": 1 if big else 0,
                              "stdout": "" if big else "{}",
                              "stderr": "Resource limits for this query exceeded" if big else ""})()

    real_run, real_ids = _run, _label_node_ids
    try:
        globals()["_run"] = fake_run
        globals()["_label_node_ids"] = lambda repo, names: {n: f"L_{n}" for n in names}
        plan5 = {n: ["role:impl"] for n in range(1, 6)}
        got = apply_reconcile("o/r", plan5, {n: f"I_{n}" for n in plan5}, batch=5, pause=0)
        chk("split-and-retry still applies every issue exactly once", got, 5)
        # 5 refused -> [2,3]; 2 applied; 3 refused -> [1,2]; 1 applied; 2 applied. A refused size
        # is NEVER re-issued at the same size (that would loop while partially applying).
        chk("oversize batch splits strictly downward, never retried at the same size",
            calls, [5, 2, 3, 1, 2])
        # CALL-SITE PIN: apply_reconcile must route its split through the guarded helper. An
        # inlined halving at the call site is correct TODAY and therefore invisible to every
        # assertion above — measured as a surviving mutant. Stub the helper with a sentinel and
        # require it to reach the caller: if the call site stops using it, nothing raises.
        sentinel = RuntimeError("split helper reached")

        def _boom(chunk, half=None):
            raise sentinel
        real_split = globals()["_split_chunk"]
        globals()["_split_chunk"] = _boom
        try:
            apply_reconcile("o/r", plan5, {n: f"I_{n}" for n in plan5}, batch=5, pause=0)
            chk("apply_reconcile splits via the guarded _split_chunk", "not called", "called")
        except RuntimeError as exc:
            chk("apply_reconcile splits via the guarded _split_chunk",
                "called" if exc is sentinel else repr(exc), "called")
        finally:
            globals()["_split_chunk"] = real_split
        calls.clear()
        globals()["_run"] = lambda a, check=True: type("R", (), {
            "returncode": 1, "stdout": "", "stderr": "Resource limits for this query exceeded"})()
        try:
            apply_reconcile("o/r", {1: ["role:impl"]}, {1: "I_1"}, batch=1, pause=0)
            chk("irreducible oversize issue fails loud", "no-exit", "SystemExit")
        except SystemExit:
            chk("irreducible oversize issue fails loud", "SystemExit", "SystemExit")
    finally:
        globals()["_run"], globals()["_label_node_ids"] = real_run, real_ids

    # --- post-migration verification (sq-gj35r / #3812) ------------------------------------------
    # The defect being pinned is an --apply that died inside pass 1: issues exist, dependency
    # markers silently do not. Every assertion below therefore separates "the board matches the
    # plan" from "the board merely has issues on it".
    # `labeled` is EXPLICIT because the label is what authenticates the marker: an issue built with
    # labeled=False is exactly the forgeable decoy shape --apply refuses to resume from, and the
    # verifier must refuse it too. Default True = the post-migration board pass 3 leaves behind.
    def _iss(num, bid, body_extra="", state="OPEN", labeled=True):
        return {"number": num, "title": f"{bid}: t", "state": state,
                "labels": [{"name": MIGRATION_LABEL}] if labeled else [],
                "body": f"prose\n<!-- bd-id:{bid} -->\n{body_extra}"}

    beads3 = {"sq-a": {"id": "sq-a"}, "sq-b": {"id": "sq-b"}, "sq-c": {"id": "sq-c"}}
    blk, par = {"sq-a": ["sq-b"]}, {"sq-a": ["sq-c"]}
    complete = [_iss(1, "sq-a", "Blocked-by: #2\nParent: #3"), _iss(2, "sq-b"), _iss(3, "sq-c")]
    rep = verify_migration(beads3, blk, par, complete)
    chk("a complete migration verifies clean",
        (rep["ok"], rep["mapped"], rep["edges_landed"], rep["edges_planned"]), (True, 3, 2, 2))
    chk("a clean verify consults nothing it was not given", rep["native_consulted"], False)
    # THE FORGED-BOARD SHAPE (review round 2). A body marker is forgeable, so an unprivileged user
    # can pre-create one issue per planned bead — WITH the right markers and the right edges, which
    # cost nothing extra to forge — and a marker-trusting verifier would report PASSED for a
    # migration that never ran. --apply already refuses to resume from these (the `unverifiable`
    # fail-closed check), so --verify must refuse them too. The ONLY difference from `complete`
    # above, which passes, is MIGRATION_LABEL: every bd-id here is unique and every edge lands.
    decoys = [_iss(1, "sq-a", "Blocked-by: #2\nParent: #3", labeled=False),
              _iss(2, "sq-b", labeled=False), _iss(3, "sq-c", labeled=False)]
    rep = verify_migration(beads3, blk, par, decoys)
    chk("unique, correctly-edged marker-only issues never verify as migrated",
        (rep["ok"], rep["mapped"], rep["unmigrated"]), (False, 0, ["sq-a", "sq-b", "sq-c"]))
    chk("the unauthenticated bd-ids are named, not silently dropped",
        rep["unauthenticated"], {"sq-a": [1], "sq-b": [2], "sq-c": [3]})
    chk("an unauthenticated marker is not a duplicate either (it is simply not a mapping)",
        (rep["duplicates"], rep["edges_landed"], len(rep["edges_unresolvable"])), ({}, 0, 2))
    lines = []
    chk("the forged board renders a FAILED verdict",
        print_verify_report(rep, log=lines.append), 1)
    chk("the report says the markers are unauthenticated",
        any(MIGRATION_LABEL in l and "not evidence of migration" in l for l in lines), True)
    # A marker-only COPY shadowing an authenticated issue is inert, exactly as resolve_resume_map
    # already treats it: the labelled issue is canonical, so the verdict is unchanged.
    rep = verify_migration(beads3, blk, par, complete + [_iss(9, "sq-b", labeled=False)])
    chk("a marker-only copy shadowing an authenticated issue is inert, not a failure",
        (rep["ok"], rep["mapped"], rep["shadowed"], rep["unauthenticated"]),
        (True, 3, {"sq-b": [9]}, {}))
    # THE DIED-MID-PASS-1 SHAPE: every issue present, not one marker written. The count reconcile
    # alone reports a perfectly healthy board here, which is exactly why the edge check exists.
    no_edges = [_iss(1, "sq-a"), _iss(2, "sq-b"), _iss(3, "sq-c")]
    rep = verify_migration(beads3, blk, par, no_edges)
    chk("issues present but NO markers is caught (the died-mid-pass-1 shape)",
        (rep["ok"], rep["mapped"], rep["edges_landed"], len(rep["edges_live_missing"])),
        (False, 3, 0, 2))
    chk("both edge kinds are reported, not just blocked-by",
        sorted(m["kind"] for m in rep["edges_live_missing"]), ["blocked-by", "parent"])
    # A marker pointing at the WRONG issue is not the planned edge. Substring-matching the marker
    # PREFIX would pass here, so the check resolves the target NUMBER.
    rep = verify_migration(beads3, blk, par,
                           [_iss(1, "sq-a", "Blocked-by: #99\nParent: #98"), _iss(2, "sq-b"),
                            _iss(3, "sq-c")])
    chk("a marker naming a different issue does not satisfy the edge",
        (rep["ok"], rep["edges_landed"]), (False, 0))
    # …and the two kinds do not satisfy each other: a Parent link is not a readiness blocker.
    rep = verify_migration(beads3, blk, par,
                           [_iss(1, "sq-a", "Parent: #2\nBlocked-by: #3"), _iss(2, "sq-b"),
                            _iss(3, "sq-c")])
    chk("a Parent marker never satisfies a blocked-by edge (or vice versa)",
        (rep["ok"], rep["edges_landed"]), (False, 0))
    # Case tolerance: ready-issues.py reads `[Bb]locked-by`, so the verifier must accept exactly
    # what that reader accepts — no more (nothing is gained by calling an unreadable edge present)
    # and no less (an edge the engine honours must not be reported as a gap).
    rep = verify_migration({"sq-a": {}, "sq-b": {}}, {"sq-a": ["sq-b"]}, {},
                           [_iss(1, "sq-a", "blocked-by:#2"), _iss(2, "sq-b")])
    chk("the verifier accepts every marker spelling the readiness engine accepts",
        (rep["ok"], rep["edges_landed"]), (True, 1))
    # Duplicates: one bd-id on two issues is never resolved by picking one.
    dup_board = [_iss(1, "sq-a", "Blocked-by: #2\nParent: #3"), _iss(2, "sq-b"), _iss(3, "sq-c"),
                 _iss(4, "sq-b")]
    rep = verify_migration(beads3, blk, par, dup_board)
    chk("a bd-id mapped to two issues fails and is named",
        (rep["ok"], rep["duplicates"]), (False, {"sq-b": [2, 4]}))
    chk("an ambiguous bd-id is NOT counted as mapped", ("sq-b" in rep["unmigrated"],
                                                        rep["mapped"]), (True, 2))
    chk("an edge through an ambiguous bd-id is unresolvable, not silently landed",
        (rep["edges_landed"], [e["to"] for e in rep["edges_unresolvable"]]), (1, ["sq-b"]))
    # Pass 1 incomplete: a planned bead with no issue at all.
    rep = verify_migration(beads3, blk, par, [_iss(1, "sq-a", "Blocked-by: #2"), _iss(2, "sq-b")])
    chk("a planned bead with no issue is reported unmigrated", rep["unmigrated"], ["sq-c"])
    chk("its edge is unresolvable rather than missing",
        (len(rep["edges_unresolvable"]), len(rep["edges_missing"])), (1, 0))
    # A mapped bd-id absent from the plan (bead closed since the run) is informational only.
    rep = verify_migration({"sq-a": {}}, {}, {}, [_iss(1, "sq-a"), _iss(2, "sq-zzz")])
    chk("a mapped bd-id outside the plan is informational, not a failure",
        (rep["ok"], rep["stray"]), (True, ["sq-zzz"]))
    # Moot edges: neither endpoint can act, so a marker there could not change any outcome.
    rep = verify_migration(beads3, blk, par,
                           [_iss(1, "sq-a"), _iss(2, "sq-b", state="CLOSED"),
                            _iss(3, "sq-c", state="CLOSED")])
    chk("a missing marker to a CLOSED issue is recorded but does not fail the verdict",
        (rep["ok"], len(rep["edges_missing"]), rep["edges_live_missing"]), (True, 2, []))
    chk("REST-cased and gh-cased closed states are both recognised",
        (_is_closed({"state": "closed"}), _is_closed({"state": "CLOSED"})), (True, True))
    # An ABSENT state is read as open — the fail-loud direction. Reading it as closed would let a
    # snapshot missing the field excuse every gap on the board at once.
    chk("an absent state is read as open (fail loud, never excuse the gap)",
        _is_closed({"number": 1}), False)
    # The native channel: a count-only summary annotates a gap, it never erases one.
    rep = verify_migration({"sq-a": {}, "sq-b": {}}, {"sq-a": ["sq-b"]}, {},
                           [_iss(1, "sq-a"), _iss(2, "sq-b")], native_blockers={1: 3})
    # Indexed defensively: a regression that empties this list must render as a named FAIL, not as
    # an IndexError that aborts _self_test and turns every later assertion into an unnamed
    # traceback — the same reason the split invariants run ahead of the apply_reconcile block.
    live = rep["edges_live_missing"]
    chk("a native edge annotates a missing marker but never launders it away",
        (rep["ok"], live[0]["native_blockers"] if live else "no missing edge reported",
         rep["native_consulted"]), (False, 3, True))
    # The printer is the only thing an operator actually sees; keep it out of the dead-code set.
    lines = []
    chk("the report renders and returns the verdict as an exit code",
        print_verify_report(verify_migration(beads3, blk, par, dup_board), log=lines.append), 1)
    chk("the rendered report names the duplicate bd-id",
        any("sq-b" in l and "#2" in l and "#4" in l for l in lines), True)
    lines = []
    chk("a clean report renders a zero exit code",
        print_verify_report(verify_migration(beads3, blk, par, complete), log=lines.append), 0)
    chk("a clean report says so", any("PASSED" in l for l in lines), True)
    chk("a clean report carries no how-to-read-a-gap caveat",
        any("NOT consulted" in l for l in lines), False)
    lines = []
    print_verify_report(verify_migration(beads3, blk, par, no_edges), log=lines.append)
    chk("a report WITH gaps says the native channel was not consulted",
        any("NOT consulted" in l for l in lines), True)
    # SYNC PIN, same reasoning as the MARKER_RE pin above: ready-issues.py is the other reader of
    # the marker channel, and routing-self-tests.yml triggers on BOTH files, so a drift on either
    # side reds this assertion on its own PR.
    chk("VERIFY_BLOCKED_BY_RE stays in sync with ready-issues.py _MARKER_BLOCKED_BY",
        VERIFY_BLOCKED_BY_RE.pattern,
        re.search(r'_MARKER_BLOCKED_BY = re\.compile\(r"(.+?)"\)',
                  open(os.path.join(os.path.dirname(os.path.abspath(__file__)),
                                    "ready-issues.py"), encoding="utf-8").read()).group(1))

    print("bd-to-issues self-test", "PASSED" if ok else "FAILED")
    return 0 if ok else 1


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--repo", default="sparq-org/sparq")
    ap.add_argument("--apply", action="store_true", help="actually create issues (bulk!) — held for go-ahead")
    ap.add_argument("--limit", type=int, help="migrate only the first N open beads (for testing)")
    ap.add_argument("--only", help="comma-separated bd-ids to migrate (curated pilot subset; ignores --limit)")
    ap.add_argument("--export-file", help="read bd export from a file instead of running bd")
    ap.add_argument("--no-reconcile", action="store_true",
                    help="skip the ADD-only label reconcile over already-migrated issues")
    ap.add_argument("--verify", action="store_true",
                    help="read-only post-migration reconcile: counts, duplicate bd-id mappings "
                         "and whether pass 2's dependency markers landed (exit 1 on any gap)")
    ap.add_argument("--no-native-check", action="store_true",
                    help="--verify: skip the native-dependency fetch (the report then says the "
                         "channel was not consulted instead of implying zero blockers)")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()

    if args.self_test:
        return _self_test()

    # Refuse rather than pick. --verify short-circuits before the apply block, so accepting both
    # would silently drop a requested bulk write — and "it printed a report" is not a signal an
    # operator would read as "your --apply did not run".
    if args.verify and args.apply:
        raise SystemExit("--verify (read-only) and --apply (bulk write) are mutually exclusive: "
                         "run --apply, then --verify to confirm it finished.")

    if args.export_file:
        lines = open(args.export_file, encoding="utf-8").read().splitlines()
    else:
        lines = subprocess.run(["bd", "export"], capture_output=True, text=True, check=True).stdout.splitlines()
    issues, edges = parse_export(lines)
    open_ids, blockers, parents = plan(issues, edges)
    if args.only:  # curated subset — keep only the named bd-ids (blockers/parents stay scoped to them)
        keep = {x.strip() for x in args.only.split(",") if x.strip()}
        missing = keep - set(open_ids)
        if missing:
            print(f"warning: --only ids not in the open set (skipped): {sorted(missing)}")
        open_ids = {k: v for k, v in open_ids.items() if k in keep}

    # --- summary (always) ---
    from collections import Counter
    prio = Counter(f"P{b.get('priority')}" for b in open_ids.values())
    roles = Counter(role_for(b) for b in open_ids.values())
    pkgs = Counter(lb[5:] for b in open_ids.values()
                   for lb in issue_labels(b) if lb.startswith("area:"))
    gated = sum(1 for b in open_ids.values() if "needs:user" in issue_labels(b))
    derived = sum(1 for b in open_ids.values()
                  if not any(lb.startswith("area:") for lb in b2i_bd_labels(b))
                  and any(lb.startswith("area:") for lb in issue_labels(b)))
    parked = sum(1 for b in open_ids.values() if "needs:area" in issue_labels(b))
    print(f"beads (all): {len(issues)}  |  to migrate (open/in_progress): {len(open_ids)}")
    print(f"blocker (blocks) edges among migrated: {sum(len(v) for v in blockers.values())}")
    print(f"parent-child links among migrated:    {sum(len(v) for v in parents.values())}")
    print(f"gated needs:user (external/audit/blocked:*): {gated}")
    print(f"area: derived from bead text (bd carried none): {derived}")
    print(f"no derivable area -> parked needs:area (maintainer-visible, never silent "
          f"__global__): {parked}")
    print(f"priority: {dict(sorted(prio.items()))}")
    print(f"roles:    {dict(roles.most_common())}")
    print(f"top packages: {dict(pkgs.most_common(8))}")
    if not open_ids:
        print("no open beads to migrate.")
        return 0
    sample = next(iter(open_ids.values()))
    print("sample issue payload:")
    print(f"  title : {sample['id']}: {sample['title'][:70]}")
    print(f"  labels: {issue_labels(sample)}")
    print(f"  blockers: {[f'{b}' for b in blockers.get(sample['id'], [])] or 'none'}")

    # --verify before --apply: it is read-only, and its verdict is what tells the operator whether
    # an --apply is needed at all. Reported over the SAME plan the summary above printed.
    if args.verify:
        native = None if args.no_native_check else fetch_native_blockers(args.repo)
        print()
        return print_verify_report(
            verify_migration(open_ids, blockers, parents, fetch_issues(args.repo), native))

    if not args.apply:
        print("\n[dry-run] nothing created. Re-run with --apply (after go-ahead) to bulk-create.")
        return 0

    tag = f" (limit {args.limit})" if args.limit else ""
    print(f"\n[apply] migrating to {args.repo}{tag} — idempotent three-pass...")
    id_map, created, reconciled, edged = apply_migration(
        args.repo, open_ids, blockers, parents, limit=args.limit,
        reconcile=not args.no_reconcile)
    print(f"[apply] created {created} new issue(s); {edged} issue(s) gained edge markers; "
          f"{reconciled} issue(s) label-reconciled; {len(id_map)} bd-ids mapped.")
    print("[apply] a second run with the same inputs must report 0/0/0 — that is the "
          "idempotence contract.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
