<!-- [FABLE-5] E2EE-queryable survey (bead sq-tag1q.3): honest design record of E2EE-for-Solid/RDF options that still support SPARQL query. Gates the e2ee-sparql.typ spec bead sq-tag1q.5. -->

# E2EE that still supports SPARQL query — the honest option space

**Status:** Deep-research design record (survey; **no implementation**, doc-only). Author:
Claude Fable 5 `[FABLE-5]`. Date: 2026-07-11. Bead **sq-tag1q.3** (epic **sq-tag1q**, the
Solid + SPARQL spec-proposal program). This record **gates** the E2EE-queryable spec draft
bead **sq-tag1q.5** (`site/specs/e2ee-sparql.typ`): the spec must normatively define only
the profiles this survey concludes are *specifiable today*.

> **Honesty banner.** General **server-side SPARQL evaluation over end-to-end-encrypted
> data without leakage does not exist** — not in the literature, not in any deployed
> system, and not in sparq. Every known scheme that lets an untrusted server answer
> queries over ciphertext buys expressiveness with a **disclosed leakage profile**, and
> the leakage-abuse literature shows those profiles are routinely exploitable
> ([IKK12], [CGPR15], [NKW15], [GSBNR17], [ZKP16]). Any spec this survey gates must say
> this plainly, must make leakage a first-class normative concept, and must never market
> a leaky profile as "end-to-end encrypted" without qualification. Additionally, every
> claim below about sparq's own ZK/MPC estate is **externally unaudited** (master gate
> **sq-qhy4**, P0, open) — nothing here is a soundness claim.

---

## 0. TL;DR / verdict

**Specifiable today (recommend the spec define these two profiles, and only these):**

- **Profile CS ("client-side"; core, mandatory-to-implement).** E2EE resources at rest;
  the server is a dumb authenticated blob store; the client syncs ciphertext, decrypts
  locally, indexes in memory, and evaluates **full SPARQL 1.1 locally**. Zero query
  leakage to the server beyond resource-level access patterns (which resources were
  fetched, when, how big). This is the only profile with both full expressiveness and a
  defensible confidentiality story, and sparq can ship it: the full engine already runs
  client-side in the browser (`crates/sparq-wasm`), ingests in-memory bytes, and builds
  indexes incrementally (§3.a). The net-new work is the encryption envelope + key
  management, not the query engine.
- **Profile SE ("structure-exposed"; optional, leakage-disclosed).** Predicate/property-
  level encryption: literal **values** are AEAD-encrypted, graph **structure** (subjects,
  predicates, objects-that-are-IRIs/bnodes, named-graph membership) stays cleartext and
  server-queryable. BGP matching, joins, and structural navigation work server-side;
  encrypted values are opaque (equality only via optional per-value deterministic tags,
  each a separately disclosed leakage step). The profile is honest only if the spec
  REQUIRES a leakage statement: **full graph topology is revealed**, and structure alone
  is highly identifying (§3.c).

**Documented but NOT specified (rationale in-line):**

- **(b) Searchable/structured encryption (SSE/STE)** — real cryptography, wrong shape:
  equality/lookup-class queries only, bespoke per-query-family indexes, well-studied
  access/search-pattern leakage, no interoperable standard to profile against. Revisit
  when an encrypted-graph scheme with a SPARQL-shaped query API exists (§3.b).
- **(e) ZK/MPC over committed data** — sparq's own estate; it provides **verifiability
  and confidentiality-in-computation, not E2EE storage** (data is cleartext at the
  prover/holders). Valuable as a *complementary annex* to the spec (proof-carrying query
  answers), never as an E2EE storage profile — and it is externally unaudited (§3.e).

**Rejected:**

- **(d) Deterministic / order-revealing encryption for FILTER/range** — REJECT as a
  normative profile. The inference-attack record ([NKW15], [GSBNR17], [GRS17]) shows
  frequency + order leakage decrypts realistic columns outright; a Solid pod's literal
  distributions (names, dates, locations) are exactly the high-skew data those attacks
  eat (§3.d).
- **(f) FHE** — impractical today for general SPARQL by orders of magnitude, and it does
  not even solve the right problem (the server learns nothing, but also *returns
  everything or runs the whole query circuit*); document and move on (§3.f).

---

## 1. Problem statement and framing

**Setting.** A Solid pod (or any RDF resource server) stores a user's RDF. The user wants
(i) the server operator to be unable to read the data — end-to-end encryption, keys held
by the data owner and their delegates — while (ii) applications retain some useful form
of SPARQL query. The tension is fundamental: SPARQL's expressiveness (BGP joins, property
paths, FILTER algebra, aggregation) is exactly the kind of rich computation that
ciphertext is designed to prevent.

**The axes every option must be scored on** (these become the matrix columns in §4):

1. **Query expressiveness** — which SPARQL fragment survives, evaluated *where*.
2. **Leakage profile** — what the server (and network observer) learns: nothing /
   access patterns / search patterns / equality patterns / order / full structure.
