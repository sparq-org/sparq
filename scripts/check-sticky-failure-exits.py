#!/usr/bin/env python3
"""Reject exception handlers that can erase a failure collected in the same script.

This is deliberately a cheap, conservative structural lint.  It flags a function when
both halves of the dangerous shape are visible in its AST or across one call edge:

* it or a function it calls records a failure via ``*.append/add(...)`` or an assignment
  to a name
  containing ``failure``/``failures``; and
* an exception handler returns/exits with status 0 or trivially falls through.

The default CI backstop scans the repository's ``scripts/`` tree.  A narrow false
positive can be documented with ``sticky-failure-allow: <reason>`` on the handler's
``except`` line.
"""

# Issue #3770 — scripts-tree backstop for exit-zero swallowing sticky failures.

from __future__ import annotations

import argparse
import ast
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ALLOW = "sticky-failure-allow:"
FAILURE_WORDS = ("failure", "failures", "failed")


def _name(node: ast.AST) -> str:
    if isinstance(node, ast.Name):
        return node.id
    if isinstance(node, ast.Attribute):
        return f"{_name(node.value)}.{node.attr}"
    if isinstance(node, ast.Subscript):
        return _name(node.value)
    return ""


def _is_zero(node: ast.AST | None) -> bool:
    # False often means a failed predicate in helpers, so keep this lint focused on
    # unambiguous process-success spellings despite SystemExit(False) also exiting 0.
    return node is None or (
        isinstance(node, ast.Constant)
        and (node.value is None or (type(node.value) is int and node.value == 0))
    )


def _is_exit_zero_call(node: ast.AST) -> bool:
    if not isinstance(node, ast.Call):
        return False
    called = _name(node.func)
    if called not in {"SystemExit", "sys.exit", "exit", "os._exit"}:
        return False
    return not node.args or _is_zero(node.args[0])


def _handler_nodes(handler: ast.ExceptHandler):
    """Yield a handler subtree without attributing nested functions to it."""
    pending: list[ast.AST] = list(reversed(handler.body))
    while pending:
        node = pending.pop()
        yield node
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.Lambda)):
            continue
        pending.extend(reversed(list(ast.iter_child_nodes(node))))


class FunctionFacts(ast.NodeVisitor):
    def __init__(self) -> None:
        self.records_failure = False
        self.zero_handlers: list[ast.ExceptHandler] = []
        self.calls: set[str] = set()

    def visit_FunctionDef(self, node: ast.FunctionDef) -> None:
        # A nested function owns its own outcome; do not mix its facts into the parent.
        return

    visit_AsyncFunctionDef = visit_FunctionDef

    def visit_Assign(self, node: ast.Assign) -> None:
        if any(any(word in _name(target).lower() for word in FAILURE_WORDS)
               for target in node.targets):
            self.records_failure = True
        self.generic_visit(node)

    def visit_AnnAssign(self, node: ast.AnnAssign) -> None:
        if any(word in _name(node.target).lower() for word in FAILURE_WORDS):
            self.records_failure = True
        self.generic_visit(node)

    def visit_AugAssign(self, node: ast.AugAssign) -> None:
        if any(word in _name(node.target).lower() for word in FAILURE_WORDS):
            self.records_failure = True
        self.generic_visit(node)

    def visit_Call(self, node: ast.Call) -> None:
        called = _name(node.func).lower()
        self.calls.add(called.rsplit(".", 1)[-1])
        receiver, _, method = called.rpartition(".")
        if method in {"append", "add"} and any(
            word in receiver for word in FAILURE_WORDS
        ):
            self.records_failure = True
        self.generic_visit(node)

    def visit_ExceptHandler(self, node: ast.ExceptHandler) -> None:
        for statement in _handler_nodes(node):
            if isinstance(statement, ast.Return) and _is_zero(statement.value):
                self.zero_handlers.append(node)
            if isinstance(statement, ast.Raise) and _is_exit_zero_call(statement.exc):
                self.zero_handlers.append(node)
            if (
                isinstance(statement, ast.Expr)
                and _is_exit_zero_call(statement.value)
            ):
                self.zero_handlers.append(node)
        self.generic_visit(node)


