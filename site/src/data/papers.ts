// [OPUS-4.8] sq-gum8 — the paper-factory registry. ONE source of truth for the /papers
// index, the per-paper static routes (generateStaticParams), the sidebar nav, and the PDF
// build step (scripts/build-papers.mjs reads `slug` + `source` from this file).
//
// To register a new paper: add an entry here + a site/papers/<source>.typ. Everything else
// (index card, route, nav link, PDF + HTML build) is data-driven off this list.
//
// HONESTY: `status` is the paper-factory readiness verdict from
// research/paper-contributions-inventory.md. A C-family (ZK/MPC) paper would be `wip-arxiv`
// and must cite sq-qhy4; neither pilot is C-family. Numbers inside each paper are gated to
// `environment: canonical` by papers/_lib/bench.typ + scripts/build-papers.mjs.

export type PaperStatus = "publishable-now" | "wip-arxiv" | "draft";
export type PaperFamily = "A" | "B" | "C";

export interface Paper {
  /** URL slug AND the basename of the built PDF/HTML artifacts. */
  slug: string;
  /** The .typ source under site/papers/ (relative path). */
  source: string;
  title: string;
  /** One-line summary for the index card + page subtitle. */
  blurb: string;
  /** Authors; rendered as "anonymized" by the .typ when an anon build is requested. */
  authors: string;
  /** Target venue / track (honest target, not an acceptance). */
  venue: string;
  status: PaperStatus;
  family: PaperFamily;
  /** What the headline evidence is — surfaced on the card so the framing is honest up front. */
  evidence: string;
}

export const STATUS_LABEL: Record<PaperStatus, string> = {
  "publishable-now": "Publishable now",
  "wip-arxiv": "WIP · arXiv only",
  draft: "Draft",
};

export const STATUS_VARIANT: Record<
  PaperStatus,
  "success" | "warning" | "muted" | "default"
> = {
  "publishable-now": "success",
  "wip-arxiv": "warning",
  draft: "muted",
};

export const FAMILY_LABEL: Record<PaperFamily, string> = {
  A: "Systems / DB",
  B: "Semantic Web",
  C: "Crypto (WIP)",
};

