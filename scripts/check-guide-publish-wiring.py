#!/usr/bin/env python3
# [OPUS-5] CI lint (issue #5022): anti-drift guard for the mdBook guide's Pages mount.
#
# WHY THIS GATE EXISTS — the guide reaches the web through a wiring nobody re-derives:
#   GitHub Pages has ONE deploy slot and pages.yml owns it, so the guide is NOT published
#   by its own workflow. Per research/docs-site-single-sourcing-anti-drift.md §7 option (a)
#   it is built inside pages.yml and OVERLAID into that single artifact at out/guide/,
#   while docs.yml keeps a separate build-VALIDATE role on PRs. Three things must hold for
#   that arrangement to actually work, and each fails SILENTLY:
#
#   1. SHARED BUILDER. docs.yml's validate lane is only a meaningful gate if it builds what
#      pages.yml ships. Both must invoke scripts/build-guide.sh — which is where the
#      load-bearing "[ERROR]" log grep lives (mdBook EXITS 0 on a broken {{#include}},
#      rust-lang/mdBook#1094). Someone inlining `mdbook build` back into either lane drops
#      those teeth without any test going red.
#   2. PINNED-VERSION PARITY. Same reason: two lanes rendering with different mdbook
#      versions are not equivalent, so the validated guide is not the published one.
#   3. MOUNT AGREEMENT. mdBook stamps book.toml's `[output.html] site-url` into the guide's
#      generated 404.html as `<base href="...">`. GitHub Pages serves that directory-level
#      404 for /guide/<typo>. If site-url and the overlay sub-path ever disagree, every
#      asset on the guide's 404 page resolves against the wrong root — a broken page that
#      no build step notices, because both halves are individually valid.
#
#   pages.yml carries in-job assertions for its own artifact (the <base href> check and the
#   out/guide/ smoke check), but its build job is gated to the CI-written publish refs
#   (main / benchmark-data), so those assertions only speak AFTER the change has landed.
#   This gate runs on every PR, in-repo, so the drift is caught before merge.
#
# WHAT THIS CHECKS (deterministic, in-repo, NO network, NO mdbook, NO build):
#   - book/book.toml declares `site-url` under [output.html], equal to the mount ("/guide/").
#   - .github/workflows/pages.yml overlays the render into `out/guide/` and smoke-checks
#     `out/guide/index.html`, so a removed overlay or a removed check is red.
#   - pages.yml asserts the guide 404's `<base href="/guide/">` — the mount agreement.
#   - both pages.yml and docs.yml install a PINNED `mdbook@<version>` and the versions match.
#   - both call scripts/build-guide.sh.
#
#   It does NOT build the book or inspect a rendered artifact: that needs mdbook + the full
#   checkout and is already done, with teeth, in both workflow lanes. The durable invariant
#   worth gating on a PR is the WIRING.
#
# WHY A HAND-ROLLED PARSER (no PyYAML): this runs in the docs-quality `ci-scripts` job,
# which installs no Python deps (mirrors check-dashboard-publish-wiring.py /
# check-install-action-tool.py / coverage-gate.py --self-test). Every fact needed here is a
# literal substring or a one-line regex over the workflow text.
#
# EXIT: 0 when the guide's publish wiring is intact; 1 with a per-offence message otherwise.
#
# Usage:
#   check-guide-publish-wiring.py                # check this repo
#   check-guide-publish-wiring.py --root <dir>   # check <dir>
#   check-guide-publish-wiring.py --self-test    # hermetic logic self-test
#
# stdlib-only.

from __future__ import annotations

import argparse
import re
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

BOOK_TOML = "book/book.toml"
PAGES_WORKFLOW = ".github/workflows/pages.yml"
DOCS_WORKFLOW = ".github/workflows/docs.yml"
BUILD_SCRIPT = "scripts/build-guide.sh"

# The decided mount (issue #5022). `MOUNT` is the sub-path as it appears in a URL;
# `OVERLAY_DIR` is where pages.yml assembles it inside the Pages artifact.
MOUNT = "/guide/"
OVERLAY_DIR = "out/guide"

# `tool: mdbook@0.4.40` — the taiki-e/install-action pin, in either workflow.
_MDBOOK_PIN_RE = re.compile(r"^\s*tool:\s*mdbook@(?P<version>[0-9][^\s#]*)", re.MULTILINE)

# `site-url = "/guide/"` under [output.html]. Captured with the section it sits in so a
# stray site-url in another table cannot satisfy the gate.
_SITE_URL_RE = re.compile(r'^\s*site-url\s*=\s*"(?P<url>[^"]*)"', re.MULTILINE)
_TABLE_RE = re.compile(r"^\s*\[(?P<name>[^\]]+)\]\s*$", re.MULTILINE)


