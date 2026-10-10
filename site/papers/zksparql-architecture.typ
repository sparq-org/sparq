// [OPUS-5.5] zkSPARQL architecture paper. The main body follows the language guide
// (site/papers/zksparql-language-guide.md) and the companion specification of zero-knowledge
// SPARQL answers (sparq-org/sparq#6786, spec v2): query request and answer presentation as RDF in
// JSON-LD, answers as results SPARQL 1.2 permits over the input dataset, open-world and
// closed-world readings, input datasets chosen by the holder or agreed in advance, the hidden,
// revealed and disclosed signature modes, the public-input rule and the verifier's processing
// order. The appendix (at most two LNCS pages) and the supplementary material after it follow the
// same guide.
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

#set document(title: "Answers Instead of Documents: Private SPARQL Answers over Verifiable Credentials")
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

// [OPUS-5.5] The double-blind build (`--input anon=true`) shows a placeholder for every value that
// identifies our repository: proof-method identifiers, commits, and the digests and image IDs of
// guest binaries, which its public records list. The normal build shows the value unchanged.
#let withheld = "[withheld for review]"
#let repo-id(body) = if anon { withheld } else { body }

// Short commit / digest rendering for bound identity records (first twelve hex characters). The
// record is looked up in both builds, so the anonymous build still checks it.
#let short-id(key) = repo-id(raw(headline(key).slice(0, 12)))

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
    Private SPARQL Answers over Verifiable Credentials
  ]
]
#authors()

#align(center)[#text(style: "italic", size: 0.9em)[
  Working draft. The measurements marked as pending in §#ref(<cost>, supplement: none) are not
  yet available.
]]

#heading(level: 2, numbering: none, outlined: false)[Abstract]

A verifier often needs the answer to a question about a person's credentials, not the credentials
themselves. zkRDF proves that each returned SPARQL solution follows from signed data but cannot
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
synthetic credential each, including an answer that no payment was returned. Executed without
proving, a later build evaluated queries with `OPTIONAL`, `MINUS`, aggregates, subqueries and
property paths over up to four credentials. Cycle counts show signature verification as the largest
step for the payment question, and fewer cycles with revealed signatures or a Merkle-based
cryptosuite. Proving times are pending.

== Introduction <intro>

A lender assessing a mortgage application asks the applicant one question: was any payment from the
applicant's account returned unpaid? The usual answer is a bank statement, which also discloses
every payment, balance and payee that the lender did not ask about. If the bank issued it as a
verifiable credential @vcdm2, the lender could check that the bank signed it, but the credential
would still disclose everything in it.

A zero-knowledge proof lets the applicant, as holder, return only the answer, with a proof that it
is correct. The lender writes its question as a SPARQL query @sparql12, our running example:

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
// TODO(citation): status of the zksparql.org preprint (author to confirm)
Such a result can rest on absence, as `false` does; we call it a closed-world answer, and it is only
as useful as its input is complete. Because the holder chooses which credentials to include,
`false` means only "no returned payment in the credentials I included". The verifier can fix the
input in advance, but `false` then still covers only that input. No stronger proof removes this
limit; the request can only state who fixed the input.

