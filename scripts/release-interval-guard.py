#!/usr/bin/env python3
"""Fail-closed publish-cadence guard for the sparq release path.

[OPUS-5] 🤖 SPARQ agent. Issue #1135 (maintainer, 2026-07-26): *"Before I do this; can I
make sure that there are protections in place to prevent publishing too regularly, I don't
want to spam the registry."*

WHAT THIS IS
============
The **belt** to the Release-PR arming exclusion's braces (scripts/release_pr_guard.py).
That exclusion stops the Release PR being merged automatically. This guard sits in the
release path ITSELF, so even a mistaken merge — by a human, by a future automation, by a
path nobody enumerated — cannot cut a release more often than ``MIN_RELEASE_INTERVAL``.

It runs at BOTH points where a release can begin:

* ``.github/workflows/release-plz.yml``'s ``release-plz-release`` job, BEFORE the
  ``release-plz release`` step that creates the ``v<version>`` tag and (once
  ``release-plz.toml`` sets ``publish = true``) runs ``cargo publish``;
* ``.github/workflows/release.yml``'s ``setup`` job (``--released-tag v<version>``, see
  below), which is the entry point every other job in that workflow depends on — so a
  refusal there stops the archives, the SBOM/VEX, the GitHub Release and the ghcr image.

Exiting non-zero stops the job before any of that happens.

THE TAG-PUSH PATH (``--released-tag``, issue #2552)
===================================================
``release-plz.yml`` only covers releases that go through the Release PR. The runbook's
canonical instruction is *"nothing publishes until you push a ``v*`` tag"* — and a
hand-pushed tag fires ``release.yml`` directly, which used to be entirely uncadenced. That
is the hole ``--released-tag`` closes.

It cannot be closed by running the guard unchanged there: on a tag push ``v<workspace
version>`` IS in the tag list, so check 3 below ("already tagged, nothing to do") would
short-circuit to ALLOW and the guard would be vacuous. ``--released-tag v<version>`` says
*"a release for exactly this tag is being cut right now"*: the named tag is EXCLUDED from
the last-release sources, and the already-tagged short-circuit does not apply. The cadence
question then reads correctly — how long since the release BEFORE this one.

**A crates.io version can never be unpublished.** Every ambiguity below therefore refuses.

THE THREE CHECKS
================
1. **Version-group coverage.** Every workspace crate that cargo would publish (no
   ``publish = false`` in its ``Cargo.toml``) must be listed in ``release-plz.toml``'s
   ``version_group``. Crates outside the group get INDEPENDENT versions from release-plz —
   silently breaking the locked single-version model the group exists to preserve — and
   are published anyway. A mismatch is exactly the "I do not know what would be published"
   condition, so it REFUSES.
2. **Registry dependency closure.** Every path dependency shipped by a publishable crate
   must itself be publishable and must carry a registry version requirement. Cargo cannot
   publish a package that points only at a private workspace member.
3. **Cadence.** ``now - last_release >= MIN_RELEASE_INTERVAL``, where ``last_release`` is
   the MAXIMUM of two authoritative sources — the newest ``v*`` git tag's creation date
   and the newest crates.io publication timestamp across the publishable crates. Taking
   the max means neither source can be used to argue for a shorter wait.
4. **Would a release even happen?** If ``v<workspace version>`` is already tagged,
   ``release-plz release`` is a no-op, so the cadence check is not applicable and the
   guard passes quietly. Without this the guard would red every ordinary push to main
   inside the interval window. This check is SKIPPED under ``--released-tag`` (there, the
   tag existing is the release happening, not evidence that it already happened).

FAIL-CLOSED, EXHAUSTIVELY
=========================
Every one of these REFUSES (exit 1) rather than publishing:

* the git tag list cannot be read, or the checkout is SHALLOW (a shallow clone reports an
  empty tag list, which would otherwise read as "never released" — the single most
  dangerous false negative here);
* crates.io cannot be reached, returns a non-200/non-404 status, or returns a body that
  does not parse (a 404 IS definitive: that crate has never been published);
* a tag or publish timestamp cannot be parsed;
* ``--released-tag`` is given a value that is not a ``vX.Y.Z`` release tag (the guard
  cannot tell which tag to exclude, so it would silently measure the interval against the
  release it is being asked to permit);
* the last release timestamp is in the FUTURE (clock skew / a bad tag date);
* the workspace manifest cannot be read, or the publishable-crate set cannot be derived;
* a publishable crate has an unpublished or unversioned workspace dependency;
* a publishable crate is missing from the version_group.

An unknown NEVER means "go ahead". There is deliberately **no override flag** — a
maintainer who genuinely needs to publish inside the window does it by hand, consciously.

MODES
=====
``--enforce``  — the CI mode. Exit 0 to permit, exit 1 to refuse.
``--dry-run``  — report ONLY. Prints the crate list, each crate's version, and the
                 dependency-ordered publish sequence, plus the cadence verdict it WOULD
                 return. Performs no git, cargo, or network mutation and always exits 0
                 (it is an inspection tool; a non-zero exit would make it unusable in a
                 pipeline). ``--dry-run`` never runs `cargo publish`, `git tag`, or
                 `git push` — see `test_release_publish_guard.py::TestDryRunIsInert`.
``--self-test``— hermetic logic self-test (no network, no git, no cargo).

stdlib-only.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import re
import subprocess
import sys
import time
import urllib.error
import urllib.request
from dataclasses import dataclass, field
from pathlib import Path

try:
    import tomllib  # stdlib >= 3.11
except ModuleNotFoundError:  # pragma: no cover - the runner ships 3.11+
    tomllib = None  # type: ignore[assignment]


# ---------------------------------------------------------------------------- constants

# THE ONE KNOB. 24 hours.
#
# WHY 24h, and not less:
#   * One sparq release publishes EVERY crate in the version_group (37 today) as a new
#     crates.io version, in lockstep. Two releases in a day is 74 irreversible versions.
#   * sparq lands on the order of 80 commits a day. Cadence tracks Release-PR MERGES, not
#     commits — but nothing except this guard bounds how often that PR can be merged, and
#     the whole point of #1135 is that the maintainer does not want the registry spammed.
#   * A day is the shortest interval that still leaves room to NOTICE a bad release before
#     the next one goes out. A version published 20 minutes after its predecessor is
#     almost certainly a mistake being repeated, not a deliberate second release.
#   * It is not restrictive in practice: the project has never cut two releases in a day,
#     and a genuine emergency patch is still available — by hand, deliberately, which is
#     precisely the friction an irreversible operation should have.
#
# WHY NOT LONGER: a week would block a legitimate same-week fix release and push people
# toward disabling the guard, which is worse than a guard with a defensible floor.
MIN_RELEASE_INTERVAL = dt.timedelta(hours=24)
MIN_RELEASE_INTERVAL_HOURS = MIN_RELEASE_INTERVAL.total_seconds() / 3600.0

CRATES_IO_API = "https://crates.io/api/v1/crates/{name}"
# crates.io requires a descriptive User-Agent and rejects generic ones.
CRATES_IO_USER_AGENT = (
    "sparq-release-interval-guard (https://github.com/sparq-org/sparq; issue #1135)"
)
CRATES_IO_TIMEOUT = 20
# [GPT-5.6] Thirty-seven registry reads make a one-off CDN/TLS reset likely enough to
# wedge a release. Retry only transient transport/status failures; remain fail-closed.
CRATES_IO_RETRY_DELAYS = (0.5, 1.5)
CRATES_IO_RETRYABLE_ERROR_RE = re.compile(
    r"^(?:request failed:|HTTP (?:408|425|429|5\d\d)$)"
)

VERSION_TAG_RE = re.compile(r"^v(\d+\.\d+\.\d+(?:[-+].*)?)$")

PROGRAM = "release-interval-guard"


class GuardRefusal(Exception):
    """A fail-closed refusal. Carrying it as an exception makes 'forgot to check the
    return value' impossible — there is no falsy success value to ignore."""


