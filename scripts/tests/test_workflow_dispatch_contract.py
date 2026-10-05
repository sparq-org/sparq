#!/usr/bin/env python3
# [OPUS-5] sq-sgu1.2 — the `.claude/workflows/*.js` dispatch contract as an INVARIANT.
"""test_workflow_dispatch_contract.py — the harness-workflow scripts must stay dispatchable.

`research/fable-work-plan.md` §6.3 states the contract every workflow in the Fable
collaboration tier must satisfy — "All new workflows must: give **every producing stage a
schema** (schema-guard trap); …" — plus the durability contract (AGENTS.md, sq-lhwo.3) that a
workflow is durable only when it is committed under `.claude/`, linked from `AGENTS.md`, and
carries a one-line `meta.description`.

Those are conventions today, held only by whoever last read the plan. Nothing FAILS when a
new `agent()` dispatch ships without a schema — the stage just returns free-form prose, the
caller reads `result.beads` off a string, and the wave silently does nothing. Nothing fails
when a workflow file stops parsing either: a JS syntax error in `.claude/workflows/` is
invisible to every existing gate and first shows up at run time, mid-tick, after the
orchestrator has already paid for the turn that launched it.

This suite turns those into gate-enforced invariants. It is deliberately STRUCTURAL (regex +
`node --check`, no execution): the harness owns the runtime, so what CI can honestly assert
is shape — every dispatch site carries a schema that is actually defined, and every script
still parses.

Scope note: this suite READS `.claude/workflows/*.js`; it never writes them. That surface is
harness-protected (AGENTS.md rule 11) and is maintainer-applied.
"""
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import yaml

REPO_ROOT = Path(__file__).resolve().parents[2]
WORKFLOW_DIR = REPO_ROOT / ".claude" / "workflows"
GATE = REPO_ROOT / ".github" / "workflows" / "routing-self-tests.yml"
AGENTS_MD = REPO_ROOT / "AGENTS.md"
SELF_PATH = "scripts/tests/test_workflow_dispatch_contract.py"
# The path filter that must wire this gate to the workflow DIRECTORY (PR #4967 review). Pinning
# the directory rather than each current file is what makes the invariant hold for a workflow
# added after this suite was written.
WORKFLOW_GLOB = ".claude/workflows/*.js"


def _gh_path_filter_re(pattern):
    """Compile a GitHub Actions `paths:` filter to a regex.

    Only the two wildcards the filters here use: `*` matches within one path segment, `**`
    crosses separators. Everything else is literal (`.` especially — `*.js` must not match
    `xxjs`)."""
    out, i = [], 0
    while i < len(pattern):
        if pattern[i] == "*":
            if pattern[i + 1:i + 2] == "*":
                out.append(".*")
                i += 2
            else:
                out.append("[^/]*")
                i += 1
        else:
            out.append(re.escape(pattern[i]))
            i += 1
    return re.compile("".join(out) + r"\Z")


# A dispatch site: `agent(` not preceded by an identifier char or a dot (so `subagent(` and
# `x.agent(` are not counted, and `agentType:` — no paren — never matches).
AGENT_CALL_RE = re.compile(r"(?<![\w.])agent\(")
# The opts object of a dispatch site. Every call site in this repo writes opts inline with
# `label:` first and no nested braces, which is what makes a flat scan sound here; the
# equal-count assertion below is what keeps that premise honest.
OPTS_OBJ_RE = re.compile(r"\{\s*label:[^{}]*\}")
SCHEMA_REF_RE = re.compile(r"schema:\s*([A-Za-z_][\w]*)")
CONST_DEF_RE = re.compile(r"^const ([A-Za-z_][\w]*)\s*=\s*\{", re.M)


def workflow_files():
    return sorted(WORKFLOW_DIR.glob("*.js"))


