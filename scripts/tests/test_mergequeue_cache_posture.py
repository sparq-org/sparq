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
#   1. CACHE-SAVE DISCIPLINE. Every `Swatinem/rust-cache` step in a workflow that
#      is IN SCOPE (defined below) must carry
#      `save-if: ${{ github.ref == 'refs/heads/main' }}`.
#      A save from a merge-queue entry is DEAD ON ARRIVAL: Actions cache scoping
#      makes an entry written from `refs/heads/gh-readonly-queue/<base>/pr-<N>-<sha>`
#      visible to that ref alone, and GitHub deletes that ref as soon as the entry
#      merges or is ejected — nothing can ever restore it, so the save is pure
#      post-step wall-clock on the queue's critical path. Branch-scoped saves also
#      churn the repo's shared 10 GB Actions-cache budget and LRU-evict the
#      main-scoped entries that every branch actually restores. This is sq-3sbrr's
#      doctrine, already live in feature-matrix.yml and the wasm lanes; sq-6vshe.15
#      closed the merge-queue half (ci.yml's 20 cache steps + vectorized-feature-off).
#      RESTORE is deliberately untouched — `save-if` gates saving only, and queue
#      refs branch off main and read main's entry, which is where the hit comes from.
#
#      SCOPE (#5166 widened this from "triggers on merge_group" to the rule below).
#      sq-6vshe.15 scoped itself to merge_group lanes because its justification was
#      the queue's critical path. The OTHER half of the sq-3sbrr rationale — shared-
#      budget churn — applies to any lane that runs on branch refs. But the guard is
#      NOT safe to apply mechanically: it is safe only when some run on
#      `refs/heads/main` RE-SEEDS the same cache key. Otherwise the lane never saves
#      AND has nothing to restore, and goes permanently cold — a regression, not a
#      win. So a workflow is in scope iff BOTH:
#
#        (a) it triggers on `pull_request` or `merge_group` — i.e. it actually runs
#            on branch refs, so there is churn to eliminate; AND
#        (b) it triggers on `push:` with `main` among its branches — so main keeps
#            re-seeding whatever keys the lane uses.
#
#      This is a FLOOR, not an exact set: a lane outside it may still carry `save-if`
#      (pages.yml does), and that is fine. What the rule forbids is an in-scope lane
#      WITHOUT the guard.
#
#      DELIBERATELY OUT OF SCOPE, and why (#5166 decided these per lane rather than
#      mechanically — each is a case where the guard is wrong or useless):
#
#        * LANES THAT NEVER RUN ON A PR REF — asan, datalog-souffle, differential,
#          differential-update, kani, lws-cth, metamorph, miri, shacl-diff-fuzz
#          (schedule and/or workflow_dispatch only), plus pkg-ingest (`push:` to main
#          + dispatch) and selection-alarm (`workflow_run` on main + dispatch). None
#          has a `pull_request` trigger, so whatever DOES run them — schedule, a main
#          push, a workflow_run — is already on main and the guard is a no-op that
#          only adds noise. (`workflow_dispatch` against a branch is the lone
#          exception, and is rare and deliberate.)
#        * mutants-diff.yml — `pull_request` ONLY, so it fails (b): main never runs
#          it and nothing would ever seed its key. Guarding it would mean never
#          saving and never restoring, i.e. a cold rebuild on every PR. Its branch-
#          scoped save is genuinely useful, because the same PR restores it on each
#          subsequent push.
#        * release.yml — runs off `push: tags:`, so `github.ref` is `refs/tags/*`
#          and a main-only guard would stop it saving at all. Its saves ARE dead in
#          the same way queue-ref saves are (a tag-scoped entry is invisible to
#          every later tag), but the correct guard there is a different one
#          (`save-if: false`) on the release critical path, which is a separate
#          decision and NOT made here.
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


def on_block(path: Path) -> list[str]:
    """The workflow's top-level `on:` block, comment lines stripped.

    Comments are dropped because several workflows carry a prose note explaining
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
        return []
    end = len(lines)
    for i in range(start + 1, len(lines)):
        line = lines[i]
        if line and not line[0].isspace() and not _is_comment(line):
            end = i
            break
    return [l for l in lines[start:end] if not _is_comment(l)]


def triggers_on(path: Path, event: str) -> bool:
    """True iff the workflow's `on:` block declares the top-level event `event`."""
    return any(re.match(rf"^\s+{event}:", l) for l in on_block(path))


def triggers_on_merge_group(path: Path) -> bool:
    return triggers_on(path, "merge_group")


def _sub_block(block: list[str], i: int, indent: int) -> list[str]:
    """The lines of `block` after index `i` indented deeper than `indent`."""
    out = []
    for line in block[i + 1 :]:
        if not line.strip():
            continue
        if len(line) - len(line.lstrip()) <= indent:
            break
        out.append(line)
    return out


