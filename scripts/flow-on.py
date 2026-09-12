#!/usr/bin/env python3
# [OPUS-4.8] Merge-triggered flow-on engine (bead sq-ncvq.2, epic sq-ncvq).
# Authored by Opus 4.8 (Fable unavailable; flag for re-review when Fable returns).
#
# Reads a MERGED PR's changed-file list + labels + title, evaluates the
# declarative rules in scripts/flow-on-rules.toml, and opens ONE GitHub issue
# (labelled `flow-on` + `auto` + role/priority/status routing labels — see
# routing_labels(), #2474) per triggered follow-on template.
#
# WHY GITHUB ISSUES, NOT `bd create`:
# (research/maintenance-flow-on-automation-design.md §2.2) In CI there is NO `bd`
# dolt DB — it lives only in the orchestrator's main checkout and is gitignored,
# and hand-editing `.beads/` is forbidden. So this script must NOT touch `bd` or
# `.beads/`. Instead it emits each follow-on as a GitHub issue; the orchestrator
# reconciles those issues into beads (`bd create`) out-of-band.
#
# IDEMPOTENCY: each `create` template has a `dedup_key`. After expansion the key
# is embedded in the issue body as `<!-- flow-on-key: <key> -->`. Before creating,
# this script searches OPEN issues for that marker and skips if one exists. So
# re-running the workflow (or re-merging) never duplicates a follow-on.
#
# Usage:
#   flow-on.py --pr 123                      # read changed files/labels/title via gh, create issues
#   flow-on.py --pr 123 --dry-run            # print what WOULD be created; no gh writes
#   flow-on.py --pr 123 --changed-files f.txt --labels-file l.txt --title "..."   # hermetic inputs (tests)
#   flow-on.py --pr 123 --rules path.toml    # override rule file
#
# Hermetic mode: if --changed-files / --labels-file / --title are all provided,
# the script makes NO gh calls for inputs. With --dry-run it makes no gh calls at
# all, so it is fully testable without network access.

from __future__ import annotations

import argparse
import fnmatch
import importlib.util
import json
import os
import re
import subprocess
import sys
import tomllib
from dataclasses import dataclass, field
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_RULES = REPO_ROOT / "scripts" / "flow-on-rules.toml"
BENCH_REGISTRY = REPO_ROOT / "bench" / "benchmarks.toml"

# Labels every flow-on issue carries (in addition to a rule's own labels and the
# routing labels computed by routing_labels() below).
BASE_LABELS = ["flow-on", "auto"]

# Public-surface paths whose change should trigger the "changed-public-feature"
# docs rule ONLY when no SKILL.md was touched in the same PR. A change anywhere
# under skills/ counts as "the docs were synced", suppressing that rule.
SKILL_PATHS = ("skills/",)

# The reactive docs rule that the pub-API-diff gate below applies to.
DOCS_RULE_ID = "changed-public-feature-docs"
BENCH_DASHBOARD_RULE_ID = "new-bench-dashboard-row"

# [OPUS-5] (#2547) The only recognised `for_each` fan-out mode: expand a rule's
# templates once per crate whose `crates/<x>/Cargo.toml` was ADDED, binding
# `{crate}` to that crate. Without it a two-crate PR would mint follow-ons for
# whichever crate the shared `{crate}` placeholder happened to resolve to first.
FOR_EACH_NEW_CRATE = "new_crate"
FOR_EACH_MODES = (FOR_EACH_NEW_CRATE,)


# Sibling scripts are loaded by path (the scripts dir is not an importable package).
def _load_script_module(stem: str):
    spec = importlib.util.spec_from_file_location(stem, REPO_ROOT / "scripts" / f"{stem}.py")
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


# [OPUS-4.8] The `changed-public-feature-docs` rule must fire ONLY on a NET
# public-API change — never on a comment-only / lint-attribute-only / pure
# relocation diff (dogfooded: PR #250 mis-minted a skill-sync follow-on for a
# comment+lint-attr-only diff — bead sq-l0a0). We reuse the EXACT `pub `-item
# multiset-diff used by merge gate G2, factored into scripts/pub_api_diff.py, so
# the reactive engine and the proactive gate can never drift.
_pad = _load_script_module("pub_api_diff")

