#!/usr/bin/env python3
"""[GPT-6] Verify SDK vendor provenance and optionally run synthetic API checks."""

from pathlib import Path
import argparse
import hashlib
import json
import os
import shutil
import subprocess
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parents[1]


def reconstruct(package: dict, directory: Path) -> None:
    """[GPT-6] Reverse to exact upstream bytes, then replay the recorded patch."""
    name = f"{package['name']}-{package['version']}"
    with tempfile.TemporaryDirectory(prefix="sparq-sdk-reconstruct-") as temporary:
        restored = Path(temporary) / name
        shutil.copytree(directory, restored)
        for addition in package.get("additional_files", []):
            (restored / addition["path"]).unlink()
        patch = str(ROOT / f"{name}.patch")
        command = ["git", "apply", "--whitespace=nowarn", "-p1"]
        subprocess.run(command + ["--reverse", patch], cwd=restored, check=True, timeout=60)
        upstream = package["upstream_files"]
        assert {str(p.relative_to(restored)) for p in restored.rglob("*") if p.is_file()} == set(upstream)
        for path, sha in upstream.items():
            assert hashlib.sha256((restored / path).read_bytes()).hexdigest() == sha, (name, path, "reconstructed upstream")
        subprocess.run(command + [patch], cwd=restored, check=True, timeout=60)
        for path in upstream:
            assert (restored / path).read_bytes() == (directory / path).read_bytes(), (name, path, "patch replay")


def selected_vendor_manifests(metadata: dict, versions: dict) -> None:
    """[GPT-6] A same-version registry or unrelated local package is not our patch."""
    active = {node["id"] for node in metadata["resolve"]["nodes"]}
    for name, version in versions.items():
        selected = [package for package in metadata["packages"]
                    if package["name"] == name and package["id"] in active]
        expected = (ROOT / f"{name}-{version}" / "Cargo.toml").resolve()
        assert len(selected) == 1, (name, "resolved vendor inventory")
        package = selected[0]
        assert package["version"] == version and package.get("source") is None, (name, "resolved vendor source")
        assert Path(package["manifest_path"]).resolve() == expected, (name, "resolved vendor manifest path")


def patch_table_paths(manifest: Path, versions: dict, inventory: set) -> None:
    """[GPT-6] Bind each detached patch edge to the exact inventoried directory."""
    patches = tomllib.loads(manifest.read_text()).get("patch", {}).get("crates-io", {})
    assert set(patches) & set(versions) == inventory, (str(manifest), "manifest patch inventory")
    for name in inventory:
        entry = patches[name]
        expected = (ROOT / f"{name}-{versions[name]}").resolve()
        assert isinstance(entry, dict) and set(entry) == {"path"}, (name, "manifest patch path entry")
        assert (manifest.parent / entry["path"]).resolve() == expected, (name, "manifest vendor patch path")


