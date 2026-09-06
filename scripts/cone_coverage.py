#!/usr/bin/env python3
# [SONNET-4.6] Changed-cone coverage selector (sq-6vshe.8).
#
# Reuses ci_select.py's diff classification and reverse-dep closure computation
# (via shared import, NOT a diverging copy) to determine which crates need fresh
# instrumented-coverage measurement vs which can inherit their floor result from
# main's last full run (because their code + their transitive deps are unchanged).
#
# WHAT THIS IS (and is NOT):
#   Given the set of paths changed in a PR (same input as ci_select.py), compute
#   the COVERAGE CONE — the set of crates that MUST be re-measured. Crates outside
#   the cone are unchanged: if they passed the floor gate on main, they still pass
#   (SAME soundness argument ci_select.py commits to for test skipping, §2).
#   Emits JSON: {"mode": "cone"|"full", "cone_crates": [...], "reason": "..."}.
#
#   FAIL-SAFE IDENTICAL TO ci_select.py (shared import, not a diverging copy):
#   if any §4.1 trigger (Cargo.lock, root Cargo.toml, .github/, scripts/, etc.)
#   is changed → mode=full → ALL crates measured. Any internal error → mode=full.
#
# TWO MODES:
#   SHADOW  (--enforce OFF, the sq-6vshe.8 landing state): does NOT change what
#           coverage.sh measures; only logs the cone-vs-full comparison so the cone
#           can be validated against a real full measurement.
#   ENFORCE (--enforce ON, bead sq-3dr4t): additionally writes --crates-output — the
#           space-separated cone crate list that CI hands to coverage.sh as
#           COVERAGE_CONE, which INTERSECTS the shard's crate set with it. Crates
#           outside the cone are then NOT measured; they inherit their floor verdict
#           from main's last full run.
#
#   FAIL-SAFE UNDER ENFORCE: --crates-output is written NON-EMPTY only when
#   mode=cone. Every full-run trigger and every internal error yields mode=full and
#   therefore an EMPTY crates file, and an empty COVERAGE_CONE applies NO filter —
#   so each degradation path measures the FULL shard exactly as before the flip.
#   Divergence detection is a SHADOW-ONLY capability: under enforce the outside-cone
#   crates are never measured, so there is nothing to compare (the report says so
#   rather than reporting "no divergences").
#
# WHERE THE CONE COMES FROM IN CI (sq-3dr4t): NOT an in-job diff. The coverage job
#   checks out at the default (shallow) depth, so `git diff base...head` cannot resolve
#   the base there and would fail-safe to mode=full forever. Instead CI passes the
#   `select` job's already-computed closure (`--select-mode` / `--select-affected`) —
#   the SAME ci_select.select() verdict, computed once on the full-history checkout.
#   See cone_from_selection(). The diff path below remains for local/manual use.
#
# USAGE:
#   cone_coverage.py --mode compute-cone --enforce \
#                    --select-mode "$SELECT_MODE" --select-affected "$SELECT_AFFECTED" \
#                    --crates-output cone-crates.txt --output cone.json   # <- the CI path
#   cone_coverage.py --mode compute-cone [--output cone.json] [--enforce]
#                    [--crates-output cone-crates.txt]
#   cone_coverage.py --mode report --cone cone.json
#                    --coverage-summary coverage-summary.json
#                    [--divergence-log divergence.json]
#                    [--floor bench/coverage-floor.json]
#   (`--mode shadow-report` is the historical alias for `--mode report`.)
#   cone_coverage.py --changed-file paths.txt --metadata-file meta.json ...
#
# STDLIB ONLY: no third-party deps (mirrors ci_select.py §7 P1).

from __future__ import annotations

import argparse
import importlib.util
import json
import os
import sys
from pathlib import Path

# --- shared import from ci_select.py -----------------------------------------
# [SONNET-4.6] Import the diff classifier and closure computation DIRECTLY from
# ci_select.py (NOT a copy) so the fail-safe rules stay in sync automatically.
# Any future change to §4.1 triggers or the closure algorithm applies here too.

def _load_ci_select():
    here = Path(__file__).resolve().parent
    ci_select_path = here / "ci_select.py"
    spec = importlib.util.spec_from_file_location("ci_select", ci_select_path)
    if spec is None or spec.loader is None:
        raise ImportError(f"Cannot load ci_select.py from {ci_select_path}")
    mod = importlib.util.module_from_spec(spec)
    sys.modules["ci_select"] = mod  # [SONNET-4.6] assign unconditionally to avoid stale-entry cross-test leakage
    spec.loader.exec_module(mod)  # type: ignore[union-attr]
    return mod


