#!/usr/bin/env python3
"""Check dependency aliases in the query compiler's committed Noir packages."""
from pathlib import Path
import tempfile
import argparse
import json
import tomllib
from urllib.parse import urlparse


def check_alias(alias: str, package: Path) -> None:
    modules = {p.stem for p in (package / "src").rglob("*.nr") if p.stem not in {"lib", "main"}}
    if alias in modules:
        raise ValueError(f"dependency alias {alias!r} equals a nested module in {package}")


def check(root: Path, cache: Path | None = None, record_known: bool = False) -> list[dict]:
    cache = cache if cache is not None else Path.home() / "nargo"
    pending = [root / "zk/noir-query/core"]
    pending.extend(p.parent for p in (root / "zk/noir-query/fixtures").rglob("Nargo.toml"))
    seen = set()
    known = []
    while pending:
        package = pending.pop().resolve()
        if package in seen:
            continue
        seen.add(package)
        manifest = tomllib.loads((package / "Nargo.toml").read_text())
        for alias, dep in manifest.get("dependencies", {}).items():
            if "path" in dep:
                target = (package / dep["path"]).resolve()
            else:
                url = urlparse(dep["git"])
                revision = dep.get("tag", dep.get("rev"))
                if revision is None:
                    raise ValueError(f"dependency has no pinned revision: {alias}")
                target = cache / url.netloc / url.path.strip("/") / revision
                target /= dep.get("directory", "")
            if not (target / "src").is_dir():
                raise ValueError(f"load dependencies with pinned nargo first: {target}")
            try:
                check_alias(alias, target)
            except ValueError:
                expected = cache / "github.com/sparq-org/noir_XPath/v0.3.0/vendor/sha256"
                if not record_known or alias != "sha256" or target.resolve() != expected.resolve():
                    raise
                known.append({
                    "status": "known_failure",
                    "id": "xpath-vendored-sha-alias",
                    "package": str(target),
                    "reason": "Pinned XPath vendored hash alias equals its nested module name.",
                })
            pending.append(target)
    return known


def self_test() -> None:
    with tempfile.TemporaryDirectory() as temporary:
        package = Path(temporary)
        (package / "src").mkdir()
        (package / "src/inner.nr").write_text("pub fn value() -> Field { 1 }\n")
        check_alias("outer", package)
        try:
            check_alias("inner", package)
        except ValueError:
            pass
        else:
            raise AssertionError("nested module alias was accepted")

        root = package / "project"
        core = root / "zk/noir-query/core"
        core.mkdir(parents=True)
        (core / "Nargo.toml").write_text(
            '[dependencies]\nouter={git="https://example.invalid/library",rev="snapshot"}\n'
        )
        git_package = package / "cache/example.invalid/library/snapshot"
        (git_package / "src").mkdir(parents=True)
        (git_package / "src/lib.nr").write_text("pub fn value() -> Field { 1 }\n")
        (git_package / "Nargo.toml").write_text('[dependencies]\ninner={path="../nested"}\n')
        nested = git_package.parent / "nested"
        (nested / "src").mkdir(parents=True)
        (nested / "src/inner.nr").write_text("pub fn value() -> Field { 1 }\n")
        (nested / "Nargo.toml").write_text("[dependencies]\n")
        try:
            check(root, package / "cache")
        except ValueError as error:
            assert "alias 'inner'" in str(error), error
        else:
            raise AssertionError("git dependency's transitive alias was accepted")
        (git_package / "Nargo.toml").write_text('[dependencies]\nlocal={path="../nested"}\n')
        check(root, package / "cache")

        cache = package / "known-cache"
        xpath = cache / "github.com/sparq-org/noir_XPath/v0.3.0/xpath"
        (xpath / "src").mkdir(parents=True)
        (xpath / "src/lib.nr").write_text("pub fn value() -> Field { 1 }\n")
        (xpath / "Nargo.toml").write_text('[dependencies]\nsha256={path="../vendor/sha256"}\n')
        nested = xpath.parent / "vendor/sha256"
        (nested / "src").mkdir(parents=True)
        (nested / "src/sha256.nr").write_text("pub fn value() -> Field { 1 }\n")
        (nested / "Nargo.toml").write_text("[dependencies]\n")
        (core / "Nargo.toml").write_text('[dependencies]\nsparq_xpath={git="https://github.com/sparq-org/noir_XPath",tag="v0.3.0",directory="xpath"}\n')
        try:
            check(root, cache)
        except ValueError as error:
            assert "alias 'sha256'" in str(error)
        else:
            raise AssertionError("strict audit accepted the registered collision")
        failures = check(root, cache, record_known=True)
        assert len(failures) == 1 and failures[0]["id"] == "xpath-vendored-sha-alias"
        (xpath / "Nargo.toml").write_text('[dependencies]\ninner={path="../vendor/sha256"}\n')
        (nested / "src/inner.nr").write_text("pub fn value() -> Field { 1 }\n")
        try:
            check(root, cache, record_known=True)
        except ValueError as error:
            assert "alias 'inner'" in str(error)
        else:
            raise AssertionError("known-failure mode accepted an unrelated collision")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--record-known-failures", action="store_true")
    args = parser.parse_args()
    self_test()
    known = check(Path(__file__).resolve().parents[1], record_known=args.record_known_failures)
    if known:
        for failure in known:
            print(json.dumps(failure))
    else:
        print("Noir dependency aliases are distinct from nested module names.")
