# scripts/run-feature-matrix-group.py:179-206
def run_leg(leg):
    """Build -> (test) -> clippy, exactly the per-leg matrix step set.
    Returns None on success, else the name of the failing step."""
    crate, features = leg["crate"], leg["features"]
    if build_with_retry(crate, features) != 0:
        return "build"
    if leg["test"]:
        if run([CARGO, "test", "-p", crate, "--features", features]) != 0:
            return "test"
    if (
        run(
            [
                CARGO,
                "clippy",
                "-p",
                crate,
                "--features",
                features,
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ]
        )
        != 0
    ):
        return "clippy"
    return None

# scripts/run-feature-matrix-group.py:209-223
def write_results(path, group, results):
    """Persist the per-leg outcomes for the trusted reporter job. Rewritten after
    every leg so an infra death mid-group still ships the completed legs."""
    payload = {
        "schema": RESULTS_SCHEMA,
        "group": group,
        "results": [
            {"name": leg["name"], "failed_step": failed_step}
            for leg, failed_step in results
        ],
    }
    tmp = f"{path}.tmp"
    with open(tmp, "w", encoding="utf-8") as fh:
        json.dump(payload, fh, ensure_ascii=False)
    os.replace(tmp, path)

# scripts/run-feature-matrix-group.py:226-277
def main():
    try:
        legs = json.loads(os.environ["GROUP_LEGS"])
    except (KeyError, json.JSONDecodeError) as exc:
        print(f"::error::GROUP_LEGS missing/malformed: {exc}", file=sys.stderr)
        return 2
    if not isinstance(legs, list) or not legs:
        print("::error::GROUP_LEGS must be a non-empty JSON array", file=sys.stderr)
        return 2
    results_path = os.environ.get("FMG_RESULTS", "")
    if not results_path:
        # The reporter job's per-leg check-runs depend on this file — a group job
        # running without it would silently drop every leg name from the gate's
        # discovery set, so refuse to run at all.
        print("::error::FMG_RESULTS unset — nowhere to write the per-leg results the reporter job posts from", file=sys.stderr)
        return 2
    os.makedirs(os.path.dirname(results_path) or ".", exist_ok=True)
    group = os.environ.get("GROUP_NAME", "?")
    print(f"feature-matrix group {group}: {len(legs)} leg(s)", flush=True)
    results = []
    for leg in legs:
        print(f"::group::opt-in {leg['name']}", flush=True)
        failed_step = run_leg(leg)
        print("::endgroup::", flush=True)
        results.append((leg, failed_step))
        write_results(results_path, group, results)
        if failed_step is not None:
            # Keep going: every leg reports (the old matrix was fail-fast: false).
            print(
                f"::error::opt-in {leg['name']}: FAILED at {failed_step} "
                "(continuing with the group's remaining legs)",
                flush=True,
            )
    summary_path = os.environ.get("GITHUB_STEP_SUMMARY", "")
    if summary_path:
        with open(summary_path, "a", encoding="utf-8") as fh:
            fh.write(f"## feature-matrix group `{group}`\n\n")
            fh.write("| leg | result |\n|---|---|\n")
            for leg, failed_step in results:
                verdict = "✅ pass" if failed_step is None else f"❌ **{failed_step}**"
                fh.write(f"| `opt-in {leg['name']}` | {verdict} |\n")
    failed = [leg["name"] for leg, failed_step in results if failed_step is not None]
    if failed:
        print(
            f"::error title=feature-matrix group {group} FAILED::"
            f"{len(failed)} of {len(results)} leg(s) failed: "
            + "; ".join(f"opt-in {n}" for n in failed),
            flush=True,
        )
        return 1
    print(f"feature-matrix group {group}: all {len(results)} leg(s) green", flush=True)
    return 0

