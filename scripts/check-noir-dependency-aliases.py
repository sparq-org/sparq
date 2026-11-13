#!/usr/bin/env python3
"""Check dependency aliases in the query compiler's committed Noir packages."""
from pathlib import Path
import tempfile
import tomllib
from urllib.parse import urlparse


def check_alias(alias: str, package: Path) -> None:
    modules = {p.stem for p in (package / "src").rglob("*.nr") if p.stem not in {"lib", "main"}}
    if alias in modules:
        raise ValueError(f"dependency alias {alias!r} equals a nested module in {package}")


def check(root: Path) -> None:
    pending = [root / "zk/noir-query/core"]
    pending.extend(p.parent for p in (root / "zk/noir-query/fixtures").rglob("Nargo.toml"))
    seen = set()
    while pending:
        package = pending.pop().resolve()
        if package in seen:
            continue
        seen.add(package)
        manifest = tomllib.loads((package / "Nargo.toml").read_text())
        for alias, dep in manifest.get("dependencies", {}).items():
            if "path" in dep:
                target = (package / dep["path"]).resolve()
                pending.append(target)
            else:
                url = urlparse(dep["git"])
                revision = dep.get("tag", dep.get("rev"))
                if revision is None:
                    raise ValueError(f"dependency has no pinned revision: {alias}")
                target = Path.home() / "nargo" / url.netloc / url.path.strip("/") / revision
                target /= dep.get("directory", "")
            if not (target / "src").is_dir():
                raise ValueError(f"load dependencies with pinned nargo first: {target}")
            check_alias(alias, target)


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


if __name__ == "__main__":
    self_test()
    check(Path(__file__).resolve().parents[1])
    print("Noir dependency aliases are distinct from nested module names.")