def mdbook_pin(workflow_text: str) -> str | None:
    """The pinned mdbook version a workflow installs, or None if it pins none."""
    m = _MDBOOK_PIN_RE.search(workflow_text)
    return m.group("version") if m else None


def executable_lines(workflow_text: str) -> str:
    """The workflow text with whole-line YAML comments stripped.

    EVERY substring check below runs against this, not the raw text, and that is
    load-bearing: both workflows DOCUMENT this arrangement in their header comments, so
    they legitimately contain the strings "scripts/build-guide.sh", "out/guide" and the
    /guide/ mount in prose. Against the raw text a check like `"scripts/build-guide.sh" in
    text` therefore stays green even after the real `run:` step is swapped back to an
    inline `mdbook build` — verified: that exact mutation survived a bare substring check
    while writing this gate, which is why it is written this way. A whole-line `#` is the
    only comment form these workflows use, so this stays dependency-free (no PyYAML)."""
    return "\n".join(
        line for line in workflow_text.splitlines() if not line.lstrip().startswith("#")
    )


def site_url_in_output_html(book_toml_text: str) -> str | None:
    """The `site-url` declared under [output.html], or None if absent/elsewhere.

    Tracks the current table while scanning so a `site-url` under some other table
    (or before any table) is NOT accepted — mdBook would ignore it there."""
    current_table: str | None = None
    for line in book_toml_text.splitlines():
        table = _TABLE_RE.match(line)
        if table:
            current_table = table.group("name").strip()
            continue
        m = _SITE_URL_RE.match(line)
        if m and current_table == "output.html":
            return m.group("url")
    return None


def find_offences(
    book_toml_text: str, pages_text: str, docs_text: str
) -> list[str]:
    """Return a list of human-readable wiring defects (empty == wired correctly)."""
    offences: list[str] = []

    # Prose mentions must not satisfy a wiring check — see executable_lines().
    pages_exec = executable_lines(pages_text)
    docs_exec = executable_lines(docs_text)

    # ---- 3. MOUNT AGREEMENT: book.toml site-url == the overlay sub-path ----
    site_url = site_url_in_output_html(book_toml_text)
    if site_url is None:
        offences.append(
            f"{BOOK_TOML}: no `site-url` under [output.html]. mdBook then defaults it to "
            f'"/", so the guide\'s generated 404.html gets `<base href="/">` and every '
            f"asset on it resolves against the SITE root instead of {MOUNT}."
        )
    elif site_url != MOUNT:
        offences.append(
            f'{BOOK_TOML}: [output.html] site-url is "{site_url}" but pages.yml publishes '
            f'the guide at "{MOUNT}". mdBook bakes site-url into the guide\'s 404.html as '
            f"`<base href=...>`, so these disagreeing breaks that page's assets."
        )

    # ---- pages.yml is the single producer: overlay + assertions must be present ----
    if OVERLAY_DIR not in pages_exec:
        offences.append(
            f"{PAGES_WORKFLOW}: no reference to `{OVERLAY_DIR}` — the guide overlay is gone, "
            f"so nothing assembles the render into the Pages artifact and {MOUNT} would 404."
        )
    if f'<base href="{MOUNT}">' not in pages_exec:
        offences.append(
            f'{PAGES_WORKFLOW}: the guide overlay no longer asserts `<base href="{MOUNT}">` '
            f"in the rendered 404.html. That assertion is what catches a site-url/mount "
            f"disagreement at build time; without it the mount can drift silently."
        )
    if f"{OVERLAY_DIR}/index.html" not in pages_exec:
        offences.append(
            f"{PAGES_WORKFLOW}: the assembled-artifact smoke check no longer asserts "
            f"`{OVERLAY_DIR}/index.html`. An empty or half-overlaid guide would then reach "
            f"deploy and 404 silently."
        )

    # ---- 1. SHARED BUILDER: both lanes build through the same script ----
    for label, text in ((PAGES_WORKFLOW, pages_exec), (DOCS_WORKFLOW, docs_exec)):
        if BUILD_SCRIPT not in text:
            offences.append(
                f"{label}: does not build the guide through `{BUILD_SCRIPT}`. Both the "
                f"validate lane (docs.yml) and the publish lane (pages.yml) must use the "
                f"shared builder, or the guide PRs validate stops being the guide that "
                f"ships — and the script is where the mdBook#1094 `[ERROR]` teeth live."
            )

    # ---- 2. PINNED-VERSION PARITY ----
    pages_pin = mdbook_pin(pages_exec)
    docs_pin = mdbook_pin(docs_exec)
    if pages_pin is None:
        offences.append(
            f"{PAGES_WORKFLOW}: installs no pinned `tool: mdbook@<version>` — the published "
            f"guide would be built by whatever mdbook happens to be available."
        )
    if docs_pin is None:
        offences.append(
            f"{DOCS_WORKFLOW}: installs no pinned `tool: mdbook@<version>`."
        )
    if pages_pin is not None and docs_pin is not None and pages_pin != docs_pin:
        offences.append(
            f"mdbook pin MISMATCH: {DOCS_WORKFLOW} validates with mdbook@{docs_pin} but "
            f"{PAGES_WORKFLOW} publishes with mdbook@{pages_pin}. The validate lane is only "
            f"a gate on the published guide if both render it with the same version."
        )

    return offences


