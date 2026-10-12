// [OPUS-5.5] ESWC 2027 submission variant of the zkSPARQL architecture paper
// (zksparql-architecture.typ): Springer LNCS format through the fine-lncs template, double-blind
// by default. The text is copied from the source paper and cut to fit the ESWC limits: a main body
// of at most 15 pages without references, an appendix of at most 2 pages, then the supplemental
// material and generative-AI statements. The supplementary material is a separate document: the
// "Supplementary material" part of zksparql-architecture.typ, whose section and table numbers
// (S1 to S8, Table S1 to S9) the references below follow. The language guide
// (zksparql-language-guide.md) binds this file as it binds the source. Every number comes from
// paper-evidence.json through the shared helpers (headline, ev) of _lib/bench.typ; none is typed.
//
// Build: typst compile --root site --input data="$(cat site/src/data/paper-evidence.json)" \
//   site/papers/zksparql-eswc.typ out.pdf
// Switches (edit the default here, or override with --input name=value):
//   opening    "credentials": the lender question opens the paper; "agents": software agents
//              relaying records on people's behalf open it.
//   coauthors  author list of the non-anonymous (camera-ready) build: "wright" or "wright-braun".
//   anon       "true" (default): double-blind submission; "false": camera-ready.

#let opening = sys.inputs.at("opening", default: "credentials")
#let coauthors = sys.inputs.at("coauthors", default: "wright")
#let anon = sys.inputs.at("anon", default: "true") == "true"

#assert(opening in ("credentials", "agents"), message: "opening must be credentials or agents")
#assert(coauthors in ("wright", "wright-braun"), message: "coauthors must be wright or wright-braun")

#import "_lib/bench.typ": headline, ev
#import "@preview/fine-lncs:0.6.5": lncs, author, institute

// Withheld repository identifiers in the double-blind build (as in the source paper).
#let withheld = "[withheld for review]"
#let repo-id(body) = if anon { withheld } else { body }

// Display helpers shared in form with the source paper: the value always comes from the evidence
// file. Executor cycle counts in millions, rounded to two decimals for display only.
#let mcycles(key) = {
  let s = str(calc.round(headline(key) / 1000000, digits: 2))
  let parts = s.split(".")
  if parts.len() == 1 { s + ".00" } else { s + "0" * calc.max(0, 2 - parts.at(1).len()) }
}

// A visible placeholder for a planned measurement that is not yet an evidence record, or for text
// the authors must still supply.
#let todo-results(body) = block(inset: 5pt, stroke: 0.8pt + red, width: 100%, breakable: false)[#text(fill: red)[*Results pending.* #body]]
#let author-todo(body) = text(fill: red)[[#body]]

// Authors. The anonymous build ignores `coauthors`.
// TODO(authors): confirm affiliations and ORCID identifiers for the camera-ready version.
#let inst-anon = institute("Anonymous institution")
#let inst-oxford = institute("University of Oxford", addr: "Oxford, United Kingdom")
#let inst-kit = institute("Karlsruhe Institute of Technology", addr: "Karlsruhe, Germany")
#let author-list = if anon {
  (author("Anonymous Author(s)", insts: inst-anon),)
} else if coauthors == "wright-braun" {
  (author("Jesse Wright", insts: inst-oxford), author("Christoph H.-J. Braun", insts: inst-kit))
} else {
  (author("Jesse Wright", insts: inst-oxford),)
}

#show: lncs.with(
  title: [Answers Instead of Documents: Private SPARQL Answers over Verifiable Credentials],
  running-title: [Answers Instead of Documents],
  running-author: if anon { [Anonymous] } else { none },
  authors: author-list,
  keywords: ("SPARQL", "Verifiable credentials", "Zero-knowledge proofs", "zkVM", "Data Integrity"),
  abstract: [
    A verifier often needs the answer to a question about a person's credentials, not the
    credentials themselves. zkRDF proves that each returned SPARQL solution follows from signed data but cannot
    prove absence, and an earlier zkVM prototype answered `SELECT` queries without stating its input.
    We show how a holder can prove, in one zero-knowledge proof and under stated assumptions, that an
    answer is a result SPARQL 1.2 permits over a stated input dataset, with the issuers' signatures
    checked inside the proof. What an answer to a monotone query contains, such as a true `ASK` or a
    returned row, stays true for any larger input, but closed-world answers, such as a false `ASK` or a
    count, are only as meaningful as the verifier's reason to believe the input complete. The
    query request therefore states who fixed the input: the holder, or the verifier, by agreeing a
    dataset commitment in advance. It also names the trusted issuers, the cryptosuites and a signature
    mode, which trades proving cost against disclosure; we propose Merkle-based cryptosuites designed
    for proving. A SPARQL evaluator running in the RISC Zero zkVM has produced verified proofs, with
    signatures checked inside, of `SELECT`, true and false `ASK` and `CONSTRUCT` answers over one
    synthetic credential each. Executed without proving, a later build evaluated queries with
    `OPTIONAL`, `MINUS`, aggregates, subqueries and property paths over up to four credentials, and
    needed fewer cycles with revealed signatures or a Merkle-based cryptosuite. Proving times are
    pending.
  ],
  bibliography: none,
  // LNCS text area (llncs.cls: 12.2 cm by 19.3 cm, running head outside it), centred on A4.
  page-config: (paper: "a4", margin: (x: 44mm, y: 52mm)),
)

// Floats: LNCS-like spacing around figures and tables (the template's default is wider).
#show figure: it => { set block(above: 1.4em, below: 1.4em); it }
#set figure(placement: auto)
// Tables: horizontal rules only, at the LNCS caption size.
#set table(stroke: (x, y) => (top: 0.4pt, bottom: 0.4pt), inset: (x: 3pt, y: 2.5pt))
#show raw.where(block: true): set text(size: 8.5pt)

= Introduction <intro>

// The two openings (switch `opening` above). Both end with the running example.
#if opening == "agents" [
Software agents may soon carry people's records between organisations. A person who vouches for her
own data can simply sign; a proof is needed when an agent relays a statement its user did not make
and must not reveal the rest. Asked by a lender whether any payment from a mortgage applicant's
account was returned unpaid, the applicant's agent would today relay a bank statement, which also
discloses every payment, balance and payee. If the bank issued it as a verifiable credential @vcdm2,
the lender could check that the bank signed it, but the credential would still disclose everything.

A zero-knowledge proof lets the applicant, as holder, return only the answer, with a proof that it
is correct. The lender's agent cannot ask a person what an accepted answer guarantees, so the
request must state it where software can check it. The lender writes its question as a SPARQL query
@sparql12, our running example:
] else [
A lender assessing a mortgage application asks the applicant one question: was any payment from the
applicant's account returned unpaid? The usual answer is a bank statement, which also discloses
every payment, balance and payee that the lender did not ask about. If the bank issued it as a
verifiable credential @vcdm2, the lender could check that the bank signed it, but the credential
would still disclose everything in it.

A zero-knowledge proof lets the applicant, as holder, return only the answer, with a proof that it
is correct. The lender writes its question as a SPARQL query @sparql12, our running example:
]

```sparql
PREFIX ex: <https://bank.example/vocab#>
ASK { ?payment a ex:Payment ; ex:paymentStatus ex:Returned . }
```

