# Language guide: "Answers Instead of Documents"

For `zksparql-architecture.typ`. "Spec §n": *Zero-Knowledge SPARQL Answers* (draft, sparq-org/sparq#6786 at `c643c7d`). "Suite draft §n": the Merkle-root cryptosuite specification (sparq-org/sparq#6789). Bare "§n": the paper's current numbering. "Guide §n": this file.

## 1. Audience and goal

Readers: ESWC reviewers, expert in RDF and SPARQL, familiar with verifiable credentials, new to zero-knowledge proofs. Aim: one argument, not a project log; established terms, and the spec's for our own concepts; short active sentences; one full explanation of each caveat; formulas only where clearer than prose.

## 2. Style rules

Rules 1 to 6 are firm. Rules 7 to 17 are strong preferences: break one only where following it would cost precision.

1. **One term per concept, one concept per term** (guide §3), defined at or before first use. Keep a distinction before simplifying its wording.
2. **Established term before coinage; describe rather than name.** Coin only for a concept used three or more times that has no established name.
3. **No internal labels in prose** (commit hashes, test ids, enum names, V1 to V5, campaign, gate, bead, `zkp-*`). Versions, hashes and proof-method identifiers go in one artifact table.
4. **Every empirical number comes from the evidence lookups** (`#headline(key)`; `#ev(key)` for preliminary timings). Compute derived numbers (ratios, totals) in the evidence data, never in prose. Never add counts from different experiments. Design constants (32 bytes) are ordinary numerals.
5. **Mark each claim as measured, argued or proposed.** Arguments are prose with numbered assumptions (guide §4).
6. **Cite our own prior work in the third person** (zkRDF; the ISWC 2025 doctoral-consortium paper), for double-blind review.
7. **One full explanation of each caveat, in its home** (guide §6). Elsewhere, add a brief qualification only where omission would mislead. The abstract and the conclusion each stand alone.
8. **Hedge with a number or a condition.** *Bad:* "Indicative development measurement, not the basis of any claim." *Good:* "Preliminary: [n] runs on a shared machine."
9. **Results, not project history.** *Bad:* "A later gate built the V5 guest and executed it directly". *Good:* "We executed the guest [n] times without proving."
10. **Short sentences, one idea each:** aim for under 25 words; split those over 40. Use a numbered list for a sequence.
11. **Active voice, named actor, subject close to verb:** "We report each experiment separately", not "Campaigns are reported separately".
12. **Old information first, new last:** "For the returned-payment question, we produced one receipt", not "At a later source, ⟨hash⟩, the adapter produced 1 genuine receipt for the returned-payment question".
13. **Verbs, not nominalisations; no stacks of three or more nouns,** except established names ("Data Integrity proof"): "the verifier checks everything, then marks the request as answered", not "complete validation before the verifier consumes its challenge".
14. **Formula or prose, whichever this reader parses faster:** a formula for a short exact condition, a table for a structure, a list for a procedure. Define symbols in words. No ∀, ∃ or ⇒ in text and no symbol at a sentence start; each sentence must read with its formula replaced by a noun.
15. **Running example:** introduce a definition with the lender question that motivates it, not with all three questions.
16. **Clear referents:** each "this", "it" and "that" has a noun. *Bad:* "That is an absence claim; nothing is ground, and it needs an exact method." *Good:* "In `?p a ex:Employee OPTIONAL { ?p ex:email ?e }`, a row with `?e` unbound shows that no email triple for `?p` matched. Only an Exact answer can show such an absence."
17. **No meta-commentary** ("we note that", roadmaps). **Tense:** present for the design, past for experiments. **Form:** British *-ise* spelling, except in titles; no em-dashes; code font for SPARQL keywords, JSON members, IRIs and cryptosuite identifiers; capitalised Supported and Exact; few acronyms, each defined once; never "VC" (except in titles) or "ZKP".

## 3. Vocabulary

Every entry's source was checked. Replacements for old terms: guide §5.

**RDF and SPARQL** (RDF 1.1 Concepts, RDF 1.1 Semantics, SPARQL 1.1, RDFC-1.0)

- **RDF dataset; default, named and active graph:** one default graph and zero or more named graphs; the graph a pattern is matched against, which is the default graph except inside `GRAPH`. Concepts §4; SPARQL §13.
- **input dataset *D*; dataset construction:** the dataset the holder evaluates *Q* over; how a proof method builds it from credentials. Default: the RDF merge of the credentials' graphs, as the default graph. A method may instead give each credential its own named graph, as a verifiable presentation does, and must say so: a query written for one layout does not match the other. Spec §2; VC DM 2.0 §5.12.
- **RDF merge:** the union of graphs after renaming blank nodes apart. Semantics §4.1; SPARQL §13.2.
- **blank-node scope:** the credential or result within which a blank node label denotes one node. The merge keeps credentials' blank nodes apart; a result's labels mean nothing outside it. Concepts §3.4; spec §5.1.
- **canonical N-Quads form; isomorphic:** the serialisation RDFC-1.0 outputs; equal up to renaming blank nodes, for graphs or datasets. Isomorphic datasets have one canonical form, but keep the serialisation and the relation apart. RDFC-1.0 §3.1; Concepts §3.6, §4.1.
- **term equality; value equality:** the same RDF term (for a literal: lexical form, datatype and language tag); equal values under a SPARQL operator. `"30000"^^xsd:integer = "030000"^^xsd:integer` is true, but the terms differ, and pattern matching uses term equality. Concepts §3.3; SPARQL §17.4.1.7.
- **solution mapping μ; multiset; solution sequence; row; projection:** variables mapped to RDF terms; solutions with repeats; ordered solutions; one solution in a returned `SELECT` result; the variables a `SELECT` returns, the only ones the verifier sees. SPARQL §18.1.8, §18.3, §15.2.
- **monotone query:** a `SELECT` or `ASK` query that, apart from a top-level `ORDER BY`, `OFFSET` and `LIMIT`, uses only basic graph patterns, group graph patterns, `UNION`, `GRAPH`, `VALUES`, projection, `DISTINCT`, and `FILTER` or `BIND` without `EXISTS` or `NOT EXISTS`. Its solutions over *D* stay solutions over any dataset that contains *D*. Supported answers are allowed only for monotone queries because the holder may leave credentials out of *D*: a Supported row must stay a solution when the input grows. Over a fixed *D*, a row of any query is a solution; monotonicity makes it survive the omitted credentials. Aggregates are not monotone: a count over a subset is not "supported", only the exact count of that subset. Spec §2, §6.1.

**Credentials and signatures** (VC DM 2.0, VC-DI, cryptosuite specifications)

- **credential; verifiable credential; claim; subject:** claims made by an issuer, with metadata; a tamper-evident credential whose authorship can be verified cryptographically ("credential" for short); an assertion about a subject; a thing claims are made about, not necessarily a person. VC DM 2.0 §2.
- **issuer; holder; verifier:** the roles; the holder is the prover. "Holder service" and "verifier service" for software. Spec §2.
- **cryptosuite:** a specification of algorithms for creating and verifying Data Integrity proofs, derived proofs included. VC-DI §1.4.
- **Data Integrity proof; proof configuration; signed message:** the issuer's proof on a credential, here a signature plus its proof configuration (the proof without `proofValue`); the bytes the signature covers. Write "signature" for modes and checks, "Data Integrity proof" where the object matters. VC-DI §2.1; spec §2.
- **verification method; verification-method identifier; issuer public key:** the parameters, here a public key, that verify a Data Integrity proof; the IRI in `verificationMethod`; the key in `publicKeyMultibase`. VC-DI §1.4; spec §4.
- **issuer-key authorisation:** the verifier's decision that a key may sign for an issuer. Data Integrity takes it from a verification relationship (`assertionMethod`) in the issuer's controlled identifier document; our requests instead list the accepted keys in `issuers`. VC-DI §2.6; spec §4.
- **`eddsa-rdfc-2022`:** Ed25519 signs the SHA-256 hash of the canonical proof configuration followed by the SHA-256 hash of the canonical document. Nothing is salted, so whoever sees the hashes can test a guessed document. EdDSA §3.2.
- **`ecdsa-rdfc-2019`; `mldsa44-rdfc-2024`:** the same with ECDSA, using P-256 with SHA-256 or P-384 with SHA-384; ML-DSA-44 with SHA-256, post-quantum, in the First Public Working Draft of Quantum-Resistant Cryptosuites v1.0 (16 June 2026).
- **`bbs-2023`; `ecdsa-sd-2023`:** from the issuer's base proof, the holder derives a proof that discloses selected claims. `bbs-2023` derived proofs are unlinkable; `ecdsa-sd-2023` ones are not (VC-DI-ECDSA §5.1).
- **our Merkle-based cryptosuites:** the issuer signs a digest of the suite identifier, a 32-byte salt, the number of quads, the root of a Merkle tree with one leaf per canonical quad, and the digest of the canonical proof configuration. `eddsa-sha256-merkle-2026` (SHA-256, Ed25519) is verified in the zkVM; `schnorr-poseidon2-merkle-2026` (draft identifier; Poseidon2, Schnorr over Baby Jubjub) is the member for Noir. Neither is post-quantum. Suite draft §1.2, §3.5, §5.
- **signature verification; credential verification; validation:** checking a Data Integrity proof under a key; checking that a credential is authentic and current, status included; checking that it meets the verifier's business requirements. We verify signatures only. VC DM 2.0 §2.
- **credential status:** whether the issuer has revoked or suspended the credential (`credentialStatus`). VC DM 2.0 §4.10.
- **credential validity period:** `validFrom` to `validUntil`; distinct from a Data Integrity proof's `created` and `expires`, and from the validity of the issuer's key. VC DM 2.0 §4.9; VC-DI §2.6.
- **holder binding:** evidence that the party presenting is the subject, or controls a key bound to the credential. Spec §10.1.

**Request and answer** (spec)

- **query request; answer presentation:** the verifier's request; the holder's response, which is not a W3C verifiable presentation. §4, §5.
- **request digest; challenge; audience; request validity period; limits:** SHA-256 of the request's JCS serialisation; 32 fresh random bytes; the verifier's identifier; `notBefore` to `notAfter`; `maxPresentationBytes` and `maxResultRows`. §3, §4.
- **answer kind: Supported, Exact.** *Supported:* every returned solution is a solution. For `SELECT`, a non-empty result without duplicates, each row in `⟦Q'⟧_D` (guide §4); for `ASK`, `true`. Monotone queries only. *Exact:* the complete SPARQL 1.1 result of *Q* over *D*: for `SELECT`, the solution sequence with its duplicates; for `ASK`, `true` exactly when *Q*'s pattern has a solution over *D*; for `CONSTRUCT`, the constructed graph, up to isomorphism. Neither kind says who signed *D*; `issuers` and the signature mode do. §5.1, §6.1.
- **Exact ordering:** before each `OFFSET` and `LIMIT`, subqueries included, solutions follow that level's `ORDER BY`; ties, and all solutions when there is no `ORDER BY`, follow the method's canonical encoding of each solution. Exact queries must not use `REDUCED`. Where SPARQL leaves a choice open, write "not uniquely determined by SPARQL", not "undefined". §4, §6.1.
- **input kind: holder-declared, verifier-agreed:** the holder chose *D* when answering; the verifier accepted *D*'s dataset commitment before sending the request. Agreement shows that the commitments are equal, not that *D* is complete in the world. §6.2.
- **dataset commitment:** a salted digest that fixes *D* without revealing it. A method may compute it differently in each signature mode; a verifier-agreed commitment then fixes the mode. §2, §6.2.
- **signature mode: hidden, revealed, disclosed:** the proof checks every signature and the verifier sees none; the verifier checks the shown signatures and the proof shows that *D* is exactly their signed data; the verifier receives the credentials, or proofs derived from them, and evaluates *Q* itself. §6.3.
- **public-input rule:** a method may make public any value the verifier can compute from its stored request and the result alone. Example: substituting a returned solution into a query that is a single basic graph pattern gives triples the verifier can compute. Under `UNION` the matched branch is not computable, so its triple does not qualify. Call each such triple a "triple supplied as a public input"; the proof must still show that it is in *D* and signed. §7.
- **disclose; delivery; accept; mark as answered:** the holder discloses the answer by sending the presentation; delivery is its arrival, which a transport may split; the verifier accepts after every check; marking the request as answered, one atomic step just before acceptance that fails if already done, prevents replay. A rejected presentation has still disclosed its answer. §8, §10.3.

**Proofs and the zkVM** (spec §2, §9; ZKProof Reference; RISC Zero terminology)

- **query proof; proof method; parameters:** the holder's evidence for the answer, in `proof` ("proof" alone means this); a way of producing and checking it, named by an identifier (IRI) and a version, whose evidence is a zero-knowledge proof, a proof that is not zero-knowledge, a TEE attestation or disclosed credentials; the method's settings that the statement depends on, such as capacity limits. A new version is a new method.
- **verification key; image ID; proof system:** what the verifier holds in its own configuration to check a method's evidence (`verificationKey`: a circuit's verification key, a zkVM image ID or an attestation root key), never taken from the presentation; the identifier of a RISC Zero guest program; the cryptographic scheme, such as RISC Zero's STARK (optionally wrapped in Groth16) or UltraHonk.
- **statement; public input; witness; relation; circuit:** what a proof establishes: that some witness, with the public inputs, satisfies the relation (ZKProof §1.2). Public inputs: request digest, answer kind, input kind, dataset commitment, result, signature mode and, in the revealed mode, signed messages, passed directly or bound by one digest (spec §7). Witness: credentials, salts and, in the hidden mode, signatures. A circuit is constraints that implement a relation; ours are written in Noir.
- **zkVM; guest program; host; host program:** a virtual machine whose executions can be proved; the program proved; the machine the zkVM runs on; the untrusted program that runs the guest and supplies its inputs.
- **receipt; journal; seal; development mode:** RISC Zero's proof, a claim (journal, image ID, exit status) plus a seal that attests to it; the public outputs; the cryptographic part; a mode that produces fake receipts, without cryptographic integrity. "Genuine" only to contrast with fake receipts.
- **the SPARQL evaluator running in the zkVM:** our guest program, a SPARQL evaluator with fixed capacity limits; "the evaluator" after first use, "with signature checks" for the version that verifies issuer signatures.
- **execution without proving; cycles; proving time:** running the guest in the executor, with no receipt; its cycle count (about one per RISC-V instruction), deterministic for a given guest and input; the time to produce a receipt. Report cycles and proving time separately.
- **attestation:** a TEE-signed report that a measured program ran, which must contain the statement digest.
- **publicly verifiable; designated-verifier:** anyone holding the proof and the request can check it; only a verifier holding secret verification information can, interactively (the proposed VOLE method) or not. Non-transferability, that the verifier cannot convince a third party, is a separate property. ZKProof §1.5.

