#!/usr/bin/env python3
# [OPUS-4.8] Dependency-graph audit + repair: bd blocker edges <-> GitHub NATIVE issue dependencies.
"""audit-issue-deps.py — measure, then repair, the dependency graph mirrored from bd into GitHub.

The migration (scripts/bd-to-issues.py) wrote blocker edges only as `Blocked-by: #NN` BODY MARKERS.
GitHub issue dependencies are now generally available, and the orchestrator's readiness engine reads
the NATIVE edges. This tool reconciles the two.

DIRECTION (the load-bearing detail — proven, not assumed):
  bd export emits `{"issue_id": X, "depends_on_id": Y, "type": "blocks"}`.
  `bd dep add <blocked> <blocker>` == "<blocked> depends on <blocker>", and `bd show X` renders the
  record under "DEPENDS ON -> Y" while `bd show Y` renders it under "BLOCKS <- X".
  => X is BLOCKED BY Y.
  => GitHub: issue(X).blocked_by must contain issue(Y).
  `--prove` re-derives this from the live tree and prints the witness pair.

EDGE TYPES: `blocks` and `blocked-by` are both blocker edges with the SAME orientation in the export
(both mean "issue_id depends on depends_on_id" — verified via `bd show`). bd-to-issues.py only
handled "blocks", silently dropping the `blocked-by` rows; this tool handles both.
`parent-child` is deliberately NOT a blocker (it becomes a GitHub sub-issue/parent link) — mirroring
it as blocked_by would reintroduce the sub-epic-leaf-invisibility bug the migration fixed.

DEFAULT is read-only (audit). `--apply` performs the repair. Idempotent: a second `--apply` is a
no-op (it re-reads live state and finds nothing to do).
"""
import argparse
import json
import re
import subprocess
import sys
import time
from collections import defaultdict

REPO = "sparq-org/sparq"
MIGRATION_LABEL = "bd-migration"
BLOCKER_TYPES = ("blocks", "blocked-by")
OPEN_STATES = ("open", "in_progress")
MARKER = re.compile(r"<!--\s*bd-id:(\S+?)\s*-->")


# --- shelling out -------------------------------------------------------------------------------
def run(args, check=True, retries=5):
    """gh wrapper with secondary-rate-limit backoff."""
    delay = 2.0
    for attempt in range(retries):
        r = subprocess.run(args, capture_output=True, text=True)
        if r.returncode == 0:
            return r
        err = (r.stderr or "") + (r.stdout or "")
        if re.search(r"secondary rate limit|rate limit exceeded|abuse detection|was submitted too quickly", err, re.I):
            time.sleep(delay)
            delay *= 2
            continue
        if attempt < retries - 1 and re.search(r"HTTP 50[0-9]|timeout|TLS handshake", err, re.I):
            time.sleep(delay)
            delay *= 2
            continue
        if check:
            raise SystemExit(f"command failed: {' '.join(args[:6])}...\n{err[:800]}")
        return r
    raise SystemExit(f"gave up after {retries} attempts: {' '.join(args[:6])}")


# --- bd side ------------------------------------------------------------------------------------
def bd_beads(export_file=None):
    """{bd-id: bead} from `bd export` (all statuses)."""
    if export_file:
        lines = open(export_file, encoding="utf-8").read().splitlines()
    else:
        lines = run(["bd", "export"]).stdout.splitlines()
    beads = {}
    for line in lines:
        line = line.strip()
        if not line:
            continue
        d = json.loads(line)
        if d.get("_type", "issue") == "issue" and "id" in d:
            beads[d["id"]] = d
    return beads


def bd_blocker_edges(beads):
    """{(blocked_bd_id, blocker_bd_id)} — X blocked-by Y — over ALL beads, any status."""
    edges = set()
    for bid, b in beads.items():
        for dep in b.get("dependencies") or []:
            if not isinstance(dep, dict):
                continue
            if dep.get("type") not in BLOCKER_TYPES:
                continue
            to = dep.get("depends_on_id") or dep.get("depends_on") or dep.get("to")
            frm = dep.get("issue_id") or bid
            if to and frm and to != frm:
                edges.add((frm, to))
    return edges