3. **Key-management burden** — key hierarchy, sharing/delegation, rotation, revocation.
4. **Server trust required** — dumb blob store ↔ trusted-for-confidentiality.
5. **Maturity** — deployed practice / published crypto / research prototype / open problem.
6. **What sparq could ship** — grounded in the actual estate (recon: `crates/sparq-wasm`,
   `crates/sparq-solid`, `crates/sparq-zk`, `crates/sparq-mpc`,
   [`research/crypto-erase-at-rest.md`](./crypto-erase-at-rest.md)).

**What "E2EE" must mean here.** Keys are generated and held client-side; the server never
possesses plaintext or decryption keys. This excludes server-side at-rest encryption
(server holds the keys — that is [`research/crypto-erase-at-rest.md`](./crypto-erase-at-rest.md)'s
crypto-erase design, bead sq-du24, an *orthogonal* operator-side control) and excludes
TLS-only transport protection. A scheme where the server can decrypt on demand is access
control, not E2EE — `crates/sparq-solid` (WAC/ACP authorization oracle) already covers
that layer and is likewise orthogonal.

**Prior art anchor for the RDF-specific corner.** The RDF-native literature is thin but
real: partial encryption of RDF graphs [Gie05], self-enforcing access control over
encrypted RDF [FKPS17], and HDT-crypt's compressed+encrypted RDF datasets [FKPS20] are
the closest antecedents; none provides server-side SPARQL over ciphertext beyond
lookup-shaped access.

---

## 2. Ground truth: what sparq has today (recon summary)

| Estate surface | What it actually provides | E2EE-relevant? |
|---|---|---|
| `crates/sparq-wasm` (npm `@sparq-org/sparq`) | Full SPARQL 1.1 query surface client-side (WCOJ BGP, FILTER, OPTIONAL, UNION, MINUS, BIND, VALUES, aggregates, ORDER BY, sub-SELECT; SELECT/ASK/CONSTRUCT/DESCRIBE), in-memory ingest via `Store.load` (Turtle/N-Triples/N-Quads/TriG), single-threaded, wasm32 4 GiB linear-memory ceiling, persistence/mmap deliberately not exported (README §capability notes) | **Yes** — the query half of Profile CS exists |
| `crates/sparq-core` | Streaming `load_reader*` from any `Read` (wrap a decryptor), incremental index build from empty `Graph::new()` | **Yes** — decrypt-then-stream-ingest is architecturally supported |
| `crates/sparq-solid` | WAC/ACP **authorization oracle** (N3-rule-materialized auth view, per-session filtered `query_as`), compiles to wasm32; explicitly *not* a confidentiality mechanism | Orthogonal (access control ≠ E2EE) |
| `crates/sparq-zk` (+`-compose`) | Single-prover ZK proofs of query correctness over **Poseidon2-BN254 per-graph commitments** (RDFC10-canonicalized); accepted fragment: SELECT/ASK over BGP scans, datatype-bucketed value FILTER lanes, single equality join, membership-indifferent modifiers; everything else fail-closed `UnsupportedFragment` (`src/verify.rs`; [`research/zksparql-fragment-extension.md`](./zksparql-fragment-extension.md) §1) | **No E2EE**: prover holds cleartext; commitment ≠ encryption |
| `crates/sparq-mpc` | Honest-majority **semi-honest** Shamir/BGW: disclosed-key equi-join (crypto-free), hidden-value all-pairs secret-shared equality join, secure comparison, bounded property paths, degree reduction; collaborative proof is a loud stub (`proof.rs` → `NotYetImplemented`); malicious security designed-only | **No E2EE**: each holder evaluates over its own **cleartext** graphs; sharing is transient, compute-time only |
| [`research/crypto-erase-at-rest.md`](./crypto-erase-at-rest.md) (sq-du24, design-only) | Server-side at-rest AEAD segments, DEK-under-KEK, crypto-shred by key destruction; explicitly "does not protect a live process", orthogonal to confidentiality-in-use | **Not E2EE** (server holds keys) but the AEAD envelope + key-hierarchy design vocabulary is reusable |
| Encryption/key-management **code** anywhere in `crates/` | **None.** Zero E2EE, client-side-decryption, or key-management primitives in the tree | The gap: Profile CS's *encryption* half is net-new |
| Audit posture | `sparq-zk`/`sparq-zk-compose`/`sparq-mpc` are `publish = false`, research-grade, **externally unaudited** — master gate **sq-qhy4** (P0, open); internal re-audits ([`research/zk-verifier-reaudit.md`](./zk-verifier-reaudit.md)) are self-review, not sufficient | Any spec text touching (e) must carry this caveat verbatim |

**Net:** sparq is unusually well positioned for the *client-side* profile — the expensive
part (a full, WASM-portable SPARQL engine) exists — and possesses exactly **zero** of the
encryption layer. Nothing in the estate is, or claims to be, E2EE storage.

---

## 3. The option space

### 3.a Client-side SPARQL over locally-decrypted data (Profile CS) — the achievable baseline

