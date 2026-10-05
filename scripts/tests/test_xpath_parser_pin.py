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
    fork_package = tomllib.loads((fork / "Cargo.toml").read_text())["package"]
    # The workspace parser is the published fork itself, never a registry range.
    parser = workspace["workspace"]["dependencies"]["spargebra"]
    if parser.get("package") != fork_package["name"] or (root / parser["path"]).resolve() != fork.resolve():
        raise ValueError("the workspace parser must be the published local fork")
    if parser["version"].lstrip("=") != fork_package["version"]:
        raise ValueError("the workspace parser version must match the fork")
    # The detached oracle reaches the same fork through the upstream-named shim.
    shim = (detached / manifest["patch"]["crates-io"]["spargebra"]["path"]).resolve()
    shim_fork = tomllib.loads((shim / "Cargo.toml").read_text())["dependencies"]["fork"]
    if (shim / shim_fork["path"]).resolve() != fork.resolve():
        raise ValueError("detached oracle must select the same local parser fork")


class XPathParserPin(unittest.TestCase):
    def test_detached_oracle_cannot_prefer_a_newer_registry_parser(self):
        validate_parser_pin(ROOT)

    def test_registry_parser_substitution_is_rejected(self):
        import tempfile
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for path in ["Cargo.toml", "vendor/spargebra/Cargo.toml", "vendor/spargebra-shim/Cargo.toml",
                         "zk/xpath/differential/Cargo.toml"]:
                output = root / path
                output.parent.mkdir(parents=True, exist_ok=True)
                output.write_text((ROOT / path).read_text())
            validate_parser_pin(root)
            manifest = root / "Cargo.toml"
            manifest.write_text(manifest.read_text().replace('package = "sparq-spargebra", ', '', 1))
            with self.assertRaisesRegex(ValueError, "published local fork"):
                validate_parser_pin(root)


if __name__ == "__main__":
    unittest.main()
