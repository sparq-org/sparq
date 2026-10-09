#!/usr/bin/env python3
# Model-free read-payload A/B measurer for the ast-grep+outline
# intervention (bead sq-lhwo.4, epic sq-lhwo). 🤖 SPARQ agent.
#
# WHAT IT MEASURES — and the honest scope
# --------------------------------------------------------------------------------
# For each frozen task it EXECUTES both arms as pure shell tools over the REAL repo
# code and measures the READ-PAYLOAD each arm forces into an agent's context:
#
#   * Arm A (baseline)  — the cheapest plausible COMPETENT status quo:
#       - "list" task   : `grep -rn <regex> <paths>`  (a competent grep, NOT a
#                          strawman "open every file").
#       - "locate/shape": a FULL `Read` of the target file (offset/limit unknown
#                          until you've located it — so the honest baseline reads
#                          the whole file to answer a "where/what is X" question).
#   * Arm B (treatment) — ast-grep + the outline-before-read discipline:
#       - "list" task   : the ast-grep YAML-rule / pattern hit list.
#       - "locate/shape": the ast-grep SIGNATURE outline (answer-sized), NOT the
#                          file body.
#
# The payload bytes are converted to a token estimate by a DOCUMENTED char/token
# proxy and run through the canonical cache-discount formula
# (metrics_row.effective_input_tokens). Both arms are charged at the SAME proxy
# rate, so the proxy is neutral on the paired DELTA (the statistic of interest).
#
# WHY THIS IS A PROXY, NOT THE §5 GOLD STANDARD (load-bearing — read this)
# --------------------------------------------------------------------------------
# The §5.1 gold standard runs EACH ARM AS A SEPARATE Claude Code session, pins model
# completions via record/replay, and DIFFS two agent_telemetry.py transcript reports
# — capturing the per-turn cached-prefix dynamics and the model round-trip / repair
# tokens. That needs ≥30×2 isolated sessions and an ANTHROPIC_API_KEY for the only
# exact Claude tokenizer (count_tokens). NEITHER is available to a single sub-agent
# in this work-box. So this tool measures the DOMINANT, model-free driver of the
# delta — the input bytes each arm forces into context — and is honest that it is a
# LOWER-FIDELITY proxy: it EXCLUDES output/repair tokens and per-turn cache dynamics.
# Its rows feed ab_stats.py exactly like the full-session rows would; the verdict
# object it produces is therefore PROVISIONAL and is marked
# `measurement="read-payload-proxy"`. The full-session A/B is the follow-up (a bead).
#
# The proxy is CONSERVATIVE on the contested arm: for a flat "list every X" task it
# charges ast-grep its full hit-list payload at the same rate as grep — so the pilot
# finding (ast-grep ≈ byte-neutral vs competent grep on flat lists) is reproduced,
# not assumed. The large win comes ONLY from the outline-vs-full-Read lever, and only
# on the locate/shape stratum — which is exactly the skill's honest claim.
#
# Every number is work-box / runtime and NON-CANONICAL (project-ec2-execution-env):
# never frozen into committed markdown. Committed artifacts = this CODE + the frozen
# tasks + the corpus manifest + the verdict schema.
#
# Stdlib-only; shells out to grep / ast-grep / jq (verified present by --check).

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Optional

HERE = Path(__file__).resolve().parent
# repo root = .../scripts/agent-telemetry/ast-grep-outline -> up 3
REPO_ROOT = HERE.parents[2]

# DOCUMENTED char/token proxy. ~3.5 chars/token is a standard, conservative English+
# code estimate; the EXACT Claude tokenizer (count_tokens) needs an API key not
# available here. Because BOTH arms use the same rate, the paired delta is invariant
# to the exact constant — only the absolute magnitudes shift. Stated, not hidden.
CHARS_PER_TOKEN = 3.5

# Cache-discount multipliers (§5.1) — mirror metrics_row.py / agent_telemetry.py.
EFF_FRESH = 1.0
EFF_CACHE_READ = 0.1
EFF_CACHE_WRITE = 1.25