**Mechanism.** Resources (RDF documents, or coarser "dataset bundles") are encrypted
client-side with a content-encryption key (AEAD: XChaCha20-Poly1305 or AES-256-GCM) and
stored on the server as opaque blobs plus a small cleartext envelope (key-ID, algorithm,
nonce, padding discipline). The server is a dumb authenticated blob store — in Solid
terms, plain resource CRUD over `application/octet-stream` payloads; existing WAC/ACP
still governs *which ciphertexts* a client may fetch. The client syncs the ciphertexts it
is authorized for, decrypts, parses, indexes in memory, and evaluates SPARQL locally.

**Query expressiveness: full SPARQL 1.1**, because evaluation happens over plaintext on
the client. This is the only option in the space with no fragment carve-out.

**Leakage profile.** The server learns: resource identifiers and container structure
(unless additionally padded/obfuscated), ciphertext sizes and update timing, and the
client's **resource-level access pattern** (which blobs are fetched together — a
traffic-analysis signal, not query-pattern leakage in the SSE sense, and it collapses
entirely under sync-everything replication). It never learns triples, query shapes, or
answers. Honest residual: sizes and co-access are real side channels; the spec should
mandate padding buckets and permit full-replica sync as the zero-access-pattern mode.

**Cost — the honest downside — is the sync/index cost.** The client must download and
decrypt every resource that *might* contribute to an answer; there is no server-side
selectivity. Quantified in shape (not numbers — box measurements are non-canonical):
cost scales with the authorized-and-relevant ciphertext corpus, not with answer size;
first-query latency includes full decrypt+parse+index; steady-state requires an
invalidation/delta protocol (ETag/notification-driven re-sync). Mitigations, in
increasing leakage order: (i) coarse **cleartext partitioning metadata** (per-resource
topic/graph tags — leaks a category), (ii) client-maintained **encrypted index sidecars**
(the client uploads its own encrypted index blobs and fetches only index shards —
approaching SSE territory, leakage = shard-access pattern), (iii) resource-granularity
tuning (many small resources = better selectivity, more access-pattern signal; one big
bundle = the opposite). The wasm32 4 GiB linear-memory ceiling and single-threaded
execution (`crates/sparq-wasm` README) bound the practical in-browser corpus; a native
client (Tauri GUI, CLI) has no such ceiling.

**Prior art.** This is the pattern the Solid ecosystem can already *almost* express:
client-side query engines over pods are mainstream (Triple Pattern Fragments [VVH16],
Comunica [TVS18], and link-traversal query over Solid pods [TV23] all evaluate SPARQL
client-side against servers that only serve documents/fragments — precisely the
evaluation topology Profile CS needs, minus the decryption step). Mylar [PSVBZ14]
pioneered (and [GMNRS16] duly broke the searchable part of) client-side-keys web apps;
the durable lesson is that the *pure* client-side-decrypt path survives scrutiny and the
server-searchable add-ons are where the bodies are buried.

**Key management.** The real engineering surface: per-resource DEKs wrapped under
per-recipient KEKs (the DEK/KEK split mirrors
[`research/crypto-erase-at-rest.md`](./crypto-erase-at-rest.md) §Option-A1, reused
client-side); sharing = re-wrapping DEKs to a delegate's public key (WebID-anchored
keypairs are the natural Solid binding); revocation = re-encrypt-on-membership-change
(lazy revocation is a documented weakening); rotation and multi-device sync need a
first-class story. None of this is exotic — it is the age-old E2EE group-sharing problem
— but the spec must define it normatively or implementations will not interoperate.

**Maturity: deployed practice** (client-side-encrypted sync stores are an established
product category) — the *combination* with SPARQL is novel glue, not novel cryptography.

**What sparq could ship.** An opt-in `sparq-e2ee` client crate (+ wasm export): AEAD
envelope codec, DEK/KEK wrap, decrypt-to-`load_reader`/`Store.load` streaming glue, and a
sync-manifest walker. Server side: nothing (that is the point) beyond serving blobs —
which any Solid server already does. Core stays lean; no engine changes required.

### 3.b Searchable / structured encryption for graphs (SSE/STE) — real crypto, wrong shape

**Mechanism.** Structured encryption [CK10] encrypts a data structure together with
per-query tokens: the client derives a token from its query; the server uses it to walk
the encrypted structure and return matching (still-encrypted) items, learning the
**search pattern** (token repetition) and **access pattern** (which entries matched).
Graph-shaped instantiations exist: adjacency/neighbor queries [CK10], approximate
shortest-distance queries (GRECS [MKNK15]), shortest-path queries via encrypted sketches
[GKT21], top-k/social-graph search (GraphSE² [LYSL19]); relational cousins (SPX [KM18],
Arx [PBP19], Blind Seer [PKVK14]) cover selection/equality-join fragments. Fuller et
al.'s SoK [FVYSHGKMC17] is the honest map of this whole space and its trust spectrum.
The attack-then-patch cycle is alive even for the graph schemes: [GKT21] itself drew a
query-recovery attack (Falzon–Paterson, ESORICS 2022 [FP22]) and a repaired successor
(PathGES, CCS 2024 [FPSO24]) within three years — a caution against freezing any
particular scheme into a spec.