def check() -> None:
    metadata = json.loads((ROOT / "UPSTREAM.json").read_text())
    expected_versions = {
        "risc0-build": "3.0.6", "rzup": "0.5.2", "ark-relations": "0.5.1",
        "ark-crypto-primitives": "0.5.0", "risc0-zkvm": "3.0.6",
        "risc0-zkos-v1compat": "2.2.3",
    }
    guest_patches = {"ark-relations", "ark-crypto-primitives", "risc0-zkvm", "risc0-zkos-v1compat"}
    for relative, inventory in [("Cargo.toml", set(expected_versions)),
                                ("methods/guest/Cargo.toml", guest_patches)]:
        patch_table_paths(REPO / "zk/sparql-evaluator" / relative, expected_versions, inventory)
    assert len(metadata["packages"]) == len(expected_versions), "complete patch inventory"
    assert {p["name"]: p["version"] for p in metadata["packages"]} == expected_versions, "complete patch inventory"
    for package in metadata["packages"]:
        name = f"{package['name']}-{package['version']}"
        directory = ROOT / name
        expected = dict(package["upstream_files"])
        for patch in package["patch_files"]:
            assert expected[patch["path"]] == patch["upstream_sha256"]
            expected[patch["path"]] = patch["patched_sha256"]
        for addition in package.get("additional_files", []):
            assert addition["path"] not in expected
            expected[addition["path"]] = addition["sha256"]
        assert {str(p.relative_to(directory)) for p in directory.rglob("*") if p.is_file()} == set(expected)
        for path, sha in expected.items():
            assert hashlib.sha256((directory / path).read_bytes()).hexdigest() == sha, (name, path)
        assert hashlib.sha256((ROOT / f"{name}.patch").read_bytes()).hexdigest() == package["patch_sha256"]
        reconstruct(package, directory)
    rzup = tomllib.loads((ROOT / "rzup-0.5.2/Cargo.toml").read_text())
    assert rzup["features"]["default"] == ["cli", "install", "publish"]
    assert all("signatures" in rzup["features"][f] for f in ["install", "publish"])
    assert rzup["dependencies"]["rsa"]["optional"] is True
    primitives = tomllib.loads((ROOT / "ark-crypto-primitives-0.5.0/Cargo.toml").read_text())
    assert primitives["features"]["default"] == ["std"]
    assert primitives["dependencies"]["derivative"]["optional"] is True
    assert all("dep:derivative" in primitives["features"][f]
               for f in ["crh", "encryption", "signature"])
    assert all("crh" in primitives["features"][f] for f in ["commitment", "merkle_tree"])
    vm = tomllib.loads((ROOT / "risc0-zkvm-3.0.6/Cargo.toml").read_text())
    kernel = tomllib.loads((ROOT / "risc0-zkos-v1compat-2.2.3/Cargo.toml").read_text())
    build = tomllib.loads((ROOT / "risc0-build-3.0.6/Cargo.toml").read_text())
    assert vm["features"]["default"] == ["client", "bonsai"]
    assert vm["dependencies"]["rrs-lib"]["optional"] is True
    assert "dep:rrs-lib" in vm["features"]["prove"]
    assert kernel["features"]["default"] == ["kernel"]
    assert kernel["features"]["kernel"] == ["dep:include_bytes_aligned", "dep:no_std_strings"]
    assert kernel["bin"][0]["required-features"] == ["kernel"]
    assert all(kernel["dependencies"][name]["optional"] is True
               for name in ["include_bytes_aligned", "no_std_strings"])
    assert all(p["dependencies"]["risc0-zkos-v1compat"]["default-features"] is False
               for p in [vm, build])
    # [GPT-6] New dependency edges must not change kernel instructions or blobs.
    for package in metadata["packages"]:
        if package["name"] in {"risc0-zkvm", "risc0-zkos-v1compat"}:
            assert [p["path"] for p in package["patch_files"]] == ["Cargo.toml"]
    for relative in ["Cargo.lock", "methods/guest/Cargo.lock"]:
        lock = tomllib.loads((REPO / "zk/sparql-evaluator" / relative).read_text())
        # [GPT-6] Registry entries are not evidence that Cargo selected our patch.
        expected_patches = {"ark-relations", "ark-crypto-primitives", "risc0-zkvm", "risc0-zkos-v1compat"}
        if relative == "Cargo.lock":
            expected_patches |= {"risc0-build", "rzup"}
        for package in metadata["packages"]:
            if package["name"] in expected_patches:
                selected = [p for p in lock["package"] if p["name"] == package["name"]]
                assert len(selected) == 1, (relative, package["name"], "patch selection")
                assert selected[0]["version"] == package["version"]
                assert "source" not in selected[0] and "checksum" not in selected[0], (relative, package["name"], "registry package selected")
        assert not {p["name"] for p in lock["package"]} & {
            "rsa", "option-ext", "dirs", "dirs-sys", "derivative", "rrs-lib",
            "downcast-rs", "no_std_strings", "include_bytes_aligned",
        }
        assert {p["version"] for p in lock["package"] if p["name"] == "risc0-zkvm"} == {"3.0.6"}
        assert {p["version"] for p in lock["package"] if p["name"] == "tracing-subscriber"} == {"0.3.23"}
    print("SDK upstream/patched inventories, patch reconstruction, hashes, retained feature defaults and detached patch selections match")


