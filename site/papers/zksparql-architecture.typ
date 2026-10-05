// [OPUS-5.5] Narrative restructure of the single-prover zkSPARQL paper (originally sq-3kd2g.1,
// epic sq-3kd2g / #1591; revised under zkp-19) to the credentials-first narrative of
// opus-5-5-narrative-proposal.md: answers instead of documents, supported versus exact answers,
// input authority, one issuer -> holder -> verifier architecture, a conservative public-data rule
// with a proposed release condition, and an evidence account in which every campaign carries its
// evidence level. Inventories, legacy fixed circuits, the native-composition experiment and the
// registry/capability tuples are kept in the appendix rather than deleted.
//
// HONESTY FRAME (empirical-honesty mandate + gate sq-qhy4, C-family / wip-arxiv): NO proven
// security, privacy, integrity or attestation property is asserted for any implementation.
// Design arguments are CONDITIONAL on named assumptions. Evidence levels are kept distinct:
// implemented source / executed native / executed guest / genuine verified receipt / external
// audit (none reached). Exactly ONE genuine receipt checks issuer signatures inside the proof
// (synthetic verifier-agreed bag SELECT, zkvcq.vcqg_*). The exact-evaluator and adapter receipts
// (SELECT/ASK/CONSTRUCT/DESCRIBE) carry source evidence None. There is NO authenticated false-ASK
// receipt yet; every place that needs one carries a TODO(evidence) marker and a visible note.
// Counts from different campaigns are never summed. Timings are INDICATIVE development
// measurements, shown only in one labelled pilot table via #ev(...).
//
// Single-source Typst. Every result number comes from paper-evidence.json through
// #headline(...) (canonical, json-pointer-bound to the frozen snapshots under
// research/zk-paper-evidence/) or #ev(...) (the indicative pilot only); none is hard-coded.

#import "_lib/bench.typ": headline, ev, provenance, authors, anon, paper_heading_numbering

#set document(title: "Answers Instead of Documents: Exact Private SPARQL Answers over Verifiable Credentials")
#set text(size: 11pt)
#set par(justify: true)
// Plain page numbers in the PDF footer (page set rules have no effect in HTML export).
#set page(numbering: "1")
// Figures may break across pages by default; short tables that must stay with their caption
// opt out locally with a scoped `set block(breakable: false)`.
#show figure: set block(breakable: true)
// Level-2 (==) headings are the top-level sections ("1.", "2.", ...), level-3 (===) are
// subsections ("1.1.", ...); the abstract is explicitly unnumbered. The appendix switches to
// letters after the references.
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

// Visible placeholder for evidence that does not exist yet. It must never be replaced by a
// result that has not been produced and frozen in research/zk-paper-evidence/.
#let todo-evidence(body) = block(
  inset: 6pt, stroke: 0.8pt + red, width: 100%, breakable: false,
)[#text(fill: red)[*Placeholder, evidence pending.* #body]]

#align(center)[
  #text(size: 17pt, weight: "bold")[
    Answers Instead of Documents
  ]
  #v(0.2em)
  #text(size: 12pt)[
    Exact Private SPARQL Answers over Verifiable Credentials
  ]
]
#authors()

#align(center)[#text(style: "italic", size: 0.9em)[
  Working draft. It reports implemented relations, and test and proof evidence at exact source
  commits that a second internal evidence inspection re-checked. It claims no proven security,
  privacy or integrity property for any implementation, and no component has had an external
  cryptographic review. Only one proof checks issuer signatures inside the proved relation, for a
  single synthetic case. All timing figures are indicative development measurements.
]]

#heading(level: 2, numbering: none, outlined: false)[Abstract]

A person asked a question about their credentials should be able to answer without handing the
credentials over. Zero-knowledge methods already let a holder prove that each released answer to a
SPARQL query is supported by signed data. Many verifier questions need more: every matching record
in an agreed period, a count, or the absence of a record. Such answers are useful only if they are
exact, meaning the complete result over a stated input, and only as trustworthy as whoever fixed
that input. We propose a query contract in which the verifier states the guarantee it needs,
whether it accepts the holder's own committed collection or an input it agreed independently, and
which signed sources count. We also propose a single-holder architecture that proves the released
answer under that contract, checking issuer signatures over the same canonical bytes that are
queried, and a conservative rule, proposed as a design requirement, for making answer-derived data
public only after an actual answer release that the contract authorises, without weakening the
statement. A bounded prototype in a zero-knowledge virtual machine has produced genuine verified
proofs of exact `SELECT`, false `ASK`, `CONSTRUCT` and `DESCRIBE` answers over committed inputs
whose issuer signatures were not checked in the proof. With signatures checked inside the proof,
it has produced one verified proof, for a synthetic verifier-agreed bag `SELECT` case, with
#headline("zkvcq.vcqg_controls") controls. Other authenticated forms, cost and a full disclosure
analysis remain open. No production cryptographic audit has been performed.

== Introduction <intro>

Consider a hypothetical mortgage application. The lender has three questions. Does a statement
from the applicant's employer show a salary above a threshold? Which loan repayments left an
agreed account during an agreed period? Was any payment from that account returned unpaid during
the period? The usual answer is to hand over payslips and statements, which discloses every
transaction, balance and payee the lender never asked about. If the employer and the bank issued
these records as verifiable credentials @vcdm2, their signatures would let the lender check where
each record came from and that it was not altered, given its trust in the issuers' keys. A
signature is evidence of origin and integrity, not of truth, and it does nothing to reduce what is
shared.

Zero-knowledge proofs can reduce it. With zkRDF @braun26, a verifier poses its question as a
SPARQL query @sparql11. The holder proves that each released answer is supported by signed data,
revealing only the terms the answer needs and proving numeric bounds on hidden values. That is the
right guarantee for the salary question: one supported answer settles it, and further records
cannot undo it. It is not the right guarantee for the other two. A list of repayments in which
every entry is genuine can still leave one out. A proof about released rows cannot show that a row
is missing, and the zkRDF authors note that their approach cannot prove non-existence. An earlier
interface design for zero-knowledge SPARQL over credentials took the same position deliberately:
in an open world it certified each returned result, made no completeness claim, and required no
proof for an empty result.
// TODO(citation): the soundness-only interface is the unpublished ISWC 2025 manuscript; decide
// whether and how to cite it under double-blind review before submission.
For a lender asking about returned payments, the empty result is the answer.

These questions need an _exact_ answer, meaning the complete result of the query over a stated
input. Exactness raises an issue that no stronger proof can remove. If the applicant chooses which
records form the input, an omitted repayment stays omitted however sound the proof. An exact
answer is therefore exact over the collection the holder committed, and no more, unless the
verifier accepted that input on independent grounds, for example a commitment the bank vouches for
as covering the account and period. Whether such commitments exist in a given deployment is an
assumption we state, not something a proof supplies.

We argue that the verifier should state in advance, in a _query contract_, which guarantee it
needs, whose input it accepts and which signed sources count, and that the holder should prove
exactly that statement. We ask three questions.

- *RQ1 (meaning).* What can a verifier conclude from an accepted private answer to a SPARQL query
  over credentials, and when does it need the exact answer over an input fixed by a party it
  trusts?
- *RQ2 (architecture).* How can a single holder prove the statement the verifier requires, so that
  issuer authentication, the queried data, the input authority and the verifier's request are
  bound into one checkable result?
- *RQ3 (disclosure).* Which parts of that proof can be made public, and when, without weakening the
  statement or revealing more than the verifier's policy permits, including when an interaction
  aborts?

Our answer is a single-holder architecture. Inside one proof, the holder's software checks issuer
signatures against keys the verifier accepts, builds the queried data from exactly the bytes that
were signed, evaluates the query, and outputs the result with a commitment to its input and a
binding to the verifier's request. Outside the proof, the verifier checks that output against its
own stored request and, where required, its agreed input. The contract names the proof method
separately from the signature suite. Supported answers can therefore use a disclosure-based method
and exact answers a general evaluator, while the obligation that the queried data is the signed
data remains explicit. Where the verifier can compute parts of the evidence from its query and an
answer actually released as the contract authorises, a conservative rule lets them be public after
that release. Proving that they were signed remains an obligation. An exchange that aborts without
an authorised release permits no such disclosure, while independently public or separately
authorised facts keep their own policy. These are proposed design requirements; we do not claim
proven pre-output or abort privacy.

We realise the exact path with a bounded SPARQL evaluator in a zero-knowledge virtual machine. It
has produced genuine verified proofs of exact `SELECT`, false `ASK`, `CONSTRUCT` and `DESCRIBE`
answers over committed inputs whose issuer signatures were not checked in the proof. With
signatures checked inside the proof, it has so far produced one verified proof, for a single
synthetic case. §#ref(<evidence>, supplement: none) states what each campaign covers. No component
has had a production cryptographic audit.

Our contributions are:

- *Answer modes and input authority* (§#ref(<meaning>, supplement: none)): a supported mode, offered
  only for positive patterns with set results, and an exact mode relative to a holder-declared or
  verifier-agreed input, with worked cases for a bag `SELECT`, a false `ASK`, a count, negation and
  "latest".
- *A single-holder architecture* (§#ref(<architecture>, supplement: none)) that binds issuer
  authentication, the signed canonical bytes, query evaluation and the verifier's request in one
  proof, followed by complete validation before the verifier consumes its challenge.
- *A conservative public-data rule* (§#ref(<minimize>, supplement: none)), extending zkRDF, with its
  counterexamples, the obligations it keeps, and a proposed release condition.
- *A prototype and an evidence account* (§#ref(<evidence>, supplement: none)) in which each
  campaign is reported separately with its evidence level.

The distinction matters more as software agents exchange records on people's behalf. An agent
relaying a bank's statement does not become an authority on its contents, and an agent receiving
an answer cannot pause to ask a person what that answer guaranteed. The guarantee has to be
written where software can check it.

== Background <background>

=== Credentials and what a signature establishes <bg-vc>

A verifiable credential @vcdm2 is a set of claims about a subject, signed by an issuer and held by
the subject or another holder, who presents it to a verifier. Credentials are signed in different
ways. The EdDSA Data Integrity suite @vcdieddsa signs the canonical N-Quads of an RDF document
under RDF Dataset Canonicalization (RDFC-1.0) @rdfc10. Other suites sign a JSON serialisation, and
BBS @bbs signs a list of messages so that a holder can later disclose some of them. A valid
signature shows that a key signed these bytes. The verifier still has to decide whether the issuer
authorised that key for this purpose, whether the credential is current, and whether the presenter
is the subject. None of these follows from the signature alone, and none of them makes the claims
true.

=== SPARQL answers are multisets <bg-sparql>

SPARQL @sparql11 has four query forms. `SELECT` returns a table of solution mappings, `ASK` returns
a boolean, and `CONSTRUCT` and `DESCRIBE` return RDF graphs. Write $⟦P⟧_D$ for the solutions of a
pattern $P$ over a dataset $D$ under the SPARQL algebra @pag09. These solutions form a multiset:
without `DISTINCT`, a `SELECT` may return the same row more than once, and how often it does can
matter. An RDF graph, by contrast, is a set of triples @rdf11, so two credentials stating the same
triple contribute it once to their union. RDF is usually read under an open-world assumption: not
seeing a fact does not make it false.

=== Zero-knowledge proofs and zkVMs <bg-zk>

A zero-knowledge proof convinces a verifier that a public statement holds for some secret witness
without revealing the witness. Circuit-based systems, such as Noir @noir with the UltraHonk backend
@barretenberg, express the statement as a fixed set of constraints. A zero-knowledge virtual
machine (zkVM), such as RISC Zero @risc0, instead proves that a given program, identified by an
image identifier, ran to completion on some input and wrote a given public output, called the
journal. The result is a receipt. A zkVM makes general computation, including a SPARQL evaluator,
provable without writing a circuit for each query. It proves that a program ran over the bytes it
received. It does not by itself prove who signed those bytes.

=== Where existing guarantees stop <bg-zkrdf>

zkRDF @braun26, building on RDF-based semantics for selective disclosure @braunkaefer25, traces
the term occurrences behind each solution. It reveals query constants and projected terms, hides
the remaining terms behind blank nodes, and proves issuer signatures, equality of hidden
occurrences and numeric bounds with native BBS and range proofs. The verifier re-evaluates a
rewritten query over the resulting selectively disclosing dataset. Its authors state that it
cannot prove non-existence and so cannot support `MINUS` or a false `ASK`.
// TODO(citation): same unpublished ISWC 2025 manuscript as in the introduction.
The earlier soundness-only interface took the same position by design: each returned result had to be a sound
entailment of data signed under listed keys, completeness was not required, and an empty result
needed no proof. The first prototype in this line ran SPARQL evaluation over Ed25519-signed
credentials inside RISC Zero @wright25dc. An earlier fixed-circuit architecture of this project
(Appendix #ref(<legacy>, supplement: none)) also proves only result membership. None of these
states an exact answer relative to an input authority, which is where this paper starts.

== What an accepted answer means <meaning>

Return to the lender. Suppose the bank issues statement credentials in which each payment is a
record such as `ex:pay1`, with `ex:fromAccount ex:acct`, an `ex:amount`, an `ex:date` and an
`ex:status`. Assume the contract forms the default graph as the union of the credential graphs.

=== Supported and exact answers <semantics>

A _supported_ answer says that each released distinct row occurs in the query's answer over
authenticated data. An _exact_ answer says that the released result is the complete answer of the
query over a committed input, compared under the equality that fits the result form: multiset
equality for an unordered table, boolean equality for `ASK`, and canonical graph equality, with a
stated blank-node policy, for `CONSTRUCT` and `DESCRIBE`. Appendix #ref(<app-relation>,
supplement: none) gives both as predicates of one relation. The lender's questions separate them.

- *Salary.* `SELECT DISTINCT ?p WHERE { ?p ex:employer ex:Acme ; ex:annualIncome ?i FILTER(?i >= 30000) }`
  needs only a supported answer. Once one signed record supports the row,
  further records cannot remove it.
- *Repayments.* `SELECT ?amount WHERE { ?p ex:fromAccount ex:acct ; ex:amount ?amount }` over a
  period may return the same amount twice, from two distinct payment records, and `DISTINCT` would
  collapse them. The distinct record IRIs matter: had two credentials stated an identical triple,
  their union would contain it once. A lender who sums repayments relies on the full multiset.
- *Returned payment.* `ASK { ?p ex:fromAccount ex:acct ; ex:status ex:Returned }` is false exactly
  when no matching record is in the evaluated input. A supported answer cannot express a false
  `ASK`.
- *Count.* `SELECT (COUNT(?p) AS ?n) WHERE { ?p ex:fromAccount ex:acct }` returns the full number
  only over input that contains every record. Over a subset it returns a smaller number that is
  equally "supported".
- *Negation and latest.* `FILTER NOT EXISTS { ... }`, `MINUS` and `ORDER BY DESC(?date) LIMIT 1` can
  each be changed by data that was never shown. One additional record may remove a row or displace
  the latest payment, although not every addition does.

For a positive pattern, meaning a basic graph pattern with joins, filters and projection, bag
evaluation is monotone: adding data can add solutions but never removes them. Mathematically, a row
found over a fully known authenticated subset therefore remains a row over all of the holder's
data. Monotonicity does not protect exact claims: that a repayment appears exactly twice, that the
count is two, that no payment was returned, or that a payment is the latest. Which guarantee a
verifier needs therefore depends on how it will use the answer, not only on the shape of the
query. A lender who only needs to know that some repayment of at least a given amount exists can
accept a supported answer to the repayments query; a lender who sums them cannot. We offer the
supported mode only for positive patterns with set (`DISTINCT`) results, where additional records
cannot invalidate a row, and we require the exact mode whenever the verifier relies on a
multiplicity, an absence, an aggregate, a negation or an ordering.

=== A count over released rows is not a count <counting>

A tempting shortcut is to let the holder release supported rows and let the verifier compute
`COUNT`, `ORDER BY` or `LIMIT` over them itself. This is sound only if the released rows are the
complete result. Otherwise the verifier is counting what the holder chose to show. The shortcut
therefore already assumes the exact statement it was meant to avoid. The same holds for a true
`ASK` over a positive pattern: one supporting solution establishes it, and a supported method can
exhibit that solution. A false `ASK` has no solution to exhibit and needs the exact mode.

=== Whose word fixed the input <authority>

An exact answer is exact over some input, and someone chose that input. We distinguish two
authorities.

*Holder-declared.* The result is exact over the collection the holder chose to commit. A false
`ASK` then means "the input I committed contains no returned payment", not "my bank records contain
no returned payment". This is useful when the commitment is reused elsewhere, so that a later
omission is detectable, or when the verifier only needs internal consistency. It must never be
presented as completeness of a wallet or of the world.

*Verifier-agreed.* The result is exact over an input whose commitment equals an anchor $k$ that the
verifier accepted independently of this presentation: for example a commitment that the bank
publishes as covering the account and period, one fixed in an earlier session under the verifier's
control, or one co-signed by a party the verifier trusts for that purpose. A verifier that accepts
whatever commitment the holder sends in the same exchange gains nothing beyond holder-declared.

Supported answers respect the open-world reading of RDF. Exact answers override it, but only
relative to the stated input and only as far as the verifier trusts whoever fixed that input. How
anchors are obtained is deployment policy; we return to it as an assumption in
§#ref(<anchor-assumption>, supplement: none). The implementation only checks that the proved
commitment equals the anchor in the request.

== Architecture <architecture>

The architecture has three parties (@fig-architecture). Issuers sign credentials. The verifier
writes a query contract and keeps its own copy. A single holder produces one proof that the
released answer satisfies that contract, and the verifier checks the proof's public output against
its stored contract before it accepts and consumes its challenge.

#[
#show figure: set block(breakable: false)
#figure(
  grid(
    columns: (1fr, auto, 1.7fr, auto, 1.2fr),
    gutter: 5pt,
    align: horizon,
    rect(width: 100%, inset: 6pt)[
      #set align(left)
      #set text(size: 0.85em)
      *Issuers* \
      Sign credentials, for example EdDSA over RDFC-1.0 canonical N-Quads. Optionally vouch for
      an input commitment (anchor $k$).
    ],
    [→],
    rect(width: 100%, inset: 6pt)[
      #set align(left)
      #set text(size: 0.85em)
      *Holder* \
      Receives contract $C$ from the verifier.
      #rect(width: 100%, inset: 5pt, stroke: (thickness: 0.6pt, dash: "dashed"))[
        *Inside the proof* \
        1. Check each signature against the verifier's key table.
        2. Build the dataset from exactly the signed canonical bytes.
        3. Evaluate $q$ under the contract's scope and bounds.
        4. Output result $r$, commitment $c$ and request binding.
      ]
    ],
    [→],
    rect(width: 100%, inset: 6pt)[
      #set align(left)
      #set text(size: 0.85em)
      *Verifier* \
      *Outside the proof:* check method, contract, session, receipt, journal against its stored
      request, anchor ($c = k$ if verifier-agreed) and bounds; then consume the challenge; then
      release $r$.
    ],
  ),
  caption: [
    One proof per presentation. The contract travels from verifier to holder; the proof and
    result travel back. Authentication, the queried data, evaluation and the request binding are
    inside the proof; acceptance policy, the anchor and challenge consumption are outside it. Only
    one prototype receipt so far has step 1 inside the proof (§#ref(<v5-evidence>, supplement:
    none)).
  ],
) <fig-architecture>
]