@dataclass(frozen=True)
class Crate:
    name: str
    version: str
    path: Path
    # Intra-workspace dependencies on OTHER publishable crates (publish order).
    deps: frozenset[str] = frozenset()


@dataclass
class Verdict:
    allowed: bool
    reason: str
    # Populated for reporting; None where a source definitively reports "never published".
    last_tag: str | None = None
    last_release_at: dt.datetime | None = None
    source: str | None = None
    notes: list[str] = field(default_factory=list)


# ------------------------------------------------------------------- workspace manifests


def _load_toml(path: Path) -> dict:
    if tomllib is None:  # pragma: no cover
        raise GuardRefusal(
            "python3 has no tomllib (needs 3.11+); cannot read the workspace manifests, "
            "so the publishable-crate set is unknown — refusing to publish"
        )
    try:
        return tomllib.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        raise GuardRefusal(f"cannot read {path}: {error}") from error


def _dep_tables(manifest: dict):
    """Yield every dependency table in a member manifest, including target-specific ones."""
    for key in ("dependencies", "build-dependencies"):
        table = manifest.get(key)
        if isinstance(table, dict):
            yield table
    targets = manifest.get("target")
    if isinstance(targets, dict):
        for cfg in targets.values():
            if isinstance(cfg, dict):
                for key in ("dependencies", "build-dependencies"):
                    table = cfg.get(key)
                    if isinstance(table, dict):
                        yield table


