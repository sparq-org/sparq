#!/usr/bin/env python3
# [SONNET-4.6] sq-bqjv — Dependabot ecosystem-coverage guard.
#
# The production-readiness program (sq-bqjv) adopts prod-solid-server's supply-chain
# practices, of which "Dependabot covers every ecosystem the repo actually ships" is
# one. That property is invisible when it breaks: a Dependabot entry aimed at a
# directory with no resolvable manifest/lock does not error, it simply produces no
# PRs — which is indistinguishable from "no updates available". This repo had exactly
# that failure: the npm entry pointed at the workspace MEMBER `/js`, whose lock is
# deliberately gitignored, so `site` / `gui/*` / `packages/*` went uncovered, and the
# two digest-pinned Dockerfiles had no docker entry at all.
#
# So this asserts coverage STRUCTURALLY against the live repo rather than trusting
# the config to be read:
#   * every TRACKED npm lock root and every TRACKED Dockerfile directory is either
#     covered by a matching Dependabot entry or carries a written exclusion;
#   * exclusions stay HONEST — an exclusion whose path no longer exists is a failure,
#     so the allowlist cannot rot into a silent blanket, and a docker exclusion's
#     stated PREMISE (consumes only this repo's own floating image, so there is no
#     third-party pin to refresh) is checked against the Dockerfile's real `FROM`
#     rather than taken on the reason string's word;
#   * the specific regression above cannot return (npm is aimed at the workspaces
#     root, not at a member whose lock is gitignored).
#
# Tracked-ness comes from `git ls-files`, not a filesystem walk: a local `npm install`
# materialises the gitignored per-member locks, and a walk would then demand coverage
# for a file that does not exist for Dependabot.
#
# Stdlib only (re/subprocess/pathlib/unittest) — matches the other Python checks in
# .github/workflows/supply-chain.yml, which runs no pip install.
#
# Run:  python3 scripts/tests/test_dependabot_coverage.py

from __future__ import annotations

import re
import subprocess
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
CONFIG = REPO_ROOT / ".github" / "dependabot.yml"

# Directories deliberately NOT given a Dependabot entry, each with the reason it is
# out of scope. Every path here is asserted to still exist (see
# test_exclusions_are_not_stale), so a stale entry fails loudly instead of silently
# widening into a blanket exemption.
EXCLUDED_NPM_ROOTS = {
    "/bench/wasm-compare/browser": (
        "self-contained benchmark harness comparing third-party WASM engines; its "
        "pinned deps are the measurement subject, so bumping them would invalidate "
        "comparisons rather than remediate risk. Not shipped to users."
    ),
    "/crates/sparq-shacl/tests/diff_fuzz": (
        "differential-fuzz oracle harness; the pinned third-party SHACL "
        "implementation IS the oracle being differentiated against, so an "
        "automatic bump silently changes the reference. Not shipped to users."
    ),
}

# Registry namespace this repo publishes its own images under. A base outside it is
# third-party by definition, and a third-party base is exactly what Dependabot exists
# to refresh — so it cannot be excluded.
FIRST_PARTY_IMAGE_PREFIX = "ghcr.io/sparq-org/"

# Each docker exclusion declares the SINGLE base image that makes it safe, not just a
# prose reason. `base` is checked both ways round (see _docker_exclusion_problems):
# the declared value must itself be first-party and floating, and the Dockerfile's
# actual `FROM` set must be exactly it.
EXCLUDED_DOCKER_DIRS = {
    "/deploy/paas/lws": {
        "base": "ghcr.io/sparq-org/sparq-lws-core:latest",
        "reason": (
            "Railway config-as-code shim that consumes this repo's OWN published image. "
            "The base is first-party and the floating tag IS the deploy mechanism, so "
            "there is no third-party pin to refresh; the upstream digest is governed by "
            "/crates/sparq-lws-core."
        ),
    },
    "/deploy/paas/sparq-server": {
        "base": "ghcr.io/sparq-org/sparq-server:latest",
        "reason": (
            "Railway config-as-code shim over this repo's OWN published image — "
            "first-party base, floating tag by design; the upstream digest is governed "
            "by the root Dockerfile."
        ),
    },
}

# The npm workspaces root. Member manifests are walked off the root lock, so members
# must NOT get their own entries (their locks are gitignored — see .gitignore).
NPM_WORKSPACES_ROOT = "/"


