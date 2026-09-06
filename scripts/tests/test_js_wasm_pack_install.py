#!/usr/bin/env python3
# [OPUS-5] sq-khm3f — INSPECTION test for how the GATING `js` lane
# (.github/workflows/js.yml) puts wasm-pack on PATH.
#
# WHY a test at all: the property is a single `uses:`/`run:` YAML line, and its
# failure mode is INVISIBLE. Reverting to `cargo install wasm-pack --locked` keeps
# the lane green on a good day — it just recompiles wasm-pack and its whole
# chrono/wasm-bindgen source tree from crates.io on every run, so a transient
# registry/CDN blip reds this HARD gate on an unrelated PR (observed on PR #1131:
# "download of wa/sm/wasm-bindgen failed / curl failed: [16] Error in the HTTP2
# framing layer", cleared by a plain re-run). Nothing goes red when the hardening
# is dropped, so the posture is pinned here instead:
#
#   1. NO SOURCE COMPILE. js.yml must not `cargo install wasm-pack` anywhere.
#   2. PREBUILT BINARY, SHA-PINNED. wasm-pack comes from jetli/wasm-pack-action
#      (one static musl tarball, fetched through @actions/tool-cache, which retries
#      the download), pinned to a 40-hex commit SHA per the repo's action-pin policy.
#   3. EXACT VERSION PIN. `version:` is an exact `vX.Y.Z`, never `latest` —
#      `latest` re-adds an unauthenticated api.github.com release lookup, i.e. a
#      second rate-limit-flake source, which is the thing this bead removes.
#   4. ORDERING. The install still precedes the root `npm ci`, because the package's
#      `prepare` lifecycle (sq-bkag git-pin build) runs on `npm ci` and needs
#      wasm-pack on PATH to compile the wasm engine.
#
# SCOPE — properties 1–4 are deliberately js.yml ONLY. gui.yml / site-e2e-hero.yml /
# site-visual.yml still `cargo install` wasm-pack; converting them is separate work and
# is NOT asserted here, so this file's silence about them is not a claim that they are
# hardened.
#
# One CROSS-WORKFLOW property is asserted, by WasmPackVersionUnified below:
#
#   5. ONE VERSION REPO-WIDE. Every workflow that installs wasm-pack through
#      jetli/wasm-pack-action must pass the action exactly one `with: version:` input
#      (read indentation-aware, so a `version:` under a sibling mapping does not count
#      as a pin), and they must all request the SAME one (a step with none takes the
#      action's default, i.e. it is an unpinned lane, which is the same regression in
#      a different shape). #5771: ci.yml's `wasm`
#      job — the lane that RUNS the headless `wasm-pack test --node` suites — pinned
#      v0.13.1 while js.yml — the lane that BUILDS the published artifact — pinned
#      v0.15.0, so a wasm-pack/wasm-bindgen behaviour change between those releases was
#      exercised in the build lane and never in the test lane. Nothing goes red when the
#      two drift apart again, hence this assertion. It says nothing about WHICH version
#      is right; bumping is fine, bumping one lane only is not.
#
# One DOC-SIDE property is asserted, by ContributorHintsMatchThePin below:
#
#   6. THE CONTRIBUTOR HINTS NAME THAT VERSION. js/README.md and the two
#      guardrails/prepare-build.mjs files tell a human whose PATH lacks wasm-pack to
#      `cargo install` it. Unversioned (#5777) that floats to the current release, so
#      a local `npm ci` builds the very bundle CI publishes with a wasm-pack — and the
#      wasm-bindgen CLI that release bundles — no lane validates. The expected value is
#      read FROM the workflow pin, so bumping the lanes without the hints goes red.
#      These are prose and JS string literals, not shell steps, so no workflow-scoped
#      checker can see them; they need this doc-side extractor.
#
# The step splitter is single-sourced from scripts/check-install-action-tool.py
# rather than re-implemented. Hermetic: stdlib only (no PyYAML, no network, no gh).
# Run:  python3 scripts/tests/test_js_wasm_pack_install.py

from __future__ import annotations

import importlib.util
import re
import sys
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
WORKFLOWS = REPO_ROOT / ".github" / "workflows"
JS_YML = WORKFLOWS / "js.yml"

