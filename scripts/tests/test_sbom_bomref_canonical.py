#!/usr/bin/env python3
# [OPUS-4.8] sq-9gli (GS-6 / F-6): hermetic regression test for the BOM-REF half of
# scripts/sbom-normalize.jq. Authored by Opus 4.8 (Fable unavailable; flag for
# re-review when Fable returns).
#
# WHY THIS, ON TOP OF test_sbom_purl_canonical.py:
# GS-6 (F-6, sq-toze.30) is specifically that cargo-cyclonedx 0.5.9 stamps the absolute
# build-machine path into the SBOM ROOT COMPONENT's `bom-ref` (and every workspace/path
# dependency `bom-ref` + `dependencies[].ref`/`dependsOn[]` edge) as
#     path+file:///abs/build/dir/crates/<name>#<version>[ <suffix>]
# which would leak the CI runner's filesystem layout into the PUBLISHED SBOM. The
# load-bearing control is scripts/sbom-normalize.jq's `canon_ref`, which rewrites every
# such ref to the canonical, host-independent `pkg:cargo/<name>@<version>[<suffix>]` and
# rewrites the dependency graph in lock-step.
#
# That `bom-ref` rewrite had NO dedicated test: the existing sbom self-test
# (test_sbom_purl_canonical.py) + its CI backstop (check-sbom-purl-canonical.py) assert
# only `purl` canonicality, and the only `bom-ref` guard is a coarse
# `grep -qE 'path+file://|/home/'` tripwire inside gen-sbom-vex.sh / supply-chain.yml.
# That tripwire would still "pass" if a future cargo-cyclonedx bump caused canon_ref to
# DROP or MANGLE a ref (no host path present, but the SBOM's internal reference graph
# silently broken / the root component mis-identified). This test pins the actual
# contract: the root bom-ref is rewritten to the CORRECT canonical identity, nested
# build-target suffixes are preserved, every dependency edge is rewritten in lock-step,
# registry refs are untouched, no host path survives, and the transform is idempotent.
#
# Hermetic w.r.t. git/network/cargo: drives the REAL scripts/sbom-normalize.jq (via jq)
# over an in-tmpdir fixture carrying the exact 0.5.9 `path+file://...#ver` shapes. A final
# test generates the REAL workspace SBOM and asserts no bom-ref/ref/dependsOn leaks a host
# path; it is SKIPPED (not failed) if cargo-cyclonedx / jq are unavailable, so the suite
# stays runnable in a bare environment while still proving the live invariant on CI.
#
# Run:  python3 scripts/tests/test_sbom_bomref_canonical.py
# (stdlib only; no pytest required — also discoverable by `pytest`.)

from __future__ import annotations

import json
import re
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
NORMALIZE_JQ = REPO_ROOT / "scripts" / "sbom-normalize.jq"

# [OPUS-4.8] sq-90ew: `cargo cyclonedx` internally runs `cargo metadata`, whose crates.io
# fetch is TRANSIENTLY flaky on hosted runners (observed `curl ... [16] Error in the HTTP2
# framing layer` -> non-zero exit at ~9-12s). PR #750 wrapped the cyclonedx invocation in the
# WORKFLOW STEPS, but this live-SBOM self-test (run in the GATING GS-6/GS-7 job) executes its
# OWN `cargo cyclonedx` BEFORE those wrapped steps, so the flake could still fail an unrelated
# PR (same root cause as the supplier/purl self-tests, main run #27810325920, post-#750).
# Mirror the #750 shell retry here: bounded 3 attempts, fixed 10s sleep, retry ONLY the
# GENERATION subprocess. This wraps NO assertion — a real GS-6 bom-ref/host-path violation is
# judged AFTER generation, on the produced files, and still fails. A genuine outage (all 3
# attempts fail) re-raises -> the test errors.
_CYCLONEDX_ATTEMPTS = 3
_CYCLONEDX_RETRY_SLEEP_S = 10


