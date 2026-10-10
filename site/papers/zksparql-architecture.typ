// [OPUS-5.5] zkSPARQL architecture paper. The main body follows the language guide
// (site/papers/zksparql-language-guide.md) and the companion specification of zero-knowledge
// SPARQL answers (sparq-org/sparq#6786): query request and answer presentation, Supported and
// Exact answers, holder-declared and verifier-agreed inputs, the hidden, revealed and disclosed
// signature modes, the public-input rule and the verifier's processing order. The appendix keeps
// its earlier wording until its own rewrite.
//
// HONESTY FRAME (empirical-honesty mandate; external-audit gate sq-qhy4): the paper asserts no
// proven security, privacy or integrity property for any implementation. Every number comes from
// paper-evidence.json through #headline(...) (canonical records) or #ev(...) (the preliminary
// pilot timings, in the appendix); none is typed into prose, and counts from different
// experiments are never added. Experiments without the internal re-check carry a dagger in the
// evidence table. Facts not yet frozen as evidence records (guest coverage, cycle counts, the
// proof-method comparison, Merkle-suite implementation status) appear only qualitatively, each
// with a TODO(evidence) comment. Planned measurements are visible todo-results placeholders.

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
// A reference to a heading drops the numbering's trailing dot, so that "§3.1" and "Appendix A"
// read cleanly in running text.
#show ref: it => {
  let el = it.element
  if el != none and el.func() == heading and el.numbering != none {
    context {
      let num = numbering(el.numbering, ..counter(heading).at(el.location()))
      let num = if type(num) == str { num.trim(".", at: end, repeat: false) } else { num }
      let body = if it.supplement == none {
        num
      } else if it.supplement == auto {
        [#el.supplement~#num]
      } else {
        [#it.supplement~#num]
      }
      link(el.location(), body)
    }
  } else {
    it
  }
}

// Short commit / digest rendering for bound identity records (first twelve hex characters).
#let short-id(key) = raw(headline(key).slice(0, 12))

// Fixed three-decimal rendering for the preliminary pilot values (a display helper; the value
// itself always comes from the evidence file).
#let fmt3(x) = {
  let s = str(calc.round(x, digits: 3))
  let parts = s.split(".")
  if parts.len() == 1 { s + ".000" } else { s + "0" * calc.max(0, 3 - parts.at(1).len()) }
}
#let pilot(key) = fmt3(ev(key))

// A visible placeholder for a planned measurement that is not yet an evidence record.
#let todo-results(body) = block(inset: 6pt, stroke: 0.8pt + red, width: 100%, breakable: false)[#text(fill: red)[*Results pending.* #body]]

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
  Working draft. The measurements marked as pending in §#ref(<planned>, supplement: none) are
  not yet available.
]]

#heading(level: 2, numbering: none, outlined: false)[Abstract]

A verifier often needs the answer to a question about a person's credentials, not the credentials
themselves. Existing zero-knowledge methods for SPARQL over credentials prove that each returned
solution follows from signed data, but cannot show that something is absent, such as a returned
payment. We define Supported answers, in which every returned solution is a solution, and Exact
answers, the complete result over a committed input dataset. As an Exact answer is only as complete
as its input, the verifier's query request also states who fixed the input: the holder, or the
verifier in advance. In our architecture, one proof shows that the answer is correct over exactly
the data the issuers signed. Three signature modes trade proving cost against what the verifier
learns, and a public-input rule lets a proof make public any value that the verifier can compute
from its request and the result alone. We also propose Merkle-based cryptosuites designed for
proving. A SPARQL evaluator running in a zero-knowledge virtual machine has produced verified proofs
of Exact answers with issuer signatures checked inside the proof, including an answer that no
payment was returned, each over one synthetic credential. Cost measurements are pending.

== Introduction <intro>

A lender assessing a mortgage application asks the applicant one question: was any payment from the
applicant's account returned unpaid? The usual answer is a bank statement, which also discloses
every payment, balance and payee that the lender did not ask about. If the bank issued the statement
as a verifiable credential @vcdm2, the lender could check that the bank signed it, but the
credential would still disclose everything in it.

A zero-knowledge proof lets the applicant, as holder, return only the answer, with a proof that it
is correct. The lender writes its question as a SPARQL query @sparql11, our running example:

```sparql
PREFIX ex: <https://bank.example/vocab#>
ASK { ?payment a ex:Payment ; ex:paymentStatus ex:Returned . }
```

