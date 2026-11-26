// [OPUS-5.5] zkp-19 — full revision of the single-prover zkSPARQL paper (originally
// sq-3kd2g.1, epic sq-3kd2g / #1591) into a working paper on CONTRACT-BOUND proofs for SPARQL
// over credential-derived RDF. The earlier fixed-circuit architecture is retained as the
// LEGACY baseline (section "legacy"); the new material is the query/disclosure contract, the
// bounded exact-evaluator path, the disclosure-specialized (public-pattern) path and the
// query-over-VC method abstraction.
//
// HONESTY FRAME (empirical-honesty mandate + gate sq-qhy4, C-family / wip-arxiv): NO proven
// security, privacy, integrity or attestation property is asserted for any implementation.
// Design arguments are CONDITIONAL on named assumptions. Evidence levels are kept distinct:
// implemented source / executed native / executed guest / genuine verified receipt / external
// audit (none reached). The exact-evaluator and adapter PROOFS carry NO conventional
// issuer-signature authentication (source evidence None). The authenticated extension of the
// exact evaluator (V5) has passed scoped native model tests (8322) and, at a later source (42d),
// a scoped guest build with DIRECT guest executions but zero proofs/receipts in that gate; the
// first bounded low-level V5 proof attempt (42d) timed out INCOMPLETE (no receipt, no controls)
// and is retained unchanged. The generic V5 VCQ adapter (7fe8) passed a scoped native gate (no
// proof) and, in a SEPARATE bounded run at the same source, produced ONE genuine verified receipt
// for ONE synthetic verifier-agreed bag SELECT case with its controls (independently audited;
// zkvcq.vcqg_*). That closes authenticated-source genuine evidence for that fixture/profile only;
// the other declared cases are unproved and the method stays unavailable for its full tuple set.
// These gates and attempts are never summed with each other. The separate native experiment's
// finite/CLI proofs are native BBS+ public-BGP proofs (not exact SPARQL, not LegoGroth16
// composition proofs); tuple composition ran only in 2 legacy test functions. Counts from
// different campaigns (and overlapping test configurations) are never summed. Timings are
// INDICATIVE development measurements, shown only in one labelled pilot table via #ev(...),
// never co-tabulated with canonical counts.
//
// Single-source Typst. Every result number comes from paper-evidence.json through
// #headline(...) (canonical, json-pointer-bound to the frozen snapshots under
// research/zk-paper-evidence/) or #ev(...) (the indicative pilot only); none is hard-coded.

#import "_lib/bench.typ": headline, ev, provenance, authors, anon, paper_heading_numbering

#set document(title: "Toward Contract-Bound Proofs for SPARQL over Verifiable Credentials")
#set text(size: 11pt)
#set par(justify: true)
// Plain page numbers in the PDF footer (page set rules have no effect in HTML export).
#set page(numbering: "1")
// Figures may break across pages by default; short tables that must stay with their caption
// opt out locally with a scoped `set block(breakable: false)`.
#show figure: set block(breakable: true)
// Level-2 (==) headings are the top-level sections ("1.", "2.", ...), level-3 (===) are
// subsections ("1.1.", ...); the abstract is explicitly unnumbered.
#set heading(numbering: paper_heading_numbering)

// Short commit / digest rendering for bound identity records (first twelve hex characters).
#let short-id(key) = raw(headline(key).slice(0, 12))

// Fixed three-decimal rendering for the indicative pilot values (a display helper; the value
// itself always comes from the evidence file).
#let fmt3(x) = {
  let s = str(calc.round(x, digits: 3))
  let parts = s.split(".")
  if parts.len() == 1 { s + ".000" } else { s + "0" * calc.max(0, 3 - parts.at(1).len()) }
}
#let pilot(key) = fmt3(ev(key))

#align(center)[
  #text(size: 17pt, weight: "bold")[
    Toward Contract-Bound Proofs for SPARQL over Verifiable Credentials
  ]
  #v(0.2em)
  #text(size: 12pt)[
    Exact Evaluation, Disclosure Specialization and Method Dispatch — a Working Paper
  ]
]
#authors()

#align(center)[#text(style: "italic", size: 0.9em)[
  Working paper and design record under the open external-audit gate `sq-qhy4`. It reports
  implemented relations, and test and proof evidence for exact source commits that a second
  internal evidence inspection re-checked. It claims no proven security, privacy or integrity
  property for any implementation, and no component has had an external cryptographic review.
  The exact-evaluation proofs cover committed or holder-declared datasets _without_ conventional
  issuer-signature authentication, with one exception: the extension that adds such
  authentication, after native model tests, direct guest execution and a first proof attempt that
  timed out, produced one genuine receipt for a single synthetic verifier-agreed bag `SELECT` case,
  and none for its other declared cases. All timing figures are indicative development measurements.
]]

#heading(level: 2, numbering: none, outlined: false)[Abstract]

A verifier that asks a credential holder a SPARQL query must know which statement an accepted
proof establishes: that each released row is supported by some authenticated data, or that the
released result is the exact answer over a dataset someone has fixed. The two differ whenever the
verifier relies on an exact multiplicity, a false `ASK`, an aggregate, negation or a top-k answer,
and the second is only as strong as the authority that fixes the dataset. We define an explicit
query/disclosure contract — method, answer mode, query, result form, dataset authority, scope,
source-evidence requirement, disclosure policy, resource bounds and session binding — together
with a method abstraction that dispatches credential signature suites and query-proof methods
separately under verifier-owned policy, and a conservative rule for turning result-inferable
triples into public inputs while retaining authentication, membership and hidden-join
obligations; the rule extends the published selective-disclosure approach of Braun, Wright and
Käfer. At exact source commits, a bounded RISC Zero exact evaluator produced genuine receipts that
collectively cover bag `SELECT`, false `ASK`, `CONSTRUCT` and `DESCRIBE` and both authority
profiles, a protocol adapter bound such receipts to requests with before-consume validation
controls, a disclosure-specialized Noir relation produced genuine proofs in a paired development
pilot that shows only small differences, and an authenticated extension of the exact evaluator
passed native model tests and direct guest execution and, after a first bounded proof attempt
timed out, produced through its adapter one genuine receipt for a single synthetic
verifier-agreed bag `SELECT` case. Retained genuine receipts for the other query-form/authority
profiles that its relation implements, conventional suites beyond this RDFC-1.0 Ed25519 case
(including JSON-signed credentials and a JSON-to-RDF mapping), authenticated linkage between
native credential proofs and circuits, and a cost-based planner remain open, so this is a design and partial-evidence contribution rather than a validated system.

== Introduction <intro>

A holder of verifiable credentials @vcdm2 is usually asked a question rather than for a
document. A lender wants to know whether an applicant is employed by an approved organisation
and earns above a threshold; a licensing body, whether any recorded disqualification exists; a
training provider, how many accredited courses were completed. SPARQL @sparql11 states such
questions directly over RDF-shaped credentials, and zero-knowledge proofs promise answers without
handing over the credentials. What is less obvious is _which statement_ a verifier obtains when
it accepts such a proof.

Two statements are routinely conflated. _Result membership_ says that every released row is
supported by some authenticated data the holder possesses. It suits positive questions: once Alice
is shown to be an Acme employee earning above the threshold, further credentials cannot undo it.
_Exact evaluation_ says that the released result is the complete answer of the query over a
stated dataset. It is needed whenever the verifier relies on what is absent or on exactly how
often something occurs — an exact number of duplicate rows under bag semantics, an `ASK` that
returns false, a `COUNT`, a `NOT EXISTS` or `MINUS`, an `ORDER BY … LIMIT` that claims the latest
entry. Exactness is itself
only meaningful relative to a dataset someone has fixed: a holder who chooses which credentials
enter the dataset can make "no disqualification recorded" true by omission.

Three further difficulties make the problem more than an engineering exercise. _Evaluation and
authentication decouple easily._ A general-purpose zkVM proves that a program ran over the bytes
it received, but not who signed those bytes; a native credential proof authenticates disclosed
terms but cannot establish an absence. _Signed representations differ from queried ones._
Credentials are signed as canonical N-Quads @rdfc10 @vcdieddsa, as JSON, or as BBS messages
@bbs, while the query runs over an RDF dataset; the verifier must know that the data queried is
the data that was signed. _Secret proof work is expensive_, and many facts are already fixed by
the public query and the released answer; removing them from secret work is attractive and easy
to get wrong.