def _generate_workspace_sbom() -> None:
    """Run `cargo cyclonedx --all` (writes <crate>.cdx.json next to each Cargo.toml), with a
    bounded retry around ONLY the transient crates.io-fetch flake. Re-raises the last
    CalledProcessError if every attempt fails (a genuine outage / toolchain break)."""
    last_exc: subprocess.CalledProcessError | None = None
    for attempt in range(1, _CYCLONEDX_ATTEMPTS + 1):
        try:
            subprocess.run(
                ["cargo", "cyclonedx", "--all", "--format", "json", "--spec-version", "1.5"],
                cwd=REPO_ROOT, check=True, capture_output=True,
            )
            return
        except subprocess.CalledProcessError as exc:
            last_exc = exc
            if attempt >= _CYCLONEDX_ATTEMPTS:
                break
            print(f"cargo cyclonedx attempt {attempt} failed; retrying in "
                  f"{_CYCLONEDX_RETRY_SLEEP_S}s...", file=sys.stderr)
            time.sleep(_CYCLONEDX_RETRY_SLEEP_S)
    assert last_exc is not None
    raise last_exc

# A ref that still embeds a build-machine path. Mirrors gen-sbom-vex.sh's belt-and-braces
# guard, but we assert it over the parsed ref STRINGS, not a raw grep, so a structurally
# broken (dropped/mangled) ref is also visible to the lock-step / identity assertions.
HOST_PATH = re.compile(r"path\+file://|download_url=file://|/home/|/Users/|/runner|/builds?/")

# Canonical cargo bom-ref/ref: pkg:cargo/<name>@<version>, optionally with a trailing
# build-target suffix (e.g. " bin-target-0"). Registry refs keep their own form and are
# matched separately.
CANONICAL_REF = re.compile(r"^pkg:cargo/[^@]+@[^?#]+( \S.*)?$")
REGISTRY_REF = re.compile(r"^registry\+https://")


def _jq_available() -> bool:
    return shutil.which("jq") is not None


