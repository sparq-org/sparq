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
