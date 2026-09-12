#!/usr/bin/env python3
"""[GPT-6] Refuse a release whose tag, checkout, and package versions disagree."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tomllib


class SourceMismatch(ValueError):
    """The requested release does not identify the checked-out source."""


def git(repo: Path, revision: str) -> str:
    return subprocess.check_output(
        ["git", "rev-parse", "--verify", revision], cwd=repo, text=True,
        stderr=subprocess.PIPE,
    ).strip()


def validate_remote_tag(repo: Path, remote: str, tag: str, commit: str) -> None:
    """Fail if the public tag no longer resolves to the workflow build commit."""
    if not re.fullmatch(r"v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z.-]+)?", tag):
        raise SourceMismatch("release tag must be vX.Y.Z (optionally with a prerelease suffix)")
    if not re.fullmatch(r"[0-9a-f]{40}|[0-9a-f]{64}", commit):
        raise SourceMismatch("the workflow build commit is missing or invalid")
    output = subprocess.check_output(
        ["git", "ls-remote", remote, f"refs/tags/{tag}", f"refs/tags/{tag}^{{}}"],
        cwd=repo,
        text=True,
        stderr=subprocess.PIPE,
    )
    refs = dict(line.split("\t", 1)[::-1] for line in output.splitlines() if "\t" in line)
    # Annotated tags expose a peeled ^{} record; lightweight tags point directly at the commit.
    tagged = refs.get(f"refs/tags/{tag}^{{}}") or refs.get(f"refs/tags/{tag}")
    if tagged is None:
        raise SourceMismatch(f"remote {remote!r} does not contain {tag}")
    if tagged != commit:
        raise SourceMismatch(f"remote {tag} no longer identifies the workflow build commit")


def validate(repo: Path, tag: str, ref: str, commit: str, npm_manifests: list[str]) -> None:
    if not re.fullmatch(r"v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z.-]+)?", tag):
        raise SourceMismatch("release tag must be vX.Y.Z (optionally with a prerelease suffix)")
    if ref != f"refs/tags/{tag}":
        raise SourceMismatch(f"run this workflow at the exact tag {tag}, not {ref!r}")
    if not re.fullmatch(r"[0-9a-f]{40}|[0-9a-f]{64}", commit):
        raise SourceMismatch("the workflow build commit is missing or invalid")
    tagged = git(repo, f"refs/tags/{tag}^{{commit}}")
    if tagged != commit or git(repo, "HEAD") != commit:
        raise SourceMismatch("release tag, workflow build commit, and checkout HEAD must match")

    version = tag[1:]
    workspace = tomllib.loads((repo / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]
    if workspace["package"]["version"] != version:
        raise SourceMismatch(f"Cargo workspace version does not match {tag}")
    for member in workspace["members"]:
        package = tomllib.loads((repo / member / "Cargo.toml").read_text(encoding="utf-8"))["package"]
        # Private implementation crates may version independently. sparq-py is
        # private to Cargo but supplies the version of the published Python wheel.
        if package.get("publish") in (False, []) and member != "crates/sparq-py":
            continue
        declared = package.get("version")
        if declared != {"workspace": True} and declared != version:
            raise SourceMismatch(f"{member}/Cargo.toml version does not match {tag}")
    project = tomllib.loads((repo / "crates/sparq-py/pyproject.toml").read_text(encoding="utf-8"))["project"]
    if project.get("version") != version and not (
        "version" not in project and "version" in project.get("dynamic", [])
    ):
        raise SourceMismatch(f"PyPI project version does not match {tag}")
    lock_workspaces = {}
    if npm_manifests:
        package_lock = json.loads((repo / "package-lock.json").read_text(encoding="utf-8"))
        lock_workspaces = package_lock.get("packages")
        if not isinstance(lock_workspaces, dict):
            raise SourceMismatch("package-lock.json packages table is missing or invalid")
    for manifest in npm_manifests:
        package = json.loads((repo / manifest).read_text(encoding="utf-8"))
        if package.get("version") != version:
            raise SourceMismatch(f"{manifest} version does not match {tag}")
        workspace = Path(manifest).parent.as_posix()
        locked = lock_workspaces.get(workspace)
        if not isinstance(locked, dict) or locked.get("version") != version:
            raise SourceMismatch(
                f"package-lock.json workspace {workspace} version does not match {tag}"
            )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--repo-root", type=Path, default=Path("."))
    parser.add_argument("--publish", action="store_true", help="check selected publish.yml npm jobs")
    parser.add_argument(
        "--remote-tag",
        metavar="REMOTE",
        help="immediately re-check REMOTE's tag before a publication side effect",
    )
    args = parser.parse_args()
    # release.yml emits the primary npm client's SBOM as well as Rust artifacts.
    # publish.yml validates only selected npm jobs; the optional compatibility
    # package has its own version and must not block an unrelated publication.
    manifests = ["js/package.json", "packages/solid-server/package.json"]
    if args.publish:
        manifests = []
        for flag, manifest in (
            ("PUBLISH_NPM", "js/package.json"),
            ("PUBLISH_SOLID_SERVER", "packages/solid-server/package.json"),
            ("PUBLISH_EYEREASONER_COMPAT", "packages/eyereasoner-compat/package.json"),
        ):
            value = os.environ.get(flag)
            if value not in ("true", "false"):
                parser.error(f"{flag} must be explicitly true or false")
            if value == "true":
                manifests.append(manifest)
    try:
        if args.remote_tag:
            validate_remote_tag(args.repo_root, args.remote_tag, args.tag,
                                os.environ.get("GITHUB_SHA", ""))
        else:
            validate(args.repo_root, args.tag, os.environ.get("GITHUB_REF", ""),
                     os.environ.get("GITHUB_SHA", ""), manifests)
    except (SourceMismatch, OSError, subprocess.CalledProcessError, ValueError, KeyError, TypeError) as error:
        print(f"release source refused: {error}", file=sys.stderr)
        return 1
    qualifier = f" on {args.remote_tag}" if args.remote_tag else ""
    print(f"release source verified{qualifier}: {args.tag} at {os.environ['GITHUB_SHA']}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