**Security properties**

- **proof soundness; knowledge soundness; proof-system completeness; zero-knowledge:** no prover convinces the verifier of a false statement, except with negligible probability; from any prover that convinces the verifier, an efficient extractor obtains a valid witness; an honest prover with a valid witness always convinces the verifier; the proof reveals nothing about the witness beyond what the statement already reveals. ZKProof §1.6.
- **hiding; binding; salted:** a commitment reveals nothing about its value; it cannot be opened to two values. Each holds computationally or statistically. A salted digest is binding if the hash is collision-resistant, and hiding only if the salt is random and secret and the hash is modelled as a random oracle. Thaler §12.3.
- **unforgeability:** existential unforgeability under chosen-message attack: an attacker who obtains signatures on messages of its choice cannot feasibly produce a valid signature on any other message. Goldwasser, Micali & Rivest 1988.
- **unlinkability:** verifiers cannot correlate presentations of one credential beyond what the disclosed values link. Name the mechanism and the disclosed data: `bbs-2023` derived proofs are unlinkable, but revealed signatures, a reused dataset commitment or an identifying result link presentations. VC DM 2.0 §2; spec §10.2.
- **classical; post-quantum:** secure against an adversary without, or with, a large quantum computer. Name the property: Ed25519, ECDSA and Schnorr signatures are unforgeable only classically, ML-DSA-44 also post-quantum; a Groth16 receipt's soundness is classical only. Suite draft §6.

