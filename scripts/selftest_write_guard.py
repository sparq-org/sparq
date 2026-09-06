#!/usr/bin/env python3
# [OPUS-5] 🤖 SPARQ agent — static guard: a self-test must not be able to write to a live repo.
#
# WHY THIS EXISTS (sparq-org/sparq#5098)
# --------------------------------------
# `scripts/retriage-needs-user.py` (PR #4184) self-tested its "never writes" guard by CALLING the
# real write function with writes enabled:
#
#     apply_one({"n": 7}, "keep", ["needs:user"], [], "r", dry_run=False, log=lambda *_: None)
#
# `apply_one`'s `repo` parameter defaulted to the module constant `REPO = "sparq-org/sparq"`, and
# its body shells out through `_run` -> `subprocess.run(["gh", "issue", ...])`. The ONLY thing
# standing between `--selftest` and a production mutation was the `if action in ("keep", "skip"):
# return False` early-out — i.e. the very branch the assertion existed to test. Mutate that branch
# (which the PR itself advertised as its verification method) and the self-test posts a real comment
# to a real issue. It did: comments 5084272766 / 5092788561 on #7 and 5098073828 / 5117974900 on #8,
# both closed dependabot PRs, each rendering the fixture reason string "r".
#
# A test whose safety depends on the correctness of the code under test is not a test. This module
# is the structural repair, expressed as two rules that hold for the whole `scripts/` tree.
#
# RULE 1 — NO LIVE DEFAULT TARGET (`live-default-target`)
#   A function that takes a dry-run-style parameter is a WRITE function. It may not give its
#   target-repository parameter a default that resolves to a live repository slug, directly or via
#   a module-level constant. The write target must be named at every call site, so no caller can
#   inherit production by omission.
#
# RULE 2 — NO REACHABLE SUBPROCESS FROM A WRITE-ENABLED SELF-TEST (`selftest-writes-live`)
#   Inside a self-test, a call that enables writes (`dry_run=False`, `apply=True`, ...) is a
#   violation when the callee can still reach a real subprocess/network seam. The exemption is
#   NEUTRALISATION, not intent: the self-test must rebind (`global` + assign) every seam reachable
#   from the callee's call graph before the call. That is the pattern `scripts/batch-merge.py`
#   already uses correctly — it swaps `run_git`/`run_gh` for recorders and restores them in a
#   `finally` — and it is exactly what `retriage-needs-user.py` did not do.
#
# The write-enabling keyword set and the live-owner set are DECLARED below rather than inferred, so
# widening the guard's blast radius is a reviewable diff. The suite
# `scripts/tests/test_selftest_never_writes.py` validates this detector against known-positive
# fixtures (including the verbatim #5098 shape) before it is ever run over the tree — an
# instrument that has only ever been run on a clean tree has not been shown to detect anything.
#
# Stdlib only. Pure functions over source text: no import, no execution of the scanned files.

from __future__ import annotations

import argparse
import ast
import re
import sys
from dataclasses import dataclass
from pathlib import Path

# A call passing one of these keyword=literal pairs is ENABLING WRITES.
WRITE_ENABLING = {
    "dry_run": False,
    "dryrun": False,
    "no_dry_run": True,
    "apply": True,
    "write": True,
    "live": True,
}
# Parameters that name the repository a write lands in.
TARGET_PARAM = re.compile(r"^(repo|repository|owner_repo|target_repo|slug)$")
# Parameter names that make a function a WRITE function for rule 1.
DRY_RUN_PARAMS = frozenset(WRITE_ENABLING)
# `owner/name`.
SLUG = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]*/[A-Za-z0-9._-]+$")
# Owners whose repositories are LIVE. A fixture slug (`fixture/repo`, `acme/widget`) is fine; these
# are the ones a stray `gh` invocation actually mutates.
LIVE_OWNERS = frozenset({"sparq-org", "jeswr"})
# A function is a self-test entry point when its name matches. `--self-test` / `--selftest` argv
# dispatch in this tree always lands on one of these.
SELFTEST_NAME = re.compile(r"^_?(self_?test|selftest)", re.IGNORECASE)
# Real-world side-effect seams. Reaching one of these from a write-enabled self-test call is the
# failure; `gh`/`git` in this tree are always spawned through `subprocess`.
SEAM_ROOTS = frozenset({"subprocess", "urllib", "requests", "socket", "httpx"})
SEAM_CALLS = frozenset({"os.system", "os.popen", "os.remove", "os.rename"})