def _normalize(doc: dict) -> dict:
    """Run the REAL scripts/sbom-normalize.jq over `doc` via jq and return the result."""
    out = subprocess.run(
        ["jq", "-f", str(NORMALIZE_JQ)],
        input=json.dumps(doc),
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    return json.loads(out)


def _walk_refs(node: object) -> list[str]:
    """Every bom-ref / dependencies[].ref / dependsOn[] string anywhere in the doc."""
    out: list[str] = []
    if isinstance(node, dict):
        for key in ("bom-ref", "ref"):
            v = node.get(key)
            if isinstance(v, str):
                out.append(v)
        for v in node.get("dependsOn", []) or []:
            if isinstance(v, str):
                out.append(v)
        for k, v in node.items():
            if k == "dependsOn":
                continue
            out.extend(_walk_refs(v))
    elif isinstance(node, list):
        for v in node:
            out.extend(_walk_refs(v))
    return out


# The exact 0.5.9 shapes (root component + nested build target + workspace member +
# dependency edges), with a CI-runner absolute build path stamped in.
def _leaky_sbom() -> dict:
    abs_cli = "path+file:///home/runner/work/sparq/sparq/crates/sparq-cli#0.1.0"
    abs_core = "path+file:///home/runner/work/sparq/sparq/crates/sparq-core#0.1.0"
    reg_serde = "registry+https://github.com/rust-lang/crates.io-index#serde@1.0.0"
    # [FABLE-5] sq-gg0qq.2 (GS-6): the exact 0.5.9 GIT-dependency shape (sparq-lws-core's
    # pinned solid-oidc-verifier) — bom-ref carries the rev query + a bare-version fragment,
    # the purl a vcs_url qualifier. Both must canonicalise; supplier must derive (GS-1).
    git_sov = (
        "git+https://github.com/jeswr/solid-oidc-verifier"
        "?rev=89c896249a726398b78302fd2f65eef0a82af681#0.1.0"
    )
    return {
        "bomFormat": "CycloneDX",
        "specVersion": "1.5",
        "metadata": {
            "component": {
                "type": "application",
                "name": "sparq-cli",
                "version": "0.1.0",
                "bom-ref": abs_cli,
                "purl": "pkg:cargo/sparq-cli@0.1.0?download_url=file://.",
                "components": [
                    {
                        "type": "application",
                        "name": "sparq-cli",
                        "bom-ref": abs_cli + " bin-target-0",
                        "purl": "pkg:cargo/sparq-cli@0.1.0?download_url=file://.#src/main.rs",
                    }
                ],
            }
        },
        "components": [
            {
                "type": "library",
                "name": "sparq-core",
                "version": "0.1.0",
                "bom-ref": abs_core,
                "purl": "pkg:cargo/sparq-core@0.1.0?download_url=file://../sparq-core",
            },
            {
                "type": "library",
                "name": "serde",
                "version": "1.0.0",
                "bom-ref": reg_serde,
                "purl": "pkg:cargo/serde@1.0.0",
            },
            {
                "type": "library",
                "name": "solid-oidc-verifier",
                "version": "0.1.0",
                "bom-ref": git_sov,
                "purl": (
                    "pkg:cargo/solid-oidc-verifier@0.1.0?vcs_url="
                    "git%2Bhttps://github.com/jeswr/solid-oidc-verifier"
                    "%4089c896249a726398b78302fd2f65eef0a82af681"
                ),
            },
        ],
        "dependencies": [
            {"ref": abs_cli, "dependsOn": [abs_core, reg_serde, git_sov]},
            {"ref": abs_core, "dependsOn": [reg_serde]},
            {"ref": git_sov, "dependsOn": [reg_serde]},
        ],
    }


@unittest.skipUnless(_jq_available(), "jq not available")
class TestBomRefNormalization(unittest.TestCase):
    """Drive the real scripts/sbom-normalize.jq over the leaky fixture and pin the GS-6
    bom-ref contract."""

    def setUp(self) -> None:
        self.norm = _normalize(_leaky_sbom())

    def test_no_ref_leaks_a_host_path(self):
        # The headline GS-6 invariant: no bom-ref / ref / dependsOn carries a build path.
        leaks = [r for r in _walk_refs(self.norm) if HOST_PATH.search(r)]
        self.assertEqual(leaks, [], f"host path leaked into ref(s): {leaks}")

    def test_root_component_bomref_is_canonical_identity(self):
        # The bead's literal subject: the ROOT component ref. It must become the canonical
        # cargo identity for sparq-cli@0.1.0 — not dropped, not mangled to something else.
        root = self.norm["metadata"]["component"]["bom-ref"]
        self.assertEqual(root, "pkg:cargo/sparq-cli@0.1.0")

    def test_explicit_package_identity_differs_from_directory(self):
        # [GPT-6] Real detached member refs use #name@version, including targets.
        raw = "path+file:///build/zk/sparql-evaluator/host#sparq-proved-evaluator@0.1.0"
        canonical = "pkg:cargo/sparq-proved-evaluator@0.1.0"
        doc = {"components": [{"name": "sparq-proved-evaluator", "bom-ref": raw,
                               "components": [{"name": "sparq_proved_evaluator",
                                               "bom-ref": raw + " bin-target-0"}]}],
               "dependencies": [{"ref": raw, "dependsOn": [raw + " bin-target-0"]}]}
        result = _normalize(doc)
        self.assertEqual(result["components"][0]["bom-ref"], canonical)
        self.assertEqual(result["components"][0]["components"][0]["bom-ref"],
                         canonical + " bin-target-0")
        self.assertEqual(result["dependencies"],
                         [{"ref": canonical, "dependsOn": [canonical + " bin-target-0"]}])
        self.assertEqual(_normalize(result), result)

    def test_nested_build_target_suffix_is_preserved(self):
        # canon_ref preserves the trailing build-target suffix while stripping the path.
        sub = self.norm["metadata"]["component"]["components"][0]["bom-ref"]
        self.assertEqual(sub, "pkg:cargo/sparq-cli@0.1.0 bin-target-0")

    def test_workspace_member_bomref_is_canonical(self):
        core = next(c for c in self.norm["components"] if c["name"] == "sparq-core")
        self.assertEqual(core["bom-ref"], "pkg:cargo/sparq-core@0.1.0")

    def test_git_dep_bomref_purl_and_supplier(self):
        # [FABLE-5] sq-gg0qq.2: a git dep canonicalises like everything else (GS-6/GS-7)
        # and gets the honestly-derivable repository-owner supplier (GS-1).
        sov = next(
            c for c in self.norm["components"] if c["name"] == "solid-oidc-verifier"
        )
        self.assertEqual(sov["bom-ref"], "pkg:cargo/solid-oidc-verifier@0.1.0")
        self.assertEqual(sov["purl"], "pkg:cargo/solid-oidc-verifier@0.1.0")
        self.assertEqual(sov["supplier"]["name"], "Jesse Wright")
        self.assertEqual(
            sov["supplier"]["url"],
            ["https://github.com/jeswr/solid-oidc-verifier"],
        )
        # The dependency graph is rewritten in lock-step: the git ref appears canonical
        # both as an edge source and inside dependsOn.
        refs = {d["ref"] for d in self.norm["dependencies"]}
        self.assertIn("pkg:cargo/solid-oidc-verifier@0.1.0", refs)
        all_depends = [r for d in self.norm["dependencies"] for r in d.get("dependsOn", [])]
        self.assertIn("pkg:cargo/solid-oidc-verifier@0.1.0", all_depends)
        self.assertNotIn(True, ["git+" in r for r in _walk_refs(self.norm)])

    def test_registry_ref_is_untouched(self):
        # Registry deps are already host-independent and must pass through verbatim.
        serde = next(c for c in self.norm["components"] if c["name"] == "serde")
        self.assertEqual(
            serde["bom-ref"],
            "registry+https://github.com/rust-lang/crates.io-index#serde@1.0.0",
        )

    def test_dependency_graph_rewritten_in_lockstep(self):
        # Every ref/dependsOn edge must be rewritten so the graph still resolves: each
        # ref is now the canonical cargo form (suffix-free) or an untouched registry ref.
        deps = {d["ref"]: d.get("dependsOn", []) for d in self.norm["dependencies"]}
        self.assertIn("pkg:cargo/sparq-cli@0.1.0", deps)
        self.assertIn("pkg:cargo/sparq-core@0.1.0", deps)
        self.assertEqual(
            sorted(deps["pkg:cargo/sparq-cli@0.1.0"]),
            sorted(
                [
                    "pkg:cargo/sparq-core@0.1.0",
                    "registry+https://github.com/rust-lang/crates.io-index#serde@1.0.0",
                    # [FABLE-5] sq-gg0qq.2: the git dep, canonicalised in lock-step too.
                    "pkg:cargo/solid-oidc-verifier@0.1.0",
                ]
            ),
        )
        for ref in deps:
            self.assertTrue(
                CANONICAL_REF.match(ref) or REGISTRY_REF.match(ref),
                f"dependency ref is neither canonical cargo nor registry: {ref!r}",
            )

    def test_idempotent(self):
        # canon_ref's canonical form contains no path+file:// so a second pass is a no-op:
        # normalizing the already-normalized doc must be byte-identical.
        once = json.dumps(self.norm, sort_keys=True)
        twice = json.dumps(_normalize(self.norm), sort_keys=True)
        self.assertEqual(once, twice)


class TestLiveWorkspaceSBOM(unittest.TestCase):
    """Generate the REAL normalized workspace SBOM and assert no ref leaks a host path —
    the exact GS-6 invariant the publication path must hold. Skipped if toolchain absent."""

    def test_real_normalized_sbom_has_no_host_path_in_refs(self):
        if not shutil.which("cargo-cyclonedx") or not _jq_available():
            self.skipTest("cargo-cyclonedx / jq not available")
        # [OPUS-4.8] sq-90ew: GENERATION only, with the transient-flake retry. The GS-6
        # host-path / bom-ref assertions below run ONCE on the produced files (NOT retried).
        _generate_workspace_sbom()
        try:
            judged = 0
            for crate in ("sparq-server", "sparq-cli"):
                src = REPO_ROOT / "crates" / crate / f"{crate}.cdx.json"
                if not src.exists():
                    continue
                out = subprocess.run(
                    ["jq", "-f", str(NORMALIZE_JQ), str(src)],
                    check=True,
                    capture_output=True,
                    text=True,
                ).stdout
                refs = _walk_refs(json.loads(out))
                leaks = [r for r in refs if HOST_PATH.search(r)]
                self.assertEqual(leaks, [], f"{crate}: host path leaked into ref(s): {leaks}")
                # The root component ref of the released binary is the canonical cargo id.
                root = json.loads(out)["metadata"]["component"]["bom-ref"]
                self.assertTrue(
                    CANONICAL_REF.match(root),
                    f"{crate}: root bom-ref not canonical: {root!r}",
                )
                judged += 1
            self.assertGreater(judged, 0, "no released-binary SBOM was generated")
        finally:
            for f in REPO_ROOT.glob("crates/**/*.cdx.json"):
                f.unlink()


if __name__ == "__main__":
    unittest.main(verbosity=2)