def _git_tracked(*patterns: str) -> list[str]:
    """Repo-relative paths of tracked files matching the given pathspecs."""
    out = subprocess.run(
        ["git", "-C", str(REPO_ROOT), "ls-files", "-z", "--", *patterns],
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    return [p for p in out.split("\0") if p]


def _as_dir(rel_path: str) -> str:
    """'site/package-lock.json' -> '/site'; 'Dockerfile' -> '/'."""
    parent = str(Path(rel_path).parent)
    return "/" if parent == "." else "/" + parent


def parse_update_entries(text: str) -> list[dict]:
    """Extract (package-ecosystem, directory) from each `updates:` entry.

    Deliberately narrow: it matches only the two keys at their exact nesting depth,
    so comment prose (which in this file quotes both key names) cannot be mistaken
    for configuration. A parse that silently yields nothing is caught by
    test_parser_sees_every_entry.
    """
    entries: list[dict] = []
    current: dict | None = None
    eco_re = re.compile(r'^  - package-ecosystem:\s*["\']?([\w-]+)["\']?\s*(?:#.*)?$')
    dir_re = re.compile(r'^    directory:\s*["\']?(\S*?)["\']?\s*(?:#.*)?$')
    for line in text.splitlines():
        m = eco_re.match(line)
        if m:
            current = {"ecosystem": m.group(1), "directory": None}
            entries.append(current)
            continue
        m = dir_re.match(line)
        if m and current is not None:
            current["directory"] = m.group(1)
    return entries


def parse_from_images(text: str) -> list[str]:
    """Image reference of every `FROM` directive, in file order.

    Per-stage flags (`--platform=…`) and stage aliases (`AS builder`) are stripped;
    the reference is returned verbatim so a digest pin or a foreign registry stays
    visible to the caller.
    """
    from_re = re.compile(r"^\s*FROM\s+(?:--\S+\s+)*(\S+)", re.IGNORECASE)
    return [m.group(1) for m in map(from_re.match, text.splitlines()) if m]


def _docker_exclusion_problems(rel: str, spec: dict, text: str) -> list[str]:
    """Reasons the docker exclusion of `rel` is NOT justified by its Dockerfile.

    The premise of every entry in EXCLUDED_DOCKER_DIRS is "this file consumes exactly
    one of this repo's own floating images, so Dependabot has no third-party pin to
    refresh here". Both halves are enforced, which is what stops the table from being
    edited into a blanket exemption: the DECLARED base must be first-party and
    un-pinned, AND the file's real `FROM` set must be exactly that one base. Empty
    list means the exclusion still holds.
    """
    problems: list[str] = []
    declared = spec["base"]
    if not declared.startswith(FIRST_PARTY_IMAGE_PREFIX):
        problems.append(
            "%s: declared base %r is not under %s, so it is third-party and the "
            "'nothing for Dependabot to refresh' premise fails"
            % (rel, declared, FIRST_PARTY_IMAGE_PREFIX)
        )
    if "@" in declared:
        problems.append(
            "%s: declared base %r is digest-pinned, and a pin is precisely what "
            "Dependabot must refresh" % (rel, declared)
        )
    found = parse_from_images(text)
    if found != [declared]:
        problems.append(
            "%s: Dockerfile FROM images %r are not exactly the declared base [%r]; "
            "the exclusion's stated premise no longer describes the file"
            % (rel, found, declared)
        )
    return problems


class TestDependabotCoverage(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.text = CONFIG.read_text(encoding="utf-8")
        cls.entries = parse_update_entries(cls.text)
        cls.covered = {
            (e["ecosystem"], e["directory"])
            for e in cls.entries
            if e["directory"] is not None
        }

    # --- parser sanity: a silently-broken parser must not pass everything ---------
    def test_parser_sees_every_entry(self):
        """Entry count from the parser must equal the raw key count in the file."""
        raw = len(re.findall(r"^  - package-ecosystem:", self.text, re.MULTILINE))
        self.assertGreater(raw, 0, "no `updates:` entries found in dependabot.yml")
        self.assertEqual(
            len(self.entries),
            raw,
            "parser dropped entries — the coverage assertions below would be vacuous",
        )
        for e in self.entries:
            self.assertIsNotNone(
                e["directory"],
                "entry for ecosystem %r has no `directory:` at the expected depth"
                % e["ecosystem"],
            )

    # --- npm ---------------------------------------------------------------------
    def test_every_tracked_npm_lock_is_covered_or_excluded(self):
        # Both pathspecs are needed: git's `**/` requires a directory prefix, so it
        # does not match the repo-root lock.
        roots = {
            _as_dir(p)
            for p in _git_tracked("**/package-lock.json", "package-lock.json")
        }
        self.assertIn(
            NPM_WORKSPACES_ROOT,
            roots,
            "the repo-root package-lock.json is untracked — this guard's premise is gone",
        )
        for root in sorted(roots):
            if root in EXCLUDED_NPM_ROOTS:
                continue
            self.assertIn(
                ("npm", root),
                self.covered,
                "npm lock at %s has no Dependabot entry and no recorded exclusion "
                "in EXCLUDED_NPM_ROOTS" % root,
            )

    def test_npm_targets_workspaces_root_not_a_member(self):
        """Regression guard for the sq-bqjv defect.

        A member directory (js, site, gui/*, packages/*) has no tracked lock — an npm
        entry aimed there resolves nothing and produces no PRs, silently.
        """
        npm_dirs = {d for eco, d in self.covered if eco == "npm"}
        self.assertIn(
            NPM_WORKSPACES_ROOT,
            npm_dirs,
            "npm must be aimed at the workspaces root so members are walked off the root lock",
        )
        members = {
            "/" + str(Path(p).parent)
            for p in _git_tracked("**/package.json")
            if str(Path(p).parent) != "."
        }
        for d in sorted(npm_dirs):
            if d in EXCLUDED_NPM_ROOTS or d == NPM_WORKSPACES_ROOT:
                continue
            self.assertNotIn(
                d,
                members,
                "npm entry points at workspace member %s, whose package-lock.json is "
                "gitignored; Dependabot has nothing to resolve there. Cover it via the "
                "workspaces root instead." % d,
            )

    # --- docker ------------------------------------------------------------------
    def test_every_tracked_dockerfile_is_covered(self):
        dirs = {
            _as_dir(p)
            for p in _git_tracked("**/Dockerfile", "Dockerfile", "**/Dockerfile.*")
        }
        self.assertTrue(dirs, "no tracked Dockerfile found — premise of this guard is gone")
        for d in sorted(dirs):
            if d in EXCLUDED_DOCKER_DIRS:
                continue
            self.assertIn(
                ("docker", d),
                self.covered,
                "Dockerfile in %s has no docker Dependabot entry, so its digest-pinned "
                "base image is never refreshed for security rebuilds" % d,
            )

    # --- exclusions stay honest ---------------------------------------------------
    def test_exclusions_are_not_stale(self):
        for rel, reason in EXCLUDED_NPM_ROOTS.items():
            path = REPO_ROOT / rel.lstrip("/") / "package-lock.json"
            self.assertTrue(
                path.exists(),
                "stale exclusion %s: the lock is gone, so the entry now only widens "
                "the exemption. Remove it." % rel,
            )
            self.assertTrue(reason.strip(), "exclusion %s must carry a reason" % rel)
        for rel, spec in EXCLUDED_DOCKER_DIRS.items():
            path = REPO_ROOT / rel.lstrip("/") / "Dockerfile"
            self.assertTrue(
                path.exists(),
                "stale exclusion %s: the Dockerfile is gone, so the entry now only "
                "widens the exemption. Remove it." % rel,
            )
            self.assertTrue(
                spec["reason"].strip(), "exclusion %s must carry a reason" % rel
            )

    def test_docker_exclusions_still_consume_only_a_first_party_floating_base(self):
        """The premise, checked against the file — not against the reason string.

        Without this, swapping a third-party or digest-pinned base into an excluded
        Dockerfile would leave it silently outside Dependabot while every other
        assertion here stayed green.
        """
        for rel, spec in EXCLUDED_DOCKER_DIRS.items():
            text = (REPO_ROOT / rel.lstrip("/") / "Dockerfile").read_text(
                encoding="utf-8"
            )
            self.assertEqual(
                [],
                _docker_exclusion_problems(rel, spec, text),
                "docker exclusion %s no longer holds: give the directory a docker "
                "Dependabot entry, or fix the base" % rel,
            )

    # --- the exclusion check itself is not vacuous --------------------------------
    def test_guard_rejects_a_third_party_pinned_base_in_the_file(self):
        spec = {"base": "ghcr.io/sparq-org/sparq-server:latest", "reason": "x"}
        problems = _docker_exclusion_problems(
            "/deploy/paas/sparq-server", spec, "FROM debian@sha256:%s\n" % ("0" * 64)
        )
        self.assertTrue(
            problems,
            "replacing the first-party floating base with a third-party pinned one "
            "must fail the exclusion guard",
        )

    def test_guard_rejects_blessing_a_third_party_base_in_the_table(self):
        """Editing EXCLUDED_DOCKER_DIRS to match a foreign base must not silence it."""
        base = "debian@sha256:%s" % ("0" * 64)
        problems = _docker_exclusion_problems(
            "/x", {"base": base, "reason": "x"}, "FROM %s\n" % base
        )
        self.assertTrue(problems, "a third-party digest-pinned base must be excludable")

    def test_guard_rejects_an_extra_stage(self):
        spec = {"base": "ghcr.io/sparq-org/sparq-server:latest", "reason": "x"}
        problems = _docker_exclusion_problems(
            "/x", spec, "FROM alpine AS build\nFROM %s\n" % spec["base"]
        )
        self.assertTrue(
            problems, "a smuggled-in third-party build stage must fail the guard"
        )


if __name__ == "__main__":
    unittest.main(verbosity=2)