def pushes_to_main(path: Path) -> bool:
    """True iff the workflow runs on `push` to `main`.

    This is the half of the scope rule that makes the guard SAFE: if main never
    runs the lane, main never re-seeds its cache key, and a main-only `save-if`
    would leave the lane permanently cold instead of merely un-churned.
    """
    block = on_block(path)
    for i, line in enumerate(block):
        m = re.match(r"^(\s+)push:", line)
        if not m:
            continue
        sub = _sub_block(block, i, len(m.group(1)))
        for j, s in enumerate(sub):
            bm = re.match(r"^(\s+)branches:(.*)$", s)
            if not bm:
                continue
            inline = bm.group(2).strip()
            if inline:  # `branches: [main, ...]`
                return "main" in re.findall(r"[\w.\-/*]+", inline)
            # block form: `branches:` then `- main`
            return any(
                t.strip().lstrip("- ").strip("\"'") == "main"
                for t in _sub_block(sub, j, len(bm.group(1)))
            )
        # A `push:` filtered to `tags:` only (release.yml) never fires on a branch.
        if any(re.match(r"^\s+tags(-ignore)?:", s) for s in sub):
            return False
        # `push:` with no ref filter at all fires on every branch, main included.
        return True
    return False


def requires_save_if(path: Path) -> bool:
    """The #5166 scope rule. See the CACHE-SAVE DISCIPLINE header note."""
    runs_on_branch_refs = triggers_on(path, "pull_request") or triggers_on(
        path, "merge_group"
    )
    return runs_on_branch_refs and pushes_to_main(path)


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


def merge_group_workflows_with_rust_cache() -> list[Path]:
    return sorted(
        p
        for p in WORKFLOWS.glob("*.yml")
        if RUST_CACHE in p.read_text() and triggers_on_merge_group(p)
    )


def workflows_requiring_save_if() -> list[Path]:
    return sorted(
        p
        for p in WORKFLOWS.glob("*.yml")
        if RUST_CACHE in p.read_text() and requires_save_if(p)
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

    def test_scope_rule_covers_the_lanes_5166_guarded(self) -> None:
        # The #5166 set: lanes that run on branch refs AND on push-to-main. If the
        # scope rule regresses to "merge_group only", these drop out and the guards
        # below stop being enforced — so the membership is pinned by name.
        names = {p.name for p in workflows_requiring_save_if()}
        for expected in (
            "bench.yml",
            "formal-verification.yml",
            "fuzz.yml",
            "python.yml",
            "xpath-differential.yml",
            "zk-toolchain.yml",
        ):
            self.assertIn(
                expected,
                names,
                f"{expected} runs on branch refs AND on push-to-main, so #5166's rule "
                f"must hold it in scope. In scope: {sorted(names)}",
            )

    def test_pr_only_lane_is_out_of_scope(self) -> None:
        # mutants-diff.yml is `pull_request`-ONLY. Guarding it would mean it never
        # saves and — since nothing on main seeds its key — never restores either,
        # i.e. a cold rebuild every PR. This is the safety half of the rule; if it
        # ever flips to a bare "runs on PRs" test, this goes red.
        wf = WORKFLOWS / "mutants-diff.yml"
        self.assertTrue(wf.exists(), "fixture-by-reference vanished; re-point this test")
        self.assertTrue(triggers_on(wf, "pull_request"), "trigger changed; re-point")
        self.assertFalse(
            pushes_to_main(wf),
            "mutants-diff.yml gained a push-to-main trigger — main now seeds its cache "
            "key, so it should be brought IN scope and given the `save-if` guard.",
        )
        self.assertFalse(requires_save_if(wf))

    def test_tag_only_lane_is_out_of_scope(self) -> None:
        # release.yml runs off `push: tags:`, where `github.ref` is `refs/tags/*` — a
        # main-only guard would stop it saving ENTIRELY. Pins that `pushes_to_main`
        # distinguishes a tag filter from a branch filter.
        wf = WORKFLOWS / "release.yml"
        self.assertTrue(wf.exists(), "fixture-by-reference vanished; re-point this test")
        self.assertFalse(
            pushes_to_main(wf),
            "release.yml's `push:` is tag-filtered; reading that as a push-to-main "
            "would put the release lane in scope and stop it caching at all.",
        )
        self.assertFalse(requires_save_if(wf))

    def test_nightly_only_lanes_are_out_of_scope(self) -> None:
        # schedule/dispatch-only lanes already run on main, so the guard is a no-op
        # that only adds noise (#5166). Pinned so a future edit does not quietly
        # mechanically-expand the rule to them.
        for name in ("miri.yml", "kani.yml", "asan.yml", "differential.yml"):
            wf = WORKFLOWS / name
            self.assertTrue(wf.exists(), f"{name} vanished; re-point this test")
            self.assertFalse(
                requires_save_if(wf),
                f"{name} is nightly/dispatch-only — it has no `pull_request` trigger, so "
                "there is no branch-ref churn for the guard to remove.",
            )

    def test_step_walker_finds_every_rust_cache_step(self) -> None:
        for wf in workflows_requiring_save_if():
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
    def test_every_in_scope_rust_cache_step_saves_on_main_only(self) -> None:
        for wf in workflows_requiring_save_if():
            lines = _lines(wf)
            for idx, body in steps_using(lines, RUST_CACHE):
                where = f"{wf.name}:{idx + 1}"
                got = with_value(body, SAVE_IF_KEY)
                self.assertIsNotNone(
                    got,
                    f"{where}: rust-cache step in a workflow that runs on branch refs AND "
                    f"on push-to-main has no `save-if`. Saves from a `gh-readonly-queue/*` "
                    f"ref are unrestorable (the ref is deleted at merge) and saves from a "
                    f"`refs/pull/<n>/merge` ref are restorable by that one PR alone, while "
                    f"both consume the shared 10 GB budget — add "
                    f"`save-if: {SAVE_IF_VALUE}` (sq-6vshe.15 lever 2, widened by #5166).",
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
        for wf in workflows_requiring_save_if():
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