_ci_select = _load_ci_select()

# Re-export the pieces we use (makes static analysis + tests easier).
select = _ci_select.select
parse_workspace = _ci_select.parse_workspace
load_metadata = _ci_select.load_metadata
load_ownership_map = _ci_select.load_ownership_map
git_changed_paths = _ci_select.git_changed_paths
SelectorError = _ci_select.SelectorError


# --- cone computation ---------------------------------------------------------

def compute_cone(
    changed_paths: list[str],
    meta: dict,
    map_entries: list[dict] | None = None,
    enforce: bool = False,
) -> dict:
    """Compute the coverage cone from the set of changed paths.

    [SONNET-4.6] Delegates entirely to ci_select.select() — the SAME diff
    classifier and reverse-dep closure used by change-based test selection. The
    cone is exactly the `affected` set: the changed crates PLUS their transitive
    reverse-dep closure (dependents whose tests might exercise the changed code).

    Returns a plain dict ready to serialise as JSON:
      {
        "mode": "cone" | "full",
        "cone_crates": [...],   # sorted; all workspace members when mode=full
        "changed_crates": [...],# directly-changed workspace members
        "all_members": [...],   # sorted full workspace member list
        "reason": "...",
        "enforce": bool,        # mirrors the `enforce` argument
        "shadow": bool,         # not enforce
      }
    """
    map_entries = map_entries or []
    try:
        sel = select(changed_paths, meta, map_entries)
    except Exception as exc:  # noqa: BLE001 — mirrors ci_select.py §4.3
        # Any internal error → full run (fail-closed). Use parse_workspace to get
        # the full member list so cone_crates is correct even on error.
        try:
            ws = parse_workspace(meta)
            all_members = sorted(ws.members)
        except Exception:  # noqa: BLE001
            all_members = []
        return {
            "mode": "full",
            "cone_crates": all_members,
            "changed_crates": [],
            "all_members": all_members,
            "reason": f"cone-coverage selector error, failing to full run: {exc}",
            "enforce": enforce,
            "shadow": not enforce,
        }

    if sel.mode == "full":
        mode = "full"
        cone_crates = sel.affected  # already == all members for a full run
    else:
        # mode == "selected": the cone is exactly the affected closure.
        mode = "cone"
        cone_crates = sel.affected

    return {
        "mode": mode,
        "cone_crates": sorted(cone_crates),
        "changed_crates": sorted(getattr(sel, "changed_crates", [])),
        "all_members": sorted(getattr(sel, "all_members", [])),
        "reason": sel.reason,
        "enforce": enforce,
        # [SONNET-4.6] sq-3dr4t: shadow is the COMPLEMENT of enforce. It stays True for
        # the default (enforce=False) call so a caller that only reads `shadow` keeps its
        # pre-flip meaning; CI passes enforce=True and reads `enforce`.
        "shadow": not enforce,
    }