class TestSchemaOnEveryProducingStage(unittest.TestCase):
    """§6.3 schema-guard trap: a producing stage without a schema returns prose, not data."""

    @classmethod
    def setUpClass(cls):
        cls.sources = {f.name: f.read_text(encoding="utf-8") for f in workflow_files()}

    def test_workflow_scripts_are_present(self):
        """Anti-vacuity: if the directory moves, every assertion below iterates an EMPTY dict
        and passes for the wrong reason."""
        self.assertTrue(self.sources, f"no *.js found under {WORKFLOW_DIR} — the scan is vacuous")

    def test_every_dispatch_site_has_a_labelled_opts_object(self):
        """The premise of the flat opts scan: one `{label: ...}` opts object per `agent(` call.

        MUTANT: add an `agent(prompt)` dispatch with no opts (or drop `label:` from one) => RED.
        """
        for name, src in self.sources.items():
            calls = len(AGENT_CALL_RE.findall(src))
            opts = len(OPTS_OBJ_RE.findall(src))
            self.assertEqual(
                calls, opts,
                f"{name}: {calls} agent() dispatch(es) but {opts} `{{label: ...}}` opts object(s) — "
                "every dispatch must pass inline opts led by `label:` (the progress display and "
                "the scan below both depend on it)")

    def test_every_producing_stage_carries_a_schema(self):
        """MUTANT: delete `schema: FRONTIER_SCHEMA` from the frontier dispatch => RED.

        Without a schema the stage's return value is its final TEXT, so the caller's
        `result.beads` is undefined and the wave silently dispatches nothing."""
        for name, src in self.sources.items():
            for opts in OPTS_OBJ_RE.findall(src):
                label = re.search(r"label:\s*'([^']*)'", opts)
                self.assertIn(
                    "schema:", opts,
                    f"{name}: dispatch {label.group(1) if label else opts[:60]!r} has no `schema:` — "
                    "every producing stage must be schema'd (research/fable-work-plan.md §6.3)")

    def test_every_referenced_schema_is_defined_in_its_file(self):
        """A typo'd schema name is `undefined` at run time — the harness then forces no
        structured output at all, i.e. the same silent failure as omitting it.

        MUTANT: rename one `const X_SCHEMA = {` definition without its reference => RED."""
        for name, src in self.sources.items():
            defined = set(CONST_DEF_RE.findall(src))
            for ref in SCHEMA_REF_RE.findall(src):
                self.assertIn(ref, defined,
                              f"{name}: `schema: {ref}` is not defined in this file")

    def test_every_schema_pins_additional_properties(self):
        """`additionalProperties: false` is what makes a schema a CONTRACT rather than a hint —
        it is why a stage returning an undocumented field is a visible validation failure
        instead of a field the caller silently never reads.

        MUTANT: drop `additionalProperties: false` from any schema => RED."""
        for name, src in self.sources.items():
            for ref in sorted(set(SCHEMA_REF_RE.findall(src))):
                start = src.index(f"const {ref} = {{")
                head = src[start:start + 200]
                self.assertRegex(
                    head, r"additionalProperties:\s*false",
                    f"{name}: schema {ref} does not pin `additionalProperties: false`")


class TestWorkflowScriptsParse(unittest.TestCase):
    """A syntax error in a workflow script is invisible until the tick that runs it."""

    def _wrapped(self, src):
        """Workflow scripts are neither plain CJS nor a plain ES module: they use
        `export const meta`, top-level `await` AND top-level `return`. The harness evaluates
        the body as an async function, so reproduce exactly that shape for the parse check."""
        body = re.sub(r"^export const meta", "const meta", src, count=1, flags=re.M)
        return "async function __wf() {\n" + body + "\n}\n"

    def test_every_workflow_script_parses(self):
        """MUTANT: drop a closing brace in any workflow script => RED."""
        node = shutil.which("node")
        if node is None:  # pragma: no cover - CI runners ship node
            self.skipTest("node not available")
        files = workflow_files()
        self.assertTrue(files, "no workflow scripts found — the parse check is vacuous")
        with tempfile.TemporaryDirectory() as tmp:
            for f in files:
                probe = Path(tmp) / "probe.js"
                probe.write_text(self._wrapped(f.read_text(encoding="utf-8")), encoding="utf-8")
                proc = subprocess.run([node, "--check", str(probe)],
                                      capture_output=True, text=True)
                self.assertEqual(proc.returncode, 0,
                                 f"{f.name} does not parse:\n{proc.stderr.strip()}")


