// [OPUS-5.5] Supplementary material of the ESWC 2027 submission (zksparql-eswc.typ). Its sections
// S1 to S8 and Tables S1 to S9 are the "Supplementary material" part of zksparql-architecture.typ,
// with every reference to the paper rewritten to the ESWC variant's numbering (paper-numbers
// below). The language guide (zksparql-language-guide.md) binds this file as it binds the paper.
// Every number comes from paper-evidence.json through headline or ev, directly or through the
// shared display helpers of _lib/zksparql.typ; none is typed. Plain A4 layout: ESWC sets no format
// for supplementary material.
//
// Build: typst compile --root site --input data="$(cat site/src/data/paper-evidence.json)" \
//   site/papers/zksparql-eswc-supplement.typ out.pdf
// Switches, as in zksparql-eswc.typ (override with --input name=value):
//   anon       "true" (default): double-blind submission, which withholds the values that identify
//              our repository; "false": camera-ready.
//   coauthors  author list of the non-anonymous build: "wright" or "wright-braun".
//   opening    accepted for symmetry with the paper; this document does not depend on it.

#let opening = sys.inputs.at("opening", default: "credentials")
#let coauthors = sys.inputs.at("coauthors", default: "wright")
#let anon = sys.inputs.at("anon", default: "true") == "true"

#assert(opening in ("credentials", "agents"), message: "opening must be credentials or agents")
#assert(coauthors in ("wright", "wright-braun"), message: "coauthors must be wright or wright-braun")

#import "_lib/bench.typ": headline, ev
#import "_lib/zksparql.typ": pilot, mcycles
#import "_lib/zksparql.typ" as zks
#let repo-id(body) = zks.repo-id(anon, body)
#let short-id(key) = zks.short-id(anon, key)

// Section and table numbers of the paper (zksparql-eswc.typ) that this document cites. A separate
// document cannot reference the paper's labels, so these follow the paper by hand: update them
// whenever the paper's sections or tables are renumbered. Keys are the paper's labels.
#let paper-numbers = (
  request: "§4.1",
  security: "§6.3",
  prototype: "§7.1",
  expressivity: "§7.2",
  proved: "§7.3",
  pilot-evidence: "§7.3",
  cost: "§7.4",
  evidence-table: "Table 3",
  phase-table: "Table 5",
)
#let paper(key) = [#paper-numbers.at(key) of the paper]

#let title = [Answers Instead of Documents: Private SPARQL Answers over Verifiable Credentials]
#let author-names = if anon {
  none
} else if coauthors == "wright-braun" {
  "Jesse Wright and Christoph H.-J. Braun"
} else {
  "Jesse Wright"
}

#set document(
  title: "Supplementary material: Answers Instead of Documents",
  author: if anon { () } else { author-names },
)
#set page(paper: "a4", numbering: "1")
#set text(size: 11pt)
#set par(justify: true)
// Figures may break across pages by default; short tables that must stay with their caption opt
// out locally with a scoped `set block(breakable: false)`.
#show figure: set block(breakable: true)
// Sections S1, S2, ... and subsections S1.1, ...; tables S1, S2, ...
#set heading(numbering: (..n) => "S" + numbering("1.1.", ..n.pos()))
#set figure(numbering: n => "S" + str(n))
// A reference to a heading drops the numbering's trailing dot, so that "§S3.1" reads cleanly.
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

#align(center)[
  #text(size: 15pt, weight: "bold")[Supplementary material]
  #v(0.2em)
  #text(size: 12pt)[#title]
  #v(0.4em)
  #if anon [#text(style: "italic")[Anonymous author(s), for double-blind review]] else [#author-names]
]

#v(0.5em)
This document supplements the paper named above. Sections S1 to S8 and Tables S1 to S9 are in this
document; every other section, table or appendix number refers to the paper.

= Security assumptions <supp-security>