# [OPUS-5] (#2547) The `new-crate-followons` rule fans its templates out per
# newly-added crate and skips the user-facing ones for a non-published crate.
# Both facts are read from merge gate G1's own helpers rather than re-derived
# here, so the reactive and proactive halves can never disagree about what counts
# as "a new crate" or as "a public crate".
_gnc = _load_script_module("gate-new-crate")

# [FABLE-5] (#2474) The shared static-triage pass, also used by triage-issue.yml and
# the retriage cron — flow-on computes each follow-on's routing labels with it so
# the three triage surfaces can never drift.
_triage = _load_script_module("triage")

# [FABLE-5] (#2474) Default priority for minted follow-ons, matching the retriage
# cron's convergence default (scripts/retriage.py DEFAULT_PRIORITY).
DEFAULT_PRIORITY = "priority:P3"


def routing_labels(labels: list[str]) -> list[str]:
    """Return the role/priority/status routing labels a follow-on needs at CREATION.

    [FABLE-5] (#2474) GitHub SUPPRESSES workflow events for objects created under
    `secrets.GITHUB_TOKEN` (recursion guard), so `triage-issue.yml` (`issues:
    opened`) never runs on flow-on issues — minted without routing labels they were
    invisible to the dispatch frontier forever. So flow-on labels each issue
    dispatch-ready itself, via the SAME static pass (scripts/triage.py) the event
    and cron triagers use. Trust is by construction: the title/body/labels come from
    the checked-in rule table, not third-party input, so skipping the author
    trust-probe (which would not recognise github-actions[bot]) is sound here.
    A default `priority:P3` is applied when no rule label carries one, so the
    result converges to `status:ready` instead of parking `status:untriaged`."""
    base = set(labels)
    effective = set(base)
    if not any(_triage._PRIO.match(lb) for lb in effective):
        effective.add(DEFAULT_PRIORITY)
    verdict = _triage.triage(effective, "task")
    return sorted((effective | verdict["add"]) - base)


@dataclass
class CreateTemplate:
    dedup_key: str
    title: str
    body: str
    labels: list[str] = field(default_factory=list)
    # [OPUS-5] (#2547) Skip this template when `{crate}` names a crate whose
    # Cargo.toml carries `publish = false`. Used for follow-ons that only make
    # sense for a surface users can reach (a website page, a guide chapter) — the
    # SAME public/internal split gate G1 uses to decide whether a new crate owes
    # a SKILL.md.
    when_public_crate: bool = False


@dataclass
class Rule:
    id: str
    description: str = ""
    when_paths: list[str] = field(default_factory=list)
    when_new_paths: list[str] = field(default_factory=list)
    # [OPUS-4.8] sq-fyzq7: added paths matching any of these globs are DROPPED
    # from the `when_new_paths` matching pool. This lets a `when_new_paths` rule
    # ignore a sub-tree that is already covered elsewhere — e.g.
    # `new-zk-circuit-gatecount` uses it to skip `zk/compose/**` members (owned by
    # the hard `snapshot_covers_every_member` gate) so a new compose member does
    # not mis-fire the "new top-level circuit" follow-on. Only filters the
    # `when_new_paths` check; other predicates are unaffected.
    exclude_new_paths: list[str] = field(default_factory=list)
    when_label: str | None = None
    when_title: str | None = None
    # [OPUS-5] (#2547) Optional fan-out mode; see FOR_EACH_MODES. `None` (the
    # default) expands the templates once, against the single shared context.
    for_each: str | None = None
    creates: list[CreateTemplate] = field(default_factory=list)


@dataclass
class FollowOn:
    """A fully-expanded follow-on issue ready to be created (or printed)."""

    rule_id: str
    dedup_key: str
    title: str
    body: str
    labels: list[str]


