#!/usr/bin/env python3
# sq-eifd: REPORT-CLI adapter KIND.
#
# Covers any external SHACL engine exposed as a file-in / validation-report-out
# CLI — structurally identical to the existing EYE adapter (binary, file->file).
# Tier-1 targets (research/capability-benchmark-program.md §3.1c): Apache Jena
# `shacl validate`, pySHACL, TopBraid.
#
#   build_argv(recipe, data, shapes, report)  : substitute {data}/{shapes}/{report}
#                                               into the engine's templated argv.
#   run_report_cli(...)                        : run it, time it, parse the report
#                                               via the SHARED shacl_report_count
#                                               so the count is comparable to
#                                               sparq's report.results.len().
#
# The validation-report -> count reduction is NOT duplicated here: it delegates to
# shacl_report_count.count_report (the single source of truth for "count sh:result,
# read sh:conforms").
#
# Output: a one-row TSV line `<engine>\t<violations>\t<validate_us>` on stdout (the
# same 3-column name\tcount\tus contract bench/lubm/run.sh + the ci-bench hook
# consume), plus a JSON sidecar to stderr/`--json` with conforms + the resolved
# version. Non-zero exit ONLY on an adapter/engine error (so a benchmark can tell
# "engine says X" from "engine could not run").
#
# RECIPE shape (from competitors.json, the `report_cli` block of a competitor):
#   {
#     "cmd": ["pyshacl", "-s", "{shapes}", "-f", "turtle", "-o", "{report}", "{data}"],
#     "report_format": "turtle",      # rdflib format of the produced report
#     "report_to": "file",            # "file" (engine writes {report}) | "stdout"
#     "version_cmd": ["pyshacl", "--version"],
#     "timeout_s": 600                # per-invocation wall-clock cap (default 600)
#   }
# {data}/{shapes} are the input paths; {report} is a temp path the adapter creates
# when report_to == "file" (omit {report} from cmd for report_to == "stdout").
import json
import os
import subprocess
import sys
import tempfile
import time

# Default per-invocation wall-clock cap so a hung/interactive external
# CLI cannot hang the whole long-running gather indefinitely. Overridable per
# engine via the recipe's "timeout_s".
DEFAULT_TIMEOUT_S = 600

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import shacl_report_count  # noqa: E402  (local sibling module)


def build_argv(template, mapping):
    """Substitute {data}/{shapes}/{report} tokens in each argv element."""
    out = []
    for tok in template:
        for k, v in mapping.items():
            tok = tok.replace("{%s}" % k, v)
        out.append(tok)
    return out


def _resolve_version(version_cmd):
    if not version_cmd:
        return "unknown"
    try:
        p = subprocess.run(version_cmd, capture_output=True, text=True, timeout=60)
        line = (p.stdout or p.stderr).strip().splitlines()
        return line[0] if line else "unknown"
    except Exception:  # noqa: BLE001
        return "not-installed"


def run_report_cli(recipe, data_path, shapes_path, engine="engine", iters=1):
    """Run a report-cli engine `iters` times (min wall-clock), parse its report.

    Returns {"engine","version","conforms","violations","validate_us"}.
    Raises RuntimeError on engine/adapter failure."""
    report_format = recipe.get("report_format", "turtle")
    report_to = recipe.get("report_to", "file")
    cmd_template = recipe["cmd"]
    timeout_s = recipe.get("timeout_s", DEFAULT_TIMEOUT_S)
    version = _resolve_version(recipe.get("version_cmd"))

    best_us = None
    last_report_text = None
    with tempfile.TemporaryDirectory(prefix="report-cli-") as td:
        report_path = os.path.join(td, "report.out")
        mapping = {"data": data_path, "shapes": shapes_path, "report": report_path}
        argv = build_argv(cmd_template, mapping)
        for _ in range(max(1, iters)):
            if report_to == "file" and os.path.exists(report_path):
                os.remove(report_path)
            t0 = time.perf_counter()
            try:
                proc = subprocess.run(
                    argv, capture_output=True, text=True, timeout=timeout_s
                )
            except subprocess.TimeoutExpired as e:
                raise RuntimeError(
                    "%s timed out after %ss (override via recipe \"timeout_s\")"
                    % (engine, timeout_s)
                ) from e
            dt_us = (time.perf_counter() - t0) * 1e6
            # pySHACL exits non-zero (1) when the data does NOT conform — that is a
            # VALID report, not an engine error. So we do not treat a non-zero exit
            # as failure by itself; we require a parseable report instead.
            if report_to == "stdout":
                last_report_text = proc.stdout
            else:
                if not os.path.exists(report_path):
                    raise RuntimeError(
                        "%s produced no report file (exit %d):\n%s"
                        % (engine, proc.returncode, proc.stderr)
                    )
                with open(report_path, "r", encoding="utf-8") as fh:
                    last_report_text = fh.read()
            if best_us is None or dt_us < best_us:
                best_us = dt_us

    counts = shacl_report_count.count_report(last_report_text, fmt=report_format)
    return {
        "engine": engine,
        "version": version,
        "conforms": counts["conforms"],
        "violations": counts["violations"],
        "validate_us": int(round(best_us)),
    }


def main(argv):
    """CLI: report_cli_adapter.py --recipe <json> --data <ttl> --shapes <ttl>
            [--engine NAME] [--iters N] [--json]

    Emits `<engine>\\t<violations>\\t<validate_us>` on stdout; with --json also
    dumps the full result dict to stderr for the gather envelope."""
    recipe_json = None
    data = shapes = None
    engine = "engine"
    iters = 1
    want_json = False
    i = 0
    while i < len(argv):
        a = argv[i]
        if a == "--recipe":
            i += 1
            recipe_json = argv[i]
        elif a == "--recipe-file":
            i += 1
            with open(argv[i], "r", encoding="utf-8") as fh:
                recipe_json = fh.read()
        elif a == "--data":
            i += 1
            data = argv[i]
        elif a == "--shapes":
            i += 1
            shapes = argv[i]
        elif a == "--engine":
            i += 1
            engine = argv[i]
        elif a == "--iters":
            i += 1
            iters = int(argv[i])
        elif a == "--json":
            want_json = True
        else:
            sys.stderr.write("report_cli_adapter: unknown arg %s\n" % a)
            return 2
        i += 1
    if not (recipe_json and data and shapes):
        sys.stderr.write(
            "usage: report_cli_adapter.py --recipe <json> --data <ttl> "
            "--shapes <ttl> [--engine NAME] [--iters N] [--json]\n"
        )
        return 2
    recipe = json.loads(recipe_json)
    try:
        res = run_report_cli(recipe, data, shapes, engine=engine, iters=iters)
    except Exception as e:  # noqa: BLE001 — adapter boundary
        sys.stderr.write("report_cli_adapter: %s\n" % e)
        return 1
    sys.stdout.write("%s\t%d\t%d\n" % (res["engine"], res["violations"], res["validate_us"]))
    if want_json:
        sys.stderr.write(json.dumps(res) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
