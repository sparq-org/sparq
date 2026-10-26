#!/usr/bin/env python3
"""#6081: guard against PR label-event fan-out. Run directly with Python 3."""
from __future__ import annotations

import ast
import re
import unittest
from pathlib import Path

from _yaml_seam import yaml_or_local_skip

yaml = yaml_or_local_skip()
WORKFLOWS = Path(__file__).resolve().parents[2] / ".github/workflows"
PR_EVENTS = {"pull_request", "pull_request_target"}


def triggers(doc):
    on = doc.get("on", doc.get(True))  # PyYAML's YAML 1.1 spelling of `on`.
    if isinstance(on, str):
        return {on: None}
    return dict.fromkeys(on, None) if isinstance(on, list) else on


def payload(action="labeled", label="bench", labels=("bench",), event="pull_request", run_id=101):
    return {"event_name": event, "ref": "refs/pull/42/merge", "run_id": run_id,
            "event": {"action": action, "label": {"name": label},
                      "pull_request": {"labels": [{"name": name} for name in labels]}}}


def evaluate(expression, github):
    """Interpret only this gate/group subset; never execute expression code."""
    if isinstance(expression, bool):
        return expression
    values = {"true": True, "false": False}

    def token(match):
        text = match[0]
        if text.startswith("github."):
            value = github
            for key in text.split(".")[1:]:
                if key != "*":
                    value = [item[key] for item in value] if isinstance(value, list) else value[key]
        elif text.startswith("'"):
            value = text[1:-1].replace("''", "'")
        elif text in ("contains", "format", "true", "false", "==", "!=", "(", ")", ",") or text.isspace():
            return text
        elif text in ("&&", "||", "!"):
            return {"&&": " and ", "||": " or ", "!": " not "}[text]
        else:
            raise ValueError(f"Unsupported expression token: {text}")
        name = f"context_{len(values)}"
        values[name] = value  # Values stay data; only generated names enter the AST.
        return name

    def visit(node):
        if isinstance(node, ast.Name):
            return values[node.id]
        if isinstance(node, ast.UnaryOp) and isinstance(node.op, ast.Not):
            return not visit(node.operand)
        if isinstance(node, ast.BoolOp):
            result, *rest = [visit(item) for item in node.values]
            for item in rest:
                result = (result and item) if isinstance(node.op, ast.And) else (result or item)
            return result
        if isinstance(node, ast.Compare) and len(node.ops) == 1:
            equal = visit(node.left) == visit(node.comparators[0])
            if isinstance(node.ops[0], (ast.Eq, ast.NotEq)):
                return equal if isinstance(node.ops[0], ast.Eq) else not equal
        if isinstance(node, ast.Call) and isinstance(node.func, ast.Name) and not node.keywords:
            args = [visit(arg) for arg in node.args]
            if node.func.id == "contains" and len(args) == 2:
                return args[1] in args[0]
            if node.func.id == "format" and len(args) == 2:
                return args[0].format(args[1])
        raise ValueError(f"Unsupported expression syntax: {ast.dump(node)}")

    expression = expression.removeprefix("${{").removesuffix("}}").strip()
    expression = re.sub(r"'(?:''|[^'])*'|github(?:\.[\w*]+)+|\w+|&&|\|\||==|!=|[!(),]|\s+|.", token, expression)
    return visit(ast.parse(expression.strip(), mode="eval").body)


def job_runs(jobs, name, github, visiting=()):
    assert name not in visiting, "cyclic needs graph"
    job = jobs[name]
    needs = job.get("needs", [])
    needs = [needs] if isinstance(needs, str) else needs
    parents = [job_runs(jobs, parent, github, (*visiting, name)) for parent in needs]
    return bool(evaluate(job.get("if", True), github)) and all(parents)


class TestHeavyWorkflowLabels(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.docs = {p.name: yaml.safe_load(p.read_text()) for p in sorted(WORKFLOWS.iterdir())
                    if p.suffix in (".yml", ".yaml")}
        cls.bench = cls.docs["bench.yml"]

    def test_only_bench_subscribes_to_label_events(self):
        found = {name for name, doc in self.docs.items() for event, config in triggers(doc).items()
                 if event in PR_EVENTS and isinstance(config, dict)
                 and {"labeled", "unlabeled"}.intersection(config.get("types", []))}
        self.assertEqual(found, {"bench.yml"})

    def test_heavy_suites_have_no_pr_trigger(self):
        for name in ("ci.yml", "feature-matrix.yml", "fuzz.yml", "vectorized-feature-off.yml"):
            with self.subTest(workflow=name):
                self.assertFalse(PR_EVENTS.intersection(triggers(self.docs[name])))

    def test_bench_triggers_and_every_job_gate(self):
        self.assertEqual(triggers(self.bench)["pull_request"]["types"],
                         ["opened", "synchronize", "reopened", "labeled"])
        self.assertNotIn("pull_request_target", triggers(self.bench))
        jobs = self.bench["jobs"]
        self.assertTrue(jobs)
        cases = [(payload(label="review:queued", labels=labels), False)
                 for labels in (("review:queued",), ("review:queued", "bench"))]
        cases += [(payload(), True)]
        cases += [(payload(event=event, label="", labels=()), True)
                  for event in ("push", "schedule", "workflow_dispatch")]
        for github, expected in cases:
            for name in jobs:
                with self.subTest(job=name, payload=github):
                    self.assertEqual(job_runs(jobs, name, github), expected)

    def test_ignored_labels_cannot_cancel_benchmark_runs(self):
        def group(github):
            return re.sub(r"\$\{\{(.*?)\}\}", lambda m: str(evaluate(m[1], github)),
                          self.bench["concurrency"]["group"], flags=re.S)
        self.assertIs(self.bench["concurrency"]["cancel-in-progress"], True)
        real = {group(payload()), group(payload(action="synchronize"))}
        for labels in (("review:queued",), ("review:queued", "bench")):
            ignored = {group(payload(label="review:queued", labels=labels, run_id=run_id))
                       for run_id in (201, 202)}
            self.assertEqual(len(ignored), 2, "ignored runs must have unique groups")
            self.assertTrue(ignored.isdisjoint(real), "ignored labels cancel real runs")


if __name__ == "__main__":
    unittest.main(verbosity=2)
