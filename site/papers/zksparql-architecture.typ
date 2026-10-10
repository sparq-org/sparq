// [OPUS-5.5] zkSPARQL architecture paper. The main body follows the language guide
// (site/papers/zksparql-language-guide.md) and the companion specification of zero-knowledge
// SPARQL answers (sparq-org/sparq#6786): query request and answer presentation, Supported and
// Exact answers, holder-declared and verifier-agreed inputs, the hidden, revealed and disclosed
// signature modes, the public-input rule and the verifier's processing order. The appendix (at
// most two LNCS pages) and the supplementary material after it follow the same guide.
//
// HONESTY FRAME (empirical-honesty mandate; external-audit gate sq-qhy4): the paper asserts no
// proven security, privacy or integrity property for any implementation. Every number comes from
// paper-evidence.json through #headline(...) (canonical records) or #ev(...) (the preliminary
// pilot timings, in the supplementary material); none is typed into prose, and counts from
// different experiments are never added. Experiments without the internal re-check carry a dagger
// in the evidence table. Facts not yet frozen as evidence records (guest coverage, cryptosuite
// implementation state, the gate breakdown) appear only qualitatively, each with a TODO(evidence)
// comment; the proof-method comparison with QuickSilver has no evidence record and is not reported,
// even qualitatively. Executor cycle counts are shown in millions through one
// display helper (mcycles). Pending measurements are visible todo-results placeholders.

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

// [OPUS-5.5] Executor cycle counts in millions, rounded to two decimals for display only (the
// value itself always comes from the evidence file through headline, so it must be canonical).
#let mcycles(key) = {
  let s = str(calc.round(headline(key) / 1000000, digits: 2))
  let parts = s.split(".")
  if parts.len() == 1 { s + ".00" } else { s + "0" * calc.max(0, 2 - parts.at(1).len()) }
}

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
  Working draft. The measurements marked as pending in §#ref(<cost>, supplement: none) are not
  yet available.
]]

#heading(level: 2, numbering: none, outlined: false)[Abstract]

