# Language guide: "Answers Instead of Documents"

For `zksparql-architecture.typ`. "Spec §n": *Zero-Knowledge SPARQL Answers* (draft, sparq-org/sparq#6786 at `c643c7d`). "Suite draft §n": the Merkle-root cryptosuite specification (sparq-org/sparq#6789). Bare "§n": the paper's current numbering. "Guide §n": this file. "Decision n": guide §7.

## 1. Audience and goal

Readers: ESWC reviewers, expert in RDF and SPARQL, familiar with verifiable credentials, new to zero-knowledge proofs. Aim: one argument, not a project log, in established terms (the spec's for our own concepts), with formulas only where clearer than prose.

## 2. Style rules

Rules 1 to 6 are firm. Rules 7 to 16 are strong preferences, broken only where precision needs it.

1. **One term per concept, one concept per term** (guide §3), defined at or before first use. Keep each distinction before simplifying.
2. **Established term before coinage; describe rather than name.** Coin only for a concept used three or more times without an established name.
3. **No internal labels in prose:** commit hashes, test ids, enum and type names, V1 to V5, campaign, gate, hosted, frozen, retained, inventory, bead, `zkp-*`, repository paths. Versions, hashes and proof-method identifiers go in one artifact table.
4. **Every empirical number comes from the evidence lookups** (`#headline(key)`; `#ev(key)` for preliminary timings). Compute derived numbers (ratios, totals) in the evidence data, never in prose. Never add counts from different experiments.
5. **Mark each claim as measured, argued or proposed.** Arguments are prose with numbered assumptions.
6. **Cite our own prior work in the third person** (zkRDF; the ISWC 2025 doctoral-consortium paper).
7. **One full explanation of each caveat, in its home** (guide §6). Elsewhere, a brief qualification only where omission would mislead. The abstract and the conclusion each stand alone.
8. **Hedge with a number or a condition:** "Preliminary: [n] runs on a shared machine", not "Indicative development measurement, not the basis of any claim".
9. **Results, not project history:** "We executed the guest [n] times without proving."
10. **Short sentences, one idea each:** aim for under 25 words; split those over 40.
11. **Active voice, named actor, subject next to verb, old information before new:** "For the returned-payment question, we produced one receipt."
12. **Verbs, not nominalisations; no stacks of three or more nouns,** except established names: "the verifier checks everything, then marks the request as answered", not "complete validation before the verifier consumes its challenge".
13. **Formula or prose, whichever this reader parses faster:** a formula for a short exact condition, a table for a structure, a list for a procedure. Define symbols in words; no ∀, ∃ or ⇒ in text, and no symbol at a sentence start.
14. **Running example:** introduce a definition with the lender question that motivates it, not with all three.
15. **Clear referents:** "In `?p a ex:Employee OPTIONAL { ?p ex:email ?e }`, a row with `?e` unbound shows that no email triple for `?p` matched. Only an Exact answer can show such an absence."
16. **Form:** no meta-commentary; present tense for the design, past for experiments; British *-ise* spelling, except in titles; no em-dashes; code font for SPARQL keywords, JSON members, IRIs and cryptosuite identifiers; capitalised Supported and Exact; few acronyms; never "VC" (except in titles) or "ZKP".

## 3. Vocabulary

**Established terms,** used as their sources define them:

- *RDF and SPARQL:* RDF dataset, default, named and active graph (SPARQL 1.1 §13, §18.1); RDF merge (RDF 1.1 Semantics §4.1); blank node scope (RDF 1.1 Concepts §3.4); canonical N-Quads form, a serialisation (RDFC-1.0 §3.1); isomorphic, a relation (Concepts §3.6, §4.1); term equality (§3.3); solution mapping μ, multiset, solution sequence (SPARQL 1.1 §18.1.8, §18.3); projection (§15.2).
- *Credentials:* claim, credential (short for verifiable credential), subject (not necessarily a person), issuer, holder, verifier, verification, validation, unlinkable disclosure (VC DM 2.0 §2); validity period (§4.9); credential status (§4.10); cryptosuite, Data Integrity proof, verification method, controlled identifier document (VC-DI §1.4).
- *Proofs:* statement, instance (say "public input"), witness, relation (ZKProof §1.2); publicly verifiable, designated verifier (§1.5); completeness, soundness, zero knowledge (§1.6); hiding, binding (Thaler §12.3); host, host program, guest program, receipt, journal, seal, image ID, executor, cycle (RISC Zero).
- *Spec:* its terms (spec §2 to §9), exactly as written. An answer presentation is not a W3C verifiable presentation.

**Rules for this paper**

*Data and queries*
- **Dataset construction** (decision 1): the method names its layout, since a query written for the merged default graph does not match one named graph per credential. A result's blank node labels mean nothing outside it.
- **Equality:** pattern matching uses term equality. `"30000"^^xsd:integer = "030000"^^xsd:integer` holds by value only (SPARQL 1.1 §17.4.1.7).
- **Row:** a solution in a returned `SELECT` result; definitions say "solution".
- **Monotone query** (spec §2): apart from a top-level `ORDER BY`, `OFFSET` and `LIMIT`, a `SELECT` or `ASK` query with only basic graph patterns, group graph patterns, `UNION`, `GRAPH`, `VALUES`, projection, `DISTINCT`, and `FILTER` or `BIND` without `EXISTS` or `NOT EXISTS`. Supported answers need one (decision 3): the holder may leave credentials out of *D*, so a Supported row must stay a solution as the input grows, and a monotone query's solutions over *D* are solutions over every dataset containing *D*. Over a fixed *D*, a row of any query is a solution; monotonicity protects it against omitted credentials. A count over a subset is that subset's exact count, never "supported".

*Credentials and signatures*
- **Signature or proof:** a cryptosuite specifies algorithms for creating and verifying Data Integrity proofs, derived ones included. The issuer's Data Integrity proof is here a signature with its proof configuration (the proof without `proofValue`); say "signature" for modes and checks. "Proof" alone is the holder's query proof.
- **Keys:** the verification method (here a public key), its identifier (`verificationMethod`) and the key bytes (`publicKeyMultibase`) differ. Data Integrity authorises an issuer's key through a verification relationship (`assertionMethod`) in the issuer's controlled identifier document (VC-DI §2.6); our requests list the accepted keys in `issuers` instead.
- **`eddsa-rdfc-2022`:** Ed25519 signs the SHA-256 hash of the canonical proof configuration followed by that of the canonical document (EdDSA §3.2). It is unsalted, so whoever sees the hashes can test a guessed document.
- **`ecdsa-rdfc-2019`:** the same with ECDSA, P-256 with SHA-256 or P-384 with SHA-384. **`mldsa44-rdfc-2024`:** ML-DSA-44 with SHA-256, post-quantum; First Public Working Draft of Quantum-Resistant Cryptosuites v1.0, 16 June 2026.
- **`bbs-2023`, `ecdsa-sd-2023`:** the holder derives from the issuer's base proof a proof disclosing selected claims. Only `bbs-2023` derived proofs are unlinkable (VC-DI-ECDSA §5.1).
- **Our Merkle-based cryptosuites** sign the digest of decision 5: `eddsa-sha256-merkle-2026` (SHA-256 tree, Ed25519), verified in the zkVM, and `schnorr-poseidon2-merkle-2026` (draft identifier; Poseidon2 tree, Schnorr over Baby Jubjub), the member for Noir. Neither is post-quantum (suite draft §1.2, §3.5, §5).
- **Verification:** we verify signatures only. Credential verification also checks that a credential is current, status included; validation checks the verifier's business requirements.
- **Three separate checks:** credential status (revoked or suspended); credential validity period (`validFrom` to `validUntil`, unlike a proof's `created` and `expires` or a key's validity); holder binding (the party presenting is the subject, or controls a key bound to the credential).

*Request and answer*
- **Supported:** every returned solution is a solution: for `SELECT`, a non-empty result without duplicates, each row in `⟦Q'⟧_D`; for `ASK`, `true`.
- **Exact:** the complete SPARQL 1.1 result of *Q* over *D*: for `SELECT`, the solution sequence with duplicates, ordered as decision 2 fixes; for `ASK`, `true` exactly when *Q*'s pattern has a solution over *D*; for `CONSTRUCT`, the constructed graph, up to isomorphism. Where SPARQL leaves a choice open, say "not uniquely determined by SPARQL", not "undefined". Neither answer kind says who signed *D*.
- **Input kinds:** agreement shows that two commitments are equal, not that *D* is complete in the world. If a method computes the dataset commitment per signature mode, an agreed commitment fixes the mode.
- **Signature modes:** hidden (the proof checks every signature; the verifier sees none); revealed (the verifier checks the shown signatures; the proof shows that *D* is exactly their signed data); disclosed (the verifier receives credentials or derived proofs and evaluates *Q* itself).
- **Public-input rule** (decision 4): substituting a returned solution into a query that is a single basic graph pattern gives triples the verifier can compute. Under `UNION` the matched branch is not computable, so its triple does not qualify. The proof must still show that each triple supplied as a public input is in *D* and signed.
- **Disclose, deliver, accept, mark as answered:** sending the presentation discloses the answer; delivery is its arrival, which a transport may split; the verifier accepts after every check; marking the request as answered, atomically and just before acceptance, prevents replay. A rejected presentation has still disclosed its answer.

*Proofs and the zkVM*
- **Proof method:** an identifier (IRI) and version, whose evidence is a zero-knowledge proof, a proof that is not zero-knowledge, a TEE attestation or disclosed credentials; a new version is a new method. The verifier checks evidence with a verification key from its own configuration (`verificationKey`: a circuit's verification key, a zkVM image ID or an attestation root key). `parameters` are settings the statement depends on. The proof system is the scheme: RISC Zero's STARK, optionally wrapped in Groth16, or UltraHonk.
- **Statement:** that some witness, with the public inputs, satisfies the relation. Public inputs: request digest, answer kind, input kind, dataset commitment, result, signature mode and, in the revealed mode, signed messages, each passed directly or bound by one digest. Witness: credentials, salts and, in the hidden mode, signatures. A circuit implements a relation as constraints; ours are in Noir.
- **zkVM:** the host is the machine, the host program the untrusted code that runs the guest program and supplies its inputs. A receipt is a claim (journal, image ID, exit status) plus a seal attesting to it. Development mode makes fake receipts, the only contrast for "genuine". Report cycles, deterministic and counted without proving, separately from proving time.
- **The SPARQL evaluator running in the zkVM:** our guest program, with fixed capacity limits; then "the evaluator", "with signature checks" for the version that verifies issuer signatures.
- **Designated-verifier:** only a verifier holding secret verification information can check the proof, interactively (the proposed VOLE method) or not. Non-transferability is a separate property.

*Security properties*
- **Knowledge soundness:** an efficient extractor obtains a valid witness from any prover that convinces the verifier. **Zero-knowledge:** the proof reveals nothing about the witness beyond the statement. For proof systems, never bare "soundness" or "completeness"; "proof-system completeness" is fine.
- **Salted:** a salted digest is binding if the hash is collision-resistant, and hiding only if the salt is random and secret and the hash is modelled as a random oracle. Say whether a property holds computationally or statistically.
- **Unforgeability:** existential unforgeability under chosen-message attack: given signatures on messages of its choice, an attacker cannot feasibly sign any other message.
- **Unlinkability:** presentations of one credential cannot be correlated beyond what they disclose. Name the mechanism and the disclosed data: `bbs-2023` derived proofs are unlinkable, but revealed signatures, a reused dataset commitment or an identifying result link presentations.
- **Classical, post-quantum:** secure against an adversary without, or with, a large quantum computer; name the property. Ed25519, ECDSA and Schnorr signatures are unforgeable only classically, ML-DSA-44 also post-quantum; a Groth16 receipt is sound only classically.

*Evaluation*
- **Experiment, run, repetition:** runs of one build answering one question; one execution of one test case; a repeated run, for timing.
- **Test case, negative test, positive control:** request, input and expected outcome; an altered, replayed or misdirected presentation the verifier must reject; the unaltered one it must accept.
- **Preliminary measurement:** a timing with its conditions stated once.
- **Baseline and public-triple circuits** (§6.5): the matched triple is part of the witness in the first, a public input in the second. Both are experimental labels, defined once.

**Results:** "sound" and "complete" appear only when relating Supported and Exact to prior work (zkRDF: soundness; Li et al.: correctness, completeness).

**Beyond spec version 1:** say so; never rename a difference away. The prototype proved `DESCRIBE` answers, which version 1 excludes; state once which triples a `DESCRIBE` returns. The paper's dataset scope (named graphs, `FROM`, `FROM NAMED`) exceeds version 1, which forbids dataset clauses. Its disclosure policy *d* has no spec member: drop it.

## 4. Notation

- *Q*; *D*: the request's query; the input dataset.
- `⟦Q⟧_D`: the SPARQL 1.1 result of *Q* over *D*, with its solution modifiers and query form applied. Say once that its algebra part is eval(D(G), P) (SPARQL 1.1 §18.6; *P*: the algebra expression of *Q*'s pattern; *G*: the default graph of *D*). Spec §2 calls it a multiset of solution mappings, so state the `ASK` case in words.
- *Q'*: *Q* without its top-level `ORDER BY`, `OFFSET` and `LIMIT`; only in the Supported definition.
- μ: a solution mapping, as in the running example's μ = {?person ↦ ex:alice}.
- μ(*t*): the triple obtained by replacing each variable of triple pattern *t* by its value under μ (SPARQL 1.1 §18.3.1), only for a *t* without blank nodes whose variables μ all binds. Drop it if §5 can say this in words.

Typst: `$[| Q |]_D$`, `$Q'$`, `$mu(t)$`.

**Remove:** `⟦P⟧_D`, `C = ⟨m, o, q, f, a, s, e, d, b, t⟩`, `c`, `k`, `r`, `π`, `h(C)`, `E`, `K`, `ρ`, `p_C`, `σ`, `Com`, `Map`, `Anc`, `Src`, `Scp`, `Bnd`, `Ans`, `supp`, `⊑`, `tμ`, `V`; *n*, dom(μ) and `card` unless a formula needs them. No "Proposition" or "Design argument": each argument becomes short appendix prose with numbered assumptions (A1, A2, …), or goes.

**Adding a symbol:** only if used twice or more, clearer than words, defined in words at first use, distinct from those above (and Ω, P, G), and replaceable by a noun.

## 5. Replacement list

Old term → use.

*Evaluation*
- campaign → experiment; "run" stays, for one execution of one test case
- declared case → test case
- evidence inspection; second-party inspected → the internal check (guide §6), unnamed
- evidence level; executed native; executed guest → column "Strongest result": tests outside the zkVM; executed without proving; proof verified
- genuine → receipt, except against fake receipts
- controls → negative tests; positive control
- paired harness; arms; work counters; equal-guarantee comparison; specialisation → paired comparison; circuits; check counts; same statement
- indicative → preliminary
- evidence, for outcomes → results
- replay, meaning re-execution → re-execution

*Request and answer*
- contract; query contract; contract tuple → query request; stored request
- authority; input authority; provenance field → input kind
- `HolderDeclared`; `HolderSelected…`; `VerifierAgreed` → holder-declared; verifier-agreed
- anchor *k* → the agreed dataset commitment
- answer mode; exact path; `SelectedResults`; supported; exact → answer kind; Supported; Exact
- scope *s*; graph catalog *K* → dataset construction
- result form *f* → query form
- request binding → request digest
- nonce; consume the challenge → challenge; mark the request as answered
- row bound; capacity profile K1, K2 → `maxResultRows`; one- or two-credential circuit
- release; authorised release → disclose, never "return"
- release condition → the timing proposal (guide §6)
- validity window → request or credential validity period

*Proofs, methods and signatures*
- method; query-proof method → proof method
- method descriptor; descriptor digest → the request's proof-method entry; its digest, never "image ID"
- registry; capability tuple → the proof methods the verifier accepts
- pin; pinned image; guest pin; artifact → image ID; verification key; guest binary ("artifact" only for the research artifact)
- signature suite → cryptosuite
- key table; key policy; source evidence *e*; `None` → the request's issuer keys; no issuer keys
- adapter → our holder and verifier services
- authenticated extension; V5; exact evaluator → the evaluator, with or without signature checks
- V1; V4; public-pattern or Noir relation → baseline circuit; public-triple circuit
- zkVM evaluator → the SPARQL evaluator running in the zkVM, then "the evaluator"
- Merkle-root cryptosuite → our Merkle-based cryptosuites
- transferable; image identifier → publicly verifiable; image ID
- "proof", for an issuer's signature → Data Integrity proof, or signature
- signatures-in-proof; quantum-safe; blinded → hidden mode; post-quantum; salted or hiding

*Semantics and disclosure*
- canonical graph equality; union of credential graphs → isomorphism; RDF merge
- bag; set results → multiset; `SELECT DISTINCT`
- positive pattern → monotone query
- blank-node closure policy → describe once, with the `DESCRIBE` results
- answer-derived; public-eligible → computable from the stored request and the result; may be a public input
- public-data rule; the rule → public-input rule
- public triple → triple supplied as a public input
- obligation; secret work → what the proof must still show
- subject linkage → the join on `?person`
- pre-output or abort privacy; disclosure analysis → what the verifier learns, before an answer or on abort
- override the open-world reading → an Exact answer shows absence only within *D*

*Words with several senses*
- binding, bound, bounded → "bind" for proofs, commitments and holder binding; else size limit, range proof or capacity limit
- native → outside the zkVM; BBS+ proofs; reference evaluator
- accept, for keys → the request lists keys
- canonical, outside RDFC-1.0 → only for a method's canonical encoding
- statement, for a bank document; record, for a credential → bank statement; credential
- relying party; presenter → verifier; holder
- the first prototype in this line → Wright [wright25dc]

## 6. Fixed phrasings

Each sentence appears in full once, in its home; elsewhere at most a brief qualification (rule 7).

- **Coverage** (§6.2, evaluation method; the abstract may say "over synthetic credentials"): "We used only synthetic credentials signed with test keys."
- **Development mode** (§6.2): "We generated all receipts with development mode disabled and verified them against the expected guest image ID."
- **Internal check** (§6.2; never name the check): "For experiments without † in Table 2, we recomputed hashes of source files, guest binaries and receipts. We compared the hashes and recorded test outcomes with the archived records. We did not verify the proofs again."
- **Audit** (§8.5, limitations): "The prototype has not undergone an external security audit."
- **Disclosure timing** (§5.4, when values may become public): "We propose that a party receive these public inputs no earlier than the answer from which it can compute them. The prototype does not enforce this condition."
- **Holder-declared input** (§3.3, input kinds): "For a holder-declared input, an Exact answer covers only the included credentials."
- **Unchecked properties** (§8.2, status and holder binding): "The SPARQL evaluator running in the zkVM does not check credential status, holder binding or credential validity periods."

## 7. Decisions and open questions

**Decided** in the spec (#6786 at `c643c7d`, 10 October 2026):

1. **Input dataset:** the RDF merge of the credentials' graphs as the default graph, or one named graph per credential if the method says so.
2. **Exact slices:** before each `OFFSET` and `LIMIT`, subqueries included, solutions follow that level's `ORDER BY`, then the method's canonical encoding; no `REDUCED`.
3. **Supported:** monotone queries only; a `SELECT` row is in `⟦Q'⟧_D`.
4. **Public inputs:** a method may make public any value the verifier can compute from its stored request and the result; no request member permits disclosure.
5. **Merkle-based cryptosuites** sign a digest of the suite identifier, a 32-byte salt, the quad count, the Merkle root (one leaf per canonical quad) and the proof-configuration digest.
6. **Wording:** "publicly verifiable", "designated-verifier", "monotone query", "a Data Integrity proof (a signature)", `verificationKey` (formerly `artifact`), "image ID"; no "pins".

**Open:**

1. **Unpublished ISWC 2025 manuscript** (the earlier results-only interface): cite it anonymously, or describe it uncited?
2. **Poseidon2 message:** suite draft §5, unchanged at the head of #6789, signs `P(3, salt, n, root, c1, c0)`, without the suite identifier of decision 5. Align one of them before the paper describes this member.
3. **`SAMPLE`, `GROUP_CONCAT`:** SPARQL does not uniquely determine their results, and decision 2 does not fix them. Forbid them in Exact queries, or order their input by the canonical encoding?

## 8. Sources

**Writing** (links in this file's first version, commit `3bdf74ae5`): Gopen & Swan 1990; Mensh & Kording 2017; Peyton Jones; Knuth et al. 1989; Halmos 1970; Widom; Ernst; Google Tech Writing; Orwell 1946; Digital.gov; GOV.UK; Pinker 2014.

**Specifications.** SPARQL 1.1 Query, https://www.w3.org/TR/sparql11-query/ · RDF 1.1 Concepts, https://www.w3.org/TR/rdf11-concepts/ · RDF 1.1 Semantics, https://www.w3.org/TR/rdf11-mt/ · RDFC-1.0, https://www.w3.org/TR/rdf-canon/ · VC DM 2.0, https://www.w3.org/TR/vc-data-model-2.0/ · VC-DI, https://www.w3.org/TR/vc-data-integrity/ · EdDSA, https://www.w3.org/TR/vc-di-eddsa/ · VC-DI-ECDSA, https://www.w3.org/TR/vc-di-ecdsa/ · BBS, https://www.w3.org/TR/vc-di-bbs/ · Quantum-Resistant Cryptosuites, https://www.w3.org/TR/vc-di-quantum-resistant/ · OpenID4VP 1.0, https://openid.net/specs/openid-4-verifiable-presentations-1_0.html · RISC Zero, https://dev.risczero.com/terminology

**Literature.** ZKProof Reference, https://docs.zkproof.org/reference.pdf · Thaler, https://people.cs.georgetown.edu/jthaler/ProofsArgsAndZK.pdf · Goldwasser, Micali & Rivest, *SIAM J. Comput.* 17(2), 1988 (unforgeability) · zkRDF (Braun, Wright & Käfer, ESWC 2026), https://publikationen.bibliothek.kit.edu/1000193675 · Wright (ISWC 2025 Companion), https://ceur-ws.org/Vol-4085/paper19.pdf · Li et al. (SIGMOD 2006), https://www.cs.bu.edu/~reyzin/papers/auth-db.pdf
