# Hand-written ACIR for the integer literal opening

A prototype that asks whether writing ACIR by hand, instead of compiling Noir,
reduces the cost of the result circuits' largest part: opening a committed
`xsd:integer` literal (`full_u64_check` in
`zk/compose/compose_core/src/result_integer.nr`).

The committed term encoding is `h2(2, hs)`, where `hs` is the BLAKE3 hash of
the literal's N-Triples token, truncated to 31 bytes. The token's length
depends on the number of decimal digits, which is private. ACIR's BLAKE3
black box takes a fixed-length input, so the Noir function hashes the token
once for each of the 20 possible digit counts and constrains all 20.

`blake3_leaf.py` constrains the same relation with one BLAKE3 over a
variable-length input:

- twenty digit witnesses in 0..9 that recompose to the value, and a one-hot
  digit-count selector; every digit above the count is zero and the leading
  digit is nonzero unless the value is below 10;
- the token's message words as selector-weighted sums of the digits and the
  constant bytes;
- two BLAKE3 compressions built from XOR and RANGE black boxes and 32-bit
  additions with carries, with the block length and flags selected by the
  digit count (a token of at most 64 bytes is one block, 65 or 66 bytes is
  two); the digest is the first or the second compression's output;
- the digest's bytes, recomposed big-endian into `hs`.

The opcodes do not depend on the value (`test_blake3_leaf.py` checks this),
so the circuit reveals no digit count. The generator appends them to the
compiled `circuits/spliced` program, which checks `h2(2, hs) == operand` and
the comparison verdict, and it solves and checks its own witnesses.

`circuits/noir_var` writes the same variable-length algorithm in Noir, so the
comparison separates the gain from the algorithm from the gain from writing
ACIR by hand. `circuits/baseline` calls `full_u64_check`. Each circuit opens
`leaf::N` values.

```sh
# nargo and bb at the versions CI pins for zk/compose
NARGO=nargo BB=bb proof-methods/acir/run.sh 0 9 10 999999999999999999 \
  1000000000000000000 9999999999999999999 10000000000000000000 18446744073709551615
bb gates -s ultra_honk -b proof-methods/acir/circuits/target/hand.json
BB=bb python3 proof-methods/acir/tamper.py 30
python3 -m unittest proof-methods/acir/test_blake3_leaf.py
```

`run.sh` proves and verifies all three circuits for the given values.
`tamper.py` changes one witness of the hand-written circuit at a time and
checks that the proof fails. Measurements are kept outside the repository.

Research prototype, not externally audited. It covers the unsigned opening
only; `full_i64_check` has the same structure.
