# Evaluation plan: signature modes, cryptosuites and query types

What the paper needs measured, so that it can compare the two signature modes across
cryptosuites and query types. It uses the mode names from the zero-knowledge SPARQL answers
specification: `hidden` and `revealed`. Every number goes into the paper through a frozen audit
record under `research/zk-paper-evidence/` and a key in `paper-evidence.json`, as it does now.

## 1. Modes

| Mode | What the verifier receives | What the proof shows |
|---|---|---|
| `hidden` | The answer and the proof's public output | Knowledge of a valid signature from an accepted issuer key on every credential; the input dataset is exactly the signed data; the query answer |
| `revealed` | Also, for each credential: signature, verification method, cryptosuite, signed message | The input dataset is exactly the data the revealed signed messages cover; the query answer. The verifier checks the signatures itself |

## 2. Cryptosuites

| Id | Cryptosuite | Status | Why it is in the paper |
|---|---|---|---|
| S1 | `eddsa-rdfc-2022` (Ed25519, RDFC-1.0, SHA-256) | W3C Recommendation | The deployed suite; already implemented |
| S2 | `ecdsa-rdfc-2019` with P-256 | W3C Recommendation | The other common RDF suite |
| S3 | `mldsa44-rdfc-2024` (ML-DSA-44) | W3C Quantum-Resistant Cryptosuites, First Public Working Draft (16 June 2026) | A post-quantum suite on the standards track |
| S4 | Our Merkle-root suite: the issuer signs the root of a Merkle tree whose leaves are field-encoded RDF terms, with literal encodings chosen for cheap comparisons | New; specification to be linked from the paper | The suite designed for proving |

S4 needs one root signature that is cheap to verify inside a proof. If time allows, a second, post-quantum root signature (for example ML-DSA-44) makes the security comparison complete.

`bbs-2023` and the JCS suites (`eddsa-jcs-2022`, `ecdsa-jcs-2019`) are discussed but not measured. BBS proofs are a native alternative to `hidden` mode. JCS suites sign JSON, so they would need a JSON-to-RDF mapping inside the proof.

## 3. Queries

**Main-body set.** These run in every mode and cryptosuite:
- Q1: the payment `ASK` (false);
- Q2: a bag `SELECT`;
- Q3: a `CONSTRUCT`;
- Q4: a numeric `FILTER` (`xsd:decimal` comparison);
- Q5: a string `FILTER` (`STRSTARTS` or `CONTAINS`).

**Appendix sweep.** One query per feature class, over the same synthetic credentials. Use only features the guest admits (see `zk/sparql-evaluator/coverage.json`). List rejected features as unsupported, not as zero cost.
- **Patterns:** BGPs with 1, 3 and 6 triple patterns, in star and chain shapes; `OPTIONAL`, `UNION`, `MINUS`, `FILTER EXISTS` and `NOT EXISTS`; `BIND`, `VALUES`, a subquery.
- **Comparisons by term kind:**
  - IRI equality;
  - plain and language-tagged string equality;
  - `xsd:integer`, `xsd:decimal` and `xsd:double` comparison;
  - `xsd:dateTime` comparison;
  - `STRSTARTS`, `CONTAINS`, `REGEX` and `LANGMATCHES`.
- **Aggregates:** `COUNT`, `SUM`, `MIN` and `MAX` with `GROUP BY` and `HAVING`.
- **Solution modifiers:** `DISTINCT`, and `ORDER BY` with `LIMIT`/`OFFSET`.
- **Property paths:** sequence, alternative, inverse, `+` and `*`.
- **Query forms:** `SELECT`, `ASK` (true and false), `CONSTRUCT` and `DESCRIBE`.

**Scale.** Use n ∈ {1, 4, 16} credentials of about {16, 64, 256} triples each. Go larger in execution-only runs if cheap.

## 4. Roles and metrics

| Role | Metrics |
|---|---|
| Issuer | Signing time; signature size; credential size; for S4, tree construction time |
| Holder (prover) | Total and user cycles (deterministic); segment count; proving time; peak memory; receipt size; presentation size |
| Verifier | Verification time, split into receipt verification, signature checks (`revealed` only) and request checks; bytes received |

**Cost breakdown.** Inside the guest, count cycles per phase with `env::cycle_count()`:
- signature verification;
- canonicalisation and hashing, or Merkle path checks;
- dataset construction;
- query evaluation;
- output.

