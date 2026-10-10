"""Unit tests for acir_to_sieve.py: python3 -m unittest proof-methods/vole/test_acir_to_sieve.py"""

import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import acir_to_sieve as a  # noqa: E402


def b(value):
    return (value % a.P).to_bytes(32, "big")


def program(public, returns, opcodes):
    return [[["main", 2, opcodes, [], public, returns, []]]]


def witness(values):
    return [[[0, {k: b(v) for k, v in values.items()}]]]


# w1 - 7 = 0
W1_IS_7 = {"AssertZero": [[], [[b(1), 1]], b(-7)]}


class PublicInputs(unittest.TestCase):
    def translate(self, public, returns, values):
        diag, rc = a.load_poseidon2_constants(a.CONSTANTS)
        t = a.Translator(program(public, returns, [W1_IS_7]), witness(values), diag, rc)
        t.run()
        return t.c

    def test_a_witness_in_both_sets_is_one_public_input(self):
        c = self.translate([1], [1], {1: 7})
        self.assertEqual(c.public, [7])

    def test_public_inputs_follow_witness_index_order(self):
        c = self.translate([2], [1], {1: 7, 2: 9})
        self.assertEqual(c.public, [7, 9])

    def test_a_witness_that_violates_the_constraint_is_rejected(self):
        with self.assertRaises(ValueError):
            self.translate([1], [1], {1: 8})


if __name__ == "__main__":
    unittest.main()