zkRDF @braun26 proves that each returned solution follows from signed data, disclosing only the
terms that the solution needs. An answer of `false`, however, has no solution to show, and zkRDF
cannot prove absence (§#ref(<bg-zkrdf>, supplement: none)).

To prove `false`, the holder must prove that the result is complete over some input. Because the
holder chooses which credentials to include, `false` means only "no returned payment in the
credentials I included", unless the verifier fixed the input in advance. No stronger proof removes
this limit; the request can only state it, so that the verifier knows what an accepted answer
guarantees.

The proof must also show that the queried data is the data the issuers signed. Checking a signature
inside the proof repeats, for every credential, the hashing and signature verification that the
verifier would otherwise run itself, and we expect this work to dominate the cost of proving
(§#ref(<planned>, supplement: none)). The holder can instead reveal the signatures for the verifier
to check. This removes their verification from the proof, but lets the verifier link presentations
of the same credential and, for the standard RDF cryptosuites, confirm a guess about its content.
The request therefore lists the signature modes the verifier accepts, trading proving cost against
disclosure.

Our contributions are:

- *Answer kinds and input kinds* (§#ref(<meaning>, supplement: none)): Supported answers, for
  monotone queries only, and Exact answers, over an input that the holder declared or the verifier
  agreed in advance.
- *An architecture* (§#ref(<architecture>, supplement: none)) in which one proof shows the answer to
  the verifier's query request over exactly the signed data, with the signatures hidden or revealed.
  A third signature mode lets the verifier evaluate the query over disclosed credentials.
- *A public-input rule* (§#ref(<minimize>, supplement: none)): a proof method may make public any
  value that the verifier can compute from its request and the result alone.
- *Merkle-based cryptosuites designed for proving* (§#ref(<cryptosuites>, supplement: none)), and a
  security and disclosure comparison of signature modes and cryptosuites.
- *An evaluation* (§#ref(<evidence>, supplement: none)) of a SPARQL evaluator running in the RISC
  Zero zkVM @risc0, with measurements planned across signature modes, cryptosuites and query types.

Our prototype has produced verified proofs of Exact answers that check issuer signatures inside the
proof, including the payment question under both input kinds. Each covers one synthetic credential,
and the cost measurements are pending.

== Background <background>

=== RDF datasets and SPARQL results <bg-sparql>

An RDF dataset has one default graph and zero or more named graphs @sparql11. SPARQL has four query
forms: `SELECT` returns a sequence of solution mappings (rows), `ASK` a boolean, and `CONSTRUCT` and
`DESCRIBE` an RDF graph. We write $[| Q |]_D$ for a multiset of solution mappings that SPARQL 1.1
permits for the pattern of query $Q$ over dataset $D$, after its solution modifiers (for `ASK`,
before it becomes a boolean). Its algebra part is $"eval"(D(G), P)$ @sparql11, with $P$ the algebra
expression of the pattern and $G$ the default graph of $D$. Where SPARQL permits several results,
$[| Q |]_D$ is the one the holder evaluated. Pattern matching compares terms, whereas the `=`
operator compares values: `"1250.00"^^xsd:decimal` and `"1250.0"^^xsd:decimal` are different terms
with equal values @rdf11. RDF is usually read under an open-world assumption: an absent triple is
not thereby false.

=== Credentials and Data Integrity proofs <bg-vc>

A verifiable credential @vcdm2 is a set of claims that an issuer makes about a subject, secured so
that its authorship can be verified; a holder presents it to a verifier. Its Data Integrity proof
@vcdi is, in this paper, a signature with its proof configuration; a cryptosuite specifies how to
create and verify it. The cryptosuite `eddsa-rdfc-2022` @vcdieddsa canonicalises the credential with
RDF Dataset Canonicalization (RDFC-1.0) @rdfc10, then signs with Ed25519 the SHA-256 hashes of the
canonical proof configuration and of the canonical document. A valid signature shows only that
someone with the signing key signed these bytes. The verifier must still check that the issuer
authorised the key, through a verification relationship in the issuer's controlled identifier
document @vcdi. It may also check credential status (revocation or suspension), the validity period,
and holder binding: that the presenter is the subject or controls a key bound to the credential.
None of these checks makes the claims true. Selective-disclosure cryptosuites such as `bbs-2023`
@vcdibbs let the holder derive a proof that discloses only selected claims.

=== Zero-knowledge proofs and zkVMs <bg-zk>

A zero-knowledge proof convinces a verifier of a statement: that, for given public inputs, some
witness satisfies a relation @zkproof. Here the result is a public input and the credentials are
part of the witness. A proof system has knowledge soundness if an efficient extractor can obtain a
valid witness from any prover that convinces the verifier. It is zero-knowledge if a proof reveals
nothing about the witness beyond the statement. Circuit-based systems, such as Noir @noir with the
UltraHonk backend @barretenberg, implement a relation as constraints. A zero-knowledge virtual
machine (zkVM) such as RISC Zero @risc0 instead proves that a guest program, identified by its image
ID, ran on some input and wrote a public output, the journal. Its proof is a receipt: the journal,
image ID and exit status, with a seal attesting to them; development mode produces fake receipts. A
zkVM can therefore prove a general SPARQL evaluator without a circuit for each query, but it proves
only that the program ran over the bytes it received, not who signed them.

=== Selective disclosure and its limits <bg-zkrdf>

zkRDF @braun26, building on RDF-based semantics for selective disclosure @braunkaefer25, proves the
soundness of SPARQL results: every returned solution is a solution over signed data. It discloses
query constants and projected terms, hides the rest, and proves signatures, equalities and numeric
bounds with BBS and range proofs. Its authors state that it cannot prove non-existence, so it
supports neither `MINUS` nor a false `ASK`. An earlier interface for zero-knowledge SPARQL over
credentials made the same choice deliberately: it certified each returned result as following from
data signed under listed keys, and required no proof of an empty result.
// TODO(citation): the earlier results-only interface is the unpublished ISWC 2025 manuscript;
// decide whether to cite it anonymously or leave it uncited (language guide, open question 1).
Neither states a complete answer over an input that a named party fixed.

== What an accepted answer means <meaning>

Suppose the bank issues the applicant a credential that lists payments, each with the type
`ex:Payment`, an amount, a date and a status. The input dataset $D$ is the RDF merge @rdf11mt of the
graphs of the credentials the holder includes, as the default graph; the merge keeps each
credential's blank nodes apart. A proof method may instead put each credential in its own named
graph. It states which layout it uses, because a query written for one does not match the other.

=== Supported and Exact answers <semantics>

Let $Q$ be the request's query. A _Supported_ answer states that every returned solution is a
solution. For `SELECT`, the result is non-empty and has no duplicate rows, and each row is in
$[| Q' |]_D$. Here $Q'$ is $Q$ without its top-level `ORDER BY`, `OFFSET` and `LIMIT`, which only
order the solutions or limit how many the holder returns. For `ASK`, the result is `true`. A
Supported answer says nothing about the solutions it does not return.

An _Exact_ answer is a result SPARQL 1.1 permits for $Q$ over $D$, with nothing added or omitted. A
`SELECT` result keeps its duplicates, an `ASK` result is `true` exactly when $[| Q |]_D$ is
non-empty, and a `CONSTRUCT` graph is compared up to isomorphism. Where SPARQL leaves a choice open,
the holder makes it. Such choices include `OFFSET` and `LIMIT` where `ORDER BY` does not fix the
order, the order of tied rows, `REDUCED`, `SAMPLE` and `GROUP_CONCAT`; a result that depends on them
is not uniquely determined by SPARQL. Without `ORDER BY`, the order of rows carries no meaning, so a
`SELECT` result is compared as a multiset. A proof method may fix these choices, and a verifier can
rely on that only where the method publishes them. Our evaluator does not yet publish its choices,
so a verifier can rely on a unique result only for queries that leave SPARQL no choice.
// Source: ZK code landing addenda (10 October), item 3; a follow-up change documents the choices.
Neither answer kind says who signed $D$ (§#ref(<linkage>, supplement: none)).

For the payment question, a Supported answer can only be `true`, shown by one returned payment. The
answer `false` must be Exact.

We allow Supported answers only for monotone queries. Apart from a top-level `ORDER BY`, `OFFSET`
and `LIMIT`, these are `SELECT` or `ASK` queries that use only basic graph patterns, group graph
patterns, `UNION`, `GRAPH`, `VALUES`, projection, `DISTINCT`, and `FILTER` or `BIND` without
`EXISTS` or `NOT EXISTS`. The holder may leave credentials out of $D$, and a solution of a monotone
query over $D$ is also a solution over any dataset that contains $D$. A Supported row therefore
stays a solution when the omitted credentials are added. With `OPTIONAL`, `MINUS`, `NOT EXISTS`,
subqueries or aggregates, one more credential can remove a row, change a count or displace the
latest payment.

=== Counting returned rows is not a count <counting>

A tempting shortcut lets the holder return Supported rows and the verifier compute `COUNT`,
`ORDER BY` or `LIMIT` over them. This works only if the returned rows are the complete result;
otherwise the verifier counts what the holder chose to show. A count over a subset of the
credentials is that subset's exact count, never a Supported count of the whole.

=== Who fixed the input <input-kinds>

An Exact answer is complete over $D$, and someone chose $D$. With a _holder-declared_ input, the
holder chose which credentials make up $D$ when it answered. For a holder-declared input, an Exact
answer covers only the included credentials. If the holder reuses the dataset commitment across
queries, the verifier can at least tell that every answer used the same input dataset, at the cost
of linking those presentations.

With a _verifier-agreed_ input, the verifier accepted a dataset commitment before sending the
request, and the proof's commitment must equal it; the Exact answer is then complete over the
dataset the verifier agreed to. Verifier agreement shows that two commitments are equal, not that
$D$ is complete in the world; that depends on why the verifier accepted the commitment
(§#ref(<anchor-assumption>, supplement: none)). Accepting whatever commitment the holder sends in
the same exchange gains nothing over a holder-declared input.

The input kind is separate from issuer checking: with either kind, if the request lists issuer keys,
the proof shows that every credential in $D$ carries a signature under one of them. Supported
answers keep the open-world reading of RDF, and an Exact answer shows absence only within $D$.

== Architecture <architecture>

@fig-architecture shows the three parties. The verifier service sends a query request and stores its
own copy. The holder service builds the input dataset from some of the holder's credentials,
evaluates the query, and returns an answer presentation: the result with one proof. A companion
specification defines both objects in JSON; an answer presentation is not a W3C verifiable
presentation.
// TODO(citation): the companion specification "Zero-Knowledge SPARQL Answers" (sparq-org/sparq#6786);
// cite it anonymously or as supplementary material under double-blind review.

#[
#show figure: set block(breakable: false)
// The paged grid is ignored by Typst's HTML export, so HTML gets an equivalent term list.
#let arch-paged = grid(
    columns: (1fr, auto, 1.7fr, auto, 1.2fr),
    gutter: 5pt,
    align: horizon,
    rect(width: 100%, inset: 6pt)[
      #set align(left)
      #set par(justify: false)
      #set text(size: 0.85em)
      *Issuers* \
      Sign credentials, for example with `eddsa-rdfc-2022`. May vouch for a dataset commitment
      that a verifier agrees in advance.
    ],
    [→],
    rect(width: 100%, inset: 6pt)[
      #set align(left)
      #set par(justify: false)
      #set text(size: 0.85em)
      *Holder* \
      Receives the query request.
      #rect(width: 100%, inset: 5pt, stroke: (thickness: 0.6pt, dash: "dashed"))[
        *Inside the proof* \
        1. In the hidden mode, check each signature against the request's issuer keys.
        2. Build $D$ from exactly the signed data.
        3. Evaluate $Q$ over $D$ within the request's limits.
        4. Output the result, the dataset commitment and the request digest.
      ]
    ],
    [→],
    rect(width: 100%, inset: 6pt)[
      #set align(left)
      #set par(justify: false)
      #set text(size: 0.85em)
      *Verifier* \
      *Outside the proof:* check the presentation against the stored request, the proof with its
      own verification key and, in the revealed mode, each signature; then mark the request as
      answered; then accept the result.
    ],
  )
#let arch-html = [
    #set align(left)
    / Issuers: Sign credentials, for example with `eddsa-rdfc-2022`. May vouch for a dataset
      commitment that a verifier agrees in advance.
    / Holder: Receives the query request. Inside the proof: (1) in the hidden mode, check each
      signature against the request's issuer keys; (2) build $D$ from exactly the signed data;
      (3) evaluate $Q$ over $D$ within the request's limits; (4) output the result, the dataset
      commitment and the request digest.
    / Verifier: Outside the proof: check the presentation against the stored request, the proof
      with its own verification key and, in the revealed mode, each signature; then mark the
      request as answered; then accept the result.
  ]
#figure(
  context if target() == "html" { arch-html } else { arch-paged },
  caption: [
    One proof per answer presentation. The query request travels from verifier to holder, and
    the presentation travels back. Dataset construction, evaluation and, in the hidden mode, the
    signature checks are inside the proof. The verifier's checks and marking the request as
    answered are outside it.
  ],
) <fig-architecture>
]

=== The query request <request>

@request-table lists the members of a query request. The verifier service stores the request and its
request digest, the SHA-256 hash of its JSON canonical form. It checks each presentation against
this stored copy, never against anything the holder returns. The request lists the accepted issuer
keys rather than resolving them, so that both sides prove and verify against the same keys.

A proof method is a way of producing and checking the proof, named by an IRI and a version, as a
cryptosuite is for Data Integrity proofs. Its evidence may be a zero-knowledge proof, a proof that
is not zero-knowledge, an attestation from a trusted execution environment (TEE), or disclosed
credentials. The verifier checks the evidence with a verification key from its own configuration,
such as a circuit's verification key or a zkVM image ID.

#[
#show figure: set block(breakable: false)
#figure(
  table(
    columns: (auto, 1fr),
    align: (left, left),
    table.header[Member][Meaning],
    [`query`], [The query $Q$: a SPARQL 1.1 `SELECT`, `ASK` or `CONSTRUCT` query, without `FROM`,
      `FROM NAMED`, `SERVICE` or functions such as `NOW` and `RAND`],
    [`answerKind`], [Supported or Exact (§#ref(<semantics>, supplement: none))],
    [`input`], [Holder-declared, or verifier-agreed with the agreed dataset commitment
      (§#ref(<input-kinds>, supplement: none))],
    [`issuers`], [The accepted issuer keys, each with its issuer, verification method and
      cryptosuite; an empty list accepts credentials without checking any signature],
    [`signatureModes`], [The accepted signature modes: hidden, revealed or disclosed
      (§#ref(<modes>, supplement: none))],
    [`proofMethods`], [The accepted proof methods, each with its version, verification key and
      parameters],
    [`limits`], [The largest presentation, in bytes, and the most result rows or triples
      (`maxResultRows`)],
    [`challenge`, `audience`], [Fresh random bytes, and the verifier's identifier],
    [`notBefore`, `notAfter`], [The period in which the verifier accepts a presentation],
  ),
  caption: [
    Members of a query request, apart from its type, version and optional base IRI. The request
    digest covers all of them.
  ],
) <request-table>
]

=== What the proof shows <linkage>

The public inputs of the statement are the request digest, the answer kind, the input kind, the
dataset commitment, the result, the signature mode and, in the revealed mode, the signed messages. A
proof method may bind them all by one digest. The proof shows that:

+ the dataset commitment fixes $D$;
+ if the request lists issuer keys, $D$ is built from exactly the data that signatures under those
  keys cover, and in the hidden mode the proof also verifies those signatures
  (§#ref(<modes>, supplement: none));
+ the result is an answer of the requested kind for $Q$ over $D$.

Because the proof binds the request digest, a proof made for one request does not verify for
another, or for another verifier or validity period, as long as SHA-256 is collision-resistant.

The second point is easily lost. For an RDFC-1.0 cryptosuite, the signed message is computed from
the credential's canonical N-Quads, so the input dataset can be built from those same quads, keeping
their signed lexical forms and each credential's blank nodes apart. For a credential signed as JSON,
building RDF needs JSON-LD processing with fixed contexts, which must then be part of the proof or a
stated assumption.

=== Signature modes <modes>

The request lists the signature modes the verifier accepts, and each proof method states which modes
it supports for each cryptosuite.

- In the _hidden_ mode, the proof shows that the holder knows a valid signature from a listed key on
  every credential in $D$. The signatures, the signed messages and which key signed which credential
  stay hidden.
- In the _revealed_ mode, the presentation carries each credential's signature and signed message,
  which the verifier checks itself. The proof then shows only that $D$ is exactly the data those
  messages cover and that the result is correct. Removing signature verification from the proof
  should make it cheaper, but this mode discloses the signatures, the signed messages, the issuer
  keys used and the number of credentials (§#ref(<security>, supplement: none)).
- In the _disclosed_ mode, for proof methods whose evidence is disclosed credentials, the holder
  sends the credentials or presentations derived from them by a selective-disclosure cryptosuite.
  The verifier checks them, evaluates $Q$ itself and sees everything disclosed.

=== Verifier processing <validation>

The verifier service processes a presentation in this order, and rejects it at the first check that
fails:

+ Before parsing, reject a presentation larger than the service's size ceiling.
+ Find the stored request with the presentation's request digest. Reject if there is none, if the
  time is outside its validity period, if it is already answered, or if the presentation exceeds its
  size limit.
+ Reject unless the presentation names a listed proof method whose verification key, loaded from the
  verifier's own configuration, equals the one in the stored request.
+ Reject unless the answer kind, the input kind and, for a verifier-agreed input, the commitment
  equal the request's.
+ If the request lists signature modes, reject any other mode. In the revealed mode, verify each
  signature under a listed issuer key.
+ Reject a result of the wrong shape or over the limits, such as a Supported `ASK` of `false`.
+ Verify the proof against the statement recomputed from the stored request and the presentation.
+ Mark the request as answered, in one atomic step that fails if it is already marked. Only then
  accept.

Checking everything first stops a malformed presentation from using up a legitimate request, and the
atomic step makes a replay, or the second of two concurrent presentations, fail. Sending a
presentation discloses its answer, even if the verifier then rejects it.

== Revealing less <minimize>

A zero-knowledge proof hides its witness, and in a circuit each hidden term needs constraints. Yet
some witness values are known to the verifier as soon as it has the result. A proof method may make
public any value that the verifier can compute from its stored request and the result alone; no
request member permits disclosing more. This public-input rule extends zkRDF's disclosure of query
constants and projected terms @braun26.

=== The public-input rule <rule>

Suppose the lender asks `SELECT ?p WHERE { ?p a ex:Payment . ?p ex:paymentStatus ex:Settled . }` and
receives one row, in which `?p` is `ex:pay1`. Substituting the row into the two triple patterns
gives `ex:pay1 a ex:Payment` and `ex:pay1 ex:paymentStatus ex:Settled`. The verifier can compute
both triples, so a proof method may make them public inputs, which the verifier computes rather than
receives. The proof must still show that each triple supplied as a public input is in $D$ and
signed: making a triple public removes its secrecy, not the need to authenticate it. Unless the
cryptosuite supports selective disclosure, the signature check still covers the whole credential,
hidden terms included.

=== Why the rule needs care <counterexamples>

The example query is a single basic graph pattern. In other queries, a returned row may not
determine which triples matched:

- *`OPTIONAL`.* In `?p a ex:Payment OPTIONAL { ?p ex:returnReason ?r }`, a row with `?r` unbound
  shows that no return-reason triple for `?p` matched. No triple stands for that absence; only an
  Exact answer can show it.
- *`UNION`.* In `{ ?p ex:paymentStatus ex:Returned } UNION { ?p ex:paymentStatus ex:Reversed }`, a
  row does not show which branch matched. The matched branch is not computable, so its triple does
  not qualify.
- *Blank nodes.* A blank node in the query acts as a hidden variable. A blank node label in a result
  is scoped to that result, so substituting it gives no triple of $D$.
- *Value equality.* `FILTER(?a = 1250)` holds for both `"1250.00"^^xsd:decimal` and
  `"1250.0"^^xsd:decimal`, so a filter does not fix the term of an unprojected variable.
- *Omission.* Triples of the returned rows supplied as public inputs say nothing about rows not
  returned; they never make an answer Exact.

=== When the values may be disclosed <release>

Being computable from the result does not make a value safe to disclose before the result. In the
single-message flow of §#ref(<validation>, supplement: none), the public inputs travel with the
answer from which they are computed. A method with several rounds could send them earlier, and an
exchange that then aborted would have disclosed facts that no answer did. We propose that a party
receive these public inputs no earlier than the answer from which it can compute them. The prototype
does not enforce this condition.

=== What the verifier still learns <leakage>

Beyond its result, an accepted answer reveals:

- the dataset commitment, which links presentations if the holder reuses a salt over the same
  credentials; a verifier-agreed commitment links them by design, to the verifier that agreed it;
- the proof method the holder chose, whose capacity limits reveal an upper limit on the size of the
  input;
- the issuers: in the hidden mode, only that a listed key signed each credential, which identifies
  the issuer if only one is listed; in the revealed mode, each key and the number of credentials;
- the size of the result, such as how many payments matched;
- from a RISC Zero receipt, part of the shape of the execution. The zero-knowledge of these receipts
  is RISC Zero's claim, not an established property;
- for an Exact answer, the holder's open choices (§#ref(<semantics>, supplement: none)). For
  `SELECT ?x WHERE { VALUES ?x { 0 1 } } LIMIT 1`, the returned value can encode one bit of the
  holder's choosing. Zero-knowledge does not prevent this covert channel, because the result is
  public. It matters when the holder does not fully trust its holder service.

A sequence of queries can also reveal together what no single query reveals.

== Cryptosuites and signature modes <cryptosuites>

=== Existing cryptosuites <existing-suites>

Three RDF cryptosuites sign the same structure: hashes of the canonical proof configuration and of
the canonical document. `eddsa-rdfc-2022` uses Ed25519 with SHA-256 @vcdieddsa; our zkVM guest
verifies it, and every zkVM experiment with signature checks in §#ref(<evidence>, supplement: none)
uses it. `ecdsa-rdfc-2019` uses ECDSA, with P-256 and SHA-256 or P-384 and SHA-384 @vcdiecdsa; we
verify it only outside the zkVM.
// TODO(evidence): bind to the implementation state reported by the ZK code landing thread
// (zksparql-evaluation-plan.md §8: ecdsa-rdfc-2019 verified natively only).
`mldsa44-rdfc-2024` uses ML-DSA-44 @fips204 with SHA-256 and is designed to resist forgery by a
quantum adversary; it is in a W3C First Public Working Draft @vcdiqr, and we have not implemented
it. None of the three salts its signed message, so whoever sees the hashes can test a guessed
document. To check such a signature inside a proof, the guest hashes each credential's whole
canonical document, and to compare typed values it must parse their lexical forms. The
selective-disclosure cryptosuites `bbs-2023` @vcdibbs and `ecdsa-sd-2023` @vcdiecdsa fit the
disclosed mode; only `bbs-2023` derived proofs are unlinkable.

=== Our Merkle-based cryptosuites <merkle-suites>

We propose Merkle-based cryptosuites designed for proving.
// TODO(citation): the Merkle-based cryptosuite draft (sparq-org/sparq#6789); cite it anonymously
// or as supplementary material under double-blind review.
The issuer builds a Merkle tree with one leaf per canonical quad of the credential. It signs a
digest of the suite identifier, a fresh 32-byte salt, the number of quads, the Merkle root and the
digest of the canonical proof configuration. Each leaf encodes the typed terms of its quad, and a
literal's encoding carries a comparison key: an order-preserving encoding of its value, for numbers,
date-times, booleans and strings.

This design has three effects. First, a proof need not repeat canonicalisation: it rebuilds the
signed tree from the credential's quads. Second, a proof compares values without parsing lexical
forms. In a circuit, comparing two integers or decimals that the key can represent takes one range
check on the difference of their keys. By a preliminary gate count of our current Noir circuits for
integer filters, the range checks on literals committed as hashed text account for most of the
gates.
// TODO(evidence): bind to the UltraHonk gate breakdown (project file
// zk-proof-methods/ultrahonk-gate-breakdown.md) once it is frozen as an evidence record.
Third, a fresh, random and secret salt makes the signed message computationally hiding, if the hash
is modelled as a random oracle, so that a verifier in the revealed mode cannot confirm a guessed
credential.

The family has three members. `eddsa-sha256-merkle-2026` uses a SHA-256 tree and Ed25519, for zkVMs;
our zkVM guest can verify it, but no experiment in §#ref(<evidence>, supplement: none) uses it yet.
// TODO(evidence): bind to the Merkle-suite implementation (sparq-org/sparq#6789) once it is
// merged and frozen as an evidence record.
`schnorr-poseidon2-merkle-2026` uses a Poseidon2 tree and Schnorr signatures over Baby Jubjub, for
circuits, and the post-quantum `mldsa44-sha256-merkle-2026` uses ML-DSA-44. These two are specified
but not implemented.

=== Security and disclosure <security>

@security-table compares signature modes, signature schemes and proof systems. In the hidden mode,
the cryptosuite does not change what the verifier learns, which depends on the public inputs and on
the zero-knowledge of the proof system. In the revealed mode, a credential's repeated signature
links its presentations, and with an RDFC cryptosuite the verifier can also confirm a guessed
credential. Whether a quantum adversary can forge a signature depends on the cryptosuite, not on the
mode. Outside the disclosed mode, the proof system's knowledge soundness must also hold. A quantum
adversary could forge UltraHonk proofs, so even proofs made earlier would cease to be evidence. RISC
Zero's succinct receipts are STARK-based, and we infer that Shor's algorithm does not break their
knowledge soundness. Our zkVM proof method accepts only succinct receipts, not the Groth16 receipts
that RISC Zero offers for cheaper verification, whose knowledge soundness holds only against a
classical adversary.

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
        [Only through a commitment repeated by reusing a salt, or a verifier-agreed commitment], [No],
      [Revealed (`eddsa-rdfc-2022`, `ecdsa-rdfc-2019`, `mldsa44-rdfc-2024`)], [Per credential:
        signature, signed message and verification method; so the issuer keys and the number of
        credentials], [Yes], [Yes: the document hash is unsalted],
      [Revealed (Merkle-based)], [As above, without the salt], [Yes],
        [No, while the salt is fresh and private],
      [Disclosed (whole credentials)], [The signed credentials], [Yes], [Not applicable],
      [Disclosed (`ecdsa-sd-2023`)], [Disclosed and mandatory statements, the base signature],
        [Yes], [No],
      [Disclosed (`bbs-2023`)], [Disclosed and mandatory statements, a BBS proof],
        [Only through what is disclosed], [No],
      table.cell(fill: luma(235))[*Component (used by)*], table.cell(fill: luma(235))[*Assumption*],
        table.cell(fill: luma(235))[*Classical adversary*], table.cell(fill: luma(235))[*Quantum adversary*],
      [Ed25519 (`eddsa-rdfc-2022`, `eddsa-sha256-merkle-2026`); ECDSA P-256 (`ecdsa-rdfc-2019`,
        `ecdsa-sd-2023`)], [Elliptic-curve discrete logarithm], [Unforgeable], [Forgeable],
      [ML-DSA-44 (`mldsa44-rdfc-2024`, `mldsa44-sha256-merkle-2026`)],
        [Module lattice problems (MLWE, SelfTargetMSIS)], [Unforgeable], [Believed unforgeable],
      [Schnorr over Baby Jubjub (`schnorr-poseidon2-merkle-2026`)],
        [Discrete logarithm; Poseidon2 as a random oracle],
        [Unforgeable: argued, inferred for this instantiation], [Forgeable],
      [BBS (`bbs-2023`)], [A pairing assumption (q-SDH)], [Unforgeable], [Forgeable],
      [SHA-256 and Poseidon2 Merkle trees], [Collision resistance], [Binding],
        [Binding, with a reduced margin],
      [RISC Zero succinct receipt (STARK)], [Random oracle model and a conjecture],
        [Knowledge-sound, under the conjecture], [Inferred: Shor's algorithm does not apply],
      [UltraHonk with zero-knowledge (KZG over BN254)],
        [Pairings over BN254 (q-SDH, algebraic group model); random oracle; trusted setup],
        [Knowledge-sound], [Not knowledge-sound, also for earlier proofs],
    )
  },
  caption: [
    Security and disclosure by signature mode, cryptosuite and proof system. "Links
    presentations" means through a value the verifier receives, without breaking any assumption.
    RISC Zero claims, but has not established, that its receipts are zero-knowledge. The
    zero-knowledge variant of UltraHonk that we use is designed to be statistically
    zero-knowledge; its documentation cites no proof. In every mode, the result and its size can
    also reveal data and link presentations.
  ],
) <security-table>
]
// Sources: scratchpad security-table.md (Tables A to C; every cell used here is sourced there)
// and security-table-addenda.md (UltraHonk as run; quantum binding of the trees; succinct-only
// receipts; RISC Zero zero-knowledge claim; Schnorr over Baby Jubjub argument).

== Evaluation <evidence>

=== Prototype and method <prototype>

The SPARQL evaluator running in the RISC Zero zkVM (our guest program, or "the evaluator") evaluates
a query over its input and writes an Exact result, the dataset commitment and the input kind to the
journal. A second build adds signature checks in front of the same evaluator: it verifies each
credential's `eddsa-rdfc-2022` signature in the hidden mode, checks the key against the request's
issuer keys, and evaluates the query over the signed canonical N-Quads. Our holder and verifier
services wrap both builds in query requests and the checks of §#ref(<validation>, supplement: none),
with a binary encoding rather than the specification's JSON. The challenge, audience and validity
period reach the proof indirectly: the journal binds a value derived from the stored request. We
have not confirmed that the verifier service performs its checks in exactly the order of
§#ref(<validation>, supplement: none). Separately, two Noir circuits produce Supported answers to
`SELECT DISTINCT` queries over one basic graph pattern: in the baseline circuit the matched triple
is part of the witness, and in the public-triple circuit a public input. Both check Schnorr
signatures over their own commitment format, not Data Integrity proofs, and check credential status
against a snapshot that the verifier accepted. They publish no dataset commitment, so they cannot
yet produce the specification's presentations.

The evaluator admits most SPARQL 1.1 query features, including `OPTIONAL`, `MINUS`, aggregates,
subqueries and property paths, and rejects `SERVICE`, `NOW`, `RAND`, `DESCRIBE`, `FROM` and
`FROM NAMED`.
// TODO(evidence): bind to the coverage manifest of the evaluator with signature checks
// (zk/sparql-evaluator/coverage-authenticated-rdf.json, draft sparq-org/sparq#6791) and the host
// query profile, once frozen as evidence records.

We used only synthetic credentials signed with test keys. We generated all receipts with development
mode disabled and verified them against the expected guest image ID. For experiments without † in
@evidence-table, we recomputed hashes of source files, guest binaries and receipts. We compared the
hashes and recorded test outcomes with the archived records. We did not verify the proofs again.
Appendix #ref(<app-inventories>, supplement: none) details the records.

#[
#show figure: set block(breakable: false)
#figure(
  {
    set text(size: 0.8em)
    set par(justify: false)
    table(
      columns: (1.3fr, 1.6fr, 0.75fr, 1.6fr),
      align: (left, left, left, left),
      table.header[Experiment][Answers][Signatures checked in the proof][Strongest result],
      [Evaluator, CI build], [Exact `SELECT` with duplicate rows, false `ASK`, `CONSTRUCT`,
        `DESCRIBE` (now rejected); both input kinds, not every form under each], [No],
        [Proof verified: #headline("zkvcq.exact_hosted_receipts") receipts],
      [Evaluator with our services], [Exact `SELECT`, true and false `ASK`, `CONSTRUCT`; both
        input kinds], [No], [Proof verified: #headline("zkvcq.adapter_receipts") receipts,
        #headline("zkvcq.adapter_row_bound_rejected") rejected at the row limit;
        #headline("zkvcq.adapter_controls") negative tests],
      [Evaluator with signature checks, executed directly], [Exact answers over the W3C test
        vector; both input kinds], [Yes], [Executed without proving:
        #headline("zkvcq.v5g_direct_v5_positive") inputs accepted,
        #headline("zkvcq.v5g_direct_v5_aborts") altered inputs rejected],
      [Same, with our services: first test case], [Exact `SELECT` with duplicate rows;
        verifier-agreed], [Yes], [Proof verified: #headline("zkvcq.vcqg_genuine_receipts")
        receipt; #headline("zkvcq.vcqg_controls") negative tests],
      [Same: payment question †], [Exact false `ASK`; both input kinds], [Yes], [Proof verified:
        one receipt per input kind; #headline("zkvcq.vcqp_controls") (verifier-agreed) and
        #headline("zkvcq.vcqph_controls") (holder-declared) negative tests],
      [Same: remaining test cases †], [Exact `SELECT`, true and false `ASK`, `CONSTRUCT`; both
        input kinds; row limit], [Yes], [Proof verified: one receipt per test case; each accepted one
        with #headline("zkvcq.ci_asktva_controls") (verifier-agreed) or
        #headline("zkvcq.ci_askfhd_controls") (holder-declared) negative tests; the row-limit case
        rejected],
      [Public-triple circuit pilot], [Supported `SELECT DISTINCT`, one basic graph pattern],
        [Schnorr, own format], [Proof verified: #headline("zkvcq.pp_genuine_proofs") proofs;
        #headline("zkvcq.pp_tamper_controls") tampering and
        #headline("zkvcq.pp_replay_controls") replay tests rejected],
    )
  },
  caption: [
    Strongest result per experiment: tests outside the zkVM, executed without proving, or proof
    verified. Every zkVM experiment with signature checks used one credential per test case.
    Holder-declared cases have two fewer negative tests, because swapping the agreed commitment
    does not apply to them. † No internal check (§#ref(<prototype>, supplement: none)). Counts
    are per experiment and are never added across rows.
  ],
) <evidence-table>
]

=== Exact answers without signature checks <exact-evidence>

A CI job built the evaluator from source and produced #headline("zkvcq.exact_hosted_receipts")
receipts. They cover a `SELECT` with duplicate rows, a false `ASK`, `CONSTRUCT` and `DESCRIBE`,
under both input kinds, though not every form under each (Appendix
#ref(<exact-detail>, supplement: none)). Some used `DESCRIBE` or `FROM NAMED`, which version 1 of
the specification excludes and the current evaluator rejects before proving. Their `DESCRIBE` answer
contained every default-graph triple whose subject was the described resource and, recursively,
every triple whose subject was a blank node reached as an object. The same job ran
#headline("zkvcq.exact_replay_cases") test cases outside the zkVM, in
#headline("zkvcq.exact_replay_jobs") runs across request formats and input kinds. Each run gave its
expected outcome, and none produced a proof.

With our holder and verifier services, on another machine and guest binary, the evaluator produced
#headline("zkvcq.adapter_receipts") receipts. The verifier accepted
#headline("zkvcq.adapter_accepted"), covering `SELECT`, true and false `ASK` and `CONSTRUCT` under
both input kinds. It rejected #headline("zkvcq.adapter_row_bound_rejected") receipt, whose result
exceeded the row limit: a valid proof is necessary, not sufficient, for acceptance. The
#headline("zkvcq.adapter_controls") negative tests exercised each verifier check (@controls-table).
These receipts show Exact evaluation bound to a request, not who issued the data.

=== Exact answers with signature checks <v5-evidence>

The build with signature checks passed #headline("zkvcq.v5_auth_tests_passed") tests outside the
zkVM, including the rejection of valid signatures under the wrong issuer, verification method, proof
purpose or cryptosuite. Executed without proving, its guest accepted
#headline("zkvcq.v5g_direct_v5_positive") valid inputs and rejected
#headline("zkvcq.v5g_direct_v5_aborts") altered ones, such as forged, spliced or unauthorised
credentials, each with its expected error.

With our holder and verifier services, the build then produced one receipt per test case, each over
a single credential, in three experiments:

- a verifier-agreed `SELECT` over the W3C test vector for `eddsa-rdfc-2022`, whose result keeps a
  duplicated row (#raw("?" + headline("zkvcq.vcqg_result_variable")) is
  #raw(headline("zkvcq.vcqg_result_row1")) twice);
- the payment question of §#ref(<intro>, supplement: none), over a synthetic credential that lists
  three settled payments and is signed with the public RFC 8032 test key, so its issuer stands in
  for a bank. Under both input kinds the journal reports `false`, and the verifier accepted the
  answer;
- the remaining test cases, over the W3C test vector: `SELECT`, false `ASK` and `CONSTRUCT` under a
  holder-declared input; true `ASK` and `CONSTRUCT` under a verifier-agreed input; and a `SELECT`
  whose result exceeds the row limit. The verifier accepted every answer except the last, which it
  rejected before marking the request as answered.

Each accepted receipt came with negative tests (@evidence-table). They alter the request, the issuer
keys, the proof method, the receipt, the input kind or the agreed commitment; present the receipt
under another guest's image ID; or replay it. Each gave its expected outcome; we report no timings
for these runs.

=== The public-triple circuit pilot <pilot-evidence>

We produced #headline("zkvcq.pp_genuine_proofs") UltraHonk proofs, alternating the baseline and
public-triple circuits over the same query, rows, credentials and capacity. All
#headline("zkvcq.pp_tamper_controls") tampering and #headline("zkvcq.pp_replay_controls") replay
tests were rejected. Every proof was #headline("zkvcq.pp_proof_bytes") bytes with either circuit,
and both circuits made the same numbers of signature checks and membership checks, so the comparison
isolates the effect of making the triple public. Preliminary: #headline("zkvcq.pp_n_per_cell") timed
runs per circuit and capacity, on a shared machine (@pilot-table). Prove and verify times differed
little relative to their totals. The runs are too few to show a material or general saving, and we
did not measure memory or larger inputs.

=== Planned measurements <planned>

We plan to compare the hidden and revealed modes with `eddsa-rdfc-2022` and
`eddsa-sha256-merkle-2026`, at one and four credentials. The planned measurements use five queries
over the payment credential: the payment `ASK`, a `SELECT` of amounts with duplicate rows, a
`CONSTRUCT` over the same pattern, a numeric `FILTER` on `xsd:decimal` amounts, and a string
`FILTER` on payment IRIs. We will report cycles, which the RISC Zero executor counts
deterministically without proving, separately from proving times, measured with development mode
disabled on one dedicated machine. First executor runs for the payment `ASK` over one credential,
not yet archived as evidence, show in-guest signature verification as the largest phase in the
hidden mode with `eddsa-rdfc-2022`.
// TODO(evidence): bind to the first executor measurements (project files zk/paper-measurements/)
// once frozen under research/zk-paper-evidence/ and bound in paper-evidence.json.

#todo-results[Cycles, proving time and receipt size for each signature mode, cryptosuite and
query, at both input sizes.]

#todo-results[Cycles per guest phase (signature verification, canonicalisation and hashing or
Merkle path checks, dataset construction, evaluation and output), testing whether the signature
check dominates the cost of proving.]

#todo-results[Issuer, holder and verifier costs: signing and tree construction; proving time,
memory and presentation size; and verification time, split into receipt, signature and request
checks.]

#todo-results[Cycles by query type, one query per feature class, at increasing input sizes,
with the full sweep in the appendix.]

#todo-results[A comparison, over the same queries, with re-evaluation over disclosed
credentials, selective disclosure with re-evaluation and, where built, designated-verifier proofs
and TEE attestation.]

== Related work <related>

_Selective disclosure over RDF._ Braun and Käfer define RDF-based semantics for selective disclosure
and zero-knowledge proofs on credentials @braunkaefer25, on which zkRDF builds
(§#ref(<bg-zkrdf>, supplement: none)), and Yamamoto, Suga and Sako formalise linked-data credentials
for selective disclosure @yamamoto22. We add Exact answers, input kinds and signature modes, and
examine what disclosure saves when the signature does not support selective disclosure.

_Verifiable query processing._ Query authentication distinguishes correctness, meaning that every
returned record is in the owner's database unmodified, from completeness, meaning that no answer is
omitted @li06. In these terms a Supported answer is correct, and an Exact answer is correct and
complete over $D$. IntegriDB @integridb, vSQL @vsql and ZKSQL @zksql prove SQL results, ZKSQL in
zero knowledge with explicit leakage of schema and cardinalities; ZKGraph proves graph queries
@zkgraph25; VeriDKG verifies that SPARQL results over a decentralised knowledge graph are complete
@veridkg23. Their completeness is over a published database, the counterpart of our verifier-agreed
input, not over a holder's private credentials. Wright @wright25dc evaluated SPARQL over
Ed25519-signed credentials in RISC Zero; zkRDF later reduced the proving cost substantially with a
data-centric design under a different signature scheme @braun26. Our evaluator returns to a zkVM to
support non-monotone queries, and inherits the cost question.

_Credentials and presentation protocols._ CL signatures @cl01, BBS @bbs, zk-creds @zkcreds and
Crescent @crescent let a holder prove statements about signed attributes, and SD-JWT @sdjwt lets a
holder disclose selected claims of a signed JSON token. OpenID for Verifiable Presentations
@openid4vp requests credentials with DCQL queries, which select credentials and the claims to
disclose. A query request could travel in it as a new credential format, but the protocol has no
step in which a verifier agrees an input in advance.

_Other proof methods._ A proof method need not produce a zero-knowledge proof. A TEE could attest
that a measured program evaluated the query and checked the signatures. A designated-verifier proof
based on vector oblivious linear evaluation, such as QuickSilver @quicksilver21, would convince only
the verifier that took part. Both are proposed in the companion specification, not built as proof
methods. In a preliminary comparison on our Noir circuits, QuickSilver proved more slowly than
UltraHonk at every circuit size we tried, and its verifier had to stay online and receive far more
data. At these sizes, the comparison found no advantage for QuickSilver other than that its proof
cannot be transferred.
// TODO(evidence): bind to the QuickSilver re-run with large LPN parameters (project file
// zk-proof-methods/quicksilver-vs-ultrahonk.md, draft sparq-org/sparq#6790) once frozen as an
// evidence record. The earlier 'medium' run must not be cited.

== Discussion and limitations <discussion>

=== Where verifier-agreed commitments could come from <anchor-assumption>

Reading a verifier-agreed Exact answer as evidence about all the relevant records rests on an
assumption we do not discharge: that a party the verifier trusts fixed an input that covers them.
For the payment question, the bank could sign a dataset commitment that covers its statement
credentials for a period. Alternatively, the verifier could agree a commitment in an earlier
exchange in which it learned which credentials the dataset holds. We do not claim that such
infrastructure exists. Without it, a deployment can offer only holder-declared inputs.

=== Agents and several holders <agents>

Software agents may soon carry people's records between organisations. A person who vouches for her
own calendar can simply sign her answer. A proof is needed when a holder relays a statement it did
not make and must not reveal the rest, as when an applicant's agent relays a bank's records. The
receiving agent cannot pause to ask a person what an accepted answer guarantees; a query request
that states the answer kind, the input kind and the accepted issuer keys records that guarantee
where software can check it. We have not evaluated agents or agent protocols.

We consider a single holder. Queries over records held by several parties that do not trust one
another could combine multi-party computation with proofs; we leave them to separate work.

=== Limitations <limitations>

The SPARQL evaluator running in the zkVM does not check credential status, holder binding or
credential validity periods, and the query request has no member for them. The public-triple circuit
checks credential status against a snapshot whose freshness is left to deployment policy. The
prototype has not undergone an external security audit. Our arguments for what acceptance implies
(Appendix #ref(<app-relation>, supplement: none)) are informal and rest on named assumptions; tests
support some of these assumptions but do not establish them. Negative tests show that specific
checks exist and fire; they cannot rule out substitutions we did not try. The evaluator is covered
by tests, not by a conformance suite. Every zkVM proof with signature checks covers one synthetic
credential, so we have no evidence yet about realistic credentials, larger inputs or the cost of
Exact answers.

== Conclusion <conclusion>

A holder can answer a verifier's SPARQL query over credentials without handing them over, but an
accepted answer is useful only if the verifier knows what it guarantees. We distinguish Supported
answers, whose returned solutions are all solutions, from Exact answers, which are complete over an
input that the holder declared or the verifier agreed in advance. One proof can show the answer over
exactly the data the issuers signed, in a signature mode that trades proving cost against
disclosure. A public-input rule lets the proof make public what the verifier can compute from its
request and the result alone. A prototype zkVM evaluator has proved Exact answers with signature
checks, including an answer that no payment was returned, each over one synthetic credential.
Realistic credentials, the cost measurements and an external audit remain to be done.

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

/ $m$, method: query-proof method identifier, version and descriptor digest (§#ref(<request>, supplement: none)).
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
listed with its SHA-256 digest in `provenance.json`. Every snapshot except those of the payment
and CI authenticated runs (§#ref(<v5-evidence>, supplement: none)) summarises a second internal
evidence inspection of a completed run (source trees re-hashed against the stated commit,
artifacts and receipts re-hashed, recorded outcomes compared) rather than a new execution. The
payment and CI snapshots are the runs' own audit records and have had no such inspection. None is
an external security review. Every number in this paper is read from `paper-evidence.json`
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

*Genuine adapter receipt for the payment false `ASK`.* At source
#short-id("zkvcq.vcqp_source_commit"), a separate run executed the same driver for
#raw(headline("zkvcq.vcqp_case")) only (`all_defined_cases_run` =
#raw(repr(headline("zkvcq.vcqp_all_cases_run")))). The input is a synthetic payment-history
credential of three settled payments, signed with the RFC 8032 section 7.1 test key, whose secret
key is public. The run produced #headline("zkvcq.vcqp_genuine_receipts") genuine receipt
(#raw(headline("zkvcq.vcqp_receipt_inner")), #raw(headline("zkvcq.vcqp_exit_code")), `dev_mode` =
#raw(repr(headline("zkvcq.vcqp_dev_mode")))) with #raw(headline("zkvcq.vcqp_r0vm_version")), of
which #headline("zkvcq.vcqp_protocol_accepted") was accepted by the protocol verifier. Its journal
reports `ASK` = #raw(repr(headline("zkvcq.vcqp_result_ask"))) with provenance
#raw(headline("zkvcq.vcqp_provenance")). The guest was rebuilt for this run, so its pinned artifact
(#short-id("zkvcq.vcqp_guest_sha256")) differs from the bag `SELECT` run's
(#short-id("zkvcq.vcqg_guest_sha256")); the image-splice control presents the receipt under the
other pin. The #headline("zkvcq.vcqp_controls") controls are the same groups as above, adapted to
the payment fixture, and each behaved as the test asserts. The retained receipt is
#short-id("zkvcq.vcqp_receipt_sha256"). This run's record has not had a second internal evidence
inspection, and it carries the same authentication limits as the bag `SELECT` run. A second job
at the same source and pin proved #raw(headline("zkvcq.vcqph_case")) with
#headline("zkvcq.vcqph_genuine_receipts") genuine receipt, of which
#headline("zkvcq.vcqph_protocol_accepted") was accepted, reporting `ASK` =
#raw(repr(headline("zkvcq.vcqph_result_ask"))) with #headline("zkvcq.vcqph_controls") controls.

*Genuine adapter receipts for the remaining declared cases (CI).* At source
#short-id("zkvcq.ci_source_commit"), one hosted workflow run proved six declared cases in parallel
jobs over the synthetic W3C test vector, with #raw(headline("zkvcq.ci_r0vm_version")) and one V5
guest artifact (#short-id("zkvcq.ci_guest_sha256")) identical across jobs. The cases are
#raw(headline("zkvcq.ci_selhd_case")), #raw(headline("zkvcq.ci_askfhd_case")),
#raw(headline("zkvcq.ci_conhd_case")), #raw(headline("zkvcq.ci_asktva_case")),
#raw(headline("zkvcq.ci_conva_case")) and #raw(headline("zkvcq.ci_rowb_case")). Each job produced
one genuine receipt (#raw(headline("zkvcq.ci_asktva_exit_code")), `dev_mode` =
#raw(repr(headline("zkvcq.ci_asktva_dev_mode")))). The five answer cases were accepted by the
protocol verifier and ran the same control groups as above, without the two anchor controls in the
holder-declared cases. The row-bound case's receipt verifies under the V5 pin with test nonces, but
the protocol verifier refused it with #raw(headline("zkvcq.ci_rowb_rejection")) before consuming
the original challenge, so it is evidence of the rejection, not of an accepted answer. Wall times
are recorded but non-canonical, and runner size and peak memory were not captured. The three
authenticated runs therefore have three guest pins: #short-id("zkvcq.vcqg_guest_sha256") for the
bag `SELECT` run, #short-id("zkvcq.vcqp_guest_sha256") for the payment run and
#short-id("zkvcq.ci_guest_sha256") for the CI run. All are built from V5 source, and they differ
because the image identifier depends on the build path.

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

The preliminary pilot timings follow (moved from the main body; §#ref(<pilot-evidence>, supplement: none)).

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
EdDSA under a verifier-owned policy. It has passed native tests and has genuine receipts for all its declared
cases, each over a single synthetic credential. Its
registry entry still conservatively does not offer these tuples as available
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