@assumptions-table lists the assumptions behind the signature schemes and proof systems of
#paper("security"), and whether each property holds against a classical and a
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

= Proof methods and request formats <capabilities>

The verifier service lists the proof methods it accepts in each query request, with a verification
key for each from its own configuration (#paper("request")). @methods-table shows
what our proof methods support, and §#ref(<repro>, supplement: none) gives their identifiers. Our
services reject a request outside these limits before proving and before marking it as answered.
Besides the queries of #paper("expressivity"), they reject a request that requires
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
with the exclusions of #paper("expressivity"). The services encode the query request
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

= Experiment records <supp-records>

@test-cases-table shows the answers that each zkVM experiment of #paper("evidence-table")
proved, by kind of input. The inputs were small synthetic graphs or, with signature checks, the W3C test vector
for `eddsa-rdfc-2022` and the payment credential of #paper("proved").
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
    several receipts. The columns are the rows with receipts in #paper("evidence-table"): the
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

== The evaluator without signature checks <supp-exact>

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

== The evaluator with signature checks <supp-signed>

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
- the payment question, over the payment credential of #paper("proved"),
  with both kinds of input;
- the remaining test cases, over the W3C test vector: `SELECT`, false `ASK` and `CONSTRUCT` over an
  input chosen by the holder; true `ASK` and `CONSTRUCT` over an agreed input; and a `SELECT` whose
  result exceeds the row limit. The verifier accepted every answer except the last, which it
  rejected before marking the request as answered.

For the first test case, the internal check covered #headline("zkvcq.vcqg_source_files_verified")
source files, unchanged during the run, and #headline("zkvcq.vcqg_archive_verified_files") archived
files.

== Negative tests <supp-negative>

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

= The public-triple circuit pilot <supp-pilot>

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
      (#paper("pilot-evidence")): median and range per circuit, and the difference,
      public-triple minus baseline, for each pair of runs.],
  ) <pilot-table>

  The verifying difference of largest magnitude came from one slow baseline run.
]

= Executor cycle counts <supp-cost>

The measurements of #paper("cost") ran the guest program with signature checks in
the RISC Zero executor, without proving, for each cryptosuite, signature mode and number of
credentials and, in the sweep, each credential size. Every request used an input chosen by the
holder, and the RFC 8032 test key signed every credential. For the #headline("zkexec.main_queries") queries, we
executed #headline("zkexec.main_runs") runs over the payment credential and, with four credentials,
copies issued to other customers. For the sweep, we executed #headline("zkexec.sweep_runs") runs
over generated credentials. Each describes a person with typed literals, a chain of acquaintances
and one blank node, padded with further triples to the stated size. The counts of the two sets are
never added. The executor runs had no internal check (#paper("prototype")).

The executor runs used our prover's session limit (#paper("cost")), and an
admitted case that reached it does not count as completed. The cycle counts, like the limit, cover
the guest program's own cycles, without the overhead and padding that the prover adds to each
segment.

For #paper("phase-table"), a separate build of the same guest program, used only for
measurement, reads the cycle counter after decoding its input and at the end of each phase. It passes the readings to the
host, not to the journal. A phase that runs once per credential is summed over the credentials, and
the readings include the instrumentation.

#paper("expressivity") lists the sweep's query cases by feature, with the cases
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
// [OPUS-5.5] §7.2 of the paper says that, with one credential, the evaluator admitted the same
// number of cases in every configuration and the guest program completed all of them; these asserts keep it true.
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

= An earlier fixed-circuit design <legacy>

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

= Combining BBS+ proofs with circuits <composition>

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

= Artifacts and reproduction <repro>

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

// The paper's bibliography file; only the works cited here are listed. The anonymised entries of
// the paper's double-blind build are not needed, because this document cites neither companion
// draft.
#bibliography(
  "zksparql-architecture.refs.yml",
  style: "springer-lecture-notes-in-computer-science",
  title: [References],
)