def bd_is_open(bead):
    return str((bead or {}).get("status", "")).lower() in OPEN_STATES


# --- GitHub side --------------------------------------------------------------------------------
def gh_issues():
    """All non-PR issues: {number: {...}} via the LIST API (the search index lags — never use it)."""
    out = run(["gh", "api", "--paginate", f"repos/{REPO}/issues?state=all&per_page=100", "--slurp"]).stdout
    flat = [x for page in json.loads(out) for x in page]
    return {x["number"]: {"state": x["state"], "title": x.get("title") or "",
                          "labels": {l["name"] for l in x.get("labels") or []},
                          "body": x.get("body") or "",
                          "author": (x.get("user") or {}).get("login") or "",
                          "assoc": x.get("author_association") or "",
                          "id": x["id"]}
            for x in flat if "pull_request" not in x}


BD_ID = re.compile(r"^sq-[0-9a-z]+(\.[0-9]+)*$")
# Repo principals that can author an issue. A `<!-- bd-id: -->` body marker is FORGEABLE, so it is
# never trusted alone. bd-to-issues.py authenticated it with the `bd-migration` LABEL, but that label
# was only ever stamped on the first 137-issue pilot (issues #2407-#2657, 2026-07-17 morning); the
# remaining 755 migrated issues were created by a later run that never applied it (repo issues #2534
# / #2535 filed exactly this gap and were closed without the backfill landing). Authenticating on
# the label alone therefore discards 85% of the real graph.
# The rule used here is strictly stronger than "marker present" and needs the same privilege the
# label did: the issue must be authored by a repo MEMBER/OWNER (or a repo App), the marker must match
# the bd-id grammar, AND the issue TITLE must independently carry the same `<bd-id>:` prefix that
# bd-to-issues.py::_create_issue writes. An outside contributor cannot satisfy the author check, so a
# decoy cannot capture a bd-id.
TRUSTED_ASSOC = {"MEMBER", "OWNER", "COLLABORATOR"}
TRUSTED_AUTHORS = {"sparq-orchestrator[bot]"}


def bd_id_map(issues):
    """{bd-id: issue_number} from body markers with MIGRATION PROVENANCE (see TRUSTED_ASSOC).

    Returns (map, unverified, collisions). `unverified` = a marker we refuse to trust."""
    trusted, unverified, collisions = {}, {}, defaultdict(list)
    for num, it in sorted(issues.items()):
        m = MARKER.search(it["body"])
        if not m:
            continue
        bid = m.group(1)
        privileged = it["assoc"] in TRUSTED_ASSOC or it["author"] in TRUSTED_AUTHORS
        if not (privileged and BD_ID.fullmatch(bid) and it["title"].startswith(bid + ":")):
            unverified.setdefault(bid, []).append(num)
            continue
        collisions[bid].append(num)
        # deterministic winner: the LOWEST issue number (the original create; later ones are dups)
        if bid not in trusted or num < trusted[bid]:
            trusted[bid] = num
    return trusted, unverified, {b: n for b, n in collisions.items() if len(n) > 1}


GQL = """
query($owner:String!, $name:String!, $cursor:String) {
  repository(owner:$owner, name:$name) {
    issues(first:50, after:$cursor, orderBy:{field:CREATED_AT, direction:ASC}) {
      pageInfo { hasNextPage endCursor }
      nodes {
        number
        blockedBy(first:50) { totalCount nodes { number } }
      }
    }
  }
}"""


