#!/usr/bin/env python3
# [FABLE-5] Issue-native orchestration: the designed RETRIAGE cron pass (audit-2026-07-17).
"""retriage.py — re-run static triage over parked `status:untriaged` issues, fail-closed.

`triage-issue.yml` only fires on opened/edited/reopened, and GitHub's `edited` activity does NOT
cover label changes — so an issue that lands `status:untriaged` (e.g. opened without a priority)
was TERMINAL: even a human later adding `priority:P2` never re-ran triage. This sweep closes the
loop the triage.py docstring already promised ("status:untriaged for the retriage cron"):

  * OPEN issues carrying `status:untriaged` are considered, PLUS every OPEN issue with NO
    `status:*` label at all — an issue the event triager never ran on. GITHUB_TOKEN event
    suppression (the `flow-on` case, #2474) is one way that happens; bulk/API issue creation is
    another, and #2474's `flow-on`-only widening covered only the first. See `_needs_triage`;
  * `trust:untrusted` is excluded (owned by promote-on-approval) and `kind:epic` is excluded
    (untriaged-by-design tracking umbrellas);
  * the author trust-gate is re-verified exactly like triage-issue.yml (the orchestrator App bot,
    or admin/maintain/write collaborators) — untrusted authors are never triaged here;
  * scripts/triage.py (the same static pass) recomputes the label delta; if the issue has NO
    priority label at all, a default `priority:P3` is applied so the loop CONVERGES instead of
    parking forever (an AMBIGUOUS priority still parks — that needs a human);
  * retriage only PROMOTES (untriaged -> ready). It never re-parks or edits gated issues, so a
    needs:* / ambiguous issue is simply left untouched.

Pure `plan_retriage()` is unit-tested (--self-test); the CLI wraps it over `gh`. Default is a
dry-run print; the cron passes --apply.
"""
import argparse
import json
import os
import subprocess
import sys
import urllib.parse

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import triage  # noqa: E402  (same-directory static-triage module)

DEFAULT_PRIORITY = "priority:P3"
TRUSTED_BOT = "sparq-orchestrator[bot]"
SKIP_LABELS = {"trust:untrusted", "kind:epic"}
# [FABLE-5] (#2474) flow-on.yml mints follow-ups under secrets.GITHUB_TOKEN, i.e. as
# github-actions[bot]. That login is trusted HERE only for `flow-on`-labelled issues:
# their content comes from the checked-in rule table (scripts/flow-on-rules.toml), not
# third-party input — this is NOT blanket [bot] trust (an arbitrary github-actions[bot]
# issue without the flow-on label still fails the collaborator probe and is skipped).
FLOW_ON_MINTER = "github-actions[bot]"
FLOW_ON_LABEL = "flow-on"


def _needs_triage(labels):
    """`status:untriaged`, or NO `status:*` label at all.

    [OPUS-5] The statusless arm used to additionally require the `flow-on` label (#2474). That
    made the predicate describe ONE KNOWN SOURCE of never-triaged issues instead of the class:
    an issue is invisible to the event triager whenever the triager never ran on it, and
    GITHUB_TOKEN event suppression is only one way that happens (bulk/API issue creation is
    another). Measured live on sparq-org/sparq 2026-07-26: 242 open statusless issues, of which
    238 carried no `flow-on` label and were therefore unreachable by this predicate AND by
    `_fetch_candidates`' label queries.

    Widening the REACH does not widen what is PROMOTED. `plan_retriage` still applies the author
    trust-gate, `SKIP_LABELS`, and `triage.triage()`'s own gates, and still returns only
    PROMOTING deltas. `FLOW_ON_LABEL` keeps its distinct, narrower meaning in `plan_retriage`,
    where it is the trust exception for `github-actions[bot]` — not blanket bot trust, so the
    statusless `github-actions[bot]` issues without it stay skipped (fixture 14).
    """
    if "status:untriaged" in labels:
        return True
    return not any(lb.startswith("status:") for lb in labels)