def cone_from_selection(
    select_mode: str | None,
    affected: str | list | None,
    meta: dict | None = None,
    enforce: bool = False,
) -> dict:
    """[SONNET-4.6] sq-3dr4t: build the cone doc from the `select` JOB's outputs.

    WHY THIS EXISTS (and is the path CI uses). compute_cone() needs the PR diff, which
    needs `git diff base...head`, which needs the base commit to be present locally. The
    coverage job checks out at the `actions/checkout` DEFAULT depth (shallow) and does NOT
    set `fetch-depth: 0` (ci-select.yml does, explicitly for this reason), so an in-job
    diff cannot resolve the base and fail-safes to mode=full. Recomputing the closure there
    would therefore never narrow anything. The `select` job ALREADY computed it, on a
    full-history checkout,
    with the SAME ci_select.select() call, and publishes it as `mode` + `affected`. So the
    cone is that output, consumed rather than recomputed: one closure, one fail-safe, no
    second full clone × 3 shards.

    `affected` is `sel.affected` — the changed crates plus their transitive reverse-dep
    closure — i.e. exactly what compute_cone() returns as `cone_crates` for mode=selected.

    FAIL-SAFE: narrowing happens ONLY for select_mode == "selected". Anything else
    ("full", the "shadow" report-only rollback, an empty/absent value, a malformed
    affected list, an empty closure) yields mode=full, so every degradation measures
    everything. `meta` is OPTIONAL and cosmetic (it only fills `all_members` for the
    report); its absence must never widen or narrow the cone.
    """
    all_members: list[str] = []
    if meta is not None:
        try:
            all_members = sorted(parse_workspace(meta).members)
        except Exception:  # noqa: BLE001 — cosmetic only; see the docstring
            all_members = []

    def _full(reason: str) -> dict:
        return {
            "mode": "full",
            "cone_crates": all_members,
            "changed_crates": [],
            "all_members": all_members,
            "reason": reason,
            "source": "ci-select job outputs",
            "enforce": enforce,
            "shadow": not enforce,
        }

    if select_mode != "selected":
        return _full(
            f"cone: select job mode={select_mode!r} is not 'selected' — full run "
            f"(only an explicit 'selected' verdict may narrow measurement)"
        )

    if isinstance(affected, str):
        try:
            affected = json.loads(affected)
        except Exception as exc:  # noqa: BLE001
            return _full(f"cone: could not parse the select job's affected list, failing to full: {exc}")
    if not isinstance(affected, list) or not all(isinstance(x, str) for x in affected):
        return _full("cone: the select job's affected list is not a list of crate names, failing to full")
    if not affected:
        # An empty closure means the select job proved NOTHING is affected — but the
        # coverage job's own `if:` already skips that case, so reaching here means the
        # output did not arrive as expected. Fail to full rather than measure nothing.
        return _full("cone: the select job's affected list is EMPTY, failing to full")

    cone_crates = sorted(set(affected))
    return {
        "mode": "cone",
        "cone_crates": cone_crates,
        "changed_crates": [],   # not published by the select job; the closure is what matters
        "all_members": all_members,
        "reason": f"cone: {len(cone_crates)} crate(s) in the select job's affected closure",
        "source": "ci-select job outputs",
        "enforce": enforce,
        "shadow": not enforce,
    }


# --- cone report (shadow AND enforce) ----------------------------------------
# NB the function name `shadow_report` is HISTORICAL (sq-6vshe.8, when shadow was the
# only mode). It renders both modes; see its docstring for how they differ.

