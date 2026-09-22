#!/usr/bin/env python3
# [SONNET-4.6] sq-6vshe.15 — INSPECTION test for the merge-queue cache/artifact
# posture (research/ci-mergequeue-speedup-2026-07.md §3.2, lever 2 of the CI
# structural-speedup program sq-6vshe; extends sq-6vshe.5, which owns key schema
# and backend sizing).
#
# WHY a test at all: both properties this bead establishes are single YAML lines
# that are invisible when absent. Nothing goes red if a future edit drops them —
# CI just silently gets slower and the shared cache budget silently churns again.
# So the two lines are PINNED here, structurally, over every workflow that still
# triggers on `merge_group`:
#
#   1. CACHE-SAVE DISCIPLINE. Every `Swatinem/rust-cache` step in a
#      merge_group-triggered workflow must carry
#      `save-if: ${{ github.ref == 'refs/heads/main' }}`.
#      A save from a merge-queue entry is DEAD ON ARRIVAL: Actions cache scoping
#      makes an entry written from `refs/heads/gh-readonly-queue/<base>/pr-<N>-<sha>`
#      visible to that ref alone, and GitHub deletes that ref as soon as the entry
#      merges or is ejected — nothing can ever restore it, so the save is pure
#      post-step wall-clock on the queue's critical path. Branch-scoped saves also
#      churn the repo's shared 10 GB Actions-cache budget and LRU-evict the
#      main-scoped entries that every branch actually restores. This is sq-3sbrr's
#      doctrine, already live in feature-matrix.yml and the wasm lanes; sq-6vshe.15
#      closed the remaining gap (ci.yml's 20 cache steps + vectorized-feature-off).
#      RESTORE is deliberately untouched — `save-if` gates saving only, and queue
#      refs branch off main and read main's entry, which is where the hit comes from.
#
#   2. ARTIFACT DIET. The `nextest-archive` upload must set `compression-level: 0`.
#      `nextest.tar.zst` is already zstd-compressed; upload-artifact's default
#      DEFLATE-6 zip pass re-compresses incompressible bytes for a ~0 size delta,
#      burning CPU on the one job every test shard is blocked on. Level 0 stores the
#      file as-is. The test also pins the ARCHIVE CONTENT-PARITY contract the diet
#      must never break: the uploaded `path:` is exactly the file `cargo nextest
#      archive --archive-file` writes, and the artifact `name:` is the one the test
#      shards download — so the shards keep receiving the same byte-identical
#      archive, and the nextest test set is unchanged.
#
# [OPUS-5] issue #5231 adds a third property of the same shape:
#
#   3. CROSS-JOB CACHE SHARING. `coverage-engine-run` (3 partitions) and
#      `coverage-engine-merge` compile the SAME instrumented sparq-engine, so they
#      must land on ONE rust-cache entry. rust-cache composes the `key:` input WITH a
#      GITHUB_JOB-derived segment and uses `shared-key:` INSTEAD of it, so the former
#      `key: coverage-engine-run-<part>` / `key: coverage-engine-run-1` pair resolved
#      to different keys and different restore-key prefixes in the two jobs — the
#      merge job restored nothing and paid a cold dep compile on the merge-queue
#      critical path, while the comment above the step claimed the opposite. Exactly
#      this file's failure mode: invisible when wrong, reds nothing. So the two jobs'
#      `shared-key` is pinned equal, and a bare `key:` on either is rejected.
#
# Deliberately NOT asserted: sccache. Bead item 3 (an sccache/GHA-backend A/B on
# build-archive) is measure-first with a >=60 s median-win adoption bar, and no such
# measurement has been taken — so nothing about sccache is wired, claimed, or pinned
# here, and no verdict on it should be inferred from this file's silence.
#
# Hermetic: stdlib only (no PyYAML, no network, no gh) so it runs anywhere.
# Run:  python3 scripts/tests/test_mergequeue_cache_posture.py

from __future__ import annotations

import re
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
WORKFLOWS = REPO_ROOT / ".github" / "workflows"
CI_YML = WORKFLOWS / "ci.yml"

RUST_CACHE = "Swatinem/rust-cache@"
SAVE_IF_KEY = "save-if:"
# The canonical guard. Bare `false` would ALSO stop the queue-ref saves — and would
# stop main from ever seeding a cache, so every job would restore nothing forever.
# The value is pinned exactly for that reason.
SAVE_IF_VALUE = "${{ github.ref == 'refs/heads/main' }}"