def plan_retriage(issues, trusted):
    """[(number, add:sortedlist, remove:sortedlist)] for open issues needing triage (see
    `_needs_triage`) whose author passes `trusted(login)`. Only PROMOTING deltas are returned
    (ready==True); everything else is left untouched for a human / the future LLM pass."""
    actions = []
    for it in issues:
        labels = {lb["name"] if isinstance(lb, dict) else lb for lb in it.get("labels", [])}
        if not _needs_triage(labels) or labels & SKIP_LABELS:
            continue
        author = str(it.get("author", ""))
        flow_on_minted = author == FLOW_ON_MINTER and FLOW_ON_LABEL in labels
        if not (trusted(author) or flow_on_minted):
            continue
        add_default = not any(triage._PRIO.match(lb) for lb in labels)
        effective = labels | ({DEFAULT_PRIORITY} if add_default else set())
        result = triage.triage(effective, "task")
        if not result["ready"]:
            continue  # ambiguous priority / needs:* / no role — parked for a human, no churn
        add = set(result["add"]) | ({DEFAULT_PRIORITY} if add_default else set())
        remove = set(result["remove"])
        actions.append((it.get("number"), sorted(add - labels), sorted(remove & labels)))
    return actions


def _gh(args):
    return subprocess.run(["gh", *args], capture_output=True, text=True, check=False)


# [OPUS-5] The closed set of labels the static pass can emit. `ensure_labels` will create these and
# NOTHING else, so a typo in triage.py can never mint an arbitrary repo label.
_EMITTABLE_LABEL = ("role:", "status:", "needs:", "priority:")
# Per-run memo. A live sweep promotes hundreds of issues but emits only a handful of DISTINCT
# labels, so without this the ensure would cost ~3 API calls per issue for no new information.
_ENSURED = set()


def _ensure_labels(repo, labels):
    """Idempotently create every emittable label BEFORE the edit. Best-effort by design: creating an
    existing label reports an error, and the ADD below is the hard gate that actually decides."""
    for lb in sorted(labels):
        if lb.startswith(_EMITTABLE_LABEL) and (repo, lb) not in _ENSURED:
            _ENSURED.add((repo, lb))
            _gh(["label", "create", lb, "-R", repo, "--color", "ededed",
                 "--description", "orchestration routing label (auto-created by retriage)"])


def apply_labels(repo, number, add, remove):
    """Apply an issue's whole label delta ATOMICALLY. Returns True iff it was written.

    [OPUS-5] FAIL-CLOSED (review of PR #4211). This used to be one `_gh(["issue","edit",…,
    "--add-label", lb])` PER label with the return code never read (`_gh` is `check=False`).
    `gh issue edit --add-label` does NOT create a missing label — it fails — so a missing ROUTING
    label (a `role:*`) silently no-opped while the sibling `status:ready` write in the same loop
    SUCCEEDED. The issue was promoted as if it carried the role it does not, and route-resolve,
    finding no `role:` label, fell through to `[defaults]`. Fail-open on the dispatch path: it is
    what turned a missing `role:gui` label into a silently inverted routing preference.

    So: ensure the labels exist, then send the whole delta as ONE `gh issue edit` — all-or-nothing,
    so a failure can no longer leave `status:ready` set without the role it was paired with — and
    READ the return code, surfacing the failure to the caller instead of discarding it.
    """
    if not add and not remove:      # nothing to write (an empty `gh issue edit` errors out)
        return True
    _ensure_labels(repo, add)
    args = ["issue", "edit", str(number), "-R", repo]
    for lb in add:
        args += ["--add-label", lb]
    for lb in remove:
        args += ["--remove-label", lb]
    r = _gh(args)
    if r.returncode != 0:
        print(f"#{number}: label write FAILED (rc={r.returncode}) — the issue is left UNPROMOTED "
              f"rather than status:ready without its role: {(r.stderr or '').strip()[:300]}",
              file=sys.stderr)
        return False
    return True


def _trusted_factory(repo):
    cache = {}

    def trusted(login):
        if not login:
            return False
        if login == TRUSTED_BOT:
            return True
        if login not in cache:
            r = _gh(["api", f"repos/{repo}/collaborators/{login}/permission",
                     "--jq", ".permission"])
            cache[login] = (r.stdout or "").strip() if r.returncode == 0 else "none"
        return cache[login] in {"admin", "maintain", "write"}

    return trusted


FETCH_CEILING = 10000


def _flatten_pages(pages):
    """Flatten `gh api --paginate --slurp` output, dropping PRs (the issues endpoint returns
    both) and any non-list/non-dict junk."""
    return [i for page in pages if isinstance(page, list) for i in page
            if isinstance(i, dict) and "pull_request" not in i]