def shadow_report(
    cone_doc: dict,
    coverage_summary: dict,
    floor_doc: dict | None,
    divergence_log_path: str | None = None,
) -> dict:
    """[SONNET-4.6] Compare the cone against what the coverage shard actually measured.

    For each workspace crate:
      - IN cone:     reports label "cone-measured" + actual line_pct from summary
      - OUTSIDE cone: reports label "inherited (unchanged cone): PASS" — we assert
                      the crate is unchanged so its main-run floor result still holds

    SHADOW vs ENFORCE (sq-3dr4t). In SHADOW mode the shard still measured every crate,
    so an outside-cone crate that came in BELOW its floor is a real DIVERGENCE (the cone
    would have skipped a regression) and is logged to `divergence_log_path`. Under
    ENFORCE the outside-cone crates were never measured, so divergence detection is
    structurally UNAVAILABLE here — the report says so instead of claiming "no
    divergences", and the inherited crates come from the summary's `cone.inherited`
    block (written by coverage.sh) so the skip is auditable rather than silent.

    Returns a report dict. Never raises (logs errors; caller continues regardless).
    """
    cone_crates = set(cone_doc.get("cone_crates", []))
    all_members = set(cone_doc.get("all_members", []))
    mode = cone_doc.get("mode", "full")
    enforce = bool(cone_doc.get("enforce", False))
    # [SONNET-4.6] sq-3dr4t: crates coverage.sh SKIPPED because COVERAGE_CONE excluded
    # them. Present only when the filter actually applied (enforce + mode=cone).
    cone_block = coverage_summary.get("cone") or {}
    inherited_declared = sorted(cone_block.get("inherited") or [])

    floors: dict[str, float] = {}
    if floor_doc:
        for crate, fentry in floor_doc.get("crates", {}).items():
            if isinstance(fentry, dict):
                floors[crate] = float(fentry.get("floor", 0))
            elif isinstance(fentry, (int, float)):
                floors[crate] = float(fentry)

    summary_crates = coverage_summary.get("crates", {})

    # [SONNET-4.6] Operate on the INTERSECTION of workspace members and crates
    # actually present in this shard's coverage-summary.json. In the CI matrix
    # each shard measures only its own subset; iterating all_members would include
    # crates with no measured data, making divergence detection meaningless and
    # inflating counts. Crates absent from the shard summary are omitted from rows;
    # a single compact count is emitted instead (not_measured_in_shard).
    #
    # [SONNET-4.6] Empty-all_members fallback: when cone.json was produced via the
    # error path, all_members may be empty (compute_cone() catches selector errors
    # and emits all_members=[] as a fail-safe). In that case the intersection would
    # be empty and the report would render no rows even though the shard summary has
    # measured crates — silent monitoring blindness. Fall back to the shard summary
    # keys as the effective member universe so divergences still surface.
    measured_in_shard = set(summary_crates.keys())
    effective_members = all_members if all_members else measured_in_shard
    reported_members = sorted(effective_members & measured_in_shard)
    # [SONNET-4.6] sq-3dr4t: the enforce-skipped crates get their OWN rows below, so
    # they must not also be folded into the compact "not measured in this shard" count
    # (that count means "measured by a different shard / not applicable here").
    not_measured_count = len(
        effective_members - measured_in_shard - set(inherited_declared)
    )

    rows: list[dict] = []
    divergences: list[dict] = []

    for crate in reported_members:
        row_summary = summary_crates.get(crate, {})
        measured = row_summary.get("measured", False)
        lines_pct = row_summary.get("lines_pct") if measured else None
        floor = floors.get(crate)

        in_cone = (mode == "full") or (crate in cone_crates)

        if in_cone:
            label = "cone-measured"
            status = "measured"
        else:
            label = "inherited (unchanged cone): PASS"
            status = "inherited"

        row: dict = {
            "crate": crate,
            "label": label,
            "status": status,
            "in_cone": in_cone,
        }
        if lines_pct is not None:
            row["lines_pct"] = lines_pct
        if floor is not None:
            row["floor"] = floor

        rows.append(row)

        # Divergence: a crate OUTSIDE the cone that is below its floor in the
        # full measurement would be a soundness problem (cone missed a regression).
        # Should never happen if the cone is correct.
        if not in_cone and lines_pct is not None and floor is not None:
            if lines_pct + 1e-9 < floor:
                divergences.append({
                    "crate": crate,
                    "lines_pct": lines_pct,
                    "floor": floor,
                    "type": "outside-cone-below-floor",
                    "explanation": (
                        "A crate outside the coverage cone is below its floor. "
                        "This should be impossible if the cone is correct — "
                        "an unchanged crate cannot regress. Investigate."
                    ),
                })

    # [SONNET-4.6] sq-3dr4t: rows for the crates the ENFORCED cone filter skipped. They
    # have no lines_pct (never measured this run) — their floor verdict is inherited from
    # main's last full measurement. Emitting them explicitly is what keeps the skip
    # auditable ("no silent truncation") instead of an unexplained gap in the table.
    for crate in inherited_declared:
        if crate in measured_in_shard:
            continue  # already has a measured row above; do not double-count
        row = {
            "crate": crate,
            "label": "inherited (unchanged cone): PASS",
            "status": "inherited",
            "in_cone": False,
        }
        if crate in floors:
            row["floor"] = floors[crate]
        rows.append(row)
    rows.sort(key=lambda r: r["crate"])

    # [SONNET-4.6] All derived counts are over the REPORTED set (intersection)
    # so that counts always match the row set. The workspace-level cone size is
    # also retained under a distinct key for context.
    reported_cone_set = (
        cone_crates & measured_in_shard if mode != "full"
        else set(reported_members)
    )
    report = {
        "mode": mode,
        "enforce": enforce,
        "shadow": not enforce,
        "cone_crates": sorted(cone_crates),
        "total_crates": len(reported_members),           # crates in this shard's summary
        "cone_size": len(reported_cone_set),             # cone crates measured in this shard
        "cone_size_workspace": (                         # workspace-level cone size for context
            len(cone_crates) if mode != "full" else len(all_members)
        ),
        "not_measured_in_shard": not_measured_count,     # compact count; no per-crate row emitted
        "inherited_count": sum(1 for r in rows if r["status"] == "inherited"),
        # [SONNET-4.6] sq-3dr4t: crates the ENFORCED filter skipped (from the summary's
        # `cone.inherited` block). Under shadow mode this is always 0.
        "inherited_declared": inherited_declared,
        # Divergence detection needs a FULL measurement to compare the cone against, which
        # only shadow mode produces. Stated explicitly so an empty `divergences` under
        # enforce is never read as evidence the cone is sound.
        "divergence_detection": (
            "unavailable (enforce: outside-cone crates were not measured)"
            if enforce and mode == "cone"
            else "shadow (full measurement compared against the cone)"
        ),
        "divergences": divergences,
        # rows == the shard's measured crates PLUS any enforce-inherited crates, so
        # len(rows) == total_crates only when nothing was skipped.
        "rows": rows,
    }

    if divergence_log_path:
        try:
            with open(divergence_log_path, "w", encoding="utf-8") as fh:
                json.dump(
                    {"divergences": divergences, "mode": mode, "cone_crates": sorted(cone_crates)},
                    fh, indent=2,
                )
                fh.write("\n")
        except OSError as exc:
            print(f"  WARN: could not write divergence log: {exc}", file=sys.stderr)

    return report


