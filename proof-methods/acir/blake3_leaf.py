#!/usr/bin/env python3
"""Hand-written ACIR for opening a committed xsd:integer literal.

`full_u64_check` (zk/compose/compose_core/src/result_integer.nr) proves that a
private u64 `value` is the literal whose term encoding is `operand`:

    operand = h2(2, hs),  hs = big-endian(blake3(token)[1..32]),
    token   = '"' + decimal(value) + '"^^<http://www.w3.org/2001/XMLSchema#integer>'

The token is 47 to 66 bytes long, depending on the private digit count D.
ACIR's BLAKE3 black box takes a fixed-length input, so the Noir function
hashes all 20 candidate tokens. This generator constrains the same relation
with one variable-length BLAKE3 instead: one-hot digit-count selectors, the
token's 17 message words as selector-weighted sums, and two BLAKE3
compressions written out of XOR and RANGE black boxes and 32-bit additions.
A token of at most 64 bytes is one block; 65 or 66 bytes is two. Both
compressions are always constrained and the digest is selected from the first
or the second, so the circuit does not depend on D.

The opcodes are appended to a compiled Noir program (`spliced/`) whose main
takes `values` and `hs` as private inputs and checks `h2(2, hs) == operand`
and the comparison verdict. The generator solves its own witnesses and
evaluates every constraint as it writes it, so a program it writes is
satisfied by the witness it writes.

Usage:
  blake3_leaf.py splice  NOIR_ARTIFACT NOIR_WITNESS OUT_ARTIFACT OUT_WITNESS
  blake3_leaf.py hs      VALUE            # prints hs as a decimal field element
"""

import base64
import gzip
import json
import sys
from collections import Counter

P = 21888242871839275222246405745257275088548364400416034343698204186575808495617
M32 = (1 << 32) - 1

IV = [0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A,
      0x510E527F, 0x9B05688C, 0x1F83D9AB, 0x5BE0CD19]
PERMUTATION = [2, 6, 3, 10, 7, 0, 4, 13, 1, 11, 12, 5, 9, 14, 15, 8]
CHUNK_START, CHUNK_END, ROOT = 1, 2, 8
SUFFIX = b'"^^<http://www.w3.org/2001/XMLSchema#integer>'
COLUMNS = [(0, 4, 8, 12), (1, 5, 9, 13), (2, 6, 10, 14), (3, 7, 11, 15),
           (0, 5, 10, 15), (1, 6, 11, 12), (2, 7, 8, 13), (3, 4, 9, 14)]
ROTATIONS = (16, 12, 8, 7)


# Reference BLAKE3 for tokens of at most two blocks (one chunk).

def compress(cv, words, block_len, flags):
    s = list(cv) + IV[:4] + [0, 0, block_len, flags]
    m = list(words)

    def g(a, b, c, d, x, y):
        s[a] = (s[a] + s[b] + x) & M32
        s[d] = rotr(s[d] ^ s[a], 16)
        s[c] = (s[c] + s[d]) & M32
        s[b] = rotr(s[b] ^ s[c], 12)
        s[a] = (s[a] + s[b] + y) & M32
        s[d] = rotr(s[d] ^ s[a], 8)
        s[c] = (s[c] + s[d]) & M32
        s[b] = rotr(s[b] ^ s[c], 7)

    for r in range(7):
        for i, (a, b, c, d) in enumerate(COLUMNS):
            g(a, b, c, d, m[2 * i], m[2 * i + 1])
        m = [m[PERMUTATION[i]] for i in range(16)]
    return [s[i] ^ s[i + 8] for i in range(8)]


def rotr(x, r):
    return ((x >> r) | (x << (32 - r))) & M32


def words_of(block):
    block = block.ljust(64, b"\0")
    return [int.from_bytes(block[4 * i:4 * i + 4], "little") for i in range(16)]


