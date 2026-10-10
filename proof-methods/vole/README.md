# QuickSilver over the Noir circuits

A prototype for the proposed `urn:sparq:vcq:method:vole-designated-verifier`
method. It proves the same constraint system that UltraHonk proves, the ACIR
that `nargo` compiles from `zk/compose`, with an interactive, designated-verifier
zero-knowledge proof based on vector oblivious linear evaluation (QuickSilver,
as implemented by Diet Mac'n'Cheese in [GaloisInc/swanky]). It measures prover
and verifier cost and communication on identical statements. It is not yet a
proof method: the request digest is not bound into the transcript, and there is
no holder-to-verifier transport.

- `acir_to_sieve.py ARTIFACT.json WITNESS.gz OUTDIR` translates a compiled ACIR
  program and its solved witness into a SIEVE IR0+ relation over the BN254
  scalar field. Black boxes are expanded into field arithmetic: RANGE as bit
  decompositions, `Poseidon2Permutation` as the full permutation with the
  Barretenberg constants (`crates/sparq-zk/src/poseidon2_constants.rs`), and
  dynamic memory access as constrained one-hot selectors. Every constraint is
  evaluated on the witness while it is written. Programs using other black
  boxes (BLAKE3, SHA-256, AND, XOR, curve operations) or several ACIR
  functions are rejected.
- `swanky-fbn254.patch` adds the BN254 scalar field to Diet Mac'n'Cheese,
  whose field dispatch does not include it. It applies to swanky at
  `409d1ceb0831e2de11eb8da1b8f961f59a8b5276`, which builds with Rust 1.99.0.
- `circuits/gen.py K ...` writes Nargo packages that check K independent copies
  of the `hidden_issuer_d4` relation (in-circuit Schnorr verification and
  issuer key-set membership), with a Prover.toml repeating the honest witness
  from `zk/compose/hidden_issuer_d4/Prover_hi_ok.toml` (written by the
  `sparq-zk-compose` e2e tests).
- `quicksilver_run.py` runs one QuickSilver proof and reports the prover's
  time and CPU time, the verifier's CPU time and the time it stays online
  (from the prover's launch to its own exit), and, with `--count-bytes`, the
  bytes sent in each direction, VOLE setup and extension included.
- `bench.sh DIETMC REPS ARTIFACT WITNESS ...` proves each circuit with `bb`
  (UltraHonk) and with QuickSilver, checks that both have the same public
  inputs in the same order, and prints one JSON line per circuit. `LPN`
  (default `large`) sets the Diet Mac'n'Cheese LPN parameter size, and
  `BB_TARGET` (default `noir-recursive`, the target `sparq-zk-compose` proves
  for) the bb verifier target. Over a large field swanky's `medium` uses
  smaller LPN sets for which it states no security level.
- `test_acir_to_sieve.py` tests the translator's public-input handling and
  witness checks (`python3 -m unittest`).

```sh
git clone https://github.com/GaloisInc/swanky && cd swanky
git checkout 409d1ceb0831e2de11eb8da1b8f961f59a8b5276
git apply ../sparq/proof-methods/vole/swanky-fbn254.patch
cargo build --release -p diet-mac-and-cheese --bin dietmc
cd ../sparq/proof-methods/vole/circuits && python3 gen.py 1 4 16 32
for k in 1 4 16 32; do (cd hidden_issuer_x$k && nargo execute w); done
cd .. && ./bench.sh ../../../swanky/target/release/dietmc 3 \
  circuits/hidden_issuer_x1/target/hidden_issuer_x1.json circuits/hidden_issuer_x1/target/w.gz
```

QuickSilver evidence convinces only the verifier taking part in the protocol;
it cannot be forwarded or audited later. The communication grows linearly with
the number of multiplications and private inputs, where an UltraHonk proof has a
fixed size. Research prototype, not externally audited.

[GaloisInc/swanky]: https://github.com/GaloisInc/swanky
