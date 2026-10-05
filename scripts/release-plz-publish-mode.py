#!/usr/bin/env python3
"""Decide whether release-plz may publish to crates.io on this run, fail-closed.

[OPUS-5.5] v0.1.4 release path. ``release-plz.toml`` names the crates.io publish set with
per-package ``publish = true`` keys, but that set may only go live AFTER the one-off manual
bootstrap (docs/release.md §4) created every crate — crates.io Trusted Publishing cannot be
registered on a crate that does not exist, and a half-published closure is unusable. This
script turns registry facts into one of two modes and writes the effective config release-plz
must use:

``tag-only``
    No publish-set crate exists on crates.io yet (pre-bootstrap), or every publish-set crate
    already carries the workspace version (nothing to publish). The effective config forces
    every ``publish`` key to ``false``: ``release-plz release`` only cuts the ``v<version>``
    tag, exactly the pre-flip behaviour. No crates.io credential is needed.
``publish``
    Every publish-set crate exists and at least one lacks the workspace version. The
    effective config is ``release-plz.toml`` with ``git_only = false``; the workflow mints a
    short-lived crates.io OIDC token for this mode only.

It REFUSES (exit 1) on a partial bootstrap (some publish-set crates exist, others do not),
on any non-200/404 registry answer, and on an unreadable or empty publish set. A crates.io
version can never be unpublished, so indeterminacy never selects ``publish``.

``--purpose pr`` (the Release-PR job) only asks whether the bootstrap is complete:
``bootstrapped=true`` selects the post-bootstrap config (``git_only = false``) so
``release-plz release-pr`` resolves the published closure; ``false`` keeps the manual
version-PR path. Partial bootstrap still refuses.

Outputs ``mode=…``, ``bootstrapped=…`` and ``config=…`` to ``$GITHUB_OUTPUT`` when set.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
import time
import tomllib
import urllib.error
import urllib.request
from pathlib import Path
from typing import Callable

USER_AGENT = "sparq-release-plz-publish-mode/1.0 (+https://github.com/sparq-org/sparq)"
# (status, versions) — status is 200 or 404; versions is the set of published version strings.
Fetch = Callable[[str], tuple[int, set[str]]]


class Refusal(Exception):
    """An indeterminate or inconsistent state: never publish."""


def publish_set(config: dict) -> list[str]:
    names = [
        pkg["name"]
        for pkg in config.get("package", [])
        if pkg.get("publish", config.get("workspace", {}).get("publish")) is True
    ]
    if not names:
        raise Refusal("release-plz.toml names no crate with `publish = true`")
    return names


def workspace_version(repo_root: Path) -> str:
    manifest = tomllib.loads((repo_root / "Cargo.toml").read_text(encoding="utf-8"))
    version = manifest.get("workspace", {}).get("package", {}).get("version")
    if not isinstance(version, str) or not version:
        raise Refusal("Cargo.toml has no [workspace.package] version")
    return version


def crates_io_fetch(name: str) -> tuple[int, set[str]]:
    url = f"https://crates.io/api/v1/crates/{name}/versions"
    last = ""
    for attempt in range(3):
        request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                body = json.load(response)
                return 200, {v["num"] for v in body.get("versions", [])}
        except urllib.error.HTTPError as err:
            if err.code == 404:
                return 404, set()
            last = f"HTTP {err.code}"
        except (urllib.error.URLError, TimeoutError, ValueError, KeyError) as err:
            last = repr(err)
        time.sleep(2 * (attempt + 1))
    raise Refusal(f"crates.io lookup for {name} is indeterminate ({last})")


def decide(names: list[str], version: str, fetch: Fetch) -> tuple[str, bool, list[str]]:
    existing, missing, pending = [], [], []
    for name in names:
        status, versions = fetch(name)
        if status == 404:
            missing.append(name)
            continue
        if status != 200:
            raise Refusal(f"crates.io returned {status} for {name}")
        existing.append(name)
        if version not in versions:
            pending.append(name)
    if existing and missing:
        raise Refusal(
            "partial crates.io bootstrap — present: "
            + ", ".join(existing)
            + "; absent: "
            + ", ".join(missing)
            + ". Finish the docs/release.md §4 bootstrap before CI may publish."
        )
    if missing:
        return "tag-only", False, [f"pre-bootstrap: none of the {len(names)} publish-set crates exist"]
    if not pending:
        return "tag-only", True, [f"every publish-set crate already has {version}; nothing to publish"]
    return "publish", True, [f"{len(pending)} publish-set crate(s) lack {version}: " + ", ".join(pending)]


def effective_config(text: str, mode: str) -> str:
    if mode == "tag-only":
        out = re.sub(r"(?m)^publish = true$", "publish = false", text)
        if any(p.get("publish") is True for p in tomllib.loads(out).get("package", [])):
            raise Refusal("could not disable every per-package publish key")
        if tomllib.loads(out)["workspace"].get("publish") is not False:
            raise Refusal("workspace publish must be false in tag-only mode")
        return out
    out, count = re.subn(r"(?m)^git_only = true$", "git_only = false", text)
    if count != 1 or tomllib.loads(out)["workspace"].get("git_only") is not False:
        raise Refusal("could not set workspace git_only = false for publish mode")
    return out


def main(argv: list[str] | None = None, fetch: Fetch = crates_io_fetch) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--repo-root", default=".", type=Path)
    parser.add_argument("--out", required=True, type=Path, help="effective config path")
    parser.add_argument("--purpose", choices=("release", "pr"), default="release")
    args = parser.parse_args(argv)
    try:
        text = (args.repo_root / "release-plz.toml").read_text(encoding="utf-8")
        names = publish_set(tomllib.loads(text))
        version = workspace_version(args.repo_root)
        mode, bootstrapped, reasons = decide(names, version, fetch)
        if args.purpose == "pr":
            # Release-PR never publishes; once bootstrapped it needs the registry baseline.
            config_mode = "publish" if bootstrapped else "tag-only"
        else:
            config_mode = mode
        args.out.write_text(effective_config(text, config_mode), encoding="utf-8")
    except (Refusal, OSError, tomllib.TOMLDecodeError) as err:
        print(f"::error::release-plz publish mode REFUSED: {err}", file=sys.stderr)
        return 1
    for reason in reasons:
        print(f"[release-plz-publish-mode] {reason}")
    print(
        f"[release-plz-publish-mode] mode={mode} bootstrapped={str(bootstrapped).lower()} "
        f"config={args.out}"
    )
    output = os.environ.get("GITHUB_OUTPUT")
    if output:
        with open(output, "a", encoding="utf-8") as handle:
            handle.write(
                f"mode={mode}\nbootstrapped={str(bootstrapped).lower()}\nconfig={args.out}\n"
            )
    return 0


if __name__ == "__main__":
    sys.exit(main())