# scripts/run-feature-matrix-group.py:289-314
def load_valid_legs(path):
    """The assembled leg set (committed fragments via the committed assembler) —
    the ONLY names/metadata the reporter will ever put in a check-run. Returns
    {leg_name: leg_dict} or None on any malformation (reporter-side config bug)."""
    try:
        with open(path, "r", encoding="utf-8") as fh:
            obj = json.load(fh)
    except (OSError, json.JSONDecodeError) as exc:
        _report_error(f"cannot read valid-legs file {path!r}: {exc}")
        return None
    include = obj.get("include") if isinstance(obj, dict) else None
    if not isinstance(include, list) or not include:
        _report_error(f"valid-legs file {path!r} is not an assembler include-object")
        return None
    valid = {}
    for leg in include:
        if (
            not isinstance(leg, dict)
            or not isinstance(leg.get("name"), str)
            or not isinstance(leg.get("crate"), str)
            or not isinstance(leg.get("features"), str)
        ):
            _report_error(f"valid-legs file {path!r} carries a malformed leg: {leg!r}")
            return None
        valid[leg["name"]] = leg
    return valid

# scripts/run-feature-matrix-group.py:317-365
def validate_results_file(path, valid_legs, seen_names):
    """HOSTILE-INPUT validation of one group-results artifact file. The file was
    produced inside a job that ran arbitrary PR-controlled build code, so every
    byte is attacker-controlled: only a name that resolves in the assembled leg
    set, a failed_step from the fixed enum, and a conservatively-shaped group id
    are accepted. Returns [(leg_name, group, failed_step)] or None on ANY
    violation (the reporter then fails closed)."""
    try:
        with open(path, "r", encoding="utf-8") as fh:
            obj = json.load(fh)
    except (OSError, json.JSONDecodeError, UnicodeDecodeError) as exc:
        _report_error(f"results file {path!r}: unreadable/not JSON ({exc})")
        return None
    if not isinstance(obj, dict) or set(obj.keys()) != {"schema", "group", "results"}:
        _report_error(f"results file {path!r}: top level must be exactly {{schema, group, results}}")
        return None
    if obj["schema"] != RESULTS_SCHEMA:
        _report_error(f"results file {path!r}: unknown schema {obj['schema']!r} (want {RESULTS_SCHEMA!r})")
        return None
    group = obj["group"]
    if not isinstance(group, str) or not GROUP_NAME_RE.match(group):
        _report_error(f"results file {path!r}: group id fails the conservative shape check")
        return None
    if not isinstance(obj["results"], list) or not obj["results"]:
        _report_error(f"results file {path!r}: results must be a non-empty list")
        return None
    out = []
    for entry in obj["results"]:
        if not isinstance(entry, dict) or set(entry.keys()) != {"name", "failed_step"}:
            _report_error(f"results file {path!r}: entry must be exactly {{name, failed_step}}: {entry!r}")
            return None
        name, failed_step = entry["name"], entry["failed_step"]
        if not isinstance(name, str) or name not in valid_legs:
            # PR-supplied free text stops HERE: an unassembled leg name is never
            # allowed to become a check-run.
            _report_error(f"results file {path!r}: leg name {name!r} is not in the assembled leg set")
            return None
        if failed_step is not None and failed_step not in VALID_STEPS:
            _report_error(f"results file {path!r}: failed_step {failed_step!r} not in {VALID_STEPS}")
            return None
        if name in seen_names:
            # Duplicate sibling check-runs are exactly the latest-run-confusion
            # attack the trusted split exists to prevent — refuse them from the
            # honest side too.
            _report_error(f"leg {name!r} appears in multiple results files ({seen_names[name]} and {path!r})")
            return None
        seen_names[name] = path
        out.append((name, group, failed_step))
    return out

