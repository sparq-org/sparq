#!/usr/bin/env python3
# [OPUS-4.8] Hermetic tests for the proactive merge-gate scripts G1/G2/G6 (beads
# sq-ncvq.4 + sq-ncvq.5 + sq-ncvq.9, epic sq-ncvq). Authored by Opus 4.8 (Fable
# unavailable; flag for re-review when Fable returns).
#
# Hermetic w.r.t. git/network: imports scripts/gate-new-crate.py +
# scripts/gate-api-skill.py + scripts/check-config-documented.py and drives their
# pure evaluate()/main() entry points against FIXTURE diff listings. NO live git
# and NO subprocess — the evaluate()-level tests inject every git/subprocess fact
# (crate stub status, bench registration, `pub`-diff heuristic, G6 net-added knob
# tokens + documented-token sets) via the *_overrides kwargs, and main() is driven
# with --changed-files fixtures + --dry-run so it never shells out. [OPUS-4.8]
# (Caveat: the main() smoke tests call main() WITHOUT overrides, so they may still
# consult the in-repo bench/benchmarks.toml or the crate READMEs/SKILL.md on disk —
# those are committed files, not git/network state, so the runs stay deterministic.)
#
# Run:  python3 scripts/tests/test_gates.py
# (stdlib only; no pytest required — also discoverable by `pytest`.)

from __future__ import annotations

import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent


def _load(name: str, filename: str):
    spec = importlib.util.spec_from_file_location(name, REPO_ROOT / "scripts" / filename)
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


g1 = _load("gate_new_crate", "gate-new-crate.py")
g2 = _load("gate_api_skill", "gate-api-skill.py")
g6 = _load("check_config_documented", "check-config-documented.py")
# [SONNET-4.6] (sq-5owmc) the merge_group PR-number resolver used by the G2/G6
# "Read PR labels" steps to honour the escape-hatch label in the merge queue.
resolve_mg = _load("resolve_merge_group_pr", "resolve-merge-group-pr.py")


def _statused(added: list[str], modified: list[str] | None = None) -> list[str]:
    """Build a `git diff --name-status`-style fixture listing."""
    lines = [f"A\t{p}" for p in added]
    lines += [f"M\t{p}" for p in (modified or [])]
    return lines


def _write(tmp: Path, name: str, lines: list[str]) -> str:
    p = tmp / name
    p.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return str(p)


# --------------------------------------------------------------------------- #
# G1 — new-crate-completeness
# --------------------------------------------------------------------------- #
class G1Test(unittest.TestCase):
    def test_new_crate_with_nothing_else_fails(self):
        changed, added = g1.parse_status_lines(
            _statused(["crates/sparq-foo/Cargo.toml"])
        )
        violations = g1.evaluate(
            changed,
            added,
            stub_overrides={"sparq-foo": False},
            bench_overrides={"sparq-foo": False},
        )
        self.assertEqual(len(violations), 1)
        crate, missing = violations[0]
        self.assertEqual(crate, "sparq-foo")
        # All three artifacts are missing: README, bench, SKILL.
        self.assertEqual(len(missing), 3)
        joined = " ".join(missing)
        self.assertIn("README", joined)
        self.assertIn("benchmark", joined)
        self.assertIn("SKILL.md", joined)

    def test_new_crate_with_bench_readme_skill_passes(self):
        added = ["crates/sparq-foo/Cargo.toml", "crates/sparq-foo/README.md"]
        modified = ["skills/sparq-foo/SKILL.md", "bench/benchmarks.toml"]
        changed, added_paths = g1.parse_status_lines(_statused(added, modified))
        violations = g1.evaluate(
            changed,
            added_paths,
            stub_overrides={"sparq-foo": False},
            bench_overrides={"sparq-foo": True},
        )
        self.assertEqual(violations, [])

    def test_stub_crate_needs_only_readme(self):
        # publish = false → bench + SKILL are waived; README still required.
        added = ["crates/sparq-foo/Cargo.toml", "crates/sparq-foo/README.md"]
        changed, added_paths = g1.parse_status_lines(_statused(added))
        violations = g1.evaluate(
            changed,
            added_paths,
            stub_overrides={"sparq-foo": True},
            bench_overrides={"sparq-foo": False},
        )
        self.assertEqual(violations, [])

    def test_stub_crate_missing_readme_still_fails(self):
        changed, added = g1.parse_status_lines(
            _statused(["crates/sparq-foo/Cargo.toml"])
        )
        violations = g1.evaluate(
            changed, added, stub_overrides={"sparq-foo": True}
        )
        self.assertEqual(len(violations), 1)
        _, missing = violations[0]
        self.assertEqual(len(missing), 1)
        self.assertIn("README", missing[0])

    def test_no_new_crate_passes(self):
        changed, added = g1.parse_status_lines(
            _statused([], ["crates/sparq-cli/src/main.rs"])
        )
        self.assertEqual(g1.evaluate(changed, added), [])

    def test_changed_not_added_cargo_does_not_trigger(self):
        # An edit to an EXISTING crate's Cargo.toml is not a "new crate".
        changed, added = g1.parse_status_lines(
            _statused([], ["crates/sparq-cli/Cargo.toml"])
        )
        self.assertEqual(g1.added_crates(added), [])
        self.assertEqual(g1.evaluate(changed, added), [])

    def test_copied_crate_counts_as_added(self):
        # [OPUS-4.8] `git diff -C` reports a newly-introduced path as a copy
        # (`C`/`C100`, "C<score>\t<old>\t<new>"); the destination must still be
        # treated as added so a copy can't evade new-crate detection.
        changed, added = g1.parse_status_lines(
            ["C100\tcrates/sparq-old/Cargo.toml\tcrates/sparq-new/Cargo.toml"]
        )
        self.assertIn("crates/sparq-new/Cargo.toml", added)
        self.assertEqual(g1.added_crates(added), ["sparq-new"])