# --------------------------------------------------------------------------- #
# Rule loading
# --------------------------------------------------------------------------- #
def load_rules(path: Path) -> list[Rule]:
    with path.open("rb") as fh:
        data = tomllib.load(fh)
    rules: list[Rule] = []
    for raw in data.get("rule", []):
        creates = [
            CreateTemplate(
                dedup_key=c["dedup_key"],
                title=c["title"],
                body=c["body"],
                labels=list(c.get("labels", [])),
                when_public_crate=bool(c.get("when_public_crate", False)),
            )
            for c in raw.get("create", [])
        ]
        rule = Rule(
            id=raw["id"],
            description=raw.get("description", ""),
            when_paths=list(raw.get("when_paths", [])),
            when_new_paths=list(raw.get("when_new_paths", [])),
            exclude_new_paths=list(raw.get("exclude_new_paths", [])),
            when_label=raw.get("when_label"),
            when_title=raw.get("when_title"),
            for_each=raw.get("for_each"),
            creates=creates,
        )
        if not (rule.when_paths or rule.when_new_paths or rule.when_label or rule.when_title):
            raise ValueError(f"rule {rule.id!r} has no trigger predicate")
        # [OPUS-5] (#2547) Fail LOUDLY on an unknown fan-out mode: silently
        # treating a typo'd `for_each` as "no fan-out" would quietly mint only
        # one crate's follow-ons, which is the exact gap this field closes.
        if rule.for_each is not None and rule.for_each not in FOR_EACH_MODES:
            raise ValueError(
                f"rule {rule.id!r} has unknown for_each={rule.for_each!r} "
                f"(known: {', '.join(FOR_EACH_MODES)})"
            )
        # [OPUS-5] (#2547) Labels are placeholder-expanded, so a `{crate}` that
        # resolves to the empty string would emit a DANGLING key — `area:` — which
        # scripts/triage.py counts as a real area and the readiness engine then
        # resolves to a partition no work belongs to. Only a `for_each` rule
        # guarantees a bound `{crate}`, so require the pairing at load time rather
        # than silently emitting a broken label at mint time.
        for tmpl in creates:
            bad = [lb for lb in tmpl.labels if "{crate}" in lb]
            if bad and rule.for_each != FOR_EACH_NEW_CRATE:
                raise ValueError(
                    f"rule {rule.id!r} label(s) {bad} use {{crate}} but the rule is "
                    f'not for_each = "{FOR_EACH_NEW_CRATE}", so {{crate}} may be empty'
                )
        if not rule.creates:
            raise ValueError(f"rule {rule.id!r} has no create templates")
        rules.append(rule)
    return rules


# --------------------------------------------------------------------------- #
# Placeholder extraction
# --------------------------------------------------------------------------- #
def _first_segment_after(prefix: str, paths: list[str]) -> str | None:
    """Return the directory name immediately under `prefix` for the first path
    that starts with it (e.g. prefix='crates/', 'crates/sparq-foo/Cargo.toml'
    -> 'sparq-foo')."""
    for p in paths:
        if p.startswith(prefix):
            rest = p[len(prefix):]
            seg = rest.split("/", 1)[0]
            if seg:
                return seg
    return None


def build_context(
    pr: int,
    pr_title: str,
    changed: list[str],
    added: list[str],
) -> dict[str, str]:
    """Compute the placeholder dict for {pr}/{pr_title}/{crate}/{suite}/{surface}/{zk_circuit}.

    Prefer ADDED paths for crate/suite (the rules that use them are new-* rules),
    falling back to all changed paths so changed-surface rules still resolve."""
    pool_for_dir = added + changed  # added first so "new crate" wins
    crate = _first_segment_after("crates/", pool_for_dir)
    suite = _first_segment_after("bench/", pool_for_dir) or _first_segment_after("zk/", pool_for_dir)
    surface = _first_segment_after("skills/", pool_for_dir)
    # [OPUS-4.8] sq-fyzq7: the NEW top-level `zk/` circuit family, for the
    # `new-zk-circuit-gatecount` rule. Derived ONLY from `zk/` (NEVER `bench/` —
    # the shared `{suite}` above prefers `bench/`, which made a #1608-shaped diff
    # mis-name the circuit `zk-compose` from the `bench/zk-compose/` output dir),
    # and with `zk/compose/` EXCLUDED: compose members are already covered by the
    # hard `snapshot_covers_every_member` gate, so a new member must not surface as
    # a phantom new "circuit". The rule's `exclude_new_paths` keeps it from firing
    # in that case, so a non-empty value here only appears for a genuine new family.
    zk_circuit = _first_segment_after(
        "zk/", [p for p in pool_for_dir if not p.startswith("zk/compose/")]
    )
    return {
        "pr": str(pr),
        "pr_title": pr_title,
        "crate": crate or "",
        "suite": suite or "",
        "surface": surface or "",
        "zk_circuit": zk_circuit or "",
    }