def blake3(data):
    assert 0 < len(data) <= 128
    if len(data) <= 64:
        out = compress(IV, words_of(data), len(data), CHUNK_START | CHUNK_END | ROOT)
    else:
        cv = compress(IV, words_of(data[:64]), 64, CHUNK_START)
        out = compress(cv, words_of(data[64:]), len(data) - 64, CHUNK_END | ROOT)
    return b"".join(w.to_bytes(4, "little") for w in out)


def token(value):
    return b'"' + str(value).encode() + SUFFIX


def hs_of(value):
    return int.from_bytes(blake3(token(value))[1:], "big")


# ACIR construction.

def fb(x):
    return (x % P).to_bytes(32, "big")


class Lin:
    """A linear combination of witnesses plus a constant."""

    def __init__(self, terms=None, const=0):
        self.terms = {k: v % P for k, v in (terms or {}).items() if v % P}
        self.const = const % P

    def __add__(self, other):
        if isinstance(other, int):
            return Lin(self.terms, self.const + other)
        terms = dict(self.terms)
        for k, v in other.terms.items():
            terms[k] = terms.get(k, 0) + v
        return Lin(terms, self.const + other.const)

    def __sub__(self, other):
        return self + other * -1

    def __mul__(self, c):
        return Lin({k: v * c for k, v in self.terms.items()}, self.const * c)

    def is_const(self):
        return not self.terms


def W(index):
    return Lin({index: 1})


def C(value):
    return Lin(const=value)


class Builder:
    def __init__(self, witness, first_free):
        self.w = dict(witness)
        self.next = first_free
        self.ops = []
        self.stats = Counter()

    def val(self, e):
        return (sum(c * self.w[k] for k, c in e.terms.items()) + e.const) % P

    def new(self, value):
        index = self.next
        self.next += 1
        self.w[index] = value % P
        return index

    def assert_zero(self, lin, products=()):
        value = self.val(lin) + sum(q * self.w[a] * self.w[b] for q, a, b in products)
        if value % P:
            raise ValueError("witness does not satisfy an AssertZero")
        self.ops.append({"AssertZero": [
            [[fb(q), a, b] for q, a, b in products],
            [[fb(c), k] for k, c in sorted(lin.terms.items())],
            fb(lin.const),
        ]})
        self.stats["assert_zero"] += 1

    def witness(self, e):
        """The witness index of `e`, adding one if `e` is not a bare witness."""
        if len(e.terms) == 1 and e.const == 0:
            (k, c), = e.terms.items()
            if c == 1:
                return k
        index = self.new(self.val(e))
        self.assert_zero(e - W(index))
        return index

    def input(self, e):
        if e.is_const():
            return {"Constant": fb(e.const)}
        return {"Witness": self.witness(e)}

    def range(self, e, bits):
        if self.val(e) >> bits:
            raise ValueError("witness does not satisfy RANGE")
        self.ops.append({"BlackBoxFuncCall": {"RANGE": [self.input(e), bits]}})
        self.stats["range"] += 1

    def mul(self, a, b):
        index = self.new(self.w[a] * self.w[b])
        self.assert_zero(W(index) * -1, [(1, a, b)])
        return index

    def boolean(self, a):
        self.assert_zero(W(a) * -1, [(1, a, a)])

    def xor(self, a, b):
        if a.is_const() and b.is_const():
            return C(a.const ^ b.const)
        if a.is_const() and a.const == 0:
            return b
        if b.is_const() and b.const == 0:
            return a
        out = self.new(self.val(a) ^ self.val(b))
        self.ops.append({"BlackBoxFuncCall": {"XOR": [self.input(a), self.input(b), 32, out]}})
        self.stats["xor"] += 1
        return W(out)

    def add32(self, *terms):
        """(sum of terms) mod 2^32. Every result feeds an XOR or a byte
        decomposition, either of which constrains it to 32 bits."""
        s = terms[0]
        for t in terms[1:]:
            s = s + t
        if s.is_const():
            return C(s.const & M32)
        v = self.val(s)
        out = self.new(v & M32)
        carry = self.new(v >> 32)
        self.assert_zero(s - W(out) - W(carry) * (1 << 32))
        self.range(W(carry), 2 if len(terms) > 2 else 1)
        return W(out)

    def rotr(self, x, r, feeds_xor=True):
        """x >>> r for a 32-bit x (an XOR output).

        With hi = x >> r and lo = x mod 2^r, the rotation is
        rot = lo * 2^(32 - r) + hi, so x = rot * 2^r - lo * (2^32 - 1).
        Constraining that, lo < 2^r and rot < 2^32 fixes rot: hi is then
        rot - lo * 2^(32 - r), which lies in (-2^32, 2^32), and a negative hi
        would make x a field element above 2^32. When rot feeds an XOR, the
        XOR's 32-bit decomposition constrains rot < 2^32; otherwise this adds
        the range check."""
        if x.is_const():
            return C(rotr(x.const, r))
        v = self.val(x)
        lo = self.new(v & ((1 << r) - 1))
        rot = self.new(rotr(v, r))
        self.assert_zero(x - W(rot) * (1 << r) + W(lo) * ((1 << 32) - 1))
        self.range(W(lo), r)
        if not feeds_xor:
            self.range(W(rot), 32)
        return W(rot)

    def compress(self, cv, words, block_len, flags):
        s = list(cv) + [C(v) for v in IV[:4]] + [C(0), C(0), block_len, flags]
        m = list(words)

        def g(a, b, c, d, x, y, last):
            s[a] = self.add32(s[a], s[b], x)
            s[d] = self.rotr(self.xor(s[d], s[a]), 16)
            s[c] = self.add32(s[c], s[d])
            s[b] = self.rotr(self.xor(s[b], s[c]), 12)
            s[a] = self.add32(s[a], s[b], y)
            # In the last round, d feeds only the addition below.
            s[d] = self.rotr(self.xor(s[d], s[a]), 8, feeds_xor=not last)
            s[c] = self.add32(s[c], s[d])
            s[b] = self.rotr(self.xor(s[b], s[c]), 7)

        for r in range(7):
            for i, (a, b, c, d) in enumerate(COLUMNS):
                g(a, b, c, d, m[2 * i], m[2 * i + 1], r == 6)
            m = [m[PERMUTATION[i]] for i in range(16)]
        return [self.xor(s[i], s[i + 8]) for i in range(8)]


