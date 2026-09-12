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
#   3. CACHE-KEY HYGIENE (#5214). Every `Swatinem/rust-cache` step in ci.yml must
#      declare a `shared-key`. The action's two key inputs are NOT interchangeable:
#      `shared-key` REPLACES the key rust-cache otherwise derives from the JOB ID,
#      whereas `key` is an ADDITIONAL component that leaves the job-derived one in
#      place. So a step declaring neither — and equally a step declaring only
#      `key` — still takes its own job-scoped entry of the repo's shared 10 GB
#      Actions-cache budget: sixteen of ci.yml's twenty steps named nothing and two
#      more named only `key`, one entry per job for what is largely the same dep
#      closure off one Cargo.lock, and budget pressure is the LRU-eviction mechanism
#      sq-3sbrr (#1395) identified as what makes a warm cache restore cold. A
#      job-derived key is also rename-fragile (renaming a job silently orphans its
#      entry) and unaddressable from any other job — which is why the `key`-only
#      form is rejected here BY NAME rather than accepted as "named": it is exactly
#      how `coverage-engine-merge` came to point at `coverage-engine-run-1` and
#      restore nothing. Same failure mode as items 1-2 — invisible when absent,
#      nothing goes red, CI just gets slower. So this pins the property, the one
#      cross-job reuse that depends on it (the coverage-engine run/merge pair), and
#      the one COLLAPSE it enabled:
#      `conformance-suite`, whose membership is exactly the six same-shaped
#      conformance/oracle jobs, each of which runs the same release-fast
#      `sparq-conformance-scoreboard` build (the coincidence the shared key rests
#      on, asserted here rather than assumed).
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

# [OPUS-5] #5214 — the one cache key ci.yml's six same-shaped conformance/oracle jobs
# share, and that group's exact membership. The canonical rationale (and the honest
# residual) lives on the `geo-conformance` rust-cache step in ci.yml.
CONFORMANCE_SUITE_KEY = "conformance-suite"
CONFORMANCE_SUITE_JOBS = {
    "geo-conformance",
    "solid-conformance",
    "odrl-conformance",
    "text-oracle",
    "rsp-oracle",
    "jsonld-conformance",
}
# The build every member runs VERBATIM — the reason their dep closures coincide
# enough to share one entry. Matched on the two load-bearing fragments of the ONE
# command line, so incidental whitespace does not red the suite but a job that keeps
# only half of it (a dev-profile scoreboard run, say) does not satisfy the premise.
SCOREBOARD_BUILD = re.compile(
    r"--profile release-fast.*--bin sparq-conformance-scoreboard"
)

# [OPUS-5] #5214 — the cross-job reuse the `shared-key` semantics are load-bearing for:
# the merge job recompiles the instrumented objects the run partitions' .profraw are
# merged against, and warms that compile off partition 1's dep cache. Only `shared-key`
# makes the two jobs address ONE entry.
COVERAGE_ENGINE_RUN_JOB = "coverage-engine-run"
COVERAGE_ENGINE_MERGE_JOB = "coverage-engine-merge"
COVERAGE_ENGINE_PART_EXPR = "${{ matrix.part }}"

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


def enclosing_job(lines: list[str], idx: int) -> str:
    """The id of the job whose block contains `lines[idx]` (`  <job-id>:`)."""
    for i in range(idx, -1, -1):
        m = re.match(r"^  ([A-Za-z0-9_-]+):\s*$", lines[i])
        if m:
            return m.group(1)
    raise AssertionError(f"no enclosing job for line {idx + 1}")


def job_body(lines: list[str], job_id: str) -> list[str]:
    """Every line of the `  <job-id>:` block, up to the next job."""
    start = next(
        (i for i, l in enumerate(lines) if re.match(rf"^  {re.escape(job_id)}:\s*$", l)),
        None,
    )
    if start is None:
        raise AssertionError(f"ci.yml has no job `{job_id}`")
    end = len(lines)
    for i in range(start + 1, len(lines)):
        if re.match(r"^  [A-Za-z0-9_-]+:\s*$", lines[i]):
            end = i
            break
    return lines[start:end]


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


