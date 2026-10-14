#!/usr/bin/env python3
"""Write a SIEVE IR0+ (version 2.0.0) Boolean circuit over GF(2) that checks N
independent BLAKE3 compressions, for QuickSilver over GF(2).

Each compression has a private 256-bit chaining value and a private 512-bit
message block; its 256-bit output chaining value is public. The counter is 0,
the block length 64 and the flags 0, all fixed in the relation. XOR is
@add, AND is @mul and rotations are rewiring. A 32-bit addition is a ripple
carry with one AND per bit, c' = c ^ ((a ^ c) & (b ^ c)), and no carry out of
the top bit: 31 ANDs. One compression has 7 rounds of 8 G functions of 6
additions: at most 10,416 ANDs, a few fewer where an operand is a fixed
constant (the zero counter) and the gate folds.

Every gate is evaluated on the witness while it is written, so a relation that
is written is satisfied by its witness.

Usage: blake3_f2.py N OUTDIR [--seed S]   writes OUTDIR/relation.txt,
       OUTDIR/public.txt, OUTDIR/private.txt and prints a JSON summary.
       blake3_f2.py --selftest              checks the reference compression.
"""

import json
import os
import random
import sys

IV = [0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A, 0x510E527F, 0x9B05688C, 0x1F83D9AB, 0x5BE0CD19]
PERMUTATION = [2, 6, 3, 10, 7, 0, 4, 13, 1, 11, 12, 5, 9, 14, 15, 8]
G_ARGS = [(0, 4, 8, 12), (1, 5, 9, 13), (2, 6, 10, 14), (3, 7, 11, 15),
          (0, 5, 10, 15), (1, 6, 11, 12), (2, 7, 8, 13), (3, 4, 9, 14)]
BLOCK_LEN, FLAGS = 64, 0
M32 = 0xFFFFFFFF


def compress(cv, block, counter, block_len, flags):
    """Reference BLAKE3 compression; returns the 8-word output chaining value."""
    def rotr(x, r):
        return ((x >> r) | (x << (32 - r))) & M32

    s = list(cv) + IV[:4] + [counter & M32, counter >> 32, block_len, flags]
    m = list(block)
    for _ in range(7):
        for i, (a, b, c, d) in enumerate(G_ARGS):
            s[a] = (s[a] + s[b] + m[2 * i]) & M32
            s[d] = rotr(s[d] ^ s[a], 16)
            s[c] = (s[c] + s[d]) & M32
            s[b] = rotr(s[b] ^ s[c], 12)
            s[a] = (s[a] + s[b] + m[2 * i + 1]) & M32
            s[d] = rotr(s[d] ^ s[a], 8)
            s[c] = (s[c] + s[d]) & M32
            s[b] = rotr(s[b] ^ s[c], 7)
        m = [m[p] for p in PERMUTATION]
    return [s[i] ^ s[i + 8] for i in range(8)]


class Circuit:
    """Writes GF(2) gates and evaluates each one on the witness.

    A bit is a wire number or a Python bool constant; gates on constants are
    folded, so constants never reach the relation."""

    def __init__(self):
        self.lines = []
        self.public = []
        self.private = []
        self.values = []
        self.ands = 0
        self.xors = 0

    def _new(self, value, line):
        wire = len(self.values)
        self.values.append(value)
        self.lines.append(f"${wire} <- {line};")
        return wire

    def priv(self, value):
        self.private.append(value)
        return self._new(value, "@private()")

    def pub(self, value):
        self.public.append(value)
        return self._new(value, "@public()")

    def value(self, x):
        return int(x) if isinstance(x, bool) else self.values[x]

    def xor(self, a, b):
        if isinstance(a, bool) and isinstance(b, bool):
            return a != b
        if isinstance(a, bool):
            a, b = b, a
        if isinstance(b, bool):
            return self._new(self.values[a] ^ 1, f"@addc(${a}, <1>)") if b else a
        self.xors += 1
        return self._new(self.values[a] ^ self.values[b], f"@add(${a}, ${b})")

    def and_(self, a, b):
        if isinstance(a, bool):
            a, b = b, a
        if isinstance(b, bool):
            return a if b else False
        self.ands += 1
        return self._new(self.values[a] & self.values[b], f"@mul(${a}, ${b})")

    def assert_zero(self, a, why):
        if self.value(a) != 0:
            raise ValueError(f"witness does not satisfy {why}")
        if isinstance(a, bool):
            return
        self.lines.append(f"@assert_zero(${a});")

    # Words are lists of 32 bits, least significant first.
    def const_word(self, x):
        return [bool((x >> i) & 1) for i in range(32)]

    def xor_word(self, a, b):
        return [self.xor(x, y) for x, y in zip(a, b)]

    def add_word(self, a, b):
        out, c = [], False
        for i in range(32):
            out.append(self.xor(self.xor(a[i], b[i]), c))
            if i < 31:
                c = self.xor(c, self.and_(self.xor(a[i], c), self.xor(b[i], c)))
        return out

    def word_value(self, w):
        return sum(self.value(b) << i for i, b in enumerate(w))