# --------------------------------------------------------------------------- tests
# Faithful miniatures of the three real files. The self-test drives find_offences()
# against mutations of them, so each check is proven to have teeth without touching the
# live workflows.
_BOOK_OK = """\
[build]
build-dir = "book"

[preprocessor.link-fixup]
command = "python3 scripts/mdbook-rewrite-links.py"

[output.html]
site-url = "/guide/"
git-repository-url = "https://github.com/sparq-org/sparq"
"""

_PAGES_OK = """\
      - name: Install mdbook (pinned)
        uses: taiki-e/install-action@18b1216eba7f8039b0f8d131d5473787f0edce68 # v2.85.3
        with:
          tool: mdbook@0.4.40
      - name: Build the mdBook guide
        run: bash scripts/build-guide.sh book
      - name: Overlay mdBook guide (out/guide/)
        run: |
          rsync -a --delete "$guide_out"/ out/guide/
          if grep -q '<base href="/guide/">' out/guide/404.html; then echo ok; else exit 1; fi
      - name: Smoke-check assembled Pages artifact
        run: |
          for f in out/guide/index.html out/guide/404.html; do test -f "$f"; done
"""

_DOCS_OK = """\
      - name: Install mdbook (pinned)
        uses: taiki-e/install-action@18b1216eba7f8039b0f8d131d5473787f0edce68 # v2.85.3
        with:
          tool: mdbook@0.4.40
      - name: Build guide
        run: bash scripts/build-guide.sh book
"""