**Query expressiveness.** Equality/lookup-class only, and only for the query families the
encrypted structure was *pre-built* for: triple-pattern lookup with constant terms,
neighbor expansion, exact-match text search. Each new query family (a join shape, a path
class) is a new bespoke encrypted index with its own leakage. Nothing resembling general
BGP joins, FILTER algebra, OPTIONAL, paths, or aggregation is on the table; a SPARQL
front-end would be a thin veneer over a drastically restricted fragment.

**Leakage profile — must be stated in the spec if this is ever profiled.** Search
pattern + access pattern at minimum; volume pattern always. The leakage-abuse literature
is unambiguous that these are exploitable under realistic auxiliary knowledge:
query-recovery from access patterns [IKK12], count/known-data attacks [CGPR15], and
file-injection attacks that *actively* recover queries [ZKP16]. RDF makes it worse: the
"documents" (triples/terms) come from low-entropy, publicly-enumerable vocabularies —
predicates and classes are drawn from published ontologies, so token-frequency analysis
has an unusually strong prior.

**Key management** is comparable to CS (client keys, token derivation), **server trust**
is "honest-but-curious with disclosed leakage", **maturity** is research-prototype for
graphs (no standard scheme, no interoperable format, no production deployments for
RDF).

**Verdict: document, don't specify.** The gap between "adjacency queries over an
encrypted graph" and "SPARQL" is the whole distance; profiling SSE now would freeze a
bespoke scheme with attack-prone leakage into a spec. Revisit if/when an encrypted-graph
scheme with a stable, SPARQL-mappable query API and a peer-reviewed leakage analysis
exists. (If a future sparq experiment wants a stepping stone, the client-built encrypted
index sidecar of §3.a-(ii) delivers most of the practical value with client-controlled
leakage.)

### 3.c Predicate/property-level encryption: encrypt values, expose structure (Profile SE)

**Mechanism.** Keep the graph *structure* cleartext — subjects, predicates, IRI/bnode
objects, named-graph membership — and AEAD-encrypt only **literal values** (per-object
ciphertexts, e.g. `"xsalsa20:…"^^sparq:enc` or a reification-free encrypted-literal
datatype), with per-predicate or per-resource DEKs so disclosure can be selective. This
is the direct descendant of partial RDF encryption [Gie05] and the
encrypt-fragments-under-policies line [FKPS17], [FKPS20].

**Query expressiveness.** Surprisingly large *structural* fragment, evaluated
server-side over the cleartext skeleton: full BGP matching and joins on
subjects/predicates/IRI-objects, property paths, OPTIONAL/UNION/MINUS over structure,
`COUNT`-style aggregation over structure. Everything touching an encrypted **value** is
opaque: no FILTER on values, no ORDER BY, no value joins, no value aggregation —
answers come back with ciphertext literals the *client* decrypts (and may post-FILTER
locally, hybrid-style). Optional extension: per-value **deterministic equality tags**
(HMAC-under-client-key) restore server-side value-equality joins and `FILTER(?x = const)`
— at the price of stepping onto the deterministic-encryption leakage ladder of §3.d
(frequency of equal values leaks); the spec must treat equality tags as a separately
opt-in, separately disclosed leakage increment, not a default.

**Leakage profile — the honest headline: full graph topology.** The server learns every
subject, every predicate, the entire join structure, all IRI-valued relationships,
degrees, co-occurrence, update dynamics — plus ciphertext lengths (value-size
fingerprints; padding required). Be explicit about how identifying that is: predicates
and classes name the *kind* of every hidden value (`foaf:name`, `dbo:diagnosis` announce
what the ciphertext is); graph structure is a fingerprint (social-graph
de-anonymization from topology alone is classical [NS09], and RDF adds labeled,
ontology-typed edges to the attacker's side); [NKW15]-style inference then works on any
value that *is* exposed via equality tags. Profile SE protects the **values**, not the
**shape of your life** — the spec must say approximately that sentence.

**Key management:** per-predicate/per-resource DEK families, recipient-wrapped as in CS;
finer granularity = more selective disclosure and more keys. **Server trust:**
trusted-for-structure, untrusted-for-values — a genuinely useful *intermediate* point,
e.g. hiding medical values from a pod host while structural app queries keep working.
**Maturity:** published research (RDF-specific) + ubiquitous deployed analogues
(field/column-level encryption in databases); no standard RDF encoding exists — that is
exactly the interoperability gap a spec profile can close.

**What sparq could ship.** Server side, nothing new: encrypted literals are just typed
literals; BGP/join evaluation over them works today. Client side: the same `sparq-e2ee`
envelope crate as CS (value-granularity encrypt/decrypt, tag derivation), plus a small
result-decryption post-processor. An engine-side "decrypt-in-FILTER" UDF is explicitly
NOT proposed (it would move keys server-side, destroying the E2EE claim).

### 3.d Deterministic / order-revealing encryption for FILTER and ranges — REJECT

**Mechanism.** DET encryption makes equal plaintexts equal ciphertexts (server-side
equality/GROUP BY/joins); OPE/ORE ([BCLO09], [BLRSZZ15]) additionally exposes order
(range FILTER, ORDER BY). CryptDB [PRZB11] made the onion-of-encryptions pattern famous
and briefly plausible.

**Why rejected.** The attack record is decisive. With frequency analysis and public
auxiliary data, [NKW15] decrypted DET/OPE-protected hospital columns wholesale;
[GSBNR17] extended leakage-abuse to even "ideal" ORE, and reconstruction-from-range-
queries attacks ([KKNO16] and successors) recover plaintexts from access patterns alone
given enough queries. Real deployments built on these primitives were broken along the
same seams ([GMNRS16] for Mylar-style multi-user search; the post-CryptDB assessment
[GRS17] concluded the deployed configurations offered little protection). RDF literals
in a personal pod — names, birthdays, places, diagnoses — are *precisely* the
low-entropy, public-prior data these attacks feast on. Order leakage on a date predicate
is a timeline; equality leakage on a location predicate is a movement profile.

**Disposition.** Not a profile, not an optional annex. The only DET-shaped thing that
survives is §3.c's narrowly-scoped, separately-disclosed per-value equality tag — and
the spec should carry a normative warning that enabling it on skewed predicates
re-opens [NKW15]. No ORE/range anything: a client needing range FILTER uses Profile CS
(local evaluation) for that data.

### 3.e ZK/MPC-assisted paths over committed data — complementary verifiability, not E2EE

**What sparq's estate actually provides** (recon §2; precise, and externally unaudited —
**sq-qhy4**):