def inspect(path: Path) -> list[str]:
    try:
        source = path.read_text(encoding="utf-8")
        tree = ast.parse(source, filename=str(path))
    except (OSError, SyntaxError, UnicodeDecodeError) as error:
        return [f"{path}: could not parse ({error})"]
    lines = source.splitlines()
    findings: list[str] = []
    functions: list[tuple[str, FunctionFacts]] = []
    for function in (
        node for node in ast.walk(tree)
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
    ):
        facts = FunctionFacts()
        for statement in function.body:
            facts.visit(statement)
        if function.body and isinstance(function.body[-1], ast.Try):
            for handler in function.body[-1].handlers:
                if handler.body and all(
                    isinstance(statement, ast.Pass) for statement in handler.body
                ):
                    facts.zero_handlers.append(handler)
        functions.append((function.name, facts))
    recording_functions = {
        function_name.lower()
        for function_name, facts in functions
        if facts.records_failure
    }
    for function_name, facts in functions:
        if not (
            facts.records_failure
            or facts.calls.intersection(recording_functions)
        ):
            continue
        seen_handlers: set[int] = set()
        for handler in facts.zero_handlers:
            if id(handler) in seen_handlers:
                continue
            seen_handlers.add(id(handler))
            line = lines[handler.lineno - 1]
            if ALLOW not in line:
                findings.append(
                    f"{path.relative_to(ROOT) if path.is_relative_to(ROOT) else path}:"
                    f"{handler.lineno}: {function_name} has an exception handler that "
                    "exits 0 while this script records failures; preserve sticky "
                    "precedence or add a reasoned "
                    f"`{ALLOW} ...` waiver"
                )
    return findings


def self_test() -> None:
    cases = {
        "interleaved_bad.py": (
            "def sweep(items):\n"
            "    failures = []\n"
            "    for item in items:\n"
            "        try:\n"
            "            run(item)\n"
            "        except HardError as error:\n"
            "            failures.append(error)\n"
            "        except TransientError:\n"
            "            return 0\n"
            "    return 1 if failures else 0\n",
            1,
        ),
        "sticky_good.py": (
            "def sweep(items):\n"
            "    failures = []\n"
            "    transient = False\n"
            "    for item in items:\n"
            "        try:\n"
            "            run(item)\n"
            "        except HardError as error:\n"
            "            failures.append(error)\n"
            "        except TransientError:\n"
            "            transient = True\n"
            "    # precedence: collected-failure > transient-exhaustion > clean\n"
            "    return 1 if failures else 0\n",
            0,
        ),
        "waived.py": (
            "def sweep():\n"
            "    failures = []\n"
            "    try:\n"
            "        run()\n"
            "    except Error:  # sticky-failure-allow: isolated probe has no prior item\n"
            "        return 0\n"
            "    failures.append('later')\n",
            0,
        ),
        "cross_function_bad.py": (
            "def sweep(items):\n"
            "    outcome.arm_failures.append(run(items))\n"
            "    return outcome\n"
            "\n"
            "def main():\n"
            "    try:\n"
            "        sweep(items)\n"
            "    except TransientError:\n"
            "        return 0\n",
            1,
        ),
        "nested_return_bad.py": (
            "def sweep(items):\n"
            "    failures = []\n"
            "    try:\n"
            "        run(items)\n"
            "    except TransientError as error:\n"
            "        if is_transient(error):\n"
            "            return 0\n",
            1,
        ),
        "append_only_bad.py": (
            "def sweep(items, failures):\n"
            "    try:\n"
            "        run(items)\n"
            "    except HardError as error:\n"
            "        failures.append(error)\n"
            "    except TransientError:\n"
            "        return 0\n",
            1,
        ),
        "ann_assign_bad.py": (
            "def main():\n"
            "    failures: list = []\n"
            "    try:\n"
            "        run()\n"
            "    except Error:\n"
            "        return None\n",
            1,
        ),
        "aug_assign_bad.py": (
            "def main():\n"
            "    failure_count += 1\n"
            "    try:\n"
            "        run()\n"
            "    except Error:\n"
            "        sys.exit()\n",
            1,
        ),
        "os_exit_bad.py": (
            "def main():\n"
            "    failures = collect()\n"
            "    try:\n"
            "        run()\n"
            "    except Error:\n"
            "        os._exit(0)\n",
            1,
        ),
        "pass_bad.py": (
            "def main():\n"
            "    failures = collect()\n"
            "    try:\n"
            "        run()\n"
            "    except Error:\n"
            "        pass\n",
            1,
        ),
        "parse_error.py": ("def broken(\n", 1),
    }
    with tempfile.TemporaryDirectory() as directory:
        for name, (source, expected) in cases.items():
            path = Path(directory) / name
            path.write_text(source, encoding="utf-8")
            actual = len(inspect(path))
            assert actual == expected, f"{name}: expected {expected}, got {actual}"
    print("sticky-failure exit lint self-test: cross-function exits + controls OK")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("paths", nargs="*", type=Path)
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0
    paths = args.paths or sorted((ROOT / "scripts").rglob("*.py"))
    findings = [finding for path in paths for finding in inspect(path)]
    if findings:
        print("\n".join(findings), file=sys.stderr)
        return 1
    print(f"sticky-failure exit lint: {len(paths)} Python scripts clean")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