zkRDF @braun26 proves that each returned solution follows from signed data. An answer of `false`,
however, has no solution to show, and its authors state that zkRDF cannot prove absence. An earlier prototype by Wright @wright25dc evaluated
`SELECT` queries in a zkVM, checking the credentials' signatures inside the proof, but its results
do not say which credentials they cover, and it had no `ASK` (§#ref(<bg-zkrdf>, supplement: none)).

We show how a holder can prove general SPARQL answers over its credentials: results that SPARQL 1.2
permits for a `SELECT`, `ASK` or `CONSTRUCT` query over a stated input dataset, with the issuers'
signatures checked inside the proof (§#ref(<expressivity>, supplement: none) lists the features our
evaluator excludes).
Such a result can rest on absence, as `false` does; we call it a closed-world answer, and it is only
as useful as its input is complete. Because the holder chooses which credentials to include,
`false` means only "no returned payment in the credentials I included". The verifier can fix the
input in advance, but `false` then still covers only that input. No stronger proof removes this
limit; the request can only state who fixed the input.

Checking signatures inside the proof is costly: for the payment question, it was the largest step
in our cycle counts (§#ref(<cost>, supplement: none)). Revealing the signatures removes that step,
but lets the verifier link presentations of a credential and, with the standard RDF cryptosuites,
confirm a guess about its content. The request therefore lists the signature modes the verifier
accepts. Our contributions are:

- *Answers and their reading* (§#ref(<meaning>, supplement: none)): an answer is a result SPARQL
  1.2 permits over the input dataset; closed-world answers depend on who fixed the input.
- *An architecture* (§#ref(<architecture>, supplement: none)) in which one proof shows such an
  answer, bound to the verifier's request, with every credential's signature checked against the
  request's trusted issuers; signature modes, Merkle-based cryptosuites and a public-input rule set
  what authenticating the input costs and discloses (§#ref(<modes>, supplement: none)–§#ref(<cryptosuites>, supplement: none)).
- *An evaluation* (§#ref(<evidence>, supplement: none)) of a SPARQL evaluator in the RISC Zero zkVM
  @risc0: proofs with signature checks of `SELECT`, true and false `ASK` and `CONSTRUCT` answers;
  executions without proving of one query case per feature; and cycle counts by signature mode and
  cryptosuite. Proving times are pending.

= Background <background>

== RDF, SPARQL and credentials <bg-sparql>

An RDF dataset has one default graph and zero or more named graphs @sparql12. SPARQL has four query
forms: `SELECT` returns a sequence of solution mappings (rows), `ASK` a boolean, and `CONSTRUCT` and
`DESCRIBE` an RDF graph. Pattern matching compares terms, whereas the `=` operator compares values
@rdf11. The companion specification of §#ref(<architecture>, supplement: none) depends on SPARQL
1.2 @sparql12 and RDF 1.2 @rdf12. RDF is usually read open-world: an absent triple is not thereby
false.

A verifiable credential @vcdm2 is a set of claims that an issuer makes about a subject, secured so
that its authorship can be verified. A holder presents it to a verifier in a verifiable
presentation, which may instead carry data derived from credentials, such as a zero-knowledge proof.
Here, a credential's Data Integrity proof @vcdi is a signature (`proofValue`) with the proof
configuration, which names the verification method, here a public key (`verificationMethod`); a
cryptosuite specifies how to create and verify the proof. The cryptosuite `eddsa-rdfc-2022` @vcdieddsa canonicalises the
credential and the proof configuration with RDF Dataset Canonicalization (RDFC-1.0) @rdfc10;
Ed25519 then signs the SHA-256 hash of the canonical proof configuration followed by that of the
canonical document. A valid signature shows only that someone with the signing key signed these
bytes. Whether the issuer authorised the key, through its controlled identifier document @vcdi, is
a separate check, as are credential status (revocation or suspension), the validity period, and
holder binding: that the presenter is the subject or controls a key bound to the credential.
Selective-disclosure cryptosuites such as `bbs-2023` @vcdibbs let the holder derive a proof that
discloses only selected claims.

== Zero-knowledge proofs and zkVMs <bg-zk>

A zero-knowledge proof convinces a verifier of a statement: that, for given public inputs, some
witness satisfies a relation @zkproof. Here the result is a public input and the credentials are
part of the witness. A proof system is knowledge-sound if an efficient extractor can obtain a valid
witness from any prover that convinces the verifier, and zero-knowledge if a proof reveals nothing
about the witness beyond the statement. Circuit-based systems, such as Noir @noir with the
UltraHonk backend @barretenberg, implement a relation as constraints. A zero-knowledge virtual
machine (zkVM) such as RISC Zero @risc0 instead proves that a guest program, identified by its image
ID, ran on some input and wrote a public output, the journal. Its proof is a receipt: the journal,
image ID and exit status, with a seal attesting to them; development mode produces fake receipts. A
zkVM can thus prove a SPARQL evaluator without a circuit per query, but only that it ran over the
bytes it received, not who signed them.

== Proofs of SPARQL answers over credentials <bg-zkrdf>

zkRDF @braun26, building on RDF-based semantics for selective disclosure @braunkaefer25, proves the
soundness of SPARQL results: every returned solution is a solution over signed data. It discloses
query constants and projected terms and proves signatures, equalities and numeric bounds over the
rest with BBS+ signatures and range proofs. Its fragment includes basic graph patterns, joins,
`UNION`, `OPTIONAL`, `VALUES`, integer `FILTER`s, `GROUP BY`, `DISTINCT`, `LIMIT` and `CONSTRUCT`,
and `ASK` only when `true` (Table 1 of @braun26). Its authors state that it cannot prove
non-existence, and so neither `MINUS` nor a false `ASK` (§5.1 of @braun26). Wright @wright25dc
evaluated SPARQL 1.1 `SELECT` queries over Ed25519-signed credentials in the RISC Zero zkVM, whose
guest program also verified the credentials (§5 of @wright25dc; §3 of @braun26). A result does not
state the input over which it holds, and `ASK` and `CONSTRUCT` were left to future work (§7 of
@wright25dc).

= What an accepted answer means <meaning>

Suppose the bank issues the applicant a credential that lists payments, each with the type
`ex:Payment`, an amount, a date and a status. The input dataset $D$ is the RDF merge @rdf11mt of the
graphs of the credentials the holder includes, as the default graph; the merge keeps each
credential's blank nodes apart. A proof method may instead put each credential in its own named
graph, and states its layout, because a query written for one does not match the other.

The proof fixes $D$ through a _dataset commitment_: a digest of the included credentials and of a
random salt that the holder chooses. It is binding if the hash is collision-resistant and, while the
salt stays secret and the hash is a random oracle, hiding: it reveals nothing about $D$ to a
computationally bounded verifier.

== Answers over the input dataset <answers>

Let $Q$ be the request's query. An _answer_ is a result that SPARQL 1.2 permits for $Q$ over $D$,
with nothing added or omitted @zksparqlspec: for `SELECT`, a solution sequence that $Q$ can produce
over $D$, with every solution and duplicate; for `ASK`, `true` exactly when the query pattern has a
solution over $D$; for `CONSTRUCT`, a graph that $Q$ can produce over $D$, compared up to
isomorphism. A result that exceeds the request's limits is rejected, never truncated. Without
`ORDER BY`, a `SELECT` result is compared as a multiset.

For some queries, such as those using `OFFSET` and `LIMIT` where `ORDER BY` does not fix the order,
`REDUCED`, `SAMPLE` or `GROUP_CONCAT`, SPARQL permits several results over the same $D$, and the
holder chooses one. A proof method may fix and publish these choices; ours does not yet, so a
verifier can rely on a unique result only for queries that leave SPARQL no choice.
// Source: ZK code landing addenda (10 October), item 3; a follow-up change documents the choices.
An answer does not say who signed $D$ (§#ref(<linkage>, supplement: none)).

== Open-world and closed-world readings <readings>

Whether an answer may be read as open-world follows from the query and its result, not from the
request. We call a query _monotone_ if, apart from a top-level `ORDER BY`, `OFFSET` and
`LIMIT`, it is a `SELECT` or `ASK` query with only basic or group graph patterns, `UNION`, `GRAPH`,
`VALUES`, projection, `DISTINCT`, and `FILTER` or `BIND` without `EXISTS` or `NOT EXISTS`. A
solution of a monotone query over $D$ is also a solution over any dataset that contains $D$, so
what its answer contains keeps the open-world reading of RDF: a true `ASK`, and each returned row,
stays true however many credentials the holder left out.

An answer that depends on what $D$ lacks reads $D$ as complete, and we call it a _closed-world
answer_: a false `ASK`; a row that `NOT EXISTS`, `MINUS` or an unbound `OPTIONAL` variable
produces; a count; the latest payment; or a `SELECT` result read as the list of all solutions. A
closed-world answer is correct over $D$, as the proof shows, but answers the verifier's question
only if $D$ holds every relevant statement, which no proof over $D$ can show. It is therefore only
as meaningful as the verifier's reason to believe that $D$ is complete.

zkRDF's soundness guarantee, that each returned solution is a solution over signed data @braun26,
corresponds to an answer to a monotone query over credentials the holder chose, read open-world. It
says nothing about the solutions not returned, and a verifier that counts the returned rows itself
counts only what the holder chose to show.

== Who fixed the input <who-fixed>

A closed-world answer reads $D$ as complete, and someone chose $D$. If the request has no
`inputCommitment` (§#ref(<request>, supplement: none)), the input dataset is _chosen by the
holder_: the holder decided which credentials make up $D$ when it answered, and could have left one
out. Over an input dataset chosen by the holder, a closed-world answer covers only the included
credentials; a count, for instance, counts only those.

If the request has an `inputCommitment`, the input dataset is _agreed_: the verifier accepted that
dataset commitment before sending the request, and the presentation's commitment must equal it.
Agreement shows that two commitments are equal, not that $D$ is complete in the world; that depends
on why the verifier agreed (§#ref(<agreed-source>, supplement: none)). Agreeing to whatever
commitment the holder sends in the same exchange gains nothing over an input chosen by the holder.

Who fixed the input is separate from who signed it. With either kind of input, if the request names
trusted issuers, the proof shows that each credential in $D$ carries a signature from one of them.
Signatures show who made the statements in $D$, not that no other statement exists.

= Architecture <architecture>

@fig-architecture shows the three parties. The verifier service sends a query request and stores its
own copy. The holder service builds the input dataset from some of the holder's credentials,
evaluates the query, and returns an answer presentation: the result with one proof. A companion
specification @zksparqlspec defines both as RDF, serialised as JSON-LD @jsonld11. The answer
presentation is a verifiable presentation @vcdm2 that carries a proof in place of credentials,
except in the disclosed mode.

#[
#show figure: set block(breakable: false)
#let arch-paged = grid(
    columns: (1fr, auto, 2.1fr, auto, 1.3fr),
    gutter: 5pt,
    align: horizon,
    rect(width: 100%, inset: 6pt)[
      #set align(left)
      #set par(justify: false)
      #set text(size: 0.8em)
      *Issuers* \
      Sign credentials, for example with `eddsa-rdfc-2022`. May vouch for an agreed dataset commitment.
    ],
    [→],
    rect(width: 100%, inset: 6pt)[
      #set align(left)
      #set par(justify: false)
      #set text(size: 0.8em)
      *Holder* \
      Receives the query request.
      #rect(width: 100%, inset: 5pt, stroke: (thickness: 0.6pt, dash: "dashed"))[
        *Inside the proof:* (1) in the hidden mode, check each signature against the trusted
        issuers' keys; (2) build $D$ from exactly the signed data; (3) evaluate $Q$ over $D$ within
        the request's limits; (4) output the result, the dataset commitment and the request digest.
      ]
    ],
    [→],
    rect(width: 100%, inset: 6pt)[
      #set align(left)
      #set par(justify: false)
      #set text(size: 0.8em)
      *Verifier* \
      *Outside the proof:* check the presentation against the stored request, the proof with the
      verification key that the request gives and, in the revealed mode, each signature; then mark
      the request as answered; then accept the result.
    ],
  )
#figure(
  arch-paged,
  kind: image,
  caption: [
    One proof per answer presentation. Dataset construction, evaluation and, in the hidden mode, the
    signature checks are inside the proof; the verifier's checks and marking the request as
    answered are outside it.
  ],
) <fig-architecture>
]

== The query request <request>

@request-table lists the members of a query request. The verifier service stores the request and its
request digest, the SHA-256 hash of the request's canonical N-Quads under RDFC-1.0, and checks each
presentation against this stored copy, never against anything the holder returns.

The trust requirements name the issuers the verifier trusts. A trust requirement states the
verifier's policy; it does not show that the issuer authorised a key
(§#ref(<bg-sparql>, supplement: none)). A proof method is a way of producing and checking the proof,
named by an IRI, as a cryptosuite is for Data Integrity proofs; a new version is a new method with a
new IRI. Its evidence may be a zero-knowledge proof, possibly interactive and designated-verifier as
in QuickSilver @quicksilver21, a proof that is not zero-knowledge, an attestation from a trusted
execution environment (TEE), or disclosed credentials. The verifier checks it with the verification
key and parameters that its request gives for the method, such as a zkVM image ID, never with a key
from the presentation.

#[
#show figure: set block(breakable: false)
#figure(
  table(
    columns: (auto, 1fr),
    align: (left, left),
    table.header[Member][Meaning],
    [`query`], [The query $Q$: a SPARQL 1.2 `SELECT`, `ASK` or `CONSTRUCT` query that reads only the
      input dataset: no `FROM`, `FROM NAMED` or `SERVICE`, and no function whose value depends on
      when or where it is evaluated, such as `NOW` or `RAND`],
    [`inputCommitment`], [Optional. A dataset commitment that the verifier agreed in advance
      (§#ref(<who-fixed>, supplement: none))],
    [`trustedIssuers`], [Optional. Trust requirements: which issuers the verifier trusts. The one
      defined type gives an issuer and its verification methods with their public keys, so that
      both sides prove and verify against the same keys. Without trust requirements, no credential
      is checked against an issuer],
    [`cryptosuite`, \ `signatureMode`], [With trust requirements: the cryptosuites accepted for the
      issuers' signatures, such as `eddsa-rdfc-2022`, and the accepted signature modes, hidden,
      revealed or disclosed (§#ref(<modes>, supplement: none))],
    [`proofMethod`], [The accepted proof methods, in order of preference: each an IRI (`method`)
      with a verification key (`verificationKey`) and parameters],
    [`maxPresentationBytes`, \ `maxResultSize`], [The largest presentation, in bytes; the most
      solutions, or triples, in the result],
    [`challenge`, `domain`], [Fresh randomness, and the verifier's identifier],
    [`validFrom`, `validUntil`], [The period in which the verifier accepts a presentation],
  ),
  caption: [
    Members of a query request, by JSON-LD term, apart from its type and optional identifier. The
    request digest covers all of them.
  ],
) <request-table>
]

== The answer presentation and its proof <linkage>

The answer presentation carries the request digest, the dataset commitment, the signature mode if
the request has trust requirements, and the result: a SPARQL Query Results JSON document for
`SELECT` and `ASK`, or a graph for `CONSTRUCT`. In the revealed mode, it also carries each
credential's signature and signed message. Its proof names a proof method by IRI, repeats the request's challenge and domain, and holds the
evidence. The statement's public inputs are the request digest, the dataset commitment, the
result, the signature mode and, in the revealed mode, the signed messages; a proof method may bind
them all by one digest. The proof shows that:

+ the dataset commitment fixes $D$;
+ if the request has trust requirements, $D$ is exactly the data that signatures from trusted
  issuers cover, in its signed lexical forms and with each credential's blank nodes kept apart, and
  in the hidden mode the proof also verifies those signatures (§#ref(<modes>, supplement: none));
+ the result is an answer for $Q$ over $D$ (§#ref(<answers>, supplement: none)).

Through the request digest, the proof binds every member of the request: if SHA-256 is
collision-resistant, a proof made for one request does not verify for another.

== Signature modes <modes>

Each proof method states the signature modes it supports per cryptosuite.

- In the _hidden_ mode, the proof shows that the holder knows a valid signature from a trusted issuer
  on every credential in $D$. The signatures, the signed messages and which key signed which
  credential are part of the witness, and stay hidden as far as the proof system is zero-knowledge
  and the public inputs do not disclose them (§#ref(<leakage>, supplement: none)).
- In the _revealed_ mode, the presentation carries each credential's signature and signed message,
  which the verifier checks itself. The proof then shows only that $D$ is exactly the data those
  messages cover and that the result is correct. This mode reduced our guest program's cycles
  (§#ref(<cost>, supplement: none)) but discloses the signatures, the signed messages, the issuers'
  keys and the number of credentials.
- In the _disclosed_ mode, for proof methods whose evidence is disclosed credentials, the holder
  sends the credentials or presentations derived from them by a selective-disclosure cryptosuite.
  The verifier checks them, builds $D$ from what they disclose and evaluates $Q$ itself, seeing
  everything disclosed. An answer to a monotone query then stays true over the full credentials,
  but a closed-world answer covers only what was disclosed.

== Verifier processing <validation>

The verifier service checks a presentation against the request it sent, in this order, and rejects
it at the first check that fails @zksparqlspec:

+ Before parsing, reject a presentation larger than `maxPresentationBytes`; then reject it unless it
  is JSON-LD that uses only the specification's terms.
+ Reject it unless its request digest is that of the stored request, the time is within the
  request's validity period, the proof's challenge and domain equal the request's, and no
  presentation with this challenge has been accepted.
+ Reject it unless it names one of the request's proof methods, whose entry gives the verification
  key and parameters.
+ If the request has an `inputCommitment`, reject it unless the presentation's commitment equals it.
+ If the request has trust requirements, reject a signature mode that the request does not list. In
  the revealed mode, verify each signature under a trusted issuer's key and a listed cryptosuite.
+ Reject a result of the wrong form for the query, or larger than `maxResultSize`.
+ Verify the proof against the statement computed from the stored request and presentation.

The verifier service then marks the request as answered, in one atomic step that fails if it is
already marked, and only then accepts. Checking first stops a malformed presentation from using up a
legitimate request, and the atomic step makes a replay, or the second of two concurrent
presentations, fail. Sending a presentation discloses its answer, even if the verifier rejects it.

= Revealing less <minimize>

A zero-knowledge proof hides its witness, yet the verifier knows some witness values as soon as it
has the result. A proof method may make public any value that the verifier can compute from its
stored request and the result alone; no request member permits disclosing more. This public-input
rule extends zkRDF's disclosure of query constants and projected terms @braun26. Our argument for it
concerns disclosure only: such a value tells the verifier nothing that its request and the result do
not. Our pilot is too small to show a saving in proving cost (§#ref(<proved>, supplement: none)).

== The public-input rule <rule>

Suppose the lender asks `SELECT ?p WHERE { ?p a ex:Payment . ?p ex:paymentStatus ex:Settled . }` and
receives one row, in which `?p` is `ex:pay1`. Substituting the row into the two triple patterns
gives `ex:pay1 a ex:Payment` and `ex:pay1 ex:paymentStatus ex:Settled`, which the verifier can
compute, so a proof method may make them public inputs. The proof must still show that each triple
supplied as a public input is in $D$ and signed: making a triple public removes its secrecy, not the
need to authenticate it. Unless the cryptosuite supports selective disclosure, the signature check
still covers the whole credential.

In queries other than a single basic graph pattern, a returned row may not determine which triples
matched. In `?p a ex:Payment OPTIONAL { ?p ex:returnReason ?r }`, a row with `?r` unbound shows that
no return-reason triple for `?p` matched; no triple stands for that absence. In
`{ ?p ex:paymentStatus ex:Returned } UNION { ?p ex:paymentStatus ex:Reversed }`, a row does not show
which branch matched, so the matched triple is not computable. `FILTER(?a = 1250)` holds for both
`"1250.00"^^xsd:decimal` and `"1250.0"^^xsd:decimal`, different terms with equal values, so a filter
does not fix the term of an unprojected variable. And triples of the returned rows say nothing about
rows not returned. A method with several rounds could send such a value before the result, and an exchange that then
aborted would have disclosed facts that no answer did. We propose that a party receive these public
inputs no earlier than the answer from which it can compute them. The prototype does not enforce
this condition.

== What the verifier still learns <leakage>

Beyond its result, an accepted answer reveals the dataset commitment, which links presentations if
the holder reuses a salt over the same credentials (an agreed commitment links them by design, to the
verifier that agreed it); the proof method, whose capacity limits bound the size of the input; the
issuers: in the hidden mode, only that a trusted issuer signed each credential, which identifies the
issuer if the request trusts only one, and in the revealed mode, each key and the number of
credentials; and the size of the result, such as how many payments matched.
// Source: security-table-addenda.md, item 4a (ZK code landing, 10 October 2026): succinct receipts
// expose the recursion control ID, which can reveal part of the execution's shape
// (zk/sparql-evaluator/README.md); RISC Zero's zero-knowledge is its claim, not established.
A RISC Zero receipt also reveals part of the shape of the execution; the zero-knowledge of these
receipts is RISC Zero's claim, not an established property. Where SPARQL permits several results,
the holder's choice can carry information (§#ref(<answers>, supplement: none)): for
`SELECT ?x WHERE { VALUES ?x { 0 1 } } LIMIT 1`, the returned value can encode one bit of the
holder's choosing, a covert channel that zero-knowledge does not prevent because the result is
public. It matters when the holder does not fully trust its holder service. A sequence of queries
can also reveal together what no single query reveals.

= Cryptosuites and signature modes <cryptosuites>

== Existing cryptosuites <existing-suites>

Two RDF cryptosuites sign the same structure as `eddsa-rdfc-2022`
(§#ref(<bg-sparql>, supplement: none)), which every zkVM experiment with signature checks in
§#ref(<evidence>, supplement: none) uses.
`ecdsa-rdfc-2019` uses ECDSA, with P-256 and SHA-256 or P-384 and SHA-384 @vcdiecdsa; we verify it
only outside the zkVM.
// TODO(evidence): bind to the implementation state reported by the ZK code landing thread
// (zksparql-evaluation-plan.md §8: ecdsa-rdfc-2019 verified natively only).
`mldsa44-rdfc-2024` uses ML-DSA-44 @fips204 with SHA-256 and is designed to resist forgery by a
quantum adversary; it is in a W3C First Public Working Draft @vcdiqr, and we have not implemented
it. None of the three salts its signed message, so whoever sees the hashes can test a guessed
document. To check such a signature inside a proof, the prover hashes each credential's whole
canonical document and must parse lexical forms to compare typed values. The selective-disclosure
cryptosuites `bbs-2023` @vcdibbs and `ecdsa-sd-2023` @vcdiecdsa fit the disclosed mode; only
`bbs-2023` derived proofs are unlinkable.

== Our Merkle-based cryptosuites <merkle-suites>

// Source: the cryptosuite draft (sparq-org/sparq#6789) specifies all three members below, and its
// Poseidon2 member signs P(3, suite, salt, n, root, c1, c0), which includes the suite identifier.
We propose Merkle-based cryptosuites designed for proving @merklesuites. The issuer builds a Merkle
tree with one leaf per canonical quad and signs a digest of the suite identifier, a fresh 32-byte
salt, the number of quads, the Merkle root and the digest of the canonical proof configuration.
Each leaf encodes the typed terms of its quad, and a literal's encoding carries a comparison key: an
order-preserving encoding of its value.

This has three effects. First, a proof need not repeat canonicalisation: it rebuilds the signed
tree from the credential's quads, which took fewer cycles (§#ref(<cost>, supplement: none)). Second, a proof compares values without parsing lexical forms:
in a circuit, comparing two integers or decimals that the key can represent takes one range check
on the difference of their keys. By a preliminary gate count of our Noir circuits for integer
filters, range checks on literals committed as hashed text account for most of the gates.
// TODO(evidence): bind to the UltraHonk gate breakdown (project file
// zk-proof-methods/ultrahonk-gate-breakdown.md) once it is frozen as an evidence record.
Third, a fresh, random and secret salt makes the signed message computationally hiding, if the hash
is modelled as a random oracle, so that a verifier in the revealed mode cannot confirm a guessed
credential.

The family has three members. `eddsa-sha256-merkle-2026` uses a SHA-256 tree and Ed25519, for zkVMs;
a later build of our guest program verifies it, executed without proving
(§#ref(<cost>, supplement: none)). `schnorr-poseidon2-merkle-2026` uses a Poseidon2 tree and Schnorr
signatures over Baby Jubjub, for circuits, and the post-quantum `mldsa44-sha256-merkle-2026` uses
ML-DSA-44; we have not implemented these two.

== Security and disclosure <security>

@security-table compares what the verifier learns in each signature mode, and supplementary Table S1
lists the assumptions behind each signature scheme and proof system. In the hidden mode, what the
verifier learns depends on the public inputs and on the zero-knowledge of the proof system, not on
the cryptosuite. Whether a quantum adversary can forge a signature depends on the cryptosuite, not on
the mode. Outside the disclosed mode, the proof system's knowledge soundness must also hold: a
quantum adversary could forge UltraHonk proofs, even ones made earlier, whereas we infer that Shor's
algorithm does not break the knowledge soundness of RISC Zero's STARK-based succinct receipts. Our
zkVM proof method accepts only these, not RISC Zero's Groth16 receipts, which are knowledge-sound
only against a classical adversary.

#[
#show figure: set block(breakable: false)
#figure(
  {
    set text(size: 0.8em)
    set par(justify: false)
    table(
      columns: (1.3fr, 1.6fr, 1fr, 1fr),
      align: (left, left, left, left),
      table.header(
        table.cell(fill: luma(235))[*Signature mode (cryptosuites)*],
        table.cell(fill: luma(235))[*The verifier also receives*],
        table.cell(fill: luma(235))[*Links presentations*],
        table.cell(fill: luma(235))[*Confirms guessed content*],
      ),
      [Hidden (any)], [A salted dataset commitment and, from a RISC Zero receipt, part of the
        execution's shape; no signature, signed message or issuer key],
        [Only through a commitment repeated by reusing a salt, or an agreed commitment], [No],
      [Revealed (`eddsa-rdfc-2022`, `ecdsa-rdfc-2019`, `mldsa44-rdfc-2024`)], [Per credential:
        signature, signed message and verification method; so the issuers' keys and the number of
        credentials], [Yes], [Yes: the document hash is unsalted],
      [Revealed (Merkle-based)], [As above, without the salt], [Yes],
        [No, while the salt is fresh and private],
      [Disclosed (whole credentials)], [The signed credentials], [Yes], [Not applicable],
      [Disclosed (`ecdsa-sd-2023`)], [Disclosed and mandatory quads, the base signature],
        [Yes], [No],
      [Disclosed (`bbs-2023`)], [Disclosed and mandatory quads, a BBS proof],
        [Only through what is disclosed], [No],
    )
  },
  caption: [
    What the verifier learns, by signature mode and cryptosuite. "Links presentations" means
    through a value the verifier receives, without breaking any assumption. RISC Zero claims, but
    has not established, that its receipts are zero-knowledge; the zero-knowledge UltraHonk variant
    we use is designed to be statistically zero-knowledge, but its documentation cites no proof. In
    every mode, the result and its size can also reveal data and link presentations.
  ],
) <security-table>
]
// Sources: scratchpad security-table.md (Tables A to C; every cell used here is sourced there)
// and security-table-addenda.md (UltraHonk as run; quantum binding of the trees; succinct-only
// receipts; RISC Zero zero-knowledge claim; Schnorr over Baby Jubjub argument).

= Evaluation <evidence>

== Prototype and method <prototype>

Our services encode query requests in binary, not as the JSON-LD of
§#ref(<architecture>, supplement: none), and compute the request digest over that encoding. Our Noir
circuits publish no dataset commitment, so they cannot yet produce the specification's
presentations. In the zkVM, the revealed mode and our Merkle-based
cryptosuite exist only in a later build, which we executed without proving.

Our guest program runs a SPARQL evaluator inside the RISC Zero zkVM and writes the result, the
dataset commitment and whether the input was agreed to the journal. A second build adds signature
checks in front of the same evaluator: in the hidden mode, it verifies each credential's
`eddsa-rdfc-2022` signature under a trusted issuer's key and evaluates the query over the signed
canonical N-Quads. Our holder and verifier services wrap both builds with query requests and the
checks of §#ref(<validation>, supplement: none); the journal binds the stored request through a
digest (supplementary §S2). We have not confirmed that the verifier service performs its checks in
exactly that order.

We used only synthetic credentials signed with test keys. We generated all receipts with development
mode disabled and verified them against the expected guest image ID. For experiments without † in
@evidence-table, we recomputed hashes of source files, guest binaries and receipts. We compared the
hashes and recorded test outcomes with the archived records. We did not verify the proofs again. The
supplementary material lists the test cases and records (§S3) and the software behind them (§S8).

#[
#show figure: set block(breakable: false)
#figure(
  {
    set text(size: 0.8em)
    set par(justify: false)
    table(
      columns: (1.3fr, 1.6fr, 0.75fr, 1.6fr),
      align: (left, left, left, left),
      table.header[Experiment][Answers][Signature checks][Strongest result],
      [Evaluator, CI build], [`SELECT` with duplicate rows, false `ASK`, `CONSTRUCT`,
        `DESCRIBE`; both kinds of input, not every form under each], [None],
        [Proof verified: #headline("zkvcq.exact_hosted_receipts") receipts],
      [Evaluator with our services], [`SELECT`, true and false `ASK`, `CONSTRUCT`; both kinds
        of input], [None], [Proof verified: #headline("zkvcq.adapter_receipts") receipts,
        #headline("zkvcq.adapter_row_bound_rejected") rejected at the row limit;
        #headline("zkvcq.adapter_controls") negative tests],
      [Evaluator with signature checks, executed directly], [Answers over the W3C test vector;
        both kinds of input], [In the guest program], [Executed without proving:
        #headline("zkvcq.v5g_direct_v5_positive") inputs accepted,
        #headline("zkvcq.v5g_direct_v5_aborts") altered inputs rejected],
      [Same, with our services: first test case], [`SELECT` with duplicate rows; agreed input],
        [In the guest program], [Proof verified: #headline("zkvcq.vcqg_genuine_receipts")
        receipt; #headline("zkvcq.vcqg_controls") negative tests],
      [Same: payment question †], [False `ASK`; both kinds of input], [Same], [Proof verified:
        one receipt per kind of input; #headline("zkvcq.vcqp_controls") (agreed) and
        #headline("zkvcq.vcqph_controls") (chosen by the holder) negative tests],
      [Same: remaining test cases †], [`SELECT`, true and false `ASK`, `CONSTRUCT`; both kinds of
        input; row limit], [Same], [Proof verified: one receipt per test case; each accepted one with
        #headline("zkvcq.ci_asktva_controls") (agreed) or #headline("zkvcq.ci_askfhd_controls")
        (chosen by the holder) negative tests; the row-limit case rejected],
      [Public-triple circuit pilot], [Returned rows of `SELECT DISTINCT` are solutions; one basic
        graph pattern],
        [Schnorr, own format, in the circuit], [Proof verified: #headline("zkvcq.pp_genuine_proofs") proofs;
        #headline("zkvcq.pp_tamper_controls") tampering and
        #headline("zkvcq.pp_replay_controls") replay tests rejected],
    )
  },
  caption: [
    Strongest result per experiment: tests outside the zkVM, executed without proving, or proof
    verified. Each zkVM experiment with signature checks used one credential per test case. Cases
    with an input chosen by the holder have two fewer negative tests, because swapping the agreed
    commitment does not apply to them. † No internal check (§#ref(<prototype>, supplement: none)).
    Counts are per experiment and are never added across rows.
  ],
) <evidence-table>
]

== Which queries the evaluator answers <expressivity>

Our evaluator implements SPARQL 1.1 @sparql11, without the additions of SPARQL 1.2 such as triple
terms. _Proved._ With signature checks, our proofs cover `SELECT` results that keep duplicate rows, true and
false `ASK`, and `CONSTRUCT`, each over one credential and a basic graph pattern
(§#ref(<proved>, supplement: none)); the false `ASK` is a closed-world answer. Without signature
checks, they also cover `UNION`, `VALUES`, `OPTIONAL`, `MINUS`, `EXISTS`, `NOT EXISTS`, a counting
subquery, property paths, `ORDER BY` with `LIMIT`, `FROM NAMED` and `DESCRIBE`
(§S3.1 of the supplementary material).

_Executed without proving._ With signature checks, we ran a sweep of #headline("zkexec.sweep_cases")
query cases, one per feature, in each configuration of §#ref(<cost>, supplement: none): first in
the evaluator outside the zkVM, which decided whether to admit each case, then in the guest
program. With one credential, the evaluator admitted
#headline("zkexec.sweep_rdfc_hidden_n1_s32_admitted") cases, and the guest program completed all
of them. They cover basic graph patterns of several shapes; `OPTIONAL`, `UNION`, `MINUS`, `BIND`,
`VALUES` and a subquery; comparisons of IRIs, strings, language-tagged strings, numbers and
date-times; `STRSTARTS`, `CONTAINS`, `REGEX` and `LANGMATCHES`; `COUNT`, `SUM`, `MIN` and `MAX`
with `GROUP BY` and `HAVING`; `DISTINCT`; `ORDER BY` with `LIMIT` and `OFFSET`; sequence,
alternative, inverse, one-or-more and zero-or-more property paths; and `SELECT`, `ASK` and
`CONSTRUCT`.
// [OPUS-5.5] Case list: the sweep audit's `cases`
// (research/zk-paper-evidence/paper-executor-sweep-audit.json). The admitted and completed counts
// are equal in every one-credential configuration; asserts beside the sweep table (supplementary
// material) keep these two sentences true if the evidence changes.

_Not supported._ The evaluator rejected `SERVICE`; `NOW`, `RAND` and `BNODE`, whose values the
query and the data do not fix; a custom function; a triple term; a nested `EXISTS`; and `EXISTS`
and `NOT EXISTS` over any credential with a blank node, as each generated credential has. With four
credentials, it also rejected a zero-or-more property path whose result exceeded its limits. The
guest program rejected the same cases, except in some runs over four credentials that reached the
session limit first. The evaluator admitted `DESCRIBE`, `FROM` and
`FROM NAMED`, but with signature checks the credentials form the default graph, and our prover and
verifier reject these three before proving and before verification. Our services also reject a
`SELECT` query with a top-level `ORDER BY`, so they return `SELECT` results only as multisets. The build with signature
checks accepts at most four credentials of at most 128 triples each, and 256 triples in total.
// Sources: the sweep audit's rejected_runs, one rejection_cause per rejected run (guest_abort or
// session_limit), and /reruns (zkexec.sweep_reruns); the DESCRIBE/FROM answer from the ZK code
// landing thread (10 October 2026); the services' shared admission check, which rejects DESCRIBE
// and a SELECT whose result keeps the order of a top-level ORDER BY
// (zk/sparql-evaluator/host/src/vcq.rs, checked_admission); and the capacity constants
// MAX_CREDENTIALS, MAX_DOCUMENT_QUADS and MAX_TOTAL_QUADS in
// zk/sparql-evaluator/model/src/authenticated_rdf.rs at the sweep's source commit.

== Proved answers <proved>

Without signature checks, the evaluator produced #headline("zkvcq.exact_hosted_receipts") receipts
in a CI build and #headline("zkvcq.adapter_receipts") with our services (@evidence-table), which
show evaluation bound to a request, not who issued the data. Our verifier service rejected
#headline("zkvcq.adapter_row_bound_rejected") receipt whose result exceeded the row limit: a valid
proof is necessary, not sufficient, for acceptance.

With signature checks and our services, the evaluator produced one receipt per test case, each over
one credential. For the payment question, the credential lists three settled payments and is signed
with the public RFC 8032 test key in place of a bank's key. With both kinds of input, the journal
reports `false`, and the verifier accepted the answer. Each accepted receipt came with negative
tests, each with its expected outcome (supplementary §S3). We report no timings for these runs.

Our two Noir circuits prove, for `SELECT DISTINCT` queries over one basic graph pattern, that each
returned row is a solution, not that none is missing; such a proof gives an answer only where those
rows form a permitted result, as when they reach the query's `LIMIT`. The matched triple is part of
the witness in the baseline circuit and a public input in the public-triple circuit; both make the
same signature and membership checks (@evidence-table). Preliminary timings, from
#headline("zkvcq.pp_n_per_cell") runs per circuit and capacity on a shared machine (supplementary
§S4), are too few to show a saving.

== Cost by signature mode and cryptosuite <cost>

We counted cycles with the RISC Zero executor, which runs the guest program without proving, using
the later build with signature checks; a cycle count is not a proving time. The executor also splits
each execution into segments, which a prover proves separately. We ran #headline("zkexec.main_queries") queries over the payment credential: the payment `ASK`, a
`SELECT` that keeps duplicate amounts, a `CONSTRUCT` over that pattern, a `FILTER` on `xsd:decimal`
amounts, and a string `FILTER` on payment IRIs, each over one credential and over four, of
#headline("zkexec.main_statements") triples each. @cost-table gives the median over the queries. In
every configuration, the revealed mode needed fewer cycles and segments than the hidden mode, and
`eddsa-sha256-merkle-2026` fewer cycles than `eddsa-rdfc-2022`.

#[
#show figure: set block(breakable: false)
// Median user cycles (in millions) and median segments over the five main-body queries.
#let cost-cells(stem) = (
  [#mcycles(stem + "_n1_user_median")], [#headline(stem + "_n1_segments_median")],
  [#mcycles(stem + "_n4_user_median")], [#headline(stem + "_n4_segments_median")],
)
#figure(
  {
    set text(size: 0.8em)
    set par(justify: false)
    table(
      columns: (1.5fr, 0.9fr, auto, auto, auto, auto),
      align: (left, left, right, right, right, right),
      table.header(
        table.cell(rowspan: 2)[Cryptosuite], table.cell(rowspan: 2)[Signature mode],
        table.cell(colspan: 2)[One credential], table.cell(colspan: 2)[Four credentials],
        [Cycles, millions], [Segments], [Cycles, millions], [Segments],
      ),
      [`eddsa-rdfc-2022`], [Hidden], ..cost-cells("zkexec.main_rdfc_hidden"),
      [`eddsa-rdfc-2022`], [Revealed], ..cost-cells("zkexec.main_rdfc_revealed"),
      [`eddsa-sha256-merkle-2026`], [Hidden], ..cost-cells("zkexec.main_merkle_hidden"),
      [`eddsa-sha256-merkle-2026`], [Revealed], ..cost-cells("zkexec.main_merkle_revealed"),
    )
  },
  caption: [
    Executor counts for the guest program with signature checks, as medians over the
    #headline("zkexec.main_queries") queries: cycles, without the prover's overhead and padding per
    segment, and segments. The counts are deterministic for a given guest program and input; they
    are not proving times.
  ],
) <cost-table>
]

An instrumented build reads the cycle counter at the end of each phase (@phase-table, the payment
`ASK` over one credential). In the hidden mode, verifying the Ed25519 signature was the largest phase with both
cryptosuites; revealing the signatures removed it and left the others almost unchanged. The
Merkle-based cryptosuite replaces canonicalising and hashing the document with building its Merkle
tree, which took fewer cycles, but its signature verification took no fewer, as it also uses
Ed25519. Evaluating the query took fewer cycles than processing the document or verifying the
signature.

#[
#show figure: set block(breakable: false)
// Cycles (in millions) per phase of the instrumented build, payment ASK over one credential.
#let phase-cell(suite, mode, phase) = [#mcycles("zkexec.q1_" + suite + "_" + mode + "_n1_" + phase)]
#let phase-cells(phase) = (
  phase-cell("rdfc", "hidden", phase), phase-cell("rdfc", "revealed", phase),
  phase-cell("merkle", "hidden", phase), phase-cell("merkle", "revealed", phase),
)
#figure(
  {
    set text(size: 0.8em)
    set par(justify: false)
    table(
      columns: (2.2fr, auto, auto, auto, auto),
      align: (left, right, right, right, right),
      table.header(
        table.cell(rowspan: 2)[Phase, in order of execution],
        table.cell(colspan: 2)[`eddsa-rdfc-2022`],
        table.cell(colspan: 2)[`eddsa-sha256-merkle-2026`],
        [Hidden], [Revealed], [Hidden], [Revealed],
      ),
      [Reading and decoding the input], ..phase-cells("input"),
      [Checking the request, issuers' keys and input sizes], ..phase-cells("request"),
      [Canonicalising and hashing the proof configuration], ..phase-cells("proof_config"),
      [Canonicalising and hashing the document, or building its Merkle tree; checking its issuer],
        ..phase-cells("document"),
      [Verifying the signature], phase-cell("rdfc", "hidden", "signature"), [–],
        phase-cell("merkle", "hidden", "signature"), [–],
      [Building the input dataset and its commitment], ..phase-cells("mapping"),
      [Evaluating the query], ..phase-cells("query"),
      [Computing the request digest and the journal], ..phase-cells("journal"),
    )
  },
  caption: [
    Cycles in millions per phase for the payment `ASK` over one credential, from an instrumented
    build of the guest program. The counts include the instrumentation; they are not proving times.
  ],
) <phase-table>
]

The sweep of §#ref(<expressivity>, supplement: none) ran in the same configurations over generated
credentials of #headline("zkexec.sweep_statements_small") and
#headline("zkexec.sweep_statements_large") triples each (supplementary §S5). Our prover stops an execution that reaches a session limit of #mcycles("zkexec.sweep_session_limit")
million cycles, and then produces no proof. With `eddsa-rdfc-2022` in the hidden mode, four
credentials of #headline("zkexec.sweep_statements_large") triples exceeded this limit in every case
that the evaluator admitted. In every other configuration, including the revealed mode and the
Merkle-based cryptosuite over the same credentials, every admitted case completed within it.
// Source: paper-executor-sweep-audit.json /configurations/*/rejected_runs, one rejection_cause per
// rejected run (guest_abort or session_limit), and /reruns (zkexec.sweep_reruns). The audit records
// no count per cause, so the sentence on rejected cases in the expressivity subsection gives none.

#todo-results[Proving time, memory and receipt size per configuration and query; issuer, holder
and verifier costs; and a comparison, over the same queries, with re-evaluation over disclosed
credentials, selective disclosure and, where built, designated-verifier proofs and TEE attestation.
These need a dedicated proving run with development mode disabled on one machine.]

= Related work <related>

_Zero-knowledge SPARQL over credentials._ zk-SPARQL @yamamotozksparql answers `SELECT` and `ASK`
queries with one basic graph pattern over BBS+-signed credentials, with `FILTER`s only on disclosed
variables. It, zkRDF and Wright's zkVM prototype (§#ref(<bg-zkrdf>, supplement: none)) each fix one
signature scheme, BBS+ or Ed25519; our requests choose among cryptosuites and signature modes.
Wright's prototype took about 7.5 minutes for `SELECT *` over one credential of 23 triples (§5 of @wright25dc), and zkRDF reports proving three orders of magnitude faster on three
small queries, under BBS+ signatures (§5.2 of @braun26); our proving times are pending. We add
closed-world answers over a stated input of issuer-signed credentials, and request terms that say
who fixed that input.

_Querying credentials without zero knowledge._ De Mulder et al. express a SPARQL subset in
OpenID4VP; the holder returns an ordinary verifiable presentation, and the verifier checks the
signatures and evaluates the query itself (§2 of @demulder25), as in our disclosed mode. The
Digital Credentials Query Language of OpenID4VP selects claims by path, with optional exact value
matching (§6 and §7 of @openid4vp); AnonCreds @anoncreds adds integer inequalities; and SD-JWT
@sdjwt and the Data Integrity BBS cryptosuites @vcdibbs disclose claims without predicates, while CL
signatures @cl01, zk-creds @zkcreds and Crescent @crescent prove statements about signed
attributes. A query request could travel in OpenID4VP as a new credential format, but the protocol
has no step in which a verifier agrees an input in advance.

_Verifiable query processing._ Query authentication distinguishes correctness, meaning that every
returned record is in the owner's database unmodified, from completeness, meaning that no answer is
omitted @li06. In these terms our answers are correct and complete over $D$, and zkRDF proves
correctness. Complete answers over a committed database exist for SQL, in IntegriDB @integridb, vSQL
@vsql and, in zero knowledge, ZKSQL @zksql, whose proofs are interactive (§2.1 of @zksql). VeriDKG
proves SPARQL results over a decentralised knowledge graph sound, complete and fresh, without
privacy (Theorem 7.1 of @veridkg23). In each, the data's owners commit to it in advance, the
counterpart of our agreed input; our input is a holder's credentials from independent issuers,
authenticated inside the proof.

= Discussion and limitations <agreed-source>

A closed-world answer over an agreed input is evidence about all the relevant records only if a
party the verifier trusts fixed an input that covers them, an assumption we do not discharge. For
the payment question, the bank could sign a dataset commitment that covers the credentials it issued to the applicant for a period, or the verifier could agree a commitment in an earlier
exchange in which it learned which credentials the dataset holds. We do not claim that such
infrastructure exists. Without it, a deployment can offer only inputs chosen by the holder.

The SPARQL evaluator running in the zkVM does not check credential status, holder binding or
credential validity periods, and the query request has no member for them. The prototype has not
undergone an external security audit. Our arguments for what acceptance implies
(Appendix #ref(<app-relation>, supplement: none)) are informal and rest on named assumptions that
tests support but do not establish. Negative tests show that specific checks exist and fire; they
cannot rule out substitutions we did not try. The evaluator is covered by tests, not by a
conformance suite. Every zkVM proof with signature checks covers one synthetic credential and a
basic graph pattern, and our cycle counts cover at most four synthetic credentials of
#headline("zkexec.sweep_statements_large") triples each, so we have no evidence yet about realistic
credentials, larger inputs or proving times. We consider a single holder.
#if opening == "agents" [We have not evaluated agents or agent protocols.]

= Conclusion <conclusion>

A holder can answer a verifier's SPARQL query over credentials without handing them over: one proof
can show that the answer is a result SPARQL 1.2 permits over a stated input dataset, closed-world
answers included, and that trusted issuers signed every credential in it. Because closed-world
answers are only as meaningful as the verifier's reason to believe the input complete, the request
states who fixed it. A prototype evaluator running in a zkVM has proved such answers with signature
checks, each over one synthetic credential. Realistic credentials, proving times and an external
audit remain to be done.

// ---------------------------------------------------------------------------------------------
// Statements outside the page limit (ESWC: references, acknowledgements, the generative-AI
// statement and the supplemental material statement do not count).
// ---------------------------------------------------------------------------------------------
#[
#set text(size: 9pt)
#set par(first-line-indent: 0pt)
#v(1em)
*Supplemental Material Statement.* The supplementary material
(#if anon [anonymous link: #author-todo[anonymised URL]] else [#author-todo[URL]]) gives the
security assumptions behind each signature scheme and proof system (§S1), what our proof methods
support (§S2), the experiment records with every test case, run and negative test (§S3), the
timings of the circuit pilot (§S4), the executor cycle counts of every sweep configuration (§S5),
an earlier fixed-circuit design (§S6), an experiment combining BBS+ proofs with circuits (§S7), and
the software behind each experiment (§S8).
#if anon [For double-blind review, it withholds the proof-method identifiers, commits, and the
digests and image IDs of guest binaries, which identify our repository.]
#author-todo[Authors: state where the source code, guest binaries, receipts and evidence records
are available, and under which licence.]

*Declaration on Generative AI.* #author-todo[Authors: describe the use of generative AI tools in
preparing this paper, following the ESWC policy, or state that none were used.]
]

#let bib-entries = yaml("zksparql-architecture.refs.yml")
#let bib-entries = if anon {
  bib-entries + (
    zksparqlspec: (
      type: "reference",
      title: "Companion specification of query requests and answer presentations (anonymised for review)",
      author: "Anonymous",
      date: 2026,
    ),
    merklesuites: (
      type: "reference",
      title: "Draft specification of the Merkle-based cryptosuites (anonymised for review)",
      author: "Anonymous",
      date: 2026,
    ),
  )
} else { bib-entries }
#[
#set text(size: 9pt)
#bibliography(
  bytes(yaml.encode(bib-entries)),
  style: "springer-lecture-notes-in-computer-science",
  title: [References],
)
]

// ---------------------------------------------------------------------------------------------
// Appendix: at most two LNCS pages after the references.
// ---------------------------------------------------------------------------------------------
#pagebreak(weak: true)
#counter(heading).update(0)
#set heading(numbering: "A.1")

= What an accepted presentation shows <app-relation>

For a zkVM proof method in the hidden or revealed mode, we argue informally that, under assumptions
A1 to A5 below, an accepted presentation means what §#ref(<meaning>, supplement: none) says.

*Relation.* Let $Q$ be the query of the request whose request digest is a public input
(§#ref(<linkage>, supplement: none)). The witness consists of the credentials, a salt and, in the
hidden mode, the credentials' signatures. Public inputs and witness satisfy the relation if:

+ the dataset commitment is the proof method's commitment to these credentials under this salt;
+ the input dataset $D$ is built from these credentials as the proof method states
  (§#ref(<meaning>, supplement: none)); with trust requirements, from their signed canonical
  N-Quads, so that every term keeps its signed lexical form;
+ if the request has trust requirements, each credential carries a signature that verifies under a
  trusted issuer's key, and the credential's issuer, verification method and cryptosuite match that
  key's entry and the request. The proof verifies these signatures in the hidden mode. In the revealed mode, the signed messages are public
  inputs computed from the credentials, and the verifier checks the signatures;
+ the result is an answer for $Q$ over $D$ (§#ref(<answers>, supplement: none)).

*Assumptions.*

/ A1: The proof system is knowledge-sound for the verifier's image ID. From any prover whose
  receipt verifies, an efficient extractor obtains an input on which the guest program completes
  and writes that receipt's journal. We accept only succinct receipts
  (§#ref(<security>, supplement: none)).
/ A2: The guest program completes only on inputs that satisfy the relation for the values it
  writes to the journal; in particular, its result is an answer for $Q$ over $D$.
/ A3: SHA-256 is collision-resistant, so the request digest determines the request and the dataset
  commitment is binding.
/ A4: The verifier service performs the checks of §#ref(<validation>, supplement: none) against its
  stored request, with the image ID that the stored request gives, and takes the result only from
  the journal.
/ A5: The cryptosuite's signatures are unforgeable, each key in the trust requirements belongs to
  the issuer that its entry names, and that issuer keeps the private key secret.

Tests support A2 and A4 (supplementary §S3 of the supplementary material); A1, A3 and A5
concern the proof system, the hash function and the issuers.

*Argument.* Suppose the verifier accepts a presentation. By A4, the receipt verifies under the
verifier's image ID for a journal that holds the stored request's digest, whether its input is
agreed, and the presented commitment and result. By A1, an input exists on which the guest program
writes this journal; by A2, that input satisfies the relation. By A3, except with negligible
probability, $Q$ is the stored request's query and the commitment fixes $D$. The result is therefore
an answer for $Q$ over $D$, and for an agreed input, $D$ is the dataset whose commitment the
verifier agreed to. If the request has trust requirements, each credential in $D$ carries a
signature under a trusted issuer's key, which by A5 only that issuer could have made.
If $Q$ is a monotone query, every solution of $Q$ over $D$, before its top-level `ORDER BY`,
`OFFSET` and `LIMIT`, is also a solution over every dataset that contains $D$ graph by graph. A true
`ASK` and each returned row therefore stay true when the holder's other credentials are added.

In the prototype, the journal binds the stored request through a SHA-256 hash
(§S2 of the supplementary material), so under A3 it also binds the challenge, the verifier's
identifier and the validity period.

*Public inputs computable from the result.* Suppose the pattern of $Q$ is a single basic graph
pattern without blank nodes, and a returned row binds each of its variables to an IRI or a literal.
The row is then a solution of the pattern over $D$ exactly when every triple obtained by
substituting the row into the pattern is in the default graph of $D$. A circuit that takes these
triples as public inputs, and checks that each is in $D$ and signed, therefore shows the same
statement as one that finds them in its witness. It discloses nothing that the verifier cannot
compute from the row. The baseline and public-triple circuits of
§#ref(<proved>, supplement: none) differ in exactly this way.