def crate_is_published(crate: str) -> bool:
    """True iff `crates/<crate>/Cargo.toml` does NOT declare `publish = false`.

    [OPUS-5] (#2547) The public/internal split for the `when_public_crate`
    templates. Delegates to gate G1's `crate_is_stub()` so the reactive engine
    and the merge gate read the same field the same way — G1 uses it to decide
    whether a new crate owes a SKILL.md, and we use it to decide whether it owes
    a website page and a guide chapter.

    Reads the crate manifest from the checkout. `flow-on.yml` checks out the
    MERGED commit, so a crate added by the PR is present. If the manifest cannot
    be read the crate is treated as PUBLISHED — fail-open toward minting a
    follow-on a human can close, rather than silently dropping one."""
    if not crate:
        return False
    return not _gnc.crate_is_stub(crate, [])


def rule_contexts(rule: Rule, ctx: dict[str, str], added: list[str]) -> list[dict[str, str]]:
    """The placeholder contexts a rule's templates are expanded against.

    [OPUS-5] (#2547) Default: the single shared context (one expansion, today's
    behaviour). With `for_each = "new_crate"`: one context per crate whose
    `crates/<x>/Cargo.toml` was ADDED, with `{crate}` rebound to that crate, so a
    PR landing two crates mints both crates' follow-ons. The crate list comes
    from gate G1's `added_crates()`, which matches `crates/<x>/Cargo.toml`
    exactly — so a rule glob that over-matches (fnmatch's `*` crosses `/`) still
    yields NO contexts, and therefore no follow-ons, rather than a phantom
    crate name."""
    if rule.for_each == FOR_EACH_NEW_CRATE:
        return [{**ctx, "crate": crate} for crate in _gnc.added_crates(added)]
    return [ctx]


def expand(text: str, ctx: dict[str, str]) -> str:
    # Only expand known placeholders; leave unknown braces untouched.
    def repl(m: re.Match[str]) -> str:
        key = m.group(1)
        return ctx.get(key, m.group(0))

    return re.sub(r"\{([a-z_]+)\}", repl, text)


# --------------------------------------------------------------------------- #
# Rule evaluation
# --------------------------------------------------------------------------- #
def _any_glob_match(globs: list[str], paths: list[str]) -> bool:
    for g in globs:
        for p in paths:
            if fnmatch.fnmatch(p, g):
                return True
    return False


def _bench_suite_needs_dashboard_follow_on(suite: str) -> bool:
    """Return whether a registered suite lacks an unfeatured disposition."""
    try:
        registry = BENCH_REGISTRY.read_text(encoding="utf-8")
    except OSError:
        return False
    # [GPT-5.6] Match the same registry reference fields as merge gate G3. An
    # unregistered research harness may live under bench/, but it has no metric
    # feed and therefore must not mint a dead dashboard-card follow-on.
    blocks = re.split(r"(?m)^\s*\[\[benchmark\]\]\s*$", registry)[1:]
    needle = re.compile(rf"\bbench/{re.escape(suite)}/")
    for block in blocks:
        refs = " ".join(
            re.findall(
                r"^\s*(?:source|invoke|dataset|records_to)\s*=\s*(.*)$",
                block,
                re.MULTILINE,
            )
        )
        if needle.search(refs):
            return not re.search(
                r"^\s*featured\s*=\s*false\b", block, re.MULTILINE
            )
    return False


