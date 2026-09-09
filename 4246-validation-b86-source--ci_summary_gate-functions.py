# scripts/ci_summary_gate.py:1225-1234
def is_advisory(name: str) -> bool:
    """[OPUS-5] #3773 — the SINGLE non-gating classifier: DECLARED in the advisory
    registry, or on the exact platform-managed allow-list. NOT a name rule — the
    display-name regex this used to consult was the #3773 correctness hole.

    Every consumer inherits the same answer — the verdict (render_verdict),
    fail-fast (failfast_failures) AND the resolver's run-level synthetic check
    (advisory_only_failure). Splitting them would leave the Dependabot workflow's red
    run to acquire a synthetic gating verdict and red the gate anyway."""
    return is_declared_advisory(name) or is_platform_managed_advisory(name)

# scripts/ci_summary_gate.py:1344-1365
def fm_group_run_id(runs: list[dict]) -> str:
    """[FABLE-5] PR #3511 finding 2: the CURRENT feature-matrix run id, extracted as
    the MAXIMUM `/actions/runs/<id>` seen across the `opt-in group (…)` check-runs'
    details_url — name-only (is_fm_group), so the zero-leg skeleton (a check-run of
    the current run) is INCLUDED. GitHub Actions run ids are monotonically ascending,
    so the max is the LATEST feature-matrix run on this head — the one whose reporter
    verdict the gate must await (or, when that run is zero-leg, the one that proves no
    reporter is expected; see fm_report_status). On a same-SHA rerun (ready_for_review
    / label) the commit carries group check-runs from BOTH the stale and the fresh run;
    the fresh run's id is the larger. Returns "" if no group check carries a parseable
    run id (then correlation degrades to the pre-finding-2 any-report behaviour — never
    a false RED, and the stale-report race is only reachable on a same-SHA rerun)."""
    best = ""
    for r in runs:
        if not is_fm_group(r.get("name", "")):
            continue
        rid = fm_run_id_of(r)
        if rid:
            # Numeric max (ids are ints of possibly-different width); compare as ints.
            if best == "" or int(rid) > int(best):
                best = rid
    return best

