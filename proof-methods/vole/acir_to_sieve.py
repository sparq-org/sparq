#!/usr/bin/env python3
"""Translate a compiled Noir ACIR program and its solved witness into a SIEVE
IR0+ (version 2.0.0) arithmetic circuit over the BN254 scalar field.

The output is a relation, a public-input file and a private-input file in the
text format that Diet Mac'n'Cheese (QuickSilver) reads. The relation is the
ACIR constraint system with every black box expanded into field arithmetic:

- AssertZero: the quadratic expression, one multiplication per product term.
- RANGE(x, n): n private bits, each constrained to {0, 1}, recomposed to x.
- Poseidon2Permutation (t = 4, BN254): the full permutation, 88 S-boxes of
  three multiplications each, with the Barretenberg round constants.
- MemoryInit / MemoryOp on `Memory` blocks: a read or write at a private index
  uses a private one-hot selector over the block, constrained to select
  exactly the index.

Brillig calls are unconstrained hints; their outputs are already in the solved
witness and are constrained by the opcodes that use them. Any other black box
(BLAKE3, SHA-256, AND, XOR, curve operations) or a call to another ACIR
function is rejected.

Every constraint is evaluated on the witness while it is written, so a
translation that succeeds is satisfied by the witness it was given.

Usage: acir_to_sieve.py ARTIFACT.json WITNESS.gz OUTDIR
Writes OUTDIR/relation.txt, OUTDIR/public.txt, OUTDIR/private.txt and prints
a JSON summary.
"""

import base64
import gzip
import json
import os
import re
import sys

P = 21888242871839275222246405745257275088548364400416034343698204186575808495617

HERE = os.path.dirname(os.path.abspath(__file__))
CONSTANTS = os.path.join(HERE, "..", "..", "crates", "sparq-zk", "src", "poseidon2_constants.rs")


class Unsupported(Exception):
    pass


def load_poseidon2_constants(path):
    text = open(path, encoding="utf-8").read()
    diag = text[text.index("INTERNAL_MATRIX_DIAGONAL_HEX"):text.index("ROUND_CONSTANT_HEX")]
    rounds = text[text.index("ROUND_CONSTANT_HEX"):]
    diag = [int(h, 16) for h in re.findall(r'"([0-9a-f]{64})"', diag)]
    flat = [int(h, 16) for h in re.findall(r'"([0-9a-f]{64})"', rounds)]
    assert len(diag) == 4 and len(flat) == 256, (len(diag), len(flat))
    return diag, [flat[4 * r:4 * r + 4] for r in range(64)]


def unpack(data):
    if data[0] != 3:
        raise Unsupported(f"serialization format {data[0]}")
    import msgpack  # only needed to read compiled artifacts

    return msgpack.unpackb(data[1:], strict_map_key=False, raw=False)


def fe(b):
    return int.from_bytes(b, "big") % P


class Circuit:
    """Writes IR0+ gates and evaluates each one on the witness."""

    def __init__(self):
        self.lines = []
        self.public = []
        self.private = []
        self.values = []
        self.muls = 0

    def _new(self, value, line):
        wire = len(self.values)
        self.values.append(value % P)
        self.lines.append(f"${wire} <- {line};")
        return wire

    def priv(self, value):
        self.private.append(value % P)
        return self._new(value, "@private()")

    def pub(self, value):
        self.public.append(value % P)
        return self._new(value, "@public()")

    def const(self, c):
        return self._new(c, f"<{c % P}>")

    def add(self, a, b):
        return self._new(self.values[a] + self.values[b], f"@add(${a}, ${b})")

    def mul(self, a, b):
        self.muls += 1
        return self._new(self.values[a] * self.values[b], f"@mul(${a}, ${b})")

    def addc(self, a, c):
        c %= P
        return a if c == 0 else self._new(self.values[a] + c, f"@addc(${a}, <{c}>)")

    def mulc(self, a, c):
        c %= P
        return a if c == 1 else self._new(self.values[a] * c, f"@mulc(${a}, <{c}>)")

    def assert_zero(self, a, why):
        if self.values[a] != 0:
            raise ValueError(f"witness does not satisfy {why}")
        self.lines.append(f"@assert_zero(${a});")

    def assert_eq(self, a, b, why):
        self.assert_zero(self.add(a, self.mulc(b, P - 1)), why)

    def lin(self, terms, constant=0):
        """sum(c * wire) + constant, with no multiplication gates."""
        acc = None
        for c, w in terms:
            t = self.mulc(w, c)
            acc = t if acc is None else self.add(acc, t)
        if acc is None:
            return self.const(constant)
        return self.addc(acc, constant)