def rotr(w, r):
    return w[r:] + w[:r]


def compress_circuit(c, cv, block):
    s = list(cv) + [c.const_word(x) for x in IV[:4]] + \
        [c.const_word(x) for x in (0, 0, BLOCK_LEN, FLAGS)]
    m = list(block)
    for _ in range(7):
        for i, (a, b, cc, d) in enumerate(G_ARGS):
            s[a] = c.add_word(c.add_word(s[a], s[b]), m[2 * i])
            s[d] = rotr(c.xor_word(s[d], s[a]), 16)
            s[cc] = c.add_word(s[cc], s[d])
            s[b] = rotr(c.xor_word(s[b], s[cc]), 12)
            s[a] = c.add_word(c.add_word(s[a], s[b]), m[2 * i + 1])
            s[d] = rotr(c.xor_word(s[d], s[a]), 8)
            s[cc] = c.add_word(s[cc], s[d])
            s[b] = rotr(c.xor_word(s[b], s[cc]), 7)
        m = [m[p] for p in PERMUTATION]
    return [c.xor_word(s[i], s[i + 8]) for i in range(8)]


def build(n, seed):
    rng = random.Random(seed)
    c = Circuit()
    for k in range(n):
        cv_words = [rng.getrandbits(32) for _ in range(8)]
        block_words = [rng.getrandbits(32) for _ in range(16)]
        expected = compress(cv_words, block_words, 0, BLOCK_LEN, FLAGS)
        cv = [[c.priv((x >> i) & 1) for i in range(32)] for x in cv_words]
        block = [[c.priv((x >> i) & 1) for i in range(32)] for x in block_words]
        out = compress_circuit(c, cv, block)
        for j, word in enumerate(out):
            if c.word_value(word) != expected[j]:
                raise AssertionError("circuit disagrees with the reference compression")
            for i, bit in enumerate(word):
                c.assert_zero(c.xor(bit, c.pub((expected[j] >> i) & 1)), f"output bit {k}.{j}.{i}")
    return c


def write(path, kind, body):
    with open(path, "w", encoding="utf-8") as f:
        f.write(f"version 2.0.0;\n{kind};\n@type field 2;\n@begin\n")
        f.write(body)
        f.write("@end\n")


def selftest():
    # BLAKE3("") is one compression of an all-zero block with flags CHUNK_START | CHUNK_END | ROOT.
    out = compress(IV, [0] * 16, 0, 0, 11)
    digest = b"".join(w.to_bytes(4, "little") for w in out).hex()
    assert digest == "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262", digest
    print("ok")


def main():
    if sys.argv[1:] == ["--selftest"]:
        return selftest()
    n, outdir = int(sys.argv[1]), sys.argv[2]
    seed = int(sys.argv[sys.argv.index("--seed") + 1]) if "--seed" in sys.argv else 1
    c = build(n, seed)
    os.makedirs(outdir, exist_ok=True)
    relation = os.path.join(outdir, "relation.txt")
    write(relation, "circuit", "".join(line + "\n" for line in c.lines))
    write(os.path.join(outdir, "public.txt"), "public_input", "".join(f"<{v}>;\n" for v in c.public))
    write(os.path.join(outdir, "private.txt"), "private_input", "".join(f"<{v}>;\n" for v in c.private))
    print(json.dumps({"n": n, "and_gates": c.ands, "xor_gates": c.xors, "private_bits": len(c.private),
                      "public_bits": len(c.public), "relation_bytes": os.path.getsize(relation)}))


if __name__ == "__main__":
    main()