def gh_native_blocked_by():
    """{issue_number: set(blocker_numbers)} for the whole repo, via GraphQL (bulk, 50/page)."""
    owner, name = REPO.split("/")
    out, cursor, truncated = {}, None, []
    while True:
        args = ["gh", "api", "graphql", "-f", f"query={GQL}", "-F", f"owner={owner}", "-F", f"name={name}"]
        if cursor:
            args += ["-F", f"cursor={cursor}"]
        data = json.loads(run(args).stdout)
        if "errors" in data and data["errors"]:
            raise SystemExit(f"graphql: {json.dumps(data['errors'])[:600]}")
        page = data["data"]["repository"]["issues"]
        for n in page["nodes"]:
            bb = n.get("blockedBy") or {}
            nodes = {x["number"] for x in (bb.get("nodes") or [])}
            if bb.get("totalCount", 0) > len(nodes):
                truncated.append(n["number"])
            if nodes:
                out[n["number"]] = nodes
        if not page["pageInfo"]["hasNextPage"]:
            break
        cursor = page["pageInfo"]["endCursor"]
    if truncated:
        print(f"WARNING: blockedBy page cap hit on issues {truncated[:10]} — rerun with a larger page")
    return out


def _issue_id(num, issues):
    """The REST database id of an issue (the POST/DELETE body takes the *id*, not the number)."""
    if num in issues and issues[num].get("id"):
        return issues[num]["id"]
    return json.loads(run(["gh", "api", f"repos/{REPO}/issues/{num}"]).stdout)["id"]


def add_blocked_by(blocked_num, blocker_num, issues):
    """issue(blocked_num).blocked_by += issue(blocker_num). IDEMPOTENT: GitHub answers a repeat POST
    with 422 'Target issue has already been taken', which is reported here as a no-op success."""
    bid = _issue_id(blocker_num, issues)
    r = run(["gh", "api", "--method", "POST", f"repos/{REPO}/issues/{blocked_num}/dependencies/blocked_by",
             "-F", f"issue_id={bid}"], check=False)
    if r.returncode == 0:
        return True, "added"
    err = ((r.stderr or "") + (r.stdout or ""))[:300]
    if re.search(r"already exist|has already been taken|duplicate", err, re.I):
        return True, "already"
    return False, err


def remove_blocked_by(blocked_num, blocker_num, issues):
    bid = _issue_id(blocker_num, issues)
    r = run(["gh", "api", "--method", "DELETE",
             f"repos/{REPO}/issues/{blocked_num}/dependencies/blocked_by/{bid}"], check=False)
    if r.returncode == 0:
        return True, "removed"
    err = ((r.stderr or "") + (r.stdout or ""))[:300]
    if re.search(r"not found|404", err, re.I):
        return True, "absent"          # idempotent: already gone
    return False, err


# --- cycles -------------------------------------------------------------------------------------
def find_cycles(edges):
    """edges = {(blocked, blocker)}. Returns list of cycles (node lists) via iterative Tarjan SCC."""
    adj = defaultdict(list)
    nodes = set()
    for a, b in edges:
        adj[a].append(b)
        nodes.add(a)
        nodes.add(b)
    index, low, onstack, stack, idx = {}, {}, set(), [], [0]
    out = []
    for root in sorted(nodes, key=str):
        if root in index:
            continue
        work = [(root, iter(adj[root]))]
        index[root] = low[root] = idx[0]
        idx[0] += 1
        stack.append(root)
        onstack.add(root)
        while work:
            v, it = work[-1]
            advanced = False
            for w in it:
                if w not in index:
                    index[w] = low[w] = idx[0]
                    idx[0] += 1
                    stack.append(w)
                    onstack.add(w)
                    work.append((w, iter(adj[w])))
                    advanced = True
                    break
                if w in onstack:
                    low[v] = min(low[v], index[w])
            if advanced:
                continue
            work.pop()
            if work:
                low[work[-1][0]] = min(low[work[-1][0]], low[v])
            if low[v] == index[v]:
                comp = []
                while True:
                    w = stack.pop()
                    onstack.discard(w)
                    comp.append(w)
                    if w == v:
                        break
                if len(comp) > 1 or v in adj[v]:
                    out.append(sorted(comp, key=str))
    return out