def publishable_crates(repo_root: Path) -> list[Crate]:
    """Every workspace member cargo would publish, with its version and workspace deps.

    Derived from the manifests directly (tomllib) rather than `cargo metadata`: no
    network, no toolchain, no lockfile resolution — so this cannot fail for reasons
    unrelated to the question, and the test suite can drive it on a fixture tree.
    """
    root = _load_toml(repo_root / "Cargo.toml")
    workspace = root.get("workspace")
    if not isinstance(workspace, dict):
        raise GuardRefusal("Cargo.toml has no [workspace] table — refusing to publish")
    members = workspace.get("members")
    if not isinstance(members, list) or not members:
        raise GuardRefusal(
            "[workspace].members is missing or empty — the publishable-crate set is "
            "unknown, refusing to publish"
        )
    workspace_version = ((workspace.get("package") or {}) or {}).get("version")

    # [GPT-5.6] Keep every member until dependency closure is validated. The old code
    # discarded `publish = false` members before walking dependencies, which made a
    # public -> private path edge invisible even though `cargo publish` cannot resolve it.
    all_members: dict[str, tuple[str, Path, dict, bool]] = {}
    for member in members:
        member_dir = repo_root / str(member)
        manifest = _load_toml(member_dir / "Cargo.toml")
        package = manifest.get("package")
        if not isinstance(package, dict):
            raise GuardRefusal(f"{member}/Cargo.toml has no [package] table")
        name = package.get("name")
        if not isinstance(name, str) or not name:
            raise GuardRefusal(f"{member}/Cargo.toml has no package.name")
        publish = package.get("publish")
        # cargo: `publish = false` or `publish = []` means never publish. Anything else
        # (absent, true, a registry list) means cargo WOULD publish it.
        is_publishable = publish is not False and publish != []
        version = package.get("version")
        if isinstance(version, dict):  # version.workspace = true
            if not version.get("workspace"):
                raise GuardRefusal(f"{name}: unreadable package.version table")
            version = workspace_version
        if not isinstance(version, str) or not version:
            raise GuardRefusal(
                f"{name}: no resolvable version (member nor [workspace.package]) — "
                "refusing to publish a crate whose version is unknown"
            )
        if name in all_members:
            raise GuardRefusal(f"duplicate workspace package name {name!r}")
        all_members[name] = (version, member_dir, manifest, is_publishable)

    raw = {
        name: (version, member_dir, manifest)
        for name, (version, member_dir, manifest, is_publishable) in all_members.items()
        if is_publishable
    }

    if not raw:
        raise GuardRefusal(
            "no publishable workspace crate found — refusing rather than assuming"
        )

    crates: list[Crate] = []
    for name, (version, member_dir, manifest) in raw.items():
        deps: set[str] = set()
        for table in _dep_tables(manifest):
            for dep_name, spec in table.items():
                # `package = "x"` renames; the real crate is the `package` value.
                real = dep_name
                if isinstance(spec, dict) and isinstance(spec.get("package"), str):
                    real = spec["package"]
                if not isinstance(spec, dict) or "path" not in spec:
                    continue
                if real not in all_members or real == name:
                    continue
                if not all_members[real][3]:
                    raise GuardRefusal(
                        f"{name}: publishable crate depends on unpublished workspace "
                        f"crate {real!r}; publish the dependency or remove the registry "
                        "package from the release set"
                    )
                requirement = spec.get("version")
                if not isinstance(requirement, str) or not requirement.strip():
                    raise GuardRefusal(
                        f"{name}: workspace dependency {real!r} has a path but no "
                        "registry version requirement; add `version = \"<workspace "
                        "version>\"` before publishing"
                    )
                deps.add(real)
        crates.append(
            Crate(name=name, version=version, path=member_dir, deps=frozenset(deps))
        )
    return sorted(crates, key=lambda c: c.name)


def crates_io_publish_enabled(repo_root: Path) -> bool:
    """Is `release-plz.toml` configured to `cargo publish` to crates.io?

    FAIL-CLOSED: anything other than an explicit `publish = false` under `[workspace]` —
    a missing key, a missing table, a non-boolean value — is treated as ENABLED, i.e. the
    strict reading. Being wrong in the strict direction costs a red job; being wrong in
    the permissive direction costs an irreversible publish.
    """
    config = _load_toml(repo_root / "release-plz.toml")
    workspace = config.get("workspace")
    if not isinstance(workspace, dict):
        return True
    return workspace.get("publish") is not False


def version_group_members(repo_root: Path) -> set[str]:
    """The crate names release-plz.toml puts in a `version_group`."""
    config = _load_toml(repo_root / "release-plz.toml")
    packages = config.get("package")
    if not isinstance(packages, list):
        raise GuardRefusal(
            "release-plz.toml declares no [[package]] entries — the version_group is "
            "unknown, refusing to publish"
        )
    grouped: set[str] = set()
    for entry in packages:
        if not isinstance(entry, dict):
            continue
        name = entry.get("name")
        if isinstance(name, str) and entry.get("version_group"):
            grouped.add(name)
    return grouped


def publish_order(crates: list[Crate]) -> list[Crate]:
    """Dependency-first order: a crate is listed after every workspace crate it needs.

    That is the order `cargo publish` must follow, because a crate cannot be published
    until its path+version dependencies already exist on the registry. Ties break by name
    so the output is deterministic and diffable.
    """
    by_name = {crate.name: crate for crate in crates}
    ordered: list[Crate] = []
    placed: set[str] = set()
    remaining = sorted(by_name, key=str)
    while remaining:
        ready = [
            name
            for name in remaining
            if all(dep in placed for dep in by_name[name].deps if dep in by_name)
        ]
        if not ready:
            # A dependency cycle among publishable crates cannot be published at all.
            raise GuardRefusal(
                "dependency cycle among publishable crates "
                f"({', '.join(sorted(remaining))}) — no valid publish order exists, "
                "refusing to publish"
            )
        for name in ready:
            ordered.append(by_name[name])
            placed.add(name)
            remaining.remove(name)
    return ordered


