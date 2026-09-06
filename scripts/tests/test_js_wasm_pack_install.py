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
# SCOPE — properties 1–4 are deliberately js.yml ONLY, because ordering (4) is a
# js.yml-specific obligation. The no-source-compile half of (1) is now repo-wide: see
# property 6.
#
# TWO CROSS-WORKFLOW properties are asserted, by WasmPackVersionUnified and
# NoSourceCompileOutsideAllowlist below:
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
#   6. NO SOURCE COMPILE REPO-WIDE, except a DECLARED allowlist. #5776 converted the
#      remaining 15 `cargo install wasm-pack` sites (gui.yml ×5, nightly-full-sweep.yml
#      ×4, publish.yml ×3, pages.yml, site-e2e-hero.yml, site-visual.yml) to the same
#      prebuilt action — they are all `ubuntu-latest` x86_64, the runner the action is
#      already proven green on in `js`/`ci`. release.yml's `gui-bundle` is NOT converted
#      (its matrix spans ubuntu-24.04-arm / macos-14 / macos-15-intel / windows-latest,
#      where the prebuilt-download path is unverified) and is the sole CARGO_INSTALL_
#      ALLOWLIST entry. The exemption is keyed at SITE granularity — (workflow, job id)
#      plus the exact number of source-install steps that job may contain — not by
#      filename: a file-keyed allowlist hands the whole of release.yml a free pass, so
#      a NEW `cargo install wasm-pack` in a sibling job (or a second one inside
#      `gui-bundle`) would be exempt too, which is the opposite of "exactly this one
#      site". Two directions are pinned, because they fail differently:
#      putting a lane BACK on `cargo install` reds test_no_source_compile_outside_
#      allowlist, while DELETING an install step outright leaves no `cargo install`
#      line to count and is caught only by test_converted_lanes_still_use_the_action.
#      (A partial revert — one of gui.yml's five steps — reds the first only, since
#      the file still reaches the action via its other four.) The allowlist is also
#      asserted LIVE and EXACTLY (test_allowlist_is_not_stale), so once
#      `gui-bundle` is converted the stale free pass must be deleted rather than rot.
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

# The source-compile command #5776 removed from every lane but one.
CARGO_INSTALL = "cargo install wasm-pack"

# {(workflow filename, job id): (how many source-install steps that job may run, why)}.
# Deliberately tiny: an entry is a standing exemption from the flake hardening, so it
# must name a REASON a reader can check, and it is asserted live and EXACTLY
# (test_allowlist_is_not_stale) so a converted lane's entry cannot linger as a silent
# free pass for a future regression.
#
# Keyed by SITE — (workflow, job) plus a step COUNT — rather than by filename. A
# filename key exempts every job in release.yml and any number of steps within them, so
# a new `cargo install wasm-pack` added to `docker`, or a second one added to
# `gui-bundle`, would inherit this pass silently; the count and the job id are what make
# "exactly this one site" mean what it says.
CARGO_INSTALL_ALLOWLIST = {
    ("release.yml", "gui-bundle"): (
        1,
        "the `gui-bundle` matrix also runs on ubuntu-24.04-arm, macos-14, "
        "macos-15-intel and windows-latest; jetli/wasm-pack-action's behaviour on "
        "those targets is unverified here, and a wrong-arch download would break a "
        "RELEASE bundle. Converting it needs a real matrix run (#5776 follow-up).",
    ),
}

# The lanes #5776 converted. Listed explicitly rather than derived, so that deleting a
# lane's install step (or reverting it to `cargo install`) is caught by name instead of
# silently shrinking the sample the version-unification assertion compares.
CONVERTED_LANES = (
    "gui.yml",
    "nightly-full-sweep.yml",
    "pages.yml",
    "publish.yml",
    "site-e2e-hero.yml",
    "site-visual.yml",
)


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


