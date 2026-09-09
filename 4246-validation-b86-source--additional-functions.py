# scripts/assemble-feature-matrix.py:276-310
def filter_legs_by_selection(legs, select_mode, affected_json):
    """[FABLE-5] sq-fmx4u.3 (design §5.2): change-based selection over the leg list.

    Keep only the legs whose `crate` is in the affected closure — but ONLY when
    the selection pre-job says `--select-mode selected`. Every other input is
    FAIL-CLOSED to the FULL leg set (running more is always sound, design §2/§4.3):
      * select_mode empty / "shadow" / "full" / anything else  => full set
      * affected missing, unparsable, or not a list of strings => full set
        (with a loud stderr warning — that combination means a wiring bug, and
        the sound degradation is the status quo, never a skip).
    An affected closure of [] legitimately yields ZERO legs; the workflow's
    `setup` job emits a `legs` count so the matrix job skips instead of
    exploding on an empty `include`. Note: matrix-key selection cannot be a
    job-level `if:` on GitHub Actions — the `matrix` context is not available
    there (docs: contexts availability), which is why this filtering happens at
    assembly time. The gate aggregator discovers checks by polling, so an
    unassembled leg is simply absent (never an "expected but missing" hang);
    requiredness continues to flow through `ci-summary / gate`.
    """
    if select_mode != "selected":
        return legs
    affected = None
    if affected_json:
        try:
            affected = json.loads(affected_json)
        except json.JSONDecodeError:
            affected = None
    if not isinstance(affected, list) or not all(isinstance(a, str) for a in affected):
        sys.stderr.write(
            "warning: --select-mode selected but --affected is missing/malformed; "
            "FAILING CLOSED to the full leg set (sq-fmx4u.3, design §4.3)\n"
        )
        return legs
    keep = set(affected)
    return [leg for leg in legs if leg["crate"] in keep]

# scripts/run-feature-matrix-group.py:444-496
def post_summary_check(repo, head_sha, conclusion, title, summary, details_url, fork_pr, trigger_run_id=""):
    """POST the head-SHA `feature-matrix report` summary check-run so ci-summary can
    DISCOVER the reporter's verdict on the PR head (the reporter runs on a
    workflow_run event, off the head commit). Returns 'ok' / 'fork-denied' /
    'failed' with the same event-gated fork tolerance as post_check_run.

    CORRELATION (PR #3511 review finding 2): the check's `external_id` is set to the
    TRIGGERING feature-matrix run id. That id comes from github.event.workflow_run.id
    (server-supplied, unforgeable — not any PR-influenced artifact) and is the SAME id
    the current `opt-in group (…)` check-runs carry in their details_url. The gate
    correlates the two so a STALE same-SHA report (from an earlier feature-matrix run —
    feature-matrix reruns on ready_for_review / labels against the same head) can never
    satisfy the reporter-await for a FRESH group run."""
    payload = {
        "name": REPORT_SUMMARY_NAME,
        "head_sha": head_sha,
        "status": "completed",
        "conclusion": conclusion,
        "output": {"title": title, "summary": summary},
    }
    if trigger_run_id:
        # The gate parses this back out and requires it to equal the latest
        # `opt-in group (…)` run id (extracted from those checks' details_url).
        payload["external_id"] = str(trigger_run_id)
    if details_url:
        payload["details_url"] = details_url
    retry_sleep = float(os.environ.get("FMG_RETRY_SLEEP", "5"))
    err = ""
    for attempt in range(1, POST_ATTEMPTS + 1):
        proc = subprocess.run(
            [GH, "api", f"repos/{repo}/check-runs", "--method", "POST", "--input", "-"],
            input=json.dumps(payload).encode("utf-8"),
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
        )
        if proc.returncode == 0:
            return "ok"
        err = proc.stderr.decode("utf-8", "replace").strip()
        if FORK_DENIAL_MARKER in err and fork_pr:
            print(
                f"::warning::could not create the {REPORT_SUMMARY_NAME!r} summary "
                "check (fork PR read-only token) — the group jobs' own conclusions "
                "remain the gating signal",
                flush=True,
            )
            return "fork-denied"
        if attempt < POST_ATTEMPTS:
            time.sleep(retry_sleep)
    _report_error(
        f"summary check-run {REPORT_SUMMARY_NAME!r} POST failed after {POST_ATTEMPTS} "
        f"attempts ({err.splitlines()[-1] if err else 'unknown error'})."
    )
    return "failed"