def rule_matches(
    rule: Rule,
    changed: list[str],
    added: list[str],
    labels: list[str],
    title: str,
    pub_changed: set[str] | None = None,
) -> bool:
    # ALL present predicates must pass; absent predicates are ignored.
    if rule.when_paths and not _any_glob_match(rule.when_paths, changed):
        return False
    if rule.when_new_paths:
        # [OPUS-4.8] sq-fyzq7: drop any added path under an `exclude_new_paths`
        # sub-tree BEFORE the match, so a rule can ignore additions already owned
        # by another gate (e.g. `zk/compose/**` members → `snapshot_covers_every_member`).
        new_pool = added
        if rule.exclude_new_paths:
            new_pool = [p for p in added if not _any_glob_match(rule.exclude_new_paths, [p])]
        if not _any_glob_match(rule.when_new_paths, new_pool):
            return False
    if rule.when_label is not None and rule.when_label not in labels:
        return False
    if rule.when_title is not None and not re.search(rule.when_title, title, re.IGNORECASE):
        return False

    if rule.id == BENCH_DASHBOARD_RULE_ID:
        suite = _first_segment_after("bench/", added)
        if not suite or not _bench_suite_needs_dashboard_follow_on(suite):
            return False

    # [OPUS-4.8] Special handling for the changed-public-feature docs rule
    # (bead sq-l0a0). The when_paths glob is only a cheap pre-filter; on top of it
    # two further conditions must hold for the follow-on to be genuine:
    #   1. no SKILL.md was touched in the same diff (the docs were already synced
    #      → suppress, identical to gate G2's SKILL suppression); AND
    #   2. the PR's diff NET-changes a `pub `-exported item under one of the
    #      matched surfaces (`pub_changed`), so a comment-only / lint-attr-only /
    #      pure-relocation edit does NOT mint a redundant follow-on.
    # `pub_changed` is the set of changed paths whose unified diff has a net
    # public-API change. `None` means "diff not available" → conservatively treat
    # as no net change (do NOT fire), so a caller that cannot inspect the diff
    # never produces a false-positive follow-on.
    if rule.id == DOCS_RULE_ID:
        if any(p.startswith(SKILL_PATHS) for p in changed):
            return False
        net = pub_changed or set()
        # Fire only when at least one of THIS rule's matched surface paths has a
        # net public-API change.
        if not any(
            _any_glob_match(rule.when_paths, [p]) and p in net for p in changed
        ):
            return False
    return True


def key_marker(dedup_key: str) -> str:
    return f"<!-- flow-on-key: {dedup_key} -->"


def evaluate(
    rules: list[Rule],
    pr: int,
    pr_title: str,
    changed: list[str],
    added: list[str],
    labels: list[str],
    pub_changed: set[str] | None = None,
) -> list[FollowOn]:
    ctx = build_context(pr, pr_title, changed, added)
    out: list[FollowOn] = []
    seen_keys: set[str] = set()
    for rule in rules:
        if not rule_matches(rule, changed, added, labels, pr_title, pub_changed):
            continue
        for rctx in rule_contexts(rule, ctx, added):
            for tmpl in rule.creates:
                # [OPUS-5] (#2547) A user-facing follow-on (website / guide) is
                # not owed by an internal `publish = false` crate.
                if tmpl.when_public_crate and not crate_is_published(rctx.get("crate", "")):
                    continue
                dedup_key = expand(tmpl.dedup_key, rctx)
                if dedup_key in seen_keys:
                    continue  # de-dup within a single run
                seen_keys.add(dedup_key)
                body = expand(tmpl.body, rctx).rstrip() + "\n\n" + key_marker(dedup_key)
                body += (
                    f"\n\n> 🤖 SPARQ agent — auto-generated by the flow-on engine "
                    f"(rule `{rule.id}`) from merged PR #{pr}. Reconcile into a bead."
                )
                labels_full = list(
                    dict.fromkeys(BASE_LABELS + [expand(lb, rctx) for lb in tmpl.labels])
                )
                # [FABLE-5] (#2474) dispatch-visibility: append role/priority/status
                # routing labels at creation (see routing_labels for why the event
                # path can never label these issues).
                labels_full += routing_labels(labels_full)
                out.append(
                    FollowOn(
                        rule_id=rule.id,
                        dedup_key=dedup_key,
                        title=expand(tmpl.title, rctx),
                        body=body,
                        labels=labels_full,
                    )
                )
    return out