=== The query contract <contract-tuple>

The contract is what the verifier asks for, written down before the holder answers. Its fields
follow from §#ref(<meaning>, supplement: none). It states the _answer mode_ $o$, supported or
exact; the _query_ $q$ with its SPARQL version; and the _result form_ $f$ with its canonical
encoding, for example whether a `SELECT` is a bag or a set and which blank-node closure a
`DESCRIBE` uses. It states the _authority_ $a$, holder-declared or verifier-agreed with an anchor
$k$, and the _scope_ $s$: which credential graphs form the default graph and which are named, and
how `FROM` and `FROM NAMED` restrict them, since an exact answer is exact only over a precise
dataset construction. It states the _source evidence_ $e$: which signature suites and which
issuer, verification-method and key table count, which mapping from signed bytes to RDF is used,
and whether status and holder binding are required. It also states the _disclosure policy_ $d$,
saying what the verifier may learn beyond the result; _bounds_ $b$ on rows, triples and capacity;
the _session_ $t$, consisting of a challenge, an audience and a validity window; and the _method_
$m$, which names the proof method and version. We write the whole as
$C = ⟨ m, o, q, f, a, s, e, d, b, t ⟩$.

The proof's public output binds the contract, a commitment $c$ to the input and the result $r$ or
its digest. In the prototype this binding is indirect: the verifier's stored method descriptor and
request determine a nonce, the nonce enters the request whose digest the journal carries, and so
the session is bound to the proof. Appendix #ref(<app-relation>, supplement: none) states the full
relation and a conditional design argument for what acceptance implies.

=== Inside the proof: the queried data is the signed data <linkage>

Authenticating a credential and querying it are linked by one requirement: _the RDF dataset queried
must be derived, inside the proved relation or under an explicitly trusted mapping, from exactly
the bytes whose signature was verified_. Without it, a proof can be sound about evaluation and
silent about provenance. For RDF Data Integrity with RDFC-1.0, the signed message is derived from
the canonical N-Quads of the document and its proof configuration, so the queried dataset can be
those same canonical quads, with their signed lexical forms and per-credential blank-node scope
preserved. For JSON-signed credentials the signed bytes are JSON. Producing RDF from them requires
JSON-LD processing with a pinned context set, and that mapping must be part of the relation or a
named trust assumption. Substituting an RDF graph for the JSON bytes that were signed silently
breaks the link.

Key authorisation is a separate step. A signature verifies under a verification method, and the
verifier must know that the issuer authorised that method for assertions. In the prototype this is
a verifier-owned issuer, verification-method and key table that is hashed into the method
descriptor, rather than live resolution. Credential status and holder binding are further
obligations: a signed claim about `ex:alice` does not authenticate the presenter as Alice.

=== Outside the proof: check everything, then consume the challenge <validation>

A presentation must be checked completely before its challenge is consumed, and the result must be
released only after consumption. The verifier (i) resolves the method in its registry and compares
descriptor digests; (ii) checks that the contract is one it accepts; (iii) checks audience and
validity window against its stored request; (iv) verifies the receipt against the pinned image,
refusing development mode; (v) compares the journal's request binding, authority, anchor and scope
with its stored request, and the claimed result with the journal's result; (vi) checks the result
against the bounds; (vii) atomically consumes the challenge, failing closed if its store fails; and
(viii) releases the result. The verifier does not know the answer in advance, so it compares the
claimed result with the journal, not with its request. Checking before consuming stops a malformed
presentation from burning a legitimate challenge, and atomic consumption stops two concurrent
verifications from both succeeding. The prototype adapters implement these checks, and their
retained controls exercise each group (Appendix #ref(<app-inventories>, supplement: none)); we have
not separately established that the internal order matches this list step for step.

=== The signature suite and the proof method are named separately <dispatch>

A signature suite says how an issuer's bytes were signed. A proof method says what relation is
proved, and on which backend. The two vary independently. The contract therefore names them
separately, each with a versioned identifier and a descriptor digest held in a verifier-side
registry, and a presentation that names an unregistered method or a mismatched digest is rejected
before any receipt is examined. Naming them separately keeps visible the obligation of
§#ref(<linkage>, supplement: none) that joins them. It also lets supported answers use a
disclosure-based method, such as zkRDF or our public-pattern relation, while exact answers use the
general evaluator. Appendix #ref(<capabilities>, supplement: none) describes the registry and the
capability tuples a verifier accepts, and Appendix #ref(<composition>, supplement: none) explains
why combining two proof systems needs an explicit linkage relation.

== Revealing less <minimize>

Some facts a proof uses are already known to the verifier once it has its query and the answer.
Such facts need not be hidden, and treating them as public can remove secret work. This section
states when that is safe, what it does not remove, and when the facts may be revealed.

=== Running example <example>

The lender asks:

```sparql
PREFIX ex: <https://example.org/>
SELECT DISTINCT ?person WHERE {
  ?person a ex:Employee .              # t1
  ?person ex:employer ex:Acme .        # t2
  ?person ex:annualIncome ?income .    # t3
  FILTER(?income >= 30000)
}
```

and receives the single row $mu = {"?person" ↦ "ex:alice"}$. Under $mu$, `t1` and `t2` become
fully ground triples built only from query constants and a projected binding, so the verifier can
compute them itself. `t3` still contains the hidden `?income`, and the filter constrains that
hidden value. @example-table classifies each obligation.

#[
#show figure: set block(breakable: false)
#figure(
  table(
    columns: (0.8fr, 1.2fr, 1.4fr),
    align: (left, left, left),
    table.header[Obligation][Status under the contract][What remains to prove],
    [`t1`, `t2` membership], [Answer-derived; may be public if $d$ permits disclosing them and their source],
    [Membership in an authenticated credential, as a public-input constraint],
    [`t3` membership], [Secret (hidden object)], [Membership with a hidden term],
    [`FILTER`], [Secret predicate on a hidden value], [Predicate over the hidden income],
    [Issuer authentication], [Unchanged by disclosure], [Signature over the whole credential, which still contains the hidden income],
    [Subject linkage], [Public via `ex:alice`],
    [Membership of every subject occurrence and its equality to `ex:alice`; checked against public inputs rather than as a hidden join, but still checked],
  ),
  caption: [
    Obligations in the running example. Disclosure removes secrecy, not proof work: `t1` and `t2`
    must still be members of an authenticated credential, and under a conventional (not
    selective-disclosure) signature the signature check covers hidden data and stays secret. If
    `?person` were not projected, `t1` to `t3` would be linked by a hidden join and none would be
    eligible.
  ],
) <example-table>
]

=== A conservative public-data rule <rule>

*Definition (public eligibility).* Let the query pattern be a positive basic graph pattern with
filters, with no `OPTIONAL`, `UNION`, `MINUS`, `NOT EXISTS`, `GRAPH` variables, property paths,
subqueries, aggregates, `BIND` or `VALUES`, with projection $V$ and released mapping $mu$. A
triple pattern $t$ is _public-eligible_ for $mu$ if and only if (i) every position of $t$ is an IRI
or literal constant of the query, or a variable $v ∈ V ∩ "dom"(mu)$ with $mu(v)$ an IRI or literal;
(ii) $t$ contains no query blank node; and (iii) the disclosure policy $d$ permits revealing $t mu$
together with its source attribution. The method must still enforce (iv) membership of $t mu$ in
the committed or authenticated dataset and (v) authentication of its source, and the verifier must
recompute $t mu$ from the query and the answer rather than accept a list supplied by the holder.

Under these conditions, replacing the hidden witness for $t mu$ by a public input leaves the proved
statement unchanged, and condition (i) excludes hidden joins through $t$ because every variable of
$t$ is public. Appendix #ref(<arguments>, supplement: none) gives this as a conditional
design argument about the relation, not about any implementation's constraints.

=== Why each condition is needed <counterexamples>

- *`OPTIONAL`.* In `?p a ex:Employee OPTIONAL { ?p ex:email ?e }`, a row with `?e` unbound asserts
  that no email triple matched. That is an absence claim; nothing is ground, and it needs an exact
  method.
- *`UNION`.* In `{ ?p ex:degree ex:MSc } UNION { ?p ex:degree ex:PhD }` with only `?p` projected,
  the row does not determine which branch held. Grounding either triple reveals the branch, which
  is more than the query and answer reveal unless $d$ explicitly permits it.
- *Blank nodes.* A query blank node, as in `?p ex:address [ ex:city ex:Oxford ]`, behaves as a
  hidden variable. A data blank node in a result has graph-scoped identity, so disclosing a triple
  around it can link presentations without saying anything stable.
- *Value equality.* `"30000"^^xsd:integer` and `"030000"^^xsd:integer` are equal values but
  different terms. Grounding by value can produce a triple that was never signed, so the rule
  grounds only by term identity and does not treat `FILTER(?x = ex:Acme)` as grounding `?x`.