Checking signatures inside the proof is costly: for the payment question, it was the largest step
in our cycle counts (§#ref(<cost>, supplement: none)). Revealing the signatures to the verifier
removes that step, but lets the verifier link presentations of a credential and, with the standard
RDF cryptosuites, confirm a guess about its content. The request therefore lists the signature
modes the verifier accepts. Our contributions are:

- *Answers and their reading* (§#ref(<meaning>, supplement: none)): an answer is a result SPARQL
  1.2 permits over the input dataset; closed-world answers depend on who fixed the input, which the
  request states.
- *An architecture* (§#ref(<architecture>, supplement: none)) in which one proof shows such an
  answer, bound to the verifier's request, with every credential's signature checked against the
  request's trusted issuers (Appendix #ref(<app-relation>, supplement: none)).
- *Signature modes and Merkle-based cryptosuites* (§#ref(<modes>, supplement: none),
  §#ref(<cryptosuites>, supplement: none)), which set what authenticating the input costs and
  discloses.
- *A public-input rule* (§#ref(<minimize>, supplement: none)): a proof method may make public any
  value that the verifier can compute from its request and the result alone.
- *An evaluation* (§#ref(<evidence>, supplement: none)) of a SPARQL evaluator in the RISC Zero zkVM
  @risc0: proofs with signature checks of `SELECT`, true and false `ASK` and `CONSTRUCT` answers;
  executions without proving of one query case per feature, with the unsupported features listed
  (§#ref(<expressivity>, supplement: none)); and cycle counts by signature mode and cryptosuite.
  Proving times are pending.

== Background <background>

=== RDF datasets and SPARQL results <bg-sparql>

An RDF dataset has one default graph and zero or more named graphs @sparql12. SPARQL has four query
forms: `SELECT` returns a sequence of solution mappings (rows), `ASK` a boolean, and `CONSTRUCT` and
`DESCRIBE` an RDF graph. Pattern matching compares terms, whereas the `=` operator compares values:
`"1250.00"^^xsd:decimal` and `"1250.0"^^xsd:decimal` are different terms with equal values @rdf11.
The companion specification of §#ref(<architecture>, supplement: none) depends on SPARQL 1.2
@sparql12 and RDF 1.2 @rdf12, which add triple terms. RDF is usually read under an open-world
assumption: an absent triple is not thereby false.

=== Credentials and Data Integrity proofs <bg-vc>

A verifiable credential @vcdm2 is a set of claims that an issuer makes about a subject, secured so
that its authorship can be verified. A holder presents it to a verifier in a verifiable
presentation, which may instead carry data derived from credentials, such as a zero-knowledge proof.
Here, a credential's Data Integrity proof @vcdi is a signature, in its `proofValue`, with the proof
configuration: the proof's other properties. A cryptosuite specifies how to create and verify the
proof, which names its verification method, here a public key, by an identifier
(`verificationMethod`). The cryptosuite `eddsa-rdfc-2022` @vcdieddsa canonicalises the
credential and the proof configuration with RDF Dataset Canonicalization (RDFC-1.0) @rdfc10;
Ed25519 then signs the SHA-256 hash of the canonical proof configuration followed by that of the
canonical document. A valid signature shows only that someone with the signing key signed these
bytes; the verifier must still check that the issuer authorised the key, through a verification
relationship in the issuer's controlled identifier document @vcdi. It may also check credential
status (revocation or suspension), the validity period, and holder binding: that the presenter is
the subject or controls a key bound to the credential. None of these checks makes the claims true.
Selective-disclosure cryptosuites such as `bbs-2023` @vcdibbs let the holder derive a proof that
discloses only selected claims.

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
zkVM can thus prove a SPARQL evaluator without a circuit per query, but only that it ran over the
bytes it received, not who signed them.

=== Proofs of SPARQL answers over credentials <bg-zkrdf>

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

== What an accepted answer means <meaning>

Suppose the bank issues the applicant a credential that lists payments, each with the type
`ex:Payment`, an amount, a date and a status. The input dataset $D$ is the RDF merge @rdf11mt of the
graphs of the credentials the holder includes, as the default graph; the merge keeps each
credential's blank nodes apart. A proof method may instead put each credential in its own named
graph, and states its layout, because a query written for one does not match the other.

The proof fixes $D$ through a _dataset commitment_: a digest of the included credentials and of a
random salt that the holder chooses. It is binding if the hash is collision-resistant, so the holder
cannot later claim another $D$ for it. While the salt stays secret, it is also hiding: if the hash
is modelled as a random oracle, the commitment reveals nothing about $D$ to a computationally
bounded verifier.

=== Answers over the input dataset <answers>

Let $Q$ be the request's query. An _answer_ is a result that SPARQL 1.2 permits for $Q$ over $D$,
with nothing added or omitted @zksparqlspec: for `SELECT`, a solution sequence that $Q$ can produce
over $D$, with every solution and duplicate; for `ASK`, `true` exactly when the query pattern has a
solution over $D$; for `CONSTRUCT`, a graph that $Q$ can produce over $D$, compared up to
isomorphism. A result that exceeds the request's limits is rejected, never truncated. Without
`ORDER BY`, a `SELECT` result is compared as a multiset.

For some queries, such as those using `OFFSET` and `LIMIT` where `ORDER BY` does not fix the order,
`REDUCED`, `SAMPLE` or `GROUP_CONCAT`, SPARQL permits several results over the same $D$, and the
holder chooses one. A proof method may fix these choices, and a verifier can rely on that only
where the method publishes them. Our evaluator does not yet publish its choices, so a verifier can
rely on a unique result only for queries that leave SPARQL no choice.
// Source: ZK code landing addenda (10 October), item 3; a follow-up change documents the choices.
An answer does not say who signed $D$ (§#ref(<linkage>, supplement: none)).

=== Open-world and closed-world readings <readings>

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
corresponds to an answer to a monotone query over credentials the holder chose, read open-world:
each returned solution stays a solution whatever the holder left out. It says nothing about the
solutions not returned, and a verifier that counts the returned rows itself counts only what the
holder chose to show.

=== Who fixed the input <who-fixed>

A closed-world answer reads $D$ as complete, and someone chose $D$. If the request has no
`inputCommitment` (§#ref(<request>, supplement: none)), the input dataset is _chosen by the
holder_: the holder decided which credentials make up $D$ when it answered, and could have left one
out. Over an input dataset chosen by the holder, a closed-world answer covers only the included
credentials; a count, for instance, counts only those. A holder that reuses the commitment across
queries shows that the answers used the same input dataset, at the cost of linking the
presentations.

If the request has an `inputCommitment`, the input dataset is _agreed_: the verifier accepted that
dataset commitment before sending the request, and the presentation's commitment must equal it.
Agreement shows that two commitments are equal, not that $D$ is complete in the world; that depends
on why the verifier agreed (§#ref(<agreed-source>, supplement: none)). Agreeing to whatever
commitment the holder sends in the same exchange gains nothing over an input chosen by the holder.

Who fixed the input is separate from who signed it. With either kind of input, if the request names
trusted issuers, the proof shows that each credential in $D$ carries a signature from one of them.
Signatures show who made the statements in $D$, not that no other statement exists.

== Architecture <architecture>

@fig-architecture shows the three parties. The verifier service sends a query request and stores its
own copy. The holder service builds the input dataset from some of the holder's credentials,
evaluates the query, and returns an answer presentation: the result with one proof. A companion
specification @zksparqlspec defines both as RDF, serialised as JSON-LD @jsonld11. The answer
presentation is a verifiable presentation @vcdm2 that carries a proof in place of credentials,
except in the disclosed mode.

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
        1. In the hidden mode, check each signature against the trusted issuers' keys.
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
      *Outside the proof:* check the presentation against the stored request, the proof with the
      verification key that the request gives and, in the revealed mode, each signature; then mark
      the request as answered; then accept the result.
    ],
  )
#let arch-html = [
    #set align(left)
    / Issuers: Sign credentials, for example with `eddsa-rdfc-2022`. May vouch for a dataset
      commitment that a verifier agrees in advance.
    / Holder: Receives the query request. Inside the proof: (1) in the hidden mode, check each
      signature against the trusted issuers' keys; (2) build $D$ from exactly the signed data;
      (3) evaluate $Q$ over $D$ within the request's limits; (4) output the result, the dataset
      commitment and the request digest.
    / Verifier: Outside the proof: check the presentation against the stored request, the proof
      with the verification key that the request gives and, in the revealed mode, each signature;
      then mark the request as answered; then accept the result.
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
request digest, the SHA-256 hash of the request's canonical N-Quads under RDFC-1.0, and checks each
presentation against this stored copy, never against anything the holder returns.

The trust requirements name the issuers the verifier trusts; the one type defined so far lists their
verification methods and public keys rather than resolving them, so that both sides prove and verify
against the same keys. A trust requirement states the verifier's policy; it does
not show that the issuer authorised a key (§#ref(<bg-vc>, supplement: none)).

A proof method is a way of producing and checking the proof, named by an IRI, as a cryptosuite is for
Data Integrity proofs; a new version is a new method with a new IRI. Its evidence may be a
zero-knowledge proof, possibly interactive and designated-verifier as in QuickSilver @quicksilver21,
a proof that is not zero-knowledge, an attestation from a trusted execution environment (TEE), or
disclosed credentials. The verifier checks it with the verification key and parameters that its
request gives for the method, such as a zkVM image ID, never with a key from the presentation.

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
    [`trustedIssuers`], [Optional. Trust requirements: which issuers the verifier trusts. The defined
      type gives an issuer and its verification methods, each with its public key
      (`publicKeyMultibase`); types for trusted lists and other sources are reserved. Without trust
      requirements, no credential is checked against an issuer],
    [`cryptosuite`], [With trust requirements: the cryptosuites accepted for the issuers'
      signatures, such as `eddsa-rdfc-2022`],
    [`signatureMode`], [With trust requirements: the accepted signature modes, hidden, revealed or
      disclosed (§#ref(<modes>, supplement: none))],
    [`proofMethod`], [The accepted proof methods, in order of preference: each an IRI (`method`)
      with a verification key (`verificationKey`) and parameters],
    [`maxPresentationBytes`], [The largest presentation, in bytes],
    [`maxResultSize`], [The most solutions, or triples, in the result],
    [`challenge`, `domain`], [Fresh randomness, and the verifier's identifier],
    [`validFrom`, `validUntil`], [The period in which the verifier accepts a presentation],
  ),
  caption: [
    Members of a query request, by JSON-LD term, apart from its type and optional identifier. The
    request digest covers all of them.
  ],
) <request-table>
]

=== The answer presentation and its proof <linkage>

The answer presentation carries the request digest, the dataset commitment, the signature mode if
the request has trust requirements, and the result: a SPARQL Query Results JSON document for
`SELECT` and `ASK`, or a graph for `CONSTRUCT`. In the revealed mode, it also carries each credential's signature and signed message.
Its proof names a proof method by IRI, repeats the request's challenge and domain, and holds the
evidence. The statement's public inputs are the request digest, the dataset commitment, the
result, the signature mode and, in the revealed mode, the signed messages; a proof method may bind
them all by one digest. The proof shows that:

+ the dataset commitment fixes $D$;
+ if the request has trust requirements, $D$ is exactly the data that signatures from trusted
  issuers cover, in its signed lexical forms and with each credential's blank nodes kept apart, and
  in the hidden mode the proof also verifies those signatures (§#ref(<modes>, supplement: none));
+ the result is an answer for $Q$ over $D$ (§#ref(<answers>, supplement: none)).

By binding the request digest, the proof binds every member of the request, the agreed commitment
included: as long as SHA-256 is collision-resistant, a proof made for one request does not verify
for another, or for another verifier or validity period.

=== Signature modes <modes>

Each proof method states which signature modes it supports for each cryptosuite.

- In the _hidden_ mode, the proof shows that the holder knows a valid signature from a trusted issuer
  on every credential in $D$. The signatures, the signed messages and which key signed which
  credential are part of the witness, and stay hidden as far as the proof system is zero-knowledge
  and the public inputs do not disclose them (§#ref(<leakage>, supplement: none)).
- In the _revealed_ mode, the presentation carries each credential's signature and signed message,
  which the verifier checks itself. The proof then shows only that $D$ is exactly the data those
  messages cover and that the result is correct. This mode reduced the cycles of our guest program
  (§#ref(<cost>, supplement: none)) but discloses the signatures, the signed messages, the issuers'
  keys and the number of credentials (§#ref(<security>, supplement: none)).
- In the _disclosed_ mode, for proof methods whose evidence is disclosed credentials, the holder
  sends the credentials or presentations derived from them by a selective-disclosure cryptosuite.
  The verifier checks them, builds $D$ from what they disclose and evaluates $Q$ itself, seeing
  everything disclosed. An answer to a monotone query then stays true over the full credentials,
  but a closed-world answer covers only what was disclosed.

=== Verifier processing <validation>

The verifier service checks a presentation against the request it sent, in this order, and rejects
it at the first check that fails @zksparqlspec:

+ Before parsing, reject a presentation larger than `maxPresentationBytes`; then reject it unless it
  is a JSON-LD document that uses only the specification's terms.
+ Reject it unless its request digest is that of the stored request, the time is within the
  request's validity period, the proof's challenge and domain equal the request's, and no
  presentation with this challenge has been accepted.
+ Reject it unless it names one of the request's proof methods, whose entry gives the verification
  key and parameters.
+ If the request has an `inputCommitment`, reject it unless the presentation's commitment equals it.
+ If the request has trust requirements, reject a signature mode that the request does not list. In
  the revealed mode, verify each signature under a trusted issuer's key and a listed cryptosuite.
+ Reject a result of the wrong form for the query, or larger than `maxResultSize`.
+ Verify the proof against the statement computed from the stored request and the presentation.

To accept at most one presentation per challenge, the verifier service then marks the request as
answered, in one atomic step that fails if it is already marked, and only then accepts. Checking
first stops a malformed presentation from using up a legitimate request, and the atomic step makes a
replay, or the second of two concurrent presentations, fail. Sending a presentation discloses its
answer, even if the verifier rejects it.

== Revealing less <minimize>

A zero-knowledge proof hides its witness, yet the verifier knows some witness values as soon as it
has the result. A proof method may make public any value that the verifier can compute from its
stored request and the result alone; no request member permits disclosing more. This public-input
rule extends zkRDF's disclosure of query constants and projected terms @braun26. Our argument for it
concerns disclosure only: such a value tells the verifier nothing that its request and the result do
not. Whether it also reduces proving cost is not established; our pilot is too small to show a
saving (§#ref(<pilot-evidence>, supplement: none)).

=== The public-input rule <rule>

Suppose the lender asks `SELECT ?p WHERE { ?p a ex:Payment . ?p ex:paymentStatus ex:Settled . }` and
receives one row, in which `?p` is `ex:pay1`. Substituting the row into the two triple patterns
gives `ex:pay1 a ex:Payment` and `ex:pay1 ex:paymentStatus ex:Settled`, which the verifier can
compute, so a proof method may make them public inputs. The proof must still show that each triple
supplied as a public input is in $D$ and signed: making a triple public removes its secrecy, not the
need to authenticate it. Unless the cryptosuite supports selective disclosure, the signature check
still covers the whole credential.

=== Why the rule needs care <counterexamples>

In queries other than a single basic graph pattern, a returned row may not determine which triples
matched:

- *`OPTIONAL`.* In `?p a ex:Payment OPTIONAL { ?p ex:returnReason ?r }`, a row with `?r` unbound
  shows that no return-reason triple for `?p` matched; no triple stands for that absence.
- *`UNION`.* In `{ ?p ex:paymentStatus ex:Returned } UNION { ?p ex:paymentStatus ex:Reversed }`, a
  row does not show which branch matched, so the matched triple is not computable.
- *Value equality.* `FILTER(?a = 1250)` holds for both `"1250.00"^^xsd:decimal` and
  `"1250.0"^^xsd:decimal`, so a filter does not fix the term of an unprojected variable.
- *Omission.* Triples of the returned rows say nothing about rows not returned.

=== When the values may be disclosed <release>

A method with several rounds could send such a value before the result, and an exchange that then
aborted would have disclosed facts that no answer did. We propose that a party receive these public inputs no earlier than the
answer from which it can compute them. The prototype does not enforce this condition.

=== What the verifier still learns <leakage>

Beyond its result, an accepted answer reveals:

- the dataset commitment, which links presentations if the holder reuses a salt over the same
  credentials; an agreed commitment links them by design, to the verifier that agreed it;
- the proof method, whose capacity limits bound the size of the input;
- the issuers: in the hidden mode, only that a trusted issuer signed each credential, which
  identifies the issuer if the request trusts only one; in the revealed mode, each key and the
  number of credentials;
- the size of the result, such as how many payments matched;
// Source: security-table-addenda.md, item 4a (ZK code landing, 10 October 2026): succinct receipts
// expose the recursion control ID, which can reveal part of the execution's shape
// (zk/sparql-evaluator/README.md); RISC Zero's zero-knowledge is its claim, not established.
- from a RISC Zero receipt, part of the shape of the execution; the zero-knowledge of these receipts
  is RISC Zero's claim, not an established property;
- where SPARQL permits several results, the holder's choice (§#ref(<answers>, supplement: none)):
  for `SELECT ?x WHERE { VALUES ?x { 0 1 } } LIMIT 1`, the returned value can encode one bit of the
  holder's choosing, a covert channel that zero-knowledge does not prevent because the result is
  public. It matters when the holder does not fully trust its holder service.

A sequence of queries can also reveal together what no single query reveals.

== Cryptosuites and signature modes <cryptosuites>

=== Existing cryptosuites <existing-suites>

Two RDF cryptosuites sign the same structure as `eddsa-rdfc-2022` (§#ref(<bg-vc>, supplement: none)),
which every zkVM experiment with signature checks in §#ref(<evidence>, supplement: none) uses.
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

=== Our Merkle-based cryptosuites <merkle-suites>

// Source: the cryptosuite draft (sparq-org/sparq#6789) specifies all three members below, and its
// Poseidon2 member signs P(3, suite, salt, n, root, c1, c0), which includes the suite identifier.
We propose Merkle-based cryptosuites designed for proving @merklesuites. The issuer builds a Merkle
tree with one leaf per canonical quad and signs a digest of the suite identifier, a fresh 32-byte
salt, the number of quads, the Merkle root and the digest of the canonical proof configuration.
Each leaf encodes the typed terms of its quad, and a literal's encoding carries a comparison key: an
order-preserving encoding of its value.

This has three effects. First, a proof need not repeat canonicalisation: it rebuilds the signed
tree from the credential's quads, which took fewer cycles in our measurements
(§#ref(<cost>, supplement: none)). Second, a proof compares values without parsing lexical forms:
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

=== Security and disclosure <security>

@security-table compares what the verifier learns in each signature mode, and supplementary
@assumptions-table lists the assumptions behind each signature scheme and proof system. In the
hidden mode, what the verifier learns depends on the public inputs and on the zero-knowledge of the
proof system, not on the cryptosuite. In the revealed mode, a credential's repeated signature links
its presentations, and with an RDFC cryptosuite the verifier can also confirm a guessed credential.
Whether a quantum adversary can forge a signature depends on the cryptosuite, not on the mode.
Outside the disclosed mode, the proof system's knowledge soundness must also hold: a quantum
adversary could forge UltraHonk proofs, even ones made earlier, whereas we infer that Shor's
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
    through a value the verifier receives, without breaking any assumption.
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

Our services encode query requests in binary, not as the JSON-LD of
§#ref(<architecture>, supplement: none), and compute the request digest over that encoding. Our Noir circuits publish no dataset commitment, so they cannot
yet produce the specification's presentations. In the zkVM, the revealed mode and our Merkle-based
cryptosuite exist only in a later build, which we executed without proving.

Our guest program runs a SPARQL evaluator inside the RISC Zero zkVM and writes the result, the
dataset commitment and whether the input was agreed to the journal. A second build adds signature
checks in front of the same evaluator: in the hidden mode, it verifies each credential's
`eddsa-rdfc-2022` signature under a trusted issuer's key and evaluates the query over the signed
canonical N-Quads. Our holder and verifier services wrap both builds with query requests and the
checks of §#ref(<validation>, supplement: none). The journal binds the stored request through a
digest (§#ref(<capabilities>, supplement: none)). We have not confirmed that the verifier service
performs its checks in exactly that order. Separately, we built two Noir circuits
(§#ref(<pilot-evidence>, supplement: none)).

We used only synthetic credentials signed with test keys. We generated all receipts with development
mode disabled and verified them against the expected guest image ID. For experiments without † in
@evidence-table, we recomputed hashes of source files, guest binaries and receipts. We compared the
hashes and recorded test outcomes with the archived records. We did not verify the proofs again. The
supplementary material lists the test cases and records (§#ref(<supp-records>, supplement: none))
and the software behind them (§#ref(<repro>, supplement: none)).

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
    verified. Each zkVM experiment with signature checks in this table used one credential per test
    case; §#ref(<cost>, supplement: none) reports cycle counts for up to four.
    Cases with an input chosen by the holder have two fewer negative tests, because swapping the
    agreed commitment does not apply to them. † No internal check (§#ref(<prototype>, supplement: none)). Counts
    are per experiment and are never added across rows.
  ],
) <evidence-table>
]

=== Which queries the evaluator answers <expressivity>

Our evaluator implements SPARQL 1.1 @sparql11, without the additions of SPARQL 1.2 such as triple
terms.

_Proved._ With signature checks, our proofs cover `SELECT` results that keep duplicate rows, true and
false `ASK`, and `CONSTRUCT`, each over one credential and a basic graph pattern
(§#ref(<proved>, supplement: none)); the false `ASK` is a closed-world answer. Without signature
checks, they also cover `UNION`, `VALUES`, `OPTIONAL`, `MINUS`, `EXISTS`, `NOT EXISTS`, a counting
subquery, property paths, `ORDER BY` with `LIMIT`, `FROM NAMED` and `DESCRIBE`
(§#ref(<supp-exact>, supplement: none)).

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
guest program rejected every case that the evaluator rejected, except in some runs over four
credentials that reached the session limit first. The evaluator admitted `DESCRIBE`, `FROM` and
`FROM NAMED`, but with signature checks the credentials form the default graph, and our prover and
verifier reject these three before proving and before verification. Our services also reject a `SELECT` query with a
top-level `ORDER BY`, so they return `SELECT` results only as multisets. The build with signature
checks accepts at most four credentials of at most 128 triples each, and 256 triples in total.
// Sources: the sweep audit's rejected_runs, one rejection_cause per rejected run (guest_abort or
// session_limit), and /reruns (zkexec.sweep_reruns); the DESCRIBE/FROM answer from the ZK code
// landing thread (10 October 2026); the services' shared admission check, which rejects DESCRIBE
// and a SELECT whose result keeps the order of a top-level ORDER BY
// (zk/sparql-evaluator/host/src/vcq.rs, checked_admission); and the capacity constants
// MAX_CREDENTIALS, MAX_DOCUMENT_QUADS and MAX_TOTAL_QUADS in
// zk/sparql-evaluator/model/src/authenticated_rdf.rs at the sweep's source commit.

=== Proved answers <proved>

Without signature checks, the evaluator produced #headline("zkvcq.exact_hosted_receipts") receipts
in a CI build and #headline("zkvcq.adapter_receipts") with our services (@evidence-table), which
show evaluation bound to a request, not who issued the data. Our verifier service rejected
#headline("zkvcq.adapter_row_bound_rejected") receipt whose result exceeded the row limit: a valid
proof is necessary, not sufficient, for acceptance.

With signature checks and our services, the evaluator produced one receipt per test case, each over
one credential. For the payment question, the credential lists three settled payments and is signed
with the public RFC 8032 test key in place of a bank's key. With both kinds of input, the journal
reports `false`, and the verifier accepted the answer. Each accepted receipt came with negative
tests, each with its expected outcome (§#ref(<supp-records>, supplement: none)). We report no
timings for these runs.

=== The public-triple circuit pilot <pilot-evidence>

Our two Noir circuits prove, for `SELECT DISTINCT` queries over one basic graph pattern, that each
returned row is a solution, not that none is missing; such a proof gives an answer only where those
rows form a permitted result, as when they reach the query's `LIMIT`. In the baseline circuit the
matched triple is part of the witness, and in the public-triple circuit a public input. Both check
Schnorr signatures over their own commitment format and credential status against a snapshot that
the verifier accepted. We produced #headline("zkvcq.pp_genuine_proofs") UltraHonk proofs,
alternating the two circuits over the same query, rows, credentials and capacity, and all
#headline("zkvcq.pp_tamper_controls") tampering and #headline("zkvcq.pp_replay_controls") replay
tests were rejected. Every proof was #headline("zkvcq.pp_proof_bytes") bytes with either circuit,
and both circuits made the same numbers of signature and membership checks, so the comparison
isolates the effect of making the triple public. Preliminary: #headline("zkvcq.pp_n_per_cell") timed
runs per circuit and capacity, on a shared machine (@pilot-table). Prove and verify times differed
little relative to their totals; the runs are too few to show a saving, and we did not measure
memory or larger inputs.

=== Cost by signature mode and cryptosuite <cost>

We counted cycles with the RISC Zero executor, which runs the guest program without proving, using
the later build with signature checks; a cycle count is not a proving time. The executor also splits
each execution into segments, which a prover proves separately.

We ran #headline("zkexec.main_queries") queries over the payment credential: the payment `ASK`, a
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
    #headline("zkexec.main_queries") queries: its cycles, without the prover's overhead and
    padding per segment, and its segments. The counts are deterministic for a given guest program
    and input; they are not proving times.
  ],
) <cost-table>
]

An instrumented build of the same guest program reads the cycle counter at the end of each phase,
so its counts include the instrumentation. @phase-table shows the phases for the payment `ASK` over
one credential. In the hidden mode, verifying the Ed25519 signature was the largest phase with both
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
      [Checking the request, the trusted issuers' keys and the input sizes],
        ..phase-cells("request"),
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

The sweep of §#ref(<expressivity>, supplement: none) ran in the same configurations over generated
credentials of #headline("zkexec.sweep_statements_small") and
#headline("zkexec.sweep_statements_large") triples each (§#ref(<supp-cost>, supplement: none)). Our
prover stops an execution that reaches a session limit of #mcycles("zkexec.sweep_session_limit")
million cycles, and then produces no proof. With `eddsa-rdfc-2022` in the hidden mode, four
credentials of #headline("zkexec.sweep_statements_large") triples exceeded this limit in every case
that the evaluator admitted. In every other configuration, including the revealed mode and the
Merkle-based cryptosuite over the same credentials, every admitted case completed within it.
// Source: paper-executor-sweep-audit.json /configurations/*/rejected_runs, one rejection_cause per
// rejected run (guest_abort or session_limit), and /reruns (zkexec.sweep_reruns). The audit records
// no count per cause, so the sentence on rejected cases in the expressivity subsection gives none.

#todo-results[Proving time, memory and receipt size for each configuration and query, and the
issuer, holder and verifier costs: signing and tree construction, and verification time split into
receipt, signature and request checks. These need a dedicated proving run with development mode
disabled on one machine.]

#todo-results[A comparison, over the same queries, with re-evaluation over disclosed
credentials, selective disclosure with re-evaluation and, where built, designated-verifier proofs
and TEE attestation.]

== Related work <related>

_Zero-knowledge SPARQL over credentials._ zk-SPARQL @yamamotozksparql answers `SELECT` and `ASK`
queries with one basic graph pattern over BBS+-signed credentials, with `FILTER`s only on disclosed
variables. It, zkRDF and Wright's zkVM prototype (§#ref(<bg-zkrdf>, supplement: none)) each fix one
signature scheme, BBS+ or Ed25519; our requests choose among cryptosuites and signature modes. Wright's prototype took about 7.5 minutes for `SELECT *` over one credential of 23
triples (§5 of @wright25dc), and zkRDF reports proving three orders of magnitude faster on three
small queries, under BBS+ signatures (§5.2 of @braun26); our proving times are pending. We add
closed-world answers over a stated input of issuer-signed credentials, and request terms that say
who fixed that input.
// TODO(citation): status of the zksparql.org preprint (author to confirm)

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

== Discussion and limitations <discussion>

=== Where agreed inputs could come from <agreed-source>

A closed-world answer over an agreed input is evidence about all the relevant records only if a
party the verifier trusts fixed an input that covers them, an assumption we do not discharge. For the payment question, the bank could sign a dataset commitment that covers the credentials
it issued to the applicant for a period, or the verifier could agree a commitment in an earlier
exchange in which it learned which credentials the dataset holds. We do not claim that such
infrastructure exists. Without it, a deployment can offer only inputs chosen by the holder.

=== Limitations <limitations>

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

== Conclusion <conclusion>

A holder can answer a verifier's SPARQL query over credentials without handing them over. One proof
can show that the answer is a result SPARQL 1.2 permits over a stated input dataset, closed-world
answers included, and that trusted issuers signed every credential in it. Closed-world answers are
only as meaningful as the verifier's reason to believe the input complete, so the request states who
fixed it; it also chooses the cryptosuites and a signature mode that trades proving cost against
disclosure. A prototype SPARQL evaluator running in a zkVM has proved `SELECT`, true and false `ASK`
and `CONSTRUCT` answers with signature checks, including an answer that no payment was returned,
each over one synthetic credential; a later build evaluated queries with `OPTIONAL`, `MINUS`,
aggregates, subqueries and property paths without proving. Its cycle counts show signature
verification as the largest step for the payment question, and fewer cycles when the holder reveals
the signatures or the issuer uses a Merkle-based cryptosuite. Realistic credentials, proving times
and an external audit remain to be done.

#pagebreak(weak: true)
#heading(level: 2, numbering: none)[References]
// The bibliography is read from the YAML file so that the double-blind build (`anon`) can replace
// the entries for our own companion drafts with anonymised ones: no author, repository or URL.
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
#bibliography(
  bytes(yaml.encode(bib-entries)),
  style: "springer-lecture-notes-in-computer-science",
  title: none,
)

// ---------------------------------------------------------------------------------------------
// Appendix: at most two LNCS pages after the references. It states what an accepted presentation
// shows and the assumptions behind that argument. The test cases behind the evidence table are in
// the supplementary material, which follows it.
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

Tests support A2 and A4 (supplementary §#ref(<supp-records>, supplement: none)); A1, A3 and A5
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
(§#ref(<capabilities>, supplement: none)), so under A3 it also binds the challenge, the verifier's
identifier and the validity period.

*Public inputs computable from the result.* Suppose the pattern of $Q$ is a single basic graph
pattern without blank nodes, and a returned row binds each of its variables to an IRI or a literal.
The row is then a solution of the pattern over $D$ exactly when every triple obtained by
substituting the row into the pattern is in the default graph of $D$. A circuit that takes these
triples as public inputs, and checks that each is in $D$ and signed, therefore shows the same
statement as one that finds them in its witness. It discloses nothing that the verifier cannot
compute from the row. The baseline and public-triple circuits of
§#ref(<pilot-evidence>, supplement: none) differ in exactly this way.

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

== Security assumptions <supp-security>

@assumptions-table lists the assumptions behind the signature schemes and proof systems of
§#ref(<security>, supplement: none), and whether each property holds against a classical and a
quantum adversary.

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
        table.cell(fill: luma(235))[*Component (used by)*], table.cell(fill: luma(235))[*Assumption*],
        table.cell(fill: luma(235))[*Classical adversary*],
        table.cell(fill: luma(235))[*Quantum adversary*],
      ),
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
      [RISC Zero succinct receipt (STARK)], [Random oracle model and the Toy Problem conjecture
        @risc0sec], [Knowledge-sound, under the conjecture],
        [Inferred: Shor's algorithm does not apply],
      [UltraHonk with zero-knowledge (KZG over BN254)],
        [Pairings over BN254 (q-SDH, algebraic group model); random oracle; trusted setup],
        [Knowledge-sound], [Not knowledge-sound, also for earlier proofs],
    )
  },
  caption: [
    Assumptions behind each signature scheme and proof system. MLWE and SelfTargetMSIS are the
    module learning-with-errors and self-target module short-integer-solution problems on
    lattices @fips204. q-SDH is the q-strong Diffie–Hellman assumption in pairing groups. KZG is
    the polynomial commitment of Kate, Zaverucha and Goldberg, which needs a trusted setup. The
    algebraic group model requires an adversary to give, with each group element it outputs,
    coefficients that express it as a combination of group elements it has received. RISC Zero's
    security model bases the knowledge soundness of its receipts on the random oracle model and
    the Toy Problem conjecture @risc0sec.
  ],
) <assumptions-table>
]

== Proof methods and request formats <capabilities>

The verifier service lists the proof methods it accepts in each query request, with a verification
key for each from its own configuration (§#ref(<request>, supplement: none)). @methods-table shows
what our proof methods support, and §#ref(<repro>, supplement: none) gives their identifiers. Our
services reject a request outside these limits before proving and before marking it as answered.
Besides the queries of §#ref(<expressivity>, supplement: none), they reject a request that requires
credential status or holder binding.

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
      [Answers], [`SELECT`, `ASK` and `CONSTRUCT` results], [Same],
        [Only that each returned row of a `SELECT DISTINCT` query over one basic graph pattern is a
        solution],
      [Inputs], [Chosen by the holder or agreed], [Same], [Chosen by the holder],
      [Input dataset], [Parsed from the committed N-Quads], [RDF merge of the signed canonical
        N-Quads], [Credentials in the circuits' own commitment format],
      [Issuer checks], [None: the request has no trust requirements], [`eddsa-rdfc-2022`, in the hidden
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
nodes in the input and the result. Behind our services, both builds use the graph-result format,
with the exclusions of §#ref(<expressivity>, supplement: none). The services encode the query request
in binary rather than as the specification's JSON-LD, compute the request digest over that
encoding, and store times as Unix seconds. Their requests list issuer keys, the prototype's form of
an `IssuerKeys` trust requirement, and name the verifier in an audience field, the counterpart of
`domain`.

*How the stored request reaches the proof.* The holder service gives the guest program the query,
whether the input is agreed, any agreed commitment and the evaluation limits. It also gives a SHA-256 hash of the
stored request and its proof-method entry. The guest program writes a digest of these values to the
journal, and the verifier service recomputes it from its own copies. The journal therefore binds
every member of the stored request, including the challenge, the audience and the validity period.
With signature checks, the proof-method entry contains a digest of the request's issuer keys and the
evaluation limits, so changing any key, issuer or verification method changes the entry.

*Dataset commitments.* Without signature checks, the evaluator commits with SHA-256 to a format tag,
its capacity limits, its policy for `DESCRIBE`, the salt, the names of any named graphs and the
exact bytes of the N-Triples or N-Quads input. It does not canonicalise the input, so equivalent
serialisations give different commitments. With signature checks, the commitment covers the digest
of the request's issuer keys and evaluation limits, the salt and the number of credentials. For each
credential, it also covers the signed hashes of the canonical document and proof configuration. A
commitment agreed under one list of issuer keys therefore cannot serve another.

== Experiment records <supp-records>

@test-cases-table shows the answers that each zkVM experiment of @evidence-table proved, by kind
of input. The inputs were small synthetic graphs or, with signature checks, the W3C test vector
for `eddsa-rdfc-2022` and the payment credential of §#ref(<proved>, supplement: none).
@tests-table lists the tests and executions without proving, by run, and @controls-table the
negative tests.

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
        [Answer], [CI build], [Our services], [First case], [Payment †], [Remaining †],
      ),
      [`SELECT`, duplicate rows], [H], [H, A], [A], [–], [H],
      [`SELECT`, `ORDER BY` and `COUNT`], [A], [–], [–], [–], [–],
      [`ASK`, true], [–], [A], [–], [–], [A],
      [`ASK`, false], [H, A], [H], [–], [H, A], [H],
      [`CONSTRUCT`], [A], [H, A], [–], [–], [H, A],
      [`DESCRIBE`], [H], [–], [–], [–], [–],
      [`SELECT` over the row limit, rejected], [–], [A], [–], [–], [A],
      [Negative tests], [In its tests], [#headline("zkvcq.adapter_controls") in total],
        [#headline("zkvcq.vcqg_controls")],
        [#headline("zkvcq.vcqp_controls") A, #headline("zkvcq.vcqph_controls") H],
        [#headline("zkvcq.ci_asktva_controls") A, #headline("zkvcq.ci_askfhd_controls") H each],
    )
  },
  caption: [
    Answers proved, by kind of input: H chosen by the holder, A agreed; an entry may stand for
    several receipts. The columns are the rows of @evidence-table with receipts: the
    evaluator's CI build, the evaluator with our services and, with signature checks, the first test
    case, the payment question and the remaining test cases. † No internal check.
  ],
) <test-cases-table>
]

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
      [Runs: test cases × request formats × kinds of input], [#headline("zkvcq.exact_replay_jobs")],
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
  rows and an unbound value (input chosen by the holder); a `SELECT` with a counting subquery,
  `OPTIONAL`, `MINUS`, `EXISTS`, property paths, `ORDER BY` and `LIMIT` (agreed input); and a false
  `ASK` whose branches use property paths, `MINUS`, `NOT EXISTS` and numeric edge cases (agreed);
- in the named-graph format, a false `ASK` over graphs named with `FROM NAMED` (chosen by the
  holder), and a `SELECT` that counts the matches in each named graph, an empty one included
  (agreed);
- in the graph-result format, a `SELECT` whose result keeps duplicate rows, unbound values and a
  blank node shared across rows (chosen by the holder); a `DESCRIBE` (chosen by the holder); and a
  `CONSTRUCT` that creates fresh blank nodes (agreed).

The `DESCRIBE` answer contained every default-graph triple whose subject was the described resource
and, recursively, every triple whose subject was a blank node reached as an object. Each test
compares the journal with the result of the evaluator run outside the zkVM. Some also check that
verification fails after a change to the query, the request's limits, whether the input is agreed,
the agreed commitment or one journal byte. They also check that it fails under another image ID and for a fake
receipt, and that a second verification of the same receipt fails as a replay. The same job ran
#headline("zkvcq.exact_replay_cases") test cases outside the zkVM, in
#headline("zkvcq.exact_replay_jobs") runs across request formats and kinds of input. Each run gave its
expected outcome, and none produced a proof.

Our services ran a separately built guest program of the same evaluator on another machine
(§#ref(<repro>, supplement: none)). Their test cases query a small synthetic graph in which triples
share an object, so the `SELECT` result has duplicate rows. Of the
#headline("zkvcq.adapter_receipts") receipts, the verifier accepted
#headline("zkvcq.adapter_accepted"), covering `SELECT`, true and false `ASK` and `CONSTRUCT` with
both kinds of input, and rejected the one whose result exceeded the row limit. The
#headline("zkvcq.adapter_controls") negative tests exercised each verifier check (@controls-table).

=== The evaluator with signature checks <supp-signed>

@tests-table lists the runs that tested this build without proving. Its
#headline("zkvcq.v5_auth_tests_passed") tests outside the zkVM use the W3C test vector and synthetic
keys. They check that valid signatures under the wrong issuer, verification method, proof purpose or
cryptosuite are rejected, and that signed lexical forms and each credential's blank-node scope are
kept. They also check that the request's issuer keys are bound into the request and the commitment,
and they evaluate `SELECT`, `ASK` and `CONSTRUCT` with both kinds of input. Executed without
proving, the guest program accepted the test vector, with both kinds of input, in
#headline("zkvcq.v5g_direct_v5_positive") executions. It rejected
#headline("zkvcq.v5g_direct_v5_aborts") altered inputs, each with its expected error: forged,
spliced or unauthorised credentials, and input that was not in canonical form or used another
encoding. A positive control preceded each group of negative tests. The evaluator without signature
checks rejected input meant for this build and still accepted its own. The tests of our services
check that changing a listed key, issuer, verification method or limit changes the proof-method
entry's digest and the digest in the journal, while reordering the keys does not. They also check
that a mismatched proof-method entry, guest program or image ID, and a fake or foreign receipt, are
rejected before the request is marked as answered.

With our services, this build produced one receipt per test case, each over a single credential, in
three experiments with different machines and guest binaries (§#ref(<repro>, supplement: none)):

- a `SELECT` over an agreed input, the W3C test vector for `eddsa-rdfc-2022`, whose result keeps a
  duplicated row (#raw("?" + headline("zkvcq.vcqg_result_variable")) is
  #raw(headline("zkvcq.vcqg_result_row1")) twice);
- the payment question, over the payment credential of §#ref(<proved>, supplement: none),
  with both kinds of input;
- the remaining test cases, over the W3C test vector: `SELECT`, false `ASK` and `CONSTRUCT` over an
  input chosen by the holder; true `ASK` and `CONSTRUCT` over an agreed input; and a `SELECT` whose
  result exceeds the row limit. The verifier accepted every answer except the last, which it
  rejected before marking the request as answered.

For the first test case, the internal check covered #headline("zkvcq.vcqg_source_files_verified")
source files, unchanged during the run, and #headline("zkvcq.vcqg_archive_verified_files") archived
files.

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
      [Another verifier identifier (audience) presented],
        [#headline("zkvcq.ctl_wrong_verifier_audience")], [Yes],
      [Stored verifier identifier changed to the presented one],
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
      table.cell(colspan: 3)[_Who fixed the input, and the agreed commitment_],
      [Proof over an input chosen by the holder, for a request with an agreed input],
        [#headline("zkvcq.ctl_scope_holder_to_agreed")], [Receipts with H],
      [Proof over an agreed input, for a request without one],
        [#headline("zkvcq.ctl_scope_agreed_to_holder")], [Receipts with A],
      [Another agreed commitment], [#headline("zkvcq.ctl_wrong_agreed_anchor")],
        [Receipts with A],
      [Agreed commitment computed with another salt], [–], [Receipts with A],
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
        [#headline("zkvcq.vcqg_controls") per receipt with A, #headline("zkvcq.vcqph_controls")
          per receipt with H],
    )
  },
  caption: [
    Negative tests, by what they change. Without signature checks: records across the
    #headline("zkvcq.adapter_accepted") accepted receipts of the evaluator with our services. With
    signature checks: tests run with each accepted receipt; "Yes" means with every receipt. H: input
    chosen by the holder; A: agreed input.
  ],
) <controls-table>
]

== The public-triple circuit pilot <supp-pilot>

The baseline and public-triple circuits prove, for `SELECT DISTINCT` queries over one basic graph
pattern without `FILTER`, that each returned row is a solution, and reject a result that repeats a
row. Each exists for one
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
credentials and, in the sweep, each credential size. Every request used an input chosen by the
holder, and the RFC 8032 test key signed every credential. For the #headline("zkexec.main_queries") queries, we
executed #headline("zkexec.main_runs") runs over the payment credential and, with four credentials,
copies issued to other customers. For the sweep, we executed #headline("zkexec.sweep_runs") runs
over generated credentials. Each describes a person with typed literals, a chain of acquaintances
and one blank node, padded with further triples to the stated size. The counts of the two sets are
never added. The executor runs had no internal check (§#ref(<prototype>, supplement: none)).

The executor runs used our prover's session limit (§#ref(<cost>, supplement: none)), and an
admitted case that reached it does not count as completed. The cycle counts, like the limit, cover
the guest program's own cycles, without the overhead and padding that the prover adds to each
segment.

For @phase-table, a separate build of the same guest program, used only for measurement, reads the
cycle counter after decoding its input and at the end of each phase. It passes the readings to the
host, not to the journal. A phase that runs once per credential is summed over the credentials, and
the readings include the instrumentation.

§#ref(<expressivity>, supplement: none) lists the sweep's query cases by feature, with the cases
that the evaluator admitted and rejected. The basic graph patterns are stars and chains of several
sizes, and further cases probe features that the evaluator should reject, or patterns that cannot
match credential data. We executed the #headline("zkexec.sweep_reruns") rejected runs over four
credentials again, keeping the executor's error: each ended with the guest program's rejection or
at the session limit. @sweep-table gives the counts and cycles per configuration.

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
// [OPUS-5.5] §7.2 says that, with one credential, the evaluator admitted the same number of cases
// in every configuration and the guest program completed all of them; these asserts keep it true.
#for suite in ("rdfc", "merkle") {
  for mode in ("hidden", "revealed") {
    for size in ("s32", "s64") {
      let stem = "zkexec.sweep_" + suite + "_" + mode + "_n1_" + size
      assert(headline(stem + "_admitted") == headline("zkexec.sweep_rdfc_hidden_n1_s32_admitted"))
      assert(headline(stem + "_completed") == headline(stem + "_admitted"))
    }
  }
}
#figure(
  {
    set text(size: 0.8em)
    set par(justify: false)
    table(
      columns: (auto, auto, auto, auto, auto, 1fr, auto),
      align: (left, right, right, right, right, right, right),
      table.header[Credentials][Triples each][Admitted][Rejected][Completed][Cycles,
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
It proves only that each returned solution is a solution, as zkRDF does, for `SELECT` and
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
holder do not yet establish holder binding and are off by default. The design cannot prove
closed-world answers, checks the standard cryptosuites only outside the circuit, and hides every
term that an operator uses. Complete results over the input dataset, signature checks inside the
proof and the public-input rule address these limits.

== Combining BBS+ proofs with circuits <composition>

BBS+ is an earlier variant of the BBS signature scheme @bbs that `bbs-2023` uses. Its signatures
have a different form, so the experimental parts below do not implement `bbs-2023`.

A tempting design verifies BBS+-signed credentials with BBS+ proofs, disclosing public terms, and
passes hidden values to a circuit for conditions that BBS+ proofs cannot express. A shared challenge
does not bind the two proofs: it shows that both belong to one exchange, not that they concern the
same value. One way to bind them is for both proofs to show that they use the same committed hidden
value. This needs one encoding of bytes into field elements, range checks where the fields differ,
and domain separation. It also needs an argument that knowledge soundness holds for the combination.
We have not built this binding.

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
challenge, replayed it, revoked the credential or changed the status epoch. These are neither
complete results over an input dataset nor combined proofs. A comparison of BBS+ proofs, circuits and zkVMs would need methods that
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
      [Evaluator with our services], [#repo-id[`urn:sparq:vcq:method:risc0-exact`], 3],
        [#short-id("zkvcq.adapter_source_commit")], [#short-id("zkvcq.adapter_guest_sha256")],
        [EC2 instance],
      [With signature checks: tests], [–], [#short-id("zkvcq.v5_source_commit")], [–],
        [EC2 instance],
      [With signature checks: executed directly], [–], [#short-id("zkvcq.v5g_source_commit")],
        [#short-id("zkvcq.v5g_guest_sha256")], [EC2 instance],
      [With signature checks and our services: tests; first test case],
        [#repo-id[`urn:sparq:vcq:method:risc0-authenticated-rdf`], 5],
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
    Software behind each experiment. #if anon [For double-blind review, we withhold the proof-method
    identifiers, commits, digests of guest binaries and image IDs.] else [Commits and SHA-256
    digests of guest binaries show their first twelve hexadecimal digits; the RISC Zero image ID is
    computed from the guest binary.] The zkVM experiments used the prover
    #raw(headline("zkvcq.exact_r0vm_version")), and the circuits were compiled with
    `nargo 1.0.0-beta.21`. † No internal check. ‡ The RISC Zero image ID, not the digest of the
    guest binary.
  ],
) <artifact-table>
]
