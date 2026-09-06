#!/usr/bin/env python3
# [OPUS-5] #4935 — the inference-conformance corpus AVAILABILITY decoupling.
#
# WHY a test at all. `Inference conformance (ratchet >= 1967 pass+divergence)` is a
# GATING check, and it used to re-download the OWL 2 test-case export from
# web.archive.org over plain http:// on every single run. When that host had a bad
# few minutes (merge group e5c2e01c, 2026-07-28: six 30 s connect timeouts) the
# corpus never arrived, NO assertion failed and nothing was measured — yet the job
# concluded `failure`, ci-summary correctly fail-fasted on it, and an unrelated PR
# (#4925, touching only scripts/ready-issues.py) was ejected from the merge queue.
#
# The fix is a cache, and a cache is INVISIBLE WHEN IT STOPS WORKING: drop the
# restore step and CI is still green — just coupled to a third party's uptime again,
# silently, until the next outage. So both halves are pinned here:
#
#   1. THE BEHAVIOUR — `fetch_pinned_file` (scripts/lib/fetch-retry.sh) makes ZERO
#      network calls when the payload is already at its pinned digest. That is the
#      whole reason an Actions-cache restore removes the coupling; asserted by
#      running the real bash function with a stub `curl` on PATH that records every
#      invocation. Non-vacuity is asserted directly: remove the pre-seeded payload
#      and the SAME call must go red with the stub fired.
#
#   2. THE WIRING — ci.yml's inference job restores tests/w3c/{owl2,rif-core} BEFORE
#      the fetch step, on a key that busts when the pins change, saving from main
#      only (repo cache-save doctrine).
#
#   3. THE RATCHET IS NOT WEAKENED — the issue is explicit that this must not be
#      "fixed" by exit-zeroing the download or softening the floor. The fetch step
#      must carry no `|| true` / `continue-on-error`, and the ratchet step must
#      still hard-fail. A cached corpus and a downloaded corpus are indistinguishable
#      to the runner, so a genuine regression still reds.
#
# Hermetic: stdlib only (no PyYAML, no network, no gh, no cargo).
# Run:  python3 scripts/tests/test_inference_corpus_cache.py

from __future__ import annotations

import hashlib
import os
import re
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
CI_YML = REPO_ROOT / ".github" / "workflows" / "ci.yml"
FETCH_LIB = REPO_ROOT / "scripts" / "lib" / "fetch-retry.sh"
FETCH_SCRIPT = REPO_ROOT / "scripts" / "fetch-inference-suites.sh"

JOB_NAME = "Inference conformance (ratchet >= 1967 pass+divergence)"
FETCH_STEP_CMD = "./scripts/fetch-inference-suites.sh"
CACHED_PATHS = ("tests/w3c/owl2", "tests/w3c/rif-core")
CACHE_RESTORE = "actions/cache/restore@"
CACHE_SAVE = "actions/cache/save@"
MAIN_ONLY = "github.ref == 'refs/heads/main'"

_JOB_KEY = re.compile(r"^ {2}[A-Za-z0-9_-]+:\s*$")


# --------------------------------------------------------------------------- #
# 1. Behaviour: offline on a cache hit.
# --------------------------------------------------------------------------- #
# A stub `curl` that records the fact it ran. `fail` mode is a dead source (what a
# blocked network looks like); `drift` mode writes a payload that does NOT match the
# pin, to check the digest gate still refuses to install it.
STUB_CURL = """#!/usr/bin/env bash
echo "curl $*" >> "$STUB_CURL_LOG"
out=""
prev=""
for a in "$@"; do
    if [ "$prev" = "-o" ]; then out="$a"; fi
    prev="$a"
done
if [ "${STUB_CURL_MODE:-fail}" = "drift" ]; then
    printf 'not-the-pinned-payload' > "$out"
    exit 0
fi
exit 7
"""