def _render_report(report: dict) -> str:
    """Render the cone report as a Markdown step summary."""
    enforce = bool(report.get("enforce", False))
    lines = [
        "### Coverage cone report (sq-6vshe.8 / sq-3dr4t) [SONNET-4.6]",
        "",
        f"**Mode:** `{report['mode']}` — "
        + (
            "ENFORCED (outside-cone crates were NOT measured; they inherit "
            "their floor verdict from main's last full run)"
            if enforce
            else "shadow (enforce=False; full measurement unchanged, cone only observed)"
        ),
        "",
    ]
    mode = report.get("mode", "full")
    not_in_shard = report.get("not_measured_in_shard", 0)
    if mode == "cone":
        total = report.get("total_crates", 0)
        cone_size = report.get("cone_size", 0)
        inherited = report.get("inherited_count", 0)
        lines += [
            f"**Cone size:** {cone_size} of {total} shard-measured crates measured fresh "
            f"({inherited} inherited from main — unchanged, no re-measurement needed)",
            "",
        ]
        skipped = report.get("inherited_declared") or []
        if skipped:
            lines += [
                f"**Skipped by the enforced cone ({len(skipped)}):** "
                + ", ".join(f"`{c}`" for c in skipped),
                "",
            ]
    else:
        lines += [
            "**Full run:** all crates measured (diff triggered full-run fail-safe).",
            "",
        ]
    if not_in_shard:
        lines += [
            f"_not-measured-in-this-shard: {not_in_shard} workspace crate(s) "
            f"(measured in other shards or not applicable here — omitted from rows)_",
            "",
        ]

    divs = report.get("divergences", [])
    if divs:
        lines += [
            f"**DIVERGENCES ({len(divs)}) — INVESTIGATE:**",
            "",
            "| Crate | lines_pct | floor | type |",
            "|---|---|---|---|",
        ]
        for d in divs:
            lines.append(
                f"| `{d['crate']}` | {d.get('lines_pct', '?')} | {d.get('floor', '?')} | {d['type']} |"
            )
        lines.append("")
    elif enforce and mode == "cone":
        # HONESTY: with the cone enforced there is no full measurement to diverge FROM,
        # so an empty divergence list is NOT evidence the cone is sound. Say that.
        lines += [
            "_Divergence detection unavailable in enforce mode: the outside-cone crates "
            "were not measured, so there is no full run to compare against. The nightly "
            "full-coverage run on `main` is the drift backstop._",
            "",
        ]
    else:
        lines += ["No divergences — cone is sound for this PR.", ""]

    lines += [
        "| Crate | Status | lines_pct | floor |",
        "|---|---|---|---|",
    ]
    for row in report.get("rows", []):
        pct = f"{row['lines_pct']:.2f}%" if row.get("lines_pct") is not None else "—"
        fl = str(row["floor"]) if row.get("floor") is not None else "—"
        lines.append(f"| `{row['crate']}` | {row['label']} | {pct} | {fl} |")
    lines.append("")
    return "\n".join(lines)


# --- input gathering (mirrors ci_select.py main) ----------------------------

def _resolve_repo_root(explicit: str | None) -> str | None:
    if explicit:
        return explicit
    import subprocess
    try:
        out = subprocess.run(
            ["git", "rev-parse", "--show-toplevel"],
            capture_output=True, text=True, check=True, timeout=30,
        )
        return out.stdout.strip()
    except Exception:  # noqa: BLE001
        return None


def _get_changed_paths(args: argparse.Namespace, repo_root: str | None) -> list[str]:
    """Get changed paths from hermetic file or live git diff."""
    if args.changed_file:
        with open(args.changed_file, encoding="utf-8") as fh:
            # [SONNET-4.6] Strip leading/trailing whitespace from each line so
            # behavior is consistent regardless of how the caller wrote the file
            # (trailing \r on Windows-style line endings, leading spaces, etc.).
            return [ln.strip() for ln in fh.read().splitlines() if ln.strip()]
    base = getattr(args, "base", None)
    head = getattr(args, "head", "HEAD")
    if base:
        return git_changed_paths(base, head, repo_root)
    # No base → can't compute diff → fail-safe to full.
    raise SelectorError("--base is required for a diff-based cone computation")


