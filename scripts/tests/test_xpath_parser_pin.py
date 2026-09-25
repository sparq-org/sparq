"""[GPT-6] Static cold-resolution contract; does not invoke Cargo or a network."""
from pathlib import Path
import tomllib
import unittest

ROOT = Path(__file__).resolve().parents[2]


def validate_parser_pin(root):
    workspace = tomllib.loads((root / "Cargo.toml").read_text())
    detached = root / "zk/xpath/differential"
    manifest = tomllib.loads((detached / "Cargo.toml").read_text())
    fork = root / "vendor/spargebra"
    version = tomllib.loads((fork / "Cargo.toml").read_text())["package"]["version"]
    if workspace["workspace"]["dependencies"]["spargebra"]["version"] != "=" + version:
        raise ValueError("vendored patch selection requires an exact workspace parser version")
    patch = manifest["patch"]["crates-io"]["spargebra"]
    if (detached / patch["path"]).resolve() != fork.resolve():
        raise ValueError("detached oracle must select the same local parser fork")


class XPathParserPin(unittest.TestCase):
    def test_detached_oracle_cannot_prefer_a_newer_registry_parser(self):
        validate_parser_pin(ROOT)

    def test_registry_range_mutation_is_rejected(self):
        import tempfile
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for path in ["Cargo.toml", "vendor/spargebra/Cargo.toml", "zk/xpath/differential/Cargo.toml"]:
                output = root / path
                output.parent.mkdir(parents=True, exist_ok=True)
                output.write_text((ROOT / path).read_text())
            manifest = root / "Cargo.toml"
            manifest.write_text(manifest.read_text().replace('version = "=0.4.6"', 'version = "0.4"'))
            with self.assertRaisesRegex(ValueError, "exact workspace"):
                validate_parser_pin(root)


if __name__ == "__main__":
    unittest.main()