class Translator:
    def __init__(self, program, witness, diag, rc):
        functions = program[0]
        if len(functions) != 1:
            raise Unsupported("calls to other ACIR functions")
        (self.name, _current_witness, self.opcodes, private_params,
         public_params, return_values, _assert_messages) = functions[0]
        (stack,) = witness
        if len(stack) != 1 or stack[0][0] != 0:
            raise Unsupported("witness stack with more than the main function")
        self.witness = {int(k): fe(v) for k, v in stack[0][1].items()}
        # ACIR's public inputs are the set union of the public parameters and
        # the return values, in witness-index order (`Circuit::public_inputs`);
        # a witness in both is one public input.
        self.public_params = sorted(set(public_params) | set(return_values))
        self.c = Circuit()
        self.wires = {}
        self.blocks = {}
        self.diag, self.rc = diag, rc
        self.range_bits = 0
        self.sboxes = 0
        self.selectors = 0
        for w in self.public_params:
            self.wires[w] = self.c.pub(self.witness[w])

    def w(self, index):
        if index not in self.wires:
            self.wires[index] = self.c.priv(self.witness.get(index, 0))
        return self.wires[index]

    def bind(self, index, wire, why):
        if index in self.wires:
            self.c.assert_eq(self.wires[index], wire, why)
        else:
            if self.c.values[wire] != self.witness.get(index, self.c.values[wire]):
                raise ValueError(f"witness disagrees with {why}")
            self.wires[index] = wire

    def expr(self, e):
        products, linear, constant = e
        terms = []
        for q, a, b in products:
            terms.append((fe(q), self.c.mul(self.w(a), self.w(b))))
        for q, a in linear:
            terms.append((fe(q), self.w(a)))
        return self.c.lin(terms, fe(constant))

    def expr_value(self, e):
        products, linear, constant = e
        v = fe(constant)
        for q, a, b in products:
            v += fe(q) * self.witness[a] * self.witness[b]
        for q, a in linear:
            v += fe(q) * self.witness[a]
        return v % P

    def input_wire(self, item):
        if "Witness" in item:
            return self.w(item["Witness"])
        return self.c.const(fe(item["Constant"]))

    def range(self, item, bits):
        if bits >= 254:
            return
        x = self.input_wire(item)
        value = self.c.values[x]
        if value >> bits:
            raise ValueError("witness does not satisfy RANGE")
        terms = []
        for i in range(bits):
            b = self.c.priv((value >> i) & 1)
            self.c.assert_zero(self.c.add(self.c.mul(b, b), self.c.mulc(b, P - 1)), "bit")
            terms.append((1 << i, b))
        self.c.assert_eq(self.c.lin(terms), x, "RANGE recomposition")
        self.range_bits += bits

    def sbox(self, x):
        self.sboxes += 1
        x2 = self.c.mul(x, x)
        x4 = self.c.mul(x2, x2)
        return self.c.mul(x4, x)

    def external(self, s):
        c = self.c
        t0 = c.add(s[0], s[1])
        t1 = c.add(s[2], s[3])
        t2 = c.add(c.mulc(s[1], 2), t1)
        t3 = c.add(c.mulc(s[3], 2), t0)
        t4 = c.add(c.mulc(t1, 4), t3)
        t5 = c.add(c.mulc(t0, 4), t2)
        t6 = c.add(t3, t5)
        t7 = c.add(t2, t4)
        return [t6, t5, t7, t4]

    def internal(self, s):
        c = self.c
        total = c.add(c.add(s[0], s[1]), c.add(s[2], s[3]))
        return [c.add(c.mulc(s[i], self.diag[i]), total) for i in range(4)]

    def poseidon2(self, inputs, outputs):
        c = self.c
        s = self.external([self.input_wire(i) for i in inputs])
        for r in range(4):
            s = self.external([self.sbox(c.addc(s[i], self.rc[r][i])) for i in range(4)])
        for r in range(4, 60):
            s = self.internal([self.sbox(c.addc(s[0], self.rc[r][0]))] + s[1:])
        for r in range(60, 64):
            s = self.external([self.sbox(c.addc(s[i], self.rc[r][i])) for i in range(4)])
        for index, wire in zip(outputs, s):
            self.bind(index, wire, "Poseidon2 output")

    def select(self, block, index_value):
        """Private one-hot selector over `block`, constrained to `index_value`'s wire."""
        c = self.c
        n = len(block)
        if index_value >= n:
            raise ValueError("memory index out of bounds")
        sel = []
        for j in range(n):
            e = c.priv(1 if j == index_value else 0)
            c.assert_zero(c.add(c.mul(e, e), c.mulc(e, P - 1)), "selector bit")
            sel.append(e)
        self.selectors += n
        c.assert_zero(c.lin([(1, e) for e in sel], P - 1), "selector sum")
        return sel

    def memory_op(self, block_id, op):
        c = self.c
        operation, index, value = op
        block = self.blocks[block_id]
        is_write = self.expr_value(operation)
        idx_value = self.expr_value(index)
        idx_products, idx_linear, _ = index
        constant_index = not idx_products and not idx_linear
        if constant_index:
            sel = None
        else:
            sel = self.select(block, idx_value)
            c.assert_eq(c.lin([(j, e) for j, e in enumerate(sel)]), self.expr(index), "selector index")
        v = self.expr(value)
        if is_write == 0:
            if sel is None:
                c.assert_eq(block[idx_value], v, "memory read")
            else:
                acc = None
                for e, cell in zip(sel, block):
                    t = c.mul(e, cell)
                    acc = t if acc is None else c.add(acc, t)
                c.assert_eq(acc, v, "memory read")
        else:
            if sel is None:
                block[idx_value] = v
            else:
                for j, (e, cell) in enumerate(zip(sel, block)):
                    delta = c.mul(e, c.add(v, c.mulc(cell, P - 1)))
                    block[j] = c.add(cell, delta)

    def run(self):
        for op in self.opcodes:
            (kind, body), = op.items()
            if kind == "AssertZero":
                self.c.assert_zero(self.expr(body), "AssertZero")
            elif kind == "BrilligCall":
                continue
            elif kind == "MemoryInit":
                block_id, init, block_type = body
                if block_type != "Memory":
                    raise Unsupported(f"memory block type {block_type}")
                self.blocks[block_id] = [self.w(i) for i in init]
            elif kind == "MemoryOp":
                block_id, mem = body[0], body[1]
                self.memory_op(block_id, mem)
            elif kind == "BlackBoxFuncCall":
                (bb, args), = body.items()
                if bb == "RANGE":
                    self.range(args[0], args[1])
                elif bb == "Poseidon2Permutation":
                    self.poseidon2(args[0], args[1])
                else:
                    raise Unsupported(f"black box {bb}")
            else:
                raise Unsupported(f"opcode {kind}")


