#!/usr/bin/env python3
# [OPUS-5] Authorised re-triage of the `needs:user` front door (maintainer OK: sparq-org/sparq#1135).
"""retriage-needs-user.py — shrink `needs:user` back to "only the maintainer can do this".

`needs:user` is a TERMINAL human hold: `ready-issues.py` lists it in `PARKED_AREA_LABELS`, so a
labelled issue is undispatchable AND reserves nothing — it is invisible to the fleet until a human
removes the label. The bd->issues migration applied it far too widely (`bd-to-issues.py`
`externally_gated()` deliberately "errs toward over-gating"), so the front door accumulated 116
open issues, most of them ordinary engineering: a `bind_join` run-grouping perf task (#3190), an
Elias-Fano column codec (#3211), SHACL pre-binding semantics (#3408). A front door with 116 items
behind it is not a front door.

This script keeps a CURATED, hand-justified KEEP set (below) and DEMOTES every other open
`needs:user` issue into a dispatchable state:

  * remove `needs:user`
  * remove the bd-migration park statuses (`status:parked`, `status:deferred`, `status:untriaged`)
  * add `status:ready` — but ONLY when nothing else legitimately gates the issue
    (`status:blocked`, or another `needs:*` gate such as `needs:ec2` / `needs:external-audit`)
  * post an audit comment recording the demotion, so the change is reversible

What it deliberately does NOT do:
  * touch `area:*` — a concurrent pass is classifying `needs:area` issues; two writers on one
    label family is how you get a double-labelled, undispatchable issue.
  * touch `priority:*`, or guess a `role:` — `ready-issues.py` never guesses a role either
    (`roleless_ready` exists precisely because guessing is worse than parking).
  * touch pull requests, or any issue in the KEEP set.

Idempotent: an issue that no longer carries `needs:user` is skipped entirely, so re-running is a
no-op and a re-park by a human survives.

Usage:
    scripts/retriage-needs-user.py --dry-run          # classification table, no writes
    scripts/retriage-needs-user.py --apply
    scripts/retriage-needs-user.py --selftest
"""
import argparse
import json
import subprocess
import sys
import time

REPO = "sparq-org/sparq"
LABEL = "needs:user"

# The bd-migration park statuses. `status:parked` has NO consumer in the pipeline (it is a pure
# migration artifact of the bd `parked` status); `status:deferred` and `status:untriaged` are in
# ready-issues.py's BUSY_STATUS and so keep an issue undispatchable on their own.
PARK_STATUS = ("status:parked", "status:deferred", "status:untriaged")
# Gates that are NOT ours to clear — an issue carrying one stays parked after the demotion, and we
# must not stamp `status:ready` on it (that would advertise a dispatchable issue no worker can run).
FOREIGN_GATES = ("status:blocked", "needs:")

# --- the KEEP set -------------------------------------------------------------------------------
# `needs:user` survives ONLY where a human is the ONLY POSSIBLE ACTOR. Every entry carries the
# one-line reason that must remain true; if the reason stops being true, remove the entry AND the
# label. Fail-closed: a borderline item is KEPT (a wrongly-kept issue costs throughput; a wrongly
# demoted one silently removes the maintainer's visibility on something only they can decide).
KEEP = {
    # --- external humans -------------------------------------------------------------------
    2553: "external accredited-cryptographer ZK audit (sq-qhy4); its own text says agent-out-of-scope by definition",
    3274: "ZK assurance promotion is hard-gated on the sq-qhy4 external sign-off (#2553)",
    # --- credentials / accounts / tokens / licences ------------------------------------------
    2964: "Apple Developer ID + Windows Authenticode code-signing certs are maintainer-held (and a spend)",
    2965: "PyPI Trusted Publisher registration, NPM_TOKEN, crates.io login — maintainer-held account steps",
    2995: "Settings->Pages->Source is a repo-admin toggle only the owner can flip",
    3262: "dismissing a Dependabot alert is a Security-tab action on the owner's repo",
    3337: "crates.io API token / cargo owner — maintainer-held credential",
    3382: "RDFox is commercial; gathering numbers needs a licence the maintainer must buy/supply",
    2961: "iOS release target needs an Apple Developer signing cert + a paid macOS runner",
    2652: "asks for maintainer review of the ~/.codex OAuth-copy-to-EC2 mechanism and the on-demand cost model (spend)",
    # --- public identity / brand / naming ----------------------------------------------------
    2957: "logo: the maintainer rejected all four prior concepts; the final mark pick is his (#207/#823)",
    3399: "final npm package name (the @jeswr/sparq placeholder) is a public-identity decision",
    2630: "publishing @jeswr/sparq needs the npm credential AND settles the public package name",
    3123: "making w3id.org/zkp-sparql/* dereference requires control of a namespace the maintainer owns",
    # --- upstream filings under his identity / PRs on other orgs needing his review -----------
    2785: "the four noir-lang drafts are gated on @jeswr's own author review (only he flips draft->ready)",
    3329: "filing the georust/geo request is his decision to engage upstream, under his account",
    3338: "the four w3c/rdf-tests filings are explicitly awaiting his go-ahead",
    3385: "offering the GML-SF parser to georust is his decision to engage upstream",
    # --- genuine policy / steer decisions -----------------------------------------------------
    3299: "GitHub Pages has ONE deploy slot; picking guide-vs-showcase at root is a product call that bricks the live site if wrong",
    2936: "sparq-algos is hard-gated on his sign-off of the invented `algo:` namespace + the ontology",
    1030: "a pure steer ask ('do you want the standalone explain() back') — there is no agent work in it",
}