def effective_input_tokens(fresh: float, cache_read: float, cache_write: float) -> float:
    return EFF_FRESH * fresh + EFF_CACHE_READ * cache_read + EFF_CACHE_WRITE * cache_write


def _bytes_to_tokens(n_bytes: int) -> float:
    return n_bytes / CHARS_PER_TOKEN


def _run(cmd: list[str], cwd: Optional[Path] = None) -> str:
    """Run a command, return stdout text (empty on non-zero — grep exits 1 on no
    match, which is a legitimate empty payload, not an error)."""
    res = subprocess.run(
        cmd, cwd=str(cwd or REPO_ROOT), capture_output=True, text=True
    )
    return res.stdout


def _ast_grep_bin() -> str:
    cand = shutil.which("ast-grep")
    if cand:
        return cand
    home = os.path.expanduser("~/.cargo/bin/ast-grep")
    if os.path.exists(home):
        return home
    raise RuntimeError(
        "ast-grep not found — install it (see .claude/skills/ast-grep/SKILL.md §0). "
        "NEVER invoke it as `sg` (that collides with newgrp on this box)."
    )


def _sha256(path: Path) -> str:
    h = hashlib.sha256()
    h.update(path.read_bytes())
    return h.hexdigest()


# --------------------------------------------------------------------------------
# Arm payloads.
# --------------------------------------------------------------------------------
def arm_a_payload(task: dict) -> str:
    """Cheapest plausible COMPETENT status-quo payload."""
    kind = task["kind"]
    if kind == "list":
        # competent `grep -rn` (not a strawman whole-tree read).
        out = _run(["grep", "-rn"] + task["armA"]["grep"])
        return out
    if kind in ("locate", "shape"):
        # honest baseline: full Read of the target file (you don't know the span yet).
        p = REPO_ROOT / task["armA"]["read_file"]
        return p.read_text(encoding="utf-8", errors="replace")
    raise ValueError(f"unknown task kind: {kind}")


def arm_b_payload(task: dict, ast_grep: str) -> str:
    """ast-grep + outline-before-read payload."""
    kind = task["kind"]
    if kind == "list":
        spec = task["armB"]
        if "rule" in spec:
            raw = _run(
                [ast_grep, "scan", "--rule", spec["rule"], spec.get("path", "crates/"),
                 "--json=compact"]
            )
        else:
            raw = _run(
                [ast_grep, "--lang", spec.get("lang", "rust"), "--pattern", spec["pattern"],
                 spec.get("path", "crates/"), "--json=compact"]
            )
        return _digest_hits(raw)
    if kind in ("locate", "shape"):
        spec = task["armB"]
        raw = _run(
            [ast_grep, "--lang", spec.get("lang", "rust"), "--pattern", spec["pattern"],
             str(REPO_ROOT / task["armB"]["outline_file"]), "--json=compact"]
        )
        return _digest_hits(raw)
    raise ValueError(f"unknown task kind: {kind}")


def _digest_hits(raw: str) -> str:
    """Reduce ast-grep --json=compact to a one-line-per-hit `file:line: signature`
    digest — the answer-sized view the agent actually reads (mirrors the SKILL
    recipes' `jq` step). Done in Python so we don't depend on jq being present."""
    raw = raw.strip()
    if not raw:
        return ""
    try:
        hits = json.loads(raw)
    except json.JSONDecodeError:
        return raw  # fall back to the raw payload rather than crashing
    lines = []
    for h in hits:
        f = h.get("file", "")
        start = h.get("range", {}).get("start", {}).get("line", 0)
        sig = (h.get("lines", "") or "").split("\n")[0]
        lines.append(f"{f}:{start + 1}: {sig}")
    return "\n".join(lines)