# --- removal plan / write budget -----------------------------------------------------------------
def removal_plan(have, have_mapped, want):
    """Split native edges that have no bd counterpart into (removable, unowned).

    ONLY an edge whose BOTH endpoints are authenticated bd-migration mappings (i.e. it lives in
    `have_mapped`) is owned by this tool and therefore removable. A native edge with an unmapped
    endpoint carries no bd identity: its absence from `want` is not evidence that the migration
    created it, so it may be a legitimate dependency authored natively in GitHub. Those are reported
    as `unowned` and NEVER deleted."""
    return sorted(have_mapped - want), sorted(have - have_mapped)


def nonneg_int(s):
    """argparse type for --limit: a write cap must be an integer >= 0.

    Guards the direction the slice fails in: `[: -1]` schedules every write BUT THE LAST, so a
    negative limit would turn a safety canary into a near-full repository-scale mutation."""
    try:
        v = int(s)
    except ValueError:
        raise argparse.ArgumentTypeError(f"expected an integer, got {s!r}")
    if v < 0:
        raise argparse.ArgumentTypeError(f"must be >= 0 (got {v}); omit --limit for no cap")
    return v


def budget_split(to_add, removable, limit):
    """Apply ONE shared --limit budget across additions THEN removals, returning (adds, removes).

    `--limit` caps the writes performed in a run; a small limit is used as a safety canary, so it
    must bound the DELETE requests too, not just the POST requests.

    `None` is the SOLE unlimited sentinel: `--limit 0` means zero writes (a legitimate dry-run of
    the apply path), not "no cap". A negative limit is rejected at the CLI boundary by
    `nonneg_int` and clamped to zero here as well, so no caller can reach the `[: -1]` slice that
    would schedule all-but-the-last write while claiming to cap them."""
    if limit is None:
        return list(to_add), list(removable)
    limit = max(0, limit)
    adds = list(to_add)[:limit]
    return adds, list(removable)[: max(0, limit - len(adds))]


# --- direction proof ---------------------------------------------------------------------------
def prove_direction(beads, edges, idmap, native):
    """Pick a live (blocked, blocker) pair that is mapped BOTH sides and re-derive the orientation
    from `bd show` output, then report the GitHub native state for the same pair."""
    for blocked, blocker in sorted(edges):
        if blocked in idmap and blocker in idmap and bd_is_open(beads.get(blocked)):
            bshow = run(["bd", "show", blocked], check=False).stdout
            oshow = run(["bd", "show", blocker], check=False).stdout
            sec_b = re.search(r"DEPENDS ON\n((?:\s+→.*\n)+)", bshow)
            sec_o = re.search(r"BLOCKS\n((?:\s+←.*\n)+)", oshow)
            fwd = bool(sec_b and blocker in sec_b.group(1))
            rev = bool(sec_o and blocked in sec_o.group(1))
            if fwd and rev:
                nb, nk = idmap[blocked], idmap[blocker]
                return {
                    "blocked_bd": blocked, "blocker_bd": blocker,
                    "blocked_issue": nb, "blocker_issue": nk,
                    "bd_show_blocked_lists_blocker_under_DEPENDS_ON": fwd,
                    "bd_show_blocker_lists_blocked_under_BLOCKS": rev,
                    "github_blocked_by_present": nk in native.get(nb, set()),
                    "statement": f"bd {blocked} is blocked by {blocker}; "
                                 f"GitHub issue #{nb} must be blocked_by #{nk}",
                }
    return None


