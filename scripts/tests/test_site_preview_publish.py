#!/usr/bin/env python3
"""Behavioural tests for scripts/site-preview-publish.py (hosted PR previews).

The publisher writes PR-built archives into a public site, so its rules are the trust
boundary: only regular files and directories confined to pr-<N>/, within the size budget,
and the existing tree untouched whenever an archive is refused or turns out corrupt
part-way through. It must also publish only same-repository pull_request builds of the
PR in question, preferring the build of the PR's current head. Hermetic and stdlib-only.
"""

import importlib.util
import pathlib
import stat
import tempfile
import unittest
import zipfile

SCRIPT = pathlib.Path(__file__).resolve().parents[1] / "site-preview-publish.py"
spec = importlib.util.spec_from_file_location("site_preview_publish", SCRIPT)
pub = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pub)

REG = stat.S_IFREG | 0o644


class Extract(unittest.TestCase):
    def setUp(self):
        self.tmp = pathlib.Path(tempfile.mkdtemp())
        self.out = self.tmp / "site"
        (self.out / "pr-7").mkdir(parents=True)
        (self.out / "pr-7" / "index.html").write_text("other preview")
        (self.out / "pr-1").mkdir()
        (self.out / "pr-1" / "old.html").write_text("old build")

    def archive(self, *entries):
        path = self.tmp / f"a{len(list(self.tmp.glob('a*.zip')))}.zip"
        with zipfile.ZipFile(path, "w") as zf:
            for name, data, mode in entries:
                info = zipfile.ZipInfo(name)
                info.external_attr = mode << 16
                zf.writestr(info, data)
        return path

    def snapshot(self):
        return sorted((str(p.relative_to(self.out)), p.read_text() if p.is_file() else None)
                      for p in self.out.rglob("*"))

    def test_replaces_the_preview_and_keeps_the_others(self):
        zpath = self.archive(("index.html", "new", REG), ("specs/", "", stat.S_IFDIR | 0o755),
                             ("specs/x/index.html", "spec", REG))
        self.assertEqual(pub.extract(zpath, self.out, 1, 1000), 7)
        self.assertEqual(self.snapshot(), [
            ("pr-1", None), ("pr-1/index.html", "new"), ("pr-1/specs", None),
            ("pr-1/specs/x", None), ("pr-1/specs/x/index.html", "spec"),
            ("pr-7", None), ("pr-7/index.html", "other preview"),
        ])

    def test_refuses_unsafe_entries_without_touching_the_tree(self):
        unsafe = {
            "symlink": ("link", "/etc", stat.S_IFLNK | 0o777),
            "parent escape": ("../pr-7/index.html", "x", REG),
            "nested escape": ("a/../../escape.html", "x", REG),
            "absolute path": ("/abs.html", "x", REG),
            "device": ("dev", "", stat.S_IFCHR | 0o644),
        }
        for label, entry in unsafe.items():
            with self.subTest(label):
                before = self.snapshot()
                zpath = self.archive(("index.html", "ok", REG), entry)
                with self.assertRaises(pub.Refused):
                    pub.extract(zpath, self.out, 1, 1000)
                self.assertEqual(self.snapshot(), before)

    def test_budget_is_checked_before_anything_is_written(self):
        zpath = self.archive(("index.html", "0123456789", REG))
        before = self.snapshot()
        with self.assertRaises(pub.Refused):
            pub.extract(zpath, self.out, 1, 9)
        self.assertEqual(self.snapshot(), before)
        self.assertEqual(pub.extract(zpath, self.out, 1, 10), 10)

    def test_corrupt_entry_mid_archive_leaves_the_tree_untouched(self):
        zpath = self.archive(("a.html", "first entry is fine", REG),
                             ("b.html", "second entry gets corrupted", REG))
        data = bytearray(zpath.read_bytes())
        at = data.index(b"second entry gets corrupted")
        data[at:at + 6] = b"XXXXXX"   # payload no longer matches its CRC
        zpath.write_bytes(bytes(data))
        before = self.snapshot()
        with self.assertRaises(pub.Refused):
            pub.extract(zpath, self.out, 1, 1000)
        self.assertEqual(self.snapshot(), before)

    def test_not_a_zip(self):
        zpath = self.tmp / "junk.zip"
        zpath.write_bytes(b"not a zip")
        with self.assertRaises(pub.Refused):
            pub.extract(zpath, self.out, 1, 1000)


def run(run_id, sha, prs, event="pull_request", repo="sparq-org/sparq"):
    return {"id": run_id, "head_sha": sha, "event": event,
            "head_repository": {"full_name": repo},
            "pull_requests": [{"number": n} for n in prs]}


class PickRun(unittest.TestCase):
    def setUp(self):
        self.calls = []
        self.responses = {}
        self.orig = pub.api

        def fake_api(path):
            self.calls.append(path)
            key = "head_sha" if "head_sha=" in path else "branch"
            return {"workflow_runs": self.responses.get(key, [])}

        pub.api = fake_api

    def tearDown(self):
        pub.api = self.orig

    def test_prefers_the_current_head_build(self):
        self.responses = {"head_sha": [run(2, "head", [5])],
                          "branch": [run(3, "old", [5]), run(2, "head", [5])]}
        self.assertEqual(pub.pick_run(5, "head", "feature")["id"], 2)

    def test_falls_back_to_the_newest_build_of_the_pr_while_the_head_builds(self):
        self.responses = {"branch": [run(4, "older", [5])]}
        self.assertEqual(pub.pick_run(5, "head", "feature")["id"], 4)

    def test_rejects_runs_of_other_prs_forks_and_other_events(self):
        self.responses = {"head_sha": [run(1, "head", [6]),
                                       run(2, "head", [5], repo="someone/fork"),
                                       run(3, "head", [5], event="workflow_dispatch")]}
        self.assertIsNone(pub.pick_run(5, "head", "feature"))

    def test_queries_are_bound_to_successful_pull_request_runs(self):
        pub.pick_run(5, "head", "a/b")
        self.assertTrue(all("event=pull_request&status=success" in c for c in self.calls))
        self.assertIn("branch=a%2Fb", self.calls[1])


if __name__ == "__main__":
    unittest.main(verbosity=2)
