"""Unit tests for blake3_leaf.py: python3 -m unittest proof-methods/acir/test_blake3_leaf.py"""

import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import blake3_leaf as m

# One-block and two-block tokens on either side of the 64-byte boundary.
VALUES = [0, 9, 10, 12345, 999999999999999999, 10 ** 18, 10 ** 19 - 1, 10 ** 19, 2 ** 64 - 1]


def build(value, hs):
    b = m.Builder({0: value, 1: hs}, 2)
    m.open_literal(b, 0, 1)
    return b


class Reference(unittest.TestCase):
    def test_blake3_vector(self):
        # BLAKE3 of "abc" from the reference implementation.
        self.assertEqual(
            m.blake3(b"abc").hex(),
            "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85")

    def test_token_lengths(self):
        self.assertEqual(len(m.token(0)), 47)
        self.assertEqual(len(m.token(10 ** 18)), 65)
        self.assertEqual(len(m.token(2 ** 64 - 1)), 66)


class Opening(unittest.TestCase):
    def test_accepts_the_literal_hash(self):
        for v in VALUES:
            build(v, m.hs_of(v))

    def test_rejects_another_literal_hash(self):
        for v in VALUES:
            with self.assertRaises(ValueError):
                build(v, m.hs_of(v + 1))

    def test_every_addition_result_is_bounded(self):
        # An unbounded sum admits a second carry and a wrong hash.
        b = build(0, m.hs_of(0))
        bounded = set()
        for op in b.ops:
            call = op.get("BlackBoxFuncCall", {})
            for name in ("XOR", "RANGE"):
                if name not in call:
                    continue
                args = call[name]
                if name == "RANGE" and args[1] != 32:
                    continue
                for arg in args[:2] if name == "XOR" else args[:1]:
                    if "Witness" in arg:
                        bounded.add(arg["Witness"])
        self.assertEqual([w for w in b.sums if w not in bounded], [])

    def test_circuit_does_not_depend_on_the_value(self):
        # The same opcodes for every digit count: the circuit reveals no length.
        first = build(VALUES[0], m.hs_of(VALUES[0])).ops
        for v in VALUES[1:]:
            self.assertEqual(build(v, m.hs_of(v)).ops, first, v)


if __name__ == "__main__":
    unittest.main()
