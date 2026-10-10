#!/usr/bin/env python3
"""Generates Nargo packages that check K independent copies of a zk/compose
relation, with a Prover.toml that repeats one honest witness K times, so the
same statement can be proved at several sizes by UltraHonk and QuickSilver.

Usage: gen.py K [K ...]   (run from this directory)
"""
import os, re, sys

HERE = os.path.dirname(os.path.abspath(__file__))
SRC = os.path.join(HERE, "..", "..", "..", "zk", "compose", "hidden_issuer_d4", "Prover_hi_ok.toml")

MAIN = """// K independent copies of the hidden-issuer relation (zk/compose/hidden_issuer_d4).
use sparq_zk_compose_core::issuer::{{hidden_issuer_attestation, Point}};

global K: u32 = {k};
global D: u32 = 4;

fn main(
    challenge: pub Field,
    m: pub [Field; K],
    key_set_root: pub Field,
    pk_x: [Field; K],
    pk_y: [Field; K],
    r_x: [Field; K],
    r_y: [Field; K],
    s: [Field; K],
    e: [Field; K],
    e_k: [Field; K],
    index: [Field; K],
    siblings: [[Field; D]; K],
) {{
    let _ = challenge;
    for i in 0..K {{
        let pk = Point {{ x: pk_x[i], y: pk_y[i] }};
        let r = Point {{ x: r_x[i], y: r_y[i] }};
        hidden_issuer_attestation::<D>(m[i], key_set_root, pk, r, s[i], e[i], e_k[i], index[i], siblings[i]);
    }}
}}
"""

def main():
    fields = dict(re.findall(r'^(\w+) = (.*)$', open(SRC).read(), re.M))
    for k in map(int, sys.argv[1:]):
        name = f"hidden_issuer_x{k}"
        os.makedirs(os.path.join(HERE, name, "src"), exist_ok=True)
        with open(os.path.join(HERE, name, "Nargo.toml"), "w") as f:
            f.write(f'[package]\nname = "{name}"\ntype = "bin"\n\n[dependencies]\n'
                    'sparq_zk_compose_core = { path = "../../../../zk/compose/compose_core" }\n')
        with open(os.path.join(HERE, name, "src", "main.nr"), "w") as f:
            f.write(MAIN.format(k=k))
        with open(os.path.join(HERE, name, "Prover.toml"), "w") as f:
            for key, value in fields.items():
                if key in ("challenge", "key_set_root"):
                    f.write(f"{key} = {value}\n")
                else:
                    f.write(f"{key} = [{', '.join([value] * k)}]\n")

if __name__ == "__main__":
    main()