class TestCacheKeyHygiene(unittest.TestCase):
    """#5214 — no anonymous cache entries, and one honest collapse (item 3)."""

    def _keys_by_step(self) -> list[tuple[str, str | None, str | None]]:
        # (job, shared-key, key) — the two inputs are modelled SEPARATELY because
        # rust-cache treats them differently: `shared-key` replaces the job-derived
        # key, `key` is merely appended alongside it. Collapsing them with an `or`
        # would score a `key`-only step as job-id-independent when it is not.
        #
        # A LIST, not a dict: a job with two rust-cache steps must not have one of
        # them silently overwrite (and hide) the other.
        lines = _lines(CI_YML)
        steps = steps_using(lines, RUST_CACHE)
        self.assertTrue(steps, "no rust-cache steps found in ci.yml — re-point this test")
        out: list[tuple[str, str | None, str | None]] = []
        for idx, body in steps:
            shared = with_value(body, "shared-key:")
            extra = with_value(body, "key:")
            out.append(
                (
                    enclosing_job(lines, idx),
                    shared.strip('"') if shared else None,
                    extra.strip('"') if extra else None,
                )
            )
        return out

    def _shared_key_of(self, job: str) -> str:
        keys = [s for j, s, _ in self._keys_by_step() if j == job]
        self.assertEqual(
            len(keys),
            1,
            f"expected exactly one rust-cache step in job `{job}`, found {len(keys)} — "
            "re-point this test.",
        )
        self.assertIsNotNone(keys[0], f"job `{job}` declares no `shared-key`")
        return keys[0]  # type: ignore[return-value]

    def test_every_rust_cache_step_declares_a_shared_key(self) -> None:
        anonymous = sorted(j for j, shared, _ in self._keys_by_step() if shared is None)
        self.assertEqual(
            anonymous,
            [],
            "these ci.yml jobs run rust-cache without a `shared-key`, so each takes its "
            "OWN entry of the shared 10 GB Actions-cache budget under a key derived from "
            "the job id — anonymous, rename-fragile, and the budget pressure that "
            f"LRU-evicts the warm entries (#5214, #1395): {anonymous}",
        )

    def test_key_alone_does_not_count_as_naming_the_key(self) -> None:
        # The distinction this whole item rests on, asserted rather than assumed:
        # rust-cache appends `key` to the job-derived key and only `shared-key`
        # replaces it. A step carrying `key:` but no `shared-key:` is therefore still
        # job-scoped — it LOOKS named, and reading it as named is what let
        # coverage-engine-merge point at `coverage-engine-run-1` and restore nothing.
        key_only = sorted(
            f"{j} (key: {extra})"
            for j, shared, extra in self._keys_by_step()
            if shared is None and extra is not None
        )
        self.assertEqual(
            key_only,
            [],
            "these ci.yml jobs declare rust-cache `key:` but no `shared-key:`. `key` is "
            "an ADDITIONAL component alongside the job-derived key, not a replacement "
            "for it, so the entry stays scoped to the job id: unaddressable from any "
            f"other job and orphaned by a rename. Use `shared-key` (#5214): {key_only}",
        )

    def test_coverage_engine_merge_reuses_partition_1s_cache(self) -> None:
        # The one deliberate CROSS-JOB restore in this file, pinned in both directions:
        # the merge job recompiles the same instrumented objects and names partition 1's
        # key to warm that compile. That only resolves to one entry while BOTH steps use
        # `shared-key`; drift in either value silently reverts it to a cold build.
        run = self._shared_key_of(COVERAGE_ENGINE_RUN_JOB)
        merge = self._shared_key_of(COVERAGE_ENGINE_MERGE_JOB)
        self.assertIn(
            COVERAGE_ENGINE_PART_EXPR,
            run,
            f"`{COVERAGE_ENGINE_RUN_JOB}` is a 3-partition matrix and its cache key must "
            "stay per-part (one entry per partition is what avoids the concurrent-save "
            f"clobber): expected `{COVERAGE_ENGINE_PART_EXPR}` in {run!r}.",
        )
        self.assertEqual(
            merge,
            run.replace(COVERAGE_ENGINE_PART_EXPR, "1"),
            f"`{COVERAGE_ENGINE_MERGE_JOB}` must name exactly the key "
            f"`{COVERAGE_ENGINE_RUN_JOB}` writes for partition 1 "
            f"({run.replace(COVERAGE_ENGINE_PART_EXPR, '1')!r}), or it restores nothing "
            f"and compiles the instrumented objects cold. Got {merge!r}.",
        )

    def test_conformance_suite_membership_is_exactly_the_six(self) -> None:
        members = {
            j for j, shared, _ in self._keys_by_step() if shared == CONFORMANCE_SUITE_KEY
        }
        self.assertEqual(
            members,
            CONFORMANCE_SUITE_JOBS,
            f"`{CONFORMANCE_SUITE_KEY}` is sized for the six same-shaped conformance/"
            "oracle jobs (one pinned-stable host toolchain, no job RUSTFLAGS, a "
            "dev-profile single-crate test plus the shared release-fast scoreboard "
            "build). Adding a differently-shaped job makes its closure fight the others "
            "for one entry; dropping one re-splits the budget. Update the canonical note "
            "on ci.yml's `geo-conformance` cache step and this set together.",
        )

    def test_every_suite_member_runs_the_shared_scoreboard_build(self) -> None:
        # The PREMISE of the shared key, asserted rather than assumed: what makes these
        # six closures coincide is that each runs the same release-fast
        # sparq-conformance-scoreboard build. A member that stopped running it would
        # keep the key while no longer sharing the build it is justified by.
        lines = _lines(CI_YML)
        for job in sorted(CONFORMANCE_SUITE_JOBS):
            body = "\n".join(l for l in job_body(lines, job) if not _is_comment(l))
            self.assertRegex(
                body,
                SCOREBOARD_BUILD,
                f"job `{job}` carries the `{CONFORMANCE_SUITE_KEY}` cache key but no "
                "longer runs the shared release-fast sparq-conformance-scoreboard "
                "build. Either restore the build or give the job its own key.",
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