# ------------------------------------------------------------------------ release times


def _run_git(repo_root: Path, args: list[str]) -> str:
    try:
        proc = subprocess.run(
            ["git", "-C", str(repo_root), *args],
            capture_output=True,
            text=True,
            timeout=60,
        )
    except (OSError, subprocess.SubprocessError) as error:
        raise GuardRefusal(f"git {' '.join(args)} could not run: {error}") from error
    if proc.returncode != 0:
        detail = (proc.stderr or proc.stdout or "").strip() or "unknown git failure"
        raise GuardRefusal(f"git {' '.join(args)} failed: {detail}")
    return proc.stdout


def git_release_tags(repo_root: Path, run_git=_run_git) -> list[tuple[str, dt.datetime]]:
    """Every `v<semver>` tag with its creation date, newest first.

    REFUSES on a SHALLOW checkout. This is the load-bearing fail-closed case: a shallow
    clone happily reports an EMPTY tag list, which the cadence check would otherwise read
    as "nothing has ever been released — publish away".
    """
    shallow = run_git(repo_root, ["rev-parse", "--is-shallow-repository"]).strip()
    if shallow != "false":
        raise GuardRefusal(
            f"the checkout is shallow (git rev-parse --is-shallow-repository = "
            f"{shallow!r}), so the `v*` tag list is NOT authoritative and an empty list "
            "would be indistinguishable from 'never released' — refusing to publish. "
            "Check out with fetch-depth: 0."
        )
    raw = run_git(
        repo_root,
        [
            "for-each-ref",
            "--format=%(refname:short)%09%(creatordate:iso-strict)",
            "refs/tags/v*",
        ],
    )
    tags: list[tuple[str, dt.datetime]] = []
    for line in raw.splitlines():
        line = line.strip()
        if not line:
            continue
        name, _, when = line.partition("\t")
        if not VERSION_TAG_RE.match(name.strip()):
            continue
        parsed = parse_timestamp(when.strip())
        if parsed is None:
            raise GuardRefusal(
                f"tag {name!r} has an unparseable creation date {when!r} — the last "
                "release time cannot be determined, refusing to publish"
            )
        tags.append((name.strip(), parsed))
    return sorted(tags, key=lambda item: item[1], reverse=True)


def parse_timestamp(text: str) -> dt.datetime | None:
    """Parse an ISO-8601 timestamp into an aware UTC datetime, or None."""
    if not text:
        return None
    candidate = text.strip().replace("Z", "+00:00")
    try:
        parsed = dt.datetime.fromisoformat(candidate)
    except ValueError:
        return None
    if parsed.tzinfo is None:
        return None  # a naive timestamp is ambiguous — treat as unparseable
    return parsed.astimezone(dt.timezone.utc)


def _http_get_json(url: str) -> tuple[dict | None, str | None]:
    """(payload, error). payload is None with error None ONLY for a definitive 404."""
    request = urllib.request.Request(
        url, headers={"User-Agent": CRATES_IO_USER_AGENT, "Accept": "application/json"}
    )
    try:
        with urllib.request.urlopen(request, timeout=CRATES_IO_TIMEOUT) as response:
            if response.status != 200:
                return None, f"HTTP {response.status}"
            body = response.read().decode("utf-8")
    except urllib.error.HTTPError as error:
        if error.code == 404:
            # DEFINITIVE: crates.io says this crate does not exist. Not an unknown.
            return None, None
        return None, f"HTTP {error.code}"
    except (urllib.error.URLError, OSError, ValueError) as error:
        return None, f"request failed: {error}"
    try:
        payload = json.loads(body)
    except json.JSONDecodeError as error:
        return None, f"unparseable JSON body: {error}"
    if not isinstance(payload, dict):
        return None, "response body is not a JSON object"
    return payload, None


def crates_io_last_publish(
    names: list[str], fetch=_http_get_json, retry_sleep=time.sleep
) -> dt.datetime | None:
    """The newest crates.io publication timestamp across `names`, or None if NONE of them
    has ever been published. Transient lookup failures receive two bounded retries; the
    final failure (and every non-transient error) REFUSES."""
    newest: dt.datetime | None = None
    for name in sorted(names):
        attempts = 0
        while True:
            attempts += 1
            payload, error = fetch(CRATES_IO_API.format(name=name))
            if error is None:
                break
            retryable = CRATES_IO_RETRYABLE_ERROR_RE.match(error) is not None
            if not retryable or attempts > len(CRATES_IO_RETRY_DELAYS):
                suffix = f" after {attempts} attempt(s)" if retryable else ""
                raise GuardRefusal(
                    f"crates.io lookup for {name!r} was indeterminate{suffix} ({error}) — "
                    "the last publication time cannot be established, refusing to publish"
                )
            retry_sleep(CRATES_IO_RETRY_DELAYS[attempts - 1])
        if payload is None:
            continue  # definitive 404: never published
        versions = payload.get("versions")
        if not isinstance(versions, list):
            raise GuardRefusal(
                f"crates.io response for {name!r} has no `versions` array — refusing"
            )
        for version in versions:
            if not isinstance(version, dict):
                continue
            stamp = parse_timestamp(str(version.get("created_at") or ""))
            if stamp is None:
                raise GuardRefusal(
                    f"crates.io returned an unparseable created_at for {name!r} "
                    f"({version.get('created_at')!r}) — refusing"
                )
            if newest is None or stamp > newest:
                newest = stamp
    return newest