# A mapping key at the head of a stripped line, e.g. `gui-bundle:`.
_KEY_RE = re.compile(r"^([A-Za-z_][A-Za-z0-9_-]*):")
# Where a `cargo install` line sits that no `jobs:` mapping accounts for. Never
# allowlistable, so an install hidden outside the job tree still reds the guard.
NO_JOB = "<outside jobs:>"


def _install_sites(text: str) -> dict[str, int]:
    """{job id: number of live `cargo install wasm-pack` lines in that job}.

    Job granularity, not file granularity: the allowlist has to name the ONE job that
    may still source-compile, otherwise exempting release.yml exempts every job in it.
    The walk is indentation-based (stdlib only, no PyYAML): the first key
    under top-level `jobs:` fixes the job-id indent, and every later key at that same
    indent starts a new job. Comment lines are dropped first (`_code_lines`), so prose
    about the removed command never counts as a live step.
    """
    sites: dict[str, int] = {}
    in_jobs = False
    job_indent: int | None = None
    job: str | None = None
    for raw in _code_lines(text):
        ind = gate._indent(raw)
        key = raw.lstrip(" ")
        if not in_jobs:
            if ind == 0 and key.startswith("jobs:"):
                in_jobs = True
            elif CARGO_INSTALL in raw:
                sites[NO_JOB] = sites.get(NO_JOB, 0) + 1
            continue
        if ind == 0:
            # Dedented back out to a sibling top-level key (`permissions:`, …).
            in_jobs = False
            job_indent = None
            job = None
            if CARGO_INSTALL in raw:
                sites[NO_JOB] = sites.get(NO_JOB, 0) + 1
            continue
        m = _KEY_RE.match(key) if job_indent is None or ind == job_indent else None
        if m:
            job_indent = ind
            job = m.group(1)
        if CARGO_INSTALL in raw:
            name = job or NO_JOB
            sites[name] = sites.get(name, 0) + 1
    return sites


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