The closest published work, zkRDF by Braun, Wright and Käfer @braun26, building on RDF-based
selective-disclosure semantics @braunkaefer25, already reveals query constants and projected
terms, hides the remaining terms behind blank nodes, and proves issuer signatures, hidden
equalities and numeric bounds with native primitives; its authors state that it cannot prove
non-existence and so cannot support `MINUS` or a false `ASK`. The earlier single-prover
architecture of this project (§#ref(<legacy>, supplement: none)) proves result membership with a
fixed family of Noir circuits. Neither states exact complete-input semantics relative to a dataset
authority, nor a contract recording which signature suite, representation mapping and query-proof
method a verifier relied upon.

*Message.* A verifier should reason about an explicit query/disclosure _contract_; a prover should
pay proof cost only for the _secret-dependent authenticated obligations_ that remain under that
contract. The long-term programme goal is the full SPARQL 1.1 read semantics (all four query forms
and the full algebra), with SPARQL 1.2 @sparql12 as an optional extension; this paper does not
narrow that goal to basic graph patterns, and it does not claim that any current path reaches it.
We organise the work around three questions:

- *RQ1 — statement.* What statement should a verifier accept for a SPARQL query over
  credential-derived RDF, and when does it require exact evaluation over a complete input fixed
  by a stated authority?
- *RQ2 — minimization.* Which proof obligations can be made public or removed under an explicit
  contract without weakening that statement or the disclosure policy?
- *RQ3 — integration and evidence.* How can signature suites and query-proof methods be combined
  behind one verifier-owned contract, and what does current evidence show about the correctness
  controls and costs of the implemented paths?

This working paper contributes:

- *A contract and relation* (§#ref(<contract>, supplement: none)): a contract tuple with an
  explicit answer mode and a request/method/result relation over a method-indexed commitment that
  separate selected-result membership from exact evaluation, with worked cases for bag
  multiplicity, false `ASK`, aggregates, negation and top-k; HolderDeclared
  and VerifierAgreed authority semantics; and conditional design arguments that name every
  assumption instead of claiming implementation soundness.
- *A query-over-credentials method abstraction* (§#ref(<method>, supplement: none)): separate
  versioned dispatch of signature suites and query-proof methods, verifier-owned capability
  tuples, a same-data linkage requirement between the signed representation and the queried RDF,
  and a before-consume validation order for method, scope, result, challenge, audience and
  expiry. It is an integration design, not a new cryptographic primitive.
- *A conservative public-triple rule* (§#ref(<minimize>, supplement: none)): an eligibility
  condition for moving result-inferable triples from secret witnesses to public inputs,
  implemented in one bounded Noir relation and proposed for exact relations, with counterexamples
  (`OPTIONAL`, `UNION`, blank nodes, value equality, omission) and the authentication, membership
  and hidden-join obligations that remain. It extends, and credits, the disclosure method of
  @braun26.
- *An evidence account* (§#ref(<evidence>, supplement: none)): three proof campaigns at exact
  commits, each re-checked by a second internal evidence inspection — a bounded exact evaluator
  with #headline("zkvcq.exact_hosted_receipts") genuine receipts, a protocol adapter with
  #headline("zkvcq.adapter_receipts") genuine receipts and #headline("zkvcq.adapter_controls")
  retained control records, and a public-pattern relation with
  #headline("zkvcq.pp_genuine_proofs") genuine proofs — plus native and direct guest-execution
  evidence for the authenticated extension, native tests of its adapter and
  #headline("zkvcq.vcqg_genuine_receipts") genuine adapter receipt for a single synthetic case,
  a separate native-composition experiment and a separately labelled
  indicative pilot, with the evidence level of every path stated
  (§#ref(<paths>, supplement: none)).

The current evidence answers each question only in part (§#ref(<discussion>, supplement: none)).
For RQ1 it shows exact-result relations executing with genuine receipts, but, apart from one
synthetic case, over datasets whose issuer provenance is not proved. For RQ2 it shows one
implemented specialization and a pilot too small to establish a general speedup. For RQ3 it shows
contract dispatch and validation controls for one method family, and issuer authentication in
front of the exact evaluator executed natively, directly in the guest and, for that one case, in
a genuine receipt, while authenticated exact proofs for the other declared cases and authenticated
linkage across proof systems remain open.

== Three proof paths and an evidence vocabulary <paths>

The project contains three proof paths that must not be read as one system, and a separate
native-composition experiment. The _legacy fixed-circuit path_ (§#ref(<legacy>, supplement: none))
proves result membership for a monotone fragment with a fixed family of Noir circuits over
Poseidon2-committed graphs. The _bounded exact-evaluator path_ runs a SPARQL evaluator for a
bounded input inside the RISC Zero zkVM @risc0 and journals a contract-bound exact result; a
protocol adapter wraps it in typed requests and verification, and an _authenticated extension_
(V5) places Ed25519 verification of RDFC-1.0 credentials in front of the same evaluator, with a
generic adapter of its own. The
_disclosure-specialized path_ keeps secret-dependent obligations in specialized Noir relations and
moves result-inferable data to public inputs; its one implemented relation is the bounded
public-pattern relation V4. The _native-composition experiment_ combines BBS+ credential proofs
with Circom circuits and exposes a public-BGP interface over RDF credentials
(§#ref(<composition>, supplement: none)); it does not evaluate SPARQL exactly.

Claims about these paths are graded on five levels, which the rest of the paper keeps distinct:

/ Implemented source: code exists at a pinned commit.
/ Executed native: host tests of the model or relation ran and passed.
/ Executed guest: the zkVM guest or circuit executed, possibly without producing a real proof.
/ Genuine verified receipt: a real proof was produced (development modes refused) and verified
  against a pinned image identifier or verification key.
/ External audit: independent cryptographic review. No path has reached this level (`sq-qhy4`).

A _second internal evidence inspection_ — a separate project-internal check that re-hashes
sources, artifacts and receipts from a frozen snapshot — is weaker than the last level: it checks
that the reported runs happened at the stated commits, not that the relations achieve their
goals, and it is not an external security review.

#[
#show figure: set block(breakable: false)
#figure(
  table(
    columns: (1fr, 1.3fr, 1fr, 1.2fr),
    align: (left, left, left, left),
    table.header[Path][Relation][Source authentication][Highest level reached],
    [Legacy fixed circuits],
    [Result membership, monotone fragment, committed graphs],
    [Issuer Schnorr signature over the graph commitment],
    [Implemented, regression-gated gate counts, internal audit; no new runs here],
    [Exact evaluator (#short-id("zkvcq.exact_source_commit"))],
    [Exact bag `SELECT`, `ASK` incl. false, `CONSTRUCT`, `DESCRIBE`; HolderDeclared and VerifierAgreed],
    [None],
    [Genuine verified receipts; the receipts collectively cover these forms and both authorities],
    [Protocol adapter (#short-id("zkvcq.adapter_source_commit"))],
    [Contract-bound exact `SELECT`/`ASK`/`CONSTRUCT` requests],
    [None (source evidence `None`)],
    [Genuine verified receipts],
    [Authenticated extension V5 (#short-id("zkvcq.v5_source_commit"), #short-id("zkvcq.v5g_source_commit"))],
    [Exact evaluation over Ed25519/RDFC-1.0-verified documents],
    [Ed25519 over RDFC-1.0, verifier-owned key table],
    [Executed native (model tests); executed guest (direct execution, no proof); first low-level proof attempt incomplete, no receipt; one receipt via the adapter (next row)],
    [Authenticated adapter (#short-id("zkvcq.vcqa_source_commit"))],
    [Contract-bound exact `SELECT`/`ASK`/`CONSTRUCT` requests over V5],
    [As V5, bound to a verifier-owned policy digest],
    [Executed native (host tests); one genuine verified receipt for one synthetic VerifierAgreed bag `SELECT` case only; other declared cases unproved],
    [Public pattern V4 (#short-id("zkvcq.pp_source_commit"))],
    [Selected `SELECT DISTINCT` rows (set inclusion; no multiplicity, completeness or `ASK`) for a fully public BGP pattern, bounded one- and two-credential profiles],
    [Issuer signature and membership inside the relation],
    [Genuine verified proofs],
    [Native public RDF (#short-id("zkvcq.nc_source_commit"))],
    [Public BGP triples reconstructed from signed slots; not exact SPARQL],
    [BBS+ over BLS12-381, verifier-owned issuer and status policy],
    [Genuine verified native BBS+ proofs in a finite declared domain; no Noir linkage],
    [Tuple composition (same source)],
    [Same-field BBS+ and Circom/LegoGroth16 tuple composition],
    [BBS+ over BLS12-381],
    [Executed native in two legacy test functions; proof count not enumerated; no Noir linkage],
    [Cross-backend linkage, planner, fusion, JSON mapping],
    [—], [—], [Design only],
  ),
  caption: [
    Proof paths and the evidence level each has reached. Read the last column before any other
    claim: only one exact receipt, for a single synthetic case, carries conventional credential
    authentication, the rows are separate systems whose counts are never combined, and no row
    reaches external audit.
  ],
) <paths-table>
]

== Contracts: what a verifier accepts <contract>

=== Membership versus exact evaluation <semantics>

Consider a holder with an employment credential from Acme (`ex:alice a ex:Employee`,
`ex:alice ex:employer ex:Acme`, an income literal), two course credentials from different
providers, and possibly a registry credential stating `ex:alice ex:hasDisqualification ex:case42`.
Each course credential describes its own completion record, `ex:rec1` in one and `ex:rec2` in
the other, each with `ex:person ex:alice` and `ex:course ex:SecurityBasics`.
Assume the contract's scope explicitly forms the default graph as the union of the credential
graphs. Write $⟦P⟧_D$ for the multiset of solution mappings of pattern $P$ over dataset $D$ under
the SPARQL algebra @pag09 @sparql11, and $⊑$ for sub-multiset inclusion.

- *Bag multiplicity.* `SELECT ?c WHERE { ?completion ex:person ex:alice ; ex:course ?c }` has two
  solutions that differ only in `?completion`; projection removes the record and leaves two
  identical rows `?c = ex:SecurityBasics`, which `DISTINCT` would collapse. The distinct record
  IRIs matter: had both credentials stated the identical triple
  `ex:alice ex:completed ex:SecurityBasics`, the union would contain it once, because an RDF graph
  is a set of triples, and the query would return one row.
- *False `ASK`.* `ASK { ex:alice ex:hasDisqualification ?x }` is false exactly when no matching
  triple is in the evaluated dataset. Selected-result membership cannot express a false answer.
- *Aggregates.* `SELECT (COUNT(?c) AS ?n) WHERE { ?completion ex:person ex:alice ; ex:course ?c }`
  returns two only over input that contains both records; over a subset it returns a smaller,
  equally "supported" number.
- *Negation and top-k.* `FILTER NOT EXISTS { … }`, `MINUS` and `ORDER BY DESC(?date) LIMIT 1` can
  each be changed by data that was never shown: a single additional triple may remove a row or
  displace the latest entry, although not every addition does.

For a positive pattern — a basic graph pattern with joins, filters and projection — bag evaluation
is monotone under sub-multiset inclusion: $D ⊆ D'$ implies $⟦P⟧_D ⊑ ⟦P⟧_(D')$. Mathematically, a
row and a lower bound on its multiplicity obtained by evaluating over a fully known authenticated
subset therefore remain valid over all of the holder's data; the completion example yields "at
least two" when evaluated over any input containing both records. That lower bound is available to
whoever knows the whole authenticated supporting subset; it is not what a selected-results
presentation establishes. `SelectedResults` (§#ref(<contract-tuple>, supplement: none)) shows only
that each released distinct row occurs in the answer, and repeated witnesses for one row prove no
further multiplicity. Monotonicity does not protect _exact_ claims: that the course appears
exactly twice, that the count is two, that no disqualification exists, or that an entry is the
latest. When a verifier relies on such a claim, it needs a statement that is exact over a
specified dataset, and that dataset must be fixed by an authority the verifier accepts.

The engine's own W3C SPARQL conformance floor (#headline("conformance.sparql_floor") passing
assertions) measures the production engine; it is not evidence about the bounded proved
evaluator, whose coverage is established only by the tests and receipts in
§#ref(<evidence>, supplement: none).

=== The contract tuple <contract-tuple>

A request carries an explicit contract
$ C = ⟨ m, o, q, f, a, s, e, d, b, t ⟩ $
whose components are:

/ $m$ — method: query-proof method identifier, version and descriptor digest (§#ref(<method>, supplement: none)).
/ $o$ — answer mode: `SelectedResults` (every released distinct row is supported; no multiplicity
  or completeness) or `Exact` (the released
  result is the complete answer). A method may fix $o$; the adapter's methods fix `Exact`.
/ $q$ — query: the query text and declared language version (SPARQL 1.1; SPARQL 1.2 optional).
/ $f$ — result form: bag `SELECT`, set `SELECT`, `ASK`, `CONSTRUCT` or `DESCRIBE` with its closure
  policy, and the canonical result encoding.
/ $a$ — authority: HolderDeclared, or VerifierAgreed with an anchor $k$.
/ $s$ — scope: default-graph and named-graph construction, including `FROM` and `FROM NAMED`.
/ $e$ — source evidence: `None`, or accepted signature suites, issuer/verification-method/key
  table, representation mapping, status requirement and holder-binding requirement.
/ $d$ — disclosure policy: what the verifier may learn beyond the result (issuer identities,
  credential count, capacity profile, method).
/ $b$ — bounds: row, triple and capacity limits.
/ $t$ — session: challenge, audience and validity window.

A presentation returns a result $r$ and a proof $pi$ whose public output (the journal, for a zkVM
method) binds the contract, a dataset commitment $c$ and $r$ or its digest. Conceptually the
contract enters as a digest $h(C)$; an implementation may realise that binding indirectly
rather than as a literal journal field. In the adapter, the verifier's stored method descriptor
and request binding determine a derived nonce, which enters the model request whose digest the
journal carries, so the session bytes in $t$ are cryptographically bound to the proof (under
assumptions A1 and A3 of §#ref(<arguments>, supplement: none)). The
verifier additionally checks the validity window and audience of $t$ on the host and consumes the
challenge (§#ref(<validation>, supplement: none)).

The commitment and the queried dataset are method-defined. Let $E$ be the signed or source
encoding of the credentials, $K$ the graph catalog that names default and named graphs, and $rho$
a salt or other auxiliary witness where the profile requires one. Let $p_C$ be the publicly known,
method-specific commitment parameters that the method extracts from $C$ — for example an
authorization table, evaluation policy or representation mapping, as applicable; $p_C$ is empty
where a method binds none. A method fixes a commitment function $"Com"_m (E, K, rho; p_C)$ and,
separately, a dataset mapping $D = "Map"_m (E, K)$ from that encoding to the RDF dataset the query
runs over. Request version 3 of the exact evaluator commits source bytes and the graph catalog;
the authenticated extension V5 commits the canonical authenticated documents together with the
verifier's authorization table as part of $p_C$. The commitment need not bind the whole contract:
the query, challenge and other session data are bound, where they are bound, through the request
and descriptor digests rather than through $"Com"_m$. Keeping $"Map"_m$ explicit prevents a
signature over one representation from being read as a signature over an abstract dataset $D$.
With public input $(C, c, r)$, the intended relation is
$ ((C, c, r), w) ∈ R_C quad ⟺ quad
  & w = (E, K, rho, sigma, x) \
  & ∧ c = "Com"_m (E, K, rho; p_C) ∧ D = "Map"_m (E, K) \
  & ∧ "Anc"_a (c) ∧ "Src"_e (E, sigma) ∧ "Scp"_s (D) \
  & ∧ "Bnd"_b (D, r) ∧ "Ans"_o^f (r, ⟦q⟧_D) $
where $sigma$ are credential signatures, $x$ further auxiliary witness data,
$"Anc"_"HolderDeclared" (c)$ is always true, $"Anc"_("VerifierAgreed"(k)) (c)$ holds iff $c = k$,
and $"Src"_"None"$ is always true.

The answer predicate depends on the mode and the form. $"Ans"_"Exact"^f$ requires $r$ to equal the
answer under the method's canonical equality for $f$: multiset equality for an unordered table,
the method's ordering and tie rules for `ORDER BY`, `LIMIT` and `OFFSET` results, boolean equality
for `ASK`, and the method's canonical graph equality, with its blank-node and closure policy, for
`CONSTRUCT` and `DESCRIBE`. $"Ans"_"SelectedResults"^f (r, R)$ is $r ⊆ "supp"(R)$, where $r$ is a
set of distinct rows and $"supp"(R)$ is the set of rows occurring in $R$ at least once. It is
defined only for unordered distinct-set (`SELECT DISTINCT`) results of a positive pattern and
asserts neither multiplicity nor completeness; it is not a relation for bag tables, booleans,
graphs or ordered results. A true `ASK` over a positive pattern is conceptually a separate
positive-existence statement, supportable by exhibiting one supporting solution rather than by
table membership; the implemented selected-results relation V4 does not support `ASK`. A false
`ASK` needs `Exact`.

=== Dataset authority <authority>

*HolderDeclared* makes the result exact over the dataset the holder chose to commit. A false
`ASK` then means "the dataset I committed contains no match" — not "my wallet contains no match".
It is useful when the commitment is reused elsewhere (so omission is detectable later) or when the
verifier only needs internal consistency, and it must never be presented as wallet completeness.

*VerifierAgreed* makes the result exact over a dataset whose commitment equals an anchor $k$ the
verifier accepted _independently of this presentation_: published by an issuer or registry that
vouches for completeness of a record set, fixed in an earlier session under the verifier's
control, or co-signed by a party the verifier trusts for that purpose. A verifier that approves
whatever root the holder sends in the same exchange obtains nothing beyond HolderDeclared. How
anchors are obtained is deployment policy outside the implemented code; the implementation only
checks that the proved commitment equals the anchor in the request.

=== Conditional design arguments <arguments>

*Design argument 1 (exact result under a contract).* Assume (A1) knowledge soundness of the
receipt system for the pinned guest image; (A2) functional correctness of the guest evaluator:
for every input admitted by bounds $b$ it computes $"Map"_m (E, K)$ and the canonical form-$f$ encoding of $⟦q⟧_D$ under the declared
semantics;
(A3) binding of the dataset commitment $"Com"_m$, and collision resistance and domain separation
of the hash functions that form the request and method-descriptor digests; and (A4) that the
verifier accepts only if the
journal's request binding, authority, anchor and scope match its stored request and the result
claimed in the response equals the result in the verified journal — the verifier does not know
the answer in advance, so it compares against the journal, not against its request. Then
acceptance implies, except with the failure probabilities of A1 and A3, that some $(E, K, rho)$
with $"Com"_m (E, K, rho; p_C) = c$ satisfies $"Ans"_"Exact"^f (r, ⟦q⟧_D)$ for $D = "Map"_m (E, K)$;
under VerifierAgreed, additionally $c = k$. Nothing follows about issuers unless $e ≠ "None"$ and
(A5) $"Src"_e$ is enforced inside the proved relation. A2 is supported by tests
and native replay (§#ref(<evidence>, supplement: none)), not proved; A1 and A3 are assumptions on
third-party components; A4 is implemented and exercised by the adapter's controls. These are
design arguments, not a proof that the implementation meets them, and they carry no weight beyond
their named assumptions while `sq-qhy4` remains open.

== A method abstraction for queries over credentials <method>

=== Two dispatches, not one <dispatch>

A _signature suite_ says how an issuer's bytes were signed: for example the EdDSA Data Integrity
suite over RDFC-1.0 canonical N-Quads @vcdieddsa @rdfc10, a JSON-based signature over a JSON
serialization, or BBS over RDF statements @bbs. A _query-proof method_ says what relation is proved
about the query and on which backend: the exact evaluator on RISC Zero, the public-pattern relation
on Noir/UltraHonk @noir @barretenberg, or the legacy fixed family. The two vary independently, and
conflating them hides the obligation that joins them — that the dataset the method evaluates is
the dataset the suite authenticated. Each is therefore identified by a versioned identifier and a
descriptor digest held in a verifier-side registry; a presentation that names an unregistered
method or a mismatched digest is rejected before any receipt is examined.

=== Verifier-owned policy and capability tuples <capabilities>

The verifier, not the holder, decides what it will accept. Its policy is a set of _capability
tuples_ $(m, o, f, a, e, "status", "holder")$: a method, answer mode, result form, authority,
source-evidence class, status-checking requirement and holder-binding requirement. A request is
valid only if its contract instantiates an accepted tuple; a method advertises only tuples it
implements and tests. The protocol adapter currently implements the tuples with $o$ = `Exact`, $f$
in bag `SELECT`, `ASK` and `CONSTRUCT`, $a$ in HolderDeclared and VerifierAgreed, source evidence
`None`, status
`NotRequested` and a bearer holder (@adapter-table). These tuples say what was proved about
evaluation; they say nothing about who issued the data. A separate authenticated adapter declares
the same forms and authorities with source evidence restricted to strict, bounded canonical-RDF
EdDSA under a verifier-owned policy. It has passed native tests and has one genuine receipt, for a
single VerifierAgreed bag `SELECT` case; until its remaining declared cases have genuine coverage,
its registry entry conservatively does not offer these tuples as available
(§#ref(<v5-evidence>, supplement: none)).

=== Issuer authentication and same-data linkage <linkage>

Authenticating a credential and querying it are linked by one requirement: _the RDF dataset
queried must be derived, inside the proved relation or under an explicitly trusted mapping, from
exactly the bytes whose signature was verified_. For RDF Data Integrity with RDFC-1.0, the signed
message is derived from the canonical N-Quads of the document and its proof configuration, so the
queried dataset can be those same canonical quads. For JSON-signed credentials the signed bytes are
JSON; producing RDF requires JSON-LD processing with a pinned context set, and that mapping must be
part of the relation or a named trust assumption. Substituting an RDF graph for the JSON bytes that
were signed silently breaks linkage.

Key authorization is a separate step: a signature verifies under a verification method, and the
verifier must know that the issuer authorized that method for assertions. In the V5 extension
this is a verifier-owned issuer/verification-method/key table rather than live resolution.
Credential status and holder binding are further obligations; a signed claim about `ex:alice` does
not authenticate the presenter as Alice.

The three paths meet this requirement to different degrees. The legacy path authenticates a
Poseidon2 graph commitment signed by the issuer with Schnorr over Baby Jubjub, which requires the
issuer to adopt that representation; conventional Ed25519 or ECDSA credentials are checked outside
the circuit at ingestion and recommitted, which a verifier cannot rely on against a dishonest
holder. The exact evaluator and adapter proofs use source evidence `None`. V5 implements
Ed25519 verification over RDFC-1.0, checked against a W3C test vector, and queries the same
canonical documents under both authorities. Its native model tests pass, its built guest has
executed directly, and, after a first bounded proof attempt timed out, its generic adapter produced
one genuine receipt for a single synthetic verifier-agreed bag `SELECT` case
(§#ref(<v5-evidence>, supplement: none)). The native-composition experiment authenticates BBS+-signed statements, but only for
public triples (§#ref(<composition>, supplement: none)).

=== Scope and result contracts <scope>

Exactness is relative to a precise dataset construction and a precise result encoding. The scope
must fix which credential graphs form the default graph and which are named, and how `FROM` and
`FROM NAMED` restrict them. The result encoding must be canonical for its form: a bag `SELECT`
keeps duplicate rows, unbound positions and row identity; `CONSTRUCT` is a set of triples with
freshly minted blank nodes; `DESCRIBE` needs an explicit blank-node closure policy, because the
SPARQL specification leaves its output implementation-defined. The hosted genuine-proof test
functions exercise each of these obligations (§#ref(<exact-evidence>, supplement: none)).

=== Session binding and before-consume validation <validation>

A presentation must be checked completely before its challenge is consumed, and the result must be
released only after consumption. We specify the order: (i) resolve the method in the registry and
compare descriptor digests; (ii) check that the contract instantiates an accepted capability tuple;
(iii) check audience and validity window against the stored request; (iv) verify the receipt
against the pinned image with development mode refused; (v) decode the journal, compare its
request binding, authority, anchor and scope with the stored request, and compare the response's
claimed result with the journal result; (vi) check the result against the bounds; (vii)
atomically consume the challenge, failing closed on store errors; (viii) release the result. Checking before consuming prevents a malformed presentation from burning a legitimate
challenge; atomic consumption prevents two concurrent verifications from both succeeding. The
adapter implements these checks and its retained controls exercise each group (@controls-table);
we have not separately established that its internal ordering matches the specification step for
step.

=== Composing native credential proofs with circuits <composition>

A tempting design verifies BBS-signed credentials natively, disclosing public terms, and hands
hidden values to a Noir circuit for predicates that native proofs do not support. The session
challenge alone cannot bind the two: it shows both proofs belong to one exchange, not that they
talk about the same income. Such composition would need an explicit linkage relation — a
commitment to the hidden value in one system opened consistently in the other, a canonical byte
and field encoding, range constraints for cross-field representation, domain separation, and a
composition argument whose extraction assumptions hold jointly for both proof systems.

A separate, experimental native path exists with two distinct parts. Its tuple composition
combines BBS+ signatures over BLS12-381 with Circom circuits proved with LegoGroth16 in the same
field. Its opt-in native-RDF interface (`issue_rdf`, `prove_public_bgp`, `verify_public_bgp`)
uses BBS+ proofs alone to authenticate reconstructed public BGP triples under a verifier-owned
issuer and status policy. The native-RDF interface discloses signed-slot indices and status references,
supports no hidden RDF predicate, has no linkage to the Noir relations, does not evaluate full
exact SPARQL and is unaudited; its evidence is in §#ref(<nc-evidence>, supplement: none). The
cross-backend linkage described above — between native credential proofs and Noir or zkVM
relations — is not implemented. Consequently we draw no conclusion about migrating backends or
about the relative merit of native proofs, circuits and zkVMs; such a comparison is meaningful
only between relations with matched statements, authentication and disclosure.

== Minimizing secret-dependent obligations <minimize>

=== Running example <example>

A lender asks:

```sparql
PREFIX ex: <https://example.org/>
SELECT DISTINCT ?person WHERE {
  ?person a ex:Employee .              # t1
  ?person ex:employer ex:Acme .        # t2
  ?person ex:annualIncome ?income .    # t3
  FILTER(?income >= 30000)
}
```

and receives the single row $mu = {"?person" ↦ "ex:alice"}$. @example-table classifies each
obligation. Under $mu$, `t1` and `t2` become fully ground triples built only from query constants
and a projected binding; the verifier can compute them itself from the query and the answer.
`t3` still contains the hidden `?income`, and the filter constrains that hidden value.

#[
#show figure: set block(breakable: false)
#figure(
  table(
    columns: (0.8fr, 1.2fr, 1.4fr),
    align: (left, left, left),
    table.header[Obligation][Status under the contract][What remains to prove],
    [`t1`, `t2` membership], [Result-inferable; may be a public input if $d$ permits disclosing them and their source],
    [Membership in the authenticated credential, as a public-input constraint],
    [`t3` membership], [Secret (hidden object)], [Membership with a hidden term],
    [`FILTER`], [Secret predicate on a hidden value], [Predicate over the hidden income],
    [Issuer authentication], [Unchanged by disclosure], [Signature over the whole credential, which still contains the hidden income],
    [Subject linkage], [Public via `ex:alice`],
    [Membership of every subject occurrence and its equality to the public binding `ex:alice`; the equalities can be checked against public inputs rather than as a hidden join, but they remain],
  ),
  caption: [
    Obligations in the running example. Disclosure removes secrecy, not proof work: `t1` and `t2`
    still have to be members of an authenticated credential, and under a conventional (non
    selective-disclosure) signature the signature check covers hidden data and stays
    secret-dependent. If `?person` were not projected, `t1`–`t3` would be linked by a hidden join
    and none would be eligible.
  ],
) <example-table>
]

=== A conservative eligibility rule <rule>

*Definition (public eligibility).* Let the query pattern be a positive basic graph pattern with
filters — no `OPTIONAL`, `UNION`, `MINUS`, `NOT EXISTS`, `GRAPH` variables, property paths,
subqueries, aggregates, `BIND` or `VALUES` — with projection $V$ and released mapping $mu$. A
triple pattern $t$ is _public-eligible_ for $mu$ iff (i) every position of $t$ is an IRI or literal
constant of the query, or a variable $v ∈ V ∩ "dom"(mu)$ with $mu(v)$ an IRI or literal; (ii) $t$
contains no query blank node; and (iii) the disclosure policy $d$ permits revealing $t mu$ together
with its source attribution. The method must still enforce (iv) membership of $t mu$ in the
committed or authenticated dataset and (v) authentication of its source, and the verifier must
recompute $t mu$ from $(q, r)$ rather than accept a prover-supplied list.

*Design argument 2 (conditional).* Under (i)–(v), replacing the hidden witness for $t mu$ by a
public input leaves $R_C$ unchanged: the constraint "the witness term equals the constant or
projected value" becomes a direct public-input constraint, and every other conjunct is untouched.
The verifier learns nothing beyond $(q, r)$ and the attribution that $d$ already permits, because
$t mu$ is a function of $(q, r)$. Condition (i) also excludes hidden joins through $t$: every
variable of $t$ is public, so $t$ connects to other patterns only through public terms. The
argument is about the relation, not about any implementation's constraint system, and it assumes
that the specialized relation still checks (iv) and (v).

=== Why each condition is needed <counterexamples>

- *`OPTIONAL`.* In `?p a ex:Employee OPTIONAL { ?p ex:email ?e }`, a row with `?e` unbound asserts
  that no email triple matched. That is an absence claim; nothing is ground, and eliminating it
  needs an exact method.
- *`UNION`.* In `{ ?p ex:degree ex:MSc } UNION { ?p ex:degree ex:PhD }` with only `?p` projected,
  the row does not determine which branch held. Grounding either triple reveals the branch — a
  disclosure beyond $(q, r)$ unless $d$ explicitly permits it.
- *Blank nodes.* A query blank node, as in `?p ex:address [ ex:city ex:Oxford ]`, behaves as a
  non-projected variable. A data blank node in a result has graph-scoped identity; disclosing a
  triple around it can link presentations without saying anything stable.
- *Value equality.* `"30000"^^xsd:integer` and `"030000"^^xsd:integer` are equal values but
  different terms. Grounding by value can produce a triple that was never signed; the rule grounds
  only by term identity, and does not treat `FILTER(?x = ex:Acme)` as grounding `?x`.
- *Omission.* Grounding released rows says nothing about rows not released. It supports result
  membership, not exactness. And a triple that is public but unauthenticated would let a holder
  fabricate `t1`: removing secrecy never removes authentication or membership.

=== Relation to selective disclosure in zkRDF <zkrdf>

The idea of disclosing query constants and projected terms is not ours. zkRDF @braun26, building
on @braunkaefer25, traces the term occurrences behind each solution, reveals constants and
projected terms, hides the rest behind blank nodes, and proves knowledge of issuer signatures
(one per contributing graph), equality of hidden occurrences and numeric bounds natively with BBS
and range proofs; the verifier re-evaluates a rewritten query over the resulting selectively
disclosing dataset. It also reports proving substantially cheaper than an earlier zkVM execution
prototype @wright25dc on a small credential benchmark, under a changed signature scheme (BBS
rather than Ed25519).

What this paper adds is narrower. First, eligibility is stated relative to an explicit contract,
including dataset authority and a disclosure policy that covers source attribution and branch
information. Second, the counterexamples delimit the rule where a method _does_ support
non-monotone operators, which zkRDF deliberately excludes. Third, the rule is implemented for a
bounded Noir relation (V4) whose signatures are not selective-disclosure signatures, where the
signature check covers hidden data and the benefit of disclosure is confined to membership and
equality work (@example-table). Applying it inside the zkVM exact relations, or more broadly, is
proposed and not implemented. Whether the benefit is material is an empirical question.

=== The implemented public-pattern relation <v4>

V4 is an opt-in Noir relation specialized for the first fully public BGP pattern in a bounded
profile: the pattern's triple is a public input, while issuer-signature verification and
credential membership remain constraints of the relation (one signature check per selected
credential in the one- and two-credential capacity profiles K1 and K2). Credential status is also
checked inside the relation: status references, status-policy paths and status-leaf membership
are constraints, while roots, salts and status indices remain private witnesses, so V4 enforces a
bounded status snapshot and policy that the verifier accepted. The profile is restricted to the
filter-free form F0 (no hidden filter), status depth 10 and K1/K2. How the verifier acquires that
snapshot and how fresh it is are deployment policy; V4 does not establish world-wide freshness of
authoritative status. Its answer mode is `SelectedResults` in the set sense of
§#ref(<contract-tuple>, supplement: none): the method admits only `SELECT DISTINCT` queries and
rejects a released result that repeats a row, so a V4 proof shows that each released distinct row
is supported and asserts neither multiplicity nor completeness; V4 does not support `ASK`. Its
tests replay a finite
set of valid and absent bindings and adversarial witnesses; genuine proofs were produced for both
profiles. The authentication, membership and status obligations are implemented within V4's relation;
what V4 does not do is authenticate conventional RDF or JSON Data Integrity credentials. V5
addresses conventional RDF credentials for the exact evaluator, not for V4
(§#ref(<linkage>, supplement: none)).

A cost-based planner that chooses between public checks, native proofs, specialized circuits and
the exact evaluator; fusion of native and circuit work; and expansion to JSON-signed suites are
research outcomes this paper does not establish.

== The legacy fixed-circuit architecture <legacy>

The earlier architecture remains the project's most complete circuit-based design and a baseline
for the new paths. It proves _result membership_ for a monotone fragment — BGP scans over one
committed graph, datatype-bucketed value `FILTER`, a hidden-credential equality `JOIN` across
distinct graphs, and membership-indifferent modifiers — under the algebra of @pag09. Non-monotone
operators, aggregation and `ORDER BY` are excluded because extra undisclosed data could falsify
them; a join binding a shared variable to blank nodes in two committed graphs is rejected, since
blank-node identity is graph-scoped @rdf11.

Each graph is canonicalised with RDFC-1.0 @rdfc10 and committed with Poseidon2 @poseidon2 over the
BN254 scalar field; the commitment is bound to an issuer key by a Schnorr signature @schnorr91 over
Baby Jubjub @eip2494, accepted only if the key is in the relying party's external key set. The
commitments are public and unblinded, so low-entropy graphs can be guessed and repeated commitments
linked. Every sub-proof uses one circuit of a fixed, named family, so the verifier can re-derive
the circuit identity and recompute its verification key rather than trust a prover-supplied key.
A JSON manifest composes sub-proofs through binding edges, and the verifier binds the query nonce
into every sub-proof's public inputs.

#figure(
  table(
    columns: (1fr, auto),
    align: (left, right),
    table.header[Committed structural fact or circuit size][Value],
    [Circuit identifier kinds], [#headline("zkarch.circuit_kinds")],
    [Compiled circuit members], [#headline("zkarch.circuit_members")],
    [Fail-closed verifier binding obligations], [#headline("zkarch.binding_obligations")],
    [Cross-cutting audit gates], [#headline("zkarch.audit_gates")],
    [`Scan`, smallest member (UltraHonk gates)], [#headline("zkarch.gates_scan_min")],
    [`Scan`, largest member], [#headline("zkarch.gates_scan_max")],
    [Composable string-canonical filter lane], [#headline("zkarch.gates_filter_lane")],
    [Opt-in dual-leaf integer value lane], [#headline("zkarch.gates_filter_value_dl_int")],
    [`JoinEq`, smallest / largest], [#headline("zkarch.gates_join_min") / #headline("zkarch.gates_join_max")],
    [`RevokeUnset` (depth 10)], [#headline("zkarch.gates_revoke")],
    [`HiddenIssuer` (in-circuit Schnorr and key-set membership)], [#headline("zkarch.gates_hidden_issuer")],
    [`HolderPok` (hidden holder; explicitly not yet sound)], [#headline("zkarch.gates_holder_pok")],
  ),
  caption: [
    Deterministic facts of the legacy family from the regression-gated gate-count snapshot and
    committed verifier source (`bb gates -s ultra_honk`, pinned `nargo 1.0.0-beta.21`). They are
    constraint counts, not timings, and describe the main-branch family only; they say nothing
    about the new paths.
  ],
) <legacy-table>

#provenance("zkarch.gates_scan_max")

The verifier's four audit gates reconstruct public inputs from the declared statement, recompute
canonical verification keys, require issuer signatures under the external key set, and enforce a
single-use nonce. An internal adversarial audit of an earlier verifier found
#headline("cozk.single_prover_audit_issues") confirmed issues — among them unreconstructed public
inputs, trusted prover-supplied keys, unsigned commitments, replayable manifests and filter
operators not bound to the query — and
#headline("zkarch.forge_findings_mapped") of them now carry a standing forge-and-verify regression
test. That evidence pins known attacks closed; it does not find unknown ones. The hidden-holder
tiers are explicitly not yet sound and off by default; the dual-leaf value lane carries an accepted
invariant downgrade; lexical/value agreement of value-bearing leaves relies on the issuer. The
path's limits motivate the new work: it cannot state exact results, it authenticates conventional
credentials only off-circuit, and it proves every retained operator in secret.

== Evidence: method and current findings <evidence>

=== Method <evidence-method>

All new-path evidence comes from frozen JSON snapshots under `research/zk-paper-evidence/`, each
listed with its SHA-256 digest in `provenance.json`. Each snapshot summarises a second internal
evidence inspection of a completed run — source trees re-hashed against the stated commit,
artifacts and receipts re-hashed, recorded outcomes compared — rather than a new execution, and
none is an external security review. Every number below is read from `paper-evidence.json`
records whose values are machine-checked against those snapshots by JSON pointer. The inspections
did not re-run cryptographic verification locally; genuine-receipt verification and refusal of
development mode are exercised by the source-bound tests whose outcomes the snapshots record.
Campaigns are reported separately and never summed; in particular, the hosted campaign and the
EC2 adapter campaign used different guest binaries (#short-id("zkvcq.exact_guest_sha256") and
#short-id("zkvcq.adapter_guest_sha256")).

=== Bounded exact evaluator (hosted campaign) <exact-evidence>

A GitHub-hosted CI job built the guest from source #short-id("zkvcq.exact_source_commit") with
locked dependency graphs, ran the native model tests, then ran the host suite that executes the
actual guest and produces genuine receipts serially with #raw(headline("zkvcq.exact_r0vm_version")).

#figure(
  table(
    columns: (1fr, auto),
    align: (left, right),
    table.header[Hosted campaign fact][Count],
    [Genuine receipts (Succinct, halted with exit 0, no assumptions, image pin matched)], [#headline("zkvcq.exact_hosted_receipts")],
    [Native model test functions passed], [#headline("zkvcq.exact_hosted_model_tests")],
    [Host guest-and-proof test functions passed], [#headline("zkvcq.exact_hosted_host_tests")],
    [Host test functions explicitly ignored], [#headline("zkvcq.exact_hosted_ignored")],
    [Typed native replay cases], [#headline("zkvcq.exact_replay_cases")],
    [Native replay jobs (cases × request versions × authorities)], [#headline("zkvcq.exact_replay_jobs")],
    [of which expected and observed accepted], [#headline("zkvcq.exact_replay_expected_positive")],
    [of which expected and observed rejected], [#headline("zkvcq.exact_replay_expected_negative")],
    [Proofs produced by the native replay], [#headline("zkvcq.exact_replay_proofs")],
  ),
  caption: [
    Hosted exact-evaluator evidence at #short-id("zkvcq.exact_source_commit"). Test functions and
    replay jobs are not W3C conformance cases, and the replay is native execution only. The two
    ignored functions are the engine-replay proof functions; they are not counted as executed.
  ],
) <exact-table>

#provenance("zkvcq.exact_hosted_receipts")

The receipts are `v1-holder-bag`, `v1-verifier-select`, `v1-verifier-false-ask`,
`v2-holder-false-ask`, `v2-verifier-catalog`, `v3-holder-bag`, `v3-holder-describe` and
`v3-verifier-construct`. Together they cover all three request versions, the listed forms and
both authorities; not every form was proved under every authority. The genuine-proof test
functions that produced them assert that a holder-declared bag preserves
duplicates, unbound values and provenance; that a false `ASK` proves absence from the accepted
complete graph and a holder-declared absence respects `FROM NAMED` scope; that catalog and nested
graph results bind every public expectation; that `DESCRIBE` binds its explicit blank-node closure
policy; that `CONSTRUCT` mints fresh nodes with graph set semantics; and that tampering with or
replaying any public binding is rejected. This run completed the legacy genuine-proof functions that earlier campaigns had left
unfinished. What it does not show: whole-SPARQL conformance, any issuer authentication (none is in
these relations), performance, or anything about the historical EC2 receipts, which remain
attributed to their own guest binary.

=== Protocol adapter (EC2 campaign) <adapter-evidence>

The adapter wraps the exact evaluator in typed requests, method descriptors and a verification
routine. Receipts were produced on an EC2 host from frozen source
#short-id("zkvcq.adapter_source_commit").

#figure(
  table(
    columns: (1.6fr, 0.9fr, 1fr, 1.3fr),
    align: (left, left, left, left),
    table.header[Case][Form][Authority][Protocol outcome],
    [`select-bag-verifier-agreed`], [bag `SELECT`], [VerifierAgreed], [accepted],
    [`select-bag-holder-declared`], [bag `SELECT`], [HolderDeclared], [accepted],
    [`ask-true-verifier-agreed`], [`ASK` (true)], [VerifierAgreed], [accepted],
    [`ask-false-holder-declared`], [`ASK` (false)], [HolderDeclared], [accepted],
    [`construct-verifier-agreed`], [`CONSTRUCT`], [VerifierAgreed], [accepted],
    [`construct-holder-declared`], [`CONSTRUCT`], [HolderDeclared], [accepted],
    [`select-bag-row-bound`], [bag `SELECT`], [—], [rejected: result exceeds row bound],
  ),
  caption: [
    The #headline("zkvcq.adapter_receipts") genuine adapter receipts:
    #headline("zkvcq.adapter_accepted") accepted and #headline("zkvcq.adapter_row_bound_rejected")
    genuine receipt rejected at the contract's row bound — a genuine proof is necessary, not
    sufficient, for acceptance. Every tuple has source evidence `None`, status `NotRequested` and
    a bearer holder.
  ],
) <adapter-table>

#[
#show figure: set block(breakable: false)
#figure(
  table(
    columns: (auto, 1fr, auto),
    align: (left, left, right),
    table.header[Validation step][Control class][Records],
    [Method, contract], [Changed query of the same form], [#headline("zkvcq.ctl_changed_query_same_form")],
    [Method, contract], [Wrong method descriptor digest], [#headline("zkvcq.ctl_wrong_descriptor_digest")],
    [Journal binding], [Altered journal result], [#headline("zkvcq.ctl_altered_journal_result")],
    [Session], [Wrong challenge], [#headline("zkvcq.ctl_wrong_challenge")],
    [Session], [Wrong verifier audience], [#headline("zkvcq.ctl_wrong_verifier_audience")],
    [Session], [Stored audience changed to match the supplied one], [#headline("zkvcq.ctl_changed_stored_audience")],
    [Session], [Expired], [#headline("zkvcq.ctl_expired")],
    [Session], [Not yet valid], [#headline("zkvcq.ctl_not_yet_valid")],
    [Session], [Substituted window still containing now], [#headline("zkvcq.ctl_changed_window")],
    [Authority, scope], [HolderDeclared proof against VerifierAgreed request], [#headline("zkvcq.ctl_scope_holder_to_agreed")],
    [Authority, scope], [VerifierAgreed proof against HolderDeclared request], [#headline("zkvcq.ctl_scope_agreed_to_holder")],
    [Authority, scope], [Commitment other than the agreed anchor], [#headline("zkvcq.ctl_wrong_agreed_anchor")],
    [Consumption], [Replay against the same store], [#headline("zkvcq.ctl_replay_same_store")],
    [Consumption], [Concurrent verifications of one challenge], [#headline("zkvcq.ctl_concurrent_verifications")],
    [Consumption], [Broken challenge store], [#headline("zkvcq.ctl_broken_store")],
  ),
  caption: [
    Inventory of the #headline("zkvcq.adapter_controls") retained control records, grouped by the
    before-consume step of §#ref(<validation>, supplement: none) they exercise. Stores are
    in-memory test doubles, not durable production stores; the controls show the checks exist and
    fire in these cases, not that no other substitution succeeds.
  ],
) <controls-table>
]

The audit wrapper for this campaign itself reported failure: a mutation check expected a test
failure line that `--nocapture` output had split across lines, although the intended assertion
mutant was killed, and the post-restore gates were not reached in that wrapper. The receipts and
controls above were verified from retained files before the mutation step. A later, separate
continuation at the same source (#short-id("zkvcq.post_restore_head")) closed the two skipped
gates: a verify-only replay of the retained row-bound presentation passed
(#headline("zkvcq.post_restore_row_bound_passed") test function, none failed) and the adapter's
all-target host Clippy passed with warnings denied. It generated
#headline("zkvcq.post_restore_new_proofs") new proofs, and the original wrapper failure remains on
record unchanged. The adapter campaign did not complete the unfinished legacy proof functions; the
hosted campaign did.

=== Public-pattern relation and development pilot <pilot-evidence>

At source #short-id("zkvcq.pp_source_commit"), a paired harness produced
#headline("zkvcq.pp_genuine_proofs") genuine Noir/UltraHonk proofs with
#headline("zkvcq.pp_positive_verifications") successful backend verifications, alternating the
baseline relation V1 and the public-pattern relation V4 with the same query, rows, credentials,
policy and realised capacity. #headline("zkvcq.pp_warmups") warmups were excluded, leaving
#headline("zkvcq.pp_measured_samples") measured samples in #headline("zkvcq.pp_measured_pairs")
pairs (#headline("zkvcq.pp_n_per_cell") per arm and profile). All
#headline("zkvcq.pp_tamper_controls") tamper controls and #headline("zkvcq.pp_replay_controls")
replay controls were rejected. Every proof payload was #headline("zkvcq.pp_proof_bytes") bytes in
both arms; full presentation size was not measured separately. The harness's work counters were
identical across arms (same selected credentials, signature checks and shared memberships), so the
comparison isolates the relation's specialization.

#let prove-diffs = range(1, headline("zkvcq.pp_measured_pairs") + 1).map(i => ev("zkvcq.pilot_pair" + str(i) + "_prove_diff"))
#let verify-diffs = range(1, headline("zkvcq.pp_measured_pairs") + 1).map(i => ev("zkvcq.pilot_pair" + str(i) + "_verify_diff"))

#block(inset: 8pt, stroke: 0.5pt + gray, width: 100%, breakable: false)[
  *Indicative development measurement — not the basis of any claim.* Paired sequential runs on
  a shared EC2 KVM guest (Intel Xeon Platinum 8488C, eight vCPUs; Linux `7.0.0-1013-aws`; instance
  type not recorded in the snapshot) under a two-CPU quota and an eight-GiB memory cap, with
  heavy-job locks held, OS and dependency caches uncontrolled, `nargo 1.0.0-beta.21` with a pinned
  `bb` nightly. Times are inclusive prove and verify API timers that nest compilation, key and host
  I/O stages; they are not exclusive backend times and must not be decomposed.

  #figure(
    table(
      columns: (auto, auto, 1fr, 1fr),
      align: (left, left, right, right),
      table.header[Profile][Arm][Prove, s: median (min–max)][Verify, s: median (min–max)],
      [K1], [baseline V1],
      [#pilot("zkvcq.pilot_k1_v1_prove_median") (#pilot("zkvcq.pilot_k1_v1_prove_min")–#pilot("zkvcq.pilot_k1_v1_prove_max"))],
      [#pilot("zkvcq.pilot_k1_v1_verify_median") (#pilot("zkvcq.pilot_k1_v1_verify_min")–#pilot("zkvcq.pilot_k1_v1_verify_max"))],
      [K1], [public pattern V4],
      [#pilot("zkvcq.pilot_k1_v4_prove_median") (#pilot("zkvcq.pilot_k1_v4_prove_min")–#pilot("zkvcq.pilot_k1_v4_prove_max"))],
      [#pilot("zkvcq.pilot_k1_v4_verify_median") (#pilot("zkvcq.pilot_k1_v4_verify_min")–#pilot("zkvcq.pilot_k1_v4_verify_max"))],
      [K2], [baseline V1],
      [#pilot("zkvcq.pilot_k2_v1_prove_median") (#pilot("zkvcq.pilot_k2_v1_prove_min")–#pilot("zkvcq.pilot_k2_v1_prove_max"))],
      [#pilot("zkvcq.pilot_k2_v1_verify_median") (#pilot("zkvcq.pilot_k2_v1_verify_min")–#pilot("zkvcq.pilot_k2_v1_verify_max"))],
      [K2], [public pattern V4],
      [#pilot("zkvcq.pilot_k2_v4_prove_median") (#pilot("zkvcq.pilot_k2_v4_prove_min")–#pilot("zkvcq.pilot_k2_v4_prove_max"))],
      [#pilot("zkvcq.pilot_k2_v4_verify_median") (#pilot("zkvcq.pilot_k2_v4_verify_min")–#pilot("zkvcq.pilot_k2_v4_verify_max"))],
    ),
    caption: [
      Indicative development pilot, four measured samples per cell, warmups excluded. Descriptive
      only: no significance test, no general speedup, no scaling claim.
    ],
  ) <pilot-table>

  In #prove-diffs.filter(x => x < 0).len() of #prove-diffs.len() pairs the V4 inclusive prove time
  was lower, with paired differences (V4 minus V1) between #fmt3(calc.min(..prove-diffs)) and
  #fmt3(calc.max(..prove-diffs)) s; verify differences were lower in
  #verify-diffs.filter(x => x < 0).len() of #verify-diffs.len() pairs and ranged from
  #fmt3(calc.min(..verify-diffs)) to #fmt3(calc.max(..verify-diffs)) s, the largest driven by a
  single slow baseline sample.
]

The pilot establishes that the harness runs, enforces its equal-guarantee comparison contract
and rejects its controls. The differences it shows are small relative to the totals and come from
too few samples on shared hardware to say more; they do not show that disclosure specialization
yields a material or general gain, and full presentation size, memory, cold-start and larger
scales are unmeasured.

=== Authenticated extension of the exact evaluator (V5) <v5-evidence>

V5 implements strict Ed25519 verification over RDFC-1.0 canonical documents, checked against a
published W3C test vector, resolves issuers through a verifier-owned
issuer/verification-method/key table, and evaluates the query over the same canonical documents
under both authority profiles. At source #short-id("zkvcq.v5_source_commit"), an earlier scoped
native run of the evaluator model passed in three feature configurations with no failed or ignored test
functions: #headline("zkvcq.v5_auth_tests_passed") with the `authenticated-rdf` feature,
#headline("zkvcq.v5_default_off_tests_passed") with default features off, and
#headline("zkvcq.v5_graph_results_tests_passed") with `graph-results`. The configurations overlap
and are not added: the `authenticated-rdf` count consists of
#headline("zkvcq.v5_new_integration_tests") new integration and
#headline("zkvcq.v5_new_unit_tests") new unit test functions plus existing model functions that
also run under `graph-results`. The new functions cover, among other things, the W3C vector,
rejection of valid signatures under the wrong issuer, method, purpose or suite, preservation of
signed lexical forms, per-credential blank-node scope, binding of the authorization table into
request and commitment, and `SELECT`, `ASK` and `CONSTRUCT` under both authorities. Scoped Clippy
with warnings denied passed in all three configurations.

That earlier run was executed-native evidence only: it made
#headline("zkvcq.v5_guest_executions") guest executions and produced #headline("zkvcq.v5_proofs")
V5 proofs. Two later scoped gates and two separately bounded genuine-proof attempts, below, ran at
other sources; their counts are never added to that run's or to each other's.

*Guest build and direct execution.* At source #short-id("zkvcq.v5g_source_commit"), a scoped gate
built the separate V5 guest (#short-id("zkvcq.v5g_guest_sha256")), pinned independently of the
exact guest, and executed it directly in the zkVM executor (@v5-guest-table). Direct execution
runs the guest without generating a proof: a positive execution shows that the guest accepts and
evaluates an input, and an abort shows that it refuses one; neither yields a receipt a verifier
could check. A positive control precedes each negative family, and each abort must match the exact
expected guest panic, not merely some failure. The negative inputs include forged, spliced and
unauthorized credentials and noncanonical or foreign wire forms; the cross-version cases show the
exact (V3) guest aborting on V5 input while still accepting its own. All
#headline("zkvcq.v5g_source_files_verified") source files matched Git,
#headline("zkvcq.v5g_locks_unchanged") lock files were unchanged, and all-target Clippy with
warnings denied passed. With the authenticated feature off and on, the exact guest was
byte-identical (#short-id("zkvcq.v5g_exact_off_sha256") and #short-id("zkvcq.v5g_exact_on_sha256"))
and, as the snapshot records, equal to controlled earlier builds made at the same absolute build
path; these bytes are not the guests of the hosted or EC2 receipt campaigns, which remain
attributed to their own binaries. An earlier comparison across different build paths failed and is
retained, so we claim artifact identity only for a fixed build path and toolchain, not
reproducibility from arbitrary paths.

#[
#show figure: set block(breakable: false)
#figure(
  table(
    columns: (1fr, auto),
    align: (left, right),
    table.header[V5 guest-gate fact][Count],
    [Native harness test functions passed], [#headline("zkvcq.v5g_native_tests_passed")],
    [Genuine-proof driver compiled but explicitly ignored], [#headline("zkvcq.v5g_genuine_driver_ignored")],
    [Direct-execution test functions passed], [#headline("zkvcq.v5g_direct_sdk_tests_passed")],
    [V5 guest: positive direct executions], [#headline("zkvcq.v5g_direct_v5_positive")],
    [V5 guest: aborts with the expected guest panic], [#headline("zkvcq.v5g_direct_v5_aborts")],
    [Exact (V3) guest: positive direct execution], [#headline("zkvcq.v5g_direct_legacy_positive")],
    [Exact (V3) guest: aborts on V5 input], [#headline("zkvcq.v5g_direct_legacy_aborts")],
    [Direct guest executions in total], [#headline("zkvcq.v5g_direct_executions")],
    [Proofs / receipts generated], [#headline("zkvcq.v5g_proofs") / #headline("zkvcq.v5g_receipts")],
  ),
  caption: [
    Scoped V5 guest gate at #short-id("zkvcq.v5g_source_commit"). Direct executions run the
    guest without proving, so this is executed-guest evidence, not receipt evidence. The total is
    the snapshot's own count of the four execution rows; no count here is added to the earlier
    native model run.
  ],
) <v5-guest-table>
]

*First genuine-proof attempt.* At the same source, one bounded attempt of the low-level V5 driver
targeted a single declared case, `select-bag-verifier-agreed` (bag `SELECT` under VerifierAgreed),
over the published fixture with the pinned V5 artifact (#short-id("zkvcq.v5p_guest_sha256")). The
command reached its time limit before any receipt existed: it completed
#headline("zkvcq.v5p_proofs_completed") of #headline("zkvcq.v5p_proofs_planned") planned proof and
#headline("zkvcq.v5p_controls_completed") of #headline("zkvcq.v5p_controls_planned") planned
controls, and produced no presentation. We retain it unchanged as an incomplete execution, neither
a semantic rejection nor evidence about soundness; it was not retried automatically, and the later
adapter-level run below is a separate attempt at another source, not a completion of this one.

*Generic authenticated adapter.* At source #short-id("zkvcq.vcqa_source_commit"), a generic
adapter wraps V5 in the typed request, descriptor and verification routine of
§#ref(<adapter-evidence>, supplement: none), for bag `SELECT`, `ASK` and `CONSTRUCT` under both
authorities. Its binding chain starts from the verifier: the verifier's own canonically ordered
copy of its issuer/verification-method/key policy is hashed into a policy digest; that digest
enters the V5 method descriptor; the descriptor and the stored request determine the nonce; the
nonce enters the V5 request whose digest the V5 journal carries; the result claimed in the
response must equal the journal result; and the original challenge is consumed last. A scoped
native gate passed #headline("zkvcq.vcqa_distinct_tests_passed") distinct test functions —
#headline("zkvcq.vcqa_new_unit_tests") new unit, #headline("zkvcq.vcqa_new_integration_tests")
new integration and #headline("zkvcq.vcqa_new_job_parser_tests") new job-parser functions,
#headline("zkvcq.vcqa_existing_v5_tests") existing low-level V5 functions and
#headline("zkvcq.vcqa_legacy_tests") legacy V3 adapter functions, whose repeat under the
legacy-only configuration is not counted again — and all-target Clippy passed in both
configurations. The new tests check, among other things, that every policy field changes every
binding while key-table order does not, that mismatched descriptors, guests and pins and fake or
foreign receipts are rejected without consuming the challenge, and that stronger, weaker and
foreign requests fail admission. The V5 guest exported at this source has the same bytes as in the
guest gate (#short-id("zkvcq.vcqa_guest_sha256")), and the exact guest is again unchanged with
the feature off and on (#short-id("zkvcq.vcqa_exact_off_sha256"),
#short-id("zkvcq.vcqa_exact_on_sha256")) at the fixed build path. In that gate the genuine-proof
driver compiled but stayed ignored: the gate made #headline("zkvcq.vcqa_direct_executions") direct
guest executions and #headline("zkvcq.vcqa_genuine_proofs") proofs.

*Genuine adapter receipt for one case.* At the same source
(#short-id("zkvcq.vcqg_source_commit")), a separately bounded run executed that driver for one
declared case only, #raw(headline("zkvcq.vcqg_case")): bag `SELECT` under a verifier-agreed anchor
over a synthetic public W3C `eddsa-rdfc-2022` test vector, whose expected bag keeps a duplicated
row, #raw("?" + headline("zkvcq.vcqg_result_variable")) = #raw(headline("zkvcq.vcqg_result_row1"))
twice. It produced #headline("zkvcq.vcqg_genuine_receipts") genuine receipt
(#raw(headline("zkvcq.vcqg_receipt_inner")), #raw(headline("zkvcq.vcqg_exit_code")), `dev_mode` =
#raw(repr(headline("zkvcq.vcqg_dev_mode")))), of which #headline("zkvcq.vcqg_protocol_accepted")
was accepted by the protocol verifier. The test verified the receipt through the SDK under the
accepted V5 pin, whose artifact (#short-id("zkvcq.vcqg_guest_sha256")) matches the V5 guest of the
two gates above, compared the result with the native oracle and the hand-written expectation, and
ran the adapter's protocol checks. Inside the proved relation the guest checks issuer
authorization against the verifier's key table and the Ed25519 signatures, and evaluates the query
over the union of the canonical authenticated documents; outside it, the verifier checks the
journal against its own independently stored request and policy copy, and consumes the challenge
last. All #headline("zkvcq.vcqg_controls") controls attached to this receipt behaved as the frozen
test asserts; the retained audit lists each. The binding controls substitute the request (query,
challenge, audience, validity window), the verifier policy (key, issuer, method, table rows; each
caught at the descriptor digest and, when spliced past it, at the proof binding), the descriptor,
the journal, the receipt, the scope or anchor, and the image — the same V5 receipt presented under
the exact guest's independently pinned image. Each was refused with its exact typed code before
the challenge was consumed, with #headline("zkvcq.vcqg_binding_store_calls") challenge-store
calls. Of the challenge controls, a replay against the same store was refused as replayed, a
broken store failed closed as an infrastructure error, and of concurrent verifications of one
challenge #headline("zkvcq.vcqg_concurrent_accepted") was accepted and
#headline("zkvcq.vcqg_concurrent_replayed") reported replay; the stores are in-memory test doubles.
Its second internal evidence inspection confirmed #headline("zkvcq.vcqg_source_files_verified") source files
unchanged across the run and matching Git, re-hashed the receipt (#short-id("zkvcq.vcqg_receipt_sha256")),
the journal (#short-id("zkvcq.vcqg_journal_sha256")) and
#headline("zkvcq.vcqg_archive_verified_files") retained files, and checked the recorded
assertions; it launched no verification process of its own. The run was a validation, not a
benchmark, and we report no timing or resource figure for it.

These records bring V5, through its generic adapter, to genuine verified receipt for this one
fixture and profile, and close the missing authenticated-source end-to-end proof evidence only
there. The run did not cover the other declared cases (`all_defined_cases_run` =
#raw(repr(headline("zkvcq.vcqg_all_cases_run")))). The same V5 relation and adapter source
implements all six advertised query-form/authority tuples, and native and direct-execution tests
exercise them, but the holder-declared, `ASK`, `CONSTRUCT` and row-bound cases have only a
compiled, unrun genuine-proof path and no retained genuine receipt, so the registry conservatively still does not offer the method as available for its six
declared tuples. None of
these gates or attempts was a full-workspace gate or a security audit. The authentication they
exercise covers the verifier's issuer table and the signed canonical RDF bytes inside the
relation; neither V5 nor its adapter checks credential status, holder binding or a credential
validity period, performs full JSON-LD or Data Integrity processing, provides a JSON, JCS or
JOSE-to-RDF relation, or establishes wallet or world completeness.

=== Native-composition experiment <nc-evidence>

The experimental native path produced two distinct kinds of evidence at source
#short-id("zkvcq.nc_source_commit"), whose proofs must not be conflated. The first is native
public-RDF evidence: BBS+ proofs over the public-BGP interface, with no Circom or LegoGroth16
component. A hosted CI run exercised a declared finite native-RDF domain with
#headline("zkvcq.nc_distinct_proofs") distinct proofs under
#headline("zkvcq.nc_distinct_nonces") distinct nonces:
#headline("zkvcq.nc_required_accepted") proofs accepted by the required verifier policy, and
#headline("zkvcq.nc_weaker_rejected") proofs that verified under a weaker policy and were rejected
by the required verifier. A further #headline("zkvcq.nc_empty_graph_admission") empty-graph cells
were refused at admission and #headline("zkvcq.nc_other_exclusions") other cases were explicitly
excluded; neither group produced a proof. Separately, #headline("zkvcq.nc_cli_proofs") retained
command-line proof verified positively and was rejected under substituted issuer, query, result
and nonce, replay, revocation and status-epoch controls; proofs made inside the native test
functions are not enumerated. These are native BBS+ public-BGP proofs within one finite domain,
not composition proofs and not exact SPARQL proofs, and they share no counts with any other
campaign. The second kind is tuple composition: same-field composition of BBS+ over BLS12-381 with
Circom circuits proved with LegoGroth16 was executed by two legacy test functions, whose internal
proof count is not enumerated. Neither kind has linkage to the Noir relations, and the native-RDF
interface discloses only public triples.

=== Reproducing the evidence <repro>

The hosted snapshot records the exact commands, including
`cargo test --locked --manifest-path zk/sparql-evaluator/Cargo.toml -p sparq-proved-evaluator -- --nocapture --test-threads=1`
for the guest and proof suite and the `graph-results` feature for the native model; toolchain
versions and hashes of the host compiler, guest compiler and prover binary; the guest ELF digest and
image identifier; and the digest of every receipt. The pilot snapshot records every sample, the
paired differences, the executable and tool hashes, the resource limits and the hardware record.
The adapter snapshot records per-receipt, presentation and journal digests and the control
inventory, and its continuation records the replayed presentation digest. The V5 native snapshot
records each native command, per-configuration test inventories, lock-file and toolchain hashes;
the V5 guest, proof-attempt and authenticated-adapter snapshots record commands or test inventories,
guest digests and image identifiers, and job and archive digests; the authenticated-adapter
genuine snapshot adds the test executable, prover, runner and job digests, the receipt, journal,
presentation and transport digests, the per-control codes and the retained archive digest; the
native-composition snapshot records plan, report, source and executable hashes and the artifact
digests. Re-running the campaigns requires the pinned toolchains and, for the proofs, hardware
able to run the RISC Zero and Barretenberg provers.

== Related work

_Selective disclosure over RDF._ Braun and Käfer define RDF-based semantics for selective
disclosure and zero-knowledge proofs on credentials @braunkaefer25, and zkRDF turns SPARQL answers
into selectively disclosing datasets with native signature, equality and bound proofs @braun26;
Yamamoto, Suga and Sako formalise linked-data credentials for selective disclosure @yamamoto22.
These works established the disclosure of public terms that §#ref(<minimize>, supplement: none)
builds on, with formal treatment of the monotone fragment. Our contribution is complementary:
exact, authority-relative statements and method dispatch for paths that evaluate non-monotone
queries, and a precise account of what disclosure does and does not save when the signature is not
a selective-disclosure signature.

_General computation in a zkVM._ The first prototype in this line proved SPARQL evaluation over
Ed25519-signed credentials in RISC Zero @wright25dc @risc0, at a proving cost that zkRDF later
undercut substantially with a data-centric design @braun26. The exact evaluator returns to a zkVM
for a different reason — non-monotone semantics under an explicit contract — and inherits the cost
question rather than answering it.

_Verifiable databases and graph queries._ IntegriDB @integridb, vSQL @vsql and ZKSQL @zksql prove
SQL results, the last with zero-knowledge and explicit leakage of schema and cardinalities;
ZKGraph decomposes graph queries into expansion-centric operators @zkgraph25. Query decomposition
and private query proofs are therefore not new here. VeriDKG provides authenticated, complete
SPARQL results over a decentralized knowledge graph using authenticated indexes @veridkg23; its
completeness is over a published dataset, the natural counterpart of our VerifierAgreed anchor,
rather than over a holder's private credentials.

_Credentials._ CL signatures @cl01, BBS @bbs, zk-creds @zkcreds and Crescent @crescent prove
statements about signed attributes, with Crescent separating reusable credential preprocessing
from fresh presentations; OpenID for Verifiable Presentations restricts query expressiveness to
limit oversharing @openid4vp. These provide the suites and presentation protocols our method
abstraction dispatches to, not the query statement itself. Carroll's graph signing @carroll03 and
RDFC-1.0 @rdfc10 underpin the signed-representation side of §#ref(<linkage>, supplement: none).

== Discussion <discussion>

=== What the evidence answers now

*RQ1.* The contract and relation of §#ref(<contract>, supplement: none) separate membership from
exact evaluation and make authority explicit. The hosted campaign's genuine receipts
collectively cover exact bag `SELECT`, false `ASK`, `CONSTRUCT`, `DESCRIBE` and catalog results
and both authorities, without every form being proved under every authority. The answer is
partial: apart from one synthetic authenticated bag `SELECT` case
(§#ref(<v5-evidence>, supplement: none)), the proved datasets carry no issuer authentication,
the evaluator is bounded and its coverage is a test suite, not conformance, and exactness under
VerifierAgreed is only as good as the deployment's anchor policy.

*RQ2.* The eligibility rule, its counterexamples and design argument 2 identify one safe class of
reductions, and V4 implements its first instance while retaining authentication and membership in
the relation; application to the exact relations is proposed only. The pilot neither confirms
nor rules out a material benefit. Sharing, witness
selection and planning across obligations are not yet studied.

*RQ3.* Separate dispatch of suites and methods, capability tuples and before-consume validation
are implemented for one method family, with retained controls for each validation step. Issuer
authentication in front of exact evaluation (V5) executes natively and directly in the guest, and
a generic adapter binds a verifier-owned policy digest through to challenge consumption; after a
first low-level proof attempt timed out, the adapter produced one genuine receipt, with its
controls, for a single synthetic verifier-agreed bag `SELECT` case. The same relation and adapter
implement the holder-declared, `ASK`, `CONSTRUCT` and row-bound cases, which native and direct
tests exercise, but none of them has a retained genuine receipt; broader conventional suites,
including JSON-signed credentials and their JSON-to-RDF mapping, are unimplemented. The native-composition experiment authenticates
public BGP triples under verifier policy, while authenticated linkage between native proofs and
Noir or zkVM relations, and a cost planner across them, remain unimplemented.

=== Limitations and threats to validity

The design arguments are informal and conditional; no part is mechanised, and the implementation
is not shown to meet them. Every piece of proof evidence is internal to the project and re-checked
by a second internal evidence inspection, not externally audited (`sq-qhy4`); the collaborative multi-prover setting is
out of scope and separately gated (`sq-9hrn`). Tests and controls demonstrate the presence of
specific checks and cannot exclude untested substitutions. Commitments in the legacy path are
public and unblinded, and the disclosure analysis here does not account for all public inputs of
every method, for linkage across repeated presentations, or for what a sequence of permitted
queries reveals together. The pilot is small, run on shared hardware, and not canonical; no latency,
memory or size conclusion about any path should be drawn from it. V5's genuine evidence is one
receipt for one synthetic fixture and case, produced once as a validation, not a benchmark; its
guest-gate evidence is direct execution, not proving, and its first proof attempt ended
incomplete; nothing about its scaling or optimization is established. Guest-artifact identity is
recorded only for a fixed build path and toolchain, since an earlier cross-path comparison failed.
The exact-evaluator, both adapters and V5 have no status relation. V4 enforces its bounded, verifier-accepted status
snapshot and policy in the relation, and the native public-RDF path applies a verifier-owned status
policy to public triples; in both, acquisition and freshness of authoritative status are
deployment policy, not a world-wide freshness guarantee. Holder binding and key-authorization freshness
are established by none of the exact-evaluator, adapter, authenticated-adapter, V5 or V4
relations.

== Conclusion

Accepting a proof of a SPARQL answer is meaningful only relative to a contract that fixes the
result semantics, the authority over the dataset, the evidence about its source and what may be
disclosed. With that contract explicit, result-inferable triples that meet the eligibility
conditions can be moved to public inputs without weakening the statement, while authentication,
membership and hidden joins remain proof obligations. At exact commits, the bounded exact
evaluator produced #headline("zkvcq.exact_hosted_receipts") genuine receipts that collectively
cover both authority profiles, with
#headline("zkvcq.exact_replay_jobs") native replay jobs matching expectation; the adapter produced
#headline("zkvcq.adapter_receipts") genuine receipts, rejected one at its row bound and retained
#headline("zkvcq.adapter_controls") validation control records; and the public-pattern relation
produced #headline("zkvcq.pp_genuine_proofs") genuine proofs with all controls rejected. The
authenticated extension passed #headline("zkvcq.v5_auth_tests_passed") native model test
functions in one run and #headline("zkvcq.v5g_direct_executions") direct guest executions in a
later one; its first bounded proof attempt completed #headline("zkvcq.v5p_proofs_completed")
proofs, and a separate run of its generic adapter then produced
#headline("zkvcq.vcqg_genuine_receipts") genuine receipt, with all
#headline("zkvcq.vcqg_controls") controls behaving as asserted, for one synthetic verifier-agreed
bag `SELECT` case — genuine authenticated exact evidence for that fixture and profile only: the
other profiles its relation implements have native and direct test evidence but no retained
genuine receipt, and broader conventional suites are unimplemented; the native-composition experiment proves public BGP triples only. Nothing is externally audited.
The work is a design and partial-evidence contribution under an open audit gate.

#pagebreak(weak: true)
#heading(level: 2, numbering: none)[References]
#bibliography("zksparql-architecture.refs.yml", style: "ieee", title: none)

#if not anon [
  #line(length: 100%)
  #text(size: 0.8em, fill: gray)[
    sparq project. Working paper under the OPEN external-audit gate `sq-qhy4`; it asserts no
    proven security, privacy, integrity or attestation property. New-path evidence traces to the
    frozen snapshots in `research/zk-paper-evidence/` (digests in `provenance.json`), bound to
    `site/src/data/paper-evidence.json` by JSON pointer. Legacy-path evidence traces to
    `crates/sparq-zk-compose/tests/gate_count_snapshot.json`,
    `crates/sparq-zk-compose/src/verifier.rs`, `research/zk-soundness-audit.md` and
    `crates/sparq-zk-compose/tests/audit_forge_map.rs`. Numbers are injected at build time.
  ]
]