def self_test() -> int:
    cases: list[tuple[str, str, str, str, int]] = [
        # (label, book_toml, pages_yml, docs_yml, expected_offence_count)
        ("fully wired", _BOOK_OK, _PAGES_OK, _DOCS_OK, 0),
        # --- mount agreement ---
        (
            "site-url removed → default '/' breaks the guide 404",
            _BOOK_OK.replace('site-url = "/guide/"\n', ""),
            _PAGES_OK,
            _DOCS_OK,
            1,
        ),
        (
            "site-url points at a different mount",
            _BOOK_OK.replace('"/guide/"', '"/docs/"'),
            _PAGES_OK,
            _DOCS_OK,
            1,
        ),
        # A site-url outside [output.html] is ignored by mdBook → must NOT satisfy the gate.
        (
            "site-url in the wrong table is not accepted",
            _BOOK_OK.replace(
                '[output.html]\nsite-url = "/guide/"\n',
                '[build]\nsite-url = "/guide/"\n\n[output.html]\n',
            ),
            _PAGES_OK,
            _DOCS_OK,
            1,
        ),
        # --- producer wiring ---
        (
            "guide overlay dropped from pages.yml",
            _BOOK_OK,
            _PAGES_OK.replace("out/guide", "out/elsewhere"),
            _DOCS_OK,
            # Two independent offences: the overlay dir itself, and the smoke check that
            # asserted the guide entry point. The `<base href="/guide/">` assertion mentions
            # the URL mount, not the artifact dir, so it survives this mutation — which is
            # exactly why it is checked separately below.
            2,
        ),
        (
            "404 <base href> assertion removed from the overlay",
            _BOOK_OK,
            _PAGES_OK.replace('<base href="/guide/">', "<html>"),
            _DOCS_OK,
            1,
        ),
        (
            "artifact smoke check no longer asserts the guide entry point",
            _BOOK_OK,
            _PAGES_OK.replace("out/guide/index.html ", ""),
            _DOCS_OK,
            1,
        ),
        # --- shared builder ---
        (
            "pages.yml inlines `mdbook build` instead of the shared builder",
            _BOOK_OK,
            _PAGES_OK.replace("bash scripts/build-guide.sh book", "mdbook build book"),
            _DOCS_OK,
            1,
        ),
        (
            "docs.yml inlines `mdbook build` instead of the shared builder",
            _BOOK_OK,
            _PAGES_OK,
            _DOCS_OK.replace("bash scripts/build-guide.sh book", "mdbook build book"),
            1,
        ),
        # --- pin parity ---
        # --- prose must not satisfy wiring (the bug this gate shipped with, then fixed) ---
        # Both real workflows document this arrangement in their headers, so they contain
        # "scripts/build-guide.sh" / "out/guide" / the mount in COMMENTS. A bare substring
        # check therefore stayed green when the real `run:` step was swapped back to an inline
        # `mdbook build` — verified against the live pages.yml. These cases pin the fix.
        (
            "comment-only mention of the shared builder does NOT count (pages.yml)",
            _BOOK_OK,
            _PAGES_OK.replace(
                "        run: bash scripts/build-guide.sh book",
                "        # built via scripts/build-guide.sh (see header)\n"
                "        run: mdbook build book",
            ),
            _DOCS_OK,
            1,
        ),
        (
            "comment-only mention of the overlay dir + mount does NOT count",
            _BOOK_OK,
            "# the guide is overlaid at out/guide/ with <base href=\"/guide/\">\n"
            "# and smoke-checked at out/guide/index.html\n"
            + _PAGES_OK.replace("out/guide", "out/elsewhere").replace(
                '<base href="/guide/">', "<html>"
            ),
            _DOCS_OK,
            3,
        ),
        (
            "mdbook pins diverge between the two lanes",
            _BOOK_OK,
            _PAGES_OK.replace("mdbook@0.4.40", "mdbook@0.4.52"),
            _DOCS_OK,
            1,
        ),
        (
            "pages.yml stops pinning mdbook",
            _BOOK_OK,
            _PAGES_OK.replace("tool: mdbook@0.4.40", "tool: mdbook"),
            _DOCS_OK,
            1,
        ),
        (
            "docs.yml stops pinning mdbook",
            _BOOK_OK,
            _PAGES_OK,
            _DOCS_OK.replace("tool: mdbook@0.4.40", "tool: mdbook"),
            1,
        ),
    ]
    failures = 0
    for label, book, pages, docs, expected in cases:
        got = len(find_offences(book, pages, docs))
        ok = got == expected
        print(
            f"  [{'PASS' if ok else 'FAIL'}] {label}: "
            f"{got} offence(s) (want {expected})"
        )
        if not ok:
            for off in find_offences(book, pages, docs):
                print(f"         | {off}")
            failures += 1
    if failures:
        print(f"\nself-test: {failures} case(s) FAILED")
        return 1
    print("\nself-test: all cases PASS")
    return 0


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(
        description="Fail if the mdBook guide's Pages publish wiring has drifted — the "
        "/guide/ mount, the shared builder, and the mdbook pin parity between the "
        "validate (docs.yml) and publish (pages.yml) lanes (issue #5022)."
    )
    ap.add_argument(
        "--root",
        default=str(REPO_ROOT),
        help="repo root to check (default: this repo).",
    )
    ap.add_argument(
        "--self-test",
        action="store_true",
        help="run the hermetic logic self-test and exit.",
    )
    args = ap.parse_args(argv)

    if args.self_test:
        return self_test()

    root = Path(args.root)
    texts: dict[str, str] = {}
    for rel in (BOOK_TOML, PAGES_WORKFLOW, DOCS_WORKFLOW):
        path = root / rel
        if not path.is_file():
            print(f"FAIL: cannot find {rel} under {root}")
            return 1
        texts[rel] = path.read_text(encoding="utf-8", errors="ignore")

    if not (root / BUILD_SCRIPT).is_file():
        print(f"FAIL: cannot find {BUILD_SCRIPT} under {root} — both the validate and the")
        print("publish lane invoke it; without it neither can build the guide.")
        return 1

    offences = find_offences(
        texts[BOOK_TOML], texts[PAGES_WORKFLOW], texts[DOCS_WORKFLOW]
    )

    if not offences:
        print(
            "guide publish-wiring gate: PASS — the mdBook guide is wired to publish at "
            f"{MOUNT} (pages.yml overlays {OVERLAY_DIR}/ and smoke-checks it; both lanes "
            f"build via {BUILD_SCRIPT} with the same pinned mdbook; book.toml's site-url "
            "matches the mount)."
        )
        return 0

    print("guide publish-wiring gate: FAIL\n")
    print(
        "The mdBook guide reaches https://sparq.jeswr.org/guide/ through pages.yml's\n"
        "artifact overlay (issue #5022; research/docs-site-single-sourcing-anti-drift.md\n"
        "§7 option (a)) — GitHub Pages has ONE deploy slot, so the guide has no workflow\n"
        "of its own. Each defect below breaks that path SILENTLY:\n"
    )
    for off in offences:
        print(f"    - {off}")
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
