#!/usr/bin/env python3
"""Soundness spot check for the openings, after run.sh.

Changes one prover-chosen witness of circuits/target/CIRCUIT.gz at a time
(adds 1) and checks that UltraHonk proving or verification fails:

- hand: any witness the generator appended (digits, selectors, rotation and
  carry splits, message words, digest bytes);
- noir_var, noir_tuned: any output of an unconstrained (Brillig) hint,
  sampled per hint function, so the rotation (`split`) and carry
  (`carry_split`) hints of noir_tuned and the compiler's division and
  inverse hints are each exercised. An inverse hint whose input is zero is
  skipped: an is-zero check leaves it free by design.

XOR outputs are skipped: Barretenberg recomputes them from the inputs, so
changing one in the witness file changes nothing.

Usage: tamper.py CIRCUIT SAMPLES [SEED]   (SAMPLES per group; BB names the bb
binary, default `bb`)
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


def accepted(circuit, witness):
    path, out = os.path.join(T, "tamper.gz"), os.path.join(T, "tamper")
    with open(path, "wb") as f:
        f.write(gzip.compress(m.pack([[[0, witness]]])))
    os.makedirs(out, exist_ok=True)
    vk = os.path.join(T, f"proof-{circuit}", "vk")

    def run(*args):
        return subprocess.run([BB, *args], capture_output=True, check=False).returncode == 0

    return run("prove", "-s", "ultra_honk", "-b", os.path.join(T, f"{circuit}.json"), "-w", path,
               "-k", vk, "-o", out) and \
        run("verify", "-s", "ultra_honk", "-p", os.path.join(out, "proof"), "-k", vk,
            "-i", os.path.join(out, "public_inputs"))


def value(expr, witness):
    products, linear, const = expr
    v = int.from_bytes(const, "big")
    for q, a, b in products:
        v += int.from_bytes(q, "big") * int.from_bytes(witness[a], "big") * int.from_bytes(witness[b], "big")
    for q, a in linear:
        v += int.from_bytes(q, "big") * int.from_bytes(witness[a], "big")
    return v % m.P


def hint_outputs(program, witness):
    """Outputs of each unconstrained (Brillig) hint, grouped by function name.

    The inverse hint of an is-zero check is free when its input is zero (any
    value satisfies the check, and nothing else reads it), so those are left
    out."""
    names = [f[0] for f in program[1]]
    groups = {}
    for op in program[0][0][2]:
        if "BrilligCall" in op:
            fid, inputs, outputs = op["BrilligCall"][:3]
            if names[fid] == "directive_invert" and value(inputs[0]["Single"], witness) == 0:
                continue
            out = groups.setdefault(names[fid], set())
            for item in outputs:
                if "Simple" in item:
                    out.add(item["Simple"])
                else:
                    out.update(item["Array"])
    return groups


def main():
    circuit, samples = sys.argv[1], int(sys.argv[2])
    random.seed(int(sys.argv[3]) if len(sys.argv) > 3 else 1)
    with open(os.path.join(T, f"{circuit}.json"), encoding="utf-8") as f:
        artifact = json.load(f)
    program = m.unpack(gzip.decompress(base64.b64decode(artifact["bytecode"])))
    xor_out = {op["BlackBoxFuncCall"]["XOR"][3] for op in program[0][0][2]
               if "XOR" in op.get("BlackBoxFuncCall", {})}
    witness = load(os.path.join(T, f"{circuit}.gz"))[0][0][1]
    assert accepted(circuit, witness), "the untampered witness must prove and verify"
    if circuit == "hand":
        first = max(load(os.path.join(T, "spliced.gz"))[0][0][1]) + 1
        groups = {"appended": {k for k in witness if k >= first}}
    else:
        groups = hint_outputs(program, witness)
    report = {}
    failed = False
    # Up to SAMPLES changes per group, so every kind of hint is exercised.
    for name, chosen in sorted(groups.items()):
        candidates = sorted(chosen - xor_out)
        picked = random.sample(candidates, min(samples, len(candidates)))
        bad = []
        for k in picked:
            w = dict(witness)
            w[k] = m.fb(int.from_bytes(w[k], "big") + 1)
            if accepted(circuit, w):
                bad.append(k)
        report[name] = {"candidates": len(candidates), "tampered": len(picked), "accepted": bad}
        failed |= bool(bad)
    print(json.dumps({"circuit": circuit, "groups": report}))
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