@dataclass(frozen=True)
class Violation:
    path: str
    line: int
    rule: str
    detail: str

    def __str__(self) -> str:  # pragma: no cover - formatting only
        return f"{self.path}:{self.line}: [{self.rule}] {self.detail}"


def _str_constants(tree: ast.Module) -> dict[str, str]:
    """Module-level `NAME = "literal"` bindings, so `repo=REPO` resolves to its value."""
    out: dict[str, str] = {}
    for node in tree.body:
        if isinstance(node, ast.Assign) and isinstance(node.value, ast.Constant) \
                and isinstance(node.value.value, str):
            for target in node.targets:
                if isinstance(target, ast.Name):
                    out[target.id] = node.value.value
    return out


def _resolve_str(node: ast.expr | None, consts: dict[str, str]) -> str | None:
    if isinstance(node, ast.Constant) and isinstance(node.value, str):
        return node.value
    if isinstance(node, ast.Name):
        return consts.get(node.id)
    return None


def is_live_slug(value: str | None) -> bool:
    return bool(value) and bool(SLUG.match(value)) and value.split("/")[0] in LIVE_OWNERS


def _functions(tree: ast.Module) -> dict[str, ast.FunctionDef | ast.AsyncFunctionDef]:
    """Module-level function definitions by name (the call-graph nodes we can resolve)."""
    return {n.name: n for n in tree.body
            if isinstance(n, (ast.FunctionDef, ast.AsyncFunctionDef))}


def _calls_in(fn: ast.AST) -> list[str]:
    return [ast.unparse(c.func) for c in ast.walk(fn) if isinstance(c, ast.Call)]


def _is_seam_call(name: str) -> bool:
    return name.split(".")[0] in SEAM_ROOTS or name in SEAM_CALLS


def direct_seams(tree: ast.Module) -> set[str]:
    """Module-level functions that themselves touch a side-effect seam."""
    return {name for name, fn in _functions(tree).items()
            if any(_is_seam_call(c) for c in _calls_in(fn))}


def seams_reachable_from(name: str, tree: ast.Module) -> set[str] | None:
    """Every seam function reachable from `name` in this module's call graph.

    Returns None when `name` is not a module-level function here — the callee lives in another
    module and this file-local analysis cannot see its seams. Callers must then fall back to the
    live-target test (rule 1 guarantees such a callee cannot default to production).
    """
    funcs = _functions(tree)
    if name not in funcs:
        return None
    seams = direct_seams(tree)
    reached: set[str] = set()
    seen: set[str] = set()
    stack = [name]
    while stack:
        cur = stack.pop()
        if cur in seen:
            continue
        seen.add(cur)
        if cur in seams:
            reached.add(cur)
        for callee in _calls_in(funcs[cur]) if cur in funcs else []:
            head = callee.split(".")[0]
            if head in funcs and head not in seen:
                stack.append(head)
    return reached


def neutralised_in(fn: ast.AST) -> set[str]:
    """Module globals this function both declares `global` AND assigns — i.e. swaps for a fake."""
    declared: set[str] = set()
    assigned: set[str] = set()
    for node in ast.walk(fn):
        if isinstance(node, ast.Global):
            declared.update(node.names)
        elif isinstance(node, ast.Assign):
            for target in node.targets:
                for leaf in ast.walk(target):
                    if isinstance(leaf, ast.Name):
                        assigned.add(leaf.id)
    return declared & assigned


def write_enabling_kwargs(call: ast.Call) -> list[str]:
    """`dry_run=False`-style keyword literals on this call."""
    return [kw.arg for kw in call.keywords
            if kw.arg in WRITE_ENABLING
            and isinstance(kw.value, ast.Constant)
            and kw.value.value is WRITE_ENABLING[kw.arg]]


def _param_names(fn: ast.FunctionDef | ast.AsyncFunctionDef) -> list[str]:
    args = fn.args
    return [a.arg for a in args.posonlyargs + args.args]


def write_enabling_positional(call: ast.Call, tree: ast.Module) -> list[str]:
    """A write-enabling literal passed POSITIONALLY — `apply_one(i, a, r, d, x, repo, False)`.

    Without this a caller sidesteps the keyword form, which is the cheapest possible bypass of a
    keyword-only guard.
    """
    if not isinstance(call.func, ast.Name):
        return []
    target = _functions(tree).get(call.func.id)
    if target is None:
        return []
    names = _param_names(target)
    found = []
    for index, arg in enumerate(call.args):
        if index >= len(names):
            break
        name = names[index]
        if name in WRITE_ENABLING and isinstance(arg, ast.Constant) \
                and arg.value is WRITE_ENABLING[name]:
            found.append(name)
    return found