# ----------------------------------------------------------------------------- decision


def decide(
    *,
    now: dt.datetime,
    workspace_version: str,
    tags: list[tuple[str, dt.datetime]],
    crates_io_at: dt.datetime | None,
    interval: dt.timedelta = MIN_RELEASE_INTERVAL,
    released_tag: str | None = None,
) -> Verdict:
    """Pure cadence decision. Every caller-visible refusal path is exercised by the tests.

    Callers MUST have already resolved `tags` and `crates_io_at` through the fail-closed
    readers above — reaching here means both sources answered DEFINITIVELY.

    `released_tag` is the tag-push path (issue #2552): the named tag is the release being
    cut RIGHT NOW, so it is excluded from the last-release sources and the already-tagged
    short-circuit below does not apply. Without both of those, the guard on that path
    would measure the interval against the very release it is deciding, and always allow.
    """
    if released_tag is not None:
        if not VERSION_TAG_RE.match(released_tag):
            raise GuardRefusal(
                f"--released-tag {released_tag!r} is not a `vX.Y.Z` release tag, so the "
                "tag being cut cannot be excluded from the last-release sources — the "
                "cadence would be measured against this very release and always pass. "
                "Refusing to publish."
            )
        tags = [(name, when) for name, when in tags if name != released_tag]
    elif f"v{workspace_version}" in {name for name, _ in tags}:
        return Verdict(
            True,
            f"v{workspace_version} is already tagged — `release-plz release` is a no-op "
            "on this push, so the cadence guard is not applicable",
            last_tag=f"v{workspace_version}",
        )

    last_tag_name = tags[0][0] if tags else None
    last_tag_at = tags[0][1] if tags else None

    candidates = [
        (stamp, label)
        for stamp, label in ((last_tag_at, "git tag"), (crates_io_at, "crates.io"))
        if stamp is not None
    ]
    if not candidates:
        return Verdict(
            True,
            "no `v*` tag exists and no publishable crate has ever appeared on crates.io "
            "(both sources answered definitively) — this is the FIRST release, so no "
            "cadence can have been violated",
            source="first-release",
        )

    last_at, source = max(candidates, key=lambda item: item[0])

    if last_at > now:
        return Verdict(
            False,
            f"the last release timestamp ({last_at.isoformat()}, from {source}) is in "
            f"the FUTURE relative to now ({now.isoformat()}) — the interval cannot be "
            "computed, refusing to publish",
            last_tag=last_tag_name,
            last_release_at=last_at,
            source=source,
        )

    elapsed = now - last_at
    if elapsed < interval:
        remaining = interval - elapsed
        return Verdict(
            False,
            f"the last release was {_humanize(elapsed)} ago ({last_at.isoformat()}, "
            f"from {source}); the minimum interval is {_humanize(interval)}. "
            f"REFUSING to publish for another {_humanize(remaining)}. A crates.io "
            "version cannot be unpublished, so releasing this soon is treated as a "
            "mistake. If it is deliberate, a maintainer publishes by hand.",
            last_tag=last_tag_name,
            last_release_at=last_at,
            source=source,
        )
    return Verdict(
        True,
        f"the last release was {_humanize(elapsed)} ago ({last_at.isoformat()}, from "
        f"{source}), at or beyond the {_humanize(interval)} minimum interval",
        last_tag=last_tag_name,
        last_release_at=last_at,
        source=source,
    )