def measure_task(task: dict, ast_grep: str) -> dict:
    a = arm_a_payload(task)
    b = arm_b_payload(task, ast_grep)
    a_bytes, b_bytes = len(a.encode("utf-8")), len(b.encode("utf-8"))
    a_tok, b_tok = _bytes_to_tokens(a_bytes), _bytes_to_tokens(b_bytes)
    # Read-payload proxy: the payload is fresh input (no cache read/write modelled —
    # those need the multi-session harness). So effective == fresh here; we still run
    # it through the formula so the column is schema-identical to the full harness.
    return {
        "task": task["id"],
        "stratum": task["stratum"],
        "kind": task["kind"],
        "arm_a_bytes": a_bytes,
        "arm_b_bytes": b_bytes,
        "arm_a_fresh": round(a_tok, 3),
        "arm_b_fresh": round(b_tok, 3),
        "arm_a_eff": round(effective_input_tokens(a_tok, 0, 0), 3),
        "arm_b_eff": round(effective_input_tokens(b_tok, 0, 0), 3),
        "delta_eff": round(a_tok - b_tok, 3),
        "measurement": "read-payload-proxy",
    }


def load_tasks(path: Path) -> list[dict]:
    tasks = []
    with open(path, encoding="utf-8") as fh:
        for line in fh:
            line = line.strip()
            if line:
                tasks.append(json.loads(line))
    return tasks


def verify_corpus(manifest_path: Path) -> list[str]:
    """Verify the frozen-corpus sha256 manifest against the live tree. Returns a list
    of drift messages (empty == clean). Drift does not crash — it is reported so a
    run over a changed corpus is flagged, not silently trusted."""
    drift = []
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    for rel, meta in manifest.items():
        # Skip the README header and directory glob targets (scanned live, not hashed).
        if rel.startswith("_") or not isinstance(meta, dict) or "sha256" not in meta:
            continue
        p = REPO_ROOT / rel
        if not p.exists():
            drift.append(f"MISSING: {rel}")
            continue
        got = _sha256(p)
        if got != meta["sha256"]:
            drift.append(f"DRIFT: {rel} (manifest {meta['sha256'][:12]}… != live {got[:12]}…)")
    return drift


def main(argv: Optional[list[str]] = None) -> int:
    ap = argparse.ArgumentParser(
        description="Measure the ast-grep+outline read-payload A/B over the frozen tasks."
    )
    ap.add_argument(
        "--tasks", default=str(HERE / "tasks" / "tasks.jsonl"), help="frozen task JSONL"
    )
    ap.add_argument(
        "--manifest", default=str(HERE / "corpus-manifest.json"),
        help="frozen-corpus sha256 manifest (drift-checked before measuring)",
    )
    ap.add_argument("--out", help="write per-task paired rows JSONL here")
    ap.add_argument("--check", action="store_true", help="only verify tools + corpus, then exit")
    args = ap.parse_args(argv)

    try:
        ast_grep = _ast_grep_bin()
    except RuntimeError as e:
        print(f"::error::{e}", file=sys.stderr)
        return 2

    manifest_path = Path(args.manifest)
    drift = verify_corpus(manifest_path) if manifest_path.exists() else ["NO MANIFEST"]
    if drift:
        print("corpus drift (numbers are provisional against the frozen corpus):", file=sys.stderr)
        for d in drift:
            print(f"  {d}", file=sys.stderr)

    if args.check:
        print(f"ast-grep: {ast_grep}")
        print("corpus:", "clean" if not drift else f"{len(drift)} drift item(s)")
        return 0

    tasks = load_tasks(Path(args.tasks))
    rows = [measure_task(t, ast_grep) for t in tasks]

    out_lines = [json.dumps(r) for r in rows]
    text = "\n".join(out_lines)
    if args.out:
        Path(args.out).write_text(text + "\n", encoding="utf-8")
    else:
        print(text)

    # Human-readable per-stratum summary to stderr (runtime-only, non-canonical).
    by_stratum: dict[str, list[float]] = {}
    for r in rows:
        by_stratum.setdefault(r["stratum"], []).append(r["delta_eff"])
    print("\nper-stratum median Δeff (A−B, +=B cheaper) [NON-CANONICAL, runtime-only]:",
          file=sys.stderr)
    for s, ds in sorted(by_stratum.items()):
        ds_sorted = sorted(ds)
        med = ds_sorted[len(ds_sorted) // 2]
        print(f"  {s:14s} n={len(ds):2d}  median Δeff={med:.1f}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