def write(path, kind, body):
    with open(path, "w", encoding="utf-8") as f:
        f.write(f"version 2.0.0;\n{kind};\n@type field {P};\n@begin\n")
        f.write(body)
        f.write("@end\n")


def main():
    artifact, witness_path, outdir = sys.argv[1:4]
    a = json.load(open(artifact, encoding="utf-8"))
    program = unpack(gzip.decompress(base64.b64decode(a["bytecode"])))
    witness = unpack(gzip.decompress(open(witness_path, "rb").read()))
    diag, rc = load_poseidon2_constants(CONSTANTS)
    t = Translator(program, witness, diag, rc)
    t.run()
    os.makedirs(outdir, exist_ok=True)
    c = t.c
    write(os.path.join(outdir, "relation.txt"), "circuit", "".join(line + "\n" for line in c.lines))
    write(os.path.join(outdir, "public.txt"), "public_input", "".join(f"<{v}>;\n" for v in c.public))
    write(os.path.join(outdir, "private.txt"), "private_input", "".join(f"<{v}>;\n" for v in c.private))
    print(json.dumps({
        "artifact": os.path.basename(artifact),
        "acir_opcodes": len(t.opcodes),
        "wires": len(c.values),
        "multiplications": c.muls,
        "public_inputs": len(c.public),
        "private_inputs": len(c.private),
        "range_bits": t.range_bits,
        "poseidon2_sboxes": t.sboxes,
        "memory_selector_cells": t.selectors,
    }))


if __name__ == "__main__":
    main()