- `sparq-zk`: **single-prover** proofs that a query answer is correct w.r.t. a
  Poseidon2-BN254 **commitment** to an RDFC10-canonicalized graph, for a deliberately
  small fragment (SELECT/ASK over BGP scans, datatype-bucketed value-FILTER lanes, one
  equality join, membership-indifferent modifiers; all else fail-closed). Designed-only
  extensions (bounded paths, UNION, expression FILTERs; dual-leaf commitments) are
  recorded in [`research/zksparql-fragment-extension.md`](./zksparql-fragment-extension.md)
  and [`research/zk-configurable-commitment-design.md`](./zk-configurable-commitment-design.md).
- `sparq-mpc`: honest-majority **semi-honest** Shamir/BGW federated evaluation —
  disclosed-key equi-joins, hidden-value equality joins, secure comparison, bounded
  property paths — where each holder evaluates sub-queries over its **own cleartext
  graphs** and only transient shares cross the wire. Malicious security and the
  collaborative proof over shared witnesses are designed-only/stubbed
  (`proof.rs` → `NotYetImplemented`; [`research/mpc-zkp-federated-sparql-design.md`](./mpc-zkp-federated-sparql-design.md) §3.1).

**Scope precisely — what this adds and what it does not.** A commitment is not
encryption: it hides nothing from whoever holds the data (the prover *is* the data
holder) and stores nothing on anyone's behalf. MPC's confidentiality is
**in-computation**: inputs stay at their owners, cleartext, and are protected only
*during a joint query* — the server-holds-only-ciphertext property of E2EE storage never
appears. What these paths genuinely add to an E2EE-queryable spec is orthogonal and
valuable: (i) **verifiability** — a Profile-CS client, or a third party, can check that
an answer over data it *cannot* see is consistent with a published commitment
(proof-carrying answers; cf. the maintainer's ZKP-SPARQL line [Wri25] — the full
ZKP-of-correct-SPARQL-evaluation paper is an ISWC 2026 **submission**, not yet
peer-reviewed — the published Braun–Wright–Käfer soundness-of-SPARQL-results paper
[BWK26], verifiable SQL analogues (ZKSQL [ZKSQL23]), and collaborative zk-SNARKs [OB22]
for the multi-prover future); (ii) **cross-pod joins without
disclosure** — two E2EE pods can answer a joint query via MPC after client-side
decryption at each owner, never re-uploading plaintext. Both belong in the spec as an
**informative annex** ("verifiable answers over committed data"), explicitly
non-normative, explicitly carrying the unaudited-estate caveat. Neither is an E2EE
storage profile, and the spec must not present them as one.

**Maturity:** research-grade, fail-closed, `publish = false`, and — again — **no
soundness claim is made or may be made until the external audit (sq-qhy4) lands**.

### 3.f Fully homomorphic encryption — impractical today, cite and close

FHE would let the server evaluate over ciphertext with *no* leakage beyond volume —
the asymptotically "right" answer, and (for the record) the only path that even
notionally closes the server-side-evaluation gap. Three reasons it is not a profile:
(1) **Cost**: despite a decade of engineering (BGV/BFV/CKKS/TFHE stacks — HElib, SEAL,
OpenFHE), general computation under FHE remains orders of magnitude slower than
plaintext — the consensus figure across the survey/acceleration literature is ~10³–10⁴×
even for scheme-favorable workloads ([VJH21]; the FHE-acceleration survey field
[FHEACC24] exists *because* the gap is that large; encrypted-SQL prototypes [FHESQL25]
remain narrow-workload) — which composes catastrophically with SPARQL's join
complexity. (2) **Shape mismatch**: FHE circuits are data-oblivious by
construction, so a query must touch the *entire* dataset (no indexes, no selectivity);
private-database-query research therefore retreats to PIR/keyword-PIR-class
functionality, not general query algebra. (3) **It solves confidentiality-in-compute,
not the systems problem**: key management, sharing, update, and multi-writer semantics
are all still Profile-CS-shaped around the outside. Disposition: two honest paragraphs
in the spec's "considered and excluded" section, citations, done.

