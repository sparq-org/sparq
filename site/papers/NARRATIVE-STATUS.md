# Narrative restructure status (zksparql-architecture.typ)

Branch `claude/zk-paper-narrative`, following `opus-5-5-narrative-proposal.md`
(credentials-first). Commits are local only and unsigned (no SSH agent identity was available).

## Done
- Title: "Answers Instead of Documents: Exact Private SPARQL Answers over Verifiable Credentials"
  (manuscript and `site/src/data/papers.ts`; the papers.ts blurb was rewritten and no longer
  claims that authenticated exact proofs have no guest execution or proof).
- Main body restructured: 1 Introduction, 2 Background, 3 What an accepted answer means (RQ1),
  4 Architecture (RQ2, one figure), 5 Revealing less (RQ3, release condition as a proposed design
  requirement), 6 Prototype and evidence (one evidence-level table, pilot labelled indicative),
  7 Related work, 8 Discussion, 9 Conclusion. Main body is about 6,900 words including the
  abstract, one figure, three tables and one listing.
- Appendix (lettered, after the references): A formal relation and design arguments; B evidence
  detail and control inventories (hosted, adapter, controls, V5 native/guest/attempt/adapter/
  genuine receipt, V4, reproduction); C registry and capability tuples; D legacy fixed circuits;
  E native-composition experiment.
- Label check by script: every `#ref(<x>)` and `@x` resolves to a label or bibliography key; no
  duplicate labels. Honesty phrase patterns and `check-no-perf-numbers.py` pass on the .typ.

## Remaining
- Compile with `typst` (not installed here) and check page count against the venue limit.
- Possibly trim main body by roughly 500 words if the LNCS render exceeds 15 pages.
- Resolve the TODOs below.

## Evidence filled
- The authenticated false-ASK placeholder is replaced by the verifier-agreed payment receipt
  (`zkvcq.vcqp_*`, record `research/zk-paper-evidence/authenticated-vcq-payment-ask-audit.json`,
  from sparq-org/sparq#6651). It has not had a second internal evidence inspection; the text says so.
- The six CI declared cases (`zkvcq.ci_*`, records `authenticated-vcq-genuine-<case>-audit.json`
  from #6651, CI run 37340835239) are reported in section 6.3 and Appendix B, including why there
  are three guest pins. Not second-party inspected; the text says so.
- The holder-declared payment case (`zkvcq.vcqph_*`, from #6651 commit 6f756537) is reported too.

## Other TODOs
- `// TODO(citation)` (two places, intro and 2.4): the soundness-only interface is the
  unpublished ISWC 2025 manuscript. Decide whether and how to cite it under double-blind review.

## Open questions (from the narrative)
- Is there a realistic source of verifier-agreed anchors? If not, narrow the exactness claim.
- Is Braun a coauthor? This decides how zkRDF is framed.

## Length and cut plan
- Rendered at A4/11pt (Typst 0.15.0): main body about 15 pages, about 8,000 words; whole PDF 27 pages.
  Estimated 18 to 20 LNCS pages, against last year's 15-page limit (ESWC 2027 limit not yet published).
- Planned cuts, in order, to be confirmed against the real limit:
  1. Section 6 (evidence): keep the evidence table and one paragraph per campaign; move run detail,
     pins and control lists to the appendix or the website.
  2. Section 5 (revealing less): shorten the counterexamples to a table and tighten the release
     condition to its definition plus one example.
  3. Section 4 (architecture): trim the contract-field prose that repeats Appendix A.
  4. Background and related work: merge overlapping zkRDF and earlier-interface material.
- Check whether ESWC allows an appendix or supplementary material; if not, the appendix moves to the
  site page and an archived artifact.

## Inputs from other threads (10 October)
- Proof methods comparison (ten ways to show query results are correct): project file
  `zk-proof-methods/comparison.md`. Methods being built: full-disclosure re-evaluation (baseline);
  selective disclosure (bbs-2023 / ecdsa-sd-2023) plus re-evaluation, supported answers only;
  designated-verifier VOLE ZK (interactive, non-transferable, LPN); TEE attestation (TDX, SEV-SNP,
  Nitro, NVIDIA confidential computing). They will measure Q1-Q5 at n in {1, 4} for the paper's
  tables. Use for related work and the method comparison once the rewrite is done.
- Spec signature modes (spec #6786): `hidden`, `revealed`, `disclosed` (the last only for methods that
  hand over credentials or derived bbs-2023 / ecdsa-sd-2023 presentations). Spec section 9 evidence
  kinds: zero-knowledge proof; proof that is not zero-knowledge; attestation; disclosed credentials.
  Use these names in the paper.
- Revealed mode for zkVM eddsa-rdfc-2022 (#6787): discloses the verification method and the SHA-256
  hashes of the canonical document and of the proof configuration. Unsalted, so presentations of one
  credential are linkable and a low-entropy document can be confirmed by guessing.
- Motivation for the Merkle-root suite (Proof methods thread, project file
  `zk-proof-methods/ultrahonk-gate-breakdown.md`): in the current Noir u64/i64 result circuits about
  87% of about 490k gates is the literal range check. Literals are committed as a BLAKE3 hash of
  their text with a private digit count, so the circuit hashes all 20 possible lengths (about 54k
  gates per filtered value). A field-native leaf does the same comparison in about 300 gates. Cite
  only once frozen as an evidence record.
- Merkle-root suite specification: sparq-org/sparq#6789 (rendered draft in the project file
  `zk/zk-merkle-cryptosuite-draft.pdf`); its section 6 lists the security properties of each member,
  for the security table. Canonical Q1-Q5 query files: `zk/sparql-evaluator/fixtures/paper/` in #6789.
- Methods comparison input (project file `zk-proof-methods/quicksilver-vs-ultrahonk.md`, draft
  #6790): same Noir-compiled ACIR circuits (K copies of hidden_issuer_d4, K in {1, 4, 16, 32}) proved
  with UltraHonk and with QuickSilver (VOLE, designated verifier). At K=32: QuickSilver prover 8.8 s
  single-threaded vs UltraHonk 14.3 s on four threads; QuickSilver needs the verifier online and
  sends about 70 MB; UltraHonk proof 14.6 kB, verifies in 19 ms. QuickSilver is slower below about
  100k constraints (fixed setup about 0.8 s, 18 MB). Plain re-evaluation over four disclosed
  credentials (#6788) about 1.5 ms. Describe VOLE as "proposed, not built" (no transcript binding).
  Cite only once frozen as evidence records.
- Selective-disclosure baseline to be measured: the existing `native-rdf` feature in
  `zk/native-composition` (SELECT DISTINCT over a BGP; per-triple Dock BBS+ signatures, slot
  disclosure, verifier re-checks). It is not W3C `bbs-2023` (no JSON pointers, no HMAC blank-node
  labels); cite it with that caveat. No `bbs-2023` implementation is planned.