# scripts/run-feature-matrix-group.py:368-441
def post_check_run(repo, head_sha, name, group, leg, failed_step, details_url, fork_pr):
    """POST one terminal `opt-in <name>` check-run. Returns 'ok', 'fork-denied'
    (the EXPECTED read-only-token degradation — accepted ONLY when fork_pr is True)
    or 'failed' (unexpected — the caller fails the reporter job: fail CLOSED)."""
    check_name = f"opt-in {name}"
    if failed_step is None:
        conclusion, title = "success", "leg passed"
        summary = (
            f"build/test/clippy green for `-p {leg['crate']} --features "
            f"{leg['features']}` (grouped leg — ran inside feature-matrix group "
            f"`{group}`; posted by the trusted reporter job)."
        )
    else:
        conclusion, title = "failure", f"leg FAILED at {failed_step}"
        summary = (
            f"`cargo {failed_step}` failed for `-p {leg['crate']} --features "
            f"{leg['features']}`. See the feature-matrix group job `{group}` log "
            f"for the full output."
        )
    payload = {
        "name": check_name,
        "head_sha": head_sha,
        "status": "completed",
        "conclusion": conclusion,
        "output": {"title": title, "summary": summary},
    }
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
            # EVENT-GATED (finding 3): tolerated ONLY on a fork-PR workflow_run,
            # where the base repo genuinely may not annotate the fork's head. The
            # group job's own conclusion remains the (coarser) gating signal. On a
            # push / merge_group / same-repo PR the SAME error falls through to the
            # fail-closed path below (a real auth regression, never a benign denial).
            print(
                f"::warning::could not create check-run {check_name!r}: read-only "
                "token (fork PR) — the group job's own conclusion remains the "
                "gating signal",
                flush=True,
            )
            return "fork-denied"
        if attempt < POST_ATTEMPTS:
            print(
                f"check-run POST for {check_name!r} failed (attempt {attempt}/"
                f"{POST_ATTEMPTS}): {err.splitlines()[-1] if err else 'unknown error'}; retrying...",
                file=sys.stderr,
                flush=True,
            )
            time.sleep(retry_sleep)
    reason = (
        "This is not a fork-PR workflow_run (FMG_FORK_PR!=true), so a permission "
        "denial here is a real auth regression"
        if (FORK_DENIAL_MARKER in err and not fork_pr)
        else "This is not the fork-PR read-only degradation"
    )
    _report_error(
        f"check-run POST for {check_name!r} failed after {POST_ATTEMPTS} attempts "
        f"with an UNEXPECTED error ({err.splitlines()[-1] if err else 'unknown error'}). "
        f"{reason} — failing CLOSED so the preserved per-leg check contract never "
        "silently evaporates."
    )
    return "failed"