def _run(args):
    return subprocess.run(args, capture_output=True, text=True, check=True)


def fetch_needs_user(repo=REPO):
    """Every OPEN issue carrying `needs:user`, via the LIST API with real pagination.

    NEVER `gh search`: GitHub's search index lags by minutes-to-hours, and a stale population here
    means demoting an issue a human re-parked, or missing one they just parked.
    """
    out = _run(["gh", "api", "--paginate",
                f"/repos/{repo}/issues?state=open&labels={LABEL}&per_page=100",
                "--jq", '.[] | select(.pull_request == null) | '
                        '{n: .number, t: .title, l: [.labels[].name]}']).stdout
    return [json.loads(line) for line in out.splitlines() if line.strip()]


def classify(issue, keep=KEEP):
    """-> (action, labels_to_remove, labels_to_add, reason).

    action is "keep" | "demote" | "demote-still-gated" | "skip".
    """
    labels = set(issue["l"])
    if LABEL not in labels:
        return "skip", [], [], "no longer carries needs:user"
    if issue["n"] in keep:
        return "keep", [], [], keep[issue["n"]]
    remove = [LABEL] + [s for s in PARK_STATUS if s in labels]
    # anything OTHER than needs:user that still gates the issue
    residual = sorted(lb for lb in labels
                      if lb != LABEL and any(lb == g or lb.startswith(g) for g in FOREIGN_GATES))
    if residual:
        # Do not claim readiness we cannot deliver: leave the park status alone too, so the issue
        # keeps an honest not-ready state rather than looking ready-but-gated.
        return ("demote-still-gated", [LABEL], [],
                "demoted, but still gated by " + ", ".join(residual))
    add = [] if "status:ready" in labels else ["status:ready"]
    return "demote", remove, add, "ordinary engineering work — no maintainer-only step"


COMMENT = (
    "> \U0001f916 **SPARQ agent** (Opus 5, 1M context) — @jeswr runs several agents on this "
    "account.\n\n"
    "**Re-triaged off `needs:user`** by the authorised re-triage pass (maintainer authorisation: "
    "#1135).\n\n"
    "`needs:user` is a *terminal* human hold — `scripts/ready-issues.py` treats it as a park, "
    "so a labelled issue is undispatchable and invisible to the fleet until a human removes it. "
    "The bd→issues migration over-applied it (`bd-to-issues.py::externally_gated` deliberately "
    "errs toward over-gating), leaving 116 open issues behind the front door — mostly ordinary "
    "engineering.\n\n"
    "This issue was judged **{reason}**, so the hold is removed{ready}.\n\n"
    "If this is wrong — if there really is a maintainer-only step here (a credential, a licence, "
    "a naming/brand call, an upstream filing under your identity, or a policy decision the repo has "
    "no basis to make) — re-adding `needs:user` restores the hold immediately and this pass will "
    "not touch it again.\n\n"
    "<!-- retriage:needs-user:v1 -->"
)