# --- CLI main ----------------------------------------------------------------

def _parse_args(argv: list[str] | None) -> argparse.Namespace:
    p = argparse.ArgumentParser(
        description="Coverage-cone selector (sq-6vshe.8) [SONNET-4.6]."
    )
    p.add_argument(
        "--mode",
        choices=["compute-cone", "report", "shadow-report"],
        required=True,
        help=(
            "compute-cone: diff → cone JSON; "
            "report: compare cone JSON vs the coverage summary "
            "(`shadow-report` is the historical alias)"
        ),
    )
    # compute-cone inputs — the ci-select job's outputs are the PREFERRED source (see
    # cone_from_selection); --base/--head drive the local/manual diff path instead.
    p.add_argument(
        "--select-mode",
        default=None,
        help="the ci-select job's `mode` output. When given, the cone is taken FROM that "
             "job (only 'selected' narrows; anything else => full). Preferred in CI: the "
             "coverage job's shallow checkout cannot resolve a diff base.",
    )
    p.add_argument(
        "--select-affected",
        default=None,
        help="the ci-select job's `affected` output (a JSON array of crate names).",
    )
    p.add_argument("--base", help="base SHA/ref for the three-dot diff")
    p.add_argument("--head", default="HEAD", help="head SHA/ref (default HEAD)")
    p.add_argument("--changed-file", help="hermetic: newline-delimited changed paths")
    p.add_argument("--metadata-file", help="hermetic: cargo metadata JSON snapshot")
    p.add_argument("--map", dest="map_file", help="ownership map toml (default ci/path-ownership.toml)")
    p.add_argument("--repo-root", help="repo root (default: git toplevel)")
    p.add_argument("--output", "-o", default=None, help="write cone JSON here (default: stdout)")
    p.add_argument(
        "--crates-output",
        default=None,
        help="write the space-separated cone crate list here, for coverage.sh's "
             "COVERAGE_CONE. Written NON-EMPTY only with --enforce AND mode=cone; "
             "an empty file means 'no filter' (measure everything) — the fail-safe.",
    )
    p.add_argument(
        "--enforce",
        action="store_true",
        help="enforce the cone (sq-3dr4t): crates outside it are NOT measured and "
             "inherit their floor verdict from main. Default OFF (shadow mode).",
    )
    # shadow-report inputs
    p.add_argument("--cone", help="cone JSON from a previous compute-cone run")
    p.add_argument("--coverage-summary", help="full coverage-summary.json from coverage.sh")
    p.add_argument("--floor", help="coverage floor JSON (default bench/coverage-floor.json)")
    p.add_argument("--divergence-log", help="write divergences JSON here")
    p.add_argument("--summary-file", help="write Markdown step-summary (default $GITHUB_STEP_SUMMARY)")
    return p.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = _parse_args(argv)
    summary_file = args.summary_file or os.environ.get("GITHUB_STEP_SUMMARY")

    if args.mode == "compute-cone":
        repo_root = _resolve_repo_root(args.repo_root)

        # [SONNET-4.6] sq-3dr4t: establish the FAIL-SAFE crates file up front — an empty
        # file means "no cone filter", so if this process dies anywhere below (or the
        # caller ignores our exit code) coverage.sh measures the full shard as before the
        # enforce flip. Every path from here only ever WIDENS it to the real cone.
        _write_crates_output([], args.crates_output, enforce=False, mode="full")

        # [SONNET-4.6] sq-3dr4t: PREFERRED INPUT — the ci-select job's already-computed
        # closure. Taken whenever --select-mode is passed AT ALL (an empty value is not
        # 'selected', so it fail-safes to full). Here cargo metadata is best-effort: it
        # only fills the cosmetic `all_members`, so a metadata failure must not change
        # what is measured. See cone_from_selection() for why CI cannot diff in-job.
        if args.select_mode is not None:
            try:
                sel_meta = load_metadata(args.metadata_file, repo_root)
            except Exception:  # noqa: BLE001
                sel_meta = None
            doc = cone_from_selection(
                args.select_mode, args.select_affected, sel_meta, enforce=args.enforce,
            )
            doc["base"] = args.base or None
            doc["head"] = args.head or None
            _write_output(doc, args.output, summary_file, enforce=args.enforce)
            _write_crates_output(
                doc.get("cone_crates", []), args.crates_output,
                enforce=args.enforce, mode=doc.get("mode", "full"),
            )
            return 0

        # Load cargo metadata (fail-closed to full on error).
        try:
            meta = load_metadata(args.metadata_file, repo_root)
        except Exception as exc:  # noqa: BLE001
            doc = {
                "mode": "full",
                "cone_crates": [],
                "changed_crates": [],
                "all_members": [],
                "reason": f"cone: could not load metadata, failing to full: {exc}",
                "enforce": args.enforce,
                "shadow": not args.enforce,
            }
            _write_output(doc, args.output, summary_file, enforce=args.enforce)
            return 0

        # Load ownership map (absent → []).
        map_file = args.map_file
        if map_file is None and repo_root is not None:
            candidate = os.path.join(repo_root, "ci", "path-ownership.toml")
            map_file = candidate if os.path.exists(candidate) else None
        try:
            map_entries = load_ownership_map(map_file)
        except Exception:  # noqa: BLE001
            map_entries = []

        # Get changed paths (fail-closed to full on error).
        try:
            changed_paths = _get_changed_paths(args, repo_root)
        except Exception as exc:  # noqa: BLE001
            # Cannot compute diff → full run (fail-safe).
            try:
                ws = parse_workspace(meta)
                all_members = sorted(ws.members)
            except Exception:  # noqa: BLE001
                all_members = []
            doc = {
                "mode": "full",
                "cone_crates": all_members,
                "changed_crates": [],
                "all_members": all_members,
                "reason": f"cone: could not get changed paths, failing to full: {exc}",
                "enforce": args.enforce,
                "shadow": not args.enforce,
            }
            _write_output(doc, args.output, summary_file, enforce=args.enforce)
            return 0

        doc = compute_cone(changed_paths, meta, map_entries, enforce=args.enforce)
        # [SONNET-4.6] sq-3dr4t: record the diff endpoints so the report can name WHICH
        # base the inherited floor verdicts came from (the auditability obligation in
        # research/engine-performance-review.md §3.1).
        doc["base"] = args.base
        doc["head"] = args.head

        _write_output(doc, args.output, summary_file, enforce=args.enforce)
        _write_crates_output(
            doc.get("cone_crates", []), args.crates_output,
            enforce=args.enforce, mode=doc.get("mode", "full"),
        )
        return 0

    elif args.mode in ("report", "shadow-report"):
        if not args.cone:
            print("::error::--cone is required for report mode", file=sys.stderr)
            return 1
        if not args.coverage_summary:
            print("::error::--coverage-summary is required for report mode", file=sys.stderr)
            return 1

        with open(args.cone, encoding="utf-8") as fh:
            cone_doc = json.load(fh)

        with open(args.coverage_summary, encoding="utf-8") as fh:
            coverage_summary = json.load(fh)

        # Load floor file (optional; skipped crates → no floor comparison).
        floor_doc = None
        floor_path = args.floor
        if floor_path is None:
            # Default: bench/coverage-floor.json relative to repo root.
            repo_root = _resolve_repo_root(args.repo_root)
            if repo_root:
                candidate = os.path.join(repo_root, "bench", "coverage-floor.json")
                floor_path = candidate if os.path.exists(candidate) else None
        if floor_path and os.path.exists(floor_path):
            with open(floor_path, encoding="utf-8") as fh:
                floor_doc = json.load(fh)

        report = shadow_report(
            cone_doc, coverage_summary, floor_doc,
            divergence_log_path=args.divergence_log,
        )

        # Print report JSON.
        text = json.dumps(report, indent=2) + "\n"
        print(text)

        # Write step summary.
        if summary_file:
            try:
                with open(summary_file, "a", encoding="utf-8") as fh:
                    fh.write(_render_report(report))
            except OSError:
                pass

        # The report NEVER fails the shard: the floor gate (coverage-gate.py) is the
        # only thing that decides pass/fail. This step reports.
        divs = report.get("divergences", [])
        if divs:
            print(
                f"::warning::cone-coverage: {len(divs)} divergence(s) found "
                f"(outside-cone crates below floor). See the step summary.",
                file=sys.stderr,
            )
        return 0

    return 0  # unreachable