# scripts/run-feature-matrix-group.py:515-618
def report_main():
    results_dir = os.environ.get("FMG_RESULTS_DIR", "")
    valid_legs_file = os.environ.get("FMG_VALID_LEGS_FILE", "")
    repo = os.environ.get("REPO", "")
    head_sha = os.environ.get("HEAD_SHA", "")
    if not results_dir or not valid_legs_file or not repo or not head_sha:
        _report_error("--report needs FMG_RESULTS_DIR, FMG_VALID_LEGS_FILE, REPO and HEAD_SHA")
        return 2
    details_url = os.environ.get("DETAILS_URL", "")
    # CORRELATION (finding 2): the TRIGGERING feature-matrix run id, embedded as the
    # summary check-run's external_id so the gate can bind the verdict to THIS group
    # run and reject a stale same-SHA report from an earlier feature-matrix run.
    trigger_run_id = os.environ.get("TRIGGER_RUN_ID", "").strip()
    # EVENT-GATED fork tolerance (finding 3): the reporter workflow sets FMG_FORK_PR
    # from server-supplied fields; only then is a permission-denied POST benign.
    fork_pr = os.environ.get("FMG_FORK_PR", "").lower() == "true"
    # The SELECTED leg set = the exact ground truth this run should have produced
    # (finding 2). It is the setup job's selected-matrix artifact, NOT a default-
    # branch re-assemble (which would not know a leg the PR adds).
    valid_legs = load_valid_legs(valid_legs_file)
    if valid_legs is None:
        return 2
    files = sorted(glob.glob(os.path.join(results_dir, "**", "*.json"), recursive=True))
    if not files:
        if os.environ.get("GROUP_JOBS_RESULT", "") == "success":
            msg = (
                "the group jobs report success but produced ZERO results artifacts — "
                "an inconsistency that would silently drop every per-leg check-run"
            )
            _report_error(msg)
            return _emit_summary_and_return(
                repo, head_sha, details_url, fork_pr, False,
                "no results despite green groups", msg, 1, trigger_run_id)
        print(
            "::warning::no group results artifacts found (group jobs did not "
            "succeed) — nothing to report; the failed/skipped group jobs gate via "
            "ci-summary",
            flush=True,
        )
        # The group jobs' own (non-success) conclusions gate; record a neutral-ish
        # summary so the reporter's verdict is visible but non-blocking here.
        return _emit_summary_and_return(
            repo, head_sha, details_url, fork_pr, True,
            "no results to report",
            "the group jobs did not succeed, so there are no per-leg results to "
            "post; the failed/skipped group jobs gate via ci-summary.", 0, trigger_run_id)
    seen_names = {}
    all_results = []
    for path in files:
        validated = validate_results_file(path, valid_legs, seen_names)
        if validated is None:
            return _emit_summary_and_return(
                repo, head_sha, details_url, fork_pr, False,
                "malformed group results artifact",
                f"a group results artifact under {results_dir} failed hostile-input "
                "validation (schema / leg-name / duplicate). See the reporter log.", 1,
                trigger_run_id)
        all_results.extend(validated)

    # COMPLETENESS (finding 2): the observed leg-name set must equal the SELECTED
    # set EXACTLY — no subset (a lost/hidden leg), no superset (already refused by
    # validate_results_file, which rejects names not in valid_legs), no duplicates
    # (already refused via seen_names). Here we catch the remaining case: a leg the
    # selected set requires that NO artifact reported. A malicious group job that
    # exited 0 while omitting a failed leg is caught here even though its own
    # conclusion lied.
    observed = {name for name, _group, _step in all_results}
    expected = set(valid_legs.keys())
    missing = sorted(expected - observed)
    if missing:
        msg = (
            f"artifact completeness violation: {len(missing)} selected leg(s) have "
            f"no result and were dropped from the per-leg checks: "
            + "; ".join(f"opt-in {n}" for n in missing[:20])
            + (" …" if len(missing) > 20 else "")
        )
        _report_error(msg)
        # Still post the legs we DID observe (their check-runs are honest), then
        # fail closed on the incompleteness via the summary check.
        _post_all(repo, head_sha, all_results, valid_legs, details_url, fork_pr)
        return _emit_summary_and_return(
            repo, head_sha, details_url, fork_pr, False,
            "incomplete per-leg results", msg, 1, trigger_run_id)

    fork_denied, failed, posted = _post_all(
        repo, head_sha, all_results, valid_legs, details_url, fork_pr
    )
    print(
        f"reporter: {posted} check-run(s) posted, {fork_denied} fork-denied, "
        f"{failed} failed, across {len(files)} group artifact(s)",
        flush=True,
    )
    if failed:
        msg = f"{failed} per-leg check-run POST(s) failed unexpectedly — failing closed"
        _report_error(msg)
        return _emit_summary_and_return(
            repo, head_sha, details_url, fork_pr, False,
            "per-leg check-run POST failed", msg, 1, trigger_run_id)
    return _emit_summary_and_return(
        repo, head_sha, details_url, fork_pr, True,
        "all per-leg checks posted",
        f"posted {posted} per-leg `opt-in <name>` check-run(s) matching the selected "
        f"leg set exactly ({fork_denied} fork-denied). The group jobs' own "
        "conclusions remain the coarse gating signal.", 0, trigger_run_id)