# scripts/ci_summary_gate.py:1368-1466
def fm_report_status(runs: list[dict]) -> str:
    """[FABLE-5] PR #3511 finding 1 (HIGH) + finding 2 (HIGH, correlation): the
    reporter-await status over the (already forgiveness-filtered, self-excluded)
    sibling set. Returns one of:

      * "n/a"     — the LATEST feature-matrix run on this head produced NO legs, so
                    NO reporter is expected. Two shapes: (1) no `opt-in group (…)`
                    check-run at all — a doc-only PR, a fully change-selected-out
                    matrix, or a merge_group that skipped the lane; (2) the latest
                    run posted ONLY the zero-leg skeleton (unexpanded placeholder,
                    server-set skipped) — no real leg ran, so the reporter correctly
                    posts nothing. Presence is LATEST-RUN-RELATIVE (finding, r3): an
                    OLDER real group run's leftover check on a same-SHA rerun does
                    NOT resurrect the requirement when the current run is zero-leg.
      * "ok"      — legs ran AND a terminal-SUCCESS `feature-matrix report`
                    check-run FOR THE CURRENT GROUP RUN is present: the reporter
                    posted its green verdict.
      * "failed"  — legs ran AND the CURRENT run's reporter check-run is terminal
                    but NOT success (crashed / completeness-violation / POST error).
                    The caller REDs. (Such a check also fails the normal gating-set
                    render on its own — this is belt-and-braces so the reporter
                    requirement is explicit and cannot be silently dropped.)
      * "pending" — legs ran but the CURRENT run's `feature-matrix report` check-run
                    is either ABSENT (its workflow_run has not landed it yet, or
                    crashed before posting) or present-but-not-terminal. The gate
                    must keep polling (still-settling); budget exhaustion in this
                    state FAILS CLOSED via render_verdict's reporter belt — never
                    a conclude-by-timing over the group jobs' bare successes.

    CORRELATION (finding 2 — same-SHA stale-report race): feature-matrix reruns on the
    SAME head SHA (ready_for_review / label events), so a STALE report from an earlier
    run can sit on the commit while the CURRENT run's reporter is delayed/crashed. Each
    reporter embeds its TRIGGERING feature-matrix run id as the report's external_id
    (server-supplied, unforgeable); the CURRENT group run's id is fm_group_run_id(runs)
    (max `/actions/runs/<id>` across the group checks). Only a report whose external_id
    equals that id counts — a stale report (older run id) is IGNORED, so it can never
    satisfy the hold for a fresh group run. When the current group run id is
    unresolvable (no parseable url — not expected in prod) OR no report carries an
    external_id (legacy reporter), we fall back to matching ANY report: a graceful
    degradation to the pre-finding-2 behaviour that is never a false RED and only
    weakens the stale-race defence, which is unreachable absent a same-SHA rerun.

    SAFETY: a reporter that FAILED to post (delayed / crashed) is indistinguishable
    from one that has not posted YET, and both map to "pending" — the fail-CLOSED
    direction. A same-SHA fork PR whose reporter is fork-denied (cannot POST with a
    read-only token) will hold here to timeout and RED; that is acceptable — a fork
    PR is not auto-merged into the queue and fail-closed is the safe posture the
    finding demands."""
    # PRESENCE is decided relative to the LATEST feature-matrix run (finding, r3).
    # A same-SHA rerun (ready_for_review / label) leaves an OLDER real group run's
    # check-runs on the commit alongside a NEWER zero-leg skeleton run; keying
    # presence off ANY real group (the old run's) while keying the run id off the
    # newer skeleton deadlocked the gate — it awaited a reporter the zero-leg run
    # correctly never posts. So: bucket the group checks by the run they belong to
    # and judge the presence + reporter requirement of the LATEST run only.
    group_checks = [r for r in runs if is_fm_group(r.get("name", ""))]
    if not group_checks:
        return "n/a"  # no feature-matrix legs on this head at all
    real_groups = [r for r in group_checks if is_real_fm_group(r)]
    if not real_groups:
        return "n/a"  # only zero-leg skeleton(s) anywhere — no real leg ever ran
    current_run_id = fm_group_run_id(runs)  # max run id across ALL group checks
    # Can we bucket by run id? Only if a latest run id resolved AND every REAL group
    # carries a parseable run id — otherwise a real leg of an UNKNOWN (possibly newer)
    # run may exist, so we cannot safely declare the latest run zero-leg. In that
    # fail-CLOSED case we keep the reporter requirement (real legs ran somewhere) and
    # let current_run_id (possibly "") drive the graceful any-report degradation below.
    real_unparseable = any(not fm_run_id_of(r) for r in real_groups)
    if current_run_id and not real_unparseable:
        latest_run_checks = [r for r in group_checks
                             if fm_run_id_of(r) == current_run_id]
        if not any(is_real_fm_group(r) for r in latest_run_checks):
            # The LATEST run posted only the zero-leg skeleton — no leg ran, no
            # reporter is expected. (A stale OLDER real run's leftover check does not
            # matter; its own reporter, if any, is not what this head awaits.)
            return "n/a"
        # else: the latest run DID run real legs (a forged placeholder name with a
        # server-set NON-skipped conclusion still counts as real — security, r2) —
        # require its reporter, correlated to current_run_id.
    reports = [r for r in runs if is_fm_report(r.get("name", ""))]
    if not reports:
        return "pending"  # legs ran; reporter verdict not on the head SHA yet
    # CORRELATION: bind the verdict to the CURRENT group run. Prefer the report(s)
    # whose external_id equals the current group run id; ignore stale reports.
    any_report_has_extid = any((r.get("external_id") or "") for r in reports)
    if current_run_id and any_report_has_extid:
        matched = [r for r in reports if (r.get("external_id") or "") == current_run_id]
        if not matched:
            # Every report on this SHA is for an OLDER feature-matrix run (or carries
            # no external_id). The current run's verdict has not landed — keep waiting
            # (fail-closed on budget exhaustion). A stale success can never green us.
            return "pending"
        reports = matched
    # If ANY (current-run) report check is non-terminal, keep waiting; else judge them.
    if any(r.get("status") != "completed" for r in reports):
        return "pending"
    if all(r.get("conclusion") == "success" for r in reports):
        return "ok"
    return "failed"