WASM_PACK_ACTION = "jetli/wasm-pack-action"
# The repo's action-pin policy: a 40-char lowercase hex commit SHA, never a tag.
SHA_RE = re.compile(r"^[0-9a-f]{40}$")
# An exact release pin, e.g. `v0.15.0`. `latest` (and any floating ref) must fail.
VERSION_RE = re.compile(r"^v\d+\.\d+\.\d+$")


def _load(name: str, filename: str):
    spec = importlib.util.spec_from_file_location(
        name, REPO_ROOT / "scripts" / filename
    )
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


gate = _load("check_install_action_tool", "check-install-action-tool.py")


def _code_lines(text: str) -> list[str]:
    """Lines with comments and blanks dropped — so a `#`-commented mention of the
    old command (this repo comments its workflows heavily) is never mistaken for a
    live step."""
    out = []
    for raw in text.splitlines():
        stripped = raw.strip()
        if not stripped or stripped.startswith("#"):
            continue
        out.append(raw)
    return out


def _with_versions(block: list[str]) -> list[str]:
    """Every value of a `version:` key that is a DIRECT child of the step's `with:`
    mapping, in order (so duplicates are visible to the caller).

    Scoping matters: `with.version` is the ONLY key that reaches the action as an
    input. A bare "any stripped line starting `version:`" scan would also read a
    `version:` living under some sibling mapping (`env:`, a matrix entry, a nested
    input object), so a step that dropped its `with.version` — and therefore silently
    takes the action's default, the #5771 regression — could still look pinned. The
    indentation-aware walk mirrors `check-install-action-tool.py`'s `_has_with_tool`.
    """
    versions: list[str] = []
    with_indent: int | None = None
    child_indent: int | None = None
    for raw in block:
        if not raw.strip() or raw.lstrip(" ").startswith("#"):
            continue
        # A key introduced on the dash line itself logically begins after the "- ".
        ind = gate._indent(raw)
        key = raw.lstrip(" ")
        if key.startswith("- "):
            ind += 2
            key = key[2:]
        if with_indent is None:
            if key.startswith("with:"):
                with_indent = ind
                child_indent = None
            continue
        if ind <= with_indent:
            # Dedented out of the `with:` mapping (e.g. into a sibling `env:`).
            with_indent = ind if key.startswith("with:") else None
            child_indent = None
            continue
        if child_indent is None:
            child_indent = ind
        if ind != child_indent:
            # Nested deeper than the mapping's own keys — not a `with:` input.
            continue
        if key.startswith("version:"):
            versions.append(key.split(":", 1)[1].strip())
    return versions