- *Omission.* Grounding released rows says nothing about rows not released. It supports a
  supported answer, not exactness. A triple that is public but unauthenticated would let a holder
  fabricate `t1`: removing secrecy never removes authentication or membership.

=== When answer-derived data may become public <release>

Eligibility says which facts _could_ be public. It does not say _when_. Being computable from the
query and an answer does not by itself permit disclosure before that answer has been released. We
therefore propose a release condition as a design requirement for any method that applies the
rule.

An answer-derived value may be public only after an actual release of the answer it is derived
from, made as the contract authorises. A contract's permission to release later is not itself a
release. An exchange that aborts with no authorised release authorises no answer-derived
disclosure. For example, a full-projection `SELECT` answer may legitimately expose a credential
triple, but exposing that triple in an exchange that then aborts reveals more than the no-answer
outcome would. An abort after an authorised release does not retroactively forbid what that
release already permitted. Facts that are independently public, or separately authorised, keep
their own allowance. Where the contract permits partial release, its explicit partial-release and
abort policy defines which partial-release observables are permitted. We assume neither that
delivery is atomic nor that proof acceptance alone constitutes or guarantees release.

These are proposed design requirements, not implemented or demonstrated gates, and they do not
establish pre-output or abort privacy. In the implemented public-pattern relation the pattern's
triple is a public input of the proof itself, so the requirement constrains when such a proof may
be sent or published rather than anything the relation checks.

=== What the verifier still learns <leakage>

Even an exact answer under a contract reveals more than the answer. The verifier sees the public
output: the input commitment $c$, which, unless it is hiding, lets a low-entropy input be guessed
and lets repeated use of one commitment link sessions; the method identifier and version; the
bounds and capacity profile, which bucket the size of the input; and, depending on the method and
on $d$, which issuers or how many credentials contributed. Proof size can leak too, although in the
public-pattern pilot every proof had the same payload size (§#ref(<pilot-evidence>, supplement:
none)). A sequence of permitted queries can reveal together what no single query reveals.
Signature-type leakage, validity periods, timing, metadata, pre-output and abort behaviour, and
linkage across sessions are part of the same open disclosure analysis. We do not claim that the
contract or the rule closes any of them.

=== Relation to selective disclosure in zkRDF <zkrdf>

The idea of revealing query constants and projected terms is not ours; it comes from zkRDF
@braun26 and @braunkaefer25. What we add is narrower. First, eligibility is stated relative to an
explicit contract, including the input authority and a disclosure policy that covers source
attribution and branch information, and it carries a release condition. Second, the
counterexamples delimit the rule where a method _does_ support non-monotone operators, which zkRDF
deliberately excludes. Third, we implement the rule for a bounded Noir relation whose signatures
are not selective-disclosure signatures, so the signature check still covers hidden data and the
saving is confined to membership and equality work (@example-table). Applying the rule inside the
exact zkVM relations is proposed and not implemented. Whether the saving is material is an
empirical question that our evidence does not settle.

== Prototype and evidence <evidence>

=== The prototype <prototype>

The _exact evaluator_ runs a bounded SPARQL evaluator inside the RISC Zero zkVM @risc0 and writes a
contract-bound exact result to the journal. It covers bag `SELECT`, `ASK`, `CONSTRUCT` and
`DESCRIBE` with an explicit blank-node closure policy, under both authorities, in three request
versions; version 3 commits source bytes and a graph catalog. A _protocol adapter_ wraps it in
typed requests, method descriptors and the validation routine of §#ref(<validation>, supplement:
none). Neither checks issuer signatures: their source evidence is `None`. An _authenticated
extension_ (V5) places strict Ed25519 verification of RDFC-1.0 credentials, checked against a
published W3C test vector, in front of the same evaluator, resolves issuers through the verifier's
key table, and evaluates the query over the same canonical documents; a _generic authenticated
adapter_ binds a digest of the verifier's own key policy into the method descriptor and through to
challenge consumption. Separately, a _public-pattern relation_ (V4) in Noir implements the rule of
§#ref(<rule>, supplement: none) for supported `SELECT DISTINCT` answers over one fully public basic
graph pattern, with issuer signature, credential membership and a bounded status check inside the
relation, in one- and two-credential capacity profiles (K1, K2). V4 does not support `ASK` and does
not authenticate conventional Data Integrity credentials. An earlier fixed-circuit design and a
native BBS+ experiment are described in Appendices #ref(<legacy>, supplement: none) and
#ref(<composition>, supplement: none).

=== Evidence levels <evidence-levels>

We grade every claim on five levels. _Implemented source_: code exists at a pinned commit.
_Executed native_: host tests of the model or relation ran and passed. _Executed guest_: the zkVM
guest or circuit executed, possibly without producing a proof. _Genuine verified receipt_: a real
proof was produced with development modes refused, and verified against a pinned image identifier
or verification key. _External audit_: independent cryptographic review, which no path has
reached. Each campaign's results come from a frozen snapshot that a second, project-internal
evidence inspection re-checked by re-hashing sources, artifacts and receipts. That inspection
checks that the reported runs happened at the stated commits, not that the relations achieve their
goals, and it is not a security review. Campaigns are reported separately and their counts are
never added together. @evidence-table summarises them; Appendix #ref(<app-inventories>, supplement:
none) gives the inventories.