class TestOfflineOnCacheHit(unittest.TestCase):
    """Drives the REAL bash `fetch_pinned_file` with a recording stub `curl`."""

    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        tmp = Path(self._tmp.name)
        bin_dir = tmp / "bin"
        bin_dir.mkdir()
        stub = bin_dir / "curl"
        stub.write_text(STUB_CURL)
        stub.chmod(0o755)
        self.log = tmp / "curl.log"
        self.dest = tmp / "corpus" / "all.rdf"
        self.dest.parent.mkdir()
        self.payload = b"<rdf:RDF/>\n"
        self.digest = hashlib.sha256(self.payload).hexdigest()
        self.env = {
            **os.environ,
            "PATH": f"{bin_dir}:{os.environ.get('PATH', '')}",
            "STUB_CURL_LOG": str(self.log),
            "FETCH_RETRY_MAX": "1",
            "FETCH_RETRY_DELAY": "0",
        }
        self.addCleanup(self._tmp.cleanup)

    def run_fetch(self, mode: str = "fail") -> subprocess.CompletedProcess:
        script = (
            f'set -uo pipefail; . "{FETCH_LIB}"; '
            f'fetch_pinned_file "http://blocked.invalid/all.rdf" "{self.dest}" '
            f'"{self.digest}" "OWL 2 test cases"'
        )
        return subprocess.run(
            ["bash", "-c", script],
            env={**self.env, "STUB_CURL_MODE": mode},
            capture_output=True,
            text=True,
        )

    def curl_calls(self) -> list[str]:
        return self.log.read_text().splitlines() if self.log.exists() else []

    def test_pinned_payload_present_makes_no_network_call(self) -> None:
        """Acceptance test 1: with the corpus restored, the fetch is fully offline."""
        self.dest.write_bytes(self.payload)
        proc = self.run_fetch()
        self.assertEqual(
            proc.returncode,
            0,
            f"fetch_pinned_file must succeed offline when the payload is already at "
            f"the pin.\nstdout: {proc.stdout}\nstderr: {proc.stderr}",
        )
        self.assertEqual(
            self.curl_calls(),
            [],
            "fetch_pinned_file reached the network despite the payload already being "
            f"at the pinned digest — the cache restore buys nothing. curl calls: "
            f"{self.curl_calls()}",
        )
        self.assertIn("already present (sha256 ok)", proc.stdout)

    def test_absent_payload_does_reach_the_network(self) -> None:
        """Acceptance test 3 (mutation): drop the restored corpus and the SAME call
        must go red having actually invoked curl. Without this, the test above would
        pass just as well if the stub were unreachable or the function were a no-op."""
        self.assertFalse(self.dest.exists())
        proc = self.run_fetch()
        self.assertNotEqual(
            proc.returncode,
            0,
            "a dead source with no cached payload must FAIL — never silently pass",
        )
        self.assertEqual(
            len(self.curl_calls()),
            1,
            f"expected exactly one curl attempt on a cache miss, got: {self.curl_calls()}",
        )
        self.assertIn("could not download", proc.stderr)

    def test_wrong_digest_is_never_installed(self) -> None:
        """The pin still governs: a payload that does not match the digest is fatal
        and leaves nothing behind, so a retry can never substitute drifted data."""
        proc = self.run_fetch(mode="drift")
        self.assertNotEqual(proc.returncode, 0, "a digest mismatch must be fatal")
        self.assertIn("checksum mismatch", proc.stderr)
        self.assertFalse(
            self.dest.exists(), "a payload failing the digest gate was installed anyway"
        )
        self.assertFalse(
            Path(str(self.dest) + ".tmp").exists(), "the rejected .tmp was left behind"
        )

    def test_stale_payload_is_refetched(self) -> None:
        """A cache entry holding the WRONG bytes must not be trusted: the digest
        check — not mere presence — is what decides the fetch is skippable."""
        self.dest.write_bytes(b"stale-corpus-from-an-older-pin")
        proc = self.run_fetch()
        self.assertNotEqual(proc.returncode, 0)
        self.assertEqual(
            len(self.curl_calls()),
            1,
            "a present-but-wrong payload must trigger a refetch, not a silent pass",
        )


# --------------------------------------------------------------------------- #
# 2/3. Wiring + no-weakening, over the real ci.yml. Line-walked (PyYAML would drop
# the comments these workflows carry, and is not installed everywhere).
# --------------------------------------------------------------------------- #
def _is_comment(line: str) -> bool:
    return line.lstrip().startswith("#")


def inference_job_lines() -> list[str]:
    """The `steps:` region of the inference-conformance job, as raw lines."""
    lines = CI_YML.read_text().split("\n")
    start = next(
        (i for i, l in enumerate(lines) if f"name: {JOB_NAME}" in l and not _is_comment(l)),
        None,
    )
    if start is None:
        return []
    end = next(
        (i for i in range(start + 1, len(lines)) if _JOB_KEY.match(lines[i])),
        len(lines),
    )
    return lines[start:end]


def step_index(lines: list[str], needle: str) -> int:
    """Index of the first non-comment line containing `needle`, or -1."""
    return next(
        (i for i, l in enumerate(lines) if needle in l and not _is_comment(l)), -1
    )


class TestParserNonVacuity(unittest.TestCase):
    """Every assertion below reads a slice of ci.yml. If the slice is empty they all
    pass while checking nothing, so pin the slice itself."""

    def test_inference_job_is_found_and_still_fetches(self) -> None:
        job = inference_job_lines()
        self.assertTrue(
            job,
            f"could not locate the job named {JOB_NAME!r} in ci.yml — if the job or "
            "its ratchet number was renamed, re-point JOB_NAME here.",
        )
        self.assertGreater(
            step_index(job, FETCH_STEP_CMD),
            -1,
            f"the inference job no longer runs {FETCH_STEP_CMD}; this suite is checking "
            "a corpus fetch that has moved.",
        )