class TestDurabilityContract(unittest.TestCase):
    """AGENTS.md sq-lhwo.3: committed + linked from AGENTS.md + a one-line description.

    Missing any one and the workflow gets silently re-improvised by the next session that
    needed it — which is the failure this contract exists to prevent, not a formality."""

    @classmethod
    def setUpClass(cls):
        cls.sources = {f.name: f.read_text(encoding="utf-8") for f in workflow_files()}
        cls.agents_md = AGENTS_MD.read_text(encoding="utf-8")

    def test_meta_name_matches_the_filename(self):
        """`Workflow({name: ...})` resolves by meta.name; a drift makes the file unreachable
        under the name every caller (and AGENTS.md) uses.

        MUTANT: change any meta.name => RED."""
        for name, src in self.sources.items():
            m = re.search(r"name:\s*'([^']+)'", src)
            self.assertIsNotNone(m, f"{name}: no meta.name")
            self.assertEqual(m.group(1), name[:-len(".js")],
                             f"{name}: meta.name must equal the filename stem")

    def test_every_workflow_has_a_one_line_description(self):
        for name, src in self.sources.items():
            m = re.search(r"description:\s*'((?:[^'\\]|\\.)+)'", src)
            self.assertIsNotNone(m, f"{name}: meta.description is required (durability contract)")
            self.assertGreater(len(m.group(1)), 40,
                               f"{name}: meta.description is too thin to be discoverable")

    def test_every_workflow_is_linked_from_agents_md(self):
        """MUTANT: add a workflow under .claude/workflows/ without an AGENTS.md entry => RED."""
        for name in self.sources:
            self.assertIn(
                f".claude/workflows/{name}", self.agents_md,
                f"{name} is not linked from AGENTS.md — the durability contract needs all three "
                "(committed + linked + one-line description) or the workflow is re-improvised")


class TestGateWiring(unittest.TestCase):
    """A guard nobody runs cannot go red; a guard whose surface is not a path trigger does not
    run on the PR that breaks it."""

    @classmethod
    def setUpClass(cls):
        cls.raw = GATE.read_text(encoding="utf-8")
        cls.doc = yaml.safe_load(cls.raw)
        # Same accessor as the sibling suite on this same file: the gating job is `validate`,
        # so an invocation moved into some other (possibly skipped) job does not count.
        cls.steps = cls.doc["jobs"]["validate"]["steps"]
        cls.trigger_section = cls.raw[:cls.raw.index("permissions:")]

    def _run_steps(self):
        return [s for s in self.steps if "run" in s]

    def test_this_suite_is_actually_invoked(self):
        """MUTANT: delete the invocation line from routing-self-tests.yml => RED."""
        runs = "\n".join(s["run"] for s in self._run_steps())
        self.assertIn(f"python3 {SELF_PATH}", runs,
                      f"`python3 {SELF_PATH}` is never RUN by {GATE.name}")

    def test_this_file_is_a_path_trigger_on_both_triggers(self):
        """Scoped to the trigger section: the filename also appears in the run: block, so a
        whole-file search would pass for the wrong reason."""
        self.assertEqual(self.trigger_section.count(f'"{SELF_PATH}"'), 2,
                         "must be a path trigger on BOTH pull_request and push")

    def test_the_workflow_directory_is_a_path_trigger(self):
        """The model-deprecation guard (test_no_deprecated_models.py::TestWorkflowJsDispatch)
        scans EVERY `.claude/workflows/*.js` for its dispatch models, but only
        fable-architect-drain.js was a path trigger — so a PR editing any other workflow's
        dispatch site never ran the gate that guards it.

        The trigger must be the DIRECTORY, not an enumeration of today's files: an enumeration
        cannot cover a workflow that does not exist yet, so a newly added script would match no
        filter, this lane would not run, and it would merge unlinked, unschema'd or unparseable —
        with every assertion in this file silently skipped rather than red.

        MUTANT: replace the glob with the individual workflow filenames => RED."""
        self.assertEqual(
            self.trigger_section.count(f'"{WORKFLOW_GLOB}"'), 2,
            f"`{WORKFLOW_GLOB}` must be a path trigger on BOTH pull_request and push — an "
            "enumeration of the current files lets a NEW workflow merge without this gate")

    def test_every_workflow_file_is_covered_by_a_declared_path_filter(self):
        """Anti-vacuity for the glob above: a workflow the declared filters do not MATCH — a
        `.mjs` script, or one nested a directory deeper — is back in the bypass class even though
        the glob is present.

        MUTANT: add `.claude/workflows/lib/helper.mjs` => RED (no declared filter matches it)."""
        on = self.doc.get(True, self.doc.get("on", {}))
        files = sorted(p for p in WORKFLOW_DIR.rglob("*") if p.is_file())
        self.assertTrue(files, f"no files under {WORKFLOW_DIR} — the coverage check is vacuous")
        for event in ("pull_request", "push"):
            patterns = [_gh_path_filter_re(p) for p in on[event]["paths"]]
            for f in files:
                rel = f.relative_to(REPO_ROOT).as_posix()
                self.assertTrue(
                    any(p.match(rel) for p in patterns),
                    f"{rel} matches no `{event}:` path filter — an edit to it would not run "
                    "this gate, whose suites nonetheless scan it")


if __name__ == "__main__":
    sys.exit(not unittest.main(verbosity=2, exit=False).result.wasSuccessful())
