<!-- [OPUS-4.8] Design-for-review authored by Opus 4.8 (1M context) (Fable unavailable) — re-review when Fable returns. -->
# Configurable ZK commitment, circuit-builder, and signature program

Maintainer-review design record for bead **sq-1s2.5.1** — the configurable-ZK-encoding
program the maintainer greenlit in **#769**. This record sits **on top of** the
finalized dual-leaf encoding (`research/zk-field-native-encoding.md`, merged PR #794)
and the gate-reduction analysis (`research/zk-age-gatecount-reduction.md`); it does
**not** restate or contradict them. Where those records stop at *one* encoding
(dual-leaf) and *one* signature scheme (Schnorr over Baby-JubJub), this record
generalises both into **selectable axes**, per the maintainer's verbatim direction.

His direction (#769, verbatim):

> "I am aware of and acknowledge this attack [the dual-leaf INV-VL desync]. I would
> still like to see this implemented — with documentation identifying the risk.
> Since this is a research prototype — the ideal scenario would be that you enable
> BOTH ways of committing, and also have configurable ways of building circuits
> depending on the way that the data was committed. Then benchmarks with each
> configuration and a write-up of the security properties. We should also have this
> configurability with the SIGNATURE SCHEMES we support (as in my previous
> codebases). In fact, to enable performance comparisons + discuss backwards
> compatibility, ideally support commitments of W3C Verifiable Credentials using
> their different cryptosuites as well."

## 0. Honesty framing (load-bearing — read first)

- The whole ZK estate is **remediated but NOT externally audited** (`SECURITY.md`,
  gap **CR-G1** / bead **sq-qhy4**, P0). Nothing in this record is a security
  guarantee. Every soundness/privacy statement below is **obligation-** or
  **negation-framed** by intent, and the per-config security table (§7) is written
  as *what is NOT guaranteed* + *what an external auditor MUST verify*.
- `sparq-mpc` carries **no** confidentiality / correctness / attestation /
  malicious-security guarantee (CR-G7); it is out of scope here.
- The dual-leaf method **removes an invariant the current circuit machine-enforces**
  (INV-VL — see §1.C and CR-G8). The maintainer has **accepted this risk at research
  grade** (#769). This record's job is to make that trade-off **selectable and
  documented per method**, never hidden.
- **No hard performance numbers** appear in this record. Every cost statement is a
  *direction* or a *measurement obligation* discharged by `bb gates` and the
  `gate_count` regression snapshot (§6). Any figure produced on this work box is
  **NON-canonical** (it is an EC2 work box, not a CI runner — MEMORY:
  project-ec2-execution-env).

## 1. Premise correction to the brief

The brief stated that PR #794 is *"currently behind main and likely conflicting with
PR #802's gap-register CR-G8 edits"* and asked me to **rebase/reconcile** it.
**That premise is stale and is corrected here:**

- **#794 is MERGED and is the current `HEAD` of `main`** (merge commit `7d41be0b`,
  merged 2026-06-19). My initial `origin/main` ref was from 2026-06-17; after
  `git fetch`, #794 is the tip.
- **#794 landed AFTER #802** (`#802` = `dd30130c`, the audit-readiness dossier;
  `#794` = `7d41be0b`). There is therefore **no CR-G8 conflict to resolve** — #794's
  dual-leaf CR-G8 revision is *already* the current state of
  `compliance/cryptoreview/gap-register.md` (verified: the CR-G8 row reads
  *"DUAL-LEAF value+lexical … REMOVES an invariant the current circuit enforces"*).
- **#794 is therefore NOT folded or superseded** — it is the **foundation** this
  record builds on, already on `main`. The one coherent current design is: dual-leaf
  is the finalized *value-bearing* method (per #769), and this record adds the
  *configurability layer* (method selection, per-method circuit builders, pluggable
  signatures, VC cryptosuites) that #794 explicitly left out of scope (#794 §5–§6
  state it "does NOT address circuit-builder selection, signature-scheme changes,
  W3C VC specs, or configurability of the encoding itself").

No `compliance/` edit is needed for the reconciliation; CR-G8 already names the
forward obligation. This record adds **one new gap row proposal, CR-G9** (§8), for
the *configurability surface* itself — registered as a bead, applied by a future
compliance pass, not edited here unless the maintainer wants it inline.

### 1.A The three commitment methods (verified against the code)

| Method | Leaf shape (per literal object) | Where it lives today |
|---|---|---|
| **(i) string-canonical** (`poseidon2-rdfc10-v1`) | `Enc = h2(TYPE_CODE_LITERAL, blake3(canonical N-Triples token))` | **IMPLEMENTED** — `crates/sparq-zk/src/encode.rs:52-55`; the `RegistryEntry.scheme` default (`ZK_SCHEME_POSEIDON2_RDFC10_V1`, `registry.rs:72,136`) |
| **(ii) value-only** (`VALUE_HOOK`) | single value-first leaf, lexical hash *dropped* | **DESIGNED-ONLY** — drafted in PR #765, **superseded** by dual-leaf (#794 §1); not implemented |
| **(iii) dual-leaf** (value + lexical) | `Enc = h3(value_component, lexical_component, TYPE_CODE_LITERAL)` where `value_component = h3(VALUE_HOOK, DATATYPE_CONST, LANG_CONST)` and `lexical_component = blake3(canonical token)` | **DESIGNED-ONLY (FINAL)** — `research/zk-field-native-encoding.md` §3.1 (#794); not implemented (impl bead `sq-j506`, audit-gated) |

Method (i) is the *only implemented* commitment today. Methods (ii)/(iii) are
design-only and audit-gated. The configurability program makes the **method an
explicit, recorded selection** rather than the single hard-wired `poseidon2-rdfc10-v1`
the registry assumes today.

### 1.B What is ALREADY a seam (verified — do not rebuild)

The codebase already separates two of the three axes the maintainer wants, so this is
mostly *plumbing the seams the prior work left*, not green-field design:

- **Commitment scheme is already a vocabulary slot.** `RegistryEntry.scheme:
  NamedNode` (`registry.rs:97`) + the `zk:scheme` / `ZK_SCHEME_POSEIDON2_RDFC10_V1`
  IRIs (`registry.rs:41,72`). Today it has exactly one value. The method axis (§2)
  adds the dual-leaf scheme IRI alongside it; the existing string-canonical scheme is
  retained byte-for-byte.
- **Signature scheme is already an enum + cryptosuite-IRI map.** `sig::SignatureScheme`
  (`sig.rs:93`) with `cryptosuite_iri()` / `from_cryptosuite_iri()` (`sig.rs:103-114`)
  and the `zk:cryptosuite` registry slot (`registry.rs:42,99`). The module doc
  **already** names this "the modularity swap-point (BBS+, SD-JWT-VC, post-quantum
  candidates ship as parallel variants)" per "Jesse's modular-commitment/signature
  design" (`sig.rs:41-44`). v1 ships `Poseidon2SchnorrV1` only (§4).
- **Circuit-builder selection is already a typed dispatch.** `manifest::CircuitId`
  (`manifest.rs:515`) + `CircuitId::package()` (`manifest.rs:644`) name the compiled
  Noir member on disk (`scan_k{k}_n{n}_r{r}`, `filter_int_d{d}`, …); the verifier
  re-derives the `CircuitId` and dispatches to the canonical compiled member
  (`verifier.rs` `derive_*_id` + `canonical_vk`). The method axis (§3) adds new
  `CircuitId` variants for the value-bearing members and gates which variant is legal
  for a given commitment method.

So the configurability is **three already-present seams** (`scheme`, `cryptosuite`,
`CircuitId`) made *coherent and cross-validated*, plus the new value-bearing circuit
members the dual-leaf method needs.

### 1.C The INV-VL trade-off, stated once (carried into §7 per method)

INV-VL = *"the compared value equals `parse(committed lexical)`, enforced in-circuit
against an arbitrary committer (even a malicious trusted issuer)."* The current
string-canonical FILTER members machine-enforce it because the value and the operand
binding derive from **one** witnessed digit array (`filter_int.nr:67-92`,
`filter_signed.nr:150-180`). The dual-leaf method witnesses the value handle and the
lexical hash **independently**, so it **removes** INV-VL: a malicious *trusted* issuer
can sign one credential that answers a value FILTER as `18` and a
`sameTerm`/`DISTINCT`/`join` question as `5`. This is **impossible today** and is a
**trust-model regression for the value-FILTER lane** (machine-enforced →
issuer-honesty-trusted). No *untrusted* party can exploit it. Mitigations are
host-side same-leaf co-binding at ingest (fail-closed) and a named canonical-issuance
precondition (#794 §6, §5.6). **This is the risk the maintainer accepted in #769** —
the configurability program's contribution is to make the method that carries it a
**deliberate per-deployment choice with the regression spelled out at the call site**,
never the silent default.

## 2. Commitment-method configuration

### 2.1 The selectable method

Introduce a host-side enum mirroring the existing `sig::SignatureScheme` shape, the
single source of truth for *how a graph was committed*:

```rust
// crates/sparq-zk/src/commit.rs (DESIGN — not implemented)
pub enum CommitmentMethod {
    /// (i) blake3 lexical token only; INV-VL machine-enforced. Today's default.
    StringCanonicalV1,           // zk:poseidon2-rdfc10-v1
    /// (ii) value-first single leaf; lexical hash dropped. DESIGN-ONLY, superseded.
    ValueOnlyV1,                 // zk:poseidon2-valuehook-v1   (NOT recommended; §7)
    /// (iii) dual-leaf value+lexical; INV-VL REMOVED, documented per #769.
    DualLeafV1,                  // zk:poseidon2-dualleaf-v1
}
```

Each variant carries a **distinct `zk:scheme` IRI** so a `RegistryEntry` records
exactly how it was committed and a verifier can fail closed on an unknown/mismatched
method. The method is chosen **at ingest** (issuance time) and is **immutable for that
graph's commitment** — a graph committed under (i) cannot later be proven with a
value-bearing circuit, because its leaf has no `value_component` (§3.4).

### 2.2 Why all three, not just dual-leaf

The maintainer asked to "enable BOTH ways of committing" explicitly because this is a
**research prototype whose purpose is the comparison** (§6, §7). Keeping (i)
implemented is also a hard **backwards-compatibility** requirement: every credential
already issued, every checked-in golden vector, and every existing scan/join/issuer
proof is string-canonical. Dropping (i) would invalidate them. So:

- **(i) string-canonical** stays the conservative, INV-VL-machine-enforced default
  and the back-compat anchor.
- **(iii) dual-leaf** is the value-optimised method for *new* issuance that needs
  cheap value FILTERs while keeping `sameTerm`/`join`/`DISTINCT` identity (#769).
- **(ii) value-only** is retained **as a measured comparison point only** — it is the
  cheapest and the *least safe* (it loses term identity entirely, not just INV-VL),
  and #794 already superseded it as a *recommendation*. It earns its place purely so
  the benchmark + security table can show the full cost/safety frontier (§6/§7). The
  recommendation (§5) is to **never select (ii) for real issuance**; it is a
  research dial.

### 2.3 Ingest-time fail-closed obligations (per method)

| Method | Ingest obligation (host) |
|---|---|
| (i) string-canonical | unchanged — RDFC10 canonicalise, blake3 the token (`encode.rs` as-is) |
| (ii) value-only | parse `VALUE_HOOK = parse(canonical(lexical))`; **fail closed** if the lexical does not canonically parse; **no** lexical hash kept (term identity is lost by construction — §7) |
| (iii) dual-leaf | compute `VALUE_HOOK` and `lexical_component` from **the same canonical bytes** and co-bind them into one leaf, **fail closed** on a parse mismatch (#794 §6). Honest sparq ingest cannot self-desync; only an external malicious issuer committing off-sparq can (§1.C) |

The fail-closed co-binding is the *honest mitigant* for the removed INV-VL on sparq's
own ingest path; it is **not** a substitute for the audit and does **not** bind a
malicious external issuer (named, not hidden — §7, CR-G8).

## 3. Per-method circuit-builder selection

The circuit must match how the data was committed: a value-bearing FILTER member that
recomputes a `value_component` is **unprovable** against a string-canonical leaf (no
value handle exists), and a blake3-token FILTER member is **needlessly expensive**
against a dual-leaf (it ignores the cheap handle). The selection must be **machine-
enforced**, not advisory, or it becomes a soundness footgun.

### 3.1 The selection mechanism (extend the existing `CircuitId` dispatch)

`CircuitId` (`manifest.rs:515`) already names the compiled member and the verifier
re-derives it (`canonical_vk`). Extend it with the value-bearing members and make the
method an input to the resolver:

```text
existing (string-canonical lane):
  FilterInt{d} | FilterSignedInt{md} | FilterDecimal{id,fd} | FilterF64{d}
      -> blake3-token binding (filter_int.nr / filter_signed.nr / filter_float.nr)

new (value-bearing lane, dual-leaf or value-only):
  FilterValue{ datatype_class }          // NO digit-count parameter (the family collapses)
      -> recompute value_component = h3(VALUE_HOOK, DT_CONST, LANG_CONST)
         + B1 range-decomposition of VALUE_HOOK (no modular wrap)
         + B4 canonical-form bind (in-circuit OR honestly downgraded — §3.3)
```

The resolver gains a guard: **`(CommitmentMethod, CircuitId)` must be a legal pair.**

| Commitment method | Legal FILTER circuit family | Identity ops (`scan`-row eq, `join_eq`, DISTINCT, sameTerm) read |
|---|---|---|
| (i) string-canonical | `FilterInt`/`FilterSignedInt`/`FilterDecimal`/`FilterF64` (blake3-token) only | the full leaf `Enc` (= `h2(LITERAL, blake3 token)`) — INV-VL holds |
| (iii) dual-leaf | `FilterValue{dt}` (value lane) for value FILTERs; the blake3 family still legal for term FILTERs | the **`lexical_component`** ONLY — reject-list (v): no identity op may read `value_component` (#794 §8) |
| (ii) value-only | `FilterValue{dt}` only | the value leaf — **term identity is unavailable** (no lexical component exists); identity ops MUST be rejected at plan time, not silently answered |

This table is the load-bearing safety property of the whole program: **the verifier's
`derive_*_id` + `canonical_vk` must refuse a `(method, circuit)` pair outside it**, so
a prover can never (a) prove a value FILTER against a method that did not commit a
value handle, or (b) route an identity operator at the `value_component` on a
many-to-one datatype. Reject-list item **(v)** (#794 §8) is enforced here
*structurally* — by the resolver refusing to bind an identity operator to a
`FilterValue` member or to the `value_component` slot — not by prose.

### 3.1.1 The lane↔member allow-list, as shipped

`(method, circuit)` legality is about the leaf SHAPE, so it admits a V1 graph's value
lane and a V2 graph's alike. It therefore does not decide which LANE a given
`datatype_const` belongs to — and `datatype_const` is a PUBLIC INPUT no shipped member
pins. `dispatch::check_lane_const_for_member` adds that second, finer decision as an
AFFIRMATIVE allow-list over DECODED field elements:

| `datatype_const` | may ride | available for a NEW proof |
|---|---|---|
| V1 `xsd:integer`, V1 `xsd:boolean` (its own `derive_key` constant) | `filter_value_dl_int` | no — `MethodUse::LegacyAlreadyIssued` only |
| V1 `xsd:double` · V2 `xsd:double` | `filter_value_dl_f64` | V2 only |
| V1 / V2 `xsd:decimal@scale=fd`, `fd ∈ 1..=19` | `filter_value_dl_decimal` | V2 only |
| V1 / V2 `xsd:dateTime`, `xsd:date` | `filter_value_dl_datetime` | V2 only |
| V2 `xsd:integer` | `filter_value_dl_int` | yes |
| V2 `xsd:boolean` | `filter_value_dl_bool` ONLY | yes |

Three properties make it a boundary rather than a lookup:

1. **affirmative** — a constant no shipped lane derives is refused on EVERY member, so
   the bit-domain member opens the V2 boolean lane and nothing else, and the shared
   integer member does not open it;
2. **decoded** — the comparison is on field elements, and a value-lane `FieldHex` must
   additionally be the CANONICAL rendering of what it decodes to, because the permissive
   parser accepts uppercase, `0x`-less, short and at-or-above-modulus spellings of the
   same element;
3. **one function, four call sites** — the value-lane builders, every value-lane
   `Prover.toml` renderer, the method-checked prove entry (which also checks the
   rendered TOML carries the checked inputs' constant) and the verifier's structural
   pre-filter all call it. The raw `CircuitProver::{prove, prove_in, gen_witness*}`
   entries deliberately do NOT: they take a `CircuitId` and TOML bytes, so a caller
   rendering its own TOML is outside this boundary (and the in-circuit vectors rely on
   that to test what the RELATION refuses rather than what the host permits).

Scope, stated plainly: (1)–(3) are HOST-side refusals plus one structural pre-filter
refusal. The value-lane verifier GATES stay closed until the verifier authenticates the
commitment method and pins member, lane, scale and bound itself (sq-j506), and
`LegacyAlreadyIssued` is a caller assertion, not evidence of issuance history. Nothing
here is an audited soundness property (CR-G8 / sq-qhy4).

### 3.2 The value-bearing FILTER family collapses

A key structural simplification (carried from #794 §4 / the gate-reduction doc §5):
the value lane needs **one relation per datatype class** (`FilterValue{integer}`,
`{decimal}`, `{double}`, …), **not** per digit count. The blake3 lane needs
`filter_int_d{1..4}`, `filter_signed_int_d{2,4}`, etc. because the digit-count witness
`[u8; D]` pins the member; the value lane has no digit array, so the
per-`D`/`MD`/`(ID,FD)` family disappears. This also **closes a leakage channel**: the
digit-count member selection no longer leaks `ceil(log10(value))` (#794 §4;
`filter_int.nr:26-28`). The benchmark (§6) must record both the new collapsed members
**and** the legacy family side by side so the comparison is honest.

### 3.3 The B1/B4 obligations the builder MUST instantiate (not just name)

This is the single most important correctness obligation carried from #794 §8 and the
gate-reduction §3.4 must-keep set. A value-bearing FILTER member is **only** sound if
it instantiates, in-circuit:

- **B1 — range-decomposition of `VALUE_HOOK` with no modular wrap.** The witnessed
  `VALUE_HOOK` MUST be proved to lie in its typed domain (magnitude `< 2^64` for
  integer/signed; scaled `ID+FD ≤ 19` for decimal; a well-formed IEEE pattern for
  double/float) **before** the comparison, and **the SAME range-decomposed value MUST
  feed both the A1 operand binding and the typed comparison** (#794 §4). Omitting B1
  silently downgrades in-range-ness from a prover-side circuit guarantee to an
  issuer/ingest assumption — **reject-list (i)**.
- **B4 — canonical-form bind.** With no digit array to attach the no-leading-zero /
  no-`-0` / canonical-scale asserts to, the member MUST **either** re-introduce a
  constrained canonicalising re-derivation of `VALUE_HOOK` in-circuit (costs gates —
  must be `bb gates`-measured), **OR** the docs/SKILL/README MUST honestly
  re-classify B4 to an issuer/ingest assumption — a strictly larger escalation
  (#794 §5.4). The configurability program's stance: **make B4's placement
  (in-circuit vs ingest) itself a recorded per-method property** so the security table
  (§7) states it truthfully for each method rather than leaving it implicit.

The circuit-builder selection is therefore not just "pick the cheap member" — it is
"pick the member **whose instantiated constraint set matches the commitment method's
recorded soundness posture**," which is exactly what §7 tabulates.

### 3.4 Backwards compatibility

The blake3-token members stay **compiled and composable forever** (the
gate-reduction §3 "Phase 4 — deprecate, don't delete"). A single-lane (string-
canonical) graph can *only* use the blake3 path; a dual-leaf graph can use *either*
the value lane (cheap value FILTER) or the blake3 family (term FILTER on the
`lexical_component`). The resolver's `(method, circuit)` guard (§3.1) is what makes
mixing safe: it never lets a dual-leaf proof route an identity op at the value lane,
and never lets a value FILTER target a method with no value handle.

## 4. Pluggable signature schemes

### 4.1 The seam already exists

`sig::SignatureScheme` (`sig.rs:93`) is the modularity swap-point the maintainer's
prior codebases used; its doc already names BBS+ / SD-JWT-VC / post-quantum as
"parallel variants." Today it has one variant, `Poseidon2SchnorrV1` (Schnorr over
Baby-JubJub, Poseidon2 challenge), chosen because Baby-JubJub's base field **is**
BN254's scalar field, so the verifier-side signature is *exactly* the one an
in-circuit verifier checks (`sig.rs:17-27`) — that is why this scheme, not Ed25519,
is the v1 choice. The configurability program **does not change the scheme**; it
**hardens the trait boundary** so a second scheme can be added without touching the
registry/verifier call sites.

### 4.2 The trait/config boundary to introduce

The current enum is a closed match; the prior-codebase pattern is an open trait. The
recommended boundary, additive over the existing enum:

```rust
// crates/sparq-zk/src/sig.rs (DESIGN)
pub trait IssuerSignatureScheme {
    /// The zk:cryptosuite IRI this scheme records in the registry.
    fn cryptosuite_iri(&self) -> &str;
    /// Verify a signature over a domain-separated commitment message (verifier-side,
    /// public data only — the relying-party path).
    fn verify(&self, pk: &VerificationKey, m: &Fr, sig: &SignatureBytes) -> bool;
    /// Whether this scheme has an in-circuit verifier member (i.e. the privacy
    /// upgrade "signed by SOME key in K" is reachable). Schnorr-BBJ: yes
    /// (hidden_issuer_d{depth}). EdDSA/ECDSA over a non-embedded curve: not without
    /// a new circuit member + curve gadget.
    fn in_circuit_member(&self) -> Option<CircuitId>;
}
```

The boundary's value is that it surfaces the property that actually matters for this
estate: **whether a scheme is verifiable in-circuit at all.** Schnorr-over-Baby-JubJub
is in-circuit-friendly *by construction*; a generic EdDSA/ECDSA-over-secp or an
RSA/lattice VC signature is **not** without a dedicated, expensive new circuit member
(or it is checked verifier-side only, sacrificing the hidden-issuer privacy upgrade).
That distinction is the honest discriminator between the schemes and is what the
benchmark (§6) and security table (§7) report.

### 4.3 Candidate schemes (the seam, honestly scoped)

| Scheme | Verifier-side check | In-circuit member feasible? | Honest note |
|---|---|---|---|
| `Poseidon2SchnorrV1` (Baby-JubJub) | **IMPLEMENTED** (`sig::verify`) | **yes** — `hidden_issuer_d{depth}` exists (`CircuitId::HiddenIssuer`) | the v1 scheme; embedded curve makes the in-circuit path native |
| EdDSA (Ed25519) over a VC | feasible (host verify) | **expensive** — non-embedded curve; needs a new gadget + vk | the `eddsa-rdfc-2022` bridge (§5) verifies the VC's signature *off-circuit at ingest*, then re-commits under the sparq scheme |
| ECDSA (secp256r1/k1) over a VC | feasible (host verify) | **expensive** — same as EdDSA | `ecdsa-rdfc-2019` bridge, ingest-side only |
| BBS+ / `bbs-2023` | feasible (host verify, selective-disclosure-native) | research-grade; not in-repo | the natural selective-disclosure match for VCs (§5.3); a real seam to add, not an in-circuit member for v1 |
| post-quantum (ML-DSA / lattice-BBS) | feasible (host verify) | not feasible in-circuit at research grade | listed for the seam's completeness, not scoped for impl |

The honest framing: **only `Poseidon2SchnorrV1` is implemented; the others are
trait-boundary slots.** The most a v1 program should *implement* is the trait + a
second *verifier-side-only* scheme (EdDSA or ECDSA over a VC, §5) so the cryptosuite
comparison has a real second data point — anything in-circuit beyond Schnorr-BBJ is a
much larger, separately audited effort and stays a seam.

## 5. W3C Verifiable Credential cryptosuite commitments

### 5.1 What a VC cryptosuite is, and the two distinct signatures in play

A W3C Data-Integrity VC carries a `proof` produced by a named **cryptosuite**:
`ecdsa-rdfc-2019`, `eddsa-rdfc-2022`, `ecdsa-sd-2023`, or `bbs-2023` (the
selective-disclosure suites) — all of which RDFC10-canonicalise the credential, then
hash and sign (sources below). **There are two different signatures to keep distinct:**

1. **The VC's own Data-Integrity proof** — produced by the issuer under a W3C
   cryptosuite (e.g. `eddsa-rdfc-2022`). This is what makes the VC a valid VC.
2. **sparq's commitment signature** — `Poseidon2SchnorrV1` over `C(G)` — which is
   what the in-circuit / verifier-side query proof binds to.

These are **not the same** and sparq does **not** verify a VC's Ed25519/ECDSA proof
*in-circuit*. The feasible scope is a **bridge**, not an in-circuit VC verifier.

### 5.2 Feasible scope: the ingest-time cryptosuite bridge

```text
VC (issuer-signed under eddsa-rdfc-2022 / ecdsa-rdfc-2019 / bbs-2023)
   │
   ├─ ingest: verify the VC's Data-Integrity proof OFF-circuit (host), under the
   │           named cryptosuite, against the issuer's published key.   [fail closed]
   │
   ├─ RDFC10-canonicalise the credential graph (sparq already does this — canon.rs)
   │
   ├─ commit under the selected CommitmentMethod (§2): string-canonical or dual-leaf
   │
   └─ sparq-sign C(G) under Poseidon2SchnorrV1, recording BOTH:
        zk:cryptosuite     = the sparq scheme IRI  (what the query proof checks)
        zk:sourceCryptosuite (NEW) = the VC's W3C cryptosuite IRI  (provenance/back-compat)
```

The registry already carries `zk:cryptosuite` (`registry.rs:42,99`); the bridge adds a
**`zk:sourceCryptosuite`** provenance slot recording *which W3C suite the source VC
used*. This is what lets the maintainer's requested **performance comparison** (cost
of ingesting + re-committing a VC under each W3C suite) and **backwards-compatibility
discussion** (which suites round-trip, which lose selective disclosure) happen
honestly, **without** claiming sparq verifies the VC's cryptographic proof in a query
circuit.

### 5.3 What is feasible vs not (honest scope boundary)

- **Feasible (v1 program):** ingest a VC, verify its `eddsa-rdfc-2022` /
  `ecdsa-rdfc-2019` proof **off-circuit** at the host, re-commit + sparq-sign, record
  the source cryptosuite. This is a clean, achievable bridge and gives the cryptosuite
  comparison real data.
- **Feasible but research-grade:** `bbs-2023` ingest. BBS is the *natural* match —
  it is selective-disclosure-native, so the issuer's base proof can be reused to
  derive a proof over a disclosed subset (source below). This aligns with sparq's
  per-leaf disclosure model, but a real BBS verifier is not in-repo; scope it as a
  seam + a documented intent, not a v1 deliverable.
- **NOT feasible at research grade:** verifying *any* of these VC proofs
  **in-circuit**. The query circuit binds to sparq's `Poseidon2SchnorrV1` commitment
  signature; the VC's own non-embedded-curve proof is checked at ingest only. Claiming
  in-circuit VC-proof verification would be an overclaim and is explicitly excluded.
- **Backwards-compatibility note:** a non-selective-disclosure suite
  (`ecdsa-rdfc-2019`, `eddsa-rdfc-2022`) signs the *whole* canonical credential, so
  re-committing under sparq's per-graph `C(G)` is faithful but disclosure is
  all-or-nothing at the VC layer; the *sparq* layer's per-leaf scan/FILTER disclosure
  is independent of that. The selective-disclosure suites (`ecdsa-sd-2023`,
  `bbs-2023`) match sparq's disclosure model more naturally.

### 5.4 Recommendation for the VC axis

Implement the **off-circuit ingest bridge for `eddsa-rdfc-2022` + `ecdsa-rdfc-2019`**
first (smallest, real, gives two cryptosuite data points), record
`zk:sourceCryptosuite`, and bench the *ingest+re-commit* cost per suite. Treat
`bbs-2023` as the next seam. Do **not** scope in-circuit VC verification.

## 6. Per-configuration benchmark plan

### 6.1 The configuration matrix

The benchmark axis is `(commitment-method × circuit-family × signature-scheme)`,
plus the VC-bridge ingest sub-axis. Driven by the **existing**, snapshot-anchored
harness — `bench/zk-compose/scripts/gate_counts.sh` reads the member list from
`crates/sparq-zk-compose/tests/gate_count_snapshot.json` so the bench JSON and the
regression gate can never drift (`gate_counts.sh:8-14`, the sq-ifur fix). The metric
is **`bb gates -s ultra_honk` `circuit_size`** — ground truth (noir-optimisation
SKILL §1: *"Always run `bb gates` before claiming a saving"*; `nargo info` is
misleading alone).

| Axis | Points to measure |
|---|---|
| commitment method | (i) string-canonical · (iii) dual-leaf · (ii) value-only (the comparison floor) |
| circuit family | per method: the legal FILTER members (§3.1) — blake3-token family vs collapsed `FilterValue{dt}` family; the **B1 range-decomposition and any B4 in-circuit re-derivation MUST be included** in the measured member (#794 §4, gate-reduction §5) |
| signature scheme | `Poseidon2SchnorrV1` verifier-side + its `hidden_issuer_d{depth}` in-circuit member; (if the EdDSA/ECDSA verifier-side seam lands) host-verify cost per suite |
| VC bridge (ingest) | host-side ingest+verify+re-commit cost per W3C cryptosuite — **NOT a gate count**; a wall-time/ingest measurement, recorded in `bench/`, explicitly **NON-canonical on this work box** |

### 6.2 Honesty discipline for the numbers

- **Every gate count is re-baselined into the snapshot** (`gate_count_snapshot.json`)
  and `bench/zk-compose/gate_counts_latest.json` together — the
  `bench_json_matches_snapshot` parity test (`gate_count.rs`) forces both, so a
  landed member's cost is a checked-in, regression-gated fact, not a prose figure.
- **No gate number appears in any markdown** (this record, SKILL, README) — the
  perf-numbers gate (`check-no-perf-numbers.py --enforce`) keeps it that way; the
  numbers live in `bench/` + the snapshot JSON.
- **Canonical vs non-canonical, stated explicitly:** gate counts are deterministic
  (compile + `bb gates`) and are **canonical** once measured on the pinned toolchain
  (the snapshot records `bb`/`nargo` versions). The **VC ingest wall-times** and any
  prover/verifier latency measured *on this EC2 work box* are **NON-canonical** and
  must be labelled so (MEMORY: project-ec2-execution-env). The §3.3 projection that a
  collapsed value member lands "far below" the blake3 family is a **direction**, not a
  claim, until the member is compiled and `bb gates`-measured.
- **The comparison is the point.** The deliverable is a *table* per configuration —
  cost AND the §7 security posture together — so the maintainer can see the full
  cost/safety frontier (the explicit purpose of "benchmarks with each configuration"
  in #769), not a single headline number.

## 7. Security-properties write-up (per configuration)

> Predicate-form caveat on every row: these are properties **to be reviewed under
> external sign-off (sq-qhy4)**, not guarantees. The estate is **remediated but NOT
> externally audited**. Each row states what is **NOT** guaranteed and what an
> auditor MUST verify.

### 7.1 Per-commitment-method posture

| Property | (i) string-canonical | (iii) dual-leaf | (ii) value-only |
|---|---|---|---|
| INV-VL (value = parse(committed lexical)) | **machine-enforced in-circuit** against an arbitrary committer (`filter_int.nr:67-92`) | **NOT machine-enforced — REMOVED.** Value-FILTER lane is issuer-honesty-trusted for value↔lexical agreement (#769 accepted; CR-G8) | **NOT applicable — there is no lexical component**; INV-VL is meaningless and term identity is lost entirely |
| term identity (`sameTerm`/`DISTINCT`/`join`) | sound — identity ops read the full blake3-token leaf | sound **only if** reject-list (v) is structurally enforced (identity ops read the `lexical_component` only, never the many-to-one `value_component`) — §3.1 | **unavailable** — must be rejected at plan time, not silently answered on a collapsing value handle |
| in-range / single-encoding (B1/B4) | machine-enforced via digit array | sound **only if** B1 range-decomposition + B4 canonical bind are **instantiated in-circuit**, else honestly downgraded to an issuer/ingest assumption (§3.3) | same B1/B4 obligation, with **no** lexical fallback for identity |
| desync residual | none (single preimage) | bounded to a malicious **trusted** issuer; sparq's own fail-closed ingest cannot self-desync; no untrusted party can exploit it (§1.C, #794 §6) | identity desync is total by construction |
| recommendation | the conservative default + back-compat anchor | the value-optimised method for new issuance that needs cheap value FILTERs **and** identity; risk accepted per #769 | **research dial only** — never select for real issuance (§2.2) |

### 7.2 Per-signature-scheme posture

- `Poseidon2SchnorrV1`: closes the unsigned-commitment hole verifier-side
  (`sig.rs:6-15`); v1 placement reveals **which** issuer signed (verifier-side clear
  key) — the hidden-issuer in-circuit member (`hidden_issuer_d{depth}`) removes that
  leak but is itself unaudited (CR-G1). Signing is **NOT asserted constant-time**
  (arkworks residual, CR-G5/`sig.rs:46-74`); rated LOW only because signing is
  issuance-side. None of this is a production guarantee.
- Any added scheme (EdDSA/ECDSA/BBS): verifier-side host check only; **no** in-circuit
  member unless one is built + separately audited. The trait's `in_circuit_member()`
  returning `None` is the honest, machine-readable statement of that limit (§4.2).

### 7.3 Per-VC-cryptosuite posture

- sparq verifies the source VC's Data-Integrity proof **off-circuit at ingest only**;
  it does **NOT** verify it in a query circuit (§5.3). The query proof binds to
  sparq's own `Poseidon2SchnorrV1` commitment signature, not to the VC's proof.
- `zk:sourceCryptosuite` is **provenance**, not a re-verifiable in-proof property.
- The selective-disclosure suites (`bbs-2023`, `ecdsa-sd-2023`) match sparq's
  per-leaf disclosure model; the plain suites are all-or-nothing at the VC layer
  (§5.3). No selective-disclosure *soundness* property is claimed for the bridge.

### 7.4 What an external auditor MUST verify (carried, not re-decided)

The CR-G8 obligations (already on `main`, #794) apply unchanged to the dual-leaf
method: (1) the relocated operand binding still binds the **scan-committed** triple
and the SAME range-decomposed `VALUE_HOOK` feeds both binding and comparison; (2) B1
+ B4 are in-circuit OR the docs say they rest on issuer honesty; (3) reject-list (v)
is structurally enforced — done here by the §3.1 `(method, circuit)` resolver guard,
which the auditor MUST confirm is fail-closed; (4) the removed-INV-VL residual is
bounded to a malicious trusted issuer; (5) the double/float IEEE-bit ingest
canonicalisation is total. **New for the configurability surface** (proposed gap
**CR-G9**, §8): the auditor MUST verify the `(method, circuit, signature)`
compatibility matrix is **enforced fail-closed end-to-end** — that no proof verifies
under a `CircuitId` illegal for its recorded `zk:scheme`, and that an unknown/mismatched
method or cryptosuite IRI rejects rather than defaults.

## 8. Proposed gap-register addition (CR-G9) — applied by a future pass, not here

This record does **not** edit `compliance/cryptoreview/gap-register.md` (CR-G8 is
already correct on `main`; no conflict). It **proposes** one new row for a future
compliance pass to apply, tracked as the doc bead in §9:

> **CR-G9 — The commitment-method × circuit × signature compatibility matrix is a new
> fail-closed soundness surface.** Selectable commitment methods (string-canonical /
> dual-leaf / value-only) and a pluggable signature trait mean a verifier MUST refuse
> any `(zk:scheme, CircuitId)` or `(zk:scheme, zk:cryptosuite)` pair outside the §3.1
> legal matrix — a value-bearing FILTER against a method with no value handle, an
> identity operator routed at a `value_component`, or an unknown method/cryptosuite
> IRI defaulting instead of rejecting are each a soundness break. **OPEN /
> EXTERNAL-REQUIRED**, folded into the sq-qhy4 pass; the design is
> `research/zk-configurable-commitment-design.md` (this record). [OPUS-4.8]

## 9. Implementation bead breakdown (orchestrator creates them)

Ordered; each is a future bead under epic **sq-1s2** (ZK query-proof build-out), all
**audit-gated** behind **sq-qhy4** for any soundness reliance, all carrying the
`[OPUS-4.8]` marker + Opus 4.8 `Co-Authored-By` trailer, both-feature-state clippy +
tests, and a `bb gates` re-baseline where a member changes.

1. **Doc/obligation registration (land FIRST, doc-only).** Add the proposed **CR-G9**
   row (§8) to `compliance/cryptoreview/gap-register.md`; add obligation-/negation-
   framed caveats to the ZK `SKILL.md` + `crates/sparq-zk/README.md` naming the
   configurable methods and their per-method INV-VL posture. Gates: privacy-claims +
   perf-numbers + markdownlint + typos. (Depends on this record being accepted.)
2. **`CommitmentMethod` config plumbing (host).** Add the `CommitmentMethod` enum +
   its `zk:scheme` IRIs to `crates/sparq-zk/src/commit.rs`/`registry.rs`; record the
   method on `RegistryEntry`; fail-closed on unknown method. String-canonical stays
   the byte-unchanged default. (Depends on 1.)
3. **Dual-leaf host encoding + same-leaf co-binding at ingest.** Implement the
   dual-leaf leaf shape in `encode.rs`/`commit.rs` with the fail-closed co-binding
   (#794 §6); this is largely the existing `sq-j506` scope — **align/merge with it**
   rather than duplicate. (Depends on 2.)
4. **Value-only host encoding (comparison-only).** Implement the value-only leaf as a
   `CommitmentMethod` variant **flagged not-for-production** (§2.2); exists for the
   §6/§7 comparison. (Depends on 2; independent of 3.)
5. **`FilterValue{dt}` circuit member(s) + B1/B4 instantiation.** Add the collapsed
   value-lane FILTER relation to `zk/compose/compose_core/` + the thin bin members;
   instantiate B1 range-decomposition and B4 canonical bind (or document the honest
   downgrade); add `CircuitId::FilterValue` + `ProofInputs::FilterValue`. (Depends on
   3.)
6. **Per-method circuit-builder selection + fail-closed `(method, circuit)` guard.**
   Extend the verifier's `derive_*_id`/`canonical_vk` resolver to enforce the §3.1
   legal matrix; structurally reject identity operators at the value lane (reject-list
   (v)); reject illegal pairs. (Depends on 5.)
7. **Identity-op + desync regression guard (expanded).** Tests proving no identity op
   reads `value_component` on a dual-leaf graph, and that value-only graphs reject
   identity ops at plan time; expand the existing desync guard to double/float/decimal
   many-to-one cases. (Depends on 6.)
8. **Pluggable signature trait boundary.** Refactor `sig::SignatureScheme` into the
   `IssuerSignatureScheme` trait (§4.2) with `Poseidon2SchnorrV1` as the first impl +
   `in_circuit_member()`; no behaviour change to the existing scheme. (Independent;
   can run parallel to 3–7.)
9. **Second verifier-side signature scheme (EdDSA or ECDSA over a VC).** Add one
   host-verify-only scheme behind the trait so the cryptosuite comparison has a real
   second point; no in-circuit member. (Depends on 8.)
10. **W3C VC ingest cryptosuite bridge.** Off-circuit verify of `eddsa-rdfc-2022` +
    `ecdsa-rdfc-2019` VC proofs at ingest, re-commit under the selected method, record
    `zk:sourceCryptosuite` (§5.2); `bbs-2023` as a follow-up seam. (Depends on 2, 9.)
11. **Per-configuration benchmark harness.** Extend `bench/zk-compose/` to sweep the
    `(method × circuit × signature)` matrix + the VC-ingest sub-axis; re-baseline the
    snapshot; emit the cost+security comparison table (§6/§7). Honest
    canonical-vs-non-canonical labelling. (Depends on 5, 6, 10.)
12. **Security-properties write-up (doc).** Finalise the §7 per-configuration table as
    a SKILL/README-linked record once the members are measured; all NEGATED/obligation-
    framed; ZK NOT externally audited. (Depends on 11.)

## 10. Open questions that genuinely need the maintainer

1. **Value-only retention.** Confirm that keeping method (ii) value-only **purely as a
   measured comparison point** (flagged not-for-production, §2.2) is wanted, or whether
   it should be dropped from the program entirely (it adds a circuit member + ingest
   path that no real deployment should select).
2. **B4 placement, per method.** For dual-leaf and value-only: is the gate cost of an
   **in-circuit** canonicalising re-derivation of `VALUE_HOOK` acceptable (keeping
   single-encoding a prover-side guarantee), or do you accept the **honest downgrade**
   to an issuer/ingest assumption (§3.3, #794 §10 Q5)? This must be `bb gates`-measured
   before deciding, but the *posture preference* is yours.
3. **VC bridge depth.** Is the **off-circuit ingest bridge** (verify VC proof at
   ingest, re-commit, record provenance) the right scope (§5.2/§5.4), or do you want
   `bbs-2023` selective-disclosure handling in the first slice (substantially larger)?
4. **Signature trait — open vs enum.** Do you want the full open `IssuerSignatureScheme`
   trait now (§4.2, matches your prior codebases), or is the existing closed enum +
   a second verifier-side variant sufficient for the comparison the program needs?
5. **`zk:sourceCryptosuite` vocabulary.** Confirm the new provenance predicate name /
   namespace (`https://sparq.dev/ns/zk#sourceCryptosuite`) before it is minted into
   the registry vocabulary (it becomes a public IRI).
6. **CR-G9 inline now?** Do you want CR-G9 (§8) applied to the gap-register in bead 1,
   or left as a proposal in this record until the configurability surface actually
   lands?

## 11. Verdict

The maintainer's program is **largely a matter of plumbing three seams the prior work
already left** (`zk:scheme`, `sig::SignatureScheme`/`zk:cryptosuite`, and the
`CircuitId` dispatch) into a coherent, **fail-closed** `(method × circuit ×
signature)` matrix, plus building the value-bearing FILTER members the dual-leaf
method needs and an off-circuit VC ingest bridge. The dual-leaf method's removed
INV-VL invariant is **real, accepted at research grade (#769), and documented per
method** — never hidden. **Nothing here is a security guarantee; the whole estate is
remediated but NOT externally audited (sq-qhy4), and every soundness property is an
obligation an external accredited cryptographer MUST verify** before any production
reliance.

---

## Sources (external prior art)

- W3C, *Verifiable Credential Data Integrity 1.0* — <https://www.w3.org/TR/vc-data-integrity/>
- W3C, *Data Integrity ECDSA Cryptosuites v1.0* (`ecdsa-rdfc-2019`, `ecdsa-sd-2023`) — <https://www.w3.org/TR/vc-di-ecdsa/>
- W3C, *Data Integrity EdDSA Cryptosuites v1.0* (`eddsa-rdfc-2022`) — <https://www.w3.org/TR/vc-di-eddsa/>
- W3C, *Verifiable Credentials Overview* (`bbs-2023` selective disclosure) — <https://www.w3.org/TR/vc-overview/>
- W3C, *Verifiable Credentials Data Model v2.0* — <https://www.w3.org/TR/vc-data-model-2.0/>

In-repo foundations (verified): `research/zk-field-native-encoding.md` (#794, the
dual-leaf FINAL design), `research/zk-age-gatecount-reduction.md` (the must-keep
constraint set), `research/zk-signed-credential-representation-design.md` (the
`jeswr/sparql_noir` prior-codebase grounding), `crates/sparq-zk/src/{encode,commit,sig,registry}.rs`,
`crates/sparq-zk-compose/src/{manifest,verifier}.rs`, `compliance/cryptoreview/gap-register.md`
(CR-G8), `bench/zk-compose/scripts/gate_counts.sh`.
