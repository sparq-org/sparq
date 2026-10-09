#!/usr/bin/env python3
"""[GPT-6] Static Wasmer provenance checks; never invokes Cargo or the network."""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import tomllib

SUPPORT = Path(__file__).resolve().parent
NATIVE = SUPPORT.parent.parent


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def inventory(root):
    paths = list(root.rglob("*"))
    require(not root.is_symlink() and not any(p.is_symlink() for p in paths), "vendor symlink")
    return {p.relative_to(root).as_posix(): sha(p.read_bytes()) for p in paths if p.is_file()}


def codegen(source):
    return {
        "zero_padding_function": sha(source[source.index("fn zero_padding"):source.index("pub fn impl_value_type")].encode()),
        "unsafe_impl_quote_body": sha(source[source.index("        unsafe impl"):source.rindex("    }")].encode()),
    }


def reconstruct(package, support, destination):
    provenance = json.loads((support / "UPSTREAM.json").read_text())
    shutil.copytree(package, destination)
    subprocess.run(["git", "apply", "--unidiff-zero", "--reverse", "-p1", str((support / "diagnostics.patch").resolve())],
                   cwd=destination, check=True, capture_output=True, timeout=15)
    require(inventory(destination) == {n:r["upstream_sha256"] for n,r in provenance["files"].items()},
            "original reconstruction differs")


def check_metadata(metadata, native=NATIVE):
    """Caller supplies actual offline locked metadata; this function does not resolve."""
    selected = [p for p in metadata["packages"] if p["name"] == "wasmer-derive"]
    require(len(selected) == 1, "one Wasmer derive selection required")
    package = selected[0]
    require(package["version"] == "6.1.0" and package["source"] is None, "local version selection")
    require(Path(package["manifest_path"]).resolve() == (native / "vendor/wasmer-derive-6.1.0/Cargo.toml").resolve(),
            "wrong resolved vendor path")
    nodes = {n["id"] for n in metadata["resolve"]["nodes"]}
    require(package["id"] in nodes, "patch missing from active graph")
    require(not any(p["name"] in ("proc-macro-error2", "proc-macro-error-attr2") for p in metadata["packages"]),
            "removed diagnostic package still resolves")


def verify(native=NATIVE, policy=None):
    support = native / "vendor-support/wasmer"
    package = native / "vendor/wasmer-derive-6.1.0"
    record = json.loads((support / "UPSTREAM.json").read_text())
    require((record["package"], record["version"], record["package_path"], record["patch_path"]) ==
            ("wasmer-derive", "6.1.0", "vendor/wasmer-derive-6.1.0", "diagnostics.patch"), "package identity")
    require(record["archive_sha256"] == "c546f3380840cd63fdcc390f04cd19002f2dfa19b4691b77ecbd27642bd93452", "archive identity")
    require(inventory(package) == {n:r["patched_sha256"] for n,r in record["files"].items()}, "vendor inventory differs")
    require(sha((support / "diagnostics.patch").read_bytes()) == record["patch_sha256"], "patch hash differs")
    require(sha((support / "LICENSE.upstream").read_bytes()) == record["upstream_license"]["sha256"], "license differs")
    require(codegen((package / "src/value_type.rs").read_text()) == record["unchanged_codegen_sha256"], "code generation differs")
    with tempfile.TemporaryDirectory(prefix="sparq-wasmer-reconstruct-") as temp:
        original = Path(temp) / "package"
        reconstruct(package, support, original)
        require(codegen((original / "src/value_type.rs").read_text()) == record["unchanged_codegen_sha256"], "upstream code generation differs")
        subprocess.run(["git", "apply", "--unidiff-zero", "-p1", str((support / "diagnostics.patch").resolve())],
                       cwd=original, check=True, capture_output=True, timeout=15)
        require(inventory(original) == inventory(package), "forward reconstruction differs")
    manifest = tomllib.loads((native / "Cargo.toml").read_text())
    require(manifest["patch"]["crates-io"]["wasmer-derive"] == {"path":"vendor/wasmer-derive-6.1.0"}, "patch path differs")
    lock = tomllib.loads((native / "Cargo.lock").read_text())
    selected = [p for p in lock["package"] if p["name"] == "wasmer-derive"]
    require(len(selected) == 1 and selected[0]["version"] == "6.1.0" and "source" not in selected[0]
            and "checksum" not in selected[0], "lock does not select local derive")
    require(not any(p["name"] in ("proc-macro-error2", "proc-macro-error-attr2") for p in lock["package"]), "diagnostic lock edge remains")
    delta = json.loads((support / "lock-delta.json").read_text())
    require(sha((native / "Cargo.lock").read_bytes()) == delta["candidate_lock_sha256"], "unreviewed native lock change")
    policy = policy or native.parents[1] / "supply-chain/config.toml"
    config = tomllib.loads(policy.read_text())
    require(config["policy"]["wasmer-derive"]["audit-as-crates-io"] is True, "upstream vet obligation missing")
    return {"static_provenance":"pass", "archive_sha256":record["archive_sha256"],
            "compiled":False, "upstream_audit":False, "generated_layout_execution":"pending remote"}


if __name__ == "__main__":
    print(json.dumps(verify(), indent=2))
