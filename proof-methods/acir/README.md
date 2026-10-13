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

`circuits/noir_var` writes the same variable-length algorithm in Noir with
u32 shifts, OR, `wrapping_add` and per-length conditional writes.
`circuits/noir_tuned` writes it in Noir the way the generator does: the
rotations and additions as a hinted split checked by one linear equation and
range checks, and the token's message words as selector-weighted sums of the
digits. `zk/compose` uses this version for its result circuits.
Together they separate the gain from the algorithm, from how the arithmetic
is written, and from writing ACIR by hand. `circuits/baseline` calls `full_u64_check`. Each circuit opens
`leaf::N` values.

```sh
# nargo and bb at the versions CI pins for zk/compose
NARGO=nargo BB=bb proof-methods/acir/run.sh 0 9 10 999999999999999999 \
  1000000000000000000 9999999999999999999 10000000000000000000 18446744073709551615
BB=bb proof-methods/acir/gates.sh
for c in hand noir_var noir_tuned; do BB=bb python3 proof-methods/acir/tamper.py "$c" 15; done
python3 -m unittest proof-methods/acir/test_blake3_leaf.py
```

`run.sh` proves and verifies all four circuits for the given values, with
bounds that do not reveal them. It does not pin tool versions or thread
counts; compare timings only from one machine and one toolchain.
`gates.sh` prints each circuit's UltraHonk gate count. `tamper.py` changes
one prover-chosen witness at a time (in the hand-written circuit, an appended
witness; in the Noir circuits, a hint output, sampled per hint function) and
checks that the proof fails. It is a smoke test for hint outputs the circuit
leaves unconstrained, not evidence that the constraints are complete: it does
not re-solve the witnesses that depend on the changed one, so a rejection can
come from those stale values rather than from the hint's own constraints, and
it samples rather than covering every call site. Completeness rests on review
of the generator and the Noir source, and, for the hand-written additions, on
the unit test that bounds every addition result. Measurements are kept
outside the repository.

Research prototype, not externally audited. It covers the unsigned opening
only; `full_i64_check` has the same structure.