def _fetch_label(repo, label=None, ceiling=FETCH_CEILING):
    """Every OPEN issue carrying `label`, or — when `label` is None — every OPEN issue, via REAL
    cursor pagination.

    [OPUS-5] The `label=None` mode exists because a label query is STRUCTURALLY unable to return
    the class retriage exists to rescue. See `_fetch_candidates`.

    [OPUS-5] This was `gh issue list --limit 500`, which SILENTLY TRUNCATES: the CLI stops at
    the limit and reports nothing. Retriage is a WORK QUEUE sweep, so a truncated fetch is not a
    slow drain: the dropped issues are never candidates on any tick, at any point in the future,
    with no signal. `gh api --paginate` follows Link headers to exhaustion; the explicit ceiling
    still fails closed on a runaway snapshot (same shape as scripts/ready-issues.py `_fetch`).

    The defect is DEPTH-CONDITIONAL, and the measurements below are POINT-IN-TIME, not a fixed
    gain. It bites only while the label's open count exceeds the limit; under it, the truncated
    and paginated fetches are identical. Two live snapshots on sparq-org/sparq, 2026-07-26:

      queue 719  ->  truncated fetch returned 500, dropping 219 (#2360..#2750) newest-first;
                     275 vs 367 promotable, i.e. 92 promotions lost on that tick
      queue 380  ->  below the limit; both fetches return the same rows, 8 vs 8, no difference

    So this is a LATENT fault that re-arms the moment the backlog crosses the limit again — and
    it does so silently, which is exactly why it is fixed rather than tuned. Do not quote the 92
    as a standing improvement.

    Field note: the REST payload names the author `user.login`, NOT the `author.login` that
    `gh issue list --json author` emits (measured: 0 of 719 live open `status:untriaged` issue
    objects carry an `author` key; all 719 carry `user`). Getting that wrong makes every author
    read as "" and `_trusted_factory` then rejects EVERY issue — a silent total stall, not a
    visible error. Dropping `labels` fails the same silent way via `_needs_triage`.

    `_self_test` pins BOTH, and pins them end-to-end through `plan_retriage` — asserting the row
    SHAPE is not enough. Review round 2: the fixture originally carried `user` AND `author` set
    to the same login, so it satisfied the right and the wrong reading equally and the
    `author.login` mutant survived with the pinning assertion itself printing `ok`. The fixture
    now carries ONLY the keys the real payload has.
    """
    selector = "" if label is None else f"&labels={urllib.parse.quote(label)}"
    described = "open" if label is None else f"'{label}'"
    r = _gh(["api", "--paginate", "--slurp",
             f"repos/{repo}/issues?state=open{selector}&per_page=100"])
    if r.returncode != 0:
        raise SystemExit(f"retriage: could not list {described} issues")
    rows = _flatten_pages(json.loads(r.stdout or "[]"))
    if len(rows) >= ceiling:
        raise SystemExit(f"retriage: fetched {len(rows)} {described} issues >= ceiling {ceiling} — "
                         "snapshot looks runaway (fail-closed). Raise the ceiling deliberately.")
    return rows


def _fetch_candidates(repo):
    """Every OPEN issue `_needs_triage` accepts, from ONE unlabelled snapshot.

    [OPUS-5] THE REACH DEFECT. This used to run two LABEL queries (`status:untriaged`, then
    `flow-on`). A label query can only return issues that already carry a label — but the class
    this sweep exists to rescue is precisely the one the event triager NEVER ran on, which by
    definition carries NO `status:*` label at all. So the queue was defined by the very label
    whose absence defines the problem, and widening `_needs_triage` alone could not help: an
    issue that is never FETCHED is never tested. #2474 hit this and worked around it for one
    source by bolting on a second label query for `flow-on`; every other statusless issue stayed
    structurally unreachable — a fix one layer too shallow.

    Measured live on sparq-org/sparq, 2026-07-26 (1408 open issues):

      label-query reach   501 candidates  ->    0 promotable  (`retriage --repo sparq-org/sparq`
                                                 printed "0 issue(s) promotable (dry-run)")
      snapshot reach      739 candidates  ->   79 promotable

    All 79 are statusless; none of the 501 already-reachable rows is lost. The 79 pass every
    pre-existing gate unchanged — nothing here weakens `plan_retriage`'s trust gate, `SKIP_LABELS`
    or `triage.triage()`, and the sweep still emits only PROMOTING deltas. The remaining ~160
    statusless issues stay unpromoted (they carry no `area:`); clearing that population is
    `needs:area` work, not this layer's.

    One unlabelled snapshot also subsumes both old queries, so the number-dedupe the two-query
    form needed is gone with them.

    Mutation note, recorded so a reviewer does not mistake it for an untested guard: deleting the
    `_needs_triage` filter below SURVIVES the suite, and correctly so — it is a bound on the
    queue size, not a guard. `plan_retriage` re-applies the same predicate, so the resulting plan
    is identical either way. The behaviour that IS load-bearing here (the unlabelled fetch) is
    killed by "statusless issues reach the candidate queue at the FETCH layer".
    """
    return [{"number": it.get("number"), "labels": it.get("labels") or [],
             "author": ((it.get("user") or {}).get("login") or "")}
            for it in _fetch_label(repo)
            if _needs_triage({lb.get("name") for lb in (it.get("labels") or [])
                              if isinstance(lb, dict)})]