def _write_output(doc: dict, output_path: str | None, summary_file: str | None, enforce: bool) -> None:
    text = json.dumps(doc, indent=2) + "\n"
    if output_path:
        with open(output_path, "w", encoding="utf-8") as fh:
            fh.write(text)
        print(f"cone_coverage: wrote cone JSON to {output_path}")
        _print_cone_summary(doc, enforce=enforce)
    else:
        sys.stdout.write(text)

    if summary_file:
        try:
            with open(summary_file, "a", encoding="utf-8") as fh:
                fh.write(_render_compute_summary(doc, enforce=enforce))
        except OSError:
            pass


def _write_crates_output(
    cone_crates: list[str], path: str | None, enforce: bool, mode: str
) -> None:
    """[SONNET-4.6] sq-3dr4t: write the COVERAGE_CONE crate list for coverage.sh.

    The file is the ENFORCEMENT DECISION, single-sourced here:
      * enforce AND mode == "cone" -> the space-separated cone crate list.
      * anything else (shadow mode, mode=full, any fail-safe path) -> EMPTY, which
        coverage.sh reads as "no filter" and measures its whole shard, i.e. exactly
        the pre-flip behaviour. So no error path can narrow what gets measured.
    """
    if path is None:
        return
    payload = " ".join(cone_crates) if (enforce and mode == "cone") else ""
    try:
        with open(path, "w", encoding="utf-8") as fh:
            fh.write(payload + "\n")
    except OSError as exc:
        # Cannot write => the caller's pre-created empty file (or absence) stands, and
        # an absent/empty COVERAGE_CONE measures everything. Loud but not fatal.
        print(f"::warning::cone_coverage: could not write {path}: {exc}", file=sys.stderr)
        return
    if payload:
        print(f"  cone_coverage: COVERAGE_CONE -> {path} ({len(cone_crates)} crate(s))")
    else:
        print(f"  cone_coverage: {path} left EMPTY -> no cone filter (measure everything)")


