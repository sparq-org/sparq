#!/usr/bin/env python3
"""Strip research-only feature edges from the manifests of crates.io-published crates.

WHY
===
The crates.io publish set (release-plz.toml's `version_group`, every workspace member
without `publish = false`) is the focused core: sparq-core, the engine, reasoner, SHACL,
HDT, the server and the CLI. Several of those crates reach research crates (sparq-zk,
sparq-trust, sparq-policy, sparq-solid, sparq-terse, ...) through *optional* features.
`cargo publish` refuses a crate whose manifest names any dependency that is not on the
registry, optional or not, so without this script every such edge would drag its whole
research subtree onto crates.io (28 crates instead of 12).

This script rewrites the publishable manifests IN PLACE, for packaging only:

* an optional dependency on a `publish = false` workspace crate is removed;
* every feature that needs a removed dependency (`dep:x`, `x`, `x/feat`) is removed,
  transitively, together with every feature that enables a removed feature;
* a weak `x?/feat` item naming a removed dependency is dropped from its feature;
* dev-dependencies on unpublished crates are removed (Cargo already drops path-only ones);
* an optional dependency left unmentioned by every surviving feature is removed;
* the root workspace is trimmed to the publishable members (plus the vendored
  `sparq-spargebra` fork), so unpublished crates that enable stripped features cannot
  break packaging-time resolution;
* `[package.metadata.docs.rs] features` naming a removed feature are dropped, and an
  example/test/bench that requires one is removed from the manifest and the package.

Published crates therefore ship without those features; they stay available from a git
build of the workspace. The code behind them is `#[cfg(feature = "...")]`-gated, so it is
compiled out. It REFUSES (exit 1) when stripping cannot be done soundly: a non-optional
dependency on an unpublished crate, a `default` feature that would be removed, or a
published crate enabling a removed feature of another published crate.

USAGE
=====
  python3 scripts/publish-strip.py --check   # report what would be stripped, write nothing
  python3 scripts/publish-strip.py           # rewrite the manifests (CI release only)
  python3 scripts/publish-strip.py --with-vendored   # + the sparq-spargebra fork
                                             # (bootstrap publish and dry runs)

Run it in a throwaway checkout (or `git stash`/`git checkout -- .` afterwards), then
`cargo publish --allow-dirty ...`. Never commit the stripped manifests.

Needs `tomlkit` (format-preserving TOML):
`python3 -m pip install --require-hashes -r .github/requirements/publish-strip.txt`.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

try:
    import tomlkit
except ImportError:  # pragma: no cover
    sys.exit("publish-strip: needs tomlkit (python3 -m pip install tomlkit==0.13.3)")

DEP_KEYS = ("dependencies", "build-dependencies", "dev-dependencies")
TARGET_KEYS = ("bin", "example", "test", "bench")


class StripError(Exception):
    pass


def _dep_tables(doc):
    """Yield (kind, table) for every dependency table, target-specific ones included."""
    for key in DEP_KEYS:
        if key in doc:
            yield key, doc[key]
    for cfg in (doc.get("target") or {}).values():
        for key in DEP_KEYS:
            if key in cfg:
                yield key, cfg[key]


def _real_name(key, spec):
    if isinstance(spec, dict) and isinstance(spec.get("package"), str):
        return str(spec["package"])
    return str(key)


def load_workspace(root: Path):
    ws = tomlkit.parse((root / "Cargo.toml").read_text(encoding="utf-8"))
    members = {}
    for member in ws["workspace"]["members"]:
        path = root / str(member) / "Cargo.toml"
        doc = tomlkit.parse(path.read_text(encoding="utf-8"))
        publish = doc["package"].get("publish")
        publishable = publish is not False and publish != []
        members[str(doc["package"]["name"])] = (path, doc, publishable)
    return members


def _item_dep(item: str):
    """For a feature item, return (dep_key, kind) with kind in {dep, strong, weak, plain}."""
    if item.startswith("dep:"):
        return item[4:], "dep"
    if "?/" in item:
        return item.split("?/", 1)[0], "weak"
    if "/" in item:
        return item.split("/", 1)[0], "strong"
    return item, "plain"


def plan(members):
    """Compute, per publishable crate, the dep keys and features to strip."""
    unpublished = {name for name, (_, _, pub) in members.items() if not pub}
    published = {name for name, (_, _, pub) in members.items() if pub}
    # dep key -> real package, per crate (normal/build only; dev handled separately).
    removed_deps: dict[str, set[str]] = {}
    dev_removed: dict[str, set[str]] = {}
    dep_target: dict[str, dict[str, str]] = {}
    for name in published:
        _, doc, _ = members[name]
        removed_deps[name] = set()
        dev_removed[name] = set()
        dep_target[name] = {}
        for kind, table in _dep_tables(doc):
            for key, spec in table.items():
                real = _real_name(key, spec)
                if kind != "dev-dependencies":
                    dep_target[name][str(key)] = real
                if real not in unpublished:
                    continue
                if kind == "dev-dependencies":
                    dev_removed[name].add(str(key))
                elif isinstance(spec, dict) and spec.get("optional"):
                    removed_deps[name].add(str(key))
                else:
                    raise StripError(
                        f"{name}: non-optional {kind} on unpublished crate {real!r}; "
                        "make it optional behind a feature or publish it"
                    )

    removed_feats: dict[str, set[str]] = {name: set() for name in published}
    changed = True
    while changed:
        changed = False
        for name in published:
            _, doc, _ = members[name]
            feats = doc.get("features") or {}
            for feat, items in feats.items():
                if feat in removed_feats[name]:
                    continue
                for item in items:
                    key, kind = _item_dep(str(item))
                    target = dep_target[name].get(key)
                    gone = False
                    if kind == "plain" and (
                        key in removed_feats[name] or key in removed_deps[name]
                    ):
                        gone = True
                    elif kind == "dep" and key in removed_deps[name]:
                        gone = True
                    elif kind == "strong" and key in removed_deps[name]:
                        gone = True
                    elif kind in ("strong", "weak") and target in published:
                        sub = str(item).split("/", 1)[1]
                        if sub in removed_feats[target]:
                            gone = True
                    if gone:
                        if feat == "default":
                            raise StripError(
                                f"{name}: default feature needs {item!r}, which is "
                                "stripped; published defaults must stay in the publish set"
                            )
                        removed_feats[name].add(str(feat))
                        changed = True
                        break

    # A published crate enabling a removed feature of another published crate through a
    # dependency's `features = [...]` list cannot be stripped soundly.
    for name in published:
        _, doc, _ = members[name]
        for kind, table in _dep_tables(doc):
            if kind == "dev-dependencies":
                continue
            for key, spec in table.items():
                real = _real_name(key, spec)
                if real not in published or not isinstance(spec, dict):
                    continue
                for f in spec.get("features") or []:
                    if str(f) in removed_feats[real]:
                        raise StripError(
                            f"{name}: dependency {key!r} enables {real}/{f}, which is "
                            "stripped from the published crate"
                        )
    # An optional dependency that no surviving feature mentions any more would surface as
    # an implicit feature named after the crate (e.g. `quinn` once `http3` is gone). Strip
    # it too: it was only ever reachable through a removed feature.
    for name in published:
        _, doc, _ = members[name]
        feats = doc.get("features") or {}
        mentioned = {
            _item_dep(str(item))[0]
            for feat, items in feats.items()
            if feat not in removed_feats[name]
            for item in items
            if not (
                _item_dep(str(item))[1] == "weak"
                and _item_dep(str(item))[0] in removed_deps[name]
            )
        }
        for kind, table in _dep_tables(doc):
            if kind == "dev-dependencies":
                continue
            for key, spec in table.items():
                if (
                    isinstance(spec, dict)
                    and spec.get("optional")
                    and str(key) not in mentioned
                    and str(key) not in removed_deps[name]
                    and _uses_dep_syntax(feats, str(key))
                ):
                    removed_deps[name].add(str(key))
    return removed_deps, dev_removed, removed_feats


def _uses_dep_syntax(feats, key):
    """True if any feature (removed or not) names `dep:key`, i.e. the dependency never had
    an implicit feature of its own. Only those can be orphaned by stripping."""
    return any(str(item) == f"dep:{key}" for items in feats.values() for item in items)


def trim_workspace(root: Path, members, with_vendored: bool):
    """Keep only publishable members in the root workspace, so unpublished crates that
    enable stripped features cannot fail the packaging-time dependency resolution."""
    path = root / "Cargo.toml"
    ws = tomlkit.parse(path.read_text(encoding="utf-8"))
    keep = {
        str(members[name][0].parent.relative_to(root))
        for name, (_, _, pub) in members.items()
        if pub
    }
    for key in ("members", "default-members"):
        if key in ws["workspace"]:
            ws["workspace"][key] = [m for m in ws["workspace"][key] if str(m) in keep]
    # --with-vendored: path-dependency forks outside the workspace (vendor/spargebra ->
    # sparq-spargebra) join it, so `cargo publish --workspace [--dry-run]` packages them
    # first and verifies their dependants against the local copy. Used for the first
    # (bootstrap) publish and for dry runs; once the fork is on crates.io, CI releases
    # leave it out and resolve the registry copy (it is versioned independently).
    for key, spec in (ws["workspace"].get("dependencies") or {}).items():
        if not with_vendored:
            break
        if not (isinstance(spec, dict) and str(spec.get("path", "")).startswith("vendor/")):
            continue
        vendored = str(spec["path"])
        manifest = root / vendored / "Cargo.toml"
        vdoc = tomlkit.parse(manifest.read_text(encoding="utf-8"))
        if "workspace" in vdoc:
            del vdoc["workspace"]
        manifest.write_text(tomlkit.dumps(vdoc), encoding="utf-8")
        ws["workspace"]["members"].append(vendored)
        if "exclude" in ws["workspace"]:
            ws["workspace"]["exclude"] = [
                e for e in ws["workspace"]["exclude"] if str(e) != vendored
            ]
    # The re-export shim patch only serves unpublished harnesses that are gone now.
    if "patch" in ws:
        del ws["patch"]
    path.write_text(tomlkit.dumps(ws), encoding="utf-8")


def apply(members, removed_deps, dev_removed, removed_feats):
    for name, feats_gone in removed_feats.items():
        path, doc, _ = members[name]
        deps_gone = removed_deps[name]
        devs_gone = dev_removed[name]
        if not (feats_gone or deps_gone or devs_gone):
            continue
        for kind, table in _dep_tables(doc):
            for key in list(table.keys()):
                if (kind == "dev-dependencies" and key in devs_gone) or (
                    kind != "dev-dependencies" and key in deps_gone
                ):
                    del table[key]
        feats = doc.get("features")
        if feats is not None:
            for feat in list(feats.keys()):
                if feat in feats_gone:
                    del feats[feat]
                    continue
                items = feats[feat]
                keep = [
                    i
                    for i in items
                    if not (
                        _item_dep(str(i))[1] == "weak"
                        and _item_dep(str(i))[0] in deps_gone
                    )
                ]
                if len(keep) != len(items):
                    feats[feat] = keep
        docsrs = ((doc.get("package") or {}).get("metadata") or {}).get("docs") or {}
        docsrs = docsrs.get("rs") if hasattr(docsrs, "get") else None
        if docsrs is not None and "features" in docsrs:
            docsrs["features"] = [f for f in docsrs["features"] if f not in feats_gone]
        for tkey in TARGET_KEYS:
            targets = doc.get(tkey)
            if not targets:
                continue
            for target in list(targets):
                req = target.get("required-features") or []
                if not any(str(f) in feats_gone for f in req):
                    continue
                if tkey == "bin":
                    raise StripError(
                        f"{name}: binary {target.get('name')!r} requires a stripped feature"
                    )
                # Examples/tests/benches are never built by `cargo publish`. Drop the
                # target and leave its source out of the package, so Cargo neither
                # rejects the manifest nor auto-discovers the file without its gate.
                default_dir = {"example": "examples", "test": "tests", "bench": "benches"}
                src = str(target.get("path") or f"{default_dir[tkey]}/{target['name']}.rs")
                targets.remove(target)
                package = doc["package"]
                if "include" in package:
                    package["include"] = [i for i in package["include"] if str(i) != src]
                else:
                    package.setdefault("exclude", []).append(src)
            if not targets:
                del doc[tkey]
        path.write_text(tomlkit.dumps(doc), encoding="utf-8")


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--check", action="store_true", help="report only; write nothing")
    parser.add_argument(
        "--with-vendored",
        action="store_true",
        help="also make vendor/* path forks (sparq-spargebra) workspace members",
    )
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parent.parent)
    args = parser.parse_args(argv)
    members = load_workspace(args.root)
    try:
        removed_deps, dev_removed, removed_feats = plan(members)
    except StripError as error:
        print(f"publish-strip: REFUSING: {error}", file=sys.stderr)
        return 1
    for name in sorted(removed_feats):
        if removed_deps[name] or removed_feats[name] or dev_removed[name]:
            print(f"{name}:")
            if removed_deps[name]:
                print(f"  optional deps: {', '.join(sorted(removed_deps[name]))}")
            if removed_feats[name]:
                print(f"  features:      {', '.join(sorted(removed_feats[name]))}")
            if dev_removed[name]:
                print(f"  dev-deps:      {', '.join(sorted(dev_removed[name]))}")
    if not args.check:
        try:
            apply(members, removed_deps, dev_removed, removed_feats)
            trim_workspace(args.root, members, args.with_vendored)
        except StripError as error:
            print(f"publish-strip: REFUSING: {error}", file=sys.stderr)
            return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