class TestCorpusCacheWiring(unittest.TestCase):
    def test_corpora_are_restored_before_the_fetch(self) -> None:
        job = inference_job_lines()
        restore = step_index(job, CACHE_RESTORE)
        fetch = step_index(job, FETCH_STEP_CMD)
        self.assertGreater(
            restore,
            -1,
            "the inference job has no actions/cache/restore step — the OWL 2 corpus is "
            "being re-downloaded from web.archive.org on every gating run (#4935).",
        )
        self.assertLess(
            restore,
            fetch,
            "the corpus cache is restored AFTER the fetch step, so the fetch still "
            "goes to the network every run.",
        )

    def test_both_third_party_corpora_are_cached(self) -> None:
        job = "\n".join(inference_job_lines())
        for path in CACHED_PATHS:
            self.assertIn(
                path,
                job,
                f"{path} is not in the cached path list; its corpus comes from a "
                "third-party host and would still be fetched live on every run.",
            )

    def test_cache_key_busts_when_the_pins_change(self) -> None:
        """The pins (OWL snapshot + sha256, RIF sha256) live in the fetch script, so
        hashing it is what stops a stale corpus outliving a pin bump."""
        job = "\n".join(l for l in inference_job_lines() if not _is_comment(l))
        self.assertIn(
            "hashFiles('scripts/fetch-inference-suites.sh')",
            job,
            "the corpus cache key does not hash the script the pins live in — a pin "
            "bump would silently keep restoring the OLD corpus.",
        )

    def test_cache_actions_are_sha_pinned(self) -> None:
        for line in inference_job_lines():
            if _is_comment(line) or "uses:" not in line:
                continue
            if CACHE_RESTORE in line or CACHE_SAVE in line:
                ref = line.split("@", 1)[1].split()[0]
                self.assertRegex(
                    ref,
                    r"^[0-9a-f]{40}$",
                    f"actions/cache must be pinned to a full commit SHA, got {ref!r} "
                    "(repo-wide SHA-pinning posture).",
                )

    def test_save_is_main_scoped(self) -> None:
        """Cache-save doctrine (sq-3sbrr / sq-6vshe.15): an entry written from a PR or
        `gh-readonly-queue/` ref is scoped to a ref that is deleted on merge/ejection —
        unreachable churn against the shared budget. Main seeds; everyone restores."""
        job = inference_job_lines()
        save = step_index(job, CACHE_SAVE)
        self.assertGreater(save, -1, "no actions/cache/save step — nothing ever seeds the cache")
        guard = next(
            (l for l in job[max(0, save - 4) : save + 4] if l.strip().startswith("if:")),
            None,
        )
        self.assertIsNotNone(guard, "the cache-save step carries no `if:` guard")
        self.assertIn(MAIN_ONLY, guard)


class TestRatchetNotWeakened(unittest.TestCase):
    """The issue is explicit that the coupling must NOT be broken by making failure
    cheap. Nothing here may soften on a bad day."""

    def test_the_fetch_step_still_fails_the_job(self) -> None:
        job = inference_job_lines()
        fetch = step_index(job, FETCH_STEP_CMD)
        step = "\n".join(job[max(0, fetch - 3) : fetch + 1])
        for escape in ("|| true", "|| echo", "continue-on-error"):
            self.assertNotIn(
                escape,
                step,
                f"the corpus fetch was made non-fatal with {escape!r}. A cache-miss "
                "fetch failure must still red — exit-zeroing it would discard genuine "
                "failures too (#4935 explicitly rules this out).",
            )

    def test_the_ratchet_floor_still_hard_fails(self) -> None:
        job = "\n".join(inference_job_lines())
        self.assertIn(
            "RATCHET=1967",
            job,
            "the inference ratchet floor moved or vanished — the caching change must "
            "not touch it.",
        )
        self.assertIn(
            "inference conformance regressed below the ratchet",
            job,
            "the ratchet's hard-fail branch is gone.",
        )

    def test_the_fetch_script_still_exits_on_a_dead_source(self) -> None:
        text = FETCH_SCRIPT.read_text()
        self.assertIn(
            "fetch_pinned_file",
            text,
            "the OWL fetch no longer goes through the digest-gated helper this suite "
            "exercises.",
        )
        self.assertIn(
            "exit 1",
            text,
            "the fetch script no longer fails on an unreachable source.",
        )


if __name__ == "__main__":
    unittest.main(verbosity=2)