# The pair whose dep cache must be ONE entry (issue #5231): three run partitions plus
# the merge job that recompiles the same instrumented objects to map the counters.
COVERAGE_ENGINE_JOBS = ("coverage-engine-run", "coverage-engine-merge")

NEXTEST_ARCHIVE_ARTIFACT = "nextest-archive"
UPLOAD_ARTIFACT = "actions/upload-artifact@"
DOWNLOAD_ARTIFACT = "actions/download-artifact@"

_NEW_LIST_ITEM = re.compile(r"^ {0,6}- ")
_ARCHIVE_FILE_ARG = re.compile(r"--archive-file\s+(\S+)")


# --------------------------------------------------------------------------- #
# Minimal structural helpers. PyYAML would flatten the comments these workflows
# carry and is not installed in every environment, so we walk lines instead.
# --------------------------------------------------------------------------- #
def _lines(path: Path) -> list[str]:
    return path.read_text().split("\n")


def _is_comment(line: str) -> bool:
    return line.lstrip().startswith("#")


def triggers_on_merge_group(path: Path) -> bool:
    """True iff the workflow's top-level `on:` block declares `merge_group`.

    Comment lines are ignored: several workflows carry a prose note explaining
    that `merge_group` was REMOVED (2026-07-18 maintainer directive), and
    counting those would be exactly the false positive that makes this suite
    assert the property over lanes that never see a queue ref.
    """
    lines = _lines(path)
    start = next(
        (i for i, l in enumerate(lines) if re.match(r'^(on|"on"):', l)),
        None,
    )
    if start is None:
        return False
    end = len(lines)
    for i in range(start + 1, len(lines)):
        line = lines[i]
        if line and not line[0].isspace() and not _is_comment(line):
            end = i
            break
    block = lines[start:end]
    return any(
        re.match(r"^\s+merge_group:", l) for l in block if not _is_comment(l)
    )


def step_body(lines: list[str], start: int) -> list[str]:
    """The lines of the `steps:` list item whose first line is `lines[start]`."""
    i = start + 1
    while i < len(lines):
        line = lines[i]
        if not line.strip():
            i += 1
            continue
        indent = len(line) - len(line.lstrip())
        if _NEW_LIST_ITEM.match(line) or indent < 8:
            break
        i += 1
    return lines[start:i]


def job_body(lines: list[str], job_id: str) -> list[str]:
    """The lines of the `jobs:` entry `job_id`, or [] when it is absent.

    Jobs sit at indent 2 under the top-level `jobs:` key, so the entry runs from its
    `  <job_id>:` header to the next non-comment line at indent <= 2.
    """
    start = next(
        (i for i, l in enumerate(lines) if l == f"  {job_id}:"),
        None,
    )
    if start is None:
        return []
    end = len(lines)
    for i in range(start + 1, len(lines)):
        line = lines[i]
        if not line.strip() or _is_comment(line):
            continue
        if len(line) - len(line.lstrip()) <= 2:
            end = i
            break
    return lines[start:end]


def steps_using(lines: list[str], marker: str) -> list[tuple[int, list[str]]]:
    """Every step whose `uses:` names `marker`, as (0-based line index, body)."""
    return [
        (i, step_body(lines, i))
        for i, l in enumerate(lines)
        if marker in l and "uses:" in l
    ]


def with_value(body: list[str], key: str) -> str | None:
    """The value of `key` inside the step's `with:` mapping, or None."""
    for line in body:
        stripped = line.strip()
        if _is_comment(line):
            continue
        if stripped.startswith(key):
            return stripped[len(key) :].strip()
    return None


def merge_group_workflows_with_rust_cache() -> list[Path]:
    return sorted(
        p
        for p in WORKFLOWS.glob("*.yml")
        if RUST_CACHE in p.read_text() and triggers_on_merge_group(p)
    )