class JsLaneWasmPackInstall(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.text = JS_YML.read_text(encoding="utf-8")
        cls.code = _code_lines(cls.text)

    def _wasm_pack_step(self) -> list[str]:
        # Match on the step's parsed `uses:` action, NOT on a substring of the block:
        # this workflow is heavily commented, and the splitter keeps comment lines, so
        # a prose mention of the action leaks into the PRECEDING step's block.
        blocks = [
            b
            for b in gate.split_steps(self.text)
            if (gate._step_uses(b) or (None,))[0] == WASM_PACK_ACTION
        ]
        self.assertEqual(
            len(blocks),
            1,
            f"js.yml must have exactly one {WASM_PACK_ACTION} step, found "
            f"{len(blocks)}",
        )
        return blocks[0]

    def test_no_cargo_install_wasm_pack(self):
        """(1) The source compile — the actual flake surface — must be gone."""
        offenders = [ln for ln in self.code if "cargo install wasm-pack" in ln]
        self.assertEqual(
            offenders,
            [],
            "js.yml must NOT `cargo install wasm-pack`: compiling it (and the "
            "chrono/wasm-bindgen source tree) from crates.io on every run is what "
            "let a transient registry blip red this gating lane (sq-khm3f). Install "
            f"the prebuilt binary via {WASM_PACK_ACTION} instead.",
        )

    def test_installs_prebuilt_binary_sha_pinned(self):
        """(2) The replacement is the SHA-pinned prebuilt-download action."""
        block = self._wasm_pack_step()
        uses = gate._step_uses(block)
        self.assertIsNotNone(uses, "the wasm-pack step must have a `uses:` line")
        action, ref = uses
        self.assertEqual(action, WASM_PACK_ACTION)
        self.assertRegex(
            ref,
            SHA_RE,
            f"{WASM_PACK_ACTION} must be pinned to a 40-hex commit SHA (repo "
            f"action-pin policy), got {ref!r}",
        )

    def test_version_is_pinned_exactly_not_latest(self):
        """(3) An exact vX.Y.Z — `latest` re-adds a network lookup to resolve it."""
        block = self._wasm_pack_step()
        versions = _with_versions(block)
        self.assertEqual(
            len(versions),
            1,
            "the wasm-pack step must carry exactly one `with: version:` input, found "
            f"{versions!r}",
        )
        self.assertRegex(
            versions[0],
            VERSION_RE,
            "wasm-pack must be pinned to an exact release (e.g. v0.15.0). "
            "`latest` costs an unauthenticated api.github.com lookup on every run "
            "— a second rate-limit-flake source (sq-khm3f).",
        )

    def test_install_precedes_npm_ci(self):
        """(4) `prepare` runs on `npm ci` and needs wasm-pack already on PATH."""
        install_at = [
            i for i, ln in enumerate(self.code) if WASM_PACK_ACTION in ln
        ]
        npm_ci_at = [
            i
            for i, ln in enumerate(self.code)
            if re.match(r"^\s*run:\s*npm ci\s*$", ln)
        ]
        self.assertEqual(len(install_at), 1, "one wasm-pack install step expected")
        self.assertTrue(npm_ci_at, "js.yml must still run the root `npm ci`")
        self.assertLess(
            install_at[0],
            min(npm_ci_at),
            "wasm-pack must be installed BEFORE `npm ci`: the package's `prepare` "
            "lifecycle (sq-bkag) runs on `npm ci` and compiles the wasm engine with "
            "wasm-pack from PATH.",
        )


def _step_version(block: list[str]) -> str:
    """The wasm-pack version one step requests, or a DESCRIPTIVE SENTINEL when the
    step does not carry exactly one non-empty `with: version:` input.

    Only a `version:` that is a direct child of the step's `with:` mapping counts
    (see `_with_versions`): that is the one the action actually receives. A duplicate
    `with.version` is a sentinel too — YAML last-key-wins makes the effective pin
    ambiguous to a reader, and the repo-wide comparison should not have to guess.

    Returning a sentinel rather than nothing is what keeps the cross-workflow
    comparison honest. A step that LOST its `version:` installs whatever the action
    defaults to — the same unpinned-lane regression #5771 is about — but if such a
    step simply contributed no entry, the remaining lanes would still agree and both
    assertions below would pass. As a sentinel it reads as a pin unlike any other, so
    it reds `test_single_version_across_workflows` (and, being no `vX.Y.Z`,
    `test_every_step_pins_an_exact_version` too).
    """
    versions = _with_versions(block)
    exact = [v for v in versions if v]
    if len(exact) != 1 or len(versions) != 1:
        return f"<not exactly one `with: version:` input: {versions!r}>"
    return exact[0]


class WasmPackVersionUnified(unittest.TestCase):
    """(5) Every jetli/wasm-pack-action step in the repo asks for the SAME version,
    and every such step actually asks for one.

    ci.yml's `wasm` job RUNS the headless suites; js.yml BUILDS + packs the npm
    package's wasm artifact. If they skew, a wasm-pack/wasm-bindgen behaviour change is
    only ever exercised in the build lane (#5771). This pins the EQUALITY, not the
    value — bumping is fine, bumping one lane only is not. Dropping a lane's `version:`
    altogether is the same regression wearing a different hat (that lane silently takes
    the action's default), so it is caught too.
    """

    @staticmethod
    def _pins_from(sources: dict[str, str]) -> dict[str, list[str]]:
        """{workflow filename: [one entry PER wasm-pack step]}.

        Per-step, not flattened-over-values: a step with no `version:` must still
        occupy a slot (as a `_step_version` sentinel) instead of disappearing.
        Split out from `_pins` so the mutation guard below can feed it synthetic
        workflows without touching the tree.
        """
        found: dict[str, list[str]] = {}
        for name, text in sorted(sources.items()):
            if WASM_PACK_ACTION not in text:
                continue
            for block in gate.split_steps(text):
                if (gate._step_uses(block) or (None,))[0] != WASM_PACK_ACTION:
                    continue
                found.setdefault(name, []).append(_step_version(block))
        return found

    @classmethod
    def _pins(cls) -> dict[str, list[str]]:
        return cls._pins_from(
            {
                path.name: path.read_text(encoding="utf-8")
                for path in sorted(WORKFLOWS.glob("*.y*ml"))
            }
        )

    def test_both_wasm_lanes_use_the_action(self):
        """Anti-vacuity: the two lanes #5771 unified must still be in the sample."""
        pins = self._pins()
        for expected in ("ci.yml", "js.yml"):
            self.assertIn(
                expected,
                pins,
                f"{expected} must install wasm-pack via {WASM_PACK_ACTION} — without "
                "it the single-version assertion below has nothing to compare and "
                "passes vacuously (#5771).",
            )

    def test_every_step_pins_an_exact_version(self):
        """Every wasm-pack step names an exact release — an omitted `version:` takes
        the action's default, which is exactly the unpinned lane #5771 removed."""
        for name, versions in sorted(self._pins().items()):
            for i, version in enumerate(versions):
                self.assertRegex(
                    version,
                    VERSION_RE,
                    f"{name}: {WASM_PACK_ACTION} step #{i} must carry exactly one "
                    f"exact `version:` (e.g. v0.15.0), got {version!r}. A step "
                    "without one installs whatever the action defaults to, so the "
                    "lane is unpinned and the repo-wide version agreement below "
                    "means nothing for it (#5771).",
                )

    def test_single_version_across_workflows(self):
        pins = self._pins()
        distinct = {v for versions in pins.values() for v in versions}
        self.assertEqual(
            len(distinct),
            1,
            "all workflows must install the SAME wasm-pack version, found "
            f"{sorted(distinct)} across { {k: v for k, v in sorted(pins.items())} }. "
            "ci.yml's `wasm` job RUNS the headless suites and js.yml BUILDS the "
            "published artifact: if they skew, a wasm-pack/wasm-bindgen behaviour "
            "change is only ever exercised in the build lane (#5771). Bump every "
            "lane together.",
        )

    # --- mutation guard for the two assertions above -------------------------
    # Hermetic synthetic workflows (no tree mutation): `_MUT_BAD` is `_MUT_GOOD`
    # with the `version:` line deleted, i.e. the exact regression the reviewer
    # described.
    _MUT_GOOD = """\
jobs:
  wasm:
    steps:
      - name: Install wasm-pack
        uses: jetli/wasm-pack-action@0d096b08b4e5a7de8c28de67e11e945404e9eefa # v0.4.0
        with:
          version: v0.15.0
      - name: Test
        run: wasm-pack test --node
"""
    _MUT_BAD = """\
jobs:
  wasm:
    steps:
      - name: Install wasm-pack
        uses: jetli/wasm-pack-action@0d096b08b4e5a7de8c28de67e11e945404e9eefa # v0.4.0
      - name: Test
        run: wasm-pack test --node
"""

    # The step has NO `with: version:` (so wasm-pack-action takes its default) but does
    # carry an unrelated, correctly-named `version:` under a SIBLING mapping. A
    # block-wide "any line starting `version:`" scan reads v0.15.0 here and calls the
    # lane pinned; only the `with:`-scoped walk sees the omission.
    _MUT_UNRELATED_VERSION = """\
jobs:
  wasm:
    steps:
      - name: Install wasm-pack
        uses: jetli/wasm-pack-action@0d096b08b4e5a7de8c28de67e11e945404e9eefa # v0.4.0
        with:
          cache-key: wasm-pack
        env:
          version: v0.15.0
      - name: Test
        run: wasm-pack test --node
"""

    # Two direct `with: version:` keys: YAML last-key-wins, so which release the lane
    # actually installs is not what a reader (or the first match) sees.
    _MUT_DUPLICATE_VERSION = """\
jobs:
  wasm:
    steps:
      - name: Install wasm-pack
        uses: jetli/wasm-pack-action@0d096b08b4e5a7de8c28de67e11e945404e9eefa # v0.4.0
        with:
          version: v0.15.0
          version: v0.13.1
      - name: Test
        run: wasm-pack test --node
"""

    def test_mutation_dropping_a_lanes_version_is_caught(self):
        """Deleting one lane's `version:` must NOT leave both assertions green."""
        good = self._pins_from({"a.yml": self._MUT_GOOD, "b.yml": self._MUT_GOOD})
        self.assertEqual(good, {"a.yml": ["v0.15.0"], "b.yml": ["v0.15.0"]})
        self.assertEqual(len({v for vs in good.values() for v in vs}), 1)

        mutated = self._pins_from({"a.yml": self._MUT_GOOD, "b.yml": self._MUT_BAD})
        # The unpinned lane is still IN the sample (so the anti-vacuity check keeps
        # passing, as the reviewer noted) — it must therefore fail on its own terms.
        self.assertIn("b.yml", mutated)
        self.assertNotRegex(
            mutated["b.yml"][0],
            VERSION_RE,
            "a step with no `version:` must not read as an exact pin",
        )
        self.assertEqual(
            len({v for vs in mutated.values() for v in vs}),
            2,
            "a step with no `version:` must count as a DISTINCT pin, so the "
            "single-version assertion goes red instead of comparing the one "
            "surviving lane against itself",
        )

    def test_mutation_version_outside_with_is_not_a_pin(self):
        """A `version:` that the action never receives must not read as a pin."""
        for label, text in (
            ("unrelated nested `version:`", self._MUT_UNRELATED_VERSION),
            ("duplicate `with: version:`", self._MUT_DUPLICATE_VERSION),
        ):
            with self.subTest(mutation=label):
                mutated = self._pins_from(
                    {"a.yml": self._MUT_GOOD, "b.yml": text}
                )
                self.assertIn("b.yml", mutated)
                self.assertNotRegex(
                    mutated["b.yml"][0],
                    VERSION_RE,
                    f"{label}: the step does not unambiguously pass one `version:` "
                    "input to the action, so it must not read as an exact pin",
                )
                self.assertEqual(
                    len({v for vs in mutated.values() for v in vs}),
                    2,
                    f"{label}: must count as a DISTINCT pin so the single-version "
                    "assertion goes red",
                )


# --- (6) contributor-facing install hints track that pin (#5777) -------------

# The three places the repo tells a HUMAN how to put wasm-pack on PATH. They are
# prose and JS string literals, not shell steps, so no workflow-scoped checker sees
# them (which is why #5777 was filed rather than folded into an existing gate).
HINT_FILES = (
    Path("js") / "guardrails" / "prepare-build.mjs",
    Path("packages") / "eyereasoner-compat" / "guardrails" / "prepare-build.mjs",
    Path("js") / "README.md",
)

HINT_START = "cargo install wasm-pack"
# Whichever delimiter closes the literal carrying the command: a JS single quote in
# the .mjs guardrails, a markdown backtick in the README. Deliberately NOT a newline
# — markdown renders an inline code span wrapped across two source lines as one
# command, so a re-wrapped README must still be read as the command it displays.
HINT_DELIMS = "'\"`"
# An unterminated literal must not swallow the rest of the file.
HINT_WINDOW = 200
# cargo's `--version` takes a version REQUIREMENT: a bare `0.15.0` is the caret range
# ^0.15.0, i.e. still floating. Only the `=` form installs the release CI installs.
EXACT_VERSION_RE = re.compile(r"--version\s+=(\d+\.\d+\.\d+)(?:\s|$)")


def _hint_commands(text: str) -> list[tuple[str, str]]:
    """`(version-or-sentinel, normalised command)` per `cargo install wasm-pack`
    occurrence, in order.

    A sentinel rather than an omission, for the same reason as `_step_version`: a
    hint that lost its `--version` is precisely the #5777 regression, and if it
    simply contributed no entry the assertion below would pass vacuously.
    """
    out: list[tuple[str, str]] = []
    pos = 0
    while True:
        start = text.find(HINT_START, pos)
        if start < 0:
            return out
        pos = start + len(HINT_START)
        window = text[start : start + HINT_WINDOW]
        ends = [window.find(d) for d in HINT_DELIMS if d in window]
        if not ends:
            out.append((f"<unterminated hint literal: {window!r}>", window))
            continue
        command = " ".join(window[: min(ends)].split())
        match = EXACT_VERSION_RE.search(command)
        out.append(
            (
                match.group(1)
                if match
                else f"<no exact `--version =X.Y.Z`: {command!r}>",
                command,
            )
        )


class ContributorHintsMatchThePin(unittest.TestCase):
    """(6) Every contributor-facing `cargo install wasm-pack` hint names the SAME
    release the gating lanes install.

    Unversioned, those hints float to whatever wasm-pack's current release is, so a
    local `npm ci` builds the very bundle CI publishes with a wasm-pack — and the
    wasm-bindgen CLI that release carries — that no lane validates (#5777). The
    expected value is READ FROM the workflows rather than restated here, so bumping
    the action pin reds this test until the hints are bumped with it; nothing else in
    the tree would notice.

    SCOPE: human-facing hints only. Several workflows (gui.yml, pages.yml,
    publish.yml, nightly-full-sweep.yml) still `cargo install wasm-pack --locked`
    unversioned; converting those is separate work and this file's silence about
    them is not a claim that they are pinned.
    """

    @staticmethod
    def _canonical_pin() -> str:
        """The one version the workflows agree on, without its leading `v`."""
        distinct = {
            v for vs in WasmPackVersionUnified._pins().values() for v in vs
        }
        assert len(distinct) == 1, (
            "the workflows do not agree on one wasm-pack version, so there is no "
            f"canonical pin for the hints to track: {sorted(distinct)} "
            "(test_single_version_across_workflows reports this properly)"
        )
        pin = distinct.pop()
        assert VERSION_RE.match(pin), f"canonical pin is not an exact release: {pin!r}"
        return pin[1:]

    def test_hint_files_still_carry_a_hint(self):
        """Anti-vacuity: a file that stopped mentioning the command (renamed, reworded)
        must not silently drop out of the sample."""
        for rel in HINT_FILES:
            with self.subTest(file=str(rel)):
                path = REPO_ROOT / rel
                self.assertTrue(path.is_file(), f"{rel} must exist to be checked")
                self.assertTrue(
                    _hint_commands(path.read_text(encoding="utf-8")),
                    f"{rel} must still show a `{HINT_START}` command — if the hint "
                    "moved, point this test at its new home rather than deleting "
                    "the entry (#5777).",
                )

    def test_every_hint_pins_the_ci_version(self):
        expected = self._canonical_pin()
        for rel in HINT_FILES:
            text = (REPO_ROOT / rel).read_text(encoding="utf-8")
            for i, (version, command) in enumerate(_hint_commands(text)):
                with self.subTest(file=str(rel), hint=i):
                    self.assertEqual(
                        version,
                        expected,
                        f"{rel}: hint #{i} must install the wasm-pack CI installs — "
                        f"`{HINT_START} --locked --version ={expected}` (the "
                        f"`with: version:` pin on {WASM_PACK_ACTION} in the "
                        f"workflows). Got {version!r} from {command!r}. Unversioned, "
                        "a local build uses a wasm-pack — and bundled wasm-bindgen "
                        "CLI — that no lane validates (#5777). Bump the hints and "
                        "the workflows together.",
                    )

    # --- mutation guard ------------------------------------------------------
    # Each entry is a synthetic hint that MUST NOT read as the canonical pin, so the
    # assertion above cannot pass on a hint that floats.
    _MUT_HINTS = (
        ("unversioned (the #5777 bug)", "'    cargo install wasm-pack --locked',"),
        ("caret range, not exact", "'    cargo install wasm-pack --version 0.15.0',"),
        ("pins a DIFFERENT release", "'    cargo install wasm-pack --version =0.13.1',"),
        ("unterminated literal", "    cargo install wasm-pack --locked"),
    )

    def test_mutation_floating_hints_are_rejected(self):
        good = _hint_commands("'    cargo install wasm-pack --locked --version =0.15.0',")
        self.assertEqual([v for v, _ in good], ["0.15.0"], "the fixed form must parse")

        for label, text in self._MUT_HINTS:
            with self.subTest(mutation=label):
                found = _hint_commands(text)
                self.assertEqual(
                    len(found),
                    1,
                    f"{label}: the occurrence must still be COUNTED — a hint that "
                    "disappears from the sample passes the assertion vacuously",
                )
                self.assertNotEqual(
                    found[0][0],
                    "0.15.0",
                    f"{label}: must not read as the canonical pin, got {found[0]!r}",
                )

    def test_mutation_wrapped_markdown_span_still_parses(self):
        """A README re-wrap splits the inline code span across two source lines; it is
        still one command to a reader, so it must still be read as pinned (rather than
        reding the gate for a purely cosmetic edit)."""
        wrapped = "(`cargo install wasm-pack --locked\n--version =0.15.0`); without"
        self.assertEqual([v for v, _ in _hint_commands(wrapped)], ["0.15.0"])


if __name__ == "__main__":
    unittest.main(verbosity=2)