class NoSourceCompileOutsideAllowlist(unittest.TestCase):
    """(6) `cargo install wasm-pack` survives ONLY in a declared, reasoned allowlist,
    and every lane #5776 converted still reaches the prebuilt action.

    The regression is invisible in both directions — a lane that goes back to the
    source compile is green on a good day and reds an unrelated PR on a bad one — so
    both are pinned here rather than left to review to notice.
    """

    @staticmethod
    def _offenders_from(sources: dict[str, str]) -> dict[tuple[str, str], int]:
        """{(workflow filename, job id): number of live `cargo install wasm-pack`
        lines in that job}.

        Per (file, job) rather than per file, because that is the granularity the
        allowlist claims to exempt: a file-keyed count cannot distinguish `gui-bundle`'s
        one declared step from a second install added to another release.yml job.
        Comment lines are dropped first (`_code_lines`): this repo comments its
        workflows heavily and several of them DISCUSS the removed command in prose —
        counting those would make the assertion permanently red for the wrong reason.
        """
        found: dict[tuple[str, str], int] = {}
        for name, text in sorted(sources.items()):
            for job, hits in sorted(_install_sites(text).items()):
                found[(name, job)] = hits
        return found

    @classmethod
    def _offenders(cls) -> dict[tuple[str, str], int]:
        return cls._offenders_from(
            {
                path.name: path.read_text(encoding="utf-8")
                for path in sorted(WORKFLOWS.glob("*.y*ml"))
            }
        )

    def test_no_source_compile_outside_allowlist(self):
        unexpected = sorted(set(self._offenders()) - set(CARGO_INSTALL_ALLOWLIST))
        self.assertEqual(
            unexpected,
            [],
            f"{unexpected} run `{CARGO_INSTALL}`, which rebuilds wasm-pack and its "
            "whole chrono/wasm-bindgen source tree from crates.io on every run — a "
            "transient registry blip then fails the lane (sq-khm3f; it red-gated PR "
            f"#1131). Install the prebuilt binary via {WASM_PACK_ACTION} instead "
            "(#5776), or, if the lane genuinely cannot, add that (workflow, job) to "
            "CARGO_INSTALL_ALLOWLIST with its step count and the reason.",
        )

    def test_allowlist_is_not_stale(self):
        """Every exemption must still describe a REAL, current source compile — and
        exactly as many of them as it declares.

        Without the presence half, converting release.yml later would leave a dead entry
        behind that silently re-permits the regression it was written to bound. Without
        the COUNT half, `gui-bundle` could grow a second `cargo install wasm-pack` and
        stay green, because the site itself is allowlisted — the free pass would then be
        wider than the reason written beside it.
        """
        offenders = self._offenders()
        for site in sorted(CARGO_INSTALL_ALLOWLIST):
            expected = CARGO_INSTALL_ALLOWLIST[site][0]
            self.assertIn(
                site,
                offenders,
                f"CARGO_INSTALL_ALLOWLIST names {site}, but it no longer runs "
                f"`{CARGO_INSTALL}`. Delete the entry — a stale exemption is a "
                "standing free pass for the next regression (#5776).",
            )
            self.assertEqual(
                offenders[site],
                expected,
                f"CARGO_INSTALL_ALLOWLIST exempts {expected} `{CARGO_INSTALL}` "
                f"step(s) at {site}, found {offenders[site]}. The exemption covers "
                "exactly the site its reason describes; convert the extra one to "
                f"{WASM_PACK_ACTION}, or widen the entry and say why (#5776).",
            )

    def test_converted_lanes_still_use_the_action(self):
        """Anti-vacuity for (5) and the other half of (6): a lane that dropped its
        install step contributes no `cargo install` line either, so the assertion
        above would go quiet about it."""
        pins = WasmPackVersionUnified._pins()
        for name in CONVERTED_LANES:
            self.assertIn(
                name,
                pins,
                f"{name} must install wasm-pack via {WASM_PACK_ACTION} (#5776). If "
                "the lane legitimately no longer needs wasm-pack, drop it from "
                "CONVERTED_LANES in the same change.",
            )

    # --- mutation guard ------------------------------------------------------
    # Synthetic workflows, so the check runs without mutating the tree.
    _MUT_CONVERTED = """\
jobs:
  build:
    steps:
      - name: Install wasm-pack
        uses: jetli/wasm-pack-action@0d096b08b4e5a7de8c28de67e11e945404e9eefa # v0.4.0
        with:
          version: v0.15.0
      - name: Build
        run: npm run build:wasm
"""
    # The exact revert #5776 undoes.
    _MUT_REVERTED = """\
jobs:
  build:
    steps:
      - name: Install wasm-pack
        run: cargo install wasm-pack --locked
      - name: Build
        run: npm run build:wasm
"""
    # Prose ABOUT the command, which must not count as a live step — otherwise the
    # assertion reds on every workflow that documents why the command was removed.
    _MUT_COMMENT_ONLY = """\
jobs:
  build:
    steps:
      # Do NOT `cargo install wasm-pack` here — see #5776.
      - name: Install wasm-pack
        uses: jetli/wasm-pack-action@0d096b08b4e5a7de8c28de67e11e945404e9eefa # v0.4.0
        with:
          version: v0.15.0
"""

    # release.yml as the allowlist describes it: ONE source compile, in `gui-bundle`.
    _MUT_RELEASE_OK = """\
jobs:
  gui-bundle:
    steps:
      - name: Install wasm-pack
        run: cargo install wasm-pack --locked
  docker:
    steps:
      - name: Build
        run: docker build .
"""
    # The same file with a SECOND source compile in a sibling job. A filename-keyed
    # allowlist exempts release.yml wholesale and never sees this.
    _MUT_RELEASE_SECOND_JOB = """\
jobs:
  gui-bundle:
    steps:
      - name: Install wasm-pack
        run: cargo install wasm-pack --locked
  docker:
    steps:
      - name: Install wasm-pack
        run: cargo install wasm-pack --locked
      - name: Build
        run: docker build .
"""
    # …and with the extra compile DUPLICATED inside the allowlisted job itself, which a
    # site key alone still admits — only the declared step count rejects it.
    _MUT_RELEASE_DUPLICATE_IN_JOB = """\
jobs:
  gui-bundle:
    steps:
      - name: Install wasm-pack
        run: cargo install wasm-pack --locked
      - name: Install wasm-pack (again)
        run: cargo install wasm-pack --locked
  docker:
    steps:
      - name: Build
        run: docker build .
"""

    def test_mutation_reverting_a_lane_is_caught(self):
        """A converted lane put back on `cargo install` must be seen as an offender."""
        clean = self._offenders_from({"gui.yml": self._MUT_CONVERTED})
        self.assertEqual(clean, {}, "the converted shape must not read as an offender")

        reverted = self._offenders_from({"gui.yml": self._MUT_REVERTED})
        self.assertEqual(reverted, {("gui.yml", "build"): 1})
        self.assertEqual(
            sorted(set(reverted) - set(CARGO_INSTALL_ALLOWLIST)),
            [("gui.yml", "build")],
            "a revert in a non-allowlisted lane must survive the allowlist "
            "subtraction, i.e. red test_no_source_compile_outside_allowlist",
        )

    def test_mutation_second_release_site_is_caught(self):
        """The exemption is ONE site, not the whole workflow: a second source install
        elsewhere in release.yml — or a duplicate inside `gui-bundle` — must go red.

        This is the hole a filename-keyed allowlist leaves open, so it is executed
        rather than argued: the baseline shape passes both assertions, each mutation
        fails one.
        """
        site = ("release.yml", "gui-bundle")
        self.assertIn(site, CARGO_INSTALL_ALLOWLIST, "baseline for this mutation")

        ok = self._offenders_from({"release.yml": self._MUT_RELEASE_OK})
        self.assertEqual(ok, {site: 1}, "the declared shape must read as one site")
        self.assertEqual(sorted(set(ok) - set(CARGO_INSTALL_ALLOWLIST)), [])
        self.assertEqual(ok[site], CARGO_INSTALL_ALLOWLIST[site][0])

        # Mutation A: a second job in the SAME allowlisted workflow.
        second = self._offenders_from({"release.yml": self._MUT_RELEASE_SECOND_JOB})
        self.assertEqual(second, {site: 1, ("release.yml", "docker"): 1})
        self.assertEqual(
            sorted(set(second) - set(CARGO_INSTALL_ALLOWLIST)),
            [("release.yml", "docker")],
            "a source install in a non-allowlisted job of an allowlisted workflow "
            "must survive the allowlist subtraction, i.e. red "
            "test_no_source_compile_outside_allowlist",
        )

        # Mutation B: a duplicate inside the allowlisted job — the site key still
        # matches, so only the declared count can reject it.
        dup = self._offenders_from({"release.yml": self._MUT_RELEASE_DUPLICATE_IN_JOB})
        self.assertEqual(sorted(set(dup) - set(CARGO_INSTALL_ALLOWLIST)), [])
        self.assertNotEqual(
            dup[site],
            CARGO_INSTALL_ALLOWLIST[site][0],
            "a duplicated source install inside the allowlisted job must not match "
            "the declared step count, i.e. red test_allowlist_is_not_stale",
        )

    def test_mutation_commented_out_command_is_not_a_live_step(self):
        """The counter reads STEPS, not prose — this file and several workflows quote
        the removed command in comments."""
        self.assertEqual(
            self._offenders_from({"gui.yml": self._MUT_COMMENT_ONLY}),
            {},
            "a `#`-commented mention of the command must not count as a live step",
        )


if __name__ == "__main__":
    unittest.main(verbosity=2)