class TestParserNonVacuity(unittest.TestCase):
    """The assertions below iterate over discovered sets. If discovery silently
    returns nothing, every other test in this file passes while checking NOTHING.
    Pin the discovery itself."""

    def test_ci_yml_is_discovered_as_a_merge_group_lane(self) -> None:
        names = [p.name for p in merge_group_workflows_with_rust_cache()]
        self.assertIn(
            "ci.yml",
            names,
            "ci.yml is THE merge-queue pole (research §2.1) and carries rust-cache "
            f"steps; the on:-block parser failed to see its merge_group trigger. Found: {names}",
        )

    def test_removed_merge_group_comment_is_not_a_trigger(self) -> None:
        # zk-toolchain.yml documents in PROSE that merge_group was removed. If the
        # parser counted comments, it would be in the set and this suite would demand
        # save-if on a lane that never runs on a queue ref.
        zk = WORKFLOWS / "zk-toolchain.yml"
        self.assertTrue(zk.exists(), "fixture-by-reference vanished; re-point this test")
        self.assertIn("merge_group", zk.read_text(), "prose reference gone; re-point")
        self.assertFalse(
            triggers_on_merge_group(zk),
            "zk-toolchain.yml only MENTIONS merge_group in a comment — the on:-block "
            "parser is matching comments.",
        )

    def test_step_walker_finds_every_rust_cache_step(self) -> None:
        for wf in merge_group_workflows_with_rust_cache():
            text = wf.read_text()
            found = steps_using(text.split("\n"), RUST_CACHE)
            self.assertEqual(
                len(found),
                text.count(RUST_CACHE),
                f"{wf.name}: the step walker saw {len(found)} rust-cache steps but the "
                f"file names the action {text.count(RUST_CACHE)} times — a step shape the "
                "walker does not understand would be silently unchecked.",
            )


class TestCacheSaveDiscipline(unittest.TestCase):
    def test_every_merge_group_rust_cache_step_saves_on_main_only(self) -> None:
        for wf in merge_group_workflows_with_rust_cache():
            lines = _lines(wf)
            for idx, body in steps_using(lines, RUST_CACHE):
                where = f"{wf.name}:{idx + 1}"
                got = with_value(body, SAVE_IF_KEY)
                self.assertIsNotNone(
                    got,
                    f"{where}: rust-cache step in a merge_group-triggered workflow has no "
                    f"`save-if`. Saves from a `gh-readonly-queue/*` ref are unrestorable "
                    f"(the ref is deleted at merge) — add "
                    f"`save-if: {SAVE_IF_VALUE}` (sq-6vshe.15 lever 2).",
                )
                self.assertEqual(
                    got,
                    SAVE_IF_VALUE,
                    f"{where}: `save-if` must be exactly `{SAVE_IF_VALUE}`. A bare `false` "
                    "also stops main from ever SEEDING a cache; a wider condition lets the "
                    "dead queue-ref save back in.",
                )

    def test_restore_is_never_disabled(self) -> None:
        # `save-if` gates SAVING only — the whole design depends on queue refs and PR
        # heads still RESTORING main's entry. `lookup-only` would break that silently
        # (a cache "hit" that unpacks nothing).
        for wf in merge_group_workflows_with_rust_cache():
            lines = _lines(wf)
            for idx, body in steps_using(lines, RUST_CACHE):
                self.assertIsNone(
                    with_value(body, "lookup-only:"),
                    f"{wf.name}:{idx + 1}: rust-cache `lookup-only` would disable RESTORE; "
                    "sq-6vshe.15 restricts SAVING only.",
                )


class TestNextestArchiveDiet(unittest.TestCase):
    def _upload_step(self) -> list[str]:
        lines = _lines(CI_YML)
        for idx, body in steps_using(lines, UPLOAD_ARTIFACT):
            if with_value(body, "name:") == NEXTEST_ARCHIVE_ARTIFACT:
                return body
        self.fail(
            f"ci.yml has no upload-artifact step named `{NEXTEST_ARCHIVE_ARTIFACT}` — "
            "the build-once contract (sq-vyxy) is gone or renamed; re-point this test."
        )

    def test_upload_does_not_recompress_the_zstd_archive(self) -> None:
        self.assertEqual(
            with_value(self._upload_step(), "compression-level:"),
            "0",
            "the nextest archive is already zstd-compressed, so upload-artifact's default "
            "DEFLATE-6 pass costs CPU on the merge-queue critical path for a ~0 size delta. "
            "Set `compression-level: 0` (sq-6vshe.15 lever 1).",
        )

    def test_uploaded_path_is_the_file_nextest_archive_writes(self) -> None:
        # Content parity, half 1: the diet must never change WHICH file ships.
        text = CI_YML.read_text()
        archive_files = set(_ARCHIVE_FILE_ARG.findall(text))
        self.assertTrue(
            archive_files,
            "ci.yml no longer passes `--archive-file` to cargo nextest; re-point this test.",
        )
        path = with_value(self._upload_step(), "path:")
        self.assertIn(
            path,
            archive_files,
            f"the uploaded path {path!r} is not one of the `--archive-file` targets "
            f"{sorted(archive_files)} — the shards would download something other than "
            "the archive that was just built.",
        )

    def test_the_test_shards_download_the_same_artifact_name(self) -> None:
        # Content parity, half 2: producer and consumers must agree on the name.
        lines = _lines(CI_YML)
        consumers = [
            idx
            for idx, body in steps_using(lines, DOWNLOAD_ARTIFACT)
            if with_value(body, "name:") == NEXTEST_ARCHIVE_ARTIFACT
        ]
        self.assertTrue(
            consumers,
            f"nothing in ci.yml downloads `{NEXTEST_ARCHIVE_ARTIFACT}` — the build-once "
            "archive would be uploaded and never consumed.",
        )