---

## 4. Trade-off matrix

Legend — Leakage tiers, worst-to-best: **T0** = plaintext-equivalent under attack;
**T1** = full structure; **T2** = search+access+volume patterns; **T3** = resource-level
access pattern + sizes/timing only; **T4** = volume/timing only.

| Option | SPARQL expressiveness | Evaluated where | Leakage to server | Key-mgmt burden | Server trust required | Maturity | sparq gap to ship |
|---|---|---|---|---|---|---|---|
| **(a) CS** client-side over decrypted | **Full SPARQL 1.1** | Client (wasm/native) | **T3** (→T4 with full-replica sync + padding) | Med–High (DEK/KEK, sharing, rotation, multi-device) | **Blob store only** | Deployed practice (novel glue, not novel crypto) | Envelope+keys crate only; engine exists (`sparq-wasm`) |
| **(b) SSE/STE** encrypted graph structures | Lookup/equality/neighbor classes; per-family bespoke indexes | Server (token-driven) | **T2** — search+access+volume; exploitable [IKK12], [CGPR15], [ZKP16] | High (keys + token discipline + index lifecycle) | Honest-but-curious w/ disclosed leakage | Research prototypes for graphs; no standard | Whole scheme net-new; **not specified** |
| **(c) SE** value-encrypt, structure-exposed | Structural BGP/joins/paths server-side; values opaque (client post-filter; opt-in DET equality tags) | Server (structure) + client (values) | **T1** — full topology + predicate types (+equality pattern if tags on) | Med (per-predicate DEK families) | Trusted-for-structure | Published RDF research [Gie05], [FKPS17], [FKPS20] + deployed field-crypto analogues; no standard encoding | Envelope + literal-encoding profile; server-side engine works today |
| **(d) DET/ORE** for FILTER/range | Equality/range/ORDER BY server-side | Server | **T0** — frequency+order; broken in practice [NKW15], [GSBNR17], [KKNO16] | Med | Nominally untrusted; effectively exposed | Deployed then broken (CryptDB lineage) | **REJECTED** |
| **(e) ZK/MPC** over committed data | ZK: small BGP+eq-join+value-FILTER fragment; MPC: federated joins/compare/bounded paths | Prover/holders (**cleartext at owner**) | N/A for storage — **not E2EE**; adds verifiability / in-compute confidentiality | High (commitments, issuers, share lifecycle) | Varies; holders trust each other per model (semi-honest today) | Research-grade, fail-closed, **externally unaudited (sq-qhy4)** | Informative annex only |
| **(f) FHE** | Notionally full; practically PIR-class | Server (oblivious circuit) | **T4** | High | Untrusted | Impractical for general query today [VJH21] | **REJECTED** (documented) |

Reading the matrix honestly: **(a) is the only cell with full expressiveness and
defensible leakage**; (c) is the only *server-side-query* cell whose leakage can be
stated in one sentence and consented to; everything else is rejected, deferred, or not
an E2EE storage option at all.

---

## 5. Recommendation to the spec bead (sq-tag1q.5)