A verifier often needs the answer to a question about a person's credentials, not the credentials
themselves. zkRDF and an earlier interface for zero-knowledge SPARQL over credentials prove that
each returned solution follows from signed data, but neither proves that something is absent, such
as a returned payment. We define Supported answers, in which every returned solution is a solution,
and Exact answers, the complete result over a committed input dataset. As an Exact answer is only as
complete as its input, the verifier's query request also states who fixed the input: the holder, or
the verifier in advance. When the request lists issuer keys, one proof in our architecture shows,
under stated assumptions, that the answer is correct over exactly the data signed under those keys.
Three signature modes trade proving cost against what the verifier learns, and a public-input rule lets a proof make public any value that the verifier can compute
from its request and the result alone. We also propose Merkle-based cryptosuites designed for
proving. A SPARQL evaluator running in a zero-knowledge virtual machine has produced verified proofs
of Exact answers with issuer signatures checked inside the proof, including an answer that no
payment was returned, each over one synthetic credential. Cycle counts taken without proving show
signature verification as the largest step inside the proof for the payment question, and fewer
cycles when the holder reveals the signatures or the issuer uses a Merkle-based cryptosuite.
Proving times are pending.

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
verifier would otherwise run itself. For the payment question over one credential, signature
verification was the largest step inside the proof in our cycle counts
(§#ref(<cost>, supplement: none)). The holder can instead reveal the signatures for the verifier to
check. This removes their verification from the proof, but lets the verifier link presentations
of the same credential and, for the standard RDF cryptosuites, confirm a guess about its content.
The request therefore lists the signature modes the verifier accepts, trading proving cost against
disclosure.

Our contributions are:

- *Answer kinds and input kinds* (§#ref(<meaning>, supplement: none)): Supported answers, for
  monotone queries only, and Exact answers, over an input that the holder declared or the verifier
  agreed in advance.
- *An architecture* (§#ref(<architecture>, supplement: none)) in which, when the request lists
  issuer keys, one proof shows the answer to the verifier's query request over exactly the data
  signed under those keys, under the assumptions of Appendix #ref(<app-relation>, supplement: none).
  The signatures are hidden in the proof or revealed; a third signature mode lets the verifier
  evaluate the query over disclosed credentials.
- *A public-input rule* (§#ref(<minimize>, supplement: none)): a proof method may make public any
  value that the verifier can compute from its request and the result alone.
- *Merkle-based cryptosuites designed for proving* (§#ref(<cryptosuites>, supplement: none)), and a
  security and disclosure comparison of signature modes and cryptosuites.
- *An evaluation* (§#ref(<evidence>, supplement: none)) of a SPARQL evaluator running in the RISC
  Zero zkVM @risc0, with cycle counts across signature modes, cryptosuites and query types; proving
  times are pending.

Our prototype has produced verified proofs of Exact answers that check issuer signatures inside the
proof, including the payment question under both input kinds. Each covers one synthetic credential.
Our cycle counts, taken without proving, cover one and four synthetic credentials.

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
  are part of the witness. They stay hidden as far as the proof system is zero-knowledge and the
  public inputs do not disclose them (§#ref(<leakage>, supplement: none)).
- In the _revealed_ mode, the presentation carries each credential's signature and signed message,
  which the verifier checks itself. The proof then shows only that $D$ is exactly the data those
  messages cover and that the result is correct. Removing signature verification from the proof
  reduced the cycles of our guest program (§#ref(<cost>, supplement: none)), but this mode
  discloses the signatures, the signed messages, the issuer keys used and the number of credentials
  (§#ref(<security>, supplement: none)).
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
signed tree from the credential's quads, which took fewer cycles in our measurements
(§#ref(<cost>, supplement: none)). Second, a proof compares values without parsing lexical
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
our zkVM guest verifies it, and we measured its cycles without proving
(§#ref(<cost>, supplement: none)).
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
subqueries and property paths, and rejects `SERVICE`, `NOW` and `RAND`. It evaluates `DESCRIBE`,
`FROM` and `FROM NAMED`, which version 1 of the specification excludes; with signature checks, our
services reject them before proving and before verification.
// TODO(evidence): bind to the coverage manifest of the evaluator with signature checks
// (zk/sparql-evaluator/coverage-authenticated-rdf.json, draft sparq-org/sparq#6791) once frozen as
// an evidence record. The executor sweep (paper-executor-sweep-*.json, §S4) already records, for
// its own query cases, the evaluator's admission and the host query profile.

We used only synthetic credentials signed with test keys. We generated all receipts with development
mode disabled and verified them against the expected guest image ID. For experiments without † in
@evidence-table, we recomputed hashes of source files, guest binaries and receipts. We compared the
hashes and recorded test outcomes with the archived records. We did not verify the proofs again.
Appendix #ref(<app-inventories>, supplement: none) lists the test cases, and the supplementary
material details the records (§#ref(<supp-records>, supplement: none)) and the software behind
them (§#ref(<repro>, supplement: none)).

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
    verified. Each zkVM experiment with signature checks in this table used one credential per test
    case; §#ref(<cost>, supplement: none) reports cycle counts for up to four.
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
#ref(<app-inventories>, supplement: none)). Some used `DESCRIBE` or `FROM NAMED`, which version 1 of
the specification excludes; with signature checks, our services now reject both before proving.
Their `DESCRIBE` answer
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

=== Cost by signature mode and cryptosuite <cost>

We counted cycles with the RISC Zero executor, which runs the guest program without proving. A cycle
count is deterministic for a given guest program and input, but it is not a proving time. The
executor also splits each execution into segments, which a prover proves separately. We used a later
version of the build with signature checks, which also supports the revealed mode and
`eddsa-sha256-merkle-2026`.

We ran #headline("zkexec.main_queries") queries over the payment credential: the payment `ASK`, a
`SELECT` of amounts with duplicate rows, a `CONSTRUCT` over the same pattern, a numeric `FILTER` on
`xsd:decimal` amounts, and a string `FILTER` on payment IRIs. Each query ran over one credential and
over four, each credential with #headline("zkexec.main_statements") statements. @cost-table gives
the median over the queries. In every configuration, the revealed mode needed fewer cycles and
segments than the hidden mode, and `eddsa-sha256-merkle-2026` needed fewer cycles than
`eddsa-rdfc-2022`.

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
    #headline("zkexec.main_queries") queries: its cycles, without the prover's overhead and
    padding per segment, and its segments. The counts are deterministic for a given guest program
    and input; they are not proving times.
  ],
) <cost-table>
]

We also executed an instrumented build of the same guest program, which reads the cycle counter at
the end of each phase; its counts include this instrumentation.
@phase-table shows the phases for the payment `ASK` over one credential. In the hidden mode,
verifying the Ed25519 signature was the largest phase with both cryptosuites. Revealing the
signatures removed this phase and left the others almost unchanged. The Merkle-based cryptosuite
replaces canonicalising and hashing the document with building its Merkle tree, which took fewer
cycles, but its signature verification took no fewer, because it also uses Ed25519. Evaluating the
query took fewer cycles than processing the document or verifying the signature.

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
      [Checking the request, its issuer keys and the input sizes], ..phase-cells("request"),
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
    Cycles in millions per phase for the payment `ASK` over one credential, by cryptosuite and
    signature mode, from an instrumented build of the guest program. The counts include the
    instrumentation and are not proving times.
  ],
) <phase-table>
]

A sweep of #headline("zkexec.sweep_cases") query cases, one per feature, ran in the same
configurations over generated credentials of #headline("zkexec.sweep_statements_small") and
#headline("zkexec.sweep_statements_large") statements each (@sweep-table). The evaluator admitted
#headline("zkexec.sweep_rdfc_hidden_n1_s32_admitted") cases with one credential and
#headline("zkexec.sweep_rdfc_hidden_n4_s32_admitted") with four, and rejected the others, such as
`SERVICE`, `NOW` and `RAND`. With one credential of #headline("zkexec.sweep_statements_small")
statements, in the hidden mode with `eddsa-rdfc-2022`, the admitted cases needed from
#mcycles("zkexec.sweep_rdfc_hidden_n1_s32_user_min") to
#mcycles("zkexec.sweep_rdfc_hidden_n1_s32_user_max") million cycles, with a median of
#mcycles("zkexec.sweep_rdfc_hidden_n1_s32_user_median"). Our prover stops an execution that reaches
a session limit of #mcycles("zkexec.sweep_session_limit") million cycles, and then produces no
proof. With `eddsa-rdfc-2022` in the hidden mode, four credentials of
#headline("zkexec.sweep_statements_large") statements exceeded this limit in every admitted case.
Every other configuration stayed within it, including the revealed mode and the Merkle-based
cryptosuite over the same credentials.

#todo-results[Proving time, memory and receipt size for each configuration and query, and the
issuer, holder and verifier costs: signing and tree construction, and verification time split into
receipt, signature and request checks. These need a dedicated proving run with development mode
disabled on one machine.]

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

_Other proof methods._ A proof method need not produce a zero-knowledge proof. The companion
specification proposes attestation by a TEE that a measured program evaluated the query and checked
the signatures. It also proposes QuickSilver @quicksilver21, a designated-verifier proof that is
interactive and that only the verifier taking part can check. We have built neither as a proof
method.
// No performance comparison with QuickSilver is reported: no evidence record exists for it.

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
credential. Our cycle counts, taken without proving, cover at most four synthetic credentials of
#headline("zkexec.sweep_statements_large") statements each, so we have no evidence yet about
realistic credentials, larger inputs or proving times.

== Conclusion <conclusion>

A holder can answer a verifier's SPARQL query over credentials without handing them over, but an
accepted answer is useful only if the verifier knows what it guarantees. We distinguish Supported
answers, whose returned solutions are all solutions, from Exact answers, which are complete over an
input that the holder declared or the verifier agreed in advance. When the request lists issuer
keys, one proof can show, under stated assumptions, the answer over exactly the data signed under
those keys, in a signature mode that trades proving cost against disclosure. A public-input rule lets the proof make public what the verifier can compute from its
request and the result alone. A prototype zkVM evaluator has proved Exact answers with signature
checks, including an answer that no payment was returned, each over one synthetic credential. Its
cycle counts, taken without proving, show signature verification as the largest step inside the
proof for the payment question, and fewer cycles when the holder reveals the signatures or the
issuer uses a Merkle-based cryptosuite. Realistic credentials, proving times and an external audit
remain to be done.

#pagebreak(weak: true)
#heading(level: 2, numbering: none)[References]
#bibliography("zksparql-architecture.refs.yml", style: "ieee", title: none)

// ---------------------------------------------------------------------------------------------
// Appendix: at most two LNCS pages after the references. It states what an accepted presentation
// shows and lists the test cases behind Table 3. The supplementary material follows it.
// ---------------------------------------------------------------------------------------------
#pagebreak(weak: true)
#heading(level: 1, numbering: none)[Appendix]
#counter(heading).update(0)
#set heading(numbering: (..n) => {
  let ns = n.pos()
  numbering("A.1.", ..if ns.len() > 1 { ns.slice(1) } else { ns })
})

== What an accepted presentation shows <app-relation>

For a zkVM proof method in the hidden or revealed mode, we argue informally that, under assumptions
A1 to A5 below, an accepted presentation means what §#ref(<meaning>, supplement: none) says.

*Relation.* Let $Q$ be the query of the request whose request digest is a public input
(§#ref(<linkage>, supplement: none)). The witness consists of the credentials, a salt and, in the
hidden mode, the credentials' signatures. Public inputs and witness satisfy the relation if:

+ the dataset commitment is the proof method's commitment to these credentials under this salt;
+ the input dataset $D$ is built from these credentials as the proof method states
  (§#ref(<meaning>, supplement: none)); with issuer keys, from their signed canonical N-Quads, so
  that every term keeps its signed lexical form;
+ if the request lists issuer keys, each credential carries a signature that verifies under a listed
  key whose entry matches the credential's issuer, verification method and cryptosuite. The proof
  verifies these signatures in the hidden mode. In the revealed mode, the signed messages are public
  inputs computed from the credentials, and the verifier checks the signatures;
+ the result is an answer of the requested kind for $Q$ over $D$
  (§#ref(<semantics>, supplement: none)).

*Assumptions.*

/ A1: The proof system is knowledge-sound: from any prover whose receipt verifies under the
  verifier's image ID, an efficient extractor obtains an input on which the guest program completes
  and writes the receipt's journal. We accept only succinct receipts
  (§#ref(<security>, supplement: none)).
/ A2: The guest program completes only on inputs that satisfy the relation for the values it
  writes to the journal; in particular, it evaluates $Q$ as SPARQL 1.1 specifies.
/ A3: SHA-256 is collision-resistant, so the request digest determines the request and the dataset
  commitment is binding.
/ A4: The verifier service performs the checks of §#ref(<validation>, supplement: none) against its
  stored request, with the image ID from its own configuration, and takes the result only from the
  journal.
/ A5: The cryptosuite's signatures are unforgeable, each listed key belongs to the issuer that its
  entry names, and that issuer keeps the private key secret.

Tests support A2 and A4 (Appendix #ref(<app-inventories>, supplement: none)); A1, A3 and A5 concern
the proof system, the hash function and the issuers.

*Argument.* Suppose the verifier accepts a presentation. By A4, the receipt verifies under the
verifier's image ID for a journal that holds the stored request's digest, its answer and input
kinds, and the presented commitment and result. By A1, an input exists on which the guest program
writes this journal; by A2, that input satisfies the relation. By A3, except with negligible
probability, $Q$ is the stored request's query and the commitment fixes $D$. The result is therefore
an answer of the requested kind for $Q$ over $D$, and for a verifier-agreed input, $D$ is the
dataset whose commitment the verifier agreed to. If the request lists issuer keys, each credential
in $D$ carries a signature under a listed key, which by A5 only that key's issuer could have made.
For a Supported answer, each returned row $mu$ is in $[| Q' |]_D$. As $Q'$ is a monotone query, $mu$
is also in $[| Q' |]_(D')$ for every dataset $D'$ that contains $D$ graph by graph, so the row stays
a solution when the holder's other credentials are added.

In the prototype, the journal binds the stored request through a SHA-256 hash
(§#ref(<capabilities>, supplement: none)), so under A3 it also binds the challenge, audience and
validity period.

*Public inputs computable from the result.* Suppose the pattern of $Q$ is a single basic graph
pattern without blank nodes, and a returned row binds each of its variables to an IRI or a literal.
The row is then a solution of the pattern over $D$ exactly when every triple obtained by
substituting the row into the pattern is in the default graph of $D$. A circuit that takes these
triples as public inputs, and checks that each is in $D$ and signed, therefore shows the same
statement as one that finds them in its witness. It discloses nothing that the verifier cannot
compute from the row. The baseline and public-triple circuits of
§#ref(<pilot-evidence>, supplement: none) differ in exactly this way.

== Test cases per experiment <app-inventories>

@test-cases-table shows the Exact answers that each zkVM experiment of @evidence-table proved, by
input kind. The inputs were small synthetic graphs or, with signature checks, the W3C test vector
for `eddsa-rdfc-2022` and the payment credential of §#ref(<v5-evidence>, supplement: none). The
supplementary material gives the tests and negative tests (§#ref(<supp-records>, supplement: none)),
and the source commits and guest binaries (§#ref(<repro>, supplement: none)).

#[
#show figure: set block(breakable: false)
#figure(
  {
    set text(size: 0.8em)
    set par(justify: false)
    table(
      columns: (1.9fr, 0.9fr, 1.1fr, 0.9fr, 1fr, 1.2fr),
      align: (left, center, center, center, center, center),
      table.header(
        [Exact answer], [CI build], [Our services], [First case], [Payment †], [Remaining †],
      ),
      [`SELECT`, duplicate rows], [H], [H, V], [V], [–], [H],
      [`SELECT`, `ORDER BY` and `COUNT`], [V], [–], [–], [–], [–],
      [`ASK`, true], [–], [V], [–], [–], [V],
      [`ASK`, false], [H, V], [H], [–], [H, V], [H],
      [`CONSTRUCT`], [V], [H, V], [–], [–], [H, V],
      [`DESCRIBE`], [H], [–], [–], [–], [–],
      [`SELECT` over the row limit, rejected], [–], [V], [–], [–], [V],
      [Negative tests], [In its tests], [#headline("zkvcq.adapter_controls") in total],
        [#headline("zkvcq.vcqg_controls")],
        [#headline("zkvcq.vcqp_controls") V, #headline("zkvcq.vcqph_controls") H],
        [#headline("zkvcq.ci_asktva_controls") V, #headline("zkvcq.ci_askfhd_controls") H each],
    )
  },
  caption: [
    Exact answers proved, by input kind: H holder-declared, V verifier-agreed; an entry may stand
    for several receipts. The columns are the rows of @evidence-table with receipts: the
    evaluator's CI build, the evaluator with our services and, with signature checks, the first test
    case, the payment question and the remaining test cases. † No internal check.
  ],
) <test-cases-table>
]

// ---------------------------------------------------------------------------------------------
// Supplementary material: not part of the appendix; at submission it goes behind an anonymous
// link. Sections and tables are numbered S1, S2, ...
// ---------------------------------------------------------------------------------------------
#pagebreak(weak: true)
#heading(level: 1, numbering: none)[Supplementary material]
#counter(heading).update(0)
#counter(figure.where(kind: table)).update(0)
#set figure(numbering: n => "S" + str(n))
#set heading(numbering: (..n) => {
  let ns = n.pos()
  "S" + numbering("1.1.", ..if ns.len() > 1 { ns.slice(1) } else { ns })
})

== Proof methods and request formats <capabilities>

The verifier service lists the proof methods it accepts in each query request, with a verification
key for each from its own configuration (§#ref(<request>, supplement: none)). @methods-table shows
what our proof methods support, and §#ref(<repro>, supplement: none) gives their identifiers. Our
services reject a request outside these limits before proving and before marking it as answered.
They also reject a base IRI, an ordered `SELECT`, `DESCRIBE`, and a request that requires credential
status or holder binding.

#[
#show figure: set block(breakable: false)
#figure(
  {
    set text(size: 0.8em)
    set par(justify: false)
    table(
      columns: (1fr, 1.3fr, 1.3fr, 1.3fr),
      align: (left, left, left, left),
      table.header[][Evaluator][Evaluator with signature checks][Noir circuits],
      [Evidence], [Succinct RISC Zero receipt, publicly verifiable], [Same],
        [UltraHonk proof, publicly verifiable],
      [Answers], [Exact `SELECT`, `ASK` and `CONSTRUCT`], [Same],
        [Supported `SELECT DISTINCT` over one basic graph pattern],
      [Input kinds], [Holder-declared, verifier-agreed], [Same], [Holder-declared],
      [Input dataset], [Parsed from the committed N-Quads], [RDF merge of the signed canonical
        N-Quads], [Credentials in the circuits' own commitment format],
      [Issuer checks], [None: the request lists no issuer keys], [`eddsa-rdfc-2022`, in the hidden
        mode], [Schnorr signatures over Baby Jubjub, inside the proof],
      [Credential status], [Not checked], [Not checked], [Checked against a snapshot that the
        verifier accepted],
      [Presentations of the specification], [Not yet: our services use a binary encoding], [Same],
        [Not yet: no dataset commitment],
    )
  },
  caption: [What our proof methods support.],
) <methods-table>
]

*Request formats.* The CI build of the evaluator read requests in three formats, each extending the
one before: the default-graph format; the named-graph format, which adds named graphs, `GRAPH`,
`FROM` and `FROM NAMED`; and the graph-result format, which adds `CONSTRUCT`, `DESCRIBE` and blank
nodes in the input and the result. Behind our services, both builds use the graph-result format;
with signature checks, the services also reject `FROM` and `FROM NAMED`
(§#ref(<prototype>, supplement: none)). The
services encode the query request in binary rather than in the specification's JSON, compute the
request digest over that encoding, and store times as Unix seconds.

*How the stored request reaches the proof.* The holder service gives the guest program the query,
the input kind, any agreed commitment, the evaluation limits and a SHA-256 hash of the stored
request and its proof-method entry. The guest program writes a digest of these values to the
journal, and the verifier service recomputes it from its own copies. The journal therefore binds
every member of the stored request, including the challenge, audience and validity period. With
signature checks, the proof-method entry contains a digest of the request's issuer keys and the
evaluation limits, so changing any key, issuer or verification method changes the entry.

*Dataset commitments.* Without signature checks, the evaluator commits with SHA-256 to a format tag,
its capacity limits, the salt, the names of any named graphs and the exact bytes of the N-Triples or
N-Quads input. It does not canonicalise the input, so equivalent serialisations give different
commitments. With signature checks, the commitment covers the digest of the request's issuer keys
and the evaluation limits, the salt, the number of credentials and, for each credential, the hashes
of the canonical document and proof configuration that the issuer signed. A commitment agreed under
one list of issuer keys therefore cannot serve another.

== Experiment records <supp-records>

@tests-table lists the tests and executions without proving, by run. Appendix
#ref(<app-inventories>, supplement: none) lists the receipts, and @controls-table the negative
tests.

#[
#show figure: set block(breakable: false)
#figure(
  {
    set text(size: 0.8em)
    set par(justify: false)
    table(
      columns: (1fr, auto),
      align: (left, right),
      table.header[Test or execution][Count],
      table.cell(colspan: 2)[_Evaluator, CI build_],
      [Evaluator tests passed, outside the zkVM], [#headline("zkvcq.exact_hosted_model_tests")],
      [Tests of the guest program passed], [#headline("zkvcq.exact_hosted_host_tests")],
      [Tests not run: proofs of re-executed test cases], [#headline("zkvcq.exact_hosted_ignored")],
      [Test cases re-executed outside the zkVM], [#headline("zkvcq.exact_replay_cases")],
      [Runs: test cases × request formats × input kinds], [#headline("zkvcq.exact_replay_jobs")],
      [Runs expected and observed to accept], [#headline("zkvcq.exact_replay_expected_positive")],
      [Runs expected and observed to reject], [#headline("zkvcq.exact_replay_expected_negative")],
      [Proofs from re-execution], [#headline("zkvcq.exact_replay_proofs")],
      table.cell(colspan: 2)[_Evaluator with signature checks: tests_],
      [Tests passed, outside the zkVM], [#headline("zkvcq.v5_auth_tests_passed")],
      [Of these, integration tests of the signature checks],
        [#headline("zkvcq.v5_new_integration_tests")],
      [Of these, unit tests of the signature checks], [#headline("zkvcq.v5_new_unit_tests")],
      [Proofs], [#headline("zkvcq.v5_proofs")],
      table.cell(colspan: 2)[_Evaluator with signature checks: executed directly_],
      [Tests passed, outside the zkVM], [#headline("zkvcq.v5g_native_tests_passed")],
      [Proving test, compiled but not run], [#headline("zkvcq.v5g_genuine_driver_ignored")],
      [Tests of direct execution passed], [#headline("zkvcq.v5g_direct_sdk_tests_passed")],
      [Executions accepting a valid input], [#headline("zkvcq.v5g_direct_v5_positive")],
      [Executions rejecting an altered input with the expected error],
        [#headline("zkvcq.v5g_direct_v5_aborts")],
      [Evaluator without signature checks, accepting its own input],
        [#headline("zkvcq.v5g_direct_legacy_positive")],
      [Evaluator without signature checks, rejecting input for the other build],
        [#headline("zkvcq.v5g_direct_legacy_aborts")],
      [Executions in total], [#headline("zkvcq.v5g_direct_executions")],
      [Proofs], [#headline("zkvcq.v5g_proofs")],
      table.cell(colspan: 2)[_Our services with signature checks_],
      [Tests passed, outside the zkVM], [#headline("zkvcq.vcqa_distinct_tests_passed")],
      [Proofs], [#headline("zkvcq.vcqa_genuine_proofs")],
    )
  },
  caption: [Tests and executions without proving, by run. Counts are per run.],
) <tests-table>
]

=== The evaluator without signature checks <supp-exact>

A CI job built the evaluator and its guest program from source with locked dependencies. It ran the
evaluator's tests, then the tests that run the guest program, one at a time. These produced
#headline("zkvcq.exact_hosted_receipts") receipts, one for each of these queries:

- in the default-graph format, a `SELECT` with `UNION` and `VALUES` whose result keeps duplicate
  rows and an unbound value (holder-declared); a `SELECT` with a counting subquery, `OPTIONAL`,
  `MINUS`, `EXISTS`, property paths, `ORDER BY` and `LIMIT` (verifier-agreed); and a false `ASK`
  whose branches use property paths, `MINUS`, `NOT EXISTS` and numeric edge cases (verifier-agreed);
- in the named-graph format, a false `ASK` over graphs named with `FROM NAMED` (holder-declared),
  and a `SELECT` that counts the matches in each named graph, an empty one included
  (verifier-agreed);
- in the graph-result format, a `SELECT` whose result keeps duplicate rows, unbound values and a
  blank node shared across rows (holder-declared); a `DESCRIBE` (holder-declared); and a `CONSTRUCT`
  that creates fresh blank nodes (verifier-agreed).

Each test compares the journal with the result of the evaluator run outside the zkVM. Some also
check that verification fails after a change to the query, the request's limits, the input kind, the
agreed commitment or one journal byte. They also check that it fails under another image ID and for
a fake receipt, and that a second verification of the same receipt fails as a replay.

Our services ran a separately built guest program of the same evaluator on another machine
(§#ref(<repro>, supplement: none)). Their test cases query a small synthetic graph in which triples
share an object, so the `SELECT` result has duplicate rows.

=== The evaluator with signature checks <supp-signed>

@tests-table lists the runs that tested this build without proving. Its tests outside the zkVM use
the W3C test vector and synthetic keys. They check that valid signatures under the wrong issuer,
verification method, proof purpose or cryptosuite are rejected, and that signed lexical forms and
each credential's blank-node scope are kept. They also check that the request's issuer keys are
bound into the request and the commitment, and they evaluate `SELECT`, `ASK` and `CONSTRUCT` under
both input kinds. Executed without proving, the guest program accepted the test vector under both
input kinds. Besides the altered credentials of §#ref(<v5-evidence>, supplement: none), it rejected
input that was not in canonical form or used another encoding. A positive control preceded each
group of negative tests. The evaluator without signature checks rejected input meant for this build
and still accepted its own. The tests of our services check that changing a listed key, issuer,
verification method or limit changes the proof-method entry's digest and the digest in the journal,
while reordering the keys does not. They also check that a mismatched proof-method entry, guest
program or image ID, and a fake or foreign receipt, are rejected before the request is marked as
answered.

The receipts came from three experiments (Appendix #ref(<app-inventories>, supplement: none)), with
different machines and guest binaries (§#ref(<repro>, supplement: none)). For the first test case,
the internal check covered #headline("zkvcq.vcqg_source_files_verified") source files, unchanged
during the run, and #headline("zkvcq.vcqg_archive_verified_files") archived files.

=== Negative tests <supp-negative>

@controls-table lists the negative tests by what they change. Each reused the receipt of an accepted
test case, so none produced a proof. A store in memory recorded which requests were answered.

#[
#show figure: set block(breakable: false)
#figure(
  {
    set text(size: 0.8em)
    set par(justify: false)
    table(
      columns: (1fr, auto, auto),
      align: (left, right, left),
      table.header[Negative test][Without signature checks][With signature checks],
      table.cell(colspan: 3)[_Stored request_],
      [Another query of the same form], [#headline("zkvcq.ctl_changed_query_same_form")], [Yes],
      [Another challenge], [#headline("zkvcq.ctl_wrong_challenge")], [Yes],
      [Another audience presented], [#headline("zkvcq.ctl_wrong_verifier_audience")], [Yes],
      [Stored audience changed to the presented one],
        [#headline("zkvcq.ctl_changed_stored_audience")], [Yes],
      [Expired], [#headline("zkvcq.ctl_expired")], [Yes],
      [Not yet valid], [#headline("zkvcq.ctl_not_yet_valid")], [Yes],
      [Another validity period that contains the current time],
        [#headline("zkvcq.ctl_changed_window")], [Yes],
      table.cell(colspan: 3)[_Proof method_],
      [Another proof-method entry digest], [#headline("zkvcq.ctl_wrong_descriptor_digest")], [Yes],
      [Receipt checked against the other build's image ID], [–], [Yes],
      table.cell(colspan: 3)[_Issuer keys and limits held by the verifier_],
      [Another key, issuer or verification method, an added key, or a lower evaluation limit; each
        with the presentation unchanged, and with its proof-method entry digest replaced to match],
        [–], [Yes],
      table.cell(colspan: 3)[_Input kind and commitment_],
      [Holder-declared proof for a verifier-agreed request],
        [#headline("zkvcq.ctl_scope_holder_to_agreed")], [Holder-declared receipts],
      [Verifier-agreed proof for a holder-declared request],
        [#headline("zkvcq.ctl_scope_agreed_to_holder")], [Verifier-agreed receipts],
      [Another agreed commitment], [#headline("zkvcq.ctl_wrong_agreed_anchor")],
        [Verifier-agreed receipts],
      [Agreed commitment computed with another salt], [–], [Verifier-agreed receipts],
      table.cell(colspan: 3)[_Receipt_],
      [Result in the journal altered], [#headline("zkvcq.ctl_altered_journal_result")], [Yes],
      [One journal byte flipped], [–], [Yes],
      [Fake receipt], [–], [Yes],
      table.cell(colspan: 3)[_Marking the request as answered_],
      [Replay of an accepted presentation], [#headline("zkvcq.ctl_replay_same_store")], [Yes],
      [Concurrent presentations: all but one rejected as replays],
        [#headline("zkvcq.ctl_concurrent_verifications")], [Yes],
      [Failing store: presentation rejected], [#headline("zkvcq.ctl_broken_store")], [Yes],
      [Total], [#headline("zkvcq.adapter_controls")],
        [#headline("zkvcq.vcqg_controls") per verifier-agreed, #headline("zkvcq.vcqph_controls")
          per holder-declared receipt],
    )
  },
  caption: [
    Negative tests, by what they change. Without signature checks: records across the
    #headline("zkvcq.adapter_accepted") accepted receipts of the evaluator with our services. With
    signature checks: tests run with each accepted receipt; "Yes" means with every receipt.
  ],
) <controls-table>
]

== The public-triple circuit pilot <supp-pilot>

The baseline and public-triple circuits give Supported answers to `SELECT DISTINCT` queries over one
basic graph pattern without `FILTER`, and reject a result that repeats a row. Each exists for one
and for two credentials. Inside the circuit, both check each credential's Schnorr signature, that
each matched triple is in a signed credential, and each credential's status against a status list
and policy that the verifier accepted. The status roots, salts and positions stay hidden. How the
verifier obtains the status snapshot, and how fresh it is, are left to the deployment. Their tests
cover a finite set of valid and absent matches and of adversarial witnesses.

// "Median (range)" cells of the pilot, and the per-pair differences (public-triple minus baseline):
// pairs 1 to n ran one credential and pairs n + 1 to 2n two, with n timed runs per cell. Every value
// is looked up; nothing is computed from the values.
#let pilot-cell(stem) = [#pilot(stem + "_median") (#pilot(stem + "_min")–#pilot(stem + "_max"))]
#let per-cell = headline("zkvcq.pp_n_per_cell")
#let pilot-diffs(kind, first) = range(first, first + per-cell).map(i => pilot(
  "zkvcq.pilot_pair" + str(i) + "_" + kind + "_diff")).join(", ")

#block(inset: 8pt, stroke: 0.5pt + gray, width: 100%, breakable: false)[
  *Preliminary timings.* We timed #headline("zkvcq.pp_n_per_cell") runs per circuit and number of
  credentials, after #headline("zkvcq.pp_warmups") warm-up runs that we excluded. The runs
  alternated the two circuits in pairs on a shared cloud machine (Intel Xeon Platinum 8488C, eight
  vCPUs) limited to two CPUs and eight GiB of memory, with caches uncontrolled. Prove and verify
  times include compilation, key and I/O stages.

  #figure(
    table(
      columns: (auto, auto, 1fr, 1fr),
      align: (left, left, right, right),
      table.header[Credentials][Circuit][Prove, s: median (range)][Verify, s: median (range)],
      [One], [Baseline], pilot-cell("zkvcq.pilot_k1_v1_prove"),
        pilot-cell("zkvcq.pilot_k1_v1_verify"),
      [One], [Public-triple], pilot-cell("zkvcq.pilot_k1_v4_prove"),
        pilot-cell("zkvcq.pilot_k1_v4_verify"),
      [One], [Difference per pair], [#pilot-diffs("prove", 1)], [#pilot-diffs("verify", 1)],
      [Two], [Baseline], pilot-cell("zkvcq.pilot_k2_v1_prove"),
        pilot-cell("zkvcq.pilot_k2_v1_verify"),
      [Two], [Public-triple], pilot-cell("zkvcq.pilot_k2_v4_prove"),
        pilot-cell("zkvcq.pilot_k2_v4_verify"),
      [Two], [Difference per pair], [#pilot-diffs("prove", 1 + per-cell)],
        [#pilot-diffs("verify", 1 + per-cell)],
    ),
    caption: [Preliminary prove and verify times of the circuit pilot
      (§#ref(<pilot-evidence>, supplement: none)): median and range per circuit, and the difference,
      public-triple minus baseline, for each pair of runs.],
  ) <pilot-table>

  The verifying difference of largest magnitude came from one slow baseline run.
]

== Executor cycle counts <supp-cost>

The measurements of §#ref(<cost>, supplement: none) ran the guest program with signature checks in
the RISC Zero executor, without proving, for each cryptosuite, signature mode and number of
credentials and, in the sweep, each credential size. Every request used a holder-declared input,
and the RFC 8032 test key signed every credential. For the #headline("zkexec.main_queries") queries,
we executed #headline("zkexec.main_runs") runs over the payment credential and, with four
credentials, copies issued to other customers. For the sweep, we executed
#headline("zkexec.sweep_runs") runs over generated credentials. Each describes a person with typed
literals, a chain of acquaintances and one blank node, padded with further statements to the stated
size. The counts of the two sets are never added. The executor runs had no internal check
(§#ref(<prototype>, supplement: none)).

The executor runs used our prover's session limit (§#ref(<cost>, supplement: none)), and a case
that reached it counts as admitted but not as completed. The cycle counts, like the limit, cover the
guest program's own cycles, without the overhead and padding that the prover adds to each segment.

For @phase-table, a separate build of the same guest program, used only for measurement, reads the
cycle counter after decoding its input and at the end of each phase. It passes the readings to the
host, not to the journal. A phase that runs once per credential is summed over the credentials, and
the readings include the instrumentation.

The sweep's query cases cover basic graph patterns of several sizes, as stars and chains;
`OPTIONAL`, `UNION`, `MINUS`, `EXISTS`, `NOT EXISTS`, `BIND`, `VALUES` and a subquery; comparisons
of IRIs, strings, language-tagged strings and numeric and date-time literals, and string functions;
aggregates; `DISTINCT`, and `ORDER BY` with `LIMIT` and `OFFSET`; property paths; each query form;
and probes of features that the evaluator should reject or that cannot match credential data. Run
outside the zkVM on the same input, the evaluator decided which cases to admit. It rejected
`EXISTS` and `NOT EXISTS`, which it does not admit over data with blank nodes such as the generated
credentials; `SERVICE`; the functions `BNODE`, `NOW` and `RAND`; a custom function; a triple term;
and a nested `EXISTS`. With four credentials, it also rejected a zero-or-more property path whose
result exceeded its limits. It admitted `DESCRIBE`, `FROM` and `FROM NAMED`, which our services
reject before proving (§#ref(<prototype>, supplement: none)). @sweep-table gives the counts and
cycles per configuration.

#[
#show figure: set block(breakable: false)
// One row per number of credentials and credential size. A configuration in which no admitted
// case completed has no cycle or segment values.
#let sweep-rows(suite, mode) = {
  let rows = ()
  for (suffix, credentials, size) in (
    ("_n1_s32", [One], "zkexec.sweep_statements_small"),
    ("_n1_s64", [One], "zkexec.sweep_statements_large"),
    ("_n4_s32", [Four], "zkexec.sweep_statements_small"),
    ("_n4_s64", [Four], "zkexec.sweep_statements_large"),
  ) {
    let stem = "zkexec.sweep_" + suite + "_" + mode + suffix
    let completed = headline(stem + "_completed")
    rows += (
      credentials, [#headline(size)], [#headline(stem + "_admitted")],
      [#headline(stem + "_rejected")], [#completed],
      if completed == 0 { [–] } else {
        let range = [#mcycles(stem + "_user_min")–#mcycles(stem + "_user_max")]
        [#mcycles(stem + "_user_median") (#range)]
      },
      if completed == 0 { [–] } else { [#headline(stem + "_segments_median")] },
    )
  }
  rows
}
#figure(
  {
    set text(size: 0.8em)
    set par(justify: false)
    table(
      columns: (auto, auto, auto, auto, auto, 1fr, auto),
      align: (left, right, right, right, right, right, right),
      table.header[Credentials][Statements each][Admitted][Rejected][Completed][Cycles,
        millions: median (range)][Segments, median],
      table.cell(colspan: 7)[`eddsa-rdfc-2022`, hidden mode],
      ..sweep-rows("rdfc", "hidden"),
      table.cell(colspan: 7)[`eddsa-rdfc-2022`, revealed mode],
      ..sweep-rows("rdfc", "revealed"),
      table.cell(colspan: 7)[`eddsa-sha256-merkle-2026`, hidden mode],
      ..sweep-rows("merkle", "hidden"),
      table.cell(colspan: 7)[`eddsa-sha256-merkle-2026`, revealed mode],
      ..sweep-rows("merkle", "revealed"),
    )
  },
  caption: [
    The sweep's #headline("zkexec.sweep_cases") query cases per configuration: cases the evaluator
    admitted and rejected, admitted cases that completed within the session limit, and the cycles
    and segments of the completed cases. Cycles exclude the prover's overhead and padding per
    segment; they are executor counts, not proving times.
  ],
) <sweep-table>
]

== An earlier fixed-circuit design <legacy>

Before the zkVM evaluator, we built a family of Noir circuits with one kind of circuit per operator.
It proves that each returned solution is a solution, as a Supported answer does, for `SELECT` and
`ASK` queries built from basic graph patterns, joins and numeric `FILTER`s, with projection,
`DISTINCT`, `REDUCED`, `LIMIT` and `OFFSET`. It excludes `OPTIONAL`, `MINUS`, `NOT EXISTS` and
aggregates, whose results more data could invalidate, and `ORDER BY`, whose order it does not prove.
It also rejects a join on blank nodes from two credentials, because each credential's blank nodes
are scoped to it. The baseline and public-triple circuits of §#ref(<supp-pilot>, supplement: none)
are later members of this family.

The issuer canonicalises each credential's graph with RDFC-1.0, commits to it with Poseidon2 over
the BN254 scalar field, and signs the commitment with a Schnorr signature over Baby Jubjub.
Credentials signed with Ed25519 or ECDSA are checked outside the circuit and committed again, which
a verifier cannot rely on if the holder is dishonest. The commitments are public and unsalted, so a
verifier can confirm a guessed graph and link repeated commitments; the later circuits keep them
hidden. A JSON manifest combines one proof per operator. The verifier derives each circuit from the
manifest and recomputes its verification key instead of accepting one from the prover. It also
reconstructs the public inputs, requires issuer signatures under its own key list, and accepts each
challenge once. @legacy-table gives the sizes of the circuits.

#[
#show figure: set block(breakable: false)
#figure(
  {
    set text(size: 0.8em)
    set par(justify: false)
    table(
      columns: (1fr, auto),
      align: (left, right),
      table.header[Circuit][UltraHonk gates],
      [Basic graph pattern match: smallest, largest],
        [#headline("zkarch.gates_scan_min"), #headline("zkarch.gates_scan_max")],
      [Value `FILTER`, literal committed as hashed text], [#headline("zkarch.gates_filter_lane")],
      [Integer `FILTER`, value committed with the literal; off by default],
        [#headline("zkarch.gates_filter_value_dl_int")],
      [Join on a hidden term: smallest, largest],
        [#headline("zkarch.gates_join_min"), #headline("zkarch.gates_join_max")],
      [Credential not revoked], [#headline("zkarch.gates_revoke")],
      [Hidden issuer: Schnorr signature and key-list membership],
        [#headline("zkarch.gates_hidden_issuer")],
      [Hidden holder; does not yet establish holder binding], [#headline("zkarch.gates_holder_pok")],
    )
  },
  caption: [
    Sizes of the earlier circuits, from #headline("zkarch.circuit_members") compiled circuits of
    #headline("zkarch.circuit_kinds") kinds. These are constraint counts, not timings. In the
    integer `FILTER` with a committed value, the issuer must ensure that the value matches the
    literal.
  ],
) <legacy-table>
]

An internal review of an earlier version of this verifier found
#headline("cozk.single_prover_audit_issues") issues. Among them were public inputs it did not
reconstruct, verification keys taken from the prover, unsigned commitments, replayable manifests and
`FILTER` operators not bound to the query. A regression test that builds the forgery and expects
rejection now covers #headline("zkarch.forge_findings_mapped") of them. The circuits for a hidden
holder do not yet establish holder binding and are off by default. The design cannot prove Exact
answers, checks the standard cryptosuites only outside the circuit, and hides every term that an
operator uses. Exact answers, signature checks inside the proof and the public-input rule address
these limits.

== Combining BBS+ proofs with circuits <composition>

A tempting design verifies BBS+-signed credentials with BBS+ proofs, disclosing public terms, and
passes hidden values to a circuit for conditions that BBS+ proofs cannot express. A shared challenge
does not bind the two proofs: it shows that both belong to one exchange, not that they concern the
same value. One way to bind them is to prove in both systems that the proofs use the same committed
hidden value, with one encoding of bytes into field elements, range checks where the fields differ,
and domain separation. This also needs an argument that knowledge soundness holds for the
combination. We have not built this binding.

We built two experimental parts. The first combines BBS+ signatures over BLS12-381 with Circom
circuits proved with LegoGroth16 over the same field. The second uses BBS+ proofs alone to show that
the triples of one basic graph pattern, rebuilt from public terms, come from credentials signed by
an issuer that the verifier lists, none of which the status list marks as revoked. It discloses
which signed messages it uses and their status references, supports no condition on hidden values,
has no link to the Noir circuits and evaluates no other SPARQL.

In one CI run over a finite set of test cases, the second part produced
#headline("zkvcq.nc_distinct_proofs") distinct BBS+ proofs under
#headline("zkvcq.nc_distinct_nonces") distinct challenges. The required verifier configuration
accepted #headline("zkvcq.nc_required_accepted") of them and rejected
#headline("zkvcq.nc_weaker_rejected") that verify under a weaker configuration. The run rejected
#headline("zkvcq.nc_empty_graph_admission") further cases with an empty graph before proving, and
excluded #headline("zkvcq.nc_other_exclusions") others. Separately, a proof made with the
command-line tool verified, and was rejected after we substituted the issuer, query, result or
challenge, replayed it, revoked the credential or changed the status epoch. These are neither Exact
answers nor combined proofs. A comparison of BBS+ proofs, circuits and zkVMs would need methods that
prove the same statement with the same signature checks and disclosure.

== Artifacts and reproduction <repro>

@artifact-table identifies the software behind each experiment. Each experiment's record lists the
commands it ran, the versions and hashes of the compilers and the prover, the hashes of the guest
binary and of each receipt, and every test outcome. The records of the executor runs list instead
each run's configuration, admission and cycle counts, with the image IDs and the prover version.
The paper reads its numbers from these records when it is built. Re-running an experiment needs the
recorded toolchains and, for proving, a machine that can run the RISC Zero or Barretenberg prover.
Guest binaries built from the same source differ between experiments because the build path enters
the binary; we obtained identical binaries only with a fixed build path and toolchain.

#[
#show figure: set block(breakable: false)
// The sweep ran the same source commit and guest image as the main queries, and both executor
// sets used the prover version named in the caption.
#assert(headline("zkexec.sweep_source_commit") == headline("zkexec.main_source_commit"))
#assert(headline("zkexec.sweep_guest_image_id") == headline("zkexec.main_guest_image_id"))
#assert(headline("zkexec.main_r0vm") == headline("zkvcq.exact_r0vm_version"))
#assert(headline("zkexec.sweep_r0vm") == headline("zkvcq.exact_r0vm_version"))
#figure(
  {
    set text(size: 0.8em)
    set par(justify: false)
    table(
      columns: (1.3fr, 1.5fr, auto, auto, 0.9fr),
      align: (left, left, left, left, left),
      table.header[Experiment][Proof method and version][Source commit][Guest binary][Machine],
      [Evaluator, CI build], [The evaluator, in all three request formats],
        [#short-id("zkvcq.exact_source_commit")], [#short-id("zkvcq.exact_guest_sha256")],
        [GitHub Actions runner],
      [Evaluator with our services], [`urn:sparq:vcq:method:risc0-exact`, 3],
        [#short-id("zkvcq.adapter_source_commit")], [#short-id("zkvcq.adapter_guest_sha256")],
        [EC2 instance],
      [With signature checks: tests], [–], [#short-id("zkvcq.v5_source_commit")], [–],
        [EC2 instance],
      [With signature checks: executed directly], [–], [#short-id("zkvcq.v5g_source_commit")],
        [#short-id("zkvcq.v5g_guest_sha256")], [EC2 instance],
      [With signature checks and our services: tests; first test case],
        [`urn:sparq:vcq:method:risc0-authenticated-rdf`, 5],
        [#short-id("zkvcq.vcqg_source_commit")], [#short-id("zkvcq.vcqg_guest_sha256")],
        [EC2 instance],
      [Payment question †], [Same], [#short-id("zkvcq.vcqp_source_commit")],
        [#short-id("zkvcq.vcqp_guest_sha256")], [Cloud container],
      [Remaining test cases †], [Same], [#short-id("zkvcq.ci_source_commit")],
        [#short-id("zkvcq.ci_guest_sha256")], [GitHub Actions runners],
      [With signature checks: executor runs (§#ref(<supp-cost>, supplement: none)) †], [–],
        [#short-id("zkexec.main_source_commit")], [#short-id("zkexec.main_guest_image_id") ‡],
        [Shared container],
      [Same: instrumented build for phase counts †], [–], [Same],
        [#short-id("zkexec.main_phase_image_id") ‡], [Shared container],
      [Public-triple circuit pilot], [Noir circuits], [#short-id("zkvcq.pp_source_commit")], [–],
        [EC2 instance],
      [BBS+ proofs (§#ref(<composition>, supplement: none))], [–],
        [#short-id("zkvcq.nc_source_commit")], [–], [GitHub Actions runner],
    )
  },
  caption: [
    Software behind each experiment. Commits and SHA-256 digests of guest binaries show their
    first twelve hexadecimal digits; the RISC Zero image ID is computed from the guest binary. The
    zkVM experiments used the prover #raw(headline("zkvcq.exact_r0vm_version")), and the circuits
    were compiled with `nargo 1.0.0-beta.21`. † No internal check. ‡ The RISC Zero image ID, not
    the digest of the guest binary.
  ],
) <artifact-table>
]