def open_literal(b, value, hs):
    """Constrain hs = big-endian(blake3(token(value))[1..32]) for the u64 at
    witness `value`, writing into witness `hs`."""
    v = b.w[value]
    n = len(str(v))

    # Twenty decimal digits, most significant first, each in 0..9.
    digits = []
    for i in range(20):
        d = b.new(v // 10 ** (19 - i) % 10)
        b.range(W(d), 4)
        b.range(C(9) - W(d), 4)
        digits.append(d)
    b.assert_zero(W(value) - sum((W(d) * 10 ** (19 - i) for i, d in enumerate(digits)), C(0)))

    # One-hot digit count: sel[k] = 1 iff the value has k + 1 digits.
    sel = [b.new(1 if k + 1 == n else 0) for k in range(20)]
    for s in sel:
        b.boolean(s)
    b.assert_zero(sum((W(s) for s in sel), C(0)) - C(1))
    # at_least[k] = 1 iff the value has at least k + 1 digits; every digit
    # above the count is zero.
    at_least = [sum((W(s) for s in sel[k:]), C(0)) for k in range(20)]
    for k in range(1, 20):
        d = digits[19 - k]
        # d * (1 - at_least[k]) = 0
        b.assert_zero(W(d), [(-1, d, b.witness(at_least[k]))])
    # Canonical form: the leading digit is nonzero unless the value is 0..9.
    leading = Lin()
    for k, s in enumerate(sel):
        leading = leading + W(b.mul(s, digits[19 - k]))
    b.range(leading - C(1) + W(sel[0]), 4)

    # Token bytes per digit count, as constants or (digit index) entries.
    products = {}

    def product(k, j):
        if (k, j) not in products:
            products[(k, j)] = b.mul(sel[k], digits[j])
        return products[(k, j)]

    def byte(k, p):
        """Byte p of the token for k + 1 digits: a Lin over sel and products."""
        dcount = k + 1
        tok_len = dcount + 46
        if p == 0:
            return W(sel[k]) * 34
        if p <= dcount:
            return W(product(k, 20 - dcount + p - 1)) + W(sel[k]) * 48
        if p < tok_len:
            return W(sel[k]) * SUFFIX[p - dcount - 1]
        return Lin()

    words = []
    for wi in range(17):
        e = Lin()
        for k in range(20):
            for q in range(4):
                e = e + byte(k, 4 * wi + q) * (256 ** q)
        words.append(e if e.is_const() else W(b.witness(e)))
    words += [C(0)] * 15

    two = W(sel[18]) + W(sel[19])
    len1 = sum((W(sel[k]) * (k + 47) for k in range(18)), C(0)) + two * 64
    flags1 = C(CHUNK_START | CHUNK_END | ROOT) - two * (CHUNK_END | ROOT)
    out1 = b.compress([C(v) for v in IV], words[:16], W(b.witness(len1)), W(b.witness(flags1)))
    len2 = W(sel[18]) + W(sel[19]) * 2
    out2 = b.compress(out1, words[16:32], W(b.witness(len2)), C(CHUNK_END | ROOT))
    two_w = b.witness(two)
    digest = []
    for o1, o2 in zip(out1, out2):
        # digest = o1 + two * (o2 - o1)
        diff = b.witness(o2 - o1)
        digest.append(o1 + W(b.mul(two_w, diff)))

    # Little-endian bytes of each word; hs is big-endian over bytes 1..31.
    digest_bytes = []
    for e in digest:
        x = b.val(e)
        parts = [b.new((x >> (8 * q)) & 0xFF) for q in range(4)]
        for p in parts:
            b.range(W(p), 8)
        b.assert_zero(e - sum((W(p) * (256 ** q) for q, p in enumerate(parts)), C(0)))
        digest_bytes += parts
    b.assert_zero(W(hs) - sum((W(p) * 256 ** (31 - i) for i, p in enumerate(digest_bytes) if i), C(0)))


def unpack(data):
    import msgpack  # only needed to read and write compiled programs

    assert data[0] == 3, "expected msgpack-serialized ACIR"
    return msgpack.unpackb(data[1:], strict_map_key=False, raw=False)


def pack(obj):
    import msgpack

    return b"\x03" + msgpack.packb(obj, use_bin_type=True)


def splice(artifact_in, witness_in, artifact_out, witness_out):
    with open(artifact_in, encoding="utf-8") as f:
        artifact = json.load(f)
    program = unpack(gzip.decompress(base64.b64decode(artifact["bytecode"])))
    with open(witness_in, "rb") as f:
        stack = unpack(gzip.decompress(f.read()))
    function = program[0][0]
    names = [p["name"] for p in artifact["abi"]["parameters"]]
    assert names[-2:] == ["values", "hs"], names
    # The private parameters are values[0..N] then hs[0..N].
    private = function[3]
    n = len(private) // 2
    witness = {int(k): int.from_bytes(v, "big") for k, v in stack[0][0][1].items()}
    b = Builder(witness, max(witness) + 1)
    for value, hs in zip(private[:n], private[n:]):
        open_literal(b, value, hs)
    function[2] = function[2] + b.ops
    artifact["bytecode"] = base64.b64encode(gzip.compress(pack(program))).decode()
    with open(artifact_out, "w", encoding="utf-8") as f:
        json.dump(artifact, f)
    stack[0][0][1] = {k: fb(v) for k, v in sorted(b.w.items())}
    with open(witness_out, "wb") as f:
        f.write(gzip.compress(pack(stack)))
    print(json.dumps({"openings": n, "added_opcodes": len(b.ops), **b.stats}))


def main():
    cmd = sys.argv[1]
    if cmd == "hs":
        print(hs_of(int(sys.argv[2])))
    elif cmd == "splice":
        splice(*sys.argv[2:6])
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main()