# --------------------------------------------------------------------------- #
# CI guard (bead sq-z0se)
# --------------------------------------------------------------------------- #
# [OPUS-4.8] An accidental dev-box run of the sibling drift scanner minted 20
# spurious GitHub issues (#397-416). This engine mints issues the same way, so it
# carries the same guard: the ISSUE-MINTING path refuses to run anywhere but CI.
# It gates on the standard CI markers (`GITHUB_ACTIONS=true`, set by GitHub
# Actions, or a truthy `CI`) and is checked ONLY on the write path —
# `--dry-run` (zero `gh` calls) stays runnable anywhere, so local previews and
# the hermetic test-suite are unaffected. An explicit `FLOW_ON_ALLOW_LOCAL=1`
# escape hatch covers a deliberate manual mint.
CI_ENV_VARS = ("GITHUB_ACTIONS", "CI")
ALLOW_LOCAL_ENV_VAR = "FLOW_ON_ALLOW_LOCAL"


def _is_truthy(val: str | None) -> bool:
    return val is not None and val.strip().lower() in {"1", "true", "yes", "on"}


def running_in_ci(env: dict[str, str] | None = None) -> bool:
    """True iff a recognised CI marker is set (GITHUB_ACTIONS or CI)."""
    e = os.environ if env is None else env
    return any(_is_truthy(e.get(v)) for v in CI_ENV_VARS)


def require_ci(env: dict[str, str] | None = None) -> None:
    """Refuse to mint issues outside CI (bead sq-z0se).

    Raises SystemExit(2) with a clear message unless a CI marker is set or the
    explicit `FLOW_ON_ALLOW_LOCAL` escape hatch is truthy. Only the write path
    calls this — `--dry-run` never does."""
    e = os.environ if env is None else env
    if running_in_ci(e) or _is_truthy(e.get(ALLOW_LOCAL_ENV_VAR)):
        return
    print(
        "flow-on: refusing to file GitHub issues outside CI "
        f"(no {' / '.join(CI_ENV_VARS)} env var set). This guard exists because "
        "an accidental dev-box run of the drift scanner minted 20 spurious issues "
        "(#397-416, bead sq-z0se). Use --dry-run to preview what would be filed, "
        f"or set {ALLOW_LOCAL_ENV_VAR}=1 only if you really intend to file issues "
        "from here.",
        file=sys.stderr,
    )
    raise SystemExit(2)


# --------------------------------------------------------------------------- #
# gh I/O
# --------------------------------------------------------------------------- #
def _gh(args: list[str]) -> str:
    proc = subprocess.run(
        ["gh", *args],
        capture_output=True,
        text=True,
        check=True,
    )
    return proc.stdout


def fetch_pr_inputs(pr: int) -> tuple[str, list[str], list[str], list[str], set[str]]:
    """Return (title, changed_files, added_files, labels, pub_changed) for a
    merged PR via gh. `pub_changed` is the set of changed paths whose unified diff
    NET-changes a `pub `-exported item (used by the docs rule)."""
    info = json.loads(_gh(["pr", "view", str(pr), "--json", "title,labels,files"]))
    title = info.get("title", "")
    labels = [lbl["name"] for lbl in info.get("labels", [])]
    files = info.get("files", [])
    changed = [f["path"] for f in files]
    # gh's files[].additions/deletions don't reveal status; parse the patch once
    # for both purely-added files (status "A") and the per-file pub-API verdict.
    patch = _gh(["pr", "diff", str(pr)])
    added = _added_files_from_patch(patch)
    pub_changed = pub_changed_from_patch(patch)
    return title, changed, added, labels, pub_changed