def smoke(offline: bool) -> None:
    if not os.environ.get("CARGO_TARGET_DIR"):
        raise ValueError("Set CARGO_TARGET_DIR to an existing compatible cache for the smoke tests")
    with tempfile.TemporaryDirectory(prefix="sparq-sdk-smoke-") as temporary:
        directory = Path(temporary)
        (directory / "src").mkdir()
        shutil.copyfile(ROOT / "tests/smoke.rs", directory / "src/lib.rs")
        # Local publish cfg only activates the unchanged included sign method.
        # It never enables rzup publication or networking in this harness.
        manifest = f'''[package]
name = "sparq-sdk-patch-smoke"
version = "0.0.0"
edition = "2021"
publish = false
[workspace]
[features]
default = ["publish"]
publish = []
[dependencies]
rzup = {{ path = {json.dumps(str(ROOT / "rzup-0.5.2"))}, default-features = false }}
ark-relations = {{ path = {json.dumps(str(ROOT / "ark-relations-0.5.1"))}, features = ["std"] }}
ark-bn254 = "=0.5.0"
tracing = "=0.1.44"
tracing-subscriber = {{ version = "=0.3.23", default-features = false, features = ["registry"] }}
rsa = "=0.9.10"
rand = "=0.8.8"
sha2 = "=0.10.9"
hex = "=0.4.3"
[profile.dev]
debug = 0
'''
        (directory / "Cargo.toml").write_text(manifest)
        shutil.copyfile(REPO / "zk/sparql-evaluator/Cargo.lock", directory / "Cargo.lock")
        env = os.environ.copy()
        env.update(SPARQ_SDK_VENDOR_ROOT=str(ROOT), SPARQ_SDK_SMOKE_ROOT=temporary,
                   RISC0_HOME=str(directory / "risc0"), GITHUB_TOKEN="synthetic-sdk-smoke",
                   CARGO_INCREMENTAL="0")
        command = ["cargo", "test", "--manifest-path", str(directory / "Cargo.toml"), "--lib"]
        if offline:
            command.append("--offline")
        subprocess.run(command, env=env, check=True, timeout=600)
        # Also compile the additive key-taking API; installer/publication
        # integration remains outside this synthetic harness.
        env["RISC0_HOME"] = str(directory / "risc0-key-mode")
        subprocess.run(command + ["--features", "rzup/signatures"], env=env, check=True, timeout=600)


def feature_matrix(offline: bool) -> None:
    """Compile every derive-consuming family, including its constraint gadgets."""
    if not os.environ.get("CARGO_TARGET_DIR"):
        raise ValueError("Set CARGO_TARGET_DIR to an existing compatible cache for the feature matrix")
    with tempfile.TemporaryDirectory(prefix="sparq-sdk-features-") as temporary:
        directory = Path(temporary)
        (directory / "src").mkdir()
        (directory / "src/lib.rs").write_text("// [GPT-6] Compile the selected upstream feature surface.\n")
        (directory / "Cargo.toml").write_text(f'''[package]
name = "sparq-sdk-feature-matrix"
version = "0.0.0"
edition = "2021"
publish = false
[workspace]
[dependencies]
ark-crypto-primitives = {{ path = {json.dumps(str(ROOT / "ark-crypto-primitives-0.5.0"))}, default-features = false }}
[patch.crates-io]
ark-relations = {{ path = {json.dumps(str(ROOT / "ark-relations-0.5.1"))} }}
[profile.dev]
debug = 0
''')
        shutil.copyfile(REPO / "zk/sparql-evaluator/Cargo.lock", directory / "Cargo.lock")
        env = os.environ.copy()
        env["CARGO_INCREMENTAL"] = "0"
        cases = [[], ["std"], ["std", "snark", "sponge"]]
        consumers = ["crh", "commitment", "encryption", "merkle_tree", "signature"]
        cases += [[family] for family in consumers]
        # Upstream commitment+r1cs already needs prf and std to compile. Keep
        # that upstream feature limit separate from this derive-dependency patch.
        cases += [[family, "r1cs", "std", "prf"] for family in consumers]
        cases += [consumers + ["r1cs", "std", "snark", "sponge", "prf"]]
        for features in cases:
            options = ["--manifest-path", str(directory / "Cargo.toml")]
            if offline:
                options += ["--offline"]
            if features:
                options += ["--features", ",".join(f"ark-crypto-primitives/{f}" for f in features)]
            print("SDK feature compile:", ",".join(features) or "no features", flush=True)
            subprocess.run(["cargo", "check", "--lib"] + options, env=env, check=True, timeout=600)
            metadata = json.loads(subprocess.check_output(
                ["cargo", "metadata", "--format-version", "1", "--locked"] + options, env=env, timeout=600))
            active = {node["id"] for node in metadata["resolve"]["nodes"]}
            has_derivative = any(p["name"] == "derivative" and p["id"] in active
                                 for p in metadata["packages"])
            assert has_derivative == any(f in consumers for f in features), features


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--smoke", action="store_true")
    parser.add_argument("--feature-matrix", action="store_true")
    parser.add_argument("--offline", action="store_true", help="use only cached smoke-test dependencies")
    args = parser.parse_args()
    check()
    if args.smoke:
        smoke(args.offline)
    if args.feature_matrix:
        feature_matrix(args.offline)
