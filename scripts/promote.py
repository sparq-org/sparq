#!/usr/bin/env python3
# [OPUS-4.8] Trust promotion: decide if a quarantined (trust:untrusted) item may be promoted.
"""promote.py — revision-bound 👍 promotion of third-party content.

GitHub Actions has NO reaction event, so a cron polls quarantined issues and calls this. An item is
promoted iff a MAINTAINER (write/maintain/admin) added a 👍 reaction **after** the item's last edit —
so editing the body after approval invalidates it (the review's S3 TOCTOU fix). Reactions before the
last edit, or from non-maintainers, do not promote.

Pure `should_promote()` is unit-tested; the cron supplies reactions + updated_at + the maintainer set.
`fetch_quarantined()` + `epoch()` are the two inputs the cron used to build INLINE, and are hosted
here so they are testable at PR time rather than only observable on a live cron fire.
"""
import datetime
import json
import subprocess
import sys
import urllib.parse

QUARANTINE_LABEL = "trust:untrusted"
FETCH_CEILING = 10000


def should_promote(reactions, updated_at, maintainers):
    """`reactions`: list of {content, user, created_at (int)}. `updated_at` (int): the item's last
    edit time. `maintainers`: set of trusted logins. Promote iff some 👍 from a maintainer was created
    at/after the last edit."""
    mset = {m.lower() for m in maintainers}
    for r in reactions:
        if (r.get("content") in ("+1", "\U0001F44D")  # '+1' is the API name for 👍
                and str(r.get("user", "")).lower() in mset
                and int(r.get("created_at", 0)) >= int(updated_at)):
            return True
    return False


def epoch(ts):
    """An ISO-8601 GitHub timestamp as integer seconds. Raises on anything unparseable — the
    caller compares it against a reaction time, so a silent 0 would read as "edited at the epoch"
    and make EVERY 👍 look post-edit. Fail loud, never fail open."""
    return int(datetime.datetime.fromisoformat(str(ts).replace("Z", "+00:00")).timestamp())


def _gh(args):
    return subprocess.run(["gh", *args], capture_output=True, text=True)


def fetch_quarantined(repo, run=_gh, ceiling=FETCH_CEILING):
    """Every OPEN issue carrying `trust:untrusted`, via REAL cursor pagination.

    [OPUS-5] (#5425, the #4985 remainder) This was `gh issue list --limit 200` inline in
    .github/workflows/promote-on-approval.yml. `--limit` SILENTLY TRUNCATES: the CLI stops at
    the limit, keeps the NEWEST rows, and reports nothing — no warning, no non-zero exit. The
    quarantine queue is a work queue polled every 10 minutes, so a truncated fetch is not a slow
    drain: once the labelled open set passes the limit the OLDEST quarantined issues drop off
    every tick, are never polled for maintainer approval, and stay quarantined forever with
    nothing in the run log saying so. A maintainer's 👍 on such an issue simply never takes
    effect, which reads to them as the automation being broken rather than as a truncation.

    `gh api --paginate` follows the Link headers to exhaustion instead; the explicit ceiling
    still fails CLOSED on a runaway snapshot rather than half-reporting (same shape as
    `retriage.py::_fetch_label`, `triage-area.py::open_issues` and `ready-issues.py::_fetch`).

    The defect is DEPTH-CONDITIONAL: below the limit the truncated and paginated fetches return
    identical rows, so this is a LATENT fault that arms itself the moment the queue crosses 200.
    No point-in-time count is quoted here because it would be read as a standing gain.

    The issues endpoint returns PRs as well as issues, so they are dropped here. No current path
    puts `trust:untrusted` on a PR (triage-issue.yml fires on `issues:` only), which makes this a
    defence against a hand-applied or future-automated label rather than an observed case: the
    promotion path below comments and re-labels as if the row were an issue, and the `gh issue
    list` call it replaces could never return a PR at all.

    Rows are normalised to the `{number, updatedAt, labels:[{name}]}` shape the cron loop reads,
    because the REST payload spells the timestamp `updated_at` while the old `gh issue list
    --json` payload spelled it `updatedAt`. Getting that wrong is the fail-OPEN direction — the
    cron would see a missing timestamp, and any 👍 at all would then out-rank the last edit and
    promote unreviewed third-party content, which is the exact S3 TOCTOU hole `should_promote`
    exists to close. `_self_test` drives fetch -> epoch -> should_promote end-to-end over a
    fixture carrying ONLY the keys the REST payload really has, so the mis-read cannot pass.
    """
    r = run(["api", "--paginate", "--slurp",
             f"repos/{repo}/issues?state=open"
             f"&labels={urllib.parse.quote(QUARANTINE_LABEL)}&per_page=100"])
    if r.returncode != 0:
        raise SystemExit(f"promote: could not list open '{QUARANTINE_LABEL}' issues")
    rows = [i for page in json.loads(r.stdout or "[]") if isinstance(page, list)
            for i in page if isinstance(i, dict) and "pull_request" not in i]
    if len(rows) >= ceiling:
        raise SystemExit(f"promote: fetched {len(rows)} quarantined issues >= ceiling {ceiling} — "
                         "snapshot looks runaway (fail-closed). Raise the ceiling deliberately.")
    out = []
    for i in rows:
        if not i.get("updated_at"):
            raise SystemExit(f"promote: issue #{i.get('number')} has no updated_at — refusing to "
                             "evaluate approvals against an unknown edit time (fail-closed).")
        out.append({"number": i["number"], "updatedAt": i["updated_at"],
                    "labels": i.get("labels") or []})
    return out