def apply_one(issue, action, remove, add, reason, repo=REPO, dry_run=True, log=print):
    n = issue["n"]
    if action in ("keep", "skip"):
        return False
    ready = " and it is now `status:ready`" if "status:ready" in add else ""
    body = COMMENT.format(reason=reason, ready=ready)
    if dry_run:
        log(f"  DRY #{n}: -{remove} +{add}")
        return False
    args = ["gh", "issue", "edit", str(n), "--repo", repo]
    for lb in remove:
        args += ["--remove-label", lb]
    for lb in add:
        args += ["--add-label", lb]
    _run(args)
    _run(["gh", "issue", "comment", str(n), "--repo", repo, "--body", body])
    return True


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--repo", default=REPO)
    ap.add_argument("--dry-run", action="store_true", default=True)
    ap.add_argument("--apply", dest="dry_run", action="store_false")
    ap.add_argument("--only", type=int, nargs="*", help="restrict to these issue numbers")
    # Two content-creating REST calls per demoted issue (edit + comment). ~100 issues is ~200
    # mutations, well past GitHub's secondary rate limit for a burst; a 403 mid-run would leave
    # the population half-demoted, which is the worst of both states.
    ap.add_argument("--pause", type=float, default=1.5,
                    help="seconds between MUTATED issues (secondary-rate-limit pacing)")
    args = ap.parse_args(argv)

    issues = fetch_needs_user(args.repo)
    if args.only:
        issues = [i for i in issues if i["n"] in set(args.only)]
    counts, changed = {}, 0
    for issue in sorted(issues, key=lambda i: i["n"]):
        action, remove, add, reason = classify(issue)
        counts[action] = counts.get(action, 0) + 1
        print(f"{action:20s} #{issue['n']:<5d} {issue['t'][:78]}\n{'':20s}   -> {reason}")
        if apply_one(issue, action, remove, add, reason,
                     repo=args.repo, dry_run=args.dry_run):
            changed += 1
            time.sleep(args.pause)
    print(f"\npopulation {len(issues)}; " +
          "; ".join(f"{k}={v}" for k, v in sorted(counts.items())) +
          f"; applied={changed}{' (DRY RUN)' if args.dry_run else ''}")
    unknown = sorted(set(KEEP) - {i["n"] for i in issues})
    if unknown and not args.only:
        print(f"NOTE: KEEP entries no longer carrying {LABEL} (prune them): {unknown}")
    return 0


# --- self-test ----------------------------------------------------------------------------------
def _selftest():
    fails = []

    def chk(name, got, want):
        if got != want:
            fails.append(f"{name}: got {got!r} want {want!r}")

    keep = {7: "because"}
    chk("a KEEP issue is kept",
        classify({"n": 7, "t": "", "l": ["needs:user", "status:deferred"]}, keep)[0], "keep")
    chk("an issue without the label is skipped",
        classify({"n": 9, "t": "", "l": ["status:ready"]}, keep)[0], "skip")
    a, rm, add, _ = classify({"n": 8, "t": "", "l": ["needs:user", "status:deferred",
                                                     "status:parked", "role:impl"]}, keep)
    chk("a plain issue is demoted", a, "demote")
    chk("the park statuses go with the hold", sorted(rm),
        ["needs:user", "status:deferred", "status:parked"])
    chk("and it lands ready", add, ["status:ready"])
    # THE point of the gate check: a second gate must survive the demotion and must NOT get
    # status:ready, or ready-issues.py would advertise an issue no worker can run.
    a, rm, add, _ = classify({"n": 10, "t": "", "l": ["needs:user", "needs:ec2",
                                                      "status:deferred"]}, keep)
    chk("a residually-gated issue is not made ready", (a, rm, add),
        ("demote-still-gated", ["needs:user"], []))
    a, rm, add, _ = classify({"n": 11, "t": "", "l": ["needs:user", "status:blocked"]}, keep)
    chk("status:blocked also survives", (a, add), ("demote-still-gated", []))
    chk("already-ready issues gain no duplicate",
        classify({"n": 12, "t": "", "l": ["needs:user", "status:ready"]}, keep)[2], [])
    # apply_one must never write for keep/skip, and must never write in dry-run.
    chk("keep never writes", apply_one({"n": 7}, "keep", ["needs:user"], [], "r",
                                       dry_run=False, log=lambda *_: None), False)
    chk("dry-run never writes", apply_one({"n": 8}, "demote", ["needs:user"], [], "r",
                                          dry_run=True, log=lambda *_: None), False)
    for f in fails:
        print("FAIL", f)
    print(f"{'FAILED' if fails else 'ok'}: retriage-needs-user selftest ({len(fails)} failure(s))")
    return 1 if fails else 0


if __name__ == "__main__":
    if "--selftest" in sys.argv:
        raise SystemExit(_selftest())
    raise SystemExit(main())
