#!/usr/bin/env python3
"""Behavioural tests for scripts/site-preview-publish.py (hosted PR previews).

The publisher extracts PR-built archives into a public site, so its archive rules are the
trust boundary: only regular files and directories, confined to the preview directory,
within the size budget, and nothing written when any entry breaks a rule. It must also
prefer a build of the PR's current head over a newer run of an older commit.
Hermetic and stdlib-only; no network.
"""

import importlib.util
import io
import pathlib
import stat
import tempfile
import unittest
import zipfile

SCRIPT = pathlib.Path(__file__).resolve().parents[1] / "site-preview-publish.py"
spec = importlib.util.spec_from_file_location("site_preview_publish", SCRIPT)
publish = importlib.util.module_from_spec(spec)
spec.loader.exec_module(publish)

REG = stat.S_IFREG | 0o644


def archive(*entries):
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w") as zf:
        for name, data, mode in entries:
            info = zipfile.ZipInfo(name)
            info.external_attr = mode << 16
            zf.writestr(info, data)
    return buf.getvalue()


class SafeExtract(unittest.TestCase):
    def setUp(self):
        self.tmp = pathlib.Path(tempfile.mkdtemp())
        self.dest = self.tmp / "pr-1"

    def files(self):
        return sorted(str(p.relative_to(self.tmp)) for p in self.tmp.rglob("*"))

    def test_extracts_regular_files_and_directories(self):
        blob = archive(("index.html", "<p>hi</p>", REG), ("specs/", "", stat.S_IFDIR | 0o755),
                       ("specs/a/index.html", "x", REG))
        self.assertEqual(publish.safe_extract(blob, self.dest, 1000), len("<p>hi</p>") + 1)
        self.assertEqual(self.files(), ["pr-1", "pr-1/index.html", "pr-1/specs",
                                        "pr-1/specs/a", "pr-1/specs/a/index.html"])

    def test_rejects_without_writing_anything(self):
        bad = {
            "symlink": ("link", "/etc", stat.S_IFLNK | 0o777),
            "parent escape": ("../outside.html", "x", REG),
            "absolute path": ("/abs.html", "x", REG),
            "device": ("dev", "", stat.S_IFCHR | 0o644),
        }
        for label, entry in bad.items():
            with self.subTest(label):
                blob = archive(("index.html", "ok", REG), entry)
                with self.assertRaises(ValueError):
                    publish.safe_extract(blob, self.dest, 1000)
                self.assertEqual(self.files(), [])

    def test_budget_is_checked_before_extraction(self):
        blob = archive(("index.html", "0123456789", REG))
        with self.assertRaises(ValueError):
            publish.safe_extract(blob, self.dest, 9)
        self.assertEqual(self.files(), [])


class PickRun(unittest.TestCase):
    def test_current_head_beats_a_newer_run_of_an_old_commit(self):
        runs = [  # API order: newest first
            {"id": 3, "event": "pull_request", "head_sha": "old"},
            {"id": 2, "event": "pull_request", "head_sha": "head"},
        ]
        self.assertEqual(publish.pick_run(runs, "head")["id"], 2)

    def test_falls_back_to_the_newest_run_while_the_head_builds(self):
        runs = [{"id": 5, "event": "workflow_dispatch", "head_sha": "main"},
                {"id": 4, "event": "pull_request", "head_sha": "older"}]
        self.assertEqual(publish.pick_run(runs, "head")["id"], 5)

    def test_no_runs(self):
        self.assertIsNone(publish.pick_run([], "head"))


if __name__ == "__main__":
    unittest.main(verbosity=2)