def _self_test():
    ok = True

    def chk(n, got, want):
        nonlocal ok
        good = got == want
        ok = ok and good
        print(f"  {'ok  ' if good else 'FAIL'} {n}: {got} (want {want})")

    def iss(n, labels, author="jeswr"):
        return {"number": n, "labels": labels, "author": author}

    trusted = lambda login: login in {"jeswr", TRUSTED_BOT}  # noqa: E731
    fixture = [
        # 1: human later added a priority by LABEL (the exact terminal case) -> promotes
        iss(1, ["status:untriaged", "role:impl", "priority:P2", "area:sparq-core"]),
        # 2: no priority at all -> default P3 -> promotes
        iss(2, ["status:untriaged", "kind:docs", "area:site"]),
        # 3: AMBIGUOUS priority -> needs a human, untouched
        iss(3, ["status:untriaged", "role:impl", "priority:P1", "priority:P2"]),
        # 4: epic umbrella -> untriaged-by-design, skipped
        iss(4, ["status:untriaged", "kind:epic", "priority:P1", "role:impl"]),
        # 5: quarantined -> owned by promote-on-approval, skipped
        iss(5, ["status:untriaged", "trust:untrusted", "priority:P1"]),
        # 6: untrusted author -> never triaged here
        iss(6, ["status:untriaged", "priority:P1", "role:impl"], author="rando"),
        # 7: needs:user gate -> triage refuses ready, untouched
        iss(7, ["status:untriaged", "priority:P1", "role:impl", "needs:user"]),
        # 8: not untriaged at all -> out of scope
        iss(8, ["status:ready", "priority:P1", "role:impl"]),
        # 9: orchestrator-bot author is trusted
        # [OPUS-4.8] (#2474 CI-fix) area:sparq-core so triage can reach status:ready — a no-area
        # issue fail-closes to needs:area (it would reserve the serializing __global__ partition),
        # so promotion REQUIRES an area, exactly as a real minted follow-on now carries one.
        iss(9, ["status:untriaged", "priority:P2", "kind:test", "area:sparq-core"], author=TRUSTED_BOT),
        # 10 (#2474): flow-on follow-up minted by github-actions[bot] with NO status:* label
        # (the GITHUB_TOKEN event-suppression class) -> default P3 -> promotes.
        # [OPUS-4.8] (#2474 CI-fix) carries area:docs, matching how flow-on-rules.toml now mints
        # every follow-on with an area:<crate> (a no-area issue can never reach status:ready —
        # triage fail-closes it to needs:area to keep it out of the __global__ partition).
        iss(10, ["flow-on", "auto", "docs", "area:docs"], author=FLOW_ON_MINTER),
        # 11 (#2474): github-actions[bot] WITHOUT the flow-on label -> no blanket bot trust
        iss(11, ["status:untriaged", "priority:P1", "kind:ci"], author=FLOW_ON_MINTER),
        # 12 (#2474): flow-on issue already routed (has a status:*) -> out of scope, no churn
        iss(12, ["flow-on", "auto", "status:ready", "priority:P3", "role:impl"],
            author=FLOW_ON_MINTER),
        # ---- [OPUS-5] THE REACH CLASS: statusless and NOT flow-on ---------------------------
        # 13: the fix. A trusted author's issue the event triager never ran on (bulk/API
        # creation), complete label-set, no `flow-on` -> promotes. Under the old
        # `FLOW_ON_LABEL in labels and ...` predicate this was invisible; live that class was
        # 238 issues, 79 of them promotable.
        iss(13, ["priority:P2", "role:impl", "area:sparq-core"]),
        # 14: SAME shape, github-actions[bot] author, still NO flow-on label. Widening the reach
        # must NOT widen trust: the flow-on exception in plan_retriage is what admits the bot,
        # and this issue does not qualify for it. If this ever promotes, an arbitrary
        # GITHUB_TOKEN-authored issue can self-promote onto the dispatch frontier.
        iss(14, ["priority:P2", "role:impl", "area:sparq-core"], author=FLOW_ON_MINTER),
        # 15: statusless but GATED — a needs:* gate must survive the wider reach untouched.
        iss(15, ["priority:P1", "role:impl", "area:sparq-zk", "needs:ec2"]),
        # 16: statusless epic umbrella -> still skipped by SKIP_LABELS.
        iss(16, ["kind:epic", "priority:P1", "role:impl", "area:sparq-core"]),
        # 17: statusless quarantined -> still owned by promote-on-approval.
        iss(17, ["trust:untrusted", "priority:P1", "role:impl", "area:sparq-core"]),
        # 18: statusless with NO area -> triage fail-closes (it would reserve __global__), so it
        # is left ENTIRELY untouched. Reaching an issue is not promoting it.
        iss(18, ["priority:P1", "role:impl"]),
        # 19: the zero-label class (76 live). Newly REACHED, still not promotable (no area) and
        # therefore not churned — clearing these is `needs:area` work, not this layer's.
        iss(19, []),
    ]
    actions = {n: (a, r) for n, a, r in plan_retriage(fixture, trusted)}
    # [FABLE-5] (#3419) .get defaults: a fixture dropped by a triage.py behavior change (how
    # the #2898 no-area parking drift surfaced) must print a clean FAIL line, not a KeyError.
    a1, a2, a9, a10 = (actions.get(n, ([], [])) for n in (1, 2, 9, 10))
    chk("promoted set", sorted(actions), [1, 2, 9, 10, 13])
    chk("label-added priority promotes", a1, (["status:ready"], ["status:untriaged"]))
    chk("default P3 applied", "priority:P3" in a2[0], True)
    chk("default promotion readies", ("status:ready" in a2[0],
                                      "status:untriaged" in a2[1]), (True, True))
    chk("role derived for default case", any(x.startswith("role:") for x in a2[0]), True)
    chk("bot author promoted", bool(a9[0]) and a9[0][0].startswith(("priority", "role", "status")), True)
    chk("flow-on statusless backlog promoted (#2474)",
        ("status:ready" in a10[0], "priority:P3" in a10[0],
         any(x.startswith("role:") for x in a10[0])), (True, True, True))
    chk("github-actions[bot] not blanket-trusted", 11 in actions, False)
    chk("already-routed flow-on untouched", 12 in actions, False)
    chk("ambiguous priority untouched", 3 in actions, False)
    chk("epic skipped", 4 in actions, False)
    chk("quarantine skipped", 5 in actions, False)
    chk("untrusted author skipped", 6 in actions, False)
    chk("needs:user untouched", 7 in actions, False)

    # ---- [OPUS-5] the REACH class + the gates that must survive widening it -------------------
    # Behavioural, not spelling: restoring `FLOW_ON_LABEL in labels and ...` in _needs_triage
    # drops #13 from the promoted set and reds the first of these.
    a13 = actions.get(13, ([], []))
    chk("statusless non-flow-on issue is REACHED and promoted",
        ("status:ready" in a13[0], "status:untriaged" in a13[1]), (True, False))
    chk("wider reach does NOT widen bot trust (statusless github-actions[bot])",
        14 in actions, False)
    chk("statusless needs:* gate still refused", 15 in actions, False)
    chk("statusless epic still skipped", 16 in actions, False)
    chk("statusless quarantine still skipped", 17 in actions, False)
    chk("reached-but-area-less issue is left entirely untouched", 18 in actions, False)
    chk("zero-label issue is reached yet not promoted", 19 in actions, False)
    chk("_needs_triage accepts the statusless class regardless of flow-on",
        (_needs_triage({"priority:P2"}), _needs_triage(set()),
         _needs_triage({"status:ready"}), _needs_triage({"status:deferred"})),
        (True, True, False, False))
    # A deferred/blocked/in-progress issue carries a status:* label, so the widened predicate
    # must not drag human-parked or in-flight work back into the triage queue.
    chk("parked and in-flight statuses stay out of the queue",
        any(_needs_triage({s}) for s in
            ("status:deferred", "status:blocked", "status:in-progress",
             "status:in-progress-review", "status:parked")), False)

    # IDEMPOTENCE: applying the plan and re-planning must be a no-op. Without this, a promoted
    # issue whose delta does not actually clear `_needs_triage` is re-edited on every 30-minute
    # cron fire forever.
    def _applied(issue):
        add, remove = actions.get(issue["number"], ([], []))
        return {**issue, "labels": sorted((set(issue["labels"]) | set(add)) - set(remove))}

    second = plan_retriage([_applied(i) for i in fixture], trusted)
    chk("re-running over the applied labels is a no-op (idempotent)", second, [])

    # --- candidate FETCH must not silently truncate the work queue (review of #3823) ------------
    # `gh issue list --limit 500` stops at the limit and reports nothing. MEASURED live
    # 2026-07-26: 719 open status:untriaged issues, 500 returned, 219 dropped (#2360..#2750)
    # newest-first. Retriage is a work-queue sweep, so those never become candidates on ANY
    # future tick. The stub below emulates BOTH CLI shapes faithfully — `gh api --paginate`
    # exhausts the Link chain, `gh issue list --limit N` truncates newest-first — so reverting
    # to the limited call is EXECUTABLE here and fails on the missing rows rather than on an
    # unrecognised command. That is what makes this a behavioural guard and not a spelling test.
    class _Resp:
        def __init__(self, stdout, returncode=0):
            self.stdout, self.returncode = stdout, returncode

    total = 620                     # > 500, so a --limit 500 fetch must lose the oldest 120
    def _row(n, name):
        # [OPUS-5] `user` ONLY — deliberately NO `author` key, mirroring the real REST payload
        # (measured: 0 of 719 live open status:untriaged issue objects carry `author`; all 719
        # carry `user`). Carrying both made the fixture satisfy the RIGHT and the WRONG reading
        # equally, so `author.login` was indistinguishable from `user.login` and the mutant
        # survived with this very assertion printing `ok`. In production that mutant resolves
        # every author to "", _trusted_factory then rejects the whole queue, and retriage
        # silently promotes NOTHING — a total stall of the drain this fetch exists to unblock.
        return {"number": n, "labels": [{"name": name}, {"name": "priority:P2"},
                                        {"name": "role:impl"}, {"name": "area:sparq-core"}],
                "user": {"login": "jeswr"}}

    # [OPUS-5] The corpus is now the repo's whole OPEN-ISSUE set, not a per-label index, because
    # the candidate fetch is now one unlabelled snapshot. Modelling it this way is what makes the
    # REACH guard executable: the stub filters by `labels=` when the caller sends a selector, so
    # reverting `_fetch_candidates` to the old two-label queries really does hide STATUSLESS
    # below, and the assertion fails on missing rows rather than on an unknown command line.
    STATUSLESS = [7001, 7002, 7003]      # never triaged: no `status:*` label, and no `flow-on`
    corpus = [_row(n, "status:untriaged") for n in range(1, total + 1)]
    # #7 and #8 additionally carry `flow-on`: under the old two-query union they came back from
    # BOTH queries and only the number-dedupe kept them out of the candidate list twice.
    for n in (7, 8):
        corpus[n - 1]["labels"].append({"name": FLOW_ON_LABEL})
    # The reach class. Complete label-set, trusted author, and NO status label of any kind — the
    # 238-issue live population the label queries could not express.
    for n in STATUSLESS:
        corpus.append({"number": n, "user": {"login": "jeswr"},
                       "labels": [{"name": "priority:P2"}, {"name": "role:impl"},
                                  {"name": "area:sparq-core"}]})
    seen_cmds = []

    def _labels_of(row):
        return {lb["name"] for lb in row["labels"]}

    def _stub_gh(args):
        """Faithful emulator of BOTH gh shapes, so either implementation is EXECUTABLE here.

        `gh api --paginate` follows the Link chain to exhaustion; WITHOUT --paginate gh returns
        only the first page; with a `labels=` selector it returns only matching rows and without
        one it returns the whole open set. `gh issue list --limit N` truncates newest-first.
        Modelling all three faithfully is what lets a reverted fetch — whether reverted to the
        limited CLI call or to the label-scoped query — fail on missing ROWS.
        """
        seen_cmds.append(list(args))
        if args[0] == "api":
            query = args[-1]
            if "labels=" in query:
                label = urllib.parse.unquote(query.split("labels=")[1].split("&")[0])
                rows = [r for r in corpus if label in _labels_of(r)]
            else:
                rows = list(corpus)
            pages = [rows[i:i + 100] for i in range(0, len(rows), 100)] or [[]]
            return _Resp(json.dumps(pages if "--paginate" in args else pages[:1]))
        if args[0:2] == ["issue", "list"]:
            label = args[args.index("--label") + 1]
            rows = [r for r in corpus if label in _labels_of(r)]
            limit = int(args[args.index("--limit") + 1]) if "--limit" in args else len(rows)
            return _Resp(json.dumps(sorted(rows, key=lambda r: -r["number"])[:limit]))
        raise AssertionError(f"unexpected gh invocation: {args}")

    real_gh = globals()["_gh"]
    globals()["_gh"] = _stub_gh
    try:
        cands = _fetch_candidates("o/r")
        nums = sorted(c["number"] for c in cands)
        expected = total + len(STATUSLESS)
        chk("every candidate is fetched, not just the first page", len(nums), expected)
        chk("the OLDEST issues survive the fetch (the truncation casualties)",
            nums[:3], [1, 2, 3])
        chk("no issue is dropped or duplicated", (nums[0], nums[-1], len(set(nums))),
            (1, STATUSLESS[-1], expected))
        # THE REACH GUARD. A label-scoped fetch cannot return an issue that carries no status
        # label, so restoring the two label queries drops all of STATUSLESS here.
        chk("statusless issues reach the candidate queue at the FETCH layer",
            [n for n in STATUSLESS if n in set(nums)], STATUSLESS)
        # #7/#8 carry both old query labels; a re-introduced union without dedupe enqueues them
        # twice and plan_retriage emits two label-edit actions for the same issue.
        chk("an issue matching BOTH old label queries appears exactly once",
            [nums.count(n) for n in (7, 8)], [1, 1])
        # REST names the author `user.login`; reading `author.login` yields "" for every issue
        # and _trusted_factory then rejects the entire queue — a silent total stall.
        chk("author login is read from the REST `user` field",
            {c["author"] for c in cands}, {"jeswr"})
        chk("labels are carried through the fetch",
            sorted(lb["name"] for lb in cands[0]["labels"]),
            ["area:sparq-core", "priority:P2", "role:impl", "status:untriaged"])
        # [OPUS-5] END-TO-END. Every field the fetch emits is only meaningful because
        # plan_retriage consumes it, and each one fails the SAME silent way: drop `labels` and
        # _needs_triage is False, drop `author` and the trust gate rejects everything — either
        # way retriage promotes NOTHING and prints no error. Asserting the row SHAPE cannot see
        # that; asserting the resulting ACTIONS can. This pins number + labels + author at once.
        actions_e2e = plan_retriage(cands, lambda login: login == "jeswr")
        chk("fetched rows actually drive promotions (not a silently empty sweep)",
            (len(actions_e2e), actions_e2e[0] if actions_e2e else None),
            (expected, (1, ["status:ready"], ["status:untriaged"])))
        # ...and the reach class specifically must survive the WHOLE pipeline, not just the fetch.
        e2e = {n: (a, r) for n, a, r in actions_e2e}
        chk("statusless candidates promote end-to-end through the real fetch",
            [e2e.get(n, ([], []))[0] for n in STATUSLESS], [["status:ready"]] * len(STATUSLESS))
        chk("the fetch uses cursor pagination, not a fixed page limit",
            all(c[0] == "api" and "--paginate" in c for c in seen_cmds), True)
        # fail-closed ceiling: a runaway snapshot must raise, never silently half-report
        for label in ("status:untriaged", None):
            try:
                _fetch_label("o/r", label, ceiling=10)
                chk(f"runaway snapshot fails closed (labels={label})", "no raise", "SystemExit")
            except SystemExit:
                chk(f"runaway snapshot fails closed (labels={label})", "SystemExit", "SystemExit")
        # PR rows come back from the issues endpoint and must not enter the triage queue
        corpus.append({"number": 9001, "labels": [{"name": "status:untriaged"}],
                       "user": {"login": "jeswr"}, "pull_request": {"url": "x"}})
        chk("pull requests are not retriage candidates",
            9001 in {c["number"] for c in _fetch_candidates("o/r")}, False)
    finally:
        globals()["_gh"] = real_gh

    # -------------------------------------------------------------------------------------------
    # [OPUS-5] THE WRITE PATH IS FAIL-CLOSED (review of PR #4211).
    # The defect was not in the PLAN, it was in APPLYING it: one `gh issue edit --add-label` per
    # label, return code unread. A missing `role:*` label fails that call while the SIBLING
    # `status:ready` call in the same loop succeeds — the issue promotes with no role and
    # route-resolve falls through to [defaults]. These assertions are behavioural: they drive
    # apply_labels() against a stub gh and inspect the calls it actually made.
    # -------------------------------------------------------------------------------------------
    writes = []

    class _WResp:
        def __init__(self, returncode=0, stderr=""):
            self.stdout, self.returncode, self.stderr = "", returncode, stderr

    def _write_gh(args, fail_on_edit=False):
        writes.append(list(args))
        if args[0:2] == ["issue", "edit"] and fail_on_edit:
            return _WResp(1, "'role:gui' not found\nfailed to update 1 issue")
        return _WResp()

    real_gh = globals()["_gh"]
    try:
        globals()["_gh"] = lambda a: _write_gh(a, fail_on_edit=False)
        okw = apply_labels("o/r", 42, ["role:gui", "status:ready"], ["status:untriaged"])
        edits = [c for c in writes if c[0:2] == ["issue", "edit"]]
        chk("a successful delta is written", okw, True)
        # ALL-OR-NOTHING: one call carrying the WHOLE delta. Two calls is the defect — the second
        # one lands even when the first has failed.
        chk("the whole label delta goes in ONE edit call, not one call per label", len(edits), 1)
        chk("the role label and status:ready are written by the SAME call",
            ("role:gui" in edits[0] and "status:ready" in edits[0]), True)
        chk("the removal is in that same call", "status:untriaged" in edits[0], True)
        # ensure_labels runs FIRST, so a label the repo lacks is created rather than silently
        # dropping the routing signal (the `role:gui` case exactly).
        creates = [c for c in writes if c[0:2] == ["label", "create"]]
        chk("every emittable add-label is ensured to exist before the edit",
            sorted(c[2] for c in creates), ["role:gui", "status:ready"])
        chk("the ensure runs BEFORE the edit", writes.index(edits[0]) > len(creates) - 1, True)

        # THE DECISIVE ONE: when the write fails, the caller must LEARN it. Before this fix the
        # rc was never read and the cron exited 0 with the issue half-labelled.
        writes.clear()
        globals()["_gh"] = lambda a: _write_gh(a, fail_on_edit=True)
        chk("a FAILED label write is reported, not discarded",
            apply_labels("o/r", 42, ["role:gui", "status:ready"], []), False)
        chk("a failed write does not retry-promote with a second call",
            len([c for c in writes if c[0:2] == ["issue", "edit"]]), 1)

        # Nothing to write -> no gh call at all (an empty `gh issue edit` errors out).
        writes.clear()
        globals()["_gh"] = lambda a: _write_gh(a, fail_on_edit=True)
        chk("an empty delta issues no edit", (apply_labels("o/r", 42, [], []), writes), (True, []))

        # The auto-create is restricted to the closed set triage.py emits: a typo cannot mint an
        # arbitrary repo label.
        writes.clear()
        globals()["_gh"] = lambda a: _write_gh(a, fail_on_edit=False)
        apply_labels("o/r", 42, ["area:sparq-core", "role:impl"], [])
        chk("only role:/status:/needs:/priority: labels are auto-created",
            sorted(c[2] for c in writes if c[0:2] == ["label", "create"]), ["role:impl"])
        # The ensure is memoised per run: a sweep promoting hundreds of issues emits only a
        # handful of DISTINCT labels, so re-creating them per issue is pure API cost.
        writes.clear()
        apply_labels("o/r", 43, ["role:impl", "needs:area"], [])
        chk("an already-ensured label is not re-created for the next issue "
            "(role:impl was ensured above; only the new needs:area is created)",
            [c[2] for c in writes if c[0:2] == ["label", "create"]], ["needs:area"])
        chk("...but the edit itself still happens for that issue",
            len([c for c in writes if c[0:2] == ["issue", "edit"]]), 1)
    finally:
        globals()["_gh"] = real_gh

    print("retriage self-test", "PASSED" if ok else "FAILED")
    return 0 if ok else 1


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--repo", default="sparq-org/sparq")
    ap.add_argument("--apply", action="store_true", help="apply the label deltas (cron mode)")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()
    if args.self_test:
        return _self_test()
    actions = plan_retriage(_fetch_candidates(args.repo), _trusted_factory(args.repo))
    failed = []
    for number, add, remove in actions:
        print(f"#{number}: +{','.join(add) or '-'} -{','.join(remove) or '-'}")
        if args.apply and not apply_labels(args.repo, number, add, remove):
            failed.append(number)
    print(f"retriage: {len(actions)} issue(s) {'promoted' if args.apply else 'promotable (dry-run)'}")
    if failed:
        # [OPUS-5] Do NOT exit 0 on a discarded write. A swallowed label failure is invisible in a
        # cron and leaves the frontier quietly wrong; the run must go red so it is noticed.
        print(f"retriage: {len(failed)} issue(s) FAILED to apply and are NOT promoted: "
              f"{failed[:10]}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