def _split_patch_per_file(patch: str) -> dict[str, list[str]]:
    """Split a unified `gh pr diff` patch into {dest_path: [diff lines]}.

    The destination path is taken from each `+++ b/<path>` header; lines before
    the first header are ignored. A deleted file (`+++ /dev/null`) is skipped."""
    per_file: dict[str, list[str]] = {}
    cur_path: str | None = None
    for line in patch.splitlines():
        if line.startswith("diff --git "):
            cur_path = None
            continue
        if line.startswith("+++ b/"):
            cur_path = line[len("+++ b/") :]
            per_file.setdefault(cur_path, [])
            continue
        if line.startswith("+++ "):  # +++ /dev/null (deletion) — no dest path
            cur_path = None
            continue
        if cur_path is not None:
            per_file[cur_path].append(line)
    return per_file


def _added_files_from_patch(patch: str) -> list[str]:
    # Detect "new file mode" hunks for added-file (status "A") paths.
    added: list[str] = []
    cur_new = False
    for line in patch.splitlines():
        if line.startswith("diff --git "):
            cur_new = False
        elif line.startswith("new file mode"):
            cur_new = True
        elif line.startswith("+++ b/") and cur_new:
            added.append(line[len("+++ b/") :])
            cur_new = False
    return added


def pub_changed_from_patch(patch: str) -> set[str]:
    """Return the set of changed paths whose per-file diff NET-changes a
    `pub `-exported item, using the shared gate-G2 multiset diff. Only
    `crates/*/src/**` paths can qualify (matches the gate's scope)."""
    out: set[str] = set()
    for path, lines in _split_patch_per_file(patch).items():
        if not re.match(r"^crates/[^/]+/src/.+", path):
            continue
        if _pad.diff_has_net_pub_change(lines):
            out.add(path)
    return out


def open_issue_exists(dedup_key: str) -> bool:
    marker = key_marker(dedup_key)
    # [OPUS-4.8] Search by the stable, punctuation-light substring
    # `flow-on-key: <key>` rather than the full HTML-comment marker — GitHub's
    # search tokeniser handles `<!--`/`-->` unreliably, which could miss an
    # existing issue and break idempotency (create a duplicate). The full marker
    # is still verified against each returned body below, so the dedup decision
    # stays exact; the looser query only widens the candidate set.
    search = f"flow-on-key: {dedup_key}"
    out = _gh(
        [
            "issue",
            "list",
            "--state",
            "open",
            "--label",
            "flow-on",
            "--search",
            search,
            "--json",
            "number,body",
            "--limit",
            "100",
        ]
    )
    for issue in json.loads(out):
        if marker in (issue.get("body") or ""):
            return True
    return False


# Labels we have already ensured exist this run (avoid redundant gh calls).
_ENSURED_LABELS: set[str] = set()


def ensure_label(label: str) -> None:
    """[OPUS-4.8] Idempotently ensure `label` exists before it is applied.

    `gh issue create --label X` ERRORS if label X does not yet exist, so on a
    fresh repo (where none of the per-rule labels — bench/docs/zk/… — nor
    `flow-on`/`auto` exist yet) no follow-on issue would ever be created. We
    create-if-missing with `gh label create --force` (upsert; needs only the
    `issues: write` token the workflow already grants). Failures are swallowed:
    if the label can't be created (e.g. it already exists from a concurrent run)
    the subsequent create still succeeds, and a genuinely un-creatable label
    surfaces as the create's own error rather than masking it here."""
    if label in _ENSURED_LABELS:
        return
    try:
        _gh(["label", "create", label, "--force"])
    except Exception:  # noqa: BLE001 - best-effort upsert; create_issue reports real failures
        pass
    _ENSURED_LABELS.add(label)


def create_issue(fo: FollowOn) -> str:
    args = ["issue", "create", "--title", fo.title, "--body", fo.body]
    for lbl in fo.labels:
        ensure_label(lbl)
        args += ["--label", lbl]
    return _gh(args).strip()