# scripts/ci_summary_gate.py:1548-1729
def render_verdict(runs: list[dict], summary_path: str = "", tier_ctx: TierContext | None = None) -> int:
    """Shared by the clean-converge, graceful-timeout, and post-extension paths, so
    every path applies IDENTICAL gating semantics. Returns the process exit code.

    DRAFT-TIER INTEGRITY ([FABLE-5], see the header): with a TierContext,
      * a FULL-tier pull_request verdict REDs while any draft-tier-marked select
        INSTANCE lacks its own later full-tier successor (stale draft-tier leg
        set — at least one selecting workflow's ready_for_review full run has
        not registered on this SHA);
      * a DRAFT-tier verdict that would otherwise be SUCCESS first re-reads the
        PR's CURRENT draft state from the API: no-longer-draft => FAILURE ("stale
        draft-tier run, full run pending"), and an unreadable state fail-closes
        to FAILURE after bounded retries. A draft-tier verdict that is already a
        FAILURE skips the re-check (a RED can never be latched by the queue).
    Without a TierContext (tests / push / merge_group) the semantics are exactly
    the pre-draft-tier ones.

    SELECTION SEMANTICS ([FABLE-5] sq-fmx4u.3, design §5.3): a `skipped`
    conclusion is satisfied ONLY when the change-based selection pre-job
    (is_select) succeeded — a skip is trustworthy iff the thing that decided to
    skip ran to a successful conclusion. Concretely:
      * every select check-run present must have conclusion == "success";
        anything else (failure, cancelled, skipped, neutral, stale) REDs the
        gate outright, even if every other sibling is green — an unobservable
        selection means the skips on this commit are unattributable (§4.3);
      * with select green (or absent — e.g. a pre-selection sibling set, where
        no skip was produced by selection), `skipped` stays non-failing exactly
        as before. Absent-select degradation is deliberately the PRE-sq-fmx4u.3
        behaviour, never a new failure mode.
    A job that FAILED still fails the gate regardless of selection — selection
    can only ever decide whether a SKIP is satisfied, never mask a failure."""
    # [FABLE-5] Draft-tier belt: a full-tier pull_request gate must never conclude
    # over a leg set whose selection was assembled draft-tier (checked FIRST — it
    # invalidates the whole set, including an otherwise-green one).
    if tier_ctx and tier_ctx.run_tier == "full" and tier_ctx.event_name == "pull_request":
        stale = draft_selects_unsuperseded(runs)
        if stale:
            counts: dict[str, int] = {}
            for n in stale:
                counts[n] = counts.get(n, 0) + 1
            detail = ", ".join(
                f"{n} ×{c}" if c > 1 else n for n, c in sorted(counts.items())
            )
            _emit(
                "### ci-summary: FAILED — stale draft-tier run, full run pending. The "
                "selection on this head SHA is (at least partly) draft-tier-assembled: "
                f"{len(stale)} draft-marked select instance(s) have no OWN later "
                f"full-tier successor ({detail}). Each selecting workflow's "
                "ready_for_review full-tier re-run must register its own successor "
                "(ci/bench/feature-matrix/fuzz share one select name — one full-tier "
                "select must never release the hold for the others). A draft-tier leg "
                "set must never admit a non-draft PR to the merge queue "
                "(docs/branch-protection.md §Draft-tier CI). " + UNSAT_HOLD_REMEDY,
                summary_path,
            )
            print("::error::ci-summary failed — stale draft-tier leg set on a non-draft head.")
            return 1
    # [FABLE-5] PR #3511 finding 1 (HIGH): STRUCTURAL AWAIT of the trusted
    # feature-matrix reporter. If `opt-in group (…)` legs ran for this head, the
    # `feature-matrix report` summary check MUST be present and terminal-SUCCESS
    # before the gate can conclude green. This is reached only on a would-CONCLUDE
    # render (clean settle or budget-exhaustion timeout), so a "pending" here at
    # RENDER time is FAIL-CLOSED, never a conclude-by-timing: the poll loop holds
    # the settle window open while the report is missing/pending (report_pending),
    # and a render still finding it unresolved means the reporter never landed
    # within the loop's own timeout. A "failed" reporter (crashed / completeness
    # violation) REDs here too (belt-and-braces: its own check-run also fails the
    # gating-set render below). Checked BEFORE the empty-set / normal-render paths
    # so a group set that is otherwise all-green cannot pass over an absent verdict.
    fm = fm_report_status(runs)
    if fm != "n/a" and fm != "ok":
        if fm == "failed":
            _emit(
                "### ci-summary: FAILED — the trusted `feature-matrix report` reporter "
                "concluded a NON-SUCCESS verdict (crashed / artifact-completeness "
                "violation / check-run POST error). The feature-matrix legs ran (an "
                "`opt-in group (…)` check-run is present on this head), so the reporter's "
                "verdict is required and it failed (fail-closed, PR #3511 finding 1).",
                summary_path,
            )
            print("::error::ci-summary failed — the feature-matrix reporter concluded non-success.")
        else:
            _emit(
                "### ci-summary: FAILED — the trusted `feature-matrix report` reporter "
                "verdict never landed on this head SHA within the gate's budget. The "
                "feature-matrix legs ran (an `opt-in group (…)` check-run is present), so "
                "its `feature-matrix report` summary check-run is STRUCTURALLY REQUIRED — "
                "a delayed or crashed reporter must never race past the gate. Fail-closed "
                "(PR #3511 finding 1): re-run the feature-matrix-report workflow (or push "
                "a new head) so the reporter posts its verdict.",
                summary_path,
            )
            print("::error::ci-summary failed — the feature-matrix reporter verdict is missing (fail-closed).")
        return 1
    total = len(runs)
    if total == 0:
        if _draft_recheck(tier_ctx, summary_path) != 0:
            return 1
        _emit("ci-summary: no sibling checks to aggregate (stable empty set) — passing.", summary_path)
        return 0
    gating = [r for r in runs if not is_advisory(r.get("name", ""))]
    excluded = total - len(gating)
    # [OPUS-5] #3773: make the formerly-silent exclusion loud in BOTH directions —
    # every check that carries an advisory name token but is NOT declared is listed
    # here and IS in `gating` above. (Diagnostic only; see undeclared_token_names.)
    for undeclared in undeclared_token_names(runs):
        _emit(
            f"note: `{undeclared}` carries an advisory/informational NAME token but has no "
            f"declaration in {ADVISORY_REGISTRY_PATH} — it GATES (#3773). Declare it there "
            f"(with an owner_bead + promotion_criteria) or drop the misleading token.",
            summary_path,
        )
    # Selection pre-job health — searched over ALL runs (not just gating) so a
    # hypothetical advisory-renamed select could still never green-light a skip.
    # NB superseded-cancelled select INSTANCES are already dropped upstream by
    # forgive_superseded (including the deterministic-select same-name SAME-TIER
    # success race-loser rule for the PURE select pre-job, sq-fmx4u.3
    # hardening), so any cancelled select that SURVIVES to here has NO
    # qualifying sibling (no strictly-later successor, and — for a pure select —
    # no same-tier same-name success either) and rightly REDs.
    select_runs = [r for r in runs if is_select(r.get("name", ""))]
    select_ok = all(r.get("conclusion") == "success" for r in select_runs)
    skipped_ct = sum(1 for r in gating if r.get("conclusion") == "skipped")
    if not select_ok:
        _emit(
            f"### ci-summary: FAILED — the change-based test-selection pre-job did not "
            f"succeed, so the {skipped_ct} skipped gating check(s) on this commit cannot "
            f"be attributed to a sound selection (fail-closed, sq-fmx4u.3 / design §4.3).",
            summary_path,
        )
        for r in select_runs:
            _emit(f"- ✗ {r.get('name')}: {r.get('conclusion') or 'incomplete'}", summary_path)
        print("::error::ci-summary failed — the selection pre-job must conclude success.")
        return 1

    def _satisfied(r: dict) -> bool:
        c = r.get("conclusion")
        if c == "skipped":
            return select_ok  # always True past the gate above; kept explicit so a
            # future refactor that moves this check cannot silently trust a skip.
        return c in _PASSING

    failed = [r for r in gating if not _satisfied(r)]
    if failed:
        _emit(
            f"### ci-summary: FAILED — {len(failed)} non-passing gating check(s) of "
            f"{len(gating)} gating ({excluded} advisory check(s) excluded — each "
            f"DECLARED in {ADVISORY_REGISTRY_PATH} or on the platform-managed "
            f"allow-list)",
            summary_path,
        )
        for r in failed:
            _emit(f"- ✗ {r.get('name')}: {r.get('conclusion') or 'incomplete'}", summary_path)
        print("::error::ci-summary failed — see the non-passing gating checks above.")
        return 1
    if _draft_recheck(tier_ctx, summary_path) != 0:
        return 1
    _emit(
        # [OPUS-5] #3774 review: the excluded set is DECLARED-in-the-registry OR on the
        # exact platform-managed allow-list (PLATFORM_MANAGED_ADVISORY_NAMES) — saying
        # "each DECLARED in the registry" understated the second, smaller source.
        f"### ci-summary: PASSED — all {len(gating)} gating check(s) green (or skipped/neutral); "
        f"{excluded} advisory check(s) excluded (each DECLARED in "
        f"{ADVISORY_REGISTRY_PATH}, or on the exact platform-managed allow-list); "
        f"set stable."
        + (
            " DRAFT-TIER verdict (reduced leg set; PR draft state re-confirmed). This "
            f"check-run is `{DRAFT_TIER_GATE_NAME}`, never the required `{GATE_CHECK_NAME}` "
            "context — it cannot satisfy branch protection; the full matrix re-runs at "
            "ready_for_review and only its full-tier gate can."
            if tier_ctx and tier_ctx.run_tier == "draft"
            else ""
        ),
        summary_path,
    )
    if select_runs:
        _emit(
            f"selection: {len(gating) - skipped_ct} of {len(gating)} gating check(s) ran, "
            f"{skipped_ct} skipped (selection and/or path-filter; selection pre-job succeeded).",
            summary_path,
        )
    return 0

# scripts/ci_summary_gate.py:2320-2338
def make_fetch_check_runs(repo: str, sha: str):
    def fetch() -> list[dict]:
        # started_at + id feed the superseded-run ordering (draft-tier CI): a
        # cancelled/stale check-run is forgiven only for a strictly LATER
        # same-normalized-name successor.
        return _gh_json_lines(
            [
                f"repos/{repo}/commits/{sha}/check-runs",
                "--paginate",
                "--jq",
                # external_id carries the reporter's finding-2 correlation token (the
                # triggering feature-matrix run id) so fm_report_status can bind a
                # `feature-matrix report` verdict to the CURRENT `opt-in group (…)`
                # run and reject a stale same-SHA report from an earlier run.
                ".check_runs[] | {name, status, conclusion, details_url, html_url, started_at, id, external_id}",
            ]
        )

    return fetch
