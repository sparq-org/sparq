# Language guide: "Answers Instead of Documents"

For `zksparql-architecture.typ`. "Spec §n": the companion specification (*Zero-Knowledge SPARQL Answers*, draft). Bare "§n": the paper. "Guide §n": this file.

## 1. Audience and goal

Readers: ESWC reviewers, expert in RDF and SPARQL, familiar with verifiable credentials, new to zero-knowledge proofs. Aim: one argument, not a project log; established terms (the spec's for our concepts); short active sentences; each claim and caveat once; formulas only where clearer than prose.

## 2. Style rules, in priority order

1. **One term per concept, one concept per term,** from guide §3, defined once, at or before first use.
2. **Established term before coinage; describe rather than name.** Coin only for a concept used three or more times with no established term.
3. **No internal labels in prose** (commit hashes, test ids, enum names, V1 to V5, campaign, gate, bead, `zkp-*`); versions and hashes go in one artifact table.
4. **Say each point once:** each caveat has one home (guide §6); abstract, introduction and conclusion summarise results once each.
5. **Hedge once, where the evidence requires, in that home,** with a number or condition. *Bad:* "Indicative development measurement, not the basis of any claim." *Good:* "Preliminary: [n] runs on a shared machine."
6. **Separate observation, argument and proposal;** label each claim as measured, argued (a proposition with its assumptions) or proposed.
7. **Results, not project history.** *Bad:* "A later gate built the V5 guest and executed it directly". *Good:* "We executed the guest [n] times without proving."
8. **Short sentences:** most under 25 words, none over 40, one idea each; a numbered list for a sequence (spec §8).
9. **Subject and verb early and close.** *Bad:* "the input commitment *c*, which, unless it is hiding, lets a low-entropy input be guessed". *Good:* "An unsalted commitment lets the verifier confirm a guessed input."
10. **Active voice, named actor:** "We report each experiment separately", not "Campaigns are reported separately".
11. **Old information first, new last.** *Bad:* "At a later source, ⟨hash⟩, the adapter produced 1 genuine receipt for the returned-payment question". *Good:* "For the returned-payment question, we produced one receipt."
12. **Verbs, not nominalisations; no stacks of three or more nouns.** *Bad:* "complete validation before the verifier consumes its challenge". *Good:* "the verifier checks everything, then marks the request as answered".
13. **No meta-commentary:** no "we note that", "it is worth noting", roadmaps or reading instructions.
14. **Formula or prose, whichever this reader parses faster:** a formula for a short exact condition ("an Exact `SELECT` answer is the multiset `⟦Q⟧_D`"); a table for structures (spec §4, not "C = ⟨m, o, q, f, a, s, e, d, b, t⟩"); a list for procedures. Define symbols in words; no ∀, ∃ or ⇒ in text or at a sentence start; each sentence reads with its formula replaced by a noun.
15. **Numbers only through the evidence lookups** (`#headline(key)`; `#ev(key)` for preliminary timings): never typed, never summed across experiments.
16. **Running example first:** the lender's three questions before each general definition.
17. **Tense:** present for the design, past for experiments.
18. **Clear referents:** each "this", "it" and "that" has a noun. *Bad:* "That is an absence claim; nothing is ground, and it needs an exact method." *Good:* "An unbound `?e` asserts that no email triple matched; only an Exact answer can show such an absence."
19. **Cite our own prior work in the third person** (zkRDF; the ISWC 2025 doctoral-consortium paper), like any other work, for double-blind review.
20. **Form:** British *-ise* spelling; no em-dashes; code font for SPARQL keywords, JSON members, IRIs and cryptosuites; capitalised Supported and Exact; few acronyms, each defined once; never "VC" (except in spec titles) or "ZKP".

## 3. Controlled vocabulary

"(unverified)": not confirmed in `language-research.md` or the spec. "–": plain English. Full ban list: guide §5.

| Term | Meaning | Source | Do not use |
|---|---|---|---|
| **RDF and SPARQL** | | | |
| input dataset, *D* | RDF dataset the query runs over | spec §2 | scope, committed input |
| merge (unverified) | Union, blank nodes kept apart | spec §2 (in words) | union |
| canonical N-Quads form; isomorphic | RDFC-1.0 output; equal up to blank node renaming | RDFC-1.0 §3.1 | canonical graph equality |
| solution (mapping) μ; multiset; solution sequence | Variables mapped to RDF terms; solutions with repeats (multiplicity); ordered solutions | SPARQL 1.1 §18.1.8, §18.3 | row, bag |
| monotone query (unverified) | Added data never removes a solution | spec §9 (in words) | positive pattern |
| result | Solutions, boolean or graph | spec §5.1 | output |
| **Credentials and signatures** | | | |
| credential; claim; credential subject | Signed claims; one claim; whom they describe | VC DM 2.0 §2 | VC, record |
| issuer; holder; verifier | The roles; the holder is the prover; "verifier service" only for software | VC DM 2.0 §2; spec §2 | presenter, relying party |
| cryptosuite | Named Data Integrity signing scheme | VC-DI | signature suite |
| signature; signed message | Signature value; the bytes it covers | VC-DI; spec §2 | "proof" for these |
| verification method; issuer key | Key identifier; key listed in `issuers` | VC-DI; spec §4 | key table |
| `eddsa-rdfc-2022` | Ed25519 over hashes of the canonical proof configuration and document; unsalted | EdDSA §3.2 | "strict Ed25519" |
| `ecdsa-rdfc-2019`; `mldsa44-rdfc-2024` (unverified) | Same with ECDSA P-256; with post-quantum ML-DSA-44 | VC-DI-ECDSA; W3C draft | S2, S3 |
| `bbs-2023`; `ecdsa-sd-2023` | Holder derives a selective-disclosure proof | BBS; spec §6.3 | native proofs |
| Merkle-root cryptosuite; `schnorr-poseidon2-merkle-2026` (ours, draft) | Schnorr signature on a value derived from a Poseidon2 Merkle root over the canonical quads; not post-quantum | ours; spec §2 | S4 |
| credential status; credential validity period; holder binding | Revocation; `validFrom` to `validUntil`; presenter controls a bound key | VC DM 2.0 §4.9–4.10; spec §10.1 | validity window |
| **Request and answer, as in the spec** | | | |
| query request; answer presentation | The verifier's request; the holder's response | spec §4, §5 | contract; verifiable presentation |
| answer kind: Supported, Exact | Supported: sound, every returned solution is in `⟦Q⟧_D` (Li et al.: correctness; zkRDF: soundness). Exact: sound and complete, equal to `⟦Q⟧_D` with multiplicities | spec §6.1 | answer mode |
| input kind: holder-declared, verifier-agreed | Holder chose *D*; verifier accepted *D*'s commitment before the request | spec §6.2 | authority |
| dataset commitment | Salted digest fixing *D* | spec §2 | anchor |
| statement; public input; witness | Public values a proof is about (ZKProof: instance); one of them; private input | spec §2; ZKProof §1.2 | claim, obligation |
| request digest | SHA-256 of the JCS-encoded request | spec §3 | request binding |
| challenge; audience; request validity period | Fresh random bytes; verifier identifier; `notBefore` to `notAfter` | spec §4 | nonce (except OpenID4VP's `nonce`), session, window |
| limits; parameters | Request size limits; proof-method settings | spec §4 | bounds |
| proof method | Identifier and version fixing a verification key or image ID | spec §9 | bare "method" |
| accept; mark as answered | Final decision; atomic one-time step before it | spec §8 | consume the challenge |
| **Proofs and the zkVM** | | | |
| proof; proof system | Proof-method output; the scheme (STARK, UltraHonk, Groth16) | spec §9 | ZKP |
| relation; circuit; verification key | What is proved; its Noir encoding; its checking key | ZKProof §1.2; spec §9 | artifact |
| zkVM; guest; host | Provable virtual machine; proved program; untrusted program running it | RISC Zero | native |
| receipt; journal; image ID; dev mode | RISC Zero proof; public output; guest identifier; fake-receipt mode | RISC Zero | genuine, pin |
| publicly verifiable; designated verifier | Anyone can check; only the participating verifier can | ZKProof §1.5 | transferable (guide §7) |
| attestation | TEE-signed report on a measured program | spec §9 | |
| **Signature modes and security properties** | | | |
| signature mode: hidden, revealed, disclosed | Hidden: the proof checks every signature; the verifier sees none. Revealed: the verifier checks the shown signatures; the proof ties *D* to their signed data. Disclosed: the verifier gets the credentials and evaluates *Q* | spec §6.3 | signatures-in-proof |
| unforgeability (unverified) | No valid signature without the private key | spec §10.2 | integrity |
| proof soundness; knowledge soundness | Accepted proof implies the statement (and a known witness), except with negligible probability | ZKProof §1.6 | bare "soundness" |
| zero-knowledge | Proof reveals only that the statement holds | ZKProof §1.6 | "private" |
| unlinkability | Presentations cannot be linked through shared credentials | VC DM 2.0 §2 | linkage |
| hiding; binding; salted | Commitment reveals nothing and opens to one value only; digest includes a random salt | Thaler §12.3; spec §2 | blinded |
| classical; post-quantum (unverified) | Secure against an adversary without, or with, a large quantum computer | spec §10.2 | quantum-safe |
| **Evaluation** | | | |
| experiment | Runs at one source version | – | campaign |
| test case | Request, input, expected outcome | – | declared case |
| negative test (unverified) | Altered or replayed presentation the verifier must reject | – | control |
| execution without proving; cycles (unverified) | Guest run in the RISC Zero executor; its cost | RISC Zero | executed guest |
| preliminary measurement | Timing with conditions stated once | – | indicative |
| baseline circuit; public-triple circuit | Noir circuits compared in §6.5 | ours | V1, V4 |

**Soundness and completeness.** "Sound" and "complete" describe results only in the definitions of Supported and Exact; elsewhere write Supported, Exact or "complete over *D*". For proof systems write "proof soundness" or "knowledge soundness", never bare "soundness" or "completeness".

**Coinages kept** (three or more uses, no established name): the spec terms (decided); *public-triple rule* and *public triple* (when a triple computable from the query and the answer may be a public input; zkRDF names no rule); *Merkle-root cryptosuite* (our family; the identifier names one variant); *zkVM evaluator* (the SPARQL evaluator as a RISC Zero guest); *baseline* and *public-triple circuit* (the Noir circuits of §6.5, without and with the query triple as a public input).

## 4. Notation

| Symbol | Meaning |
|---|---|
| *Q*; *D* | The query; the input dataset |
| `⟦Q⟧_D` | Multiset of solution mappings of *Q* over *D*; say once that it is eval(D(G), P) of SPARQL 1.1 §18.6 (*P*: algebra expression of *Q*; *G*: default graph of *D*) |
| μ; dom(μ) | A solution mapping; its domain |
| *t*; μ(*t*) | A triple pattern; its instance under μ (SPARQL 1.1 §18.3, unverified) |
| `card[⟦Q⟧_D](μ)` | Multiplicity of μ (SPARQL 1.1 §18.3), only if a formula needs it |
| *n* | Credentials per input dataset |
| *x*, *w*, *R* | Statement, witness, relation (ZKProof §1.2); appendix only |

Typst: `$[| Q |]_D$`, `$mu(t)$`.

**Remove:** `⟦P⟧_D`, `C = ⟨m, o, q, f, a, s, e, d, b, t⟩`, `c`, `k`, `r`, `π`, `h(C)`, `E`, `K`, `ρ`, `p_C`, `σ`, `Com`, `Map`, `Anc`, `Src`, `Scp`, `Bnd`, `Ans`, `supp`, `⊑`, `tμ`, `V`. "Design argument" with A1 to A5 becomes "Proposition 1 (informal)": numbered assumptions and a proof sketch.

**Adding a symbol:** only if used twice or more, clearer than words, defined in words at first use, distinct from the letters above (and Ω, P, G), and replaceable by a noun; then list it here.

## 5. Replacement list

Counts: main body/appendix, from the draft audit; "n/c": a GPT-review term the audit did not count.

- campaign (8/13); run, noun (~20/~25) → experiment
- declared case (4/6); test ids → test case
- evidence inspection; second-party inspected (7/3) → guide §6; never named
- evidence level (2/0, +2); executed native; executed guest → column "Strongest result": tests outside the zkVM, executed without proving, proof verified
- genuine (23/19) → receipt (guide §6); "genuine" only against fake receipts
- controls; control records (17/18) → negative tests
- gate (3/18); hosted (3/10); frozen (2/3); retained (3/10); inventory (3/6) → describe, or delete
- "at a later source" (5/11); commit hashes (11/31) → artifact table only
- paired harness (3/1); arms (3/0); work counters (1/0); equal-guarantee comparison (1/0); specialisation (1/3) → paired comparison; circuits; check counts; same statement
- indicative (4/0) → preliminary
- query contract (5/0); contract (42/10); stored contract (5/2); contract tuple (0/1) → query request
- input authority (8/0); authority (24/11); provenance field (3/6); authority-relative (n/c) → input kind
- `HolderDeclared` (0/8); `HolderSelected…` (2/0); `VerifierAgreed` (0/9) → holder-declared; verifier-agreed
- anchor *k* (21/5) → dataset commitment
- method (29/29); query-proof method (4/2); method descriptor (4/4); descriptor digest (6/12) → proof method; image ID; verification key
- signature suite (4/2) → cryptosuite
- key table (8/1); key policy (2/1); issuer, verification-method and key table (n/c); source evidence *e* (2/5); `None` (1/0) → issuer keys; no issuer keys
- scope *s* (5/14); graph catalog *K* (1/4) → input dataset
- result form *f* (2/2) → query form
- registry (4/4); capability tuple (1/2) → the request's proof methods
- adapter (9/26); protocol adapter (3/3); generic authenticated adapter (5/2); authenticated extension (1/2) → our verifier; zkVM evaluator with signature checks
- V1, V3, V4, V5 (4/3, 0/6, 9/5, 11/31); request versions, exact evaluator (3/4); public-pattern, baseline, Noir relation (10/1) → baseline circuit; zkVM evaluator; public-triple circuit
- pin; pinned image; guest pin; artifact (8/16; 2/7) → image ID
- request binding (5/2) → request digest
- row bound (7/6); capacity profile K1, K2 (2/1) → `maxResultRows`; one- or two-credential circuit
- nonce (2/8); consume the challenge (16/8) → challenge; mark the request as answered
- answer mode (3/3); supported/exact mode (5/2); exact path (3/0); `SelectedResults` (0/4) → answer kind
- supported (21/4); exact (52/24) → Supported; Exact
- canonical graph equality (1/1) → isomorphism
- blank-node closure policy (2/3) → delete with `DESCRIBE`
- union of credential graphs (2/0) → merge
- bag; set results (20/21) → multiset; `SELECT DISTINCT`
- release (39/7); authorised release (6/0); release condition (4/0); acceptance (6/1) → return; guide §6; accept
- answer-derived (7/0) → computable from the query and the answer
- public eligibility; public-eligible (4/1) → may be a public input
- conservative public-data rule (3/0); public-data rule (7/0); the rule (5/1) → public-triple rule
- positive pattern (5/3) → monotone query
- obligation (10/1); secret work; membership and equality work (1/0 each) → what the proof must still show
- subject linkage (n/c) → the join on `?person`
- pre-output or abort privacy (3/0); disclosure analysis (3/0); partial-release observables (1/0) → what the verifier learns
- design argument (3/5) → Proposition (informal)
- binding, bound, bounded (14/19; 14/14; 10/5) → "bind" for proofs, commitments, holder binding only; else limit, range proof (unverified)
- native (11/29); native or V5 model (4/8); native oracle (0/1) → outside the zkVM; BBS proofs; evaluator outside the zkVM
- replay as re-execution (in 3/15); native replay jobs (n/c) → re-execution
- evidence (21/23); evidence account (1/0) → results; evaluation
- statement as a bank document (6/0); host as a machine (in 2/7) → bank statement; machine
- accept for keys or inputs (in 35/27); canonical outside RDFC-1.0 (in 15/14) → lists, agreed; delete
- protocol verifier; relying party; presenter → verifier; holder
- `HolderSelectedAuthenticated`, `VerifierAgreedAuthenticated`, `CapacityExceeded`, `Succinct` (1–2 each); `risc0-r0vm 3.0.6` (1/0); `NotRequested`, `all_defined_cases_run`, `dev_mode`, `Halted(0)` (appendix) → words; versions in the artifact table
- typed (2/1); strict (1/0); wire forms (0/1); admission (0/2) → delete; explain; serialisations; request checks
- positive-existence statement (0/1); membership-indifferent modifiers (0/1) → true `ASK`; solution sequence modifiers
- lane, tiers, invariant downgrade, audit gates, integration design (appendix); `sq-qhy4` and repository paths (footer) → delete
- the first prototype in this line (2/0) → Wright [wright25dc]
- an input it agreed independently (1/0) → an input whose commitment it accepted in advance
- override the open-world reading (n/c) → an Exact answer shows absence only within *D*

## 6. Fixed phrasings

Each sentence appears once, in the section named; elsewhere point to that section or say nothing.

- **Coverage** (§6.1; the abstract may add "over synthetic credentials"): "All credentials in our experiments are synthetic and signed with test keys."
- **Dev mode** (§6.1): "We produced every receipt with RISC Zero dev mode off and verified it against the guest's image ID."
- **Internal re-check** (§6.1; never name the check): "For every experiment not marked † in Table 2, we later re-hashed its source files, guest binaries and receipts, and compared the hashes and recorded test outcomes with the experiment's record; this check re-ran no proof verification."
- **No external audit** (§8.5 only): "No part of the prototype has had an external security audit."
- **Timing of public triples** (§5.3): "We propose one condition, which the prototype does not enforce: a public triple may reach a party only with or after the answer it is computed from."
- **Holder-declared scope** (§3.3): "With a holder-declared input, an Exact answer covers only the credentials the holder chose to include."
- **Unchecked properties** (§8.2): "The prototype does not check credential status, holder binding or credential validity periods."

## 7. Open questions for the author

1. **Merge or named graphs?** Spec §2's "union … with blank nodes kept apart" is an RDF merge (unverified); VC DM 2.0 §5.12 puts each credential in a named graph. Say "merge"; justify once?
2. **Exact with `LIMIT` or `OFFSET` but no `ORDER BY`, or with `REDUCED`:** SPARQL leaves the solutions (§15.4) or multiplicities unspecified, so spec §6.1's "exactly the multiset" is undefined. Exclude them?
3. **Supported for which queries?** Spec §4 allows any `SELECT` or `ASK`; the paper only monotone ones. State this in the spec?
4. **Disclosure policy:** condition (iii) of the public-triple rule needs a request member permitting disclosure; spec §4 has none. Drop (iii)?
5. **Merkle-root cryptosuite:** is `schnorr-poseidon2-merkle-2026` final, and specified where? Spec §2 says it signs "the root" over "RDF terms"; the code signs a value derived from root, quad count, depth and proof-configuration digest, one leaf per quad.
6. **Spec wording to align:** "transferable" (§9; ZKProof §1.5: "publicly verifiable"), "positive basic graph patterns" (§9), "a valid proof" for a signature (§6.2), "artifact" and "pins" (§4, §9), "image identifier" (§8).
7. **Unpublished ISWC 2025 manuscript** (the earlier results-only interface): cite anonymously, or describe uncited?

## 8. Sources

**Writing.** Gopen & Swan 1990, https://www.jstor.org/stable/29774235 · Mensh & Kording 2017, https://doi.org/10.1371/journal.pcbi.1005619 · Peyton Jones, https://www.microsoft.com/en-us/research/academic-program/write-great-research-paper/ · Knuth et al. 1989, https://jmlr.csail.mit.edu/reviewing-papers/knuth_mathematical_writing.pdf · Halmos 1970, https://www-users.cse.umn.edu/~cberkesc/5385/Spring2018/halmosWrite.pdf · Widom, https://cs.stanford.edu/people/widom/paper-writing.html · Ernst, https://homes.cs.washington.edu/~mernst/advice/write-technical-paper.html · Google Tech Writing, https://developers.google.com/tech-writing/one · Orwell 1946, https://www.orwellfoundation.com/the-orwell-foundation/orwell/essays-and-other-works/politics-and-the-english-language/ · Digital.gov, https://digital.gov/guides/plain-language/writing · GOV.UK, https://guidance.publishing.service.gov.uk/writing-to-gov-uk-standards/writing-guidelines/clear-language/ · Pinker 2014, https://www.chronicle.com/article/why-academics-stink-at-writing/

**Specifications.** SPARQL 1.1 Query, https://www.w3.org/TR/sparql11-query/ · RDF 1.1 Concepts, https://www.w3.org/TR/rdf11-concepts/ · RDFC-1.0, https://www.w3.org/TR/rdf-canon/ · VC DM 2.0, https://www.w3.org/TR/vc-data-model-2.0/ · VC-DI, https://www.w3.org/TR/vc-data-integrity/ · EdDSA, https://www.w3.org/TR/vc-di-eddsa/ · VC-DI-ECDSA, https://www.w3.org/TR/vc-di-ecdsa/ · BBS, https://www.w3.org/TR/vc-di-bbs/ · OpenID4VP 1.0, https://openid.net/specs/openid-4-verifiable-presentations-1_0.html · RISC Zero docs, https://dev.risczero.com/terminology

**Literature.** ZKProof Reference, https://docs.zkproof.org/reference.pdf · Thaler, https://people.cs.georgetown.edu/jthaler/ProofsArgsAndZK.pdf · zkRDF (Braun, Wright & Käfer, ESWC 2026), https://publikationen.bibliothek.kit.edu/1000193675 · Wright (ISWC 2025 Companion), https://ceur-ws.org/Vol-4085/paper19.pdf · Li et al. (SIGMOD 2006), https://www.cs.bu.edu/~reyzin/papers/auth-db.pdf