**Evaluation**

- **experiment; run; repetition:** runs of one build that answer one question; one execution of one test case; a repeat of a run, for timing.
- **test case; negative test; positive control:** request, input and expected outcome; an altered, replayed or misdirected presentation the verifier must reject; the unaltered presentation it must accept.
- **preliminary measurement:** a timing whose conditions are stated once.
- **baseline circuit; public-triple circuit:** the Noir circuits of §6.5; the matched triple is part of the witness in the first and supplied as a public input in the second.

**Soundness and completeness.** "Sound" and "complete" describe results only when relating Supported and Exact to prior work (zkRDF: soundness; Li et al.: correctness and completeness). Elsewhere write Supported, Exact or "complete over *D*". For proof systems write "proof soundness", "knowledge soundness" or "proof-system completeness", never bare "soundness" or "completeness".

**Coinages kept:** the spec's terms; *public-input rule*; *baseline circuit* and *public-triple circuit*, as experimental labels. "Our Merkle-based cryptosuites" and "the SPARQL evaluator running in the zkVM" are descriptions, not names.

**Beyond spec version 1:** say so; never rename a difference away. The prototype proved `DESCRIBE` answers, which version 1 excludes: state once which triples a `DESCRIBE` returns, with those receipts. The paper's dataset scope (named graphs, `FROM`, `FROM NAMED`) exceeds version 1, which forbids dataset clauses. The paper's disclosure policy *d* has no spec member: drop it from the rule.

