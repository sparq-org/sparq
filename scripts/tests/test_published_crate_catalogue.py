#!/usr/bin/env python3
"""Keep every publishable-crate catalogue aligned with Cargo manifests."""

from pathlib import Path
import re
import tomllib
import unittest


ROOT = Path(__file__).resolve().parents[2]


def publishable_crates() -> set[str]:
    crates = set()
    for manifest in (ROOT / "crates").glob("*/Cargo.toml"):
        with manifest.open("rb") as handle:
            package = tomllib.load(handle)["package"]
        if package.get("publish") is not False:
            crates.add(package["name"])
    return crates


def published_runtime_dependencies(manifest: Path) -> set[str]:
    with manifest.open("rb") as handle:
        data = tomllib.load(handle)
    tables = [data.get("dependencies", {}), data.get("build-dependencies", {})]
    for target in data.get("target", {}).values():
        tables.extend((target.get("dependencies", {}), target.get("build-dependencies", {})))
    return {
        value.get("package", key) if isinstance(value, dict) else key
        for table in tables
        for key, value in table.items()
    }


def published_surface_mapping() -> dict[str, str]:
    source = (ROOT / "site/src/data/surfaces.ts").read_text(encoding="utf-8")
    block = re.search(
        r"export const PUBLISHED_CRATE_SURFACE = \{(?P<body>.*?)\n\} as const;",
        source,
        re.DOTALL,
    )
    assert block is not None, "PUBLISHED_CRATE_SURFACE declaration is missing"
    return dict(re.findall(r'^\s+"([^"]+)":\s+"([^"]+)",$', block["body"], re.MULTILINE))


class TestPublishedCrateCatalogue(unittest.TestCase):
    def test_all_catalogues_match_manifests(self) -> None:
        expected = publishable_crates()

        with (ROOT / "release-plz.toml").open("rb") as handle:
            release_packages = {item["name"] for item in tomllib.load(handle)["package"]}

        runbook = (ROOT / "docs/release.md").read_text(encoding="utf-8")
        publish_commands = set(re.findall(r"^cargo publish -p (sparq-[a-z0-9-]+)", runbook, re.MULTILINE))

        catalogue = (ROOT / "book/src/getting-started/rust-crates.md").read_text(encoding="utf-8")
        catalogue_rows = set(re.findall(r"^\| \[`(sparq-[a-z0-9-]+)`\]", catalogue, re.MULTILINE))

        surfaces = set(published_surface_mapping())
        self.assertEqual(release_packages, expected)
        self.assertEqual(publish_commands, expected)
        self.assertEqual(catalogue_rows, expected)
        self.assertEqual(surfaces, expected)

    def test_every_mapped_surface_slug_exists(self) -> None:
        source = (ROOT / "site/src/data/surfaces.ts").read_text(encoding="utf-8")
        known_slugs = set(re.findall(r'^\s+slug:\s+"([^"]+)",$', source, re.MULTILINE))
        mapped_slugs = set(published_surface_mapping().values())
        self.assertLessEqual(mapped_slugs, known_slugs)

    def test_bootstrap_commands_are_dependency_first(self) -> None:
        runbook = (ROOT / "docs/release.md").read_text(encoding="utf-8")
        commands = re.findall(r"^cargo publish -p (sparq-[a-z0-9-]+)", runbook, re.MULTILINE)
        position = {name: index for index, name in enumerate(commands)}
        for manifest in (ROOT / "crates").glob("*/Cargo.toml"):
            with manifest.open("rb") as handle:
                name = tomllib.load(handle)["package"]["name"]
            if name not in position:
                continue
            for dependency in published_runtime_dependencies(manifest) & position.keys():
                self.assertLess(position[dependency], position[name], f"{dependency} must precede {name}")


if __name__ == "__main__":
    unittest.main()
