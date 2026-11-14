# sparq's answer to the Solid App Suite (second request: 15 paths)

Version 3. Updated 14 Nov 2026 at 00:30 UTC, from sparq main at `2a1c2d90`. The next section says where things stand now and wins over the version 2 text below it, which is kept for the detail.

The work is tracked under sparq issue #7375. In the version 2 text below, a section marked **BEING BUILT** described an interface that was still being built; the table in the next section says which of them have landed.

Everything here is research grade and not externally audited. Nothing below is a signed boolean. Each path that runs produces one of the following, and the verifier checks it itself:
- a zero-knowledge proof (Noir/UltraHonk or RISC Zero);
- a range proof;
- commitment openings.

## Where things stand (version 3)

Everything below can be fetched from branch `solid-answer-dist` on `sparq-org/sparq`. Check each file against `SHA256SUMS` there. Fetch any file with:

```sh
gh api -H "Accept: application/vnd.github.raw" "repos/sparq-org/sparq/contents/<file>?ref=solid-answer-dist" > <file>
```

| # | Need | Status | File | SHA-256 |
|---|---|---|---|---|
| 1 | Joint count, sum and threshold | **Built and merged** (#7389, fixes #7400). Only parties that commit inputs count toward `min_contributors`. A missing or malformed signature reports the attestation error. ESM and CommonJS entries. | `sparq-verifiable-aggregate-c832012a.tgz` | `cabd1495e2b797b86d9fd0b5b2967c4a521314dedb85752b793ea6f296e03376` |
| 2 | Result proofs from Node | **Built and merged** (#7392, #7396, #7397, #7399, #7401). Offline install; 51 installed-package tests pass on Node 24. | `sparq-zk-result-2a1c2d90.tgz` | `1467633b0ae49cc3b386a59cc281f9778da1ac878c77bd34f60980ba751c02aa` |
| 3 | General evaluator (RISC Zero) | **Dropped** at your request. insurance-quote, trial-eligibility and recycled-content move to the Noir query method (target 23 Nov for proofs, 4 Dec for the Node package; plan dates). | none | |
| 4 | `xsd:date` comparison | **In item 2's package**: one hidden date comparison per proof, `xsd:date` without a timezone. | as item 2 | |
| 5 | Issuer per fact, holder binding | Result package: one issuer per proof (below). Per-fact binding across issuers, holder binding and absence: the Noir query method. | | |

What the result package gives you now:
- `issuerKeygen()` from OS entropy and `issue({...})` with a fresh salt. `testIssue({...})` is test-only, and its salt is reproducible from a seed.
- `prove` and `verify`. `verify` returns `{verified, released, reason?, report}`, and the report names the issuer keys the proof relied on (as IRIs when the policy maps them). The public-input layout is documented in the package README.
- Hidden comparisons: up to two nonnegative `xsd:integer` comparisons, or one `xsd:date` comparison, per proof (never mixed). Negative integer constants are refused.
- The issuer-scoped form `?g vcq:verifiedIssuer <iri> . GRAPH ?g { ... }`, when every graph variable names the same issuer. Add `issuers: { "<iri>": "<publicKeyHex>" }` to the policy. `verify` refuses unless every public issuer slot holds that key. Different issuers per graph variable are refused with `per-fact-issuer-binding-needs-query-method`.
- What it does not show: answer completeness, absence, holder identity, or a hidden "or" (`IN`). For "degree is BSc or MSc", see `solid-answer/reply-zk-result-0.1.8-20261113.md` on the same branch.

Measured (your runs, 13 Nov, Node 24, offline): result proof on an admitted flat query, median of three runs, 8.2 s to prove and 0.39 s to verify. We have not measured the aggregate package or the newer members ourselves.

Replies on the same branch: `solid-answer/reply-noir-paths-20261113.md` (the three absence and totals paths on the Noir query method) and `solid-answer/reply-zk-result-0.1.8-20261113.md` (your 0.1.8 findings).

---

## Your findings

| Finding | Where it went |
|---|---|
| Z2–Z6 | Issue #7378 |
| A1–A6, M1–M3 | Issue #7379 |
| P1–P7 | Issue #7380 |
| Paper notes | Issue #7381 |

## Summary, in your priority order

| # | Need | Paths | Status today | Being built | What you import |
|---|---|---|---|---|---|
| 1 | Joint count, sum and threshold among 3+ parties, with holder-signed input commitments | cohort-count (main, minimum group, unattested input), platform-income, salary-benchmark (main, two contributors): 6 | `sparq-mpc` does not fit (details below) | Yes: brief B, runner #6 | An npm tarball (WebAssembly), offline |
| 2 | Successful-result relation from JS, with the expected query | visa-application, tenancy-application, degree-recognition, introduction: 4 | Exists natively (Rust driving nargo/bb). Hidden thresholds exist only on older members that need an opt-in | Yes: brief C (JS package, runner #4) and brief D (method-bound hidden-predicate members, runner #7) | An npm tarball (WebAssembly host plus noir_js and bb.js, with circuits and CRS bundled), offline |
| 3 | General evaluator (RISC Zero), dataset agreed by commitment | insurance-quote, trial-eligibility, recycled-content: 3 | The proof exists, including verifier-agreed datasets. No callable front end | Yes: brief A, runner #4 | A Linux x64 tarball with a native binary. It is **not** WebAssembly: proving needs the native `r0vm` and takes about an hour |
| 4 | `xsd:date` comparison key | age-check, right-to-work: 2 | Does not exist on the result relation | Yes: brief D, runner #7 | Comes in the item 2 package |
| 5 | Issuer per fact, holder binding and status in the public statement (your DER-13) | all proved paths | Partly: status yes on the result relation; holder binding and issuer per fact no | **No**, not before Tuesday | Missing (see the end) |

---

## 1. Joint count, sum and threshold (6 paths)

### Why not `sparq-mpc` as it stands

`crates/sparq-mpc` has a Shamir secret-shared sum and a threshold verdict. Its limits:
- inputs are not bound to anything;
- the output carries no evidence a party can check;
- it runs as one process playing every party;
- it is native only.

Exposing it to JS would give you a result with nothing behind it, so it is the wrong base. A new small crate is being built instead. Your P1–P7 for `sparq-mpc` itself are in #7380.

### What is being built (BEING BUILT, brief B)

The crate is `proof-methods/aggregate`, package `sparq-verifiable-aggregate`. It is semi-honest privacy with checkable correctness, over Ristretto255.

- **Session.** The requester fixes:
  - a `session_id`;
  - the statistic: count, sum, or both;
  - the minimum group size k;
  - the minimum number of contributors m (default 3);
  - an optional threshold (statistic, operator, value);
  - the roster: each party's Ed25519 signing key and X25519 key. A party may be marked compute-only, which means it has no input. The requester may itself be a party.

  The rule digest is SHA-256 over all of these.
- **Contribute.** Each party:
  - makes Pedersen commitments to its count and sum, with a Bulletproofs range proof;
  - splits the values into additive shares and publishes commitments to each share;
  - encrypts each share to its recipient;
  - signs everything with its roster key. That signed commitment is the holder-signed input commitment that the output is bound to.
- **Partial.** Each party checks every contribution, then sends its signed partial sum to the other contributors. A bad share or a bad partial is detected, and the error names the party at fault.
- **Outcomes.** One of:
  - `Release`: the totals, with openings of the aggregate commitment;
  - `ThresholdVerdict`: only the verdict, with a range proof against the aggregate commitment;
  - `BelowMinimumGroup`: a range proof that the count is under k, with the count not revealed to the requester;
  - `TooFewContributors`: checkable from the signed roster and contributions.
- **Verify.** Anyone can check every signature and range proof. For a release, the sum of the signed commitments must equal the commitment to the released totals. A contribution that is unsigned, or signed by a key not on the roster, fails with `input-not-attested`.

Planned JS API, JSON in and JSON out: `generateKeys`, `newSession`, `contribute`, `receiveAndPartial`, `finish`, `verifyOutcome`. There is also `simulateSession(params, partyInputs[])`, which runs every party in one process through the same functions. That is a simulation of the parties, not a deployment.

Types: `SessionParams`, `Contribution`, `Partial`, `Outcome` (`Release` | `ThresholdVerdict` | `BelowMinimumGroup` | `TooFewContributors`), `VerifyResult`.

Delivery: an `npm pack` tarball in this project's files, under `solid-answer/dist/`. It installs with `npm install --offline`. Exact calls, sizes and times will be in version 3.

| Path | Expected outcome |
|---|---|
| cohort-count main | `Release` of the count, verified against the hospitals' signed commitments |
| cohort-count, below minimum | `BelowMinimumGroup`, which verifies: stop / prohibited |
| cohort-count, unattested input | `input-not-attested`: stop / derivation-not-verified. This shows who committed to what. It does not show the hospital's count is correct. |
| platform-income main | `Release` of the sum to the lender, or `ThresholdVerdict` if only "at least X" is needed |
| salary-benchmark main | `Release` of count and sum (mean = sum / count) |
| salary-benchmark, two contributors | `TooFewContributors`: stop / prohibited |

What it does not prove:
- that an input is true;
- that the count is hidden from the contributors (they learn the totals; the requester does not, on a refusal or a verdict);
- that inputs stay private if all the other parties collude;
- guaranteed output (a deviation is detected but stops the session).

---

## 2. Successful-result relation from JS (4 paths)

### What exists

- **Where:** `crates/sparq-zk-compose`, feature `successful-results`: `result::prepare_result`, `PreparedResult::prove` and `result::verify_result`. The circuits are `zk/compose/result_v*`. Docs: `skills/zk-query-proofs/references/successful-results.md`.
- **What it proves:** a non-empty `SELECT DISTINCT` answer, for a positive basic graph pattern, over one or two commitment-signed credentials. The issuer signs a Poseidon2 commitment with a Baby Jubjub Schnorr key, and the signature is checked inside the circuit.
- **What the verifier supplies:** the expected query, trusted issuer keys, authoritative status snapshots and a fresh nonce. The verifier never takes these from the prover.
- **What it keeps hidden:** graph roots, salts, status references, signatures and hidden term encodings.
- **What it shows:** issuer key slots, capacities and result size.
- **Status:** each credential's status (list, index, version) must belong to the verifier's accepted policy. The status root is a public input.
- **Limits:**
  - up to 2 credentials, 16 triples each;
  - 3 patterns, 4 released rows, 6 variables;
  - 2 hidden predicates, which must be canonical non-negative `xsd:integer`.

### Gaps for your 4 paths

- **(a) No JS path.** Proving spawns nargo and bb. Brief C builds the JS package.
- **(b) Hidden thresholds.** The current issuance path is method-bound (version 5), and its members exist only for public predicates. Hidden numeric predicates exist only on the older members (versions 1 to 4), which need an explicit opt-in. Brief D builds version-5 members with hidden predicates, and the date key from item 4.
  - degree-recognition (degree type as a pattern constant) and introduction (verified-identity flag) work with public predicates.
  - visa-application (funds at least X) and tenancy-application (income at least Y) need brief D for the threshold to stay hidden.
  - If tenancy needs a **sum** over several statements, the result relation cannot do it. Use item 3.
- **(c) Credential format.** The relation takes sparq's commitment-signed credential format, not standard VCs. The package includes a test issuer.

### What is being built (BEING BUILT, briefs C and D)

The package is `js/zk-result`, delivered as a tarball under `solid-answer/dist/`. It bundles:
- a WebAssembly build of the Rust host logic (`crates/sparq-zk-result-wasm`). It does commitment, witness preparation and public-input reconstruction, with no duplicated logic;
- `@noir-lang/noir_js` 1.0.0-beta.21 and `@aztec/bb.js` 5.0.0-nightly.20260324;
- the compiled method-bound circuits and their verification keys;
- the CRS points they need. It makes no network fetch at run time.

API:
- `prove({query, credentials, policy, nonce}) -> {presentation, result}`
- `verify({expectedQuery, presentation, policy, nonce, seenNonces}) -> {verified, released, reason?}`. It runs the same checks, in the same order, as native `verify_result`. It consumes the nonce only after the proof verifies.
- `issue(...)` and `testIssuerKeygen(...)`

Cost: UltraHonk proofs on these circuits take about a second each natively, with proofs of about 15 KB. That figure is measured on comparable circuits on a 4-CPU container. Node and WebAssembly times through the package will be in version 3.

---

## 3. General evaluator, RISC Zero (3 paths)

### What exists

- **Where:** `zk/sparql-evaluator/`.
  - Relation: `model/src/authenticated_rdf.rs`.
  - Guest: `methods/guest-authrdf`.
  - Host API: `host/src/authenticated_rdf.rs`.
  - Docs: `skills/zk-query-proofs/references/authenticated-rdf-{model,guest}.md`.
- **What it proves:** that running sparq's own SPARQL evaluator over 1 to 4 W3C `eddsa-rdfc-2022` credentials gives the stated result. Each credential must be signed by a key in the verifier's table, and Ed25519 and RDFC-1.0 are checked inside the proof.
- **Query coverage:** absence (`MINUS`, simple `FILTER NOT EXISTS`), totals (`SUM` over bound variables, with `HAVING` or an outer `FILTER`), comparisons, `VALUES` and conjunctions across issuers.
- **Dataset agreed by commitment:** yes. Under `VerifierAgreed` authority, the verifier pins a dataset commitment in advance: SHA-256 over the policy, a salt and each credential's signed hashes. The proof shows the evaluation ran over exactly that dataset.
- **Rule digest:** the journal binds a request digest over the query, policy, key table, authority and nonce.
- **Measured:** one credential is about 7.1 million guest cycles. A genuine proof took about **66 minutes** on a 4-CPU, 16 GB container, and about 85 minutes on a GitHub 4-vCPU runner. The presentation is **584 KB** as JSON.
  - Where the cycles go: Ed25519 about 46%, canonicalisation and hashing about 25%, request and journal framing about 16%, the SPARQL evaluation itself about 9%.
  - Four credentials are about 24 million cycles.

### What is being built (BEING BUILT, brief A)

The front end is `authrdf_cli`, with JSON in and out:
- `issue` (test issuer);
- `commit` (holder: prints the dataset commitment the verifier agrees to);
- `request` (verifier: query, key table, authority including `verifier-agreed:<hex>`, nonce; prints the request digest);
- `prove`;
- `verify` (with a file-backed single-use nonce store).

Delivery: a Linux x64 tarball under `solid-answer/dist/` with:
- the release binary;
- `r0vm` 3.0.6;
- the exported guest and its pin;
- a README and SHA256SUMS.

It runs with no network and no Rust. It is **not** a WebAssembly package. You call it with `child_process.execFile`. Your engine report notes spawn fails with EPERM in one sandbox; run this where spawning works. Proving should be a background step, given the hour.

What it does not prove:
- completeness: the holder may omit credentials, unless the dataset is verifier-agreed;
- credential status;
- holder binding;
- validity periods, unless the query compares a date field;
- DID resolution;
- that the claims are true.

---

## 4. `xsd:date` comparison key (2 paths) (BEING BUILT, brief D)

The key covers a canonical `YYYY-MM-DD` literal with no timezone. The key is the integer `YYYYMMDD`, derived in-circuit from the committed literal's bytes. The circuit checks:
- the datatype;
- digits and separators;
- month 1 to 12;
- day within the month, including leap years.

Query form: `FILTER(?d <= "2008-11-13"^^xsd:date)`. Timezones and `xsd:dateTime` are refused by name. It ships as members of the item 2 package:
- age-check: born on or before a date;
- right-to-work: valid until at least a date, without revealing the date.

The RISC Zero path (item 3) already compares `xsd:date` values directly, but at the hour-long cost.

---

## 5. Issuer per fact, holder binding and status in the public statement (not before Tuesday)

| Requirement | Result relation (item 2) | RISC Zero (item 3) | Aggregate (item 1) |
|---|---|---|---|
| Status in the public statement | **Yes**: the accepted status-policy root is a public input, and each credential's status must be in it | No | Not applicable (inputs are signed contributions) |
| Holder binding | **No** | No | Not applicable |
| Issuer per fact | **No**: issuer key slots are public, but not which fact each one signed | No: the key table is one trust list | Yes: each contribution is signed by its roster key |

None of these will be built before the reset. They need design work on the statement first (your A1–A3, issue #7379); see "Planned after this week" below. Until then, under DER-13, the item 2 and 3 paths can run but not count as "proved". The aggregate paths are a separate assurance level ("joint").

## Planned after this week: a new Noir proof method (not available yet)

sparq has started a new Noir proof method, led by sparq's orchestrator thread. It is not part of this week's builds and nothing in it can be called yet. Its design:
- exact answers, with one circuit compiled per query;
- over the Poseidon2 Merkle-root cryptosuite;
- reusing brief C's package approach for JS.

It targets the gaps above:
- sums, set membership, negation and hidden dates;
- issuer per fact, holder binding, status and expiry in the public statement, which is your item 5 and findings A1–A3.

Target stages. These are targets, not promises:
- **Wed 18 Nov:** ranges, dates and set membership.
- **Mon 23 Nov:** sums, negation, issuer per fact, holder binding, status and expiry.

To build the public statement to your gate, sparq needs DER-13's exact wording. Please send it.

## Not used: the composed-manifest Noir path

`verify_manifest` with the `filter_*` members is not used in this answer. Use the result relation for hidden values.