This breakdown is the evidence for the claim that proving knowledge of the signature dominates the cost.

## 5. How to measure

- **Cycles and sizes:**
  - Take them from the RISC Zero executor, which needs no proof. They are deterministic, so the paper can report them exactly.
  - Run the whole appendix sweep this way.
- **Times:**
  - Produce full proofs, with dev mode off, for the main-body set and a representative subset of the sweep. The subset should be enough to show how proving time grows with cycles.
  - Repeat each run at least 5 times and report the median and interquartile range.
- **Machine:**
  - Run all timings on one dedicated machine with no other load. The existing m7i.2xlarge in eu-west-2 is the default.
  - Record the machine type, CPU, cores, memory, operating system, Rust toolchain, r0vm version and guest image ID.
  - Time issuer and verifier steps on the same machine.
  - A laptop-class verifier timing is optional.
- **Receipt type:**
  - Record whether each receipt is succinct (STARK) or Groth16.
  - Proof soundness against a quantum adversary differs between the two.

## 6. Facts the security table needs from the specification and code

For each mode and cryptosuite:
- **Signature scheme:** classical and post-quantum unforgeability.
- **Receipt type:** classical and post-quantum soundness of the proof.
- **Disclosure:** exactly what the verifier receives. That means:
  - the issuer identity;
  - the signature value;
  - the signed message or Merkle root;
  - any input commitment;
  - bounds and sizes.
- **Linkability across presentations:**
  - through a revealed value, which needs no break;
  - only by breaking zero-knowledge, and whether that property is statistical or computational;
  - or not at all.
- **Guessing hidden content:** whether a revealed digest or root lets a verifier confirm guesses about low-entropy hidden content, that is, whether it is salted or hiding.

## 7. Priorities

1. **Main body (needed for submission).** Cover:
   - `hidden` and `revealed`;
   - S1 and S4 (one root signature);
   - the main-body queries;
   - n ∈ {1, 4};
   - cycles, sizes and full-proof timings;
   - issuer and verifier timings;
   - the per-phase cycle breakdown.
2. **Appendix.** Run the execution-only cycle sweep over all query classes and scales for every main-body configuration. Add proof timings for a subset.
3. **If time allows.** Add, in this order:
   1. S3 (`mldsa44-rdfc-2024`);
   2. S2 (`ecdsa-rdfc-2019`);
   3. a post-quantum root signature for S4.

## 8. State of the implementation (from ZK code landing, 10 October 2026)

- `eddsa-rdfc-2022` is verified natively and inside the RISC Zero V5 guest. `ecdsa-rdfc-2019` is
  verified natively only. `bbs-2023` and `ecdsa-sd-2023` are not verified yet.
- S4 draft: `schnorr-poseidon2-merkle-2026`, Schnorr over Baby Jubjub with a Poseidon2 challenge
  (not post-quantum). A post-quantum variant could sign the same root
  (`mldsa44-poseidon2-merkle-2026`). The tree is a binary Poseidon2 tree whose leaves are the
  RDFC-1.0 canonical quads. Each leaf encodes subject, predicate, object and graph with a typed term
  encoding that includes an order-preserving lane for numbers and dates. The signed message is one
  BN254 field element derived from the root, quad count, tree depth and proof-configuration digest.
- Modes: the Noir path supports `hidden` and `revealed`. The RISC Zero V5 path supports `hidden`
  today; `revealed` is next.

## 9. Canonical main-body queries (fixed by ZK code landing, 10 October 2026)

All five use the 17-statement payment credential behind the payment `ASK` evidence: three settled
`xsd:decimal` payments, signed with `eddsa-rdfc-2022`.

- Q1 `ASK`: is any payment's status `bank:Returned`? (false)
- Q2 `SELECT ?amount` over a two-pattern BGP; a bag, because two amounts are equal.
- Q3 `CONSTRUCT` over the same BGP.
- Q4 `FILTER(?amount > 1300.00)`.
- Q5 `FILTER(STRSTARTS(STR(?p), "https://bank.example/payments/2026-07"))`: a string comparison
  over `STR` of an IRI, since the credential has no plain string literal. A comparison on a string
  literal belongs in the appendix sweep.

The Merkle-root suite now has two members: `eddsa-sha256-merkle-2026` (zkVM, built into V5) and a
Poseidon2 member specified for Noir.
