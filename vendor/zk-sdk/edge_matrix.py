#!/usr/bin/env python3
"""[GPT-6] Compile the SDK feature edges; this does not execute a guest or prove."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

import verify


def run(offline: bool) -> None:
    for flag in ["DOCS_RS", "RISC0_SKIP_BUILD_KERNELS"]:
        if flag in os.environ:
            raise ValueError(f"Feature compilation rejects the {flag} build stub setting")
    if not os.environ.get("CARGO_TARGET_DIR"):
        raise ValueError("Set CARGO_TARGET_DIR to an existing compatible cache")
    if offline:
        # Cargo's offline switch alone does not restrict dependency build scripts.
        archive = os.environ.get("RECURSION_SRC_PATH")
        if not archive or hashlib.sha256(Path(archive).read_bytes()).hexdigest() != (
            "744b999f0a35b3c86753311c7efb2a0054be21727095cf105af6ee7d3f4d8849"
        ):
            raise ValueError("Offline prover compilation requires the upstream-pinned RECURSION_SRC_PATH archive")
    with tempfile.TemporaryDirectory(prefix="sparq-sdk-edges-") as temporary:
        directory = Path(temporary)
        (directory / "src").mkdir()
        (directory / "src/lib.rs").write_text('''// [GPT-6] Exercise the unchanged embedded kernel library API.
#[test]
fn embedded_kernel_matches_recorded_source() {
    assert_eq!(risc0_zkos_v1compat::V1COMPAT_ELF,
        include_bytes!(concat!(env!("SPARQ_SDK_VENDOR_ROOT"),
            "/risc0-zkos-v1compat-2.2.3/elfs/v1compat.elf")));
}
''')
        patches = json.loads((verify.ROOT / "UPSTREAM.json").read_text())["packages"]
        manifest = '''[package]
name = "sparq-sdk-edge-matrix"
version = "0.0.0"
edition = "2024"
rust-version = "1.85"
publish = false
[workspace]
[features]
sdk-default = ["risc0-zkvm/default"]
sdk-client = ["risc0-zkvm/client"]
sdk-prove = ["risc0-zkvm/prove"]
kernel-default = ["risc0-zkos-v1compat/default"]
[dependencies]
risc0-zkvm = { version = "=3.0.6", default-features = false }
risc0-zkos-v1compat = { version = "=2.2.3", default-features = false }
[profile.dev]
debug = 0
[patch.crates-io]
'''
        for package in patches:
            name = package["name"]
            path = verify.ROOT / f"{name}-{package['version']}"
            manifest += f"{name} = {{ path = {json.dumps(str(path))} }}\n"
        (directory / "Cargo.toml").write_text(manifest)
        shutil.copyfile(verify.REPO / "zk/sparql-evaluator/Cargo.lock", directory / "Cargo.lock")
        env = os.environ.copy()
        env.update(CARGO_INCREMENTAL="0", SPARQ_SDK_VENDOR_ROOT=str(verify.ROOT))
        # Native stub binaries do not establish RISC-V kernel build coverage.
        cases = [[], ["sdk-client"], ["sdk-default"], ["kernel-default"], ["sdk-prove"]]
        for features in cases:
            options = ["--manifest-path", str(directory / "Cargo.toml")]
            if offline:
                options.append("--offline")
            if features:
                options += ["--features", ",".join(features)]
            print("SDK edge compile:", ",".join(features) or "no features", flush=True)
            subprocess.run(["cargo", "check", "--lib"] + options, env=env, check=True, timeout=1800)
            metadata = json.loads(subprocess.check_output(
                ["cargo", "metadata", "--format-version", "1", "--locked"] + options,
                env=env, timeout=600))
            active = {n["id"] for n in metadata["resolve"]["nodes"]}
            names = {p["name"] for p in metadata["packages"] if p["id"] in active}
            assert ("rrs-lib" in names) == ("sdk-prove" in features), features
            for name in ["no_std_strings", "include_bytes_aligned"]:
                assert (name in names) == ("kernel-default" in features), (features, name)
        # Run the public ELF slice comparison without the server/prover feature.
        subprocess.run(["cargo", "test", "--lib", "--manifest-path",
                        str(directory / "Cargo.toml")] + (["--offline"] if offline else []),
                       env=env, check=True, timeout=600)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--offline", action="store_true")
    args = parser.parse_args()
    verify.check()
    run(args.offline)