## 4. Notation

- *Q*; *D*: the request's query; the input dataset.
- `⟦Q⟧_D`: the SPARQL 1.1 result of *Q* over *D*, with its solution modifiers and query form applied. Say once that its algebra part is eval(D(G), P) (SPARQL 1.1 §18.6; *P*: the algebra expression of *Q*'s pattern; *G*: the default graph of *D*).
- *Q'*: *Q* without its top-level `ORDER BY`, `OFFSET` and `LIMIT`; only in the Supported definition.
- μ: a solution mapping, as in the running example's μ = {?person ↦ ex:alice}.
- μ(*t*): the triple obtained by replacing each variable of triple pattern *t* by its value under μ (SPARQL 1.1 §18.3.1); only for a *t* without blank nodes all of whose variables μ binds. Drop it if §5 can say this in words.
- *x*, *w*, *R*: public input, witness, relation (ZKProof §1.2); appendix only, and only if a formula needs them.

Typst: `$[| Q |]_D$`, `$Q'$`, `$mu(t)$`.

**Remove:** `⟦P⟧_D`, `C = ⟨m, o, q, f, a, s, e, d, b, t⟩`, `c`, `k`, `r`, `π`, `h(C)`, `E`, `K`, `ρ`, `p_C`, `σ`, `Com`, `Map`, `Anc`, `Src`, `Scp`, `Bnd`, `Ans`, `supp`, `⊑`, `tμ`, `V`; *n*, dom(μ) and `card` unless a formula needs them. No "Proposition", "Theorem" or "Design argument": each argument becomes short appendix prose with numbered assumptions (A1, A2, …), or goes if it adds nothing.

**Adding a symbol:** only if used twice or more, clearer than words, defined in words at first use, distinct from the symbols above (and Ω, P, G), and replaceable by a noun; then list it here.

## 5. Replacement list

Old term → use.

**Evaluation**
- campaign → experiment; "run" stays, for one execution of one test case
- declared case; test ids → test case
- evidence inspection; second-party inspected → the internal check (guide §6), never named
- evidence level; executed native; executed guest → column "Strongest result": tests outside the zkVM; executed without proving; proof verified
- genuine → receipt ("genuine" only against fake receipts)
- controls; control records → negative tests; positive control
- gate; hosted; frozen; retained; inventory → describe, or delete
- "at a later source"; commit hashes → artifact table
- paired harness; arms; work counters; equal-guarantee comparison; specialisation → paired comparison; circuits; check counts; same statement
- indicative → preliminary
- evidence, for experiment outcomes → results ("evidence" only for what a proof method produces, spec §9)
- replay, meaning re-execution → re-execution ("replay" only for the attack)

**Request and answer**
- query contract; contract; stored contract; contract tuple → query request; stored request
- input authority; authority; provenance field → input kind
- `HolderDeclared`; `HolderSelected…`; `VerifierAgreed` → holder-declared; verifier-agreed
- anchor *k* → the agreed dataset commitment
- an input it agreed independently → an input whose dataset commitment it accepted in advance
- answer mode; supported/exact mode; exact path; `SelectedResults` → answer kind
- supported; exact → Supported; Exact
- scope *s*; graph catalog *K* → dataset construction (guide §3, beyond version 1)
- result form *f* → query form, with `DISTINCT` where it matters
- request binding → request digest
- nonce; consume the challenge → challenge; mark the request as answered ("nonce" only for OpenID4VP's `nonce`)
- row bound; capacity profile K1, K2 → `maxResultRows`; one- or two-credential circuit
- release; authorised release → disclose (the holder discloses the answer), never "return"
- release condition → the timing proposal (guide §6)
- acceptance → only for the verifier's decision
- validity window → request validity period; credential validity period

**Proofs, methods and signatures**
- method; query-proof method → proof method
- method descriptor; descriptor digest → the request's proof-method entry (`id`, `version`, `verificationKey`, `parameters`); its digest, in words, never "image ID"
- registry; capability tuple → the proof methods the verifier accepts
- pin; pinned image; guest pin; artifact → image ID; verification key; guest binary ("artifact" only for the research artifact)
- signature suite → cryptosuite
- key table; key policy; source evidence *e*; `None` → the request's issuer keys; no issuer keys
- adapter; protocol adapter; generic authenticated adapter → our holder and verifier services
- authenticated extension; V5; exact evaluator; request versions → the evaluator, with or without signature checks; versions in the artifact table
- V1; V4; public-pattern, baseline or Noir relation → baseline circuit; public-triple circuit
- zkVM evaluator → the SPARQL evaluator running in the zkVM, then "the evaluator"
- Merkle-root cryptosuite → our Merkle-based cryptosuites
- transferable; image identifier → publicly verifiable; image ID
- "proof", for an issuer's signature → Data Integrity proof, or signature
- signatures-in-proof; quantum-safe; blinded; integrity of a signature → hidden mode; post-quantum; salted or hiding; unforgeability

**Semantics and disclosure**
- canonical graph equality → isomorphism
- union of credential graphs → RDF merge
- bag; set results → multiset; `SELECT DISTINCT`
- positive pattern; positive basic graph patterns → monotone query
- blank-node closure policy → describe once, with the `DESCRIBE` results
- answer-derived → computable from the stored request and the result
- public eligibility; public-eligible → may be a public input
- conservative public-data rule; public-data rule; the rule → public-input rule
- public triple → triple supplied as a public input
- obligation; secret work; membership and equality work → what the proof must still show
- subject linkage → the join on `?person`
- pre-output or abort privacy; disclosure analysis; partial-release observables → what the verifier learns, before an answer or on abort
- design argument → prose with numbered assumptions (guide §4)
- override the open-world reading → an Exact answer shows absence only within *D*
- positive-existence statement; membership-indifferent modifiers → true `ASK`; solution sequence modifiers

**Words with several senses**
- binding, bound, bounded → "bind" for proofs, commitments (spec §7) and holder binding; otherwise size limit, range proof or capacity limit
- native; native or V5 model; native oracle → outside the zkVM; BBS+ proofs; reference evaluator
- accept, for keys or inputs → the request lists keys; the verifier accepts a dataset commitment (spec §6.2)
- canonical, outside RDFC-1.0 → only for a method's canonical encoding (spec §5.1); never "canonical equality"
- host, for the program → host program ("host" only for the machine)
- statement, for a bank document; record, for a credential → bank statement; credential
- protocol verifier; relying party; presenter → verifier; holder

**Labels and one-offs**
- `HolderSelectedAuthenticated`, `VerifierAgreedAuthenticated`, `CapacityExceeded`, `Succinct`, `risc0-r0vm 3.0.6`; in the appendix `NotRequested`, `all_defined_cases_run`, `dev_mode`, `Halted(0)` → words; versions in the artifact table
- typed; strict; wire forms; admission → delete; explain; serialisations; request checks
- lane, tiers, invariant downgrade, audit gates, integration design; `sq-qhy4` and repository paths → delete
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

1. **Input dataset:** the RDF merge of the credentials' graphs as the default graph, or one named graph per credential where a method says so.
2. **Exact slices:** before each `OFFSET` and `LIMIT`, order by that level's `ORDER BY`, then by the method's canonical encoding; Exact queries use no `REDUCED`.
3. **Supported:** monotone queries only (spec §2); a Supported `SELECT` row is in `⟦Q'⟧_D`.
4. **Public inputs:** any value the verifier can compute from its stored request and the result; no request member permits disclosure.
5. **Merkle-based cryptosuites** sign a digest of the suite identifier, a 32-byte salt, the quad count, the Merkle root (one leaf per canonical quad) and the proof-configuration digest.
6. **Wording:** "publicly verifiable" and "designated-verifier"; "monotone query"; "a Data Integrity proof (a signature)"; `verificationKey` for `artifact`; no "pins"; "image ID".

**Open:**

1. **Unpublished ISWC 2025 manuscript** (the earlier results-only interface): cite it anonymously, or describe it uncited?
2. **Poseidon2 message:** the suite draft (§5, unchanged at the head of #6789) signs `P(3, salt, n, root, c1, c0)`, without the suite identifier that decision 5 lists. Align one of them before the paper describes this member.
3. **`SAMPLE` and `GROUP_CONCAT`:** SPARQL does not uniquely determine their results, and decision 2 does not fix them. Forbid them in Exact queries, or fix them by the canonical encoding?
4. **`⟦Q⟧_D`:** spec §2 calls it a multiset of solution mappings; guide §4 uses the full result. Align spec §2, or state the `ASK` case in words, as guide §3 does.
5. **Circuit label:** "public-triple circuit" keeps a phrase guide §5 replaces elsewhere. Keep it, defined once in §6.5, or rename it?

## 8. Sources

**Writing.** Gopen & Swan 1990, https://www.jstor.org/stable/29774235 · Mensh & Kording 2017, https://doi.org/10.1371/journal.pcbi.1005619 · Peyton Jones, https://www.microsoft.com/en-us/research/academic-program/write-great-research-paper/ · Knuth et al. 1989, https://jmlr.csail.mit.edu/reviewing-papers/knuth_mathematical_writing.pdf · Halmos 1970, https://www-users.cse.umn.edu/~cberkesc/5385/Spring2018/halmosWrite.pdf · Widom, https://cs.stanford.edu/people/widom/paper-writing.html · Ernst, https://homes.cs.washington.edu/~mernst/advice/write-technical-paper.html · Google Tech Writing, https://developers.google.com/tech-writing/one · Orwell 1946, https://www.orwellfoundation.com/the-orwell-foundation/orwell/essays-and-other-works/politics-and-the-english-language/ · Digital.gov, https://digital.gov/guides/plain-language/writing · GOV.UK, https://guidance.publishing.service.gov.uk/writing-to-gov-uk-standards/writing-guidelines/clear-language/ · Pinker 2014, https://www.chronicle.com/article/why-academics-stink-at-writing/

**Specifications.** SPARQL 1.1 Query, https://www.w3.org/TR/sparql11-query/ · RDF 1.1 Concepts, https://www.w3.org/TR/rdf11-concepts/ · RDF 1.1 Semantics, https://www.w3.org/TR/rdf11-mt/ · RDFC-1.0, https://www.w3.org/TR/rdf-canon/ · VC DM 2.0, https://www.w3.org/TR/vc-data-model-2.0/ · VC-DI, https://www.w3.org/TR/vc-data-integrity/ · EdDSA, https://www.w3.org/TR/vc-di-eddsa/ · VC-DI-ECDSA, https://www.w3.org/TR/vc-di-ecdsa/ · BBS, https://www.w3.org/TR/vc-di-bbs/ · Quantum-Resistant Cryptosuites, https://www.w3.org/TR/vc-di-quantum-resistant/ · OpenID4VP 1.0, https://openid.net/specs/openid-4-verifiable-presentations-1_0.html · RISC Zero, https://dev.risczero.com/terminology

**Literature.** ZKProof Reference, https://docs.zkproof.org/reference.pdf · Thaler, https://people.cs.georgetown.edu/jthaler/ProofsArgsAndZK.pdf · Goldwasser, Micali & Rivest, *SIAM J. Comput.* 17(2), 1988 · zkRDF (Braun, Wright & Käfer, ESWC 2026), https://publikationen.bibliothek.kit.edu/1000193675 · Wright (ISWC 2025 Companion), https://ceur-ws.org/Vol-4085/paper19.pdf · Li et al. (SIGMOD 2006), https://www.cs.bu.edu/~reyzin/papers/auth-db.pdf