# --------------------------------------------------------------------------- #
# CLI
# --------------------------------------------------------------------------- #
def _read_lines(path: str) -> list[str]:
    return [ln.strip() for ln in Path(path).read_text().splitlines() if ln.strip()]


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description="Merge-triggered flow-on follow-on issue emitter.")
    ap.add_argument("--pr", type=int, required=True, help="merged PR number")
    ap.add_argument("--rules", default=str(DEFAULT_RULES), help="rule TOML path")
    ap.add_argument("--dry-run", action="store_true", help="print what WOULD be created; no gh writes")
    # Hermetic inputs (for tests / local runs without gh).
    ap.add_argument("--changed-files", help="file with one changed path per line")
    ap.add_argument("--added-files", help="file with one ADDED path per line (subset of changed)")
    ap.add_argument("--labels-file", help="file with one PR label per line")
    ap.add_argument("--title", help="PR title")
    # [OPUS-4.8] Hermetic injection of the "net public-API change" verdict used by
    # the changed-public-feature-docs rule (bead sq-l0a0): one path per line, each
    # a changed crate src file whose diff NET-changes a `pub `-item. When omitted
    # in hermetic mode the set is EMPTY, so the docs rule does NOT fire (a
    # comment-only diff mints nothing). The real gh path computes this from the
    # PR patch via the shared gate-G2 pub-item multiset diff.
    ap.add_argument(
        "--pub-diff-file",
        help="file with one changed src path per line that has a NET pub-API change",
    )
    args = ap.parse_args(argv)

    rules = load_rules(Path(args.rules))

    hermetic = args.changed_files is not None and args.title is not None
    if hermetic:
        changed = _read_lines(args.changed_files)
        # [OPUS-4.8] Default `added` to EMPTY, not `list(changed)`: a changed
        # file is not provably newly-added (mirrors the real `gh` path, which
        # derives `added` from "new file mode" diff hunks). Defaulting to all
        # changed files made `when_new_paths` rules fire on every modified path,
        # producing false follow-ons in hermetic/CI dry-runs. Pass --added-files
        # to exercise new-path rules.
        added = _read_lines(args.added_files) if args.added_files else []
        labels = _read_lines(args.labels_file) if args.labels_file else []
        title = args.title
        pub_changed = set(_read_lines(args.pub_diff_file)) if args.pub_diff_file else set()
    else:
        title, changed, added, labels, pub_changed = fetch_pr_inputs(args.pr)

    follow_ons = evaluate(rules, args.pr, title, changed, added, labels, pub_changed)

    if not follow_ons:
        print(f"flow-on: no rules triggered for PR #{args.pr}.")
        return 0

    # Issue-minting path: refuse outside CI (bead sq-z0se). Only guard the real
    # write path — a --dry-run preview makes no gh calls and runs anywhere.
    if not args.dry_run:
        require_ci()

    created = 0
    skipped = 0
    for fo in follow_ons:
        if args.dry_run:
            print(f"[dry-run] WOULD create issue (rule={fo.rule_id}, key={fo.dedup_key}):")
            print(f"          title : {fo.title}")
            print(f"          labels: {', '.join(fo.labels)}")
            continue
        if open_issue_exists(fo.dedup_key):
            print(f"flow-on: skip (open issue exists) key={fo.dedup_key} rule={fo.rule_id}")
            skipped += 1
            continue
        url = create_issue(fo)
        print(f"flow-on: created {url} (rule={fo.rule_id}, key={fo.dedup_key})")
        created += 1

    if not args.dry_run:
        print(f"flow-on: done — {created} created, {skipped} skipped (already open).")
        # Write a GitHub Actions step summary if available.
        summary = os.environ.get("GITHUB_STEP_SUMMARY")
        if summary:
            with open(summary, "a") as fh:
                fh.write(f"### flow-on (PR #{args.pr})\n\n")
                fh.write(f"- created: {created}\n- skipped (already open): {skipped}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