#[
#show figure: set block(breakable: false)
#figure(
  table(
    columns: (1fr, 1.4fr, 0.8fr, 1.4fr),
    align: (left, left, left, left),
    table.header[Campaign (source)][What is proved][Signatures checked in proof][Highest level reached],
    [Exact evaluator, hosted (#short-id("zkvcq.exact_source_commit"))],
    [Exact bag `SELECT`, false `ASK`, `CONSTRUCT`, `DESCRIBE`; both authorities, not every form under each],
    [No],
    [Genuine verified receipts (#headline("zkvcq.exact_hosted_receipts"))],
    [Protocol adapter (#short-id("zkvcq.adapter_source_commit"))],
    [Contract-bound exact `SELECT`, `ASK` (true and false), `CONSTRUCT`],
    [No],
    [Genuine verified receipts (#headline("zkvcq.adapter_receipts"), one rejected at its row bound); #headline("zkvcq.adapter_controls") control records],
    [Authenticated evaluator V5 (#short-id("zkvcq.v5_source_commit"), #short-id("zkvcq.v5g_source_commit"))],
    [Exact evaluation over Ed25519/RDFC-1.0-verified documents],
    [Yes],
    [Executed native; executed guest (direct execution, no proof); first proof attempt incomplete],
    [Authenticated adapter (#short-id("zkvcq.vcqa_source_commit"))],
    [Contract-bound exact request over V5, bound to the verifier's key policy],
    [Yes],
    [One genuine verified receipt, one synthetic verifier-agreed bag `SELECT` case, #headline("zkvcq.vcqg_controls") controls; other declared cases unproved],
    [Authenticated false `ASK`],
    [Exact false `ASK` over authenticated input],
    [Yes],
    [None yet (placeholder below)],
    [Public pattern V4 (#short-id("zkvcq.pp_source_commit"))],
    [Supported `SELECT DISTINCT` rows, one fully public pattern, K1 and K2],
    [Yes, for the relation's own credentials],
    [Genuine verified proofs (#headline("zkvcq.pp_genuine_proofs")); timings indicative],
    [Fixed circuits, native BBS+ (Appendices #ref(<legacy>, supplement: none), #ref(<composition>, supplement: none))],
    [Result membership; public BGP triples, not exact SPARQL],
    [Off-circuit; BBS+],
    [No new runs; genuine native BBS+ proofs in a finite domain],
    [Cross-backend linkage, planner, JSON mapping],
    [n/a], [n/a], [Design only],
  ),
  caption: [
    Evidence by campaign. Read the last column before any other claim. Only one exact receipt, for
    a single synthetic case, checks issuer signatures inside the proof; rows are separate systems
    whose counts are never combined; no row reaches external audit.
  ],
) <evidence-table>
]

=== Exact answers without source authentication <exact-evidence>

A hosted CI job built the exact guest from source and produced
#headline("zkvcq.exact_hosted_receipts") genuine receipts with
#raw(headline("zkvcq.exact_r0vm_version")). Together they cover all three request versions, bag
`SELECT`, false `ASK`, `CONSTRUCT` and `DESCRIBE`, and both authorities; not every form was proved
under every authority (Appendix #ref(<exact-detail>, supplement: none) lists them and what their
tests assert). The same job ran #headline("zkvcq.exact_replay_jobs")
native replay jobs over #headline("zkvcq.exact_replay_cases") typed cases, each matching its
expected acceptance or rejection; the replay is native execution and produced no proofs.

On a separate host and guest binary, the protocol adapter produced
#headline("zkvcq.adapter_receipts") genuine receipts:
#headline("zkvcq.adapter_accepted") accepted, covering bag `SELECT`, true and false `ASK` and
`CONSTRUCT` across both authorities, and #headline("zkvcq.adapter_row_bound_rejected") rejected
because its result exceeded the contract's row bound. A genuine proof is necessary, not sufficient,
for acceptance. #headline("zkvcq.adapter_controls") retained control records exercise each
validation group (@controls-table). These receipts show exact evaluation
and request binding. They say nothing about who issued the data.

=== Exact answers with source authentication <v5-evidence>

The authenticated path reached genuine-receipt level in steps that we report separately. A native
run of the V5 model passed #headline("zkvcq.v5_auth_tests_passed") test functions with the
authenticated feature enabled, including the W3C vector and rejection of valid signatures under the
wrong issuer, method, purpose or suite; it made no guest executions and no proofs. A later gate
built the V5 guest and executed it directly, without proving,
#headline("zkvcq.v5g_direct_executions") times, with each negative input aborting with its exact
expected panic. A first bounded proof attempt then reached its time limit before any receipt
existed; we retain it as an incomplete execution, not as evidence either way.

At a later source, the generic authenticated adapter produced
#headline("zkvcq.vcqg_genuine_receipts") genuine receipt for one declared case,
#raw(headline("zkvcq.vcqg_case")): a bag `SELECT` under a verifier-agreed anchor over a synthetic
public W3C `eddsa-rdfc-2022` test vector, whose expected bag keeps a duplicated row
(#raw("?" + headline("zkvcq.vcqg_result_variable")) = #raw(headline("zkvcq.vcqg_result_row1"))
twice). Inside the proved relation the guest checked issuer authorisation against the verifier's key
table and the Ed25519 signatures, and evaluated the query over the union of the canonical
authenticated documents. Outside it, the verifier checked the journal against its own stored
request and policy copy and consumed the challenge last. All #headline("zkvcq.vcqg_controls")
controls attached to this receipt behaved as the frozen test asserts, each substitution being
refused before the challenge was consumed (Appendix #ref(<v5-detail>, supplement: none)). The run
was a validation, not a benchmark, and we report no timing for it.

This is the only proof in which issuer signatures are checked inside the proved relation. It
illustrates complete bag semantics over authenticated input; it is not a repayments application.
The same relation and adapter implement holder-declared, `ASK`, `CONSTRUCT` and row-bound cases,
which native and direct tests exercise, but none of them has a genuine receipt, so the registry
does not yet offer the method as available. The returned-payment question of
§#ref(<intro>, supplement: none) needs an authenticated false `ASK`, which does not yet exist.

// TODO(evidence): authenticated false-ASK receipt (V5 through the generic authenticated adapter,
// holder-declared and verifier-agreed). Insert only after a genuine receipt and its controls are
// frozen under research/zk-paper-evidence/ and bound in paper-evidence.json.
#todo-evidence[
  An authenticated false-`ASK` receipt, through the same adapter and with its controls, is the
  evidence the returned-payment example needs. It has not been produced. No result is reported
  here until it exists and has been inspected.
]

=== Supported answers with public patterns <pilot-evidence>

At source #short-id("zkvcq.pp_source_commit"), a paired harness produced
#headline("zkvcq.pp_genuine_proofs") genuine Noir/UltraHonk proofs, alternating a baseline relation
V1 and the public-pattern relation V4 over the same query, rows, credentials, policy and capacity.
All #headline("zkvcq.pp_tamper_controls") tamper controls and
#headline("zkvcq.pp_replay_controls") replay controls were rejected, every proof payload was
#headline("zkvcq.pp_proof_bytes") bytes in both arms, and the harness's work counters were
identical across arms, so the comparison isolates the relation's specialisation.

#let prove-diffs = range(1, headline("zkvcq.pp_measured_pairs") + 1).map(i => ev("zkvcq.pilot_pair" + str(i) + "_prove_diff"))
#let verify-diffs = range(1, headline("zkvcq.pp_measured_pairs") + 1).map(i => ev("zkvcq.pilot_pair" + str(i) + "_verify_diff"))

#block(inset: 8pt, stroke: 0.5pt + gray, width: 100%, breakable: false)[
  *Indicative development measurement, not the basis of any claim.* Paired sequential runs on a
  shared EC2 KVM guest (Intel Xeon Platinum 8488C, eight vCPUs) under a two-CPU quota and an
  eight-GiB memory cap, with caches uncontrolled. Times are inclusive prove and verify API timers
  that nest compilation, key and I/O stages; #headline("zkvcq.pp_warmups") warmups were excluded.

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
      Indicative development pilot, #headline("zkvcq.pp_n_per_cell") measured samples per cell.
      Descriptive only: no significance test, no general speedup, no scaling claim.
    ],
  ) <pilot-table>

  In #prove-diffs.filter(x => x < 0).len() of #prove-diffs.len() pairs the V4 inclusive prove time
  was lower, with paired differences (V4 minus V1) between #fmt3(calc.min(..prove-diffs)) and
  #fmt3(calc.max(..prove-diffs)) s; verify differences were lower in
  #verify-diffs.filter(x => x < 0).len() of #verify-diffs.len() pairs and ranged from
  #fmt3(calc.min(..verify-diffs)) to #fmt3(calc.max(..verify-diffs)) s, the largest driven by a
  single slow baseline sample.
]

The pilot shows that the harness runs, enforces its equal-guarantee comparison and rejects its
controls. The differences are small relative to the totals and come from too few samples on shared
hardware to say more. They do not show that the public-data rule yields a material or general
saving; presentation size, memory, cold start and larger scales are unmeasured.

=== What the evidence does not show <evidence-limits>

The evidence does not show conformance to SPARQL: the engine's own W3C conformance floor
(#headline("conformance.sparql_floor") passing assertions) measures the production engine, not the
bounded proved evaluator, whose coverage is only the tests and receipts above. It does not show
issuer authentication for any exact form other than the one synthetic bag `SELECT`, nor any
JSON-signed suite or JSON-to-RDF mapping. It does not show credential status, holder binding or
validity-period checks for any exact path. It says nothing about cost at realistic scale, and
nothing externally audited.

== Related work <related>

_Selective disclosure over RDF._ Braun and Käfer define RDF-based semantics for selective disclosure
and zero-knowledge proofs on credentials @braunkaefer25, and zkRDF turns SPARQL answers into
selectively disclosing datasets with native signature, equality and bound proofs @braun26; Yamamoto,
Suga and Sako formalise linked-data credentials for selective disclosure @yamamoto22. These works
established the disclosure of public terms that §#ref(<minimize>, supplement: none) builds on, with
formal treatment of the monotone fragment. Our contribution is complementary: exact,
authority-relative answers under a verifier-owned contract, and an account of what disclosure does
and does not save when the signature is not a selective-disclosure signature.

_General computation in a zkVM._ The first prototype in this line proved SPARQL evaluation over
Ed25519-signed credentials in RISC Zero @wright25dc @risc0, at a proving cost that zkRDF later
undercut substantially with a data-centric design under a different signature scheme @braun26. The
exact evaluator returns to a zkVM for a different reason, namely non-monotone semantics under an
explicit contract, and inherits the cost question rather than answering it.

_Verifiable databases and graph queries._ IntegriDB @integridb, vSQL @vsql and ZKSQL @zksql prove
SQL results, the last with zero knowledge and explicit leakage of schema and cardinalities; ZKGraph
decomposes graph queries into expansion-centric operators @zkgraph25. Private query proofs are
therefore not new. VeriDKG provides authenticated, complete SPARQL results over a decentralised
knowledge graph using authenticated indexes @veridkg23. Its completeness is over a published
dataset, the natural counterpart of our verifier-agreed anchor, rather than over a holder's
private credentials.

_Credentials._ CL signatures @cl01, BBS @bbs, zk-creds @zkcreds and Crescent @crescent prove
statements about signed attributes, with Crescent separating reusable credential preprocessing from
fresh presentations. OpenID for Verifiable Presentations restricts query expressiveness to limit
oversharing @openid4vp. These provide the suites and presentation protocols that a contract names,
not the query statement itself. Carroll's graph signing @carroll03 and RDFC-1.0 @rdfc10 underpin the
signed-representation side of §#ref(<linkage>, supplement: none).

== Discussion <discussion>

=== Where a verifier-agreed input could come from <anchor-assumption>

The strongest exactness claim rests on an assumption we do not discharge: that some party the
verifier trusts has fixed the input. A verifier-agreed anchor asserts that the committed input
contains every record of a stated kind within a stated scope, such as every payment from one
account in one period, and that the party publishing the anchor is trusted for that completeness.
One illustrative source is the issuer itself: a bank could publish or sign a commitment to a
statement period alongside the statement credentials. We do not claim that such infrastructure
exists, and if no realistic source of anchors is available in a deployment, the exactness that
deployment can offer narrows to holder-declared input, which is still useful for consistency and
for detecting later omission, but is not completeness.

=== Status, holder binding and key freshness

None of the exact paths checks credential status, holder binding or validity periods, and the key
table is a verifier-owned snapshot rather than live resolution. V4 checks a bounded status snapshot
that the verifier accepted, but its acquisition and freshness are deployment policy. A contract can
name these obligations; a method must then implement them.

=== Agents

Software agents may soon carry people's records between organisations. When the sender is also the
authority, as when a person vouches for her own calendar, she can simply sign the statement and no
proof is needed. Proofs earn their place when a holder relays someone else's authority and must not
reveal the rest, as when an applicant's agent relays a bank's records. That is the credentials case
with an agent as the holder. What changes is that the receiving agent cannot pause to ask a person
what an accepted answer guarantees. A contract that states the answer mode, the input authority and
the accepted sources is a guarantee written where software can check it. We have not evaluated
agents, delegation or agent protocols.

=== Several holders

This paper concerns a single holder. Answering a query over records held by several parties, each
unwilling to reveal its records to the others, calls for multi-party computation combined with
proofs, and raises its own questions of input authority and leakage. We treat it as a separate
line of work.

=== Limitations and threats to validity

The design arguments are informal and conditional; no part is mechanised, and the implementation
is not shown to meet them. All proof evidence is internal to the project and has not been
externally audited. Tests and controls show that specific checks exist and fire; they cannot rule
out untested substitutions. The bounded evaluator is covered by a test suite, not by conformance.
The authenticated evidence is one receipt for one synthetic fixture, produced once as a validation.
Guest-artifact identity is recorded only for a fixed build path and toolchain. The disclosure
account of §#ref(<leakage>, supplement: none) is a list of open concerns, not an analysis, and the
release condition of §#ref(<release>, supplement: none) is a proposed requirement, not an
implemented gate.

== Conclusion <conclusion>

A person should be able to answer a verifier's question without handing over the documents behind
it. An accepted private answer is useful only if the verifier knows what it guarantees: that each
released row is supported by authenticated data, or that the result is the complete answer over a
stated input, fixed by a stated authority. We propose that the verifier write this down in advance
as a query contract, and that a single holder prove exactly that statement over the same bytes the
issuers signed, checked completely before the verifier consumes its challenge. Facts the verifier
can compute from its query and an authorised released answer may then be public, under a
conservative rule that keeps authentication and membership as obligations. A bounded zkVM prototype
has produced genuine exact receipts for `SELECT`, false `ASK`, `CONSTRUCT` and `DESCRIBE` without
source authentication, and one genuine receipt, for a synthetic verifier-agreed bag `SELECT`, with
issuer signatures checked inside the proof. An authenticated false `ASK`, the other authenticated
forms, cost at scale, a disclosure analysis and an external audit remain to be done.

#pagebreak(weak: true)
#heading(level: 2, numbering: none)[References]
#bibliography("zksparql-architecture.refs.yml", style: "ieee", title: none)

// ---------------------------------------------------------------------------------------------
// Appendix: lettered sections after the references. Not part of the main-body page budget.
// ---------------------------------------------------------------------------------------------
#pagebreak(weak: true)
#counter(heading).update(0)
#set heading(numbering: (..n) => {
  let ns = n.pos()
  numbering("A.1.", ..if ns.len() > 1 { ns.slice(1) } else { ns })
})

== Formal relation and conditional design arguments <app-relation>

=== The contract tuple <app-contract>

A request carries an explicit contract
$ C = ⟨ m, o, q, f, a, s, e, d, b, t ⟩ $
whose components are:

/ $m$, method: query-proof method identifier, version and descriptor digest (§#ref(<dispatch>, supplement: none)).
/ $o$, answer mode: `SelectedResults`, the supported mode (every released distinct row is
  supported; no multiplicity or completeness), or `Exact` (the released result is the complete
  answer). A method may fix $o$; the adapters' methods fix `Exact`.
/ $q$, query: the query text and declared language version (SPARQL 1.1; SPARQL 1.2 @sparql12 optional).
/ $f$, result form: bag `SELECT`, set `SELECT`, `ASK`, `CONSTRUCT` or `DESCRIBE` with its closure
  policy, and the canonical result encoding.
/ $a$, authority: HolderDeclared, or VerifierAgreed with an anchor $k$.
/ $s$, scope: default-graph and named-graph construction, including `FROM` and `FROM NAMED`.
/ $e$, source evidence: `None`, or accepted signature suites, issuer/verification-method/key
  table, representation mapping, status requirement and holder-binding requirement.
/ $d$, disclosure policy: what the verifier may learn beyond the result (issuer identities,
  credential count, capacity profile, method).
/ $b$, bounds: row, triple and capacity limits.
/ $t$, session: challenge, audience and validity window.

A presentation returns a result $r$ and a proof $pi$ whose public output (the journal, for a zkVM
method) binds the contract, a dataset commitment $c$ and $r$ or its digest. Conceptually the
contract enters as a digest $h(C)$; an implementation may realise that binding indirectly rather
than as a literal journal field. In the adapter, the verifier's stored method descriptor and
request binding determine a derived nonce, which enters the model request whose digest the journal
carries, so the session bytes in $t$ are cryptographically bound to the proof (under assumptions A1
and A3 below). The verifier additionally checks the validity window and audience of $t$ on the host
and consumes the challenge (§#ref(<validation>, supplement: none)).

=== The relation <app-relation-def>

The commitment and the queried dataset are method-defined. Let $E$ be the signed or source encoding
of the credentials, $K$ the graph catalog that names default and named graphs, and $rho$ a salt or
other auxiliary witness where the profile requires one. Let $p_C$ be the publicly known,
method-specific commitment parameters that the method extracts from $C$, for example an
authorisation table, evaluation policy or representation mapping, as applicable; $p_C$ is empty
where a method binds none. A method fixes a commitment function $"Com"_m (E, K, rho; p_C)$ and,
separately, a dataset mapping $D = "Map"_m (E, K)$ from that encoding to the RDF dataset the query
runs over. Request version 3 of the exact evaluator commits source bytes and the graph catalog; the
authenticated extension V5 commits the canonical authenticated documents together with the
verifier's authorisation table as part of $p_C$. The commitment need not bind the whole contract:
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
answer under the method's canonical equality for $f$: multiset equality for an unordered table, the
method's ordering and tie rules for `ORDER BY`, `LIMIT` and `OFFSET` results, boolean equality for
`ASK`, and the method's canonical graph equality, with its blank-node and closure policy, for
`CONSTRUCT` and `DESCRIBE`. Write $⊑$ for sub-multiset inclusion. $"Ans"_"SelectedResults"^f (r, R)$
 is $r ⊆ "supp"(R)$, where $r$ is a set of distinct rows and $"supp"(R)$ is the set of rows
occurring in $R$ at least once. It is defined only for unordered distinct-set (`SELECT DISTINCT`)
results of a positive pattern and asserts neither multiplicity nor completeness; it is not a
relation for bag tables, booleans, graphs or ordered results. A true `ASK` over a positive pattern
is conceptually a separate positive-existence statement, supportable by exhibiting one supporting
solution rather than by table membership; the implemented selected-results relation V4 does not
support `ASK`. A false `ASK` needs `Exact`.

For a positive pattern, bag evaluation is monotone under sub-multiset inclusion: $D ⊆ D'$ implies
$⟦P⟧_D ⊑ ⟦P⟧_(D')$. A row and a lower bound on its multiplicity obtained by evaluating over a fully
known authenticated subset therefore remain valid over all of the holder's data. That lower bound
is available to whoever knows the whole authenticated supporting subset; it is not what a
selected-results presentation establishes, which shows only that each released distinct row occurs
in the answer. Repeated witnesses for one row prove no further multiplicity.

=== Conditional design arguments <arguments>

*Design argument 1 (exact result under a contract).* Assume (A1) knowledge soundness of the
receipt system for the pinned guest image; (A2) functional correctness of the guest evaluator: for
every input admitted by bounds $b$ it computes $"Map"_m (E, K)$ and the canonical form-$f$ encoding
of $⟦q⟧_D$ under the declared semantics; (A3) binding of the dataset commitment $"Com"_m$, and
collision resistance and domain separation of the hash functions that form the request and
method-descriptor digests; and (A4) that the verifier accepts only if the journal's request
binding, authority, anchor and scope match its stored request and the result claimed in the
response equals the result in the verified journal. Then acceptance implies, except with the
failure probabilities of A1 and A3, that some $(E, K, rho)$ with $"Com"_m (E, K, rho; p_C) = c$
satisfies $"Ans"_"Exact"^f (r, ⟦q⟧_D)$ for $D = "Map"_m (E, K)$; under VerifierAgreed,
additionally $c = k$. Nothing follows about issuers unless $e ≠ "None"$ and (A5) $"Src"_e$ is
enforced inside the proved relation. A2 is supported by tests and native replay
(§#ref(<evidence>, supplement: none)), not proved; A1 and A3 are assumptions on third-party
components; A4 is implemented and exercised by the adapters' controls. These are design arguments,
not a proof that the implementation meets them, and they carry no weight beyond their named
assumptions while the external audit remains open.

*Design argument 2 (public eligibility, conditional).* Under conditions
(i)–(v) of §#ref(<rule>, supplement: none), replacing the hidden witness for $t mu$ by a public
input leaves $R_C$ unchanged: the constraint "the witness term equals the constant or projected
value" becomes a direct public-input constraint, and every other conjunct is untouched. The
verifier learns nothing beyond $(q, r)$ and the attribution that $d$ already permits, because
$t mu$ is a function of $(q, r)$; this concerns the statement after release and says nothing about
disclosure before release or on abort (§#ref(<release>, supplement: none)). Condition (i) also
excludes hidden joins through $t$: every variable of $t$ is public, so $t$ connects to other
patterns only through public terms. The argument is about the relation, not about any
implementation's constraint system, and it assumes that the specialised relation still checks (iv)
and (v).

== Evidence detail and control inventories <app-inventories>

=== Method <evidence-method>

All new-path evidence comes from frozen JSON snapshots under `research/zk-paper-evidence/`, each
listed with its SHA-256 digest in `provenance.json`. Each snapshot summarises a second internal
evidence inspection of a completed run (source trees re-hashed against the stated commit,
artifacts and receipts re-hashed, recorded outcomes compared) rather than a new execution, and none
is an external security review. Every number in this paper is read from `paper-evidence.json`
records whose values are machine-checked against those snapshots by JSON pointer. The inspections
did not re-run cryptographic verification locally; genuine-receipt verification and refusal of
development mode are exercised by the source-bound tests whose outcomes the snapshots record.
Campaigns are reported separately and never summed; in particular, the hosted campaign and the EC2
adapter campaign used different guest binaries (#short-id("zkvcq.exact_guest_sha256") and
#short-id("zkvcq.adapter_guest_sha256")).

=== Hosted exact-evaluator campaign <exact-detail>

A GitHub-hosted CI job built the guest from source #short-id("zkvcq.exact_source_commit") with
locked dependency graphs, ran the native model tests, then ran the host suite that executes the
actual guest and produces genuine receipts serially.

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
`v3-verifier-construct`. The genuine-proof test functions that produced them assert that a
holder-declared bag preserves duplicates, unbound values and provenance; that a false `ASK` proves
absence from the accepted complete graph and a holder-declared absence respects `FROM NAMED` scope;
that catalog and nested graph results bind every public expectation; that `DESCRIBE` binds its
explicit blank-node closure policy; that `CONSTRUCT` mints fresh nodes with graph set semantics;
and that tampering with or replaying any public binding is rejected. This run completed the legacy
genuine-proof functions that earlier campaigns had left unfinished. It does not show whole-SPARQL
conformance, any issuer authentication, performance, or anything about the historical EC2
receipts, which remain attributed to their own guest binary.

=== Protocol adapter campaign <adapter-evidence>

Receipts were produced on an EC2 host from frozen source #short-id("zkvcq.adapter_source_commit").

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
    [`select-bag-row-bound`], [bag `SELECT`], [n/a], [rejected: result exceeds row bound],
  ),
  caption: [
    The #headline("zkvcq.adapter_receipts") genuine adapter receipts:
    #headline("zkvcq.adapter_accepted") accepted and #headline("zkvcq.adapter_row_bound_rejected")
    genuine receipt rejected at the contract's row bound. Every case has source evidence `None`,
    status `NotRequested` and a bearer holder.
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
    validation step of §#ref(<validation>, supplement: none) they exercise. Stores are in-memory
    test doubles, not durable production stores; the controls show the checks exist and fire in
    these cases, not that no other substitution succeeds.
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

=== Authenticated extension V5 and its adapter <v5-detail>

*Native model run.* At source #short-id("zkvcq.v5_source_commit"), a scoped native run of the
evaluator model passed in three feature configurations with no failed or ignored test functions:
#headline("zkvcq.v5_auth_tests_passed") with the `authenticated-rdf` feature,
#headline("zkvcq.v5_default_off_tests_passed") with default features off, and
#headline("zkvcq.v5_graph_results_tests_passed") with `graph-results`. The configurations overlap
and are not added: the `authenticated-rdf` count consists of
#headline("zkvcq.v5_new_integration_tests") new integration and
#headline("zkvcq.v5_new_unit_tests") new unit test functions plus existing model functions that
also run under `graph-results`. The new functions cover, among other things, the W3C vector,
rejection of valid signatures under the wrong issuer, method, purpose or suite, preservation of
signed lexical forms, per-credential blank-node scope, binding of the authorisation table into
request and commitment, and `SELECT`, `ASK` and `CONSTRUCT` under both authorities. Scoped Clippy
with warnings denied passed in all three configurations. That run made
#headline("zkvcq.v5_guest_executions") guest executions and produced #headline("zkvcq.v5_proofs")
proofs.

*Guest build and direct execution.* At source #short-id("zkvcq.v5g_source_commit"), a scoped gate
built the separate V5 guest (#short-id("zkvcq.v5g_guest_sha256")), pinned independently of the
exact guest, and executed it directly in the zkVM executor (@v5-guest-table). Direct execution
runs the guest without generating a proof: a positive execution shows that the guest accepts and
evaluates an input, and an abort shows that it refuses one; neither yields a receipt a verifier
could check. A positive control precedes each negative family, and each abort must match the exact
expected guest panic. The negative inputs include forged, spliced and unauthorised credentials and
noncanonical or foreign wire forms; the cross-version cases show the exact (V3) guest aborting on
V5 input while still accepting its own. All #headline("zkvcq.v5g_source_files_verified") source
files matched Git, #headline("zkvcq.v5g_locks_unchanged") lock files were unchanged, and all-target
Clippy with warnings denied passed. With the authenticated feature off and on, the exact guest was
byte-identical (#short-id("zkvcq.v5g_exact_off_sha256") and #short-id("zkvcq.v5g_exact_on_sha256"))
and equal to controlled earlier builds made at the same absolute build path; these bytes are not
the guests of the hosted or EC2 receipt campaigns. An earlier comparison across different build
paths failed and is retained, so we claim artifact identity only for a fixed build path and
toolchain.

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
    Scoped V5 guest gate at #short-id("zkvcq.v5g_source_commit"). Direct executions run the guest
    without proving, so this is executed-guest evidence, not receipt evidence. No count here is
    added to the earlier native model run.
  ],
) <v5-guest-table>
]

*First genuine-proof attempt.* At the same source, one bounded attempt of the low-level V5 driver
targeted a single declared case, `select-bag-verifier-agreed`, over the published fixture with the
pinned V5 artifact (#short-id("zkvcq.v5p_guest_sha256")). The command reached its time limit
before any receipt existed: it completed #headline("zkvcq.v5p_proofs_completed") of
#headline("zkvcq.v5p_proofs_planned") planned proof and #headline("zkvcq.v5p_controls_completed")
of #headline("zkvcq.v5p_controls_planned") planned controls, and produced no presentation. We
retain it unchanged as an incomplete execution, neither a semantic rejection nor evidence about
soundness; it was not retried automatically.

*Generic authenticated adapter gate.* At source #short-id("zkvcq.vcqa_source_commit"), a generic
adapter wraps V5 in the typed request, descriptor and verification routine of the protocol
adapter, for bag `SELECT`, `ASK` and `CONSTRUCT` under both authorities. Its binding chain starts
from the verifier: the verifier's own canonically ordered copy of its issuer, verification-method
and key policy is hashed into a policy digest; that digest enters the V5 method descriptor; the
descriptor and the stored request determine the nonce; the nonce enters the V5 request whose
digest the V5 journal carries; the result claimed in the response must equal the journal result;
and the original challenge is consumed last. A scoped native gate passed
#headline("zkvcq.vcqa_distinct_tests_passed") distinct test functions
(#headline("zkvcq.vcqa_new_unit_tests") new unit, #headline("zkvcq.vcqa_new_integration_tests")
new integration and #headline("zkvcq.vcqa_new_job_parser_tests") new job-parser functions,
#headline("zkvcq.vcqa_existing_v5_tests") existing low-level V5 functions and
#headline("zkvcq.vcqa_legacy_tests") legacy adapter functions, whose repeat under the legacy-only
configuration is not counted again), and all-target Clippy passed in both configurations. The new
tests check, among other things, that every policy field changes every binding while key-table
order does not, that mismatched descriptors, guests and pins and fake or foreign receipts are
rejected without consuming the challenge, and that stronger, weaker and foreign requests fail
admission. The V5 guest exported at this source has the same bytes as in the guest gate
(#short-id("zkvcq.vcqa_guest_sha256")), and the exact guest is again unchanged with the feature off
and on (#short-id("zkvcq.vcqa_exact_off_sha256"), #short-id("zkvcq.vcqa_exact_on_sha256")) at the
fixed build path. In that gate the genuine-proof driver compiled but stayed ignored: it made
#headline("zkvcq.vcqa_direct_executions") direct guest executions and
#headline("zkvcq.vcqa_genuine_proofs") proofs.

*Genuine adapter receipt for one case.* At the same source (#short-id("zkvcq.vcqg_source_commit")),
a separately bounded run executed that driver for #raw(headline("zkvcq.vcqg_case")) only. It
produced #headline("zkvcq.vcqg_genuine_receipts") genuine receipt
(#raw(headline("zkvcq.vcqg_receipt_inner")), #raw(headline("zkvcq.vcqg_exit_code")), `dev_mode` =
#raw(repr(headline("zkvcq.vcqg_dev_mode")))), of which #headline("zkvcq.vcqg_protocol_accepted")
was accepted by the protocol verifier. The test verified the receipt through the SDK under the
accepted V5 pin, whose artifact (#short-id("zkvcq.vcqg_guest_sha256")) matches the V5 guest of the
two gates above, compared the result with the native oracle and the hand-written expectation, and
ran the adapter's protocol checks. The binding controls substitute the request (query, challenge,
audience, validity window), the verifier policy (key, issuer, method, table rows; each caught at the
descriptor digest and, when spliced past it, at the proof binding), the descriptor, the journal,
the receipt, the scope or anchor, and the image, presenting the same V5 receipt under the exact
guest's independently pinned image. Each was refused with its exact typed code before the
challenge was consumed, with #headline("zkvcq.vcqg_binding_store_calls") challenge-store calls. Of
the challenge controls, a replay against the same store was refused as replayed, a broken store
failed closed as an infrastructure error, and of concurrent verifications of one challenge
#headline("zkvcq.vcqg_concurrent_accepted") was accepted and
#headline("zkvcq.vcqg_concurrent_replayed") reported replay; the stores are in-memory test doubles.
Its second internal evidence inspection confirmed #headline("zkvcq.vcqg_source_files_verified")
source files unchanged across the run and matching Git, re-hashed the receipt
(#short-id("zkvcq.vcqg_receipt_sha256")), the journal (#short-id("zkvcq.vcqg_journal_sha256")) and
#headline("zkvcq.vcqg_archive_verified_files") retained files, and checked the recorded
assertions; it launched no verification process of its own.

The run did not cover the other declared cases (`all_defined_cases_run` =
#raw(repr(headline("zkvcq.vcqg_all_cases_run")))). None of these gates or attempts was a
full-workspace gate or a security audit. The authentication they exercise covers the verifier's
issuer table and the signed canonical RDF bytes inside the relation; neither V5 nor its adapter
checks credential status, holder binding or a credential validity period, performs full JSON-LD or
Data Integrity processing, provides a JSON, JCS or JOSE-to-RDF relation, or establishes wallet or
world completeness.

=== The public-pattern relation V4 <v4>

V4 is an opt-in Noir relation specialised for the first fully public BGP pattern in a bounded
profile: the pattern's triple is a public input, while issuer-signature verification and credential
membership remain constraints of the relation (one signature check per selected credential in the
K1 and K2 profiles). Credential status is also checked inside the relation: status references,
status-policy paths and status-leaf membership are constraints, while roots, salts and status
indices remain private witnesses, so V4 enforces a bounded status snapshot and policy that the
verifier accepted. The profile is restricted to the filter-free form F0 (no hidden filter), status
depth 10 and K1/K2. How the verifier acquires that snapshot and how fresh it is are deployment
policy. Its answer mode is the supported mode in the set sense: the method admits only
`SELECT DISTINCT` queries and rejects a released result that repeats a row. Its tests replay a
finite set of valid and absent bindings and adversarial witnesses. In the pilot,
#headline("zkvcq.pp_positive_verifications") backend verifications succeeded and
#headline("zkvcq.pp_measured_samples") samples in #headline("zkvcq.pp_measured_pairs") pairs were
measured after warmups. The pilot snapshot records every sample, the paired differences, the
executable and tool hashes, the resource limits and the hardware record (Linux
`7.0.0-1013-aws`; instance type not recorded; `nargo 1.0.0-beta.21` with a pinned `bb` nightly; OS
and dependency caches uncontrolled; heavy-job locks held). A cost-based planner that chooses
between public checks, native proofs, specialised circuits and the exact evaluator, fusion of native
and circuit work, and expansion to JSON-signed suites are not established.

=== Reproducing the evidence <repro>

The hosted snapshot records the exact commands, including
`cargo test --locked --manifest-path zk/sparql-evaluator/Cargo.toml -p sparq-proved-evaluator -- --nocapture --test-threads=1`
for the guest and proof suite and the `graph-results` feature for the native model; toolchain
versions and hashes of the host compiler, guest compiler and prover binary; the guest ELF digest and
image identifier; and the digest of every receipt. The adapter snapshot records per-receipt,
presentation and journal digests and the control inventory, and its continuation records the
replayed presentation digest. The V5 native snapshot records each native command, per-configuration
test inventories, lock-file and toolchain hashes; the V5 guest, proof-attempt and
authenticated-adapter snapshots record commands or test inventories, guest digests and image
identifiers, and job and archive digests; the authenticated-adapter genuine snapshot adds the test
executable, prover, runner and job digests, the receipt, journal, presentation and transport
digests, the per-control codes and the retained archive digest; the native-composition snapshot
records plan, report, source and executable hashes and the artifact digests. Re-running the
campaigns requires the pinned toolchains and, for the proofs, hardware able to run the RISC Zero
and Barretenberg provers.

== Registry and capability tuples <capabilities>

The verifier, not the holder, decides what it will accept. Each signature suite and query-proof
method is identified by a versioned identifier and a descriptor digest in a verifier-side registry.
The verifier's policy is a set of _capability tuples_ $(m, o, f, a, e, "status", "holder")$: a
method, answer mode, result form, authority, source-evidence class, status-checking requirement and
holder-binding requirement. A request is valid only if its contract instantiates an accepted tuple,
and a method advertises only tuples it implements and tests.

The protocol adapter currently implements the tuples with $o$ = `Exact`, $f$ in bag `SELECT`, `ASK`
and `CONSTRUCT`, $a$ in HolderDeclared and VerifierAgreed, source evidence `None`, status
`NotRequested` and a bearer holder (@adapter-table). These tuples say what was proved about
evaluation; they say nothing about who issued the data. The generic authenticated adapter declares
the same forms and authorities with source evidence restricted to strict, bounded canonical-RDF
EdDSA under a verifier-owned policy. It has passed native tests and has one genuine receipt, for a
single VerifierAgreed bag `SELECT` case. Until its remaining declared cases have genuine coverage,
its registry entry conservatively does not offer these tuples as available
(Appendix #ref(<v5-detail>, supplement: none)). The registry is an integration design, not a new
cryptographic primitive.

== The legacy fixed-circuit architecture <legacy>

The earlier architecture remains the project's most complete circuit-based design and a baseline
for the new paths. It proves _result membership_ for a monotone fragment (basic graph pattern scans
over one committed graph, datatype-bucketed value `FILTER`, a hidden-credential equality `JOIN`
across distinct graphs, and membership-indifferent modifiers) under the algebra of @pag09.
Non-monotone operators, aggregation and `ORDER BY` are excluded because extra undisclosed data could
falsify them; a join binding a shared variable to blank nodes in two committed graphs is rejected,
since blank-node identity is graph-scoped @rdf11.

Each graph is canonicalised with RDFC-1.0 @rdfc10 and committed with Poseidon2 @poseidon2 over the
BN254 scalar field; the commitment is bound to an issuer key by a Schnorr signature @schnorr91 over
Baby Jubjub @eip2494, accepted only if the key is in the relying party's external key set. This
requires the issuer to adopt that representation; conventional Ed25519 or ECDSA credentials are
checked outside the circuit at ingestion and recommitted, which a verifier cannot rely on against a
dishonest holder. The commitments are public and unblinded, so low-entropy graphs can be guessed
and repeated commitments linked. Every sub-proof uses one circuit of a fixed, named family, so the
verifier can re-derive the circuit identity and recompute its verification key rather than trust a
prover-supplied key. A JSON manifest composes sub-proofs through binding edges, and the verifier
binds the query nonce into every sub-proof's public inputs.

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
#headline("cozk.single_prover_audit_issues") confirmed issues, among them unreconstructed public
inputs, trusted prover-supplied keys, unsigned commitments, replayable manifests and filter
operators not bound to the query, and #headline("zkarch.forge_findings_mapped") of them now carry a
standing forge-and-verify regression test. That evidence pins known attacks closed; it does not
find unknown ones. The hidden-holder tiers are explicitly not yet sound and off by default; the
dual-leaf value lane carries an accepted invariant downgrade; lexical/value agreement of
value-bearing leaves relies on the issuer. The path's limits motivate the new work: it cannot state
exact results, it authenticates conventional credentials only off-circuit, and it proves every
retained operator in secret.

== The native-composition experiment <composition>

A tempting design verifies BBS-signed credentials natively, disclosing public terms, and hands
hidden values to a Noir circuit for predicates that native proofs do not support. The session
challenge alone cannot bind the two: it shows both proofs belong to one exchange, not that they
talk about the same income. Such composition would need an explicit linkage relation: a commitment
to the hidden value in one system opened consistently in the other, a canonical byte and field
encoding, range constraints for cross-field representation, domain separation, and a composition
argument whose extraction assumptions hold jointly for both proof systems. That linkage is not
implemented.

A separate, experimental native path exists with two distinct parts. Its tuple composition combines
BBS+ signatures over BLS12-381 with Circom circuits proved with LegoGroth16 in the same field. Its
opt-in native-RDF interface (`issue_rdf`, `prove_public_bgp`, `verify_public_bgp`) uses BBS+ proofs
alone to authenticate reconstructed public BGP triples under a verifier-owned issuer and status
policy. The native-RDF interface discloses signed-slot indices and status references, supports no
hidden RDF predicate, has no linkage to the Noir relations, does not evaluate full exact SPARQL and
is unaudited.

At source #short-id("zkvcq.nc_source_commit"), a hosted CI run exercised a declared finite
native-RDF domain with #headline("zkvcq.nc_distinct_proofs") distinct BBS+ proofs under
#headline("zkvcq.nc_distinct_nonces") distinct nonces: #headline("zkvcq.nc_required_accepted")
proofs accepted by the required verifier policy, and #headline("zkvcq.nc_weaker_rejected") proofs
that verified under a weaker policy and were rejected by the required verifier. A further
#headline("zkvcq.nc_empty_graph_admission") empty-graph cells were refused at admission and
#headline("zkvcq.nc_other_exclusions") other cases were explicitly excluded; neither group produced
a proof. Separately, #headline("zkvcq.nc_cli_proofs") retained command-line proof verified
positively and was rejected under substituted issuer, query, result and nonce, replay, revocation
and status-epoch controls; proofs made inside the native test functions are not enumerated. These
are native BBS+ public-BGP proofs within one finite domain, not composition proofs and not exact
SPARQL proofs, and they share no counts with any other campaign. Tuple composition was executed by
two legacy test functions, whose internal proof count is not enumerated. We therefore draw no
conclusion about the relative merit of native proofs, circuits and zkVMs; such a comparison is
meaningful only between relations with matched statements, authentication and disclosure.

#if not anon [
  #line(length: 100%)
  #text(size: 0.8em, fill: gray)[
    sparq project. Working paper under the open external-audit gate `sq-qhy4`; it asserts no
    proven security, privacy, integrity or attestation property. New-path evidence traces to the
    frozen snapshots in `research/zk-paper-evidence/` (digests in `provenance.json`), bound to
    `site/src/data/paper-evidence.json` by JSON pointer. Legacy-path evidence traces to
    `crates/sparq-zk-compose/tests/gate_count_snapshot.json`,
    `crates/sparq-zk-compose/src/verifier.rs`, `research/zk-soundness-audit.md` and
    `crates/sparq-zk-compose/tests/audit_forge_map.rs`. Numbers are injected at build time.
  ]
]