def _defaults(fn: ast.FunctionDef | ast.AsyncFunctionDef) -> dict[str, ast.expr]:
    args = fn.args
    positional = args.posonlyargs + args.args
    out: dict[str, ast.expr] = {}
    for arg, default in zip(positional[len(positional) - len(args.defaults):], args.defaults):
        out[arg.arg] = default
    for arg, default in zip(args.kwonlyargs, args.kw_defaults):
        if default is not None:
            out[arg.arg] = default
    return out


def check_source(source: str, path: str = "<source>") -> list[Violation]:
    """Both rules over one Python source file. Never imports or executes it."""
    try:
        tree = ast.parse(source)
    except SyntaxError as exc:  # a file that does not parse is not this guard's business
        return [Violation(path, exc.lineno or 0, "unparseable", str(exc))]
    consts = _str_constants(tree)
    violations: list[Violation] = []

    # --- rule 1: a write function may not default its target to a live repo -------------------
    for fn in ast.walk(tree):
        if not isinstance(fn, (ast.FunctionDef, ast.AsyncFunctionDef)):
            continue
        args = fn.args
        names = {a.arg for a in args.posonlyargs + args.args + args.kwonlyargs}
        if not names & DRY_RUN_PARAMS:
            continue
        for param, default in _defaults(fn).items():
            if not TARGET_PARAM.match(param):
                continue
            value = _resolve_str(default, consts)
            if is_live_slug(value):
                violations.append(Violation(
                    path, fn.lineno, "live-default-target",
                    f"{fn.name}() takes a dry-run parameter, so it is a write function, but its "
                    f"`{param}` parameter defaults to the live repository {value!r} "
                    f"(via `{ast.unparse(default)}`). Drop the default so every call site must "
                    f"name the repository it writes to."))

    # --- rule 2: a write-enabled self-test call may not reach a seam ---------------------------
    for fn in ast.walk(tree):
        if not isinstance(fn, (ast.FunctionDef, ast.AsyncFunctionDef)):
            continue
        if not SELFTEST_NAME.match(fn.name):
            continue
        neutralised = neutralised_in(fn)
        for call in ast.walk(fn):
            if not isinstance(call, ast.Call):
                continue
            enabling = write_enabling_kwargs(call) + write_enabling_positional(call, tree)
            if not enabling:
                continue
            callee = ast.unparse(call.func)
            reachable = seams_reachable_from(callee.split(".")[0], tree)
            if reachable is None:
                # Cross-module callee: rule 1 guarantees it cannot default to a live repo, so the
                # residual risk is a live slug handed over explicitly at this call site.
                live = [ast.unparse(a) for a in call.args
                        if is_live_slug(_resolve_str(a, consts))]
                live += [f"{kw.arg}={ast.unparse(kw.value)}" for kw in call.keywords
                         if is_live_slug(_resolve_str(kw.value, consts))]
                if live:
                    violations.append(Violation(
                        path, call.lineno, "selftest-writes-live",
                        f"self-test {fn.name}() calls {callee}() with {', '.join(enabling)} "
                        f"against the live repository ({', '.join(live)})."))
                continue
            exposed = sorted(reachable - neutralised)
            if exposed:
                violations.append(Violation(
                    path, call.lineno, "selftest-writes-live",
                    f"self-test {fn.name}() calls {callee}() with {', '.join(enabling)} while "
                    f"{', '.join(exposed)} still reach{'es' if len(exposed) == 1 else ''} a real "
                    f"subprocess. Either drop the write-enabling argument or rebind "
                    f"{', '.join(exposed)} (`global` + assign) for the duration of the call, as "
                    f"scripts/batch-merge.py does for run_git/run_gh."))
    return violations


def check_tree(root: Path) -> list[Violation]:
    """Every `*.py` under `root`, sorted, so the report is stable."""
    out: list[Violation] = []
    for path in sorted(root.rglob("*.py")):
        out.extend(check_source(path.read_text(encoding="utf-8", errors="replace"),
                                str(path)))
    return out


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0] if __doc__ else None)
    ap.add_argument("root", nargs="?", default=str(Path(__file__).resolve().parent),
                    help="directory to scan (default: scripts/)")
    args = ap.parse_args(argv)
    violations = check_tree(Path(args.root))
    for violation in violations:
        print(violation)
    print(f"{'FAILED' if violations else 'ok'}: selftest-write-guard "
          f"({len(violations)} violation(s))")
    return 1 if violations else 0


if __name__ == "__main__":
    raise SystemExit(main())
