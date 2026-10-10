# Language guide: "Answers Instead of Documents"

For `zksparql-architecture.typ`. "Spec §n": *Zero-Knowledge SPARQL Answers: Request and Presentation Data Model* (draft, sparq-org/sparq#6786 at `62000d63`, "spec v2"). "Suite draft §n": the Merkle-root cryptosuite specification (sparq-org/sparq#6789). Bare "§n": the paper's current numbering. "Guide §n": this file. "Decision n": guide §7.

## 1. Audience and goal

Readers: ESWC reviewers, expert in RDF and SPARQL, familiar with verifiable credentials, new to zero-knowledge proofs. Aim: one argument, not a project log, in established terms (the spec's for our own concepts), with formulas only where clearer than prose. The argument is about expressivity: which SPARQL answers a holder can prove over issuer-signed credentials, over which input, and what an accepted answer then means.

## 2. Style rules

Rules 1 to 6 are firm. Rules 7 to 16 are strong preferences, broken only where precision needs it.

1. **One term per concept, one concept per term** (guide §3), defined at or before first use. Keep each distinction before simplifying.
2. **Established term before coinage; describe rather than name.** Coin only for a concept used three or more times without an established name.
3. **No internal labels in prose:** commit hashes, test ids, enum and type names, V1 to V5, campaign, gate, hosted, frozen, retained, inventory, bead, `zkp-*`, repository paths. Versions, hashes and proof-method identifiers go in one artifact table.
4. **Every empirical number comes from the evidence lookups** (`#headline(key)`; `#ev(key)` for preliminary timings). Compute derived numbers (ratios, totals) in the evidence data, never in prose. Never add counts from different experiments. Design limits of the prototype (such as its credential capacity) and figures reported by prior work are not evidence: state them with a source (a code constant in a source comment, or a citation with a pointer).
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
15. **Clear referents:** "In `?p a ex:Employee OPTIONAL { ?p ex:email ?e }`, a row with `?e` unbound shows that no email triple for `?p` matched. Such a row reads *D* as complete."
16. **Form:** no meta-commentary; present tense for the design, past for experiments; British *-ise* spelling, except in titles and quoted names; no em-dashes; code font for SPARQL keywords, JSON-LD terms, IRIs and cryptosuite identifiers; few acronyms; never "VC" (except in titles) or "ZKP".

## 3. Vocabulary

**Established terms,** used as their sources define them:

- *RDF and SPARQL:* SPARQL 1.2 Query and RDF 1.2, on which the spec depends (spec §10.5), define what an answer is; cite SPARQL 1.2 for that, and the sections below for the terms they define. Our evaluator implements SPARQL 1.1 without the features new in 1.2, such as triple terms: say so once (§7.2). RDF dataset, default, named and active graph (SPARQL 1.1 §13, §18.1); RDF merge (RDF 1.1 Semantics §4.1); blank node scope (RDF 1.1 Concepts §3.4); triple term (RDF 1.2 Concepts); canonical N-Quads form, a serialisation (RDFC-1.0 §3.1); isomorphic, a relation (Concepts §3.6, §4.1); term equality, which pattern matching uses, unlike value equality in operators (Concepts §3.3; SPARQL 1.1 §17.4.1.7); solution mapping (in a returned `SELECT` result, a row), multiset, solution sequence (SPARQL 1.1 §18.1.8, §18.3); projection (§15.2).
- *Credentials:* claim, credential, verifiable credential ("credential" for short once defined), subject (not necessarily a person), issuer, holder, verifier, verification, validation, unlinkable disclosure (VC DM 2.0 §2; we verify signatures only, not status or business requirements); verifiable presentation, which may carry data derived from credentials, such as a zero-knowledge proof, instead of the credentials (VC DM 2.0); validity period (`validFrom`, `validUntil`; §4.9); credential status (§4.10); cryptosuite, Data Integrity proof, verification method, controlled identifier document, and the proof options `challenge` and `domain` (VC-DI §1.4).
- *Proofs:* statement, instance (say "public input"), witness, relation (ZKProof §1.2); publicly verifiable, designated verifier, which may be non-interactive and differs from non-transferable (§1.5); completeness, soundness, zero-knowledge (§1.6); hiding, binding (Thaler §12.3); host, host program, guest program, receipt, journal, seal, image ID, executor, cycle (RISC Zero).
- *Spec:* its terms (spec §2 to §9), exactly as written: query request, answer presentation, input dataset, dataset commitment, request digest, trust requirement, signature mode, proof method, proof, statement, signed message.

**Rules for this paper**

*Data and queries*
- **Dataset construction** (decision 1): the method names its layout, since a query written for the merged default graph does not match one named graph per credential. A result's blank node labels mean nothing outside it.
- **Answer** (spec §6.1; decision 2): a result that SPARQL 1.2 permits for *Q* over *D*, with nothing added or omitted, and never truncated: for `SELECT`, a solution sequence with every solution and duplicate; for `ASK`, `true` exactly when the query pattern has a solution over *D*; for `CONSTRUCT`, a graph *Q* can produce over *D*, compared up to isomorphism. Without `ORDER BY`, compare a `SELECT` result as a multiset. Where SPARQL permits several results (`OFFSET` and `LIMIT` where `ORDER BY` does not fix the order, `REDUCED`, `SAMPLE`, `GROUP_CONCAT`, `MIN` or `MAX` over values SPARQL does not order, implementation-defined arithmetic precision), the holder chooses: such a result is "not uniquely determined by SPARQL", never "undefined". A method MAY fix these choices; a verifier relies on that only where the method publishes them. The choice can act as a covert channel: over the values 0 and 1, `LIMIT 1` encodes one hidden bit, and zero-knowledge does not prevent this because the result is public (spec §10.3). An answer does not say who signed *D*. Say "answer" or "result"; never Supported, Exact, partial answer or answer kind.
- **Monotone query** (the precise notion behind the open-world reading): apart from a top-level `ORDER BY`, `OFFSET` and `LIMIT`, a `SELECT` or `ASK` query with only basic graph patterns, group graph patterns, `UNION`, `GRAPH`, `VALUES`, projection, `DISTINCT`, and `FILTER` or `BIND` without `EXISTS` or `NOT EXISTS`. A solution of a monotone query over *D* is a solution over any dataset that contains *D*.
- **Open-world and closed-world reading** (spec §10.1): which reading applies follows from the query and, for `ASK`, its answer; neither the request nor the presentation states it. An answer to a monotone query keeps the open-world reading in what it contains: a true `ASK` and each returned row stay true for any larger input. A **closed-world answer** reads *D* as complete: a false `ASK`; a row produced by `NOT EXISTS`, `MINUS` or an unbound `OPTIONAL` variable; a count or other aggregate; "the latest"; or a `SELECT` result read as all the solutions. It is correct over *D*, and the proof shows that; it answers the verifier's question only if *D* holds every relevant statement, so it is only as meaningful as the verifier's reason to believe that *D* is complete. Use "open-world reading", "closed-world reading" and "closed-world answer"; coin nothing further.
- **Counting:** a count over credentials the holder chose counts only those, and a verifier that counts returned rows itself counts only what the holder chose to show.
- **zkRDF's guarantee** (once, §3.2): its soundness guarantee, that each returned solution is a solution over signed data, corresponds to answers to monotone queries over credentials the holder chose.

*Credentials and signatures*
- **Signature or proof:** a cryptosuite specifies algorithms for creating and verifying Data Integrity proofs, derived ones included. The issuer's Data Integrity proof is here a signature with its proof configuration (the proof without `proofValue`); say "signature" for modes and checks. "Proof" alone is the holder's query proof.
- **Keys; issuer-key authorisation:** the verification method (here a public key), its identifier (`verificationMethod`) and the key bytes (`publicKeyMultibase`) differ. Data Integrity authorises an issuer's key through a verification relationship (`assertionMethod`) in the issuer's controlled identifier document (VC-DI §2.6); our requests state trust requirements instead.
- **Trust requirements** (spec §6.3): `trustedIssuers` is a list of trust requirements; an issuer that meets one is a *trusted issuer*. The one defined type, `IssuerKeys`, gives an issuer and its verification methods, each a `Multikey` with `id`, `controller` and `publicKeyMultibase`, given rather than looked up, so that both sides prove and verify against the same keys. The other types (trusted lists, federation trust anchors, recognition credentials, trust frameworks) are reserved and not yet usable. The accepted cryptosuites are a separate member (`cryptosuite`). A trust requirement states the verifier's policy; it does not show that the issuer authorised the key. Say "trust requirements" and "trusted issuer"; never `issuers`, "issuer keys" for the request member, or "accepted issuer keys". Our prototype's requests list issuer keys, its form of an `IssuerKeys` requirement: say so where it matters (supplementary material).
- **`eddsa-rdfc-2022`:** Ed25519 signs the SHA-256 hash of the canonical proof configuration followed by that of the canonical document (EdDSA §3.2). It is unsalted, so whoever sees the hashes can test a guessed document.
- **`ecdsa-rdfc-2019`:** the same with ECDSA, P-256 with SHA-256 or P-384 with SHA-384. **`mldsa44-rdfc-2024`:** ML-DSA-44 with SHA-256, post-quantum; First Public Working Draft of Quantum-Resistant Cryptosuites v1.0, 16 June 2026.
- **`bbs-2023`, `ecdsa-sd-2023`:** the holder derives from the issuer's base proof a proof disclosing selected claims. Only `bbs-2023` derived proofs are unlinkable (VC-DI-ECDSA §5.1).
- **Our Merkle-based cryptosuites** sign the digest of decision 5. The suite draft specifies three members: `eddsa-sha256-merkle-2026` (SHA-256 tree, Ed25519), verified in the zkVM; `schnorr-poseidon2-merkle-2026` (draft identifier; Poseidon2 tree, Schnorr over Baby Jubjub), the member for Noir; and `mldsa44-sha256-merkle-2026` (SHA-256 tree, ML-DSA-44). Only the ML-DSA-44 member is post-quantum, and only the Ed25519 member is implemented (suite draft §1.2, §3.5, §5). Cite the draft as `merklesuites`.
- **Credential status:** whether the issuer has revoked or suspended the credential.
- **Validity period:** a credential's `validFrom` to `validUntil` (VC DM 2.0 §4.9); a query request's `validFrom` to `validUntil` (spec §4), the period in which the verifier accepts a presentation, "the request's validity period". Distinct from a proof's `created` and `expires` and from a key's validity. Never `notBefore` or `notAfter`.
- **Holder binding:** the party presenting is the subject, or controls a key bound to the credential.

*Request and answer*
- **Query request; answer presentation** (spec §3 to §5): both are RDF, serialised as JSON-LD 1.1 in compacted form with the spec's context. The answer presentation is a verifiable presentation (types `VerifiablePresentation` and `QueryAnswerPresentation`) that carries a proof instead of credentials, except in the disclosed mode. Never "an answer presentation is not a W3C verifiable presentation", and never "JSON" alone for either resource.
- **Request members** (spec §4): `query`, `inputCommitment`, `trustedIssuers`, `cryptosuite`, `signatureMode`, `proofMethod` (each entry with `method`, `verificationKey` and `parameters`), `maxPresentationBytes`, `maxResultSize`, `challenge`, `domain`, `validFrom` and `validUntil`, besides `type` and an optional `id`. Never `answerKind`, `input`, `issuers`, `signatureModes`, `proofMethods`, `limits`, `maxResultRows`, `audience`, `notBefore`, `notAfter`, a base IRI member or a version member.
- **Presentation members** (spec §5): `requestDigest`, `inputCommitment`, `signatureMode`, `revealedSignature` (revealed mode only), `result` (a SPARQL Query Results JSON document, for `SELECT` and `ASK`) or `resultGraph` (`CONSTRUCT`), and `proof` (`proofMethod`, `challenge`, `domain`, `proofValue`).
- **Request digest** (spec §3): SHA-256 over the RDFC-1.0 canonical N-Quads of the request; two JSON-LD documents with the same RDF content have the same digest. Our services compute it over a binary encoding instead: say so (§7.1).
- **`domain`:** identifies the verifier (the Data Integrity proof option); "the verifier's identifier" in prose; never "audience".
- **Input dataset chosen by the holder; agreed input dataset** (spec §6.2): if the request has no `inputCommitment`, the input dataset is chosen by the holder, who chose *D* when it answered and could have left a credential out. If it has one, the input dataset is agreed: the verifier agreed that dataset commitment before sending the request, and the presentation's `inputCommitment` must equal it. Short forms: "an input chosen by the holder", "an agreed input", "both kinds of input"; in tables H and A. "Who fixed the input" names the argument. Agreement shows that two commitments are equal, not that *D* is complete in the world. Never holder-declared, holder-selected, verifier-agreed or input kind.
- **Signature modes** (spec §6.4): hidden, revealed, disclosed. In the revealed mode the proof ties *D* to the signatures the verifier checks; in the disclosed mode the verifier evaluates *Q* itself.
- **Public-input rule** (decision 4; spec §7): substituting a returned solution into a query that is a single basic graph pattern gives triples the verifier can compute. Under `UNION` the matched branch is not computable, so its triple does not qualify. The proof must still show that each triple supplied as a public input is in *D* and signed.
- **Disclose, deliver, accept, mark as answered:** sending the presentation discloses the answer; delivery is its arrival, which a transport may split; the verifier accepts after every check, and accepts at most one presentation per challenge (spec §8); marking the request as answered, atomically and just before acceptance, enforces this and prevents replay. A rejected presentation has still disclosed its answer.

*Proofs and the zkVM*
- **Proof method** (spec §9): a way of producing and checking the proof, named by an IRI; a new version is a new method with a new IRI (`…:risc0-authenticated-rdf:v5`). Its evidence is a zero-knowledge proof, a proof that is not zero-knowledge, a TEE attestation or disclosed credentials. The verifier checks the evidence with the verification key and parameters that its request's entry for the method gives (`verificationKey`: a circuit's verification key, a zkVM image ID or an attestation root key), never with a key from the presentation. The proof system is the scheme: RISC Zero's STARK, optionally wrapped in Groth16, or UltraHonk. Our prototype names methods by an identifier and a separate version number; the artifact table reports that form.
- **Relation, statement, circuit:** the relation is the condition that public inputs and witness must jointly satisfy; the statement asserts that, for the given public inputs, some witness satisfies it; a circuit implements a relation as constraints (ours in Noir). Public inputs: the statement's values (spec §2: request digest, dataset commitment, result, signature mode and, in the revealed mode, the signed messages), each passed directly or bound by one digest. Witness: credentials, salts and, in the hidden mode, signatures.
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
- **Proved; executed without proving:** say which query features our proofs cover and which the guest program only executed without proving; never call an executed feature proved. A sweep case the evaluator admits is "admitted", one whose execution ends within the session limit "completed".
- **Baseline and public-triple circuits** (§7.4): the matched triple is part of the witness in the first, a public input in the second. Both prove only that each returned row is a solution, not that none is missing.

**Results:** "sound" and "complete" appear only when relating answers to prior work (zkRDF: soundness; Li et al.: correctness, completeness).

**Beyond the spec:** say so; never rename a difference away. The spec allows `SELECT`, `ASK` and `CONSTRUCT` queries that read only the input dataset: no `DESCRIBE`, `FROM`, `FROM NAMED` or `SERVICE`, and no `NOW`, `RAND`, `UUID`, `STRUUID` or argument-less `BNODE`. The evaluator without signature checks proved `DESCRIBE` and `FROM NAMED` answers; state once which triples a `DESCRIBE` returns. Its disclosure policy *d* has no spec member: drop it from the rule.

**Prior work:** make only claims that the prior-work research note (10 October 2026) lists as safe, and say exactly what each source supports, with a section or table pointer in the citation (`@braun26[Table 1]`). Never claim to be first to: run SPARQL, or negation or aggregates in `SELECT`, in a zkVM; check issuer signatures inside such a proof; offer zero-knowledge SPARQL over credentials; or prove absence or completeness in verifiable or zero-knowledge querying. Never claim that the earlier zkVM prototype cannot prove absence: it evaluated `SELECT` queries over whatever credentials the holder supplied, without stating that input, and had no `ASK`. Never claim "all of SPARQL", all four query forms, that prior work supports only basic graph patterns, or that we are faster than prior work. The authors' own preprint (zksparql.org) has an unknown status: mark where it would be cited (open question 3) and phrase the novelty so that it holds whatever that status.

## 4. Notation

- *Q*; *D*: the request's query; the input dataset.

Typst: `$Q$`, `$D$`.

**Remove:** `⟦Q⟧_D`, `Q'`, μ and μ(*t*) (an answer is now defined in words, as spec §6.1 does), eval(D(G), P), `⟦P⟧_D`, `C = ⟨m, o, q, f, a, s, e, d, b, t⟩`, `c`, `k`, `r`, `π`, `h(C)`, `E`, `K`, `ρ`, `p_C`, `σ`, `Com`, `Map`, `Anc`, `Src`, `Scp`, `Bnd`, `Ans`, `supp`, `⊑`, `tμ`, `V`; *n*, dom(μ) and `card` unless a formula needs them. No "Proposition" or "Design argument": each argument becomes short appendix prose with numbered assumptions (A1, A2, …), or goes.

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
- coverage, supports (for executions without proving) → admitted; completed; executed without proving

*Request and answer*
- contract; query contract; contract tuple → query request; stored request
- JSON (for the request or presentation); JSON canonical form → JSON-LD; the request's canonical N-Quads (RDFC-1.0)
- "not a W3C verifiable presentation" → the answer presentation is a verifiable presentation that carries a proof
- authority; input authority; provenance field; input kind → who fixed the input
- `HolderDeclared`; `HolderSelected…`; holder-declared → input (dataset) chosen by the holder
- `VerifierAgreed`; verifier-agreed → agreed input (dataset)
- anchor *k* → the agreed dataset commitment; the request's `inputCommitment`
- answer mode; exact path; `SelectedResults`; answer kind; Supported; Exact; Supported or Exact answer → answer; an answer to a monotone query; a closed-world answer
- scope *s*; graph catalog *K* → dataset construction
- result form *f* → query form
- request binding → request digest
- nonce; consume the challenge → challenge; mark the request as answered
- row bound; capacity profile K1, K2; `maxResultRows` → `maxResultSize`; one- or two-credential circuit
- `audience` → `domain`, "the verifier's identifier"
- `notBefore`, `notAfter`; validity window → `validFrom`, `validUntil`; the request's or the credential's validity period
- release; authorised release → disclose, never "return"
- release condition → the timing proposal (guide §6)

*Proofs, methods and signatures*
- method; query-proof method → proof method
- method descriptor; descriptor digest → the request's proof-method entry; its digest, never "image ID"
- registry; capability tuple → the proof methods the verifier accepts
- an IRI and a version → an IRI (a new version is a new IRI)
- pin; pinned image; guest pin; artifact → image ID; verification key; guest binary ("artifact" only for the research artifact)
- signature suite → cryptosuite
- key table; key policy; source evidence *e*; `None`; `issuers`; accepted issuer keys → the request's trust requirements (`trustedIssuers`); trusted issuers; no trust requirements
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
- non-monotone answer; negative answer → closed-world answer
- blank-node closure policy → describe once, with the `DESCRIBE` results
- answer-derived; public-eligible → computable from the stored request and the result; may be a public input
- public-data rule; the rule → public-input rule
- public triple → triple supplied as a public input
- obligation; secret work → what the proof must still show
- pre-output or abort privacy; disclosure analysis → what the verifier learns, before an answer or on abort
- override the open-world reading → a closed-world answer shows absence only within *D*

*Words with several senses*
- binding, bound, bounded → "bind" for proofs, commitments and holder binding; else size limit, range proof or capacity limit
- native → outside the zkVM; BBS+ proofs; reference evaluator
- accept, for keys → the request's trust requirements list keys
- canonical, outside RDFC-1.0 → only for a method's canonical encoding
- statement, for a bank document; record, for a credential → bank statement; credential
- the first prototype in this line → Wright [wright25dc]; "the earlier zkVM prototype"
- general SPARQL → only with the evaluator's exclusions stated nearby; never "all of SPARQL"

## 6. Fixed phrasings

Each appears in full once, in its home (rule 7).

- **Coverage** (§7.1; the abstract may say "over synthetic credentials"): "We used only synthetic credentials signed with test keys."
- **Development mode** (§7.1): "We generated all receipts with development mode disabled and verified them against the expected guest image ID."
- **Internal check** (§7.1; never name the check): "For experiments without † in Table 2, we recomputed hashes of source files, guest binaries and receipts. We compared the hashes and recorded test outcomes with the archived records. We did not verify the proofs again."
- **Audit** (§9.3): "The prototype has not undergone an external security audit."
- **Disclosure timing** (§5.3): "We propose that a party receive these public inputs no earlier than the answer from which it can compute them. The prototype does not enforce this condition."
- **Input chosen by the holder** (§3.3): "Over an input dataset chosen by the holder, a closed-world answer covers only the included credentials."
- **Unchecked properties** (§9.3): "The SPARQL evaluator running in the zkVM does not check credential status, holder binding or credential validity periods."

## 7. Decisions and open questions

**Decided** in the spec (#6786; spec v2 at `62000d63`, 10 October 2026, replacing the answer kinds and input kinds of `e312ad47`):

1. **Input dataset:** the credentials' graphs, RDF-merged into the default graph, or one named graph each if the method says so.
2. **Answers:** an answer is "a result SPARQL 1.2 permits over the input dataset" (spec §6.1); where SPARQL leaves a choice open, the holder makes it, and a method MAY fix and publish such choices. Supported and Exact answers are gone: whether an answer may be read as open-world follows from the query (spec §6.1, §10.1). A method that proves only that returned solutions are solutions can answer only queries for which that is a permitted result, such as a true `ASK`, or `SELECT DISTINCT … LIMIT k` with *k* solutions (spec §9).
3. **Inputs:** an input dataset chosen by the holder, or an agreed input dataset, signalled by an `inputCommitment` in the request (spec §6.2).
4. **Public inputs:** a method may make public any value the verifier can compute from its stored request and the result alone; no request member permits disclosure (spec §7).
5. **Merkle-based cryptosuites** sign a digest of the suite identifier, a 32-byte salt, the quad count, the Merkle root (one leaf per canonical quad) and the proof-configuration digest.
6. **Data model:** the request and the presentation are RDF, serialised as JSON-LD; the presentation is a verifiable presentation; the request digest is computed over the request's RDFC-1.0 canonical N-Quads; issuers are trust requirements (`trustedIssuers`); `validFrom` and `validUntil` replace `notBefore` and `notAfter`; `domain` replaces `audience`; proof methods are named by IRI, the version folded in; the spec depends on SPARQL 1.2 and RDF 1.2. The signature modes are unchanged.
7. **Wording:** "publicly verifiable", "designated-verifier", "monotone query", "a Data Integrity proof (a signature)", `verificationKey` (formerly `artifact`), "image ID"; no "pins".

**Resolved** (10 October 2026):

- **Poseidon2 message:** the suite draft (#6789) now has the Poseidon2 member sign `P(3, suite, salt, n, root, c1, c0)`, with `suite` a digest of `schnorr-poseidon2-merkle-2026`, so every member signs the fields of decision 5. The draft specifies all three members. The paper cites it (`merklesuites`) and the companion specification (`zksparqlspec`); both entries are anonymised in the double-blind build.

**Open:**

1. **Unpublished ISWC 2025 manuscript** (the earlier results-only interface): the paper now contrasts the published doctoral-consortium paper (Wright, ISWC 2025 Companion) and no longer describes the manuscript. Cite it anonymously, or leave it out?
2. **Choices our evaluator fixes:** which open choices of decision 2 does the evaluator fix, and where is that published? Until then, a verifier can rely on a unique result only for queries that leave SPARQL no choice.
3. **The zksparql.org preprint** (Wright, Shadbolt, Zhao, Zhao and Braun, labelled an ISWC 2026 submission; Noir circuits over Merkle-committed signed RDF, claiming `NOT EXISTS` and `MINUS` in the circuit for single-triple patterns): status unknown, author to confirm. The paper marks where it would be cited with `TODO(citation)`, and its novelty sentence claims no priority on proving absence.

## 8. Sources

**Writing.** The twelve guides behind guide §2, from Gopen & Swan (1990) to Pinker (2014), are listed with links in this file's first version (commit `3bdf74ae5`).

**Specifications.** SPARQL 1.2 Query, https://www.w3.org/TR/sparql12-query/ · SPARQL 1.1 Query, https://www.w3.org/TR/sparql11-query/ · RDF 1.2 Concepts, https://www.w3.org/TR/rdf12-concepts/ · RDF 1.1 Concepts, https://www.w3.org/TR/rdf11-concepts/ · RDF 1.1 Semantics, https://www.w3.org/TR/rdf11-mt/ · RDFC-1.0, https://www.w3.org/TR/rdf-canon/ · JSON-LD 1.1, https://www.w3.org/TR/json-ld11/ · VC DM 2.0, https://www.w3.org/TR/vc-data-model-2.0/ · VC-DI, https://www.w3.org/TR/vc-data-integrity/ · EdDSA, https://www.w3.org/TR/vc-di-eddsa/ · VC-DI-ECDSA, https://www.w3.org/TR/vc-di-ecdsa/ · BBS, https://www.w3.org/TR/vc-di-bbs/ · Quantum-Resistant Cryptosuites, https://www.w3.org/TR/vc-di-quantum-resistant/ · OpenID4VP 1.0, https://openid.net/specs/openid-4-verifiable-presentations-1_0.html · AnonCreds v1.0, https://anoncreds.github.io/anoncreds-spec/ · RISC Zero, https://dev.risczero.com/terminology

**Literature.** ZKProof Reference, https://docs.zkproof.org/reference.pdf · Thaler, https://people.cs.georgetown.edu/jthaler/ProofsArgsAndZK.pdf · Goldwasser, Micali & Rivest, *SIAM J. Comput.* 17(2), 1988 (unforgeability) · zkRDF (Braun, Wright & Käfer, ESWC 2026), https://publikationen.bibliothek.kit.edu/1000193675 · Wright (ISWC 2025 Companion), https://ceur-ws.org/Vol-4085/paper19.pdf · De Mulder, Dedecker, De Meester & Colpaert (ISWC 2025 Companion), https://ceur-ws.org/Vol-4085/paper81.pdf · Yamamoto, Suga & Sako, zk-SPARQL (talk, SCIS 2023), https://speakerdeck.com/yamdan/20230125-zk-sparql-pub · Li et al. (SIGMOD 2006), https://www.cs.bu.edu/~reyzin/papers/auth-db.pdf · ZKSQL (PVLDB 16(8), 2023) · VeriDKG (PVLDB 17(4), 2023)