# --- main ---------------------------------------------------------------------------------------
def _self_test():
    ok = True

    def chk(n, got, want):
        nonlocal ok
        good = got == want
        ok = ok and good
        print(f"  {'ok  ' if good else 'FAIL'} {n}: {got!r} (want {want!r})")

    # --- orientation: the export record {issue_id: X, depends_on_id: Y} means X is BLOCKED BY Y.
    # A reversed reader would emit (Y, X) here — this is the guard against inverting the graph.
    beads = {"X": {"id": "X", "status": "open",
                   "dependencies": [{"issue_id": "X", "depends_on_id": "Y", "type": "blocks"}]},
             "Y": {"id": "Y", "status": "open"}}
    chk("blocks edge orients (blocked, blocker)", bd_blocker_edges(beads), {("X", "Y")})
    # `blocked-by` rows have the SAME orientation (verified via `bd show sq-8ic6`) and must NOT be
    # dropped — bd-to-issues.py::plan only matched "blocks" and silently lost them.
    beads["X"]["dependencies"][0]["type"] = "blocked-by"
    chk("blocked-by edge is kept, same orientation", bd_blocker_edges(beads), {("X", "Y")})
    # parent-child is NOT a blocker (mirroring it re-creates sub-epic-leaf-invisibility)
    beads["X"]["dependencies"][0]["type"] = "parent-child"
    chk("parent-child is not a blocker edge", bd_blocker_edges(beads), set())
    beads["X"]["dependencies"][0]["type"] = "related"
    chk("related is not a blocker edge", bd_blocker_edges(beads), set())

    # --- cycles
    chk("no cycle in a chain", find_cycles({("a", "b"), ("b", "c")}), [])
    chk("2-cycle found", [sorted(c) for c in find_cycles({("a", "b"), ("b", "a")})], [["a", "b"]])
    chk("3-cycle found", [sorted(c) for c in find_cycles({("a", "b"), ("b", "c"), ("c", "a")})],
        [["a", "b", "c"]])
    chk("self-loop found", [sorted(c) for c in find_cycles({("a", "a")})], [["a"]])
    chk("cycle found with a dangling tail",
        [sorted(c) for c in find_cycles({("a", "b"), ("b", "a"), ("c", "a")})], [["a", "b"]])

    # --- provenance: a marker alone must NOT capture a bd-id
    base = {"body": "<!-- bd-id:sq-abc -->", "title": "sq-abc: t", "labels": set(), "id": 1,
            "state": "open", "author": "outsider", "assoc": "NONE"}
    chk("outsider marker rejected", bd_id_map({5: dict(base)})[0], {})
    chk("member marker accepted", bd_id_map({5: dict(base, author="jeswr", assoc="MEMBER")})[0],
        {"sq-abc": 5})
    chk("member marker with a DISAGREEING title rejected",
        bd_id_map({5: dict(base, author="jeswr", assoc="MEMBER", title="sq-zzz: t")})[0], {})
    chk("non-grammar marker rejected (doc issues quoting the marker format)",
        bd_id_map({5: dict(base, author="jeswr", assoc="MEMBER", body="<!-- bd-id:… -->",
                           title="… : t")})[0], {})
    chk("lowest issue number wins a duplicate bd-id",
        bd_id_map({9: dict(base, author="j", assoc="MEMBER"),
                   5: dict(base, author="j", assoc="MEMBER")})[0], {"sq-abc": 5})

    # --- removal plan: DELETE is restricted to the bd-managed graph.
    # (1,2) is bd-managed with no bd counterpart -> removable. (3,4) has an unmapped endpoint, so it
    # may be a native GitHub dependency this tool does not own -> reported unowned, NEVER removed.
    have = {(1, 2), (3, 4), (5, 6)}
    have_mapped = {(1, 2), (5, 6)}
    want = {(5, 6)}
    chk("unmapped native edge is excluded from the removal plan",
        removal_plan(have, have_mapped, want)[0], [(1, 2)])
    chk("unmapped native edge is reported as unowned",
        removal_plan(have, have_mapped, want)[1], [(3, 4)])
    chk("an edge bd still wants is never removable",
        removal_plan({(5, 6)}, {(5, 6)}, {(5, 6)}), ([], []))

    # --- --limit is ONE budget over additions AND removals, so total mutations never exceed it.
    adds, removes = budget_split([("a", 1), ("a", 2), ("a", 3)], [("r", 1), ("r", 2)], 4)
    chk("limit spills over into removals once additions are covered", (adds, removes),
        ([("a", 1), ("a", 2), ("a", 3)], [("r", 1)]))
    chk("total mutations under a mixed plan never exceed --limit", len(adds) + len(removes), 4)
    a2, r2 = budget_split([("a", 1), ("a", 2), ("a", 3)], [("r", 1), ("r", 2)], 2)
    chk("a limit consumed by additions leaves no removal budget", (a2, r2),
        ([("a", 1), ("a", 2)], []))
    chk("no limit means the whole plan runs",
        budget_split([("a", 1)], [("r", 1)], None), ([("a", 1)], [("r", 1)]))
    # None is the SOLE unlimited sentinel; 0 must mean ZERO writes, not "falsy => no cap".
    chk("--limit 0 performs no writes at all",
        budget_split([("a", 1), ("a", 2)], [("r", 1)], 0), ([], []))
    # A negative limit must never reach the `[: -1]` slice, which would schedule every write but
    # the last -- the exact inversion of the cap the flag promises.
    chk("a negative limit is clamped to zero, not an all-but-the-last slice",
        budget_split([("a", 1), ("a", 2), ("a", 3)], [("r", 1), ("r", 2)], -1), ([], []))

    # --- the CLI boundary itself rejects a negative cap (argparse accepts any int without this).
    def _limit_rejects(s):
        try:
            nonneg_int(s)
            return False
        except argparse.ArgumentTypeError:
            return True

    chk("--limit -1 is rejected during argument parsing", _limit_rejects("-1"), True)
    chk("--limit nonsense is rejected during argument parsing", _limit_rejects("all"), True)
    chk("--limit 0 is accepted during argument parsing", nonneg_int("0"), 0)
    chk("--limit 7 is accepted during argument parsing", nonneg_int("7"), 7)
    print("audit-issue-deps self-test", "PASSED" if ok else "FAILED")
    return 0 if ok else 1


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--self-test", action="store_true")
    ap.add_argument("--apply", action="store_true", help="write the repair (default: audit only)")
    ap.add_argument("--export-file", help="read `bd export` output from a file")
    ap.add_argument("--prove", action="store_true", help="print the direction-proof witness and exit")
    ap.add_argument("--remove-spurious", action="store_true",
                    help="with --apply, also DELETE native edges with no bd counterpart whose BOTH "
                         "endpoints are authenticated bd mappings; native edges with an unmapped "
                         "endpoint are reported as unowned and never removed")
    ap.add_argument("--include-closed-blockers", action="store_true",
                    help="also mirror edges whose blocker issue is already CLOSED (inert for "
                         "readiness — GitHub's issueDependenciesSummary.blockedBy excludes them)")
    ap.add_argument("--json-out", help="write the full audit record here")
    ap.add_argument("--limit", type=nonneg_int, metavar="N",
                    help="cap the TOTAL writes this run (N >= 0), shared across additions and "
                         "removals; 0 performs no writes, omit for no cap")
    args = ap.parse_args()
    if args.self_test:
        return _self_test()

    beads = bd_beads(args.export_file)
    edges_bd = bd_blocker_edges(beads)
    issues = gh_issues()
    idmap, unverified, collisions = bd_id_map(issues)
    rev = {n: b for b, n in idmap.items()}
    native = gh_native_blocked_by()

    if args.prove:
        print(json.dumps(prove_direction(beads, edges_bd, idmap, native), indent=2))
        return 0

    # ---- 1. bd edges where BOTH endpoints have issues -------------------------------------------
    mappable = {(a, b) for (a, b) in edges_bd if a in idmap and b in idmap}
    want = {(idmap[a], idmap[b]) for (a, b) in mappable}       # (blocked_issue, blocker_issue)
    have = {(n, k) for n, ks in native.items() for k in ks}
    have_mapped = {(n, k) for (n, k) in have if n in rev and k in rev}

    absent = sorted(want - have)
    # removable = bd-managed (BOTH ends authenticated) native edge with no bd counterpart.
    # unowned   = native edge with an unmapped end — outside this tool's graph, reported not removed.
    spurious, unowned = removal_plan(have, have_mapped, want)
    present = sorted(want & have)

    # An absent edge whose BLOCKER ISSUE is already CLOSED is SATISFIED, not missing: MEASURED,
    # GitHub reports `issueDependenciesSummary.blockedBy = 0` for a closed blocker (while
    # `totalBlockedBy` counts it), and scripts/ready-issues.py counts a `Blocked-by: #NN` marker
    # only when the number is in `open_numbers`. Neither consumer would hold the child. They are
    # reported in their own bucket so a re-run does not carry a permanent phantom "missing" count
    # that would mask a real future gap. `--include-closed-blockers` mirrors them anyway.
    def blocker_closed(e):
        return issues.get(e[1], {}).get("state") != "open"

    satisfied = [e for e in absent if blocker_closed(e)]
    missing = [e for e in absent if not blocker_closed(e)]

    # ---- 2. stale blockers: an edge whose BLOCKER issue is already CLOSED -----------------------
    stale_native = sorted((n, k) for (n, k) in have
                          if issues.get(k, {}).get("state") == "closed"
                          and issues.get(n, {}).get("state") == "open")
    # bd-side equivalent: open bead blocked by a closed bead. NOT a defect on its own — `bd ready`
    # is blocker-aware and treats a closed blocker as satisfied (verified: sq-0xquh, whose only
    # blocker sq-0wo9e is closed, IS listed by `bd ready --limit 2000`). Reported for visibility.
    stale_bd = sorted((a, b) for (a, b) in edges_bd
                      if bd_is_open(beads.get(a)) and not bd_is_open(beads.get(b)))

    # ---- 3. writes we will actually perform -----------------------------------------------------
    to_add = missing + (satisfied if args.include_closed_blockers else [])

    # ---- 4. cycles -------------------------------------------------------------------------------
    cyc_bd = find_cycles({(a, b) for (a, b) in edges_bd
                          if bd_is_open(beads.get(a)) and bd_is_open(beads.get(b))})
    cyc_gh_after = find_cycles(want | have)

    rep = {
        "beads_total": len(beads),
        "beads_open": sum(1 for b in beads.values() if bd_is_open(b)),
        "gh_issues_nonpr": len(issues),
        "bd_ids_mapped_to_issues": len(idmap),
        "unauthenticated_markers": {k: v for k, v in list(unverified.items())[:20]},
        "unauthenticated_marker_count": len(unverified),
        "bd_id_collisions": collisions,
        "bd_blocker_edges_total": len(edges_bd),
        "bd_blocker_edges_both_endpoints_mapped": len(mappable),
        "github_native_edges_total": len(have),
        "github_native_edges_both_ends_bd_mapped": len(have_mapped),
        "present": len(present),
        "missing_active": len(missing),
        "satisfied_blocker_closed_not_mirrored": len(satisfied),
        "satisfied_distinct_children": len({n for n, _ in satisfied}),
        "will_write": len(to_add),
        "spurious_bd_managed_removable": len(spurious),
        "unowned_native_edges_never_removed": len(unowned),
        "stale_native_open_issue_blocked_by_closed_issue": len(stale_native),
        "stale_bd_open_bead_blocked_by_closed_bead": len(stale_bd),
        "cycles_bd_open": cyc_bd,
        "cycles_github_projected": cyc_gh_after,
        "samples": {
            "missing_active": [{"blocked": f"#{n} ({rev.get(n)})", "blocker": f"#{k} ({rev.get(k)})"}
                               for n, k in missing[:15]],
            "satisfied_blocker_closed": [
                {"blocked": f"#{n} ({rev.get(n)})", "closed_blocker": f"#{k} ({rev.get(k)})"}
                for n, k in satisfied[:20]],
            "spurious_bd_managed_removable": [
                {"blocked": f"#{n} ({rev.get(n, '-')})", "blocker": f"#{k} ({rev.get(k, '-')})"}
                for n, k in spurious[:15]],
            "unowned_native_edges_never_removed": [
                {"blocked": f"#{n} ({rev.get(n, '-')})", "blocker": f"#{k} ({rev.get(k, '-')})"}
                for n, k in unowned[:15]],
            "stale_native": [{"blocked": f"#{n}", "closed_blocker": f"#{k}"} for n, k in stale_native[:15]],
            "stale_bd": [{"open_bead": a, "closed_blocker_bead": b} for a, b in stale_bd[:15]],
        },
    }

    print(json.dumps({k: v for k, v in rep.items() if k != "samples"}, indent=2))
    print("\nSAMPLES\n" + json.dumps(rep["samples"], indent=2))

    proof = prove_direction(beads, edges_bd, idmap, native)
    print("\nDIRECTION PROOF\n" + json.dumps(proof, indent=2))
    rep["direction_proof"] = proof

    if args.apply:
        if not proof or not (proof["bd_show_blocked_lists_blocker_under_DEPENDS_ON"]
                             and proof["bd_show_blocker_lists_blocked_under_BLOCKS"]):
            raise SystemExit("refusing --apply: direction proof did not verify against the live tree")
        added = noop = failed = 0
        # ONE budget across additions AND removals, so a small --limit is a real safety canary.
        work, rwork = budget_split(to_add, spurious if args.remove_spurious else [], args.limit)
        for i, (n, k) in enumerate(work, 1):
            ok, why = add_blocked_by(n, k, issues)
            if not ok:
                failed += 1
                print(f"  FAILED #{n} blocked_by #{k}: {why}")
            elif why == "already":
                noop += 1
            else:
                added += 1
            if i % 25 == 0:
                print(f"  ... {i}/{len(work)} (added {added}, already-present {noop}, failed {failed})")
        print(f"[apply] added {added} native blocked_by edge(s); {noop} already present (no-op); "
              f"{failed} failure(s)")
        rep["applied_added"] = added
        rep["applied_noop"] = noop
        rep["applied_failed"] = failed
        if args.remove_spurious:
            removed = rnoop = rfail = 0
            for n, k in rwork:
                ok, why = remove_blocked_by(n, k, issues)
                if not ok:
                    rfail += 1
                    print(f"  FAILED remove #{n} blocked_by #{k}: {why}")
                elif why == "absent":
                    rnoop += 1
                else:
                    removed += 1
            print(f"[apply] removed {removed} of {len(spurious)} bd-managed spurious edge(s); "
                  f"{rnoop} already absent; {rfail} failure(s); "
                  f"{len(unowned)} unowned native edge(s) left untouched")
            rep["applied_removed"] = removed
            rep["applied_remove_noop"] = rnoop
            rep["applied_remove_deferred_by_limit"] = len(spurious) - len(rwork)
    else:
        print(f"\n[audit-only] would add {len(to_add)} native edge(s); "
              f"{len(satisfied)} not mirrored (blocker already closed — inert); "
              f"{len(spurious)} bd-managed spurious edge(s) removable; "
              f"{len(unowned)} unowned native edge(s) reported only (never removed). "
              f"Re-run with --apply.")

    if args.json_out:
        json.dump(rep, open(args.json_out, "w"), indent=2, default=list)
    return 0


if __name__ == "__main__":
    sys.exit(main())