def _humanize(delta: dt.timedelta) -> str:
    total = int(delta.total_seconds())
    if total < 60:
        return f"{total}s"
    if total < 3600:
        return f"{total // 60}m"
    hours, minutes = divmod(total // 60, 60)
    return f"{hours}h{minutes:02d}m"


def check_version_group(crates: list[Crate], grouped: set[str]) -> list[str]:
    """Publishable crates NOT covered by release-plz.toml's version_group."""
    return sorted(crate.name for crate in crates if crate.name not in grouped)


# --------------------------------------------------------------------------------- main


def _report(crates: list[Crate], ordered: list[Crate], log) -> None:
    log(f"[{PROGRAM}] {len(crates)} publishable crate(s); publish order (deps first):")
    width = max((len(c.name) for c in ordered), default=0)
    for index, crate in enumerate(ordered, start=1):
        log(f"  {index:>3}. {crate.name.ljust(width)}  {crate.version}")


def run(
    repo_root: Path,
    *,
    dry_run: bool,
    now: dt.datetime | None = None,
    interval: dt.timedelta = MIN_RELEASE_INTERVAL,
    released_tag: str | None = None,
    git_runner=_run_git,
    fetch=_http_get_json,
    log=print,
) -> int:
    now = now or dt.datetime.now(dt.timezone.utc)
    mode = "DRY-RUN (reporting only, nothing is mutated)" if dry_run else "ENFORCE"
    log(f"[{PROGRAM}] mode: {mode}")
    if released_tag is not None:
        log(
            f"[{PROGRAM}] tag-push path: {released_tag} is the release being cut now — "
            "excluded from the last-release sources (issue #2552)"
        )
    try:
        crates = publishable_crates(repo_root)
        ordered = publish_order(crates)
        _report(crates, ordered, log)

        publish_enabled = crates_io_publish_enabled(repo_root)
        log(
            f"[{PROGRAM}] release-plz.toml crates.io publish: "
            f"{'ENABLED' if publish_enabled else 'disabled (publish = false)'}"
        )

        grouped = version_group_members(repo_root)
        ungrouped = check_version_group(crates, grouped)
        if ungrouped:
            message = (
                f"{len(ungrouped)} publishable crate(s) are NOT in release-plz.toml's "
                f"version_group: {', '.join(ungrouped)}. release-plz would version them "
                "INDEPENDENTLY of the locked workspace version and publish them anyway, "
                "so what would be published is not what the config describes. "
                "FIX: add a [[package]] entry with version_group = \"sparq\" for each, or "
                "set publish = false in the crate's Cargo.toml if it should not ship."
            )
            if publish_enabled:
                # crates.io is reachable from this release: an un-described crate could be
                # published irreversibly. REFUSE.
                raise GuardRefusal(message + " REFUSING while publish is enabled.")
            # `publish = false`: nothing can reach crates.io on this release, so the
            # mismatch cannot cause a bad publish today — it is a BLOCKER FOR THE FLIP,
            # not for cutting a tag. Warn loudly; the same condition becomes a hard
            # refusal the moment `publish = true` lands, which is the point.
            log(
                f"::warning title={PROGRAM} version_group drift (blocks `publish = "
                f"true`)::{message} This is a WARNING only because release-plz.toml "
                "still has publish = false; it becomes a hard refusal on the flip."
            )
        else:
            log(
                f"[{PROGRAM}] version_group covers all {len(crates)} publishable "
                "crate(s)."
            )

        root = _load_toml(repo_root / "Cargo.toml")
        workspace_version = (
            ((root.get("workspace") or {}).get("package") or {}).get("version")
        )
        if not isinstance(workspace_version, str) or not workspace_version:
            raise GuardRefusal(
                "[workspace.package].version is missing — cannot tell which version "
                "would be released, refusing to publish"
            )

        tags = git_release_tags(repo_root, run_git=git_runner)
        # Short-circuit the no-op push BEFORE hitting crates.io: `release-plz release`
        # does nothing when the current version is already tagged, and on this repo that
        # is every ordinary push to main. Skipping the ~37 registry requests there keeps
        # the guard cheap; the decision is identical (decide() returns the same verdict,
        # which the self-test pins).
        # On the tag-push path that short-circuit does not apply (the tag exists BECAUSE
        # the release is happening), so crates.io is always consulted there.
        crates_io_at: dt.datetime | None = None
        if released_tag is not None or not any(
            name == f"v{workspace_version}" for name, _ in tags
        ):
            crates_io_at = crates_io_last_publish([c.name for c in crates], fetch=fetch)
        verdict = decide(
            now=now,
            workspace_version=workspace_version,
            tags=tags,
            crates_io_at=crates_io_at,
            interval=interval,
            released_tag=released_tag,
        )
    except GuardRefusal as refusal:
        log(f"::error title={PROGRAM} REFUSED to publish::{refusal}")
        if dry_run:
            log(
                f"[{PROGRAM}] dry-run: the above would REFUSE the release. Exiting 0 "
                "because --dry-run reports and never decides."
            )
            return 0
        return 1

    if verdict.allowed:
        log(f"[{PROGRAM}] ALLOW — {verdict.reason}")
        return 0
    log(f"::error title={PROGRAM} REFUSED to publish::{verdict.reason}")
    if dry_run:
        log(
            f"[{PROGRAM}] dry-run: the above would REFUSE the release. Exiting 0 "
            "because --dry-run reports and never decides."
        )
        return 0
    return 1


def self_test() -> int:
    failures: list[str] = []

    def check(label: str, condition: bool) -> None:
        if not condition:
            failures.append(label)
        print(f"  [{'PASS' if condition else 'FAIL'}] {label}")

    now = dt.datetime(2026, 7, 26, 12, 0, tzinfo=dt.timezone.utc)
    day = dt.timedelta(hours=24)

    inside = decide(
        now=now,
        workspace_version="0.2.0",
        tags=[("v0.1.0", now - dt.timedelta(hours=3))],
        crates_io_at=None,
    )
    check("a release 3h after the last one is REFUSED", not inside.allowed)

    outside = decide(
        now=now,
        workspace_version="0.2.0",
        tags=[("v0.1.0", now - dt.timedelta(hours=25))],
        crates_io_at=None,
    )
    check("a release 25h after the last one is ALLOWED", outside.allowed)

    boundary = decide(
        now=now,
        workspace_version="0.2.0",
        tags=[("v0.1.0", now - day)],
        crates_io_at=None,
    )
    check("exactly at the interval is ALLOWED (>=, not >)", boundary.allowed)

    check(
        "crates.io wins when it is NEWER than the newest tag (max, not first source)",
        not decide(
            now=now,
            workspace_version="0.2.0",
            tags=[("v0.1.0", now - dt.timedelta(days=30))],
            crates_io_at=now - dt.timedelta(hours=2),
        ).allowed,
    )
    check(
        "the tag wins when IT is newer than crates.io",
        not decide(
            now=now,
            workspace_version="0.2.0",
            tags=[("v0.1.0", now - dt.timedelta(hours=2))],
            crates_io_at=now - dt.timedelta(days=30),
        ).allowed,
    )
    check(
        "a FUTURE last-release timestamp REFUSES (clock skew is not a green light)",
        not decide(
            now=now,
            workspace_version="0.2.0",
            tags=[("v0.1.0", now + dt.timedelta(hours=1))],
            crates_io_at=None,
        ).allowed,
    )
    check(
        "a genuine FIRST release (both sources definitively empty) is ALLOWED",
        decide(
            now=now, workspace_version="0.1.0", tags=[], crates_io_at=None
        ).allowed,
    )
    already = decide(
        now=now,
        workspace_version="0.1.0",
        tags=[("v0.1.0", now - dt.timedelta(hours=1))],
        crates_io_at=None,
    )
    check(
        "an ALREADY-TAGGED version is a no-op push, not a cadence violation",
        already.allowed and "already tagged" in already.reason,
    )

    # ---- the tag-push path (issue #2552). The SAME inputs that read as a harmless no-op
    # above must read as a cadence VIOLATION once the tag is the release being cut, or the
    # guard is vacuous on release.yml. These two cases are the discriminating pair.
    check(
        "the tag being cut does NOT excuse itself (v0.1.0 pushed 1h after v0.0.9)",
        not decide(
            now=now,
            workspace_version="0.1.0",
            tags=[
                ("v0.1.0", now),
                ("v0.0.9", now - dt.timedelta(hours=1)),
            ],
            crates_io_at=None,
            released_tag="v0.1.0",
        ).allowed,
    )
    check(
        "the tag being cut is EXCLUDED from the sources (v0.1.0 pushed 25h after v0.0.9)",
        decide(
            now=now,
            workspace_version="0.1.0",
            tags=[
                ("v0.1.0", now),
                ("v0.0.9", now - dt.timedelta(hours=25)),
            ],
            crates_io_at=None,
            released_tag="v0.1.0",
        ).allowed,
    )
    check(
        "the FIRST tag push (nothing else tagged, never published) is ALLOWED",
        decide(
            now=now,
            workspace_version="0.1.0",
            tags=[("v0.1.0", now)],
            crates_io_at=None,
            released_tag="v0.1.0",
        ).allowed,
    )
    check(
        "crates.io still bounds the tag-push path (tag list empty, published 2h ago)",
        not decide(
            now=now,
            workspace_version="0.1.0",
            tags=[("v0.1.0", now)],
            crates_io_at=now - dt.timedelta(hours=2),
            released_tag="v0.1.0",
        ).allowed,
    )
    try:
        decide(
            now=now,
            workspace_version="0.1.0",
            tags=[],
            crates_io_at=None,
            released_tag="not-a-tag",
        )
    except GuardRefusal as error:
        check(
            "a non-release --released-tag REFUSES (it cannot be excluded)",
            "not a `vX.Y.Z` release tag" in str(error),
        )
    else:
        check("a non-release --released-tag REFUSES", False)

    # ---- fail-closed readers.
    def shallow_git(_root, args):
        if args[:2] == ["rev-parse", "--is-shallow-repository"]:
            return "true\n"
        return ""

    try:
        git_release_tags(Path("."), run_git=shallow_git)
    except GuardRefusal as error:
        check("a SHALLOW checkout REFUSES (empty tags must not read as 'never released')",
              "shallow" in str(error))
    else:
        check("a SHALLOW checkout REFUSES", False)

    def failing_git(_root, args):
        raise GuardRefusal("git for-each-ref failed: fatal: not a git repository")

    try:
        git_release_tags(Path("."), run_git=failing_git)
    except GuardRefusal:
        check("an unreadable tag list REFUSES", True)
    else:
        check("an unreadable tag list REFUSES", False)

    def bad_date_git(_root, args):
        if args[:2] == ["rev-parse", "--is-shallow-repository"]:
            return "false\n"
        return "v0.1.0\tnot-a-date\n"

    try:
        git_release_tags(Path("."), run_git=bad_date_git)
    except GuardRefusal as error:
        check("an unparseable tag date REFUSES", "unparseable" in str(error))
    else:
        check("an unparseable tag date REFUSES", False)

    def ok_git(_root, args):
        if args[:2] == ["rev-parse", "--is-shallow-repository"]:
            return "false\n"
        return "v0.1.0\t2026-07-01T00:00:00+00:00\nnot-a-version-tag\t2026-07-02T00:00:00+00:00\n"

    tags = git_release_tags(Path("."), run_git=ok_git)
    check(
        "a good tag list parses, and non-version tags are ignored",
        [name for name, _ in tags] == ["v0.1.0"],
    )

    # ---- crates.io reader.
    try:
        crates_io_last_publish(
            ["sparq-core"],
            fetch=lambda _u: (None, "HTTP 503"),
            retry_sleep=lambda _seconds: None,
        )
    except GuardRefusal as error:
        check("an UNREACHABLE crates.io REFUSES", "indeterminate" in str(error))
    else:
        check("an UNREACHABLE crates.io REFUSES", False)

    check(
        "a definitive 404 means 'never published', NOT an error",
        crates_io_last_publish(["sparq-core"], fetch=lambda _u: (None, None)) is None,
    )
    check(
        "the newest created_at across crates is returned",
        crates_io_last_publish(
            ["a", "b"],
            fetch=lambda url: (
                {
                    "versions": [
                        {"created_at": "2026-07-01T00:00:00Z"},
                        {"created_at": "2026-07-20T00:00:00Z"},
                    ]
                },
                None,
            ),
        )
        == dt.datetime(2026, 7, 20, tzinfo=dt.timezone.utc),
    )
    try:
        crates_io_last_publish(
            ["a"], fetch=lambda _u: ({"versions": [{"created_at": "nope"}]}, None)
        )
    except GuardRefusal:
        check("an unparseable crates.io created_at REFUSES", True)
    else:
        check("an unparseable crates.io created_at REFUSES", False)

    # ---- publish order.
    core = Crate("sparq-core", "0.1.0", Path("."), frozenset())
    engine = Crate("sparq-engine", "0.1.0", Path("."), frozenset({"sparq-core"}))
    cli = Crate("sparq-cli", "0.1.0", Path("."), frozenset({"sparq-engine"}))
    order = [c.name for c in publish_order([cli, engine, core])]
    check(
        "publish order is dependency-first",
        order == ["sparq-core", "sparq-engine", "sparq-cli"],
    )
    try:
        publish_order(
            [
                Crate("a", "0.1.0", Path("."), frozenset({"b"})),
                Crate("b", "0.1.0", Path("."), frozenset({"a"})),
            ]
        )
    except GuardRefusal:
        check("a dependency CYCLE REFUSES (no valid publish order)", True)
    else:
        check("a dependency CYCLE REFUSES", False)

    # ---- version-group coverage.
    check(
        "a publishable crate outside the version_group is reported",
        check_version_group([core, engine], {"sparq-core"}) == ["sparq-engine"],
    )
    check(
        "full coverage reports nothing",
        check_version_group([core, engine], {"sparq-core", "sparq-engine"}) == [],
    )

    # ---- the publish flag is read FAIL-CLOSED: only an explicit `false` disables.
    import tempfile as _tempfile

    with _tempfile.TemporaryDirectory(prefix="sparq-publish-flag-") as tmp:
        root = Path(tmp)
        for label, body, expected in (
            ("explicit publish = false disables", '[workspace]\npublish = false\n', False),
            ("explicit publish = true enables", '[workspace]\npublish = true\n', True),
            ("a MISSING publish key reads as ENABLED", "[workspace]\n", True),
            ("a MISSING [workspace] table reads as ENABLED", "", True),
            ("a non-boolean publish reads as ENABLED", '[workspace]\npublish = "no"\n', True),
        ):
            (root / "release-plz.toml").write_text(body, encoding="utf-8")
            check(label, crates_io_publish_enabled(root) is expected)

    # ---- the constant itself. If someone widens it to a token value, say so loudly.
    check(
        "MIN_RELEASE_INTERVAL is at least 24h",
        MIN_RELEASE_INTERVAL >= dt.timedelta(hours=24),
    )

    if failures:
        print(f"\n{PROGRAM} self-test: {len(failures)} case(s) FAILED")
        for label in failures:
            print(f"  - {label}")
        return 1
    print(f"\n{PROGRAM} self-test: PASS")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Fail-closed publish-cadence guard (issue #1135)."
    )
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument(
        "--enforce",
        action="store_true",
        help="CI mode: exit 1 (refuse) unless a release is provably permitted.",
    )
    mode.add_argument(
        "--dry-run",
        action="store_true",
        help="Report the crate list, versions, publish order and cadence verdict. "
        "Mutates nothing and always exits 0.",
    )
    mode.add_argument(
        "--self-test", action="store_true", help="hermetic logic self-test"
    )
    parser.add_argument(
        "--repo-root",
        default=".",
        help="workspace root containing Cargo.toml + release-plz.toml (default: .)",
    )
    parser.add_argument(
        "--released-tag",
        default=None,
        metavar="vX.Y.Z",
        help="the tag-push path (issue #2552): the `v*` tag whose release is being cut "
        "RIGHT NOW. It is excluded from the last-release sources and suppresses the "
        "already-tagged no-op short-circuit, so the interval is measured against the "
        "PREVIOUS release. Refuses if the value is not a release tag.",
    )
    args = parser.parse_args(argv)

    if args.self_test:
        return self_test()
    return run(
        Path(args.repo_root).resolve(),
        dry_run=bool(args.dry_run),
        released_tag=args.released_tag,
    )


if __name__ == "__main__":
    sys.exit(main())