export const PAPERS: Paper[] = [
  {
    // [OPUS-5] sq-gum8.3 REVISION 2 (adversarial PC panel — novelty / rigor / reproducibility
    // / clarity): "pre-registered" corrected to "specified" (no registry deposit exists — the
    // same correction odrl-policy-bridge took under PR #1330); the answer-safety scope
    // re-pointed at the path that actually carries it (the SPARQL `vec:` path is ANSWER-EXACT,
    // so pre≡post is unconditional there — the prior "approximate path preconditioned,
    // broad-mask" hedge described a path the SPARQL surface never reaches, and the stated
    // precondition was necessary-but-not-sufficient anyway); the 0.90 filtered recall floor
    // re-attributed to the approximate traversal it actually measures; the
    // sideways-information-passing lineage cited rather than implicitly claimed as novel.
    slug: "filtered-ann",
    source: "filtered-ann.typ",
    title:
      "Filter-as-Query: Filtered Approximate Nearest-Neighbour Search over SPARQL, where the Filter is an Exact BGP over the Engine's Own Dictionary Ids",
    blurb:
      "An RDF-native filtered-ANN integration: the filter on a vector-neighbour variable is the join-connected sub-BGP of the query itself, evaluated exactly by the engine and materialised as an id-mask over the shared dictionary-id space (no metadata mirroring, no boundary id translation). The pre≡post answer contract is enforced as machine-checked invariants and holds unconditionally on the path the SPARQL surface reaches, which is answer-exact — both physical strategies rank the complete admitted pool. The crate's approximate filtered traversal is a separate path the paper claims nothing for. Positioning is explicit that the constraint pushdown is a static instantiation of sideways-information-passing (magic sets, semi-joins, RDF-3X SIP) rather than a new strategy; related work covers the 2023–26 filtered-ANN wave incl. the engine-integrated systems (VBASE/NaviX) it is closest to. The performance evaluation is specified (baselines, workloads, falsification criteria) but unexecuted, pending the canonical runner.",
    authors: "Jesse Wright · the sparq project",
    venue:
      "EDBT short / demo or ESWC in-use/resources — results-free systems description; research-track submission deferred until the specified evaluation is executed",
    status: "draft",
    family: "A",
    evidence:
      "Deterministic only: recall floors (0.95 unfiltered, 0.90 approximate-filtered — sanity checks on a traversal the SPARQL surface does not reach, labelled as such) + asserted pre-filter ≡ post-filter equivalence (single / transitive / cyclic), unconditional because the SPARQL-level filtered path is answer-exact, + the cost-model crossover constant. A canonical invariant record that went false now aborts the paper build rather than rendering blank. No latency/throughput claim; the performance evaluation is specified (not pre-registered — no registry deposit exists) and unexecuted, blocked on the canonical runner.",
  },
  {
    slug: "solid-acl-conformance",
    source: "solid-acl-conformance.typ",
    title:
      "Library-Level Solid Access-Control Conformance: WAC and ACP Decision-Parity Ratchets Joining a Cross-Family Scoreboard",
    blurb:
      "A library-level conformance signal for both Solid access-control models: a per-construct, fail-closed-checked decision-parity corpus for WAC and for ACP, each ratcheted to a monotone floor and registered as a row of the single cross-family scoreboard with a drift guard. The honest evidence is conformance, not speed, and no security property is claimed; HTTP / CTH wire conformance is explicitly out of scope.",
    authors: "Jesse Wright · the sparq project",
    venue: "ISWC resources / in-use track (or demo)",
    status: "publishable-now",
    family: "B",
    evidence:
      "Deterministic conformance ratchet floors: Solid WAC decision parity (12) and Solid ACP decision parity (12), as the two newest rows of a 7-suite cross-family scoreboard totalling 3442, each floor CI-enforced, monotone, and guarded against drift. No latency claim and no security/soundness claim; library-level decision parity only, not HTTP/CTH wire conformance.",
  },
  {
    // [FABLE-5] sq-gum8.3-odrl REVISION 2 (PR #1330 review response): ONE track picked (ESWC
    // research track — the "in-use" framing is dropped: library-level, single-node, no
    // deployment/users); the WAC/ACP floors reframed as CONTEXT for the target layer, not
    // bridge evidence (the only direct bridge evidence is the four invariants); related work
    // extended to the decision-caching / authorization-recycling / materialised-view
    // neighborhood for C3; the conflict default corrected against ODRL IM 2.2 (`invalid` is
    // the spec default — the bridge hard-wires `prohibit` and cannot honour `perm`/`invalid`,
    // a first-class limitation); "pre-registered" corrected to "specified" (no registry
    // deposit exists); worked end-to-end example + artifact statement added. Status stays
    // `draft` until the §5.3 comparative study runs — the explicit bar for submission.
    slug: "odrl-policy-bridge",
    source: "odrl-policy-bridge.typ",
    title:
      "An ODRL Policy Bridge for SPARQL Access Control: Fail-Closed Compilation of Usage Policies into a Queryable Solid Access-Control View",
    blurb:
      "Compile, don't co-evaluate: a matched ODRL Permission/Prohibition compiles into the same triples the engine's existing, queryable WAC/ACP view already understands — no second enforcement engine, and every compiled decision is auditable, provenance-tagged RDF (the paper walks one policy end-to-end: Turtle policy → compiled auth/provenance triples → SPARQL audit query and its result). The contribution is the fail-closed lifecycle discipline compilation demands: ODRL's prohibit conflict strategy realised structurally by allow-minus-deny set subtraction (the perm and spec-default invalid strategies are not representable — a disclosed, first-class limitation), asymmetric three-valued deny retraction on policy refresh (positioned against decision caching, authorization recycling, Zanzibar-style consistent authorization, and materialised-view maintenance — not only ODRL enforcers), per-session re-checked conditional grants with safe one-shot fallback, and atomic count budgets. Evidence, honestly: four machine-checked invariants are the only direct bridge evidence; the WAC/ACP decision-parity floors are context for the pre-existing target layer, not bridge evaluation; the specified comparative decision-agreement study vs ODRE/OAC has not run (hence draft — not submittable until it has). The federated ODRL→MPC / ODRL-Duty→ZK composition is deferred, unbuilt, and claims nothing (sq-qhy4).",
    authors: "Jesse Wright · the sparq project",
    venue:
      "ESWC research track (policy) — draft; not submittable until the §5.3 comparative study runs",
    status: "draft",
    family: "B",
    evidence:
      "Honestly partitioned. Direct evidence (all of it, today): deterministic, test-proven answer-safety invariants of the bridge (prohibit-strategy set subtraction correct through unchanged enforcement; asymmetric fail-closed deny retraction; recipient constraints persist as re-checked conditional grants with one-shot fallback; atomic stateful count enforcement). Context — NOT bridge evidence: the CI-enforced, drift-guarded Solid WAC (12) and ACP (12) decision-parity ratchet floors of the pre-existing target layer. PENDING — the blocking gap to submission: a fully specified (not pre-registered: no registry deposit exists) decision-agreement study vs the ODRE enforcement engines and an OAC-style matcher over the cited systems' own policy corpora. No latency claim; no novel-semantics claim; no in-use claim; the federated/ZK disclosure half is explicitly deferred (research-grade crypto, not externally audited; sq-qhy4).",
  },
  {
    slug: "cozk-witness-validation",
    source: "cozk-witness-validation.typ",
    title:
      "Cannot Certify, So We Encode the Obligation: A Collaborative-zk-SNARK Re-Audit as a Witness-Validation Negative Result for Federated SPARQL",
    blurb:
      "An adversarial re-audit of an engine's intended collaborative (multi-prover) zk-SNARK path against the CRYPTO'25 (eprint 2025/1026) failure modes. The honest disposition is a negative result: the path is unbuilt, so soundness cannot be certified and every lens is RE-OPEN. The durable output is R-WV, a witness-validation-before-proving test obligation encoded as a build-time gate. NO security, privacy, or attestation property is claimed.",
    authors: "Jesse Wright · the sparq project",
    venue: "PoPETs / a security workshop (negative-result · security-engineering lessons)",
    status: "wip-arxiv",
    family: "C",
    evidence:
      "Negative result — asserts NO proven security/privacy/soundness/attestation property (the collaborative path is unbuilt: 6 proof/attestation entry points fail closed with NotYetImplemented). Committed structural counts only: 4 re-audit lenses, all RE-OPEN; a 5-clause R-WV witness-validation obligation encoded as a build-time gate; 12 prior single-prover findings under the open external-audit gate. Estate is research-grade and not externally audited; cites the gates sq-qhy4 (external single-prover audit, open) + sq-9hrn (coZK re-audit). No performance claim.",
  },
  {
    // [FABLE-5] sq-3kd2g.1 (epic sq-3kd2g / #1591) — the PURE SINGLE-PROVER zkSPARQL
    // ARCHITECTURE paper. Distinct from the two existing C-family papers by design (verified
    // gap, research/zksparql-fragment-extension.md §6): cozk-witness-validation.typ is the
    // COLLABORATIVE (multi-prover) negative result; verifiable-fed-sparql.typ is the SoK; THIS
    // is the single-prover SYSTEM (commitment scheme + fixed named circuit family + manifest
    // composition + verifier re-derivation + the provable fragment). Status `draft`: the
    // architecture + gate counts are real and traceable today, but the family is under the OPEN
    // external-audit gate sq-qhy4, so it is C-family and asserts NO proven property. Headline
    // evidence is DETERMINISTIC gate counts (bb ultra_honk from the regression-gated snapshot) +
    // structural verifier facts; NO wall-clock headline (work-box timings are non-canonical — a
    // proving-cost MODEL is published instead). Evidence keys: zkarch.* (canonical) +
    // cozk.single_prover_audit_issues (the 12 internal-audit findings).
    slug: "zksparql-architecture",
    source: "zksparql-architecture.typ",
    title:
      "A Single-Prover Zero-Knowledge Architecture for Verifiable SPARQL over Committed RDF Graphs",
    blurb:
      "The system design of a single-prover zero-knowledge stack that proves a SPARQL result is a genuine evaluation over committed, issuer-attested RDF graphs — without disclosing the graphs. It describes the pieces that make the prover-as-adversary model tractable: per-graph Poseidon2 commitments over the RDFC-1.0 canonical form bound to Schnorr issuer attestations; a fixed, named family of 13 circuit kinds (31 compiled members) each proving one operator instance of a monotone, open-world-conforming SPARQL fragment (BGP scans, datatype-bucketed value FILTER, a hidden-credential equality JOIN); a JSON manifest that composes sub-proofs through checkable binding edges; and a verifier that re-derives the circuit identity and the claimed statement from the query text and the relying party's trust anchors — a 12-obligation, 4-audit-gate fail-closed pipeline that trusts nothing the prover declares. Cost is reported as deterministic bb UltraHonk gate counts from the regression-gated snapshot plus a linear proving-cost model over them (no wall-clock headline: development timings are non-canonical, so the missing per-host constant is named rather than quoted). Security is stated under the OPEN external-audit gate: the verifier is internally re-audited (12 findings found and remediated, each pinned closed by a standing forge-negative regression test) but NOT externally audited, so no proven property is claimed and the internal audit is framed as necessary-not-sufficient.",
    authors: "Jesse Wright · the sparq project",
    venue:
      "PoPETs / a security or semantic-web privacy workshop — WIP; C-family, so arXiv-only until the external audit gate (sq-qhy4) closes and a canonical proving-time runner replaces the gate-count cost model",
    status: "draft",
    family: "C",
    evidence:
      "Deterministic artifact facts only, asserting NO proven security/soundness/privacy/attestation property (external audit gate sq-qhy4 OPEN). Canonical headline evidence: bb UltraHonk gate counts from the regression-gated snapshot (scan lattice 5991–34821; composable filter lanes gate-identical at 17416; join 7025–18681; revocation/hidden-issuer/holder members) + structural verifier facts (13 circuit kinds, 31 compiled members, 12 fail-closed binding obligations, 4 audit gates) + the internal-audit posture (12 confirmed findings, each pinned by a 1:1 forge-negative regression test). NO wall-clock headline — a linear proving-cost model over the gate counts is published because development timings are non-canonical (no canonical runner for this family). Cites sq-qhy4 throughout; hidden-holder tiers explicitly not-yet-sound; dual-leaf lane an accepted invariant downgrade.",
  },
  {
    slug: "fo-km-agent",
    source: "fo-km-agent.typ",
    title:
      "Formal Ontologies for LLM-Agent Knowledge Management: schema.org vs gUFO vs DOLCE",
    blurb:
      "A controlled pilot asking whether typing a project knowledge graph under a formal upper ontology helps an LLM agent answer knowledge-management questions — and which ontology. Holding instance data, agent, tasks, and grading fixed across four committed typing overlays in a fully-crossed single run, two overlays beat the untyped baseline — schema.org-as-top by the largest margin, DOLCE modestly — while gUFO fell below it. The overlay ranking is consistent with LLM training-data fluency, stated as a correlational hypothesis with its confounds named (overlay verbosity/closure noise, hand-authored overlay quality), not a demonstrated mechanism. Every accuracy figure is an indicative single-run measurement from one small agent model (Claude Haiku); a multi-run scale-up, to be pre-registered before it runs, is the stated gate before any venue submission.",
    authors: "Jesse Wright · the sparq project",
    venue:
      "ISWC / ESWC research (empirical) after the scale-up (to be pre-registered); K-CAP or an LLM+KG workshop as a pilot",
    status: "draft",
    family: "B",
    evidence:
      "Indicative-only pilot: deterministically-graded answer accuracy of a non-deterministic LLM agent (Claude Haiku, one fresh instance per condition-task pair) over a committed 16-task corpus fully crossed with 4 committed ontology-overlay conditions (bench/fo-km). The only canonical records are structural corpus counts (16 tasks, 4 conditions); every accuracy/abstention figure is environment=indicative (single run, heuristic gold-key grading, dev work-box) and is structurally barred from headline citation. No latency claim, no significance claim.",
  },
  {
    // [FABLE-5] sq-gum8.4 — Paper C-SoK: the systematization paper over the verifiable/
    // confidential federated-SPARQL estate (zkSPARQL single-prover lane + MPC-SPARQL lane +
    // attested-input binding). C-family, so wip-arxiv and it cites the open external-audit
    // gate sq-qhy4 throughout; it asserts NO security/privacy/soundness/attestation property.
    slug: "verifiable-fed-sparql",
    source: "verifiable-fed-sparql.typ",
    title: "SoK: Systematizing Verifiable and Private Federated SPARQL",
    blurb:
      "A systematization of verifiable and confidential federated SPARQL under a stated method (inclusion criteria plus a dated search protocol) along three independent axes — cryptographic mechanism (single-prover ZK over committed graphs / MPC evaluation over secret-shared graphs / attested-input binding), prover topology (one holder vs N mutually distrusting holders), and adversary model (semi-honest / covert / malicious, crossed with output guarantees under Cleve's bound) — axes assembled from established MPC and collaborative-proof taxonomy, applied jointly to federated SPARQL. Contributes an operator-level capability matrix cross-referenced to the published systems, exposing the disclosed/hidden two-regime split and the disclosed-key-join cost advantage global IRIs give RDF — analyzed together with its privacy price (the cleartext join disclosing exactly the cross-source linkage that source-unlinkability protects); a catalogue of settled negatives (Cleve, the post-quantum boundary of the signature and SNARK layers — with the hash-based Poseidon2 commitment binding stated as PQ-resilient, blank-node cross-graph joins, in-circuit entailment) distinguished from merely vacant cells; and a method-bounded statement of the open frontier — verifying issuer signatures over a secret-shared witness inside one source-unlinkable collaborative proof, a composition no publication in the searched peer-reviewed venues or preprint archives instantiates as of the stated survey dates. Anchored in one research-grade estate whose capability tiers are self-reported documentary provenance (a stated independence limitation): single-prover layer not externally audited (sq-qhy4, open), MPC layer semi-honest by default, collaborative-proof path unbuilt and fail-closed. No security property is claimed and no wall-clock number appears.",
    authors: "Jesse Wright · the sparq project",
    venue:
      "PoPETs / IEEE S&P SoK track — WIP; C-family, so arXiv-only until the external audit gate (sq-qhy4) and the frontier's publication-absence claims are re-verified at submission time",
    status: "wip-arxiv",
    family: "C",
    evidence:
      "Systematization — asserts NO proven security/privacy/soundness/attestation property and cites no wall-clock measurement. The only build-injected counts are the deterministic structural facts already gated as canonical for the C-family estate (fail-closed collaborative entry points, re-audit lenses all RE-OPEN, witness-validation obligation clauses, prior single-prover audit findings under the open gate sq-qhy4). Capability tiers are documentary (spec corpus + reconciled capability review), stated as such in the paper's limitations.",
  },
  {
    // [SONNET-4.6] sq-gum8.9 — Register the SPARQL logic-bug testing paper. Status is
    // `draft`: the merged harness (sq-gum8.6, crates/sparq-metamorph) is the committed
    // instrument, but the cross-engine bug-hunting campaign (bead sq-gum8.11) has NOT run
    // yet. No third-party bug is claimed anywhere; every campaign table in the .typ is an
    // explicitly marked PLACEHOLDER. Evidence keys wired: metamorph.selftest_oracles,
    // metamorph.selftest_seeds, metamorph.grammar_exclusion_seeds (all deterministic, canonical).
    // Campaign keys (confirmed/reported/rejected) are pending sq-gum8.11 and NOT wired here.
    slug: "sparql-logic-bugs",
    source: "sparql-logic-bugs.typ",
    title:
      "Reifying the Error: Metamorphic and Differential Logic-Bug Testing for SPARQL Engines",
    blurb:
      "SPARQL has no dedicated logic-bug testing work. We re-derive TLP and NoREC for SPARQL by reifying its third evaluation outcome — a type error, not a value — with the language's only error-absorbing forms, yielding a partition that provably recomposes the unpartitioned query under the SPARQL 1.1 spec. The merged instrument (crates/sparq-metamorph) includes a TLP + NoREC + differential oracle suite whose self-tests assert non-vacuity against a seeded wrong-result mutant on the real engine. This is a first draft against the merged instrument only; the cross-engine bug-hunting campaign has not run, every campaign table is an explicit placeholder, and the paper's honest publishability condition — previously-unknown, developer-confirmed bugs in third-party engines — is not yet met.",
    authors: "Jesse Wright · the sparq project",
    venue:
      "ISSTA 2027 (testing track) — first choice; PVLDB Vol 20 rolling or FSE 2027 — second choice. Draft; not submittable until the campaign (bead sq-gum8.11) yields confirmed third-party bugs",
    status: "draft",
    family: "A",
    evidence:
      "Deterministic instrument self-tests only (no campaign results yet): 3 oracle types (TLP, NoREC, differential) each asserting non-vacuity against a seeded wrong-result mutant on the real sparq engine; oracle correctness verified across 50 generated seeds on the pristine engine; grammar exclusion list verified across 200 seeds (no banned non-deterministic construct). Campaign-dependent evidence (bugs confirmed/reported/rejected) is pending bead sq-gum8.11 and is shown as an explicit PLACEHOLDER in the paper. No third-party bug is claimed.",
  },
  {
    // [HAIKU-4.5] sq-gum8.9 — Register the engine systems paper. Status is `draft`:
    // the architecture + substrate extraction + conformance breadth are traceable to the
    // codebase today; the submission-gating evaluation (bead sq-vw3ax.12) has NOT run, so
    // all competitive performance/memory figures are at-risk. Evidence records will be wired
    // when those measurements land; no records are added here.
    slug: "sparq-engine-systems",
    source: "sparq-engine-systems.typ",
    title:
      "One Substrate, Many Standards: An Out-of-Core SPARQL Engine and a Measured Zero-Overhead Evaluation Core Across the W3C/OGC Spec Families",
    blurb:
      "RDF triple stores historically force a speed/breadth/frugality three-way trade-off. This paper describes an out-of-core engine reaching for all three: it stores triples memory-mapped in six permutations with inline-tagged ids, evaluates queries via mixed binary/worst-case-optimal/bind joins, and extracts its evaluation core into a single substrate shared unchanged across SPARQL, OWL profiles, RIF, stream processing, GeoSPARQL, and SHACL — validated as behaviour-neutral by deterministic layout ratchets and a cross-family conformance scoreboard. Competitive performance and memory claims are gated on a canonical-host evaluation (not yet run) and are not asserted.",
    authors: "Jesse Wright · the sparq project",
    venue:
      "PVLDB Vol 20 rolling (monthly deadline through 2027-03-01); ICDE 2027 R2 (2026-11-11) or EDBT 2027 cycle 3 (2026-10-07) as alternatives. Draft; not submittable until the canonical-host evaluation (bead sq-vw3ax.12) yields the gated performance/memory results",
    status: "draft",
    family: "A",
    evidence:
      "The architecture (six permutation indexes, inline-tagged ids, mixed join families), the substrate extraction (single leaf crate consumed by all standards), and the conformance breadth (OWL RL/EL/QL, RIF, RSP, GeoSPARQL, SHACL, each pinned with a ratchet floor) are real and traceable to crates/sparq-core, crates/sparq-engine, crates/sparq-substrate, crates/sparq-reason*, crates/sparq-rsp, crates/sparq-geo, crates/sparq-shacl. Competitive performance and memory figures (latency vs native QLever/Virtuoso, bytes-per-triple vs HDT/qEndpoint, substrate zero-overhead delta, Sparqloscope and qEndpoint comparisons) are deterministic gating measurements on bead sq-vw3ax.12 and carry environment=canonical; they are pending and not claimed here.",
  },
];

export function paperBySlug(slug: string): Paper | undefined {
  return PAPERS.find((p) => p.slug === slug);
}
