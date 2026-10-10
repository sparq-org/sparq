# Language guide: "Answers Instead of Documents"

For `zksparql-architecture.typ`. "Spec §n": *Zero-Knowledge SPARQL Answers* (draft, sparq-org/sparq#6786 at `e312ad47`). "Suite draft §n": the Merkle-root cryptosuite specification (sparq-org/sparq#6789). Bare "§n": the paper's current numbering. "Guide §n": this file. "Decision n": guide §7.

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
12. **Verbs, not nominalisations; no stacks of three or more nouns,** except established names: "the verifier checks everything, then marks the request as answered."
13. **Formula or prose, whichever this reader parses faster:** a formula for a short exact condition, a table for a structure, a list for a procedure. Define symbols in words; no ∀, ∃ or ⇒ in text, and no symbol at a sentence start.
14. **Running example:** introduce a definition with the lender question that motivates it, not with all three.
15. **Clear referents:** "In `?p a ex:Employee OPTIONAL { ?p ex:email ?e }`, a row with `?e` unbound shows that no email triple for `?p` matched. Only an Exact answer can show such an absence."
16. **Form:** no meta-commentary; present tense for the design, past for experiments; British *-ise* spelling, except in titles; no em-dashes; code font for SPARQL keywords, JSON members, IRIs and cryptosuite identifiers; capitalised Supported and Exact; few acronyms; never "VC" (except in titles) or "ZKP".

## 3. Vocabulary

**Established terms,** used as their sources define them:

- *RDF and SPARQL:* RDF dataset, default, named and active graph (SPARQL 1.1 §13, §18.1); RDF merge (RDF 1.1 Semantics §4.1); blank node scope (RDF 1.1 Concepts §3.4); canonical N-Quads form, a serialisation (RDFC-1.0 §3.1); isomorphic, a relation (Concepts §3.6, §4.1); term equality, which pattern matching uses, unlike value equality in operators (Concepts §3.3; SPARQL 1.1 §17.4.1.7); solution mapping μ (in a returned `SELECT` result, a row), multiset, solution sequence (SPARQL 1.1 §18.1.8, §18.3); projection (§15.2).
- *Credentials:* claim, credential, verifiable credential ("credential" for short once defined), subject (not necessarily a person), issuer, holder, verifier, verification, validation, unlinkable disclosure (VC DM 2.0 §2; we verify signatures only, not status or business requirements); validity period (§4.9); credential status (§4.10); cryptosuite, Data Integrity proof, verification method, controlled identifier document (VC-DI §1.4).
- *Proofs:* statement, instance (say "public input"), witness, relation (ZKProof §1.2); publicly verifiable, designated verifier, which may be non-interactive and differs from non-transferable (§1.5); completeness, soundness, zero-knowledge (§1.6); hiding, binding (Thaler §12.3); host, host program, guest program, receipt, journal, seal, image ID, executor, cycle (RISC Zero).
- *Spec:* its terms (spec §2 to §9), exactly as written. An answer presentation is not a W3C verifiable presentation.

**Rules for this paper**

*Data and queries*
- **Dataset construction** (decision 1): the method names its layout, since a query written for the merged default graph does not match one named graph per credential. A result's blank node labels mean nothing outside it.
- **Monotone query** (spec §2): apart from a top-level `ORDER BY`, `OFFSET` and `LIMIT`, a `SELECT` or `ASK` query with only basic graph patterns, group graph patterns, `UNION`, `GRAPH`, `VALUES`, projection, `DISTINCT`, and `FILTER` or `BIND` without `EXISTS` or `NOT EXISTS`. Supported answers are allowed only for these (decision 3) because the holder may leave credentials out of *D*: a Supported row must stay a solution when those credentials are added, as a monotone query's solutions do. Over *D* itself, a row needs no monotonicity. A count over a subset is that subset's exact count, never "supported".

*Credentials and signatures*
- **Signature or proof:** a cryptosuite specifies algorithms for creating and verifying Data Integrity proofs, derived ones included. The issuer's Data Integrity proof is here a signature with its proof configuration (the proof without `proofValue`); say "signature" for modes and checks. "Proof" alone is the holder's query proof.
- **Keys; issuer-key authorisation:** the verification method (here a public key), its identifier (`verificationMethod`) and the key bytes (`publicKeyMultibase`) differ. Data Integrity authorises an issuer's key through a verification relationship (`assertionMethod`) in the issuer's controlled identifier document (VC-DI §2.6); our requests list the accepted keys in `issuers` instead.
- **`eddsa-rdfc-2022`:** Ed25519 signs the SHA-256 hash of the canonical proof configuration followed by that of the canonical document (EdDSA §3.2). It is unsalted, so whoever sees the hashes can test a guessed document.
- **`ecdsa-rdfc-2019`:** the same with ECDSA, P-256 with SHA-256 or P-384 with SHA-384. **`mldsa44-rdfc-2024`:** ML-DSA-44 with SHA-256, post-quantum; First Public Working Draft of Quantum-Resistant Cryptosuites v1.0, 16 June 2026.
- **`bbs-2023`, `ecdsa-sd-2023`:** the holder derives from the issuer's base proof a proof disclosing selected claims. Only `bbs-2023` derived proofs are unlinkable (VC-DI-ECDSA §5.1).
- **Our Merkle-based cryptosuites** sign the digest of decision 5: `eddsa-sha256-merkle-2026` (SHA-256 tree, Ed25519), verified in the zkVM, and `schnorr-poseidon2-merkle-2026` (draft identifier; Poseidon2 tree, Schnorr over Baby Jubjub), the member for Noir. Neither is post-quantum (suite draft §1.2, §3.5, §5).
- **Credential status:** whether the issuer has revoked or suspended the credential.
- **Credential validity period:** `validFrom` to `validUntil`; distinct from a proof's `created` and `expires` and from a key's validity.
- **Holder binding:** the party presenting is the subject, or controls a key bound to the credential.

*Request and answer*
- **Supported:** every returned solution is a solution: for `SELECT`, a non-empty result without duplicates, each row in `⟦Q'⟧_D`; for `ASK`, `true`.
- **Exact** (decision 2): a result SPARQL 1.1 permits for *Q* over *D*, with nothing added or omitted and duplicates kept; where SPARQL leaves a choice open, the holder makes it. Open choices include `OFFSET` and `LIMIT` where `ORDER BY` does not fix the order, `REDUCED`, `SAMPLE` and `GROUP_CONCAT`: such a result is "not uniquely determined by SPARQL", never "undefined". The choice can act as a covert channel: over the values 0 and 1, `LIMIT 1` encodes one hidden bit, and zero-knowledge does not prevent this because the result is public (spec §10.2). For `ASK`, `true` exactly when `⟦Q⟧_D` is non-empty; a `CONSTRUCT` graph is compared up to isomorphism. Neither answer kind says who signed *D*.
- **Input kinds:** verifier agreement shows that two commitments are equal, not that *D* is complete in the world.
- **Signature modes** (spec §6.3): in the revealed mode the proof ties *D* to the signatures the verifier checks; in the disclosed mode the verifier evaluates *Q* itself.
- **Public-input rule** (decision 4): substituting a returned solution into a query that is a single basic graph pattern gives triples the verifier can compute. Under `UNION` the matched branch is not computable, so its triple does not qualify. The proof must still show that each triple supplied as a public input is in *D* and signed.
- **Disclose, deliver, accept, mark as answered:** sending the presentation discloses the answer; delivery is its arrival, which a transport may split; the verifier accepts after every check; marking the request as answered, atomically and just before acceptance, prevents replay. A rejected presentation has still disclosed its answer.

*Proofs and the zkVM*
- **Proof method:** a way of producing and checking the proof, named by an identifier (IRI) and a version; its evidence is a zero-knowledge proof, a proof that is not zero-knowledge, a TEE attestation or disclosed credentials. The verifier checks the evidence with a verification key from its own configuration (`verificationKey`: a circuit's verification key, a zkVM image ID or an attestation root key) and the request's `parameters`. The proof system is the scheme: RISC Zero's STARK, optionally wrapped in Groth16, or UltraHonk.
- **Relation, statement, circuit:** the relation is the condition that public inputs and witness must jointly satisfy; the statement asserts that, for the given public inputs, some witness satisfies it; a circuit implements a relation as constraints (ours in Noir). Public inputs: the values spec §2 lists, each passed directly or bound by one digest. Witness: credentials, salts and, in the hidden mode, signatures.
- **zkVM:** the host is the machine, the host program the untrusted code that runs the guest program and supplies its inputs. A receipt is a claim (journal, image ID, exit status) plus a seal attesting to it. Development mode makes fake receipts. Report cycles, deterministic and counted without proving, separately from proving time.

*Security properties*
- **Knowledge soundness:** an efficient extractor obtains a valid witness from any prover that convinces the verifier. **Zero-knowledge:** the proof reveals nothing about the witness beyond the statement. For proof systems, never bare "soundness" or "completeness"; "proof-system completeness" is fine.
- **Salted:** a salted digest is binding if the hash is collision-resistant, and hiding only if the salt is random and secret and the hash is modelled as a random oracle. Say whether a property holds computationally or statistically.
- **Unforgeability:** existential unforgeability under chosen-message attack: given signatures on messages of its choice, an attacker cannot feasibly sign any other message.
- **Unlinkability:** presentations of one credential cannot be correlated beyond what they disclose. Name the mechanism and the disclosed data: `bbs-2023` derived proofs are unlinkable, but revealed signatures, a reused dataset commitment or an identifying result link presentations.
- **Classical, post-quantum:** secure against an adversary without, or with, a large quantum computer; name the property. Ed25519, ECDSA and Schnorr signatures are unforgeable only classically, ML-DSA-44 also post-quantum; a Groth16 receipt is sound only classically.

*Evaluation*
- **Experiment, run, repetition:** runs of one build answering one question; one execution of one test case; a repeated run, for timing.
- **Test case, negative test, positive control:** request, input and expected outcome; an altered, replayed or misdirected presentation the verifier must reject; the unaltered one it must accept.
- **Baseline and public-triple circuits** (§6.5): the matched triple is part of the witness in the first, a public input in the second.

**Results:** "sound" and "complete" appear only when relating Supported and Exact to prior work (zkRDF: soundness; Li et al.: correctness, completeness).

**Beyond spec version 1:** say so; never rename a difference away. The prototype proved `DESCRIBE` answers, which version 1 excludes; state once which triples a `DESCRIBE` returns. The paper's dataset scope (named graphs, `FROM`, `FROM NAMED`) exceeds version 1, which forbids dataset clauses. Its disclosure policy *d* has no spec member: drop it from the rule.

## 4. Notation

- *Q*; *D*: the request's query; the input dataset.
- `⟦Q⟧_D`: as spec §2 defines it, a multiset of solution mappings SPARQL 1.1 permits for *Q*'s pattern over *D* after its solution modifiers (for `ASK`, before it becomes a boolean); where several are permitted, the one the holder evaluated. The query form then gives the result. Do not write its algebra form eval(D(G), P): the paper never uses it.
- *Q'*: *Q* without its top-level `ORDER BY`, `OFFSET` and `LIMIT`; only in the Supported definition.
- μ: a solution mapping.
- μ(*t*): triple pattern *t* with each variable replaced by its value under μ (SPARQL 1.1 §18.3.1); only for a *t* without blank nodes whose variables μ all binds, and only if §5 cannot say it in words.

Typst: `$[| Q |]_D$`, `$Q'$`, `$mu(t)$`.

**Remove:** eval(D(G), P), `⟦P⟧_D`, `C = ⟨m, o, q, f, a, s, e, d, b, t⟩`, `c`, `k`, `r`, `π`, `h(C)`, `E`, `K`, `ρ`, `p_C`, `σ`, `Com`, `Map`, `Anc`, `Src`, `Scp`, `Bnd`, `Ans`, `supp`, `⊑`, `tμ`, `V`; *n*, dom(μ) and `card` unless a formula needs them. No "Proposition" or "Design argument": each argument becomes short appendix prose with numbered assumptions (A1, A2, …), or goes.

**Adding a symbol:** only if used twice or more, clearer than words, and defined in words at first use.

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
- indicative → preliminary (a timing whose conditions are stated once)
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
- zkVM evaluator → the SPARQL evaluator running in the zkVM (our guest program), then "the evaluator"
- Merkle-root cryptosuite → our Merkle-based cryptosuites
- transferable; image identifier → publicly verifiable; image ID
- "proof", for an issuer's signature → Data Integrity proof, or signature
- signatures-in-proof; blinded → hidden mode; salted or hiding

*Semantics and disclosure*
- canonical graph equality; union of credential graphs → isomorphism; RDF merge
- bag; set results → multiset; `SELECT DISTINCT`
- positive pattern → monotone query
- blank-node closure policy → describe once, with the `DESCRIBE` results
- answer-derived; public-eligible → computable from the stored request and the result; may be a public input
- public-data rule; the rule → public-input rule
- public triple → triple supplied as a public input
- obligation; secret work → what the proof must still show
- pre-output or abort privacy; disclosure analysis → what the verifier learns, before an answer or on abort
- override the open-world reading → an Exact answer shows absence only within *D*

*Words with several senses*
- binding, bound, bounded → "bind" for proofs, commitments and holder binding; else size limit, range proof or capacity limit
- native → outside the zkVM; BBS+ proofs; reference evaluator
- accept, for keys → the request lists keys
- canonical, outside RDFC-1.0 → only for a method's canonical encoding
- statement, for a bank document; record, for a credential → bank statement; credential
- the first prototype in this line → Wright [wright25dc]

## 6. Fixed phrasings

Each appears in full once, in its home (rule 7).

- **Coverage** (§6.2; the abstract may say "over synthetic credentials"): "We used only synthetic credentials signed with test keys."
- **Development mode** (§6.2): "We generated all receipts with development mode disabled and verified them against the expected guest image ID."
- **Internal check** (§6.2; never name the check): "For experiments without † in Table 2, we recomputed hashes of source files, guest binaries and receipts. We compared the hashes and recorded test outcomes with the archived records. We did not verify the proofs again."
- **Audit** (§8.5): "The prototype has not undergone an external security audit."
- **Disclosure timing** (§5.4): "We propose that a party receive these public inputs no earlier than the answer from which it can compute them. The prototype does not enforce this condition."
- **Holder-declared input** (§3.3): "For a holder-declared input, an Exact answer covers only the included credentials."
- **Unchecked properties** (§8.2): "The SPARQL evaluator running in the zkVM does not check credential status, holder binding or credential validity periods."

## 7. Decisions and open questions

**Decided** in the spec (#6786, 10 October 2026; decision 2 as revised at `ecd614fa`):

1. **Input dataset:** the credentials' graphs, RDF-merged into the default graph, or one named graph each if the method says so.
2. **Exact:** a result SPARQL 1.1 permits for *Q* over *D*, with nothing added or omitted; where SPARQL leaves a choice open, the holder makes it. A method MAY fix open choices; a verifier relies on that only where the method publishes them. This replaces the tie-break rule and the `REDUCED` ban.
3. **Supported:** monotone queries only; a `SELECT` row is in `⟦Q'⟧_D`.
4. **Public inputs:** a method may make public any value the verifier can compute from its stored request and the result alone; no request member permits disclosure.
5. **Merkle-based cryptosuites** sign a digest of the suite identifier, a 32-byte salt, the quad count, the Merkle root (one leaf per canonical quad) and the proof-configuration digest.
6. **Wording:** "publicly verifiable", "designated-verifier", "monotone query", "a Data Integrity proof (a signature)", `verificationKey` (formerly `artifact`), "image ID"; no "pins".

**Open:**

1. **Unpublished ISWC 2025 manuscript** (the earlier results-only interface): cite it anonymously, or describe it uncited?
2. **Poseidon2 message:** suite draft §5, also at the head of #6789, signs `P(3, salt, n, root, c1, c0)`, without the suite identifier of decision 5. Align one of them.
3. **Choices our evaluator fixes:** which open choices of decision 2 does the evaluator fix, and where is that published? Until then, an Exact claim means one permitted result, chosen by the holder.

## 8. Sources

**Writing.** The twelve guides behind guide §2, from Gopen & Swan (1990) to Pinker (2014), are listed with links in this file's first version (commit `3bdf74ae5`).

**Specifications.** SPARQL 1.1 Query, https://www.w3.org/TR/sparql11-query/ · RDF 1.1 Concepts, https://www.w3.org/TR/rdf11-concepts/ · RDF 1.1 Semantics, https://www.w3.org/TR/rdf11-mt/ · RDFC-1.0, https://www.w3.org/TR/rdf-canon/ · VC DM 2.0, https://www.w3.org/TR/vc-data-model-2.0/ · VC-DI, https://www.w3.org/TR/vc-data-integrity/ · EdDSA, https://www.w3.org/TR/vc-di-eddsa/ · VC-DI-ECDSA, https://www.w3.org/TR/vc-di-ecdsa/ · BBS, https://www.w3.org/TR/vc-di-bbs/ · Quantum-Resistant Cryptosuites, https://www.w3.org/TR/vc-di-quantum-resistant/ · OpenID4VP 1.0, https://openid.net/specs/openid-4-verifiable-presentations-1_0.html · RISC Zero, https://dev.risczero.com/terminology

**Literature.** ZKProof Reference, https://docs.zkproof.org/reference.pdf · Thaler, https://people.cs.georgetown.edu/jthaler/ProofsArgsAndZK.pdf · Goldwasser, Micali & Rivest, *SIAM J. Comput.* 17(2), 1988 (unforgeability) · zkRDF (Braun, Wright & Käfer, ESWC 2026), https://publikationen.bibliothek.kit.edu/1000193675 · Wright (ISWC 2025 Companion), https://ceur-ws.org/Vol-4085/paper19.pdf · Li et al. (SIGMOD 2006), https://www.cs.bu.edu/~reyzin/papers/auth-db.pdf