1. **Normatively define exactly two conformance profiles.**
   - **Profile CS (core, MUST-implement for E2EE conformance):** encrypted-resource-at-
     rest + dumb blob server + client-side full-SPARQL evaluation. Normative content:
     the AEAD envelope format (algorithm registry, nonce/padding discipline), the
     DEK/KEK wrapping + WebID-anchored recipient keys, sharing/rotation/revocation
     semantics, the sync-manifest/delta protocol, and the padding/access-pattern
     mitigations (with full-replica sync as the zero-access-pattern mode). Server
     conformance = ordinary Solid resource semantics over opaque payloads (near-zero new
     server surface — deliberate).
   - **Profile SE (optional, leakage-disclosed):** encrypted-literal encoding (a
     normative `sparq:enc`-style datatype/envelope for literal values), per-predicate
     key families, client result-decryption, and — separately opt-in with its own
     normative warning — deterministic equality tags. **Normative requirement: an
     implementation MUST surface the leakage statement** ("full graph structure,
     predicate vocabulary, and (if tags) value-equality patterns are visible to the
     server") in its conformance documentation.
2. **Make leakage a normative vocabulary.** Define the T0–T4 tiers (§4) in the spec and
   require every profile (and every future extension) to declare its tier. This is the
   device that keeps the spec honest as it evolves.
3. **Carry the impossibility statement in the spec body**, not a footnote: no known
   construction provides general server-side SPARQL over E2EE data without leakage;
   profiles are points on a disclosed trade-off curve, not workarounds of this fact.
4. **Include a non-normative annex** "Verifiable answers over committed data" scoping
   the ZK/MPC composition (proof-carrying answers for Profile-CS clients; MPC cross-pod
   joins), with the **sq-qhy4 externally-unaudited caveat stated verbatim** and no
   soundness language.
5. **Record the rejections with citations** (SSE/STE deferred-with-criteria; DET/ORE
   rejected on the attack record; FHE rejected on cost/shape) so the spec inherits this
   survey's reasoning and future re-litigation starts from evidence.
6. **Do not specify**: server-side decrypt UDFs (breaks E2EE), ORE/range anything,
   SSE indexes (premature), or any normative dependency on the unaudited ZK/MPC crates.

**Follow-on implementation shape (post-spec, separate beads, all opt-in):** a single
client-side `sparq-e2ee` crate (AEAD envelope, DEK/KEK, decrypt-to-ingest streaming glue
for `load_reader`/`Store.load`, SE literal codec, equality-tag derivation) + wasm export;
zero changes to `sparq-core`/`sparq-engine`; server untouched. Core stays lean.

---

## 6. Open questions (candidate beads for the spec/impl phase)

- Envelope algorithm registry governance (tie to the spec's SOTD process, sq-rvgr2).
- Key discovery/binding in Solid: where recipient public keys live (WebID profile
  document? `.well-known`?) and how that interacts with `crates/sparq-solid`'s
  session model — coordinate with the jeswr/solid-specs program (sq-tag1q.8 gate).
- Delta-sync protocol vs the SPARQL-CRDT track (sq-tag1q.4): an E2EE pod that is also a
  CRDT replica multiplies both designs' constraints; flag the interaction now, design
  later.
- Whether Profile SE's equality tags should be per-recipient (unlinkable across
  audiences) at the cost of tag-set blowup.
- Access-pattern hardening beyond padding (bucketized fetch? PIR for index shards?) —
  strictly future work; do not gate the profiles on it.

---

## References

Verified against public records 2026-07-11 (web-verification pass by a research
sub-agent). Entries marked ◐ were not re-verified in that pass and are cited from
settled literature knowledge — re-confirm before promoting into normative spec text.

- [BCLO09] Boldyreva, Chenette, Lee, O'Neill. *Order-Preserving Symmetric Encryption.* EUROCRYPT 2009, LNCS 5479. <https://eprint.iacr.org/2012/624>
- [BLRSZZ15] Boneh, Lewi, Raykova, Sahai, Zhandry, Zimmerman. *Semantically Secure Order-Revealing Encryption: Multi-input Functional Encryption Without Obfuscation.* EUROCRYPT 2015 (Part II), pp. 563–594.
- [BWK26] Braun, Wright, Käfer. *Proving Soundness of SPARQL Query Results Using Selective Disclosure of RDF Datasets and Zero-Knowledge Proofs.* The Semantic Web (ESWC 2026), Springer, pp. 297–318. DOI 10.1007/978-3-032-25156-5_16.
- [CGPR15] Cash, Grubbs, Perry, Ristenpart. *Leakage-Abuse Attacks Against Searchable Encryption.* ACM CCS 2015, pp. 668–679.
- [CK10] Chase, Kamara. *Structured Encryption and Controlled Disclosure.* ASIACRYPT 2010, LNCS 6477, pp. 577–594. <https://eprint.iacr.org/2011/010>
- [FHEACC24] *Practical solutions in fully homomorphic encryption: a survey analyzing existing acceleration methods.* Cybersecurity (SpringerOpen) 7, 2024. <https://cybersecurity.springeropen.com/articles/10.1186/s42400-023-00187-4>
- [FHESQL25] *FHE-SQL: Fully Homomorphic Encrypted SQL Database.* arXiv:2510.15413, 2025 (preprint).
- [FKPS17] Fernández, Kirrane, Polleres, Steyskal. *Self-Enforcing Access Control for Encrypted RDF.* ESWC 2017, LNCS 10249, pp. 607–622.
- [FKPS20] Fernández, Kirrane, Polleres, Steyskal. *HDTcrypt: Compression and encryption of RDF datasets.* Semantic Web 11(2):337–359, 2020. DOI 10.3233/SW-180335.
- [FP22] Falzon, Paterson. Query-recovery attack on the [GKT21] graph-encryption scheme. ESORICS 2022.
- [FPSO24] *PathGES* — repaired graph-encryption scheme for shortest-path queries. ACM CCS 2024.
- [FVYSHGKMC17] Fuller, Varia, Yerukhimovich, Shen, Hamlin, Gadepally, Shay, Mitchell, Cunningham. *SoK: Cryptographically Protected Database Search.* IEEE S&P 2017, pp. 172–191. <https://arxiv.org/abs/1703.02014>
- [Gie05] ◐ Giereth. *On Partial Encryption of RDF-Graphs.* ISWC 2005.
- [GKT21] Ghosh, Kamara, Tamassia. *Efficient Graph Encryption Scheme for Shortest Path Queries.* ACM ASIA CCS 2021. DOI 10.1145/3433210.3453099.
- [GMNRS16] Grubbs, McPherson, Naveed, Ristenpart, Shmatikov. *Breaking Web Applications Built On Top of Encrypted Data.* ACM CCS 2016.
- [GRS17] Grubbs, Ristenpart, Shmatikov. *Why Your Encrypted Database Is Not Secure.* HotOS 2017.
- [GSBNR17] Grubbs, Sekniqi, Bindschaedler, Naveed, Ristenpart. *Leakage-Abuse Attacks against Order-Revealing Encryption.* IEEE S&P 2017. <https://eprint.iacr.org/2016/895>
- [IKK12] Islam, Kuzu, Kantarcioglu. *Access Pattern Disclosure on Searchable Encryption: Ramification, Attack and Mitigation.* NDSS 2012.
- [KKNO16] ◐ Kellaris, Kollios, Nissim, O'Neill. *Generic Attacks on Secure Outsourced Databases.* ACM CCS 2016.
- [KM18] ◐ Kamara, Moataz. *SQL on Structurally-Encrypted Databases.* ASIACRYPT 2018.
- [LYSL19] Lai, Yuan, Sun, Liu, et al. *GraphSE²: An Encrypted Graph Database for Privacy-Preserving Social Search.* ACM AsiaCCS 2019. <https://arxiv.org/abs/1905.04501>
- [MKNK15] Meng, Kamara, Nissim, Kollios. *GRECS: Graph Encryption for Approximate Shortest Distance Queries.* ACM CCS 2015, pp. 504–517. <https://eprint.iacr.org/2015/266>
- [NKW15] Naveed, Kamara, Wright. *Inference Attacks on Property-Preserving Encrypted Databases.* ACM CCS 2015, pp. 644–655.
- [NS09] ◐ Narayanan, Shmatikov. *De-anonymizing Social Networks.* IEEE S&P 2009.
- [OB22] Ozdemir, Boneh. *Experimenting with Collaborative zk-SNARKs: Zero-Knowledge Proofs for Distributed Secrets.* USENIX Security 2022, pp. 4291–4308.
- [PBP19] Poddar, Boelter, Popa. *Arx: An Encrypted Database using Semantically Secure Encryption.* PVLDB 12(11):1664–1678, 2019.
- [PKVK14] ◐ Pappas, Krell, Vo, Kolesnikov, Malkin, Choi, George, Keromytis, Bellovin. *Blind Seer: A Scalable Private DBMS.* IEEE S&P 2014.
- [PRZB11] Popa, Redfield, Zeldovich, Balakrishnan. *CryptDB: Protecting Confidentiality with Encrypted Query Processing.* SOSP 2011, pp. 85–100.
- [PSVBZ14] Popa, Stark, Valdez, Helfer, Zeldovich, Kaashoek, Balakrishnan. *Building Web Applications on Top of Encrypted Data Using Mylar.* NSDI 2014.
- [TV23] Taelman, Verborgh. *Link Traversal Query Processing over Decentralized Environments with Structural Assumptions.* ISWC 2023, LNCS 14265. <https://arxiv.org/abs/2302.06933>
- [TVS18] Taelman, Van Herwegen, Vander Sande, Verborgh. *Comunica: A Modular SPARQL Query Engine for the Web.* ISWC 2018, LNCS 11137, pp. 239–255.
- [VJH21] ◐ Viand, Jattke, Hithnawi. *SoK: Fully Homomorphic Encryption Compilers.* IEEE S&P 2021.
- [VVH16] Verborgh, Vander Sande, Hartig, Van Herwegen, De Vocht, De Meester, Haesendonck, Colpaert. *Triple Pattern Fragments: a Low-cost Knowledge Graph Interface for the Web.* Journal of Web Semantics 37–38:184–206, 2016.
- [Wri25] Wright (jeswr). *Towards Provable Provenance and Privacy-Preserving Queries in Decentralised Data Architectures.* ISWC 2025 Companion (Doctoral Consortium), CEUR-WS Vol-4085, paper 19. <https://ceur-ws.org/Vol-4085/paper19.pdf> — the maintainer's line; the full ZKP-of-correct-SPARQL-evaluation paper (Wright, Shadbolt, Zhao, Zhao, Braun) is an ISWC 2026 **submission**, not yet peer-reviewed; the sparq secprop ontology extends its `sec-prop:` namespace (vendored at `crates/sparq-trust/ontologies/zkp-sparql/`).
- [ZKP16] Zhang, Katz, Papamanthou. *All Your Queries Are Belong to Us: The Power of File-Injection Attacks on Searchable Encryption.* USENIX Security 2016, pp. 707–720.
- [ZKSQL23] *ZKSQL: Verifiable and Efficient Query Evaluation with Zero-Knowledge Proofs.* PVLDB 2023. DOI 10.14778/3594512.3594513.

In-estate records cited: [`crypto-erase-at-rest.md`](./crypto-erase-at-rest.md),
[`zksparql-fragment-extension.md`](./zksparql-fragment-extension.md),
[`zk-configurable-commitment-design.md`](./zk-configurable-commitment-design.md),
[`mpc-zkp-federated-sparql-design.md`](./mpc-zkp-federated-sparql-design.md),
[`mpc-sparql-capability-matrix.md`](./mpc-sparql-capability-matrix.md),
[`zk-audit-readiness-dossier.md`](./zk-audit-readiness-dossier.md),
[`sparq-solid-scope.md`](./sparq-solid-scope.md).