class TestCoverageEngineCacheIsShared(unittest.TestCase):
    """Property 3 (issue #5231): the engine-coverage partitions and the merge job
    must resolve to ONE rust-cache entry, which only `shared-key` achieves."""

    def _rust_cache_steps(self, job_id: str) -> list[list[str]]:
        body = job_body(_lines(CI_YML), job_id)
        self.assertTrue(
            body,
            f"ci.yml has no `{job_id}:` job — the engine coverage split was renamed or "
            "removed; re-point this test rather than deleting it.",
        )
        return [b for _, b in steps_using(body, RUST_CACHE)]

    def test_each_job_has_exactly_one_rust_cache_step(self) -> None:
        # Non-vacuity: the assertions below read [0]. If the walker found nothing they
        # would raise rather than pass, but a SECOND cache step would go unchecked.
        for job_id in COVERAGE_ENGINE_JOBS:
            self.assertEqual(
                len(self._rust_cache_steps(job_id)),
                1,
                f"`{job_id}` no longer has exactly one rust-cache step; this test only "
                "reasons about a single dep cache per job.",
            )

    def test_both_jobs_declare_the_same_shared_key(self) -> None:
        keys = {
            job_id: with_value(self._rust_cache_steps(job_id)[0], "shared-key:")
            for job_id in COVERAGE_ENGINE_JOBS
        }
        for job_id, got in keys.items():
            self.assertIsNotNone(
                got,
                f"`{job_id}`'s rust-cache step has no `shared-key`. Without it rust-cache "
                "appends the GITHUB_JOB-derived segment, so the two jobs get different "
                "keys AND different restore-key prefixes and the merge job compiles its "
                "deps cold (issue #5231).",
            )
        self.assertEqual(
            len(set(keys.values())),
            1,
            "coverage-engine-run and coverage-engine-merge must share ONE rust-cache "
            "entry — they compile the same instrumented sparq-engine, and the merge job's "
            "whole cache warm-up depends on the two `shared-key` values being identical. "
            f"Got: {keys}",
        )

    def test_neither_job_re_splits_the_entry_with_a_bare_key(self) -> None:
        # `key:` is composed WITH the automatic job-id segment rather than replacing it,
        # so re-adding one alongside `shared-key` (or instead of it) silently re-splits
        # the shared entry back into one-per-job. That is the exact #5231 regression.
        for job_id in COVERAGE_ENGINE_JOBS:
            self.assertIsNone(
                with_value(self._rust_cache_steps(job_id)[0], "key:"),
                f"`{job_id}`'s rust-cache step declares a bare `key:`. rust-cache adds the "
                "job id to `key:`, so the partitions and the merge job would stop sharing "
                "an entry (issue #5231). Use `shared-key:` alone.",
            )


class TestSuiteIsWiredIntoCi(unittest.TestCase):
    """A structural test that never runs is a comment. Pin its own call site."""

    def test_docs_quality_invokes_this_suite(self) -> None:
        dq = (WORKFLOWS / "docs-quality.yml").read_text()
        self.assertIn(
            "scripts/tests/test_mergequeue_cache_posture.py",
            dq,
            "this suite must be invoked by docs-quality.yml or it silently stops gating.",
        )


if __name__ == "__main__":
    unittest.main(verbosity=2)