def _print_cone_summary(doc: dict, enforce: bool) -> None:
    mode = doc.get("mode", "full")
    cone = doc.get("cone_crates", [])
    all_m = doc.get("all_members", [])
    reason = doc.get("reason", "")
    shadow = not enforce

    if mode == "full":
        print(f"  cone_coverage: mode=full (all {len(all_m)} crates) — {reason}")
    else:
        inherited = max(0, len(all_m) - len(cone))
        print(
            f"  cone_coverage: mode=cone — {len(cone)} of {len(all_m)} crates in cone "
            f"({inherited} inherited from main) — {reason}"
        )
    if shadow:
        print("  [SHADOW MODE] enforce=False: full coverage.sh run unchanged; "
              "cone is computed for monitoring only.")
    elif mode == "cone":
        print("  [ENFORCE MODE] crates outside the cone are NOT measured; their floor "
              "verdict is inherited from main's last full run.")
    else:
        print("  [ENFORCE MODE] mode=full — the fail-safe applies, so everything is "
              "measured (no crate inherits).")


def _render_compute_summary(doc: dict, enforce: bool) -> str:
    mode = doc.get("mode", "full")
    cone = doc.get("cone_crates", [])
    all_m = doc.get("all_members", [])
    changed = doc.get("changed_crates", [])
    reason = doc.get("reason", "")

    base = doc.get("base")

    lines = [
        "### Coverage cone (sq-6vshe.8 / sq-3dr4t) [SONNET-4.6]",
        "",
        f"**Mode:** `{mode}` — {reason}",
        "",
    ]
    if not enforce:
        lines += [
            "**SHADOW MODE**: enforce=False — full coverage.sh run unchanged; "
            "cone is computed for monitoring only.",
            "",
        ]
    elif mode == "cone":
        lines += [
            "**ENFORCE MODE**: crates outside the cone are NOT measured this run — they "
            "inherit their floor verdict from `main`"
            + (f" at base `{base}`" if base else "")
            + ". The nightly full-coverage run on `main` is the drift backstop.",
            "",
        ]
    else:
        lines += [
            "**ENFORCE MODE, but the full-run fail-safe applies** — every crate is "
            "measured, nothing inherits.",
            "",
        ]
    if changed:
        lines += [
            f"**Directly-changed crates ({len(changed)}):** " + ", ".join(f"`{c}`" for c in changed),
            "",
        ]
    if mode == "cone":
        inherited = max(0, len(all_m) - len(cone))
        lines += [
            f"**Coverage cone ({len(cone)} of {len(all_m)} crates):** "
            + (", ".join(f"`{c}`" for c in cone) or "_empty_"),
            "",
            f"**{'Inherits' if enforce else 'Would inherit'} from main "
            f"({inherited} crates — unchanged cone):** "
            + (", ".join(f"`{c}`" for c in sorted(set(all_m) - set(cone))) or "_none_"),
            "",
        ]
    else:
        lines += [
            "**Full run:** all crates measured (diff triggered full-run fail-safe).",
            "",
        ]
    return "\n".join(lines) + "\n"


if __name__ == "__main__":
    sys.exit(main())
