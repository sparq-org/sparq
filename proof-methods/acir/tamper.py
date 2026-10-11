#!/usr/bin/env python3
"""Soundness spot check for the hand-written opening, after run.sh.

Changes one witness of circuits/target/hand.gz at a time (adds 1) and checks
that UltraHonk proving or verification fails. XOR outputs are skipped:
Barretenberg recomputes them from the inputs, so changing one in the witness
file changes nothing.

Usage: tamper.py SAMPLES [SEED]   (BB names the bb binary, default `bb`)
"""

import base64
import gzip
import json
import os
import random
import subprocess
import sys

import blake3_leaf as m

BB = os.environ.get("BB", "bb")
T = os.path.join(os.path.dirname(os.path.abspath(__file__)), "circuits", "target")


def load(path):
    with open(path, "rb") as f:
        return m.unpack(gzip.decompress(f.read()))


def accepted(witness):
    path, out = os.path.join(T, "tamper.gz"), os.path.join(T, "tamper")
    with open(path, "wb") as f:
        f.write(gzip.compress(m.pack([[[0, witness]]])))
    os.makedirs(out, exist_ok=True)
    vk = os.path.join(T, "proof-hand", "vk")

    def run(*args):
        return subprocess.run([BB, *args], capture_output=True, check=False).returncode == 0

    return run("prove", "-s", "ultra_honk", "-b", os.path.join(T, "hand.json"), "-w", path,
               "-k", vk, "-o", out) and \
        run("verify", "-s", "ultra_honk", "-p", os.path.join(out, "proof"), "-k", vk,
            "-i", os.path.join(out, "public_inputs"))


def main():
    samples = int(sys.argv[1])
    random.seed(int(sys.argv[2]) if len(sys.argv) > 2 else 1)
    with open(os.path.join(T, "hand.json"), encoding="utf-8") as f:
        artifact = json.load(f)
    opcodes = m.unpack(gzip.decompress(base64.b64decode(artifact["bytecode"])))[0][0][2]
    xor_out = {op["BlackBoxFuncCall"]["XOR"][3] for op in opcodes
               if "XOR" in op.get("BlackBoxFuncCall", {})}
    first = max(load(os.path.join(T, "spliced.gz"))[0][0][1]) + 1
    witness = load(os.path.join(T, "hand.gz"))[0][0][1]
    assert accepted(witness), "the untampered witness must prove and verify"
    added = sorted(k for k in witness if k >= first and k not in xor_out)
    bad = []
    for k in random.sample(added, samples):
        w = dict(witness)
        w[k] = m.fb(int.from_bytes(w[k], "big") + 1)
        if accepted(w):
            bad.append(k)
    print(json.dumps({"tampered": samples, "accepted": bad}))
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
