import unittest

import blake3_f2


class Blake3F2Test(unittest.TestCase):
    def test_reference_matches_official_vector(self):
        blake3_f2.selftest()

    def test_circuit_matches_reference_and_counts_ands(self):
        c = blake3_f2.build(2, seed=3)
        self.assertEqual(len(c.public), 2 * 256)
        self.assertEqual(len(c.private), 2 * 768)
        self.assertLessEqual(c.ands, 2 * 10416)
        self.assertGreater(c.ands, 2 * 10400)

    def test_wrong_public_output_is_rejected(self):
        original = blake3_f2.compress

        def tampered(*args):
            out = original(*args)
            return [out[0] ^ 1] + out[1:]

        blake3_f2.compress = tampered
        try:
            with self.assertRaises(AssertionError):
                blake3_f2.build(1, seed=1)
        finally:
            blake3_f2.compress = original

    def test_flipped_output_bit_fails_its_assertion(self):
        c = blake3_f2.Circuit()
        bit = c.priv(1)
        with self.assertRaises(ValueError):
            c.assert_zero(c.xor(bit, c.pub(0)), "flipped")


if __name__ == "__main__":
    unittest.main()
