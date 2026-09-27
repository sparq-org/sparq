// [OPUS-4.8] sq-rvgr2.1 — the spec-factory registry, the /specs analogue of src/data/papers.ts.
// ONE source of truth for the /specs index, the per-spec static routes
// (generateStaticParams), the Cmd-K palette entry, and the build step
// (scripts/build-specs.mjs reads slug/source/title/shortName/status/date/editors from this
// file via parallel-array regexes — keep every field a SINGLE-LINE string literal).
//
// To register a new draft: add an entry here + a site/specs/<source>.typ authored with the
// helpers in site/specs/_lib/spec.typ. Everything else (index card, route, ToC, PDF + HTML
// build) is data-driven off this list.
//
// HONESTY: every draft in this series is a W3C-style *Unofficial Proposal Draft* — it has NO
// W3C standing and is NOT a standard (the Status-of-This-Document box on each page states this
// plainly, and the `status` label below never claims otherwise).

export type SpecStatus = "unofficial" | "cg-draft" | "draft";

export interface Spec {
  /** URL slug AND the basename of the built PDF/HTML artifacts. */
  slug: string;
  /** The .typ source under site/specs/ (relative path). */
  source: string;
  /** Document title — also injected into the doc header so it can't drift from this card. */
  title: string;
  /** ReSpec-style short name (a stable identifier for the draft). */
  shortName: string;
  /** Readiness/standing. NONE of these is a W3C standard. */
  status: SpecStatus;
  /** ISO date the draft was last built/edited. */
  date: string;
  /** Editors line, rendered in the doc header. */
  editors: string;
  /** One-line summary for the index card + page subtitle (not parsed by the build step). */
  blurb: string;
}

export const STATUS_LABEL: Record<SpecStatus, string> = {
  unofficial: "Unofficial Proposal Draft",
  "cg-draft": "Draft Community Group Report",
  draft: "Editor's Draft",
};

export const STATUS_VARIANT: Record<
  SpecStatus,
  "success" | "warning" | "muted" | "default"
> = {
  unofficial: "muted",
  "cg-draft": "warning",
  draft: "default",
};

export const SPECS: Spec[] = [
  {
    slug: "zksparql",
    source: "zksparql.typ",
    title: "zkSPARQL: Zero-Knowledge Query Proofs over SPARQL",
    shortName: "zksparql",
    status: "unofficial",
    date: "2026-07-01",
    editors: "Jesse Wright · the sparq project",
    blurb:
      "Proving SPARQL query answers over committed RDF graphs in zero knowledge — an explicit threat model, the committed data model, a scoped query fragment and circuit family, the ProofManifest format, and the fail-closed verifier obligations and audit gates. Research-grade and NOT externally audited (sq-qhy4).",
  },
  {
    slug: "mpc-sparql",
    source: "mpc-sparql.typ",
    title:
      "MPC-SPARQL: Secure Multi-Party Federated SPARQL — Requirements and Reference Architecture",
    shortName: "mpc-sparql",
    status: "unofficial",
    date: "2026-07-01",
    editors: "Jesse Wright · the sparq project",
    blurb:
      "A requirements and reference-architecture draft (not yet a protocol specification — the interoperable byte formats are explicitly open) for evaluating federated SPARQL across mutually distrusting sources under secure multi-party computation, with conformance categories separating what is built (M0–M3) from what is designed-not-built. Nothing is production-claimable today: the planned external audit clears the single-prover ZK layer only, and the MPC layer needs its own.",
  },
  {
    slug: "proposed-specifications-template",
    source: "proposed-specifications-template.typ",
    title: "SPARQ Proposed Specifications — Template",
    shortName: "sparq-spec-template",
    status: "unofficial",
    date: "2026-07-01",
    editors: "Jesse Wright · the sparq project",
    blurb:
      "The template and worked example every sparq Unofficial Proposal Draft follows — status notice, numbered sections, an RFC 2119 conformance section, a worked example, and references. Proves the single-source PDF + in-site render pipeline.",
  },
  {
    slug: "sparql-vector-genai",
    source: "sparql-vector-genai.typ",
    title: "SPARQL Vector & GenAI Extension",
    shortName: "sparql-vec-genai",
    status: "unofficial",
    date: "2026-07-01",
    editors: "Jesse Wright · the sparq project",
    blurb:
      "The vec: vector-search extension to SPARQL — magic-predicate k-NN patterns with score bindings, answer-exact vs approximate modes, filtered-search answer-safety, the persisted store format and its staleness contract, embedding acquisition, and grounded-generation obligations — plus a normative embedding-provenance record (model, version, metric, normalisation, dimension) with a MUST-reject compatibility rule and a deterministic tie-break. Every normative assertion carries a testable ID; an informative report states which requirements the sparq build satisfies today.",
  },
  {
    slug: "trust-expression",
    source: "trust-expression.typ",
    title:
      "Trust Expression: A Verifier-Holder Contract for Framework-Anchored Attestation Queries",
    shortName: "trust-expression",
    status: "unofficial",
    date: "2026-07-06",
    editors: "Jesse Wright · the sparq project",
    blurb:
      "The minimal verifier-holder contract for trust-framework-anchored questions — one SPARQL query, one RDF trust-requirements document, one nonce — with two trust modes (enumerated issuers; framework-certified issuers with certification-scope conformance, e.g. eIDAS 2.0 / UK DIATF), non-revocation and certification as positive time-windowed attestations (OWA-monotone, fail-closed), a normative RDF 1.2 reifier response encoding with a fixed named-graph mapping, and a reference rewrite into plain SPARQL that doubles as the conformance oracle. Framework trust bottoms out in a trust anchor, not cryptography; the ZK realisation is research-grade and NOT externally audited (sq-qhy4).",
  },
  {
    slug: "trust-graph-authz",
    source: "trust-graph-authz.typ",
    title:
      "Trust-Graph Authorisation for Solid/LWS: Pod-Side Admission of Certified Issuers",
    shortName: "trust-graph-authz",
    status: "unofficial",
    date: "2026-07-12",
    editors: "Jesse Wright · the sparq project",
    blurb:
      "Pod-side admission of certified issuers for Solid/LWS: a normative, fail-closed derivation from Control-gated anchor rules over signed, time-windowed, scope-attenuating certification edges to an effective trust-rule set, its five safety invariants (attenuation-only, no ambient edges, meta-scope non-escalation, deny-by-absence, strict additivity), and a stateless trust block on a WAC/ACP decision endpoint. Clear-path only — no ZK, privacy, or unlinkability claim; the signature estate is not externally audited (open gate sq-qhy4).",
  },
  // [GPT-5.6] sq-tag1q.4 — publish the greenfield SPARQL-CRDT proposal through /specs.
  {
    slug: "sparql-crdt",
    source: "sparql-crdt.typ",
    title: "SPARQL-CRDT: Conflict-Free Replicated RDF Datasets under SPARQL Update",
    shortName: "sparql-crdt",
    status: "unofficial",
    date: "2026-07-15",
    editors: "Jesse Wright · the sparq project",
    blurb:
      "A greenfield, implementation-ready proposal for named-graph-aware replicated RDF datasets: dotted add-wins observed-remove state, mandatory origin skolemisation, evaluate-at-origin SPARQL Update compilation, a canonical out-of-band delta journal, precisely scoped dataset convergence, and separate replica, delta-relay, and origin-evaluator conformance classes.",
  },
];

export function specBySlug(slug: string): Spec | undefined {
  return SPECS.find((s) => s.slug === slug);
}