# --------------------------------------------------------------------------- #
# G1 — the `<!-- flow-on-exempt: reason -->` crate-README escape hatch (#5701)
# --------------------------------------------------------------------------- #
class G1FlowOnExemptTest(unittest.TestCase):
    def _bare_new_crate(self):
        return g1.parse_status_lines(_statused(["crates/sparq-foo/Cargo.toml"]))

    def test_exempt_readme_waives_the_whole_gate(self):
        changed, added = self._bare_new_crate()
        # Without the marker this diff is a 3-violation FAIL (see G1Test above).
        self.assertTrue(
            g1.evaluate(
                changed,
                added,
                stub_overrides={"sparq-foo": False},
                bench_overrides={"sparq-foo": False},
            )
        )
        self.assertEqual(
            g1.evaluate(
                changed,
                added,
                stub_overrides={"sparq-foo": False},
                bench_overrides={"sparq-foo": False},
                exempt_overrides={"sparq-foo": "vendored fork, tracked in sq-xxxx"},
            ),
            [],
        )

    def test_reason_is_required(self):
        # The reason IS the audit record, so a marker without one waives nothing.
        self.assertIsNone(g1.flow_on_exempt_reason("<!-- flow-on-exempt: -->"))
        self.assertIsNone(g1.flow_on_exempt_reason("<!-- flow-on-exempt -->"))
        self.assertIsNone(g1.flow_on_exempt_reason("no marker here"))
        self.assertIsNone(g1.flow_on_exempt_reason(None))
        self.assertEqual(
            g1.flow_on_exempt_reason("intro\n<!-- flow-on-exempt: bench lands in sq-1 -->\n"),
            "bench lands in sq-1",
        )

    def test_reasonless_marker_is_reported_as_malformed(self):
        self.assertTrue(g1.flow_on_exempt_marker_is_malformed("<!-- flow-on-exempt -->"))
        self.assertFalse(
            g1.flow_on_exempt_marker_is_malformed("<!-- flow-on-exempt: why -->")
        )
        self.assertFalse(g1.flow_on_exempt_marker_is_malformed("# sparq-foo"))

    def test_marker_is_read_from_the_crate_readme_on_disk(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            crate = root / "crates" / "sparq-foo"
            crate.mkdir(parents=True)
            (crate / "README.md").write_text(
                "# sparq-foo\n\n<!-- flow-on-exempt: internal fork, bead sq-9 -->\n",
                encoding="utf-8",
            )
            self.assertEqual(
                g1.crate_flow_on_exempt("sparq-foo", root), "internal fork, bead sq-9"
            )
            self.assertEqual(
                g1.exempt_crates(["crates/sparq-foo/Cargo.toml"], root),
                {"sparq-foo": "internal fork, bead sq-9"},
            )
            # A crate with no README at all is never exempt.
            self.assertIsNone(g1.crate_flow_on_exempt("sparq-absent", root))

    def test_marker_only_exempts_the_crate_that_carries_it(self):
        changed, added = g1.parse_status_lines(
            _statused(["crates/sparq-foo/Cargo.toml", "crates/sparq-bar/Cargo.toml"])
        )
        violations = g1.evaluate(
            changed,
            added,
            stub_overrides={"sparq-foo": False, "sparq-bar": False},
            bench_overrides={"sparq-foo": False, "sparq-bar": False},
            exempt_overrides={"sparq-foo": "tracked in sq-xxxx"},
        )
        self.assertEqual([c for c, _ in violations], ["sparq-bar"])


# --------------------------------------------------------------------------- #
# G2 — public-api → skill
# --------------------------------------------------------------------------- #
class G2Test(unittest.TestCase):
    # [OPUS-4.8] G2 now fires ONLY on a NET `pub `-item signature change in a
    # published crate's src/** (no more blanket binding-crate trip). The
    # evaluate()-level tests inject the pub_api_changed verdict via pub_overrides
    # so they stay hermetic (no live git); the _scan_pub_diff tests exercise the
    # added/removed-multiset logic directly.

    def test_server_pub_change_without_skill_fails(self):
        # A real pub-item change in a binding crate's src, no SKILL → FAIL.
        path = "crates/sparq-server/src/routes.rs"
        ok, hits = g2.evaluate(
            [path], labels=[], base="main", pub_overrides={path: True}
        )
        self.assertFalse(ok)
        self.assertIn(path, hits)

    def test_server_pub_change_with_skill_passes(self):
        path = "crates/sparq-server/src/routes.rs"
        ok, _ = g2.evaluate(
            [path, "skills/http-server/SKILL.md"],
            labels=[],
            base="main",
            pub_overrides={path: True},
        )
        self.assertTrue(ok)

    def test_binding_crate_comment_only_change_passes(self):
        # [OPUS-4.8] REGRESSION (blocked #250): a comment-/attribute-only edit to a
        # BINDING crate's src is NOT a public-surface change — it must PASS even
        # without a SKILL.md. pub_overrides={...: False} models "no net pub change".
        path = "crates/sparq-cli/src/main.rs"
        ok, hits = g2.evaluate(
            [path], labels=[], base="main", pub_overrides={path: False}
        )
        self.assertTrue(ok)
        self.assertEqual(hits, [])

    def test_ci_only_change_passes(self):
        # [OPUS-4.8] REGRESSION (#244 framing): a PR that touches NO crates/*/src/**
        # — only CI workflows / tests / non-src files — can never reach the `pub`
        # check, so G2 always passes. No pub_overrides needed: these paths never hit
        # git because _CRATE_SRC_RE rejects them.
        changed = [
            ".github/workflows/feature-matrix.yml",
            "crates/sparq-reason/tests/explain_owl.rs",
            "compliance/asvs/gap-register.md",
        ]
        ok, hits = g2.evaluate(changed, labels=[], base="main")
        self.assertTrue(ok)
        self.assertEqual(hits, [])

    def test_pub_item_relocation_does_not_trip(self):
        # [OPUS-4.8] REGRESSION (mis-fired #244): a pure relocation of a pub item —
        # the identical signature added once and removed once within a file — is
        # net-zero and must NOT count as a public-API change. Drive the pure
        # _scan_pub_diff + pub_api_changed multiset logic directly.
        diff = [
            "--- a/crates/sparq-reason/src/explain.rs",
            "+++ b/crates/sparq-reason/src/explain.rs",
            "@@ -257,1 +257,1 @@",
            "+pub fn n3_proof_tree(",
            "@@ -327,1 +400,0 @@",
            "-pub fn n3_proof_tree(",
        ]
        added, removed = g2._scan_pub_diff(diff)
        self.assertEqual(sorted(added), sorted(removed))  # cancels out
        # And a genuinely NEW export does trip (added with no matching removal).
        diff2 = [
            "+++ b/crates/sparq-core/src/store.rs",
            "+pub fn brand_new_export() {}",
        ]
        added2, removed2 = g2._scan_pub_diff(diff2)
        self.assertNotEqual(sorted(added2), sorted(removed2))

    def test_rustfmt_line_wrap_does_not_trip(self):
        # [OPUS-4.8] REGRESSION (false-positive, tripped #469 — bead sq-5x2i): a
        # one-shot `cargo fmt` LINE-WRAPS a long `pub fn` signature. The diff
        # REMOVES the single-line form and ADDS the multi-line wrapped form (same
        # tokens, only rustfmt whitespace + an inserted trailing comma). The two
        # must canonicalise EQUAL so a pure reformat is NOT a public-surface change
        # — no `skill-not-needed` label needed on fmt-only PRs. This is the exact
        # `compress_chunks` reflow from #469 (crates/sparq-parse/src/lib.rs).
        diff = [
            "--- a/crates/sparq-parse/src/lib.rs",
            "+++ b/crates/sparq-parse/src/lib.rs",
            "@@ -40,7 +40,12 @@",
            "-pub fn compress_chunks<S: AsRef<[u8]> + Sync>(chunks: &[S], codec: "
            "&Codec, mode: Mode) -> io::Result<Vec<Vec<u8>>> {",
            "+pub fn compress_chunks<S: AsRef<[u8]> + Sync>(",
            "+    chunks: &[S],",
            "+    codec: &Codec,",
            "+    mode: Mode,",
            "+) -> io::Result<Vec<Vec<u8>>> {",
            "     match mode {",
        ]
        added, removed = g2._scan_pub_diff(diff)
        self.assertEqual(sorted(added), sorted(removed))  # reflow cancels out
        self.assertFalse(g2._pad.diff_has_net_pub_change(diff))

    def test_rustfmt_unwrap_does_not_trip(self):
        # [OPUS-4.8] The symmetric direction: rustfmt UN-wraps a previously wrapped
        # signature (removes the multi-line form, adds the one-liner). Same key.
        diff = [
            "+++ b/crates/sparq-parse/src/lib.rs",
            "-pub fn gzip_single_member<S: AsRef<[u8]>>(",
            "-    chunks: &[S],",
            "-    level: u32,",
            "-    mode: Mode,",
            "-) -> io::Result<Vec<Vec<u8>>> {",
            "+pub fn gzip_single_member<S: AsRef<[u8]>>(chunks: &[S], level: u32, "
            "mode: Mode) -> io::Result<Vec<Vec<u8>>> {",
        ]
        self.assertFalse(g2._pad.diff_has_net_pub_change(diff))

    def test_real_signature_change_still_trips_after_normalize(self):
        # [OPUS-4.8] Normalisation must NOT mask a GENUINE signature change: if a
        # param TYPE actually changes (Mode -> FastMode), the wrapped-added key
        # differs from the one-line-removed key and the gate still fires.
        diff = [
            "+++ b/crates/sparq-parse/src/lib.rs",
            "-pub fn compress_chunks<S: AsRef<[u8]> + Sync>(chunks: &[S], codec: "
            "&Codec, mode: Mode) -> io::Result<Vec<Vec<u8>>> {",
            "+pub fn compress_chunks<S: AsRef<[u8]> + Sync>(",
            "+    chunks: &[S],",
            "+    codec: &Codec,",
            "+    mode: FastMode,",
            "+) -> io::Result<Vec<Vec<u8>>> {",
        ]
        self.assertTrue(g2._pad.diff_has_net_pub_change(diff))

    def test_wrapped_use_and_const_reflow_cancel(self):
        # [OPUS-4.8] Wrapping is not fn-only: a long `pub use` re-export and a
        # `pub const` can reflow too. The `;`/`=` terminators must close the
        # accumulation and the keys must cancel against the one-line forms.
        use_diff = [
            "+++ b/crates/sparq-core/src/lib.rs",
            "-pub use crate::store::{Alpha, Beta, Gamma, Delta, Epsilon, Zeta};",
            "+pub use crate::store::{",
            "+    Alpha, Beta, Gamma, Delta, Epsilon, Zeta,",
            "+};",
        ]
        self.assertFalse(g2._pad.diff_has_net_pub_change(use_diff))

    def test_wrapped_where_clause_reflow_cancels(self):
        # [OPUS-4.8] rustfmt commonly hoists a `where`-clause onto its own lines
        # and adds a trailing comma after the last bound — which lands right
        # before the body `{`. The wrapped and one-line forms must canonicalise
        # equal (trailing comma before `{` dropped), but a GENUINE bound change
        # (Send -> Sync) must still trip.
        reflow = [
            "+++ b/crates/x/src/a.rs",
            "-pub fn f<T>(x: T) -> T where T: Clone + Send {",
            "+pub fn f<T>(x: T) -> T",
            "+where",
            "+    T: Clone + Send,",
            "+{",
        ]
        self.assertFalse(g2._pad.diff_has_net_pub_change(reflow))
        changed = [
            "+++ b/crates/x/src/a.rs",
            "-pub fn f<T>(x: T) -> T where T: Clone + Send {",
            "+pub fn f<T>(x: T) -> T",
            "+where",
            "+    T: Clone + Sync,",
            "+{",
        ]
        self.assertTrue(g2._pad.diff_has_net_pub_change(changed))

    def test_wrapped_tuple_struct_reflow_cancels(self):
        # [OPUS-4.8] A long tuple-struct (terminates at `;`, not a body `{`) that
        # rustfmt wraps one field per line must also cancel against its one-liner.
        diff = [
            "+++ b/crates/x/src/a.rs",
            "-pub struct P(pub u64, pub u64, pub u64, pub u64, pub u64, pub u64);",
            "+pub struct P(",
            "+    pub u64,",
            "+    pub u64,",
            "+    pub u64,",
            "+    pub u64,",
            "+    pub u64,",
            "+    pub u64,",
            "+);",
        ]
        self.assertFalse(g2._pad.diff_has_net_pub_change(diff))

    def test_normalize_signature_is_wrap_invariant(self):
        # [OPUS-4.8] Unit-level: the canonical key is identical for the one-line
        # and wrapped forms (whitespace stripped + trailing comma dropped).
        one = "pub fn f<T: Clone>(a: T, b: T) -> Vec<T> {"
        wrapped = "pub fn f<T: Clone>( a: T, b: T, ) -> Vec<T> {"
        self.assertEqual(
            g2._pad.normalize_signature(one),
            g2._pad.normalize_signature(wrapped),
        )

    def test_pub_item_regex_matches_all_item_forms_excludes_restricted(self):
        # [OPUS-4.8] The pub-item pattern must match every exported FORM
        # (fn/struct/enum/trait/const/type/mod/use) but NOT restricted
        # visibilities (`pub(crate)`/`pub(super)`/`pub(in …)`) nor a struct
        # `pub` field. Guard against silent drift of the source pattern.
        for exported in (
            "+pub fn foo()",
            "-pub use crate::X;",
            "+    pub struct S",
            "+pub enum E {",
            "-pub trait T {",
            "+pub const C: u8 = 0;",
            "+pub type Alias = u8;",
            "+pub mod m;",
        ):
            self.assertTrue(g2._PUB_ITEM_RE.match(exported), exported)
        for not_an_export in (
            "+pub(crate) fn foo()",
            "-pub(super) struct S",
            "+    pub(in crate::a) const X: u8 = 0;",
            "+    pub name: String,",  # struct field, not an item
            "+publicize();",  # `pub` not a whole word
        ):
            self.assertFalse(g2._PUB_ITEM_RE.match(not_an_export), not_an_export)

    def test_cli_change_suppressed_by_skill_not_needed_label(self):
        # Even a real pub change is suppressed by the escape-hatch label.
        path = "crates/sparq-cli/src/main.rs"
        ok, _ = g2.evaluate(
            [path],
            labels=["skill-not-needed"],
            base="main",
            pub_overrides={path: True},
        )
        self.assertTrue(ok)

    def test_published_crate_pub_change_without_skill_fails(self):
        # A pub API change in a published (non-binding) crate is in scope.
        path = "crates/sparq-core/src/store.rs"
        ok, hits = g2.evaluate(
            [path], labels=[], base="main", pub_overrides={path: True}
        )
        # crate_is_published reads disk; sparq-core has no publish=false, so it's
        # published. The injected pub_override makes it a pub change.
        self.assertFalse(ok)
        self.assertIn(path, hits)

    def test_published_crate_nonpub_change_passes(self):
        # A non-pub internal change is NOT a public surface.
        path = "crates/sparq-core/src/internal.rs"
        ok, hits = g2.evaluate(
            [path], labels=[], base="main", pub_overrides={path: False}
        )
        self.assertTrue(ok)
        self.assertEqual(hits, [])

    def test_non_surface_change_passes(self):
        ok, hits = g2.evaluate(
            ["research/notes.md", "bench/watdiv/run.sh"], labels=[], base="main"
        )
        self.assertTrue(ok)
        self.assertEqual(hits, [])


# --------------------------------------------------------------------------- #
# G6 — new-config/flag → docs
# --------------------------------------------------------------------------- #
class G6Test(unittest.TestCase):
    # [OPUS-4.8] G6 fires on a NET-added CLI flag literal / SPARQ_* env var in a
    # sparq-cli|sparq-server src file that no docs surface documents. The
    # evaluate()-level tests inject the net-added-knob set (code_knobs), the
    # docs-added set (doc_added) and the on-disk documented set (doc_disk) so they
    # stay hermetic (no live git, no disk). The token-extraction + net-diff helpers
    # are exercised directly.

    SERVER_MAIN = "crates/sparq-server/src/main.rs"
    CLI_MAIN = "crates/sparq-cli/src/main.rs"

    def test_new_flag_without_docs_fails(self):
        ok, undoc = g6.evaluate(
            [self.SERVER_MAIN],
            labels=[],
            base="main",
            code_knobs={self.SERVER_MAIN: {"--new-knob"}},
            doc_added=set(),
            doc_disk=set(),
        )
        self.assertFalse(ok)
        self.assertEqual([k for k, _ in undoc], ["--new-knob"])
        self.assertEqual(undoc[0][1], self.SERVER_MAIN)

    def test_new_flag_documented_in_same_diff_passes(self):
        # The same PR adds the flag to the crate README → documented.
        ok, undoc = g6.evaluate(
            [self.SERVER_MAIN, "crates/sparq-server/README.md"],
            labels=[],
            base="main",
            code_knobs={self.SERVER_MAIN: {"--new-knob"}},
            doc_added={"--new-knob"},
            doc_disk=set(),
        )
        self.assertTrue(ok)
        self.assertEqual(undoc, [])

    def test_new_flag_documented_in_skill_in_same_diff_passes(self):
        ok, undoc = g6.evaluate(
            [self.CLI_MAIN, "skills/cli/SKILL.md"],
            labels=[],
            base="main",
            code_knobs={self.CLI_MAIN: {"--new-knob"}},
            doc_added={"--new-knob"},
            doc_disk=set(),
        )
        self.assertTrue(ok)
        self.assertEqual(undoc, [])

    def test_new_env_var_without_docs_fails(self):
        ok, undoc = g6.evaluate(
            [self.CLI_MAIN],
            labels=[],
            base="main",
            code_knobs={self.CLI_MAIN: {"SPARQ_NEW_THING"}},
            doc_added=set(),
            doc_disk=set(),
        )
        self.assertFalse(ok)
        self.assertEqual([k for k, _ in undoc], ["SPARQ_NEW_THING"])

    def test_knob_already_documented_on_disk_passes(self):
        # A rewire of an EXISTING flag whose name is already written down on disk
        # (the README/SKILL was not touched in this PR) is still covered.
        ok, undoc = g6.evaluate(
            [self.SERVER_MAIN],
            labels=[],
            base="main",
            code_knobs={self.SERVER_MAIN: {"--audit-log"}},
            doc_added=set(),
            doc_disk={"--audit-log"},
        )
        self.assertTrue(ok)
        self.assertEqual(undoc, [])

    def test_config_internal_label_suppresses(self):
        ok, undoc = g6.evaluate(
            [self.SERVER_MAIN],
            labels=["config-internal"],
            base="main",
            code_knobs={self.SERVER_MAIN: {"--secret-internal"}},
            doc_added=set(),
            doc_disk=set(),
        )
        self.assertTrue(ok)
        self.assertEqual(undoc, [])

    def test_non_config_crate_src_never_trips(self):
        # A flag literal added in a NON-config crate's src can never reach the knob
        # check — out of scope. No code_knobs needed: config_src_changes filters it.
        ok, undoc = g6.evaluate(
            ["crates/sparq-core/src/store.rs"],
            labels=[],
            base="main",
            doc_added=set(),
            doc_disk=set(),
        )
        self.assertTrue(ok)
        self.assertEqual(undoc, [])

    def test_ci_only_change_passes(self):
        # No config src in the diff at all → always passes (mirrors G2).
        ok, undoc = g6.evaluate(
            [".github/workflows/ci.yml", "research/notes.md"],
            labels=[],
            base="main",
            doc_added=set(),
            doc_disk=set(),
        )
        self.assertTrue(ok)
        self.assertEqual(undoc, [])

    def test_knob_token_extraction(self):
        # CODE-side: flags are extracted from DOUBLE-QUOTED literals (how Rust source
        # writes them) + env vars; --help and short flags are NOT knobs; a
        # backtick-wrapped / bare flag is NOT a code token (code never writes those).
        toks = g6.code_knob_tokens(
            '            "--max-results" => { SPARQ_MAX_RESULTS } "--help" "-h"'
        )
        self.assertIn("--max-results", toks)
        self.assertIn("SPARQ_MAX_RESULTS", toks)
        self.assertNotIn("--help", toks)
        self.assertNotIn("-h", toks)
        # knob_tokens is the back-compat alias for the code extractor.
        self.assertEqual(g6.knob_tokens('"--addr"'), {"--addr"})
        self.assertEqual(g6.code_knob_tokens("`--addr`"), set())  # not a code form

    def test_doc_extractor_recognizes_backtick_flags(self):
        # THE FIX: docs write a flag in markdown BACKTICKS (`--addr`), the dominant
        # convention — and also `--flag <ARG>` / `--flag*` / `--flag N`, bare in
        # prose, and double-quoted. The doc extractor recognises ALL of these (so a
        # code-defined double-quoted flag matches its backtick-documented form),
        # while a bare `--` dash fragment and short flags are NOT picked up.
        self.assertEqual(g6.doc_knob_tokens("set `--addr` to bind"), {"--addr"})
        self.assertEqual(
            g6.doc_knob_tokens("`--access-audit <file|stderr>` writes audit"),
            {"--access-audit"},
        )
        self.assertEqual(
            g6.doc_knob_tokens("`--max-subscriptions*` family, `--max-results N`"),
            {"--max-subscriptions", "--max-results"},
        )
        self.assertEqual(g6.doc_knob_tokens("the --reason flag enables"), {"--reason"})
        self.assertEqual(g6.doc_knob_tokens('use "--proof" here'), {"--proof"})
        self.assertEqual(g6.doc_knob_tokens("SPARQ_ADDR is the env"), {"SPARQ_ADDR"})
        self.assertEqual(g6.doc_knob_tokens("a `git -- pathspec` fence"), set())
        self.assertNotIn("--help", g6.doc_knob_tokens("`--help` prints usage"))

    def test_code_flag_documented_only_in_backticks_passes(self):
        # END-TO-END through BOTH real extractors: a flag defined in code as a
        # double-quoted literal and documented ONLY in markdown backticks must be
        # recognised as documented (the asymmetric-extractor bug: a quote-only doc
        # extractor saw 0/29 flags and false-positived on the compliant path).
        src = "crates/sparq-server/src/main.rs"
        code_added = g6.scan_added_knobs(['+            "--addr" => addr = next(),'])
        doc_disk = g6.doc_knob_tokens("Bind address: `--addr <HOST:PORT>` (default …).")
        self.assertEqual(code_added, {"--addr"})
        self.assertIn("--addr", doc_disk)  # the two forms map to the same token
        ok, undoc = g6.evaluate(
            [src],
            labels=[],
            base="main",
            code_knobs={src: code_added},
            doc_added=set(),
            doc_disk=doc_disk,
        )
        self.assertTrue(ok)  # backtick-documented flag PASSES
        self.assertEqual(undoc, [])

    def test_code_flag_not_documented_anywhere_fails(self):
        # Conversely, a flag in code that no doc surface mentions (in ANY form)
        # FAILS — the gate is not merely permissive after the fix.
        src = "crates/sparq-cli/src/main.rs"
        code_added = g6.scan_added_knobs(['+            "--never-documented" => x,'])
        doc_disk = g6.doc_knob_tokens("Docs that mention `--addr` and `--reason` only.")
        ok, undoc = g6.evaluate(
            [src],
            labels=[],
            base="main",
            code_knobs={src: code_added},
            doc_added=set(),
            doc_disk=doc_disk,
        )
        self.assertFalse(ok)
        self.assertEqual([k for k, _ in undoc], ["--never-documented"])

    def test_real_repo_backcatalogue_flags_are_documented(self):
        # Regression guard against the asymmetric extractor on the REAL tree: every
        # double-quoted flag literal under the config crates' src must be recognised
        # as documented by the doc extractor reading the actual READMEs/SKILL.md.
        # (Pre-fix this was 0/29; the gate false-positived on its own clean tree.)
        flag_re = g6._CODE_FLAG_RE
        code_flags: set[str] = set()
        for crate in ("sparq-cli", "sparq-server"):
            for rs in (REPO_ROOT / "crates" / crate / "src").rglob("*.rs"):
                for m in flag_re.findall(rs.read_text(encoding="utf-8", errors="ignore")):
                    code_flags.add(m)
        code_flags.discard("--help")
        documented = g6.documented_on_disk()
        missing = sorted(code_flags - documented)
        self.assertGreaterEqual(len(code_flags), 20, "expected the back-catalogue flags")
        self.assertEqual(missing, [], f"undocumented back-catalogue flags: {missing}")

    def test_scan_added_knobs_net_diff(self):
        # A pure relocation (same flag removed once + added once) cancels; a
        # genuinely new flag is reported; a removed-only flag is not "added".
        diff = [
            "--- a/crates/sparq-server/src/main.rs",
            "+++ b/crates/sparq-server/src/main.rs",
            '+            "--relocated" => foo,',
            '-            "--relocated" => foo,',
            '+            "--brand-new" => bar,',
            '-            "--deleted" => baz,',
        ]
        added = g6.scan_added_knobs(diff)
        self.assertEqual(added, {"--brand-new"})

    def test_comment_mentioning_existing_flag_is_inert(self):
        # A doc-comment that merely names an existing flag adds the token, but the
        # token is already documented on disk → not undocumented. (Models the very
        # common "//! --audit-log does X" comment edit.)
        path = self.SERVER_MAIN
        ok, undoc = g6.evaluate(
            [path],
            labels=[],
            base="main",
            code_knobs={path: g6.scan_added_knobs(['+    // see --audit-log flag'])},
            doc_added=set(),
            doc_disk={"--audit-log"},
        )
        self.assertTrue(ok)
        self.assertEqual(undoc, [])


# --------------------------------------------------------------------------- #
# merge_group PR-number resolution — G2/G6 escape-hatch label in the queue
# [SONNET-4.6] (sq-5owmc)
# --------------------------------------------------------------------------- #
class MergeGroupPrResolveTest(unittest.TestCase):
    # [SONNET-4.6] (sq-5owmc) The G2/G6 "Read PR labels" steps must resolve the PR
    # number in merge_group context so the `skill-not-needed` / `config-internal`
    # escape-hatch labels are honoured in the merge queue. Before the fix the step
    # resolved the PR via `commits/<merge_group.head_sha>/pulls`; that returned
    # empty (synthetic merge commit / empty head_sha), pr-labels.txt was empty, the
    # label never suppressed, the gate failed the GROUP and the queue silently
    # ejected the PR (hit #1542 twice). The fix parses the PR number
    # DETERMINISTICALLY from the gh-readonly-queue head ref; these tests exercise
    # that pure parse (the network fallback + loud warn live in the workflow).

    def test_parses_pr_from_observed_ref_format(self):
        # The exact ref shape GitHub sets on github.event.merge_group.head_ref for
        # the single-PR merge group that ejected #1542.
        ref = "refs/heads/gh-readonly-queue/main/pr-1542-" + "0" * 40
        self.assertEqual(resolve_mg.parse_pr_number_from_ref(ref), 1542)

    def test_parses_ref_without_refs_heads_prefix(self):
        # head_ref may arrive with or without the leading refs/heads/.
        ref = "gh-readonly-queue/main/pr-42-" + "d" * 40
        self.assertEqual(resolve_mg.parse_pr_number_from_ref(ref), 42)

    def test_parses_ref_with_slashed_base_branch(self):
        # A base branch that itself contains a slash must not break the parse.
        ref = "refs/heads/gh-readonly-queue/release/v1/pr-7-" + "a" * 40
        self.assertEqual(resolve_mg.parse_pr_number_from_ref(ref), 7)

    def test_empty_ref_returns_none(self):
        # The fail-closed path: an empty/absent head_ref yields None so the workflow
        # warns LOUDLY and runs the gate unsuppressed (never silently passes).
        self.assertIsNone(resolve_mg.parse_pr_number_from_ref(""))
        self.assertIsNone(resolve_mg.parse_pr_number_from_ref(None))

    def test_non_queue_ref_returns_none(self):
        self.assertIsNone(resolve_mg.parse_pr_number_from_ref("refs/heads/feature/x"))
        self.assertIsNone(resolve_mg.parse_pr_number_from_ref("refs/heads/main"))

    def test_cli_entry_point(self):
        # The workflow invokes the module as a CLI: prints the number + exit 0 on a
        # successful parse; exit 1 with no output when unparseable.
        import contextlib
        import io

        ref = "refs/heads/gh-readonly-queue/main/pr-99-" + "f" * 40
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc_ok = resolve_mg.main(["prog", ref])
        self.assertEqual(rc_ok, 0)
        self.assertEqual(buf.getvalue().strip(), "99")

        buf_empty = io.StringIO()
        with contextlib.redirect_stdout(buf_empty):
            rc_empty = resolve_mg.main(["prog", ""])
        self.assertEqual(rc_empty, 1)
        self.assertEqual(buf_empty.getvalue().strip(), "")

        self.assertEqual(resolve_mg.main(["prog"]), 1)  # missing arg → exit 1


# --------------------------------------------------------------------------- #
# main() smoke (hermetic, via --changed-files + --dry-run; no git/network)
# --------------------------------------------------------------------------- #
class MainSmokeTest(unittest.TestCase):
    def test_g1_main_dry_run_reports_violation_but_exits_zero(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            f = _write(tmp, "diff.txt", _statused(["crates/sparq-zzznew/Cargo.toml"]))
            rc = g1.main(["--dry-run", "--changed-files", f])
            self.assertEqual(rc, 0)  # dry-run never fails

    def test_g2_main_dry_run_on_clean_diff_passes(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            f = _write(tmp, "diff.txt", ["research/foo.md"])
            rc = g2.main(["--dry-run", "--changed-files", f])
            self.assertEqual(rc, 0)

    def test_g6_main_dry_run_on_clean_diff_passes(self):
        # No config-surface src in the diff → no knob check → PASS (exit 0).
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            f = _write(tmp, "diff.txt", _statused([], ["research/foo.md"]))
            rc = g6.main(["--dry-run", "--changed-files", f])
            self.assertEqual(rc, 0)


# --------------------------------------------------------------------------- #
# comp_store_bytes_per_triple ratchet enforcement (sq-7d3dj.32.2.5) [SONNET-4.6]
# --------------------------------------------------------------------------- #
perf_gate = _load("perf_gate", "perf-gate.py")


class CompStoreRatchetTest(unittest.TestCase):
    """Mutation tests for the comp_store_bytes_per_triple ratchet.

    [SONNET-4.6] (sq-7d3dj.32.2.5) The compressed-profile B/triple floor must:
      1. HARD-FAIL (exit 2) when the measured value exceeds floor*(1+0.02).
      2. PASS (exit 0) when the measured value equals or is below the floor
         (including a genuine improvement that triggers the auto-ratchet-down).
    These use perf-gate.py's pure evaluate() / main() entry points so they are
    hermetic (no ci-bench.sh build needed, no network) — exactly the same
    pattern as the perf-gate self-test in scripts/perf-gate.py --self-test.
    """

    FLOOR = 56       # the seeded floor (B/triple, SPQCPRM2 V2 post-#1824)
    METRIC = "comp_store_bytes_per_triple"

    def _baseline(self, floor=None):
        f = self.FLOOR if floor is None else floor
        return {
            self.METRIC: {
                "floor": float(f),
                "threshold": 0.02,
                "mode": "auto",
            }
        }

    def test_regression_above_band_hard_fails(self):
        # MUTATION: inject a value ABOVE floor*(1+0.02) — simulates a compressed-
        # store regression where into_compressed() now uses more bytes per triple.
        # The gate must exit 2 (HARD FAIL) — the regression cannot drift undetected.
        regressed_value = self.FLOOR * 1.05  # +5%, clearly above the 2% band
        regressions, _ = perf_gate.evaluate(
            {self.METRIC: regressed_value}, self._baseline()
        )
        self.assertEqual(
            [r[0] for r in regressions],
            [self.METRIC],
            f"a regression of +5% above the floor must be caught; got regressions={regressions}",
        )

    def test_within_band_passes(self):
        # A within-band reading (at the floor itself, or at floor+1%) must PASS.
        for cur in (self.FLOOR, self.FLOOR * 1.01):
            regressions, _ = perf_gate.evaluate(
                {self.METRIC: cur}, self._baseline()
            )
            self.assertEqual(
                regressions,
                [],
                f"a within-band value ({cur:g}) must pass; got regressions={regressions}",
            )

    def test_genuine_improvement_passes_and_ratchets_down(self):
        # A value BELOW the floor is an improvement: gate passes, and
        # update_baseline should lower the floor to the new best-ever value.
        improved = self.FLOOR - 5  # e.g. 51 B/triple after a format optimisation
        regressions, _ = perf_gate.evaluate(
            {self.METRIC: improved}, self._baseline()
        )
        self.assertEqual(
            regressions, [], "an improved value must pass the gate"
        )
        new_bl, changes = perf_gate.update_baseline(
            {self.METRIC: improved}, self._baseline()
        )
        self.assertEqual(
            new_bl[self.METRIC]["floor"],
            improved,
            "the floor must ratchet DOWN to the genuine improvement",
        )
        self.assertTrue(
            any("auto-ratchet" in c[3] for c in changes),
            f"auto-ratchet-down change expected; got {changes}",
        )

    def test_boundary_exactly_at_plus_two_percent_passes(self):
        # Exactly at floor*(1+0.02) is boundary-inclusive (not strictly greater).
        boundary = self.FLOOR * 1.02
        regressions, _ = perf_gate.evaluate(
            {self.METRIC: boundary}, self._baseline()
        )
        self.assertEqual(
            regressions,
            [],
            f"a value exactly at the +2% boundary ({boundary:g}) must pass (inclusive)",
        )

    def test_one_byte_above_band_fails(self):
        # Even one integer unit above the boundary must trip the ratchet.
        just_over = self.FLOOR * 1.02 + 1  # 58.12 + 1 = 59.12 → FAIL
        regressions, _ = perf_gate.evaluate(
            {self.METRIC: just_over}, self._baseline()
        )
        self.assertEqual(
            [r[0] for r in regressions],
            [self.METRIC],
            f"a value just above the +2% band ({just_over:g}) must hard-fail",
        )


if __name__ == "__main__":
    unittest.main(verbosity=2)