def _self_test():
    ok = True

    def chk(n, got, want):
        nonlocal ok
        good = got == want
        ok = ok and good
        print(f"  {'ok  ' if good else 'FAIL'} {n}: {got} (want {want})")

    M = {"jeswr"}
    chk("maintainer 👍 after edit", should_promote([{"content": "+1", "user": "jeswr", "created_at": 100}], 90, M), True)
    chk("👍 BEFORE edit (revoked)", should_promote([{"content": "+1", "user": "jeswr", "created_at": 80}], 90, M), False)
    chk("non-maintainer 👍", should_promote([{"content": "+1", "user": "rando", "created_at": 100}], 90, M), False)
    chk("wrong reaction", should_promote([{"content": "heart", "user": "jeswr", "created_at": 100}], 90, M), False)
    chk("no reactions", should_promote([], 90, M), False)
    chk("exactly at edit time", should_promote([{"content": "+1", "user": "jeswr", "created_at": 90}], 90, M), True)

    # --- the quarantine FETCH must not silently truncate the queue (#5425) --------------------
    # `gh issue list --limit 200` keeps the NEWEST 200 and reports nothing, so past 200 the
    # OLDEST quarantined issues are never polled on ANY tick. The stub below emulates BOTH CLI
    # shapes faithfully — `gh api --paginate` exhausts the Link chain, `gh issue list --limit N`
    # truncates newest-first — so reverting this fetch to the limited call is EXECUTABLE here and
    # fails on the MISSING ROWS rather than on an unrecognised command. That is what makes this a
    # behavioural guard and not a spelling test.
    TOTAL = 250  # > 200, so a --limit 200 fetch must lose the oldest 50 (#1..#50)
    QL = [{"name": QUARANTINE_LABEL}, {"name": "kind:bug"}]
    # Fixture rows carry ONLY the keys the REST payload really has: `updated_at`, never the
    # `updatedAt` that `gh issue list --json` emitted. Reading the old spelling therefore
    # fails CLOSED here instead of surviving the assertion.
    corpus = [{"number": n, "updated_at": f"2026-01-01T00:00:{n % 60:02d}Z", "labels": QL}
              for n in range(1, TOTAL + 1)]
    corpus.append({"number": 9001, "updated_at": "2026-01-01T00:00:00Z", "labels": QL,
                   "pull_request": {"url": "https://api.github.com/repos/o/r/pulls/9001"}})
    corpus.append({"number": 9002, "updated_at": "2026-01-01T00:00:00Z",
                   "labels": [{"name": "kind:bug"}]})  # not quarantined

    class _Resp:
        def __init__(self, stdout, returncode=0):
            self.stdout, self.returncode = stdout, returncode

    seen = []

    def _stub_gh(args, rows=corpus):
        """Faithful emulator of both gh shapes, so either implementation runs here.

        `gh api --paginate` follows the Link chain to exhaustion; without `--paginate` gh
        returns only the first page. `gh issue list --limit N` truncates newest-first."""
        seen.append(list(args))

        def labelled(label):
            return [r for r in rows if label in {lb["name"] for lb in r["labels"]}]

        if args[0] == "api":
            url = args[-1]
            sel = urllib.parse.unquote(url.split("labels=")[1].split("&")[0]) if "labels=" in url else None
            hits = labelled(sel) if sel else list(rows)
            pages = [hits[i:i + 100] for i in range(0, len(hits), 100)] or [[]]
            return _Resp(json.dumps(pages if "--paginate" in args else pages[:1]))
        if args[0:2] == ["issue", "list"]:
            hits = labelled(args[args.index("--label") + 1])
            limit = int(args[args.index("--limit") + 1]) if "--limit" in args else len(hits)
            # `gh issue list --json` spells the timestamp `updatedAt` and returns issues only.
            # Emitting its REAL shape is what makes a reverted fetch fail on the MISSING ROWS
            # rather than on an incidental key rename.
            return _Resp(json.dumps([{"number": r["number"], "updatedAt": r["updated_at"],
                                      "labels": r["labels"]}
                                     for r in sorted(hits, key=lambda r: -r["number"])
                                     if "pull_request" not in r][:limit]))
        raise AssertionError(f"unexpected gh invocation: {args}")

    fetched = fetch_quarantined("o/r", run=_stub_gh)
    nums = [it["number"] for it in fetched]
    chk("every quarantined issue is polled, not just the newest page", len(nums), TOTAL)
    chk("the OLDEST quarantined issues survive the fetch", nums[:3], [1, 2, 3])
    chk("the fetch uses cursor pagination, not a fixed page limit",
        all(c[0] == "api" and "--paginate" in c for c in seen), True)
    chk("pull requests are dropped (the issues endpoint returns both)", 9001 in nums, False)
    chk("unlabelled issues are not fetched", 9002 in nums, False)
    chk("rows carry the keys the cron reads", sorted(fetched[0]), ["labels", "number", "updatedAt"])

    # End-to-end: a fetched row must drive the REAL promotion decision, both directions. If the
    # REST timestamp were mis-read the edit time would collapse to 0/"" and the second case would
    # promote — the fail-OPEN direction, unreviewed third-party content out of quarantine.
    row = next((it for it in fetched if it["number"] == 42), None)  # updated_at ...:42Z
    edited = epoch(row["updatedAt"]) if row else 0  # a truncated-away row reports as FAIL, not a crash
    chk("fetched row: 👍 after the last edit promotes",
        should_promote([{"content": "+1", "user": "jeswr", "created_at": edited + 1}], edited, M), True)
    chk("fetched row: 👍 before the last edit does NOT promote",
        should_promote([{"content": "+1", "user": "jeswr", "created_at": edited - 1}], edited, M), False)
    chk("the fetched timestamp is a real edit time, not the epoch", edited > 0, True)

    def raises(fn):
        try:
            fn()
            return False
        except SystemExit:
            return True

    chk("a row with no updated_at fails closed (never 'edited at the epoch')",
        raises(lambda: fetch_quarantined("o/r", run=lambda a: _Resp(json.dumps([[{"number": 7, "labels": []}]])))), True)
    chk("a runaway snapshot fails closed", raises(lambda: fetch_quarantined("o/r", run=_stub_gh, ceiling=10)), True)
    chk("a failed gh call fails closed (never an empty queue)",
        raises(lambda: fetch_quarantined("o/r", run=lambda a: _Resp("", returncode=1))), True)

    print("promote self-test", "PASSED" if ok else "FAILED")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(_self_test() if "--self-test" in sys.argv else 0)
