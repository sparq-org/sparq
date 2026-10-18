<!-- [OPUS-4.8] Security-properties write-up authored by Opus 4.8 (1M context) (Fable
unavailable) — re-review when Fable returns. Bead sq-pkrl. -->
# Security properties of the configurable ZK commitment surface

Maintainer-review record for bead **sq-pkrl** — the per-method security-properties
write-up the maintainer asked for in **#769** ("benchmarks with each configuration and a
write-up of the security properties"). It sits on top of the merged commitment-method
registry (**#891 / sq-zzxt**) and the design record
[`research/zk-configurable-commitment-design.md`](./zk-configurable-commitment-design.md);
it does **not** restate or contradict that design or its dual-leaf foundation
([`research/zk-field-native-encoding.md`](./zk-field-native-encoding.md), #794). Its job is
to state, per configuration, **what each commitment method does and does NOT guarantee** —
written as obligations and negations, never as settled guarantees.

## 0. Honesty framing (load-bearing — read first)

This is the audit surface, so the honesty mandate is stated up front and applies to every
row below:

- **The whole ZK estate is remediated but NOT externally audited.** `SECURITY.md` publishes
  the `sparq-zk` / `sparq-zk-compose` verifier as *"remediated, but NOT externally
  audited"*: the original audit found it broken, the binding layer landed, and the internal
  re-audit found it *"sound as landed for the threat model the prior audit assumed"* — **but**
  no external accredited cryptographer has reviewed it. The external sign-off is gap
  **CR-G1** / bead **sq-qhy4** (P0, OPEN). **Do not present any result from this estate as a
  production-grade guarantee** until sq-qhy4 completes.
- **No production security / privacy / soundness claim is made anywhere in this record.**
  Every property below is either machine-checked *within the unaudited estate* (so it is an
  obligation an external auditor must confirm, not a settled fact) or explicitly a
  trust-assumption / negation.
- **The dual-leaf INV-VL downgrade is a documented trust assumption, not a proven
  property.** Selecting `dual-leaf` moves value↔lexical agreement on the value-FILTER lane
  from a machine-enforced in-circuit invariant to **trusted-issuer honesty**. The maintainer
  **explicitly accepted this at research grade in #769**. This record spells out the threat,
  who is trusted, and the residual guarantees — it does **not** argue the downgrade away.
- **`sparq-mpc` is out of scope and carries no guarantee.** It is honest-majority
  semi-honest only, with the malicious-security / collaborative-proof core deferred (CR-G7).
  Nothing here relies on it.
- **No hard-coded performance numbers.** Every cost statement is a *direction* or a
  *measurement obligation* discharged by `bb gates` and the `gate_count` snapshot. Any
  figure produced on this EC2 work box is **NON-canonical** (it is a work box, not a CI
  runner).

## 1. What is IMPLEMENTED today vs PENDING (ground truth)

The brief's premise — *"the merged `commit.rs` contains the three methods
`StringCanonicalV1`, `DualLeafV1`, `ValueOnlyV1`"* — is **correct in the main, with two
corrections that matter for the security posture**, verified against the code at
`crates/sparq-zk/src/commit.rs` on `origin/main` (#891):

1. **`ValueOnlyV1` is feature-gated, compiled out by default.** It exists only behind the
   OFF-by-default `commitment-value-only` cargo feature
   (`commit.rs:91` `#[cfg(feature = "commitment-value-only")] ValueOnlyV1`). With the feature
   off, its `zk:scheme` IRI is **unknown and rejected** by `from_scheme_iri` — a normal build
   cannot select it. The feature name is the warning. This is stronger than "a research dial";
   it is a research dial that does not exist in a default build.
2. **What landed is CONFIG ONLY — no leaf-shape, no circuit, no encoding change.** #891 added
   the `CommitmentMethod` enum, its distinct `zk:scheme` IRIs, fail-closed parse, and the
   `RegistryEntry::method()` plumbing (the method is now set at construction —
   `new_already_issued` / `new_for_issuance` / `issued` — and covered by the issuer signature). The string-canonical
   encode/commit pipeline (`encode.rs`/`commit.rs::commit_canonical`) is **byte-unchanged**;
   the dual-leaf and value-only **leaf encodings are NOT implemented** (impl bead sq-j506),
   and the `(method, circuit)` dispatch is NOT implemented (sq-cfmv). The 28-circuit
   `gate_count` snapshot and `forge_gates` are byte-stable.

Implemented vs pending, precisely:

| Component | State | Where |
|---|---|---|
| `CommitmentMethod` registry (enum + IRIs + fail-closed parse + `method()` + construction-time method) | **IMPLEMENTED** (#891 / sq-zzxt, CLOSED) | `crates/sparq-zk/src/commit.rs`, `registry.rs` |
| string-canonical leaf encode + commit + its in-circuit FILTER members | **IMPLEMENTED** (the only end-to-end method today) | `encode.rs`, `commit.rs`, `zk/compose/.../filter_*.nr` |
| dual-leaf / value-only **leaf encoding** (host) | **PENDING** — sq-j506 (audit-gated), blocked on the dual-leaf circuit | not in tree |
| dual-leaf **circuit member** (value-FILTER over the value handle) | **PENDING** — sq-xojl | not in tree |
| `(method, circuit)` **fail-closed dispatch matrix** | **PENDING** — sq-cfmv | not in tree |
| pluggable **signature-scheme seam** (open trait, off-circuit) | **PENDING / in flight** — sq-1hsl | `sig.rs` is a closed enum today |
| W3C **VC cryptosuite ingest bridge** | **DEFERRED** — design-only (§5 of the design record) | not in tree |
| per-config **benchmark matrix** | **PENDING** — sq-ot3x | `bench/zk-compose/` harness exists |

**Brief-premise correction (dependency).** The brief states the circuit beads
*"sq-cfmv / sq-xojl [are] blocked on #637"*. That is not what the bead graph records: both
sq-cfmv and sq-xojl **depend on sq-zzxt (the registry, now CLOSED)**, not on #637. #637
(`sq-mslu`, the general `xsd:double` decimal→IEEE-754 conversion FILTER building block) is
**still OPEN** and is *related* — the dual-leaf double/float handling may obsolete its
in-circuit RNE parser for the *comparison* path (design §10 Q3) — but it is **not** the
recorded blocker for the circuit work. The honest statement is: the circuit beads are gated
on the registry (now landed) for their dispatch input, and on the external audit (sq-qhy4)
for any soundness reliance; #637 is an adjacent, possibly-superseded building block, not a
hard prerequisite.

## 2. The threat model these properties are stated against

So "guarantees" and "downgrades" below are unambiguous, the parties are:

- **Untrusted prover / relying party.** Constructs query proofs from committed graphs;
  may try to forge a false answer, route an operator illegally, or invent triples. The
  verifier's soundness obligations are stated **against this party** — it is the adversary
  every "machine-enforced" property must hold against.
- **Trusted issuer.** Signs `C(G)` under the sparq commitment signature; a graph's facts
  are trusted *because* a key in the disclosed key-set `K` signed them. A **malicious**
  trusted issuer is one that signs a credential whose internal value/lexical components
  disagree. The INV-VL downgrade is exactly a regression in what a malicious *trusted* issuer
  can do — it does **not** widen what an untrusted party can do.
- **External auditor (sq-qhy4).** The party who must confirm every "machine-enforced"
  property actually holds in the circuits and the binding layer. Until they sign off, every
  such property is an **obligation**, not a fact.

The signed-commitment hole that the issuer signature closes (an unsigned, prover-invented
`C(G)`, a truncated-leaf suppression, a key-not-in-`K` signature) is described in
`sig.rs`; it is **orthogonal** to the commitment-method axis and applies identically to all
three methods.

## 3. INV-VL — the invariant at the centre of the downgrade

**INV-VL** = *"the compared value equals `parse(committed lexical)`, enforced in-circuit
against an arbitrary committer (even a malicious trusted issuer)."* It is the property that
makes a value FILTER and a term-identity operation answer about *the same thing*.

- **Today (string-canonical) it is machine-enforced.** The current
  `filter_int` / `filter_signed` / `filter_decimal` / `filter_float` members derive both the
  compared value and the operand binding from **one** witnessed digit array
  (`filter_int.nr:67-92`, `filter_signed.nr:150-180`), so a prover cannot make the value and
  the lexical token disagree. This holds against an **arbitrary** committer.
- **dual-leaf removes it.** The dual leaf witnesses the value handle (`value_component`) and
  the lexical hash (`lexical_component`) as **independent** witnesses with nothing tying them
  (`research/zk-field-native-encoding.md` §3.1, §2.1). A malicious trusted issuer can sign
  one credential whose value-FILTER lane answers `18` while its
  `sameTerm` / `DISTINCT` / `join` lane answers `5`. This is **impossible today** and is the
  documented, #769-accepted regression. No *untrusted* party can exploit it (it requires a
  signing key in `K`).

The code records this honestly, not as prose only: `CommitmentMethod::removes_inv_vl()`
returns `false` for string-canonical and `true` for dual-leaf (and value-only); the doc
comment on `DualLeafV1` states *"REMOVES INV-VL — value↔lexical agreement rests on
trusted-issuer honesty … accepted at research grade per #769 … open external-audit
obligation (CR-G8 / sq-qhy4)."* The honest mitigant on sparq's **own** ingest is the
fail-closed same-leaf co-binding (`value_component` and `lexical_component` are computed from
the *same* canonical bytes at ingest, so honest sparq ingest cannot self-desync); it does
**not** bind an external malicious issuer that commits off-sparq.

## 4. Per-method security posture

> Predicate-form caveat on every cell: these are properties **to be confirmed under external
> sign-off (sq-qhy4)**, not guarantees. The estate is remediated but NOT externally audited.
> Each cell states what is enforced *within the unaudited estate*, what is downgraded to a
> trust assumption, and what is unavailable.

### 4.1 (i) `string-canonical` (`StringCanonicalV1`, `zk:poseidon2-rdfc10-v1`)

The conservative default and back-compat anchor — **the only method implemented end-to-end
today**. Its leaf is `h2(TYPE_CODE_LITERAL, blake3(canonical N-Triples token))`.

- **value↔lexical agreement (INV-VL): machine-enforced in-circuit** against an arbitrary
  committer. The value and the operand binding come from one witnessed digit array
  (B1 range-decomposition + B4 canonical-form constraints in `filter_int.nr` /
  `filter_signed.nr`), so a prover cannot desync them. *(Obligation form: the external
  auditor must confirm the B1/B4 constraints are present and that the value feeding the
  comparison is the same one feeding the binding — CR-G8 item (2). Within the estate, this is
  what the current members enforce.)*
- **term identity (`sameTerm` / `DISTINCT` / `join`): available and sound**, because identity
  operators read the full blake3-token leaf (one preimage per term).
- **in-range / single-encoding (B1/B4): machine-enforced** via the digit array.
- **desync residual: none** — there is a single preimage, so value and lexical cannot
  diverge.
- **what it does NOT give you:** cheap value FILTERs. The in-circuit `blake3` over the
  canonical token dominates the member cost (the motivation for the value lane, sq-j506); and
  the digit-count member selection leaks `ceil(log10(value))` (the value lane closes that —
  §6).
- **recommendation:** the safe default; the back-compat anchor for every already-issued
  credential and golden vector. `is_production_selectable()` returns `true`.

### 4.2 (iii) `dual-leaf` (`DualLeafV1`, `zk:poseidon2-dualleaf-v1`)

The value-optimised method for *new* issuance (#769). Its leaf carries **both** a per-datatype
value handle (`value_component`) and a lexical-identity hash (`lexical_component`) so it can do
cheap value FILTERs **and** retain term identity. **The leaf encoding and circuit are NOT
implemented** (sq-j506 / sq-xojl); the registry can record an ALREADY-ISSUED selection (`RegistryEntry::new_already_issued`) today.

- **value↔lexical agreement (INV-VL): NOT machine-enforced — REMOVED.** This is the headline
  downgrade. The value-FILTER lane is **trusted-issuer-honesty** for value↔lexical agreement.
  - **Threat:** a malicious *trusted* issuer signs one credential answering a value FILTER
    as `18` and an identity question as `5`.
  - **Who is trusted:** the issuer whose key is in `K`. Honesty of that issuer is now a
    *precondition* for value↔lexical agreement, where string-canonical needed no such trust.
  - **Who CANNOT exploit it:** any untrusted prover or relying party — it requires a signing
    key in `K`. The downgrade does not widen the untrusted-adversary surface.
  - **Residual guarantees that survive:** revocation/suspension still bites (a dropped
    status leaf still fails the signed-commitment check); an unsigned or key-not-in-`K`
    commitment still fails; a graph committed under one method's leaf shape cannot be proven
    with a circuit expecting another shape (immutable-method property, §1 of the design).
  - **Honest mitigants (named, not a substitute for audit):** (a) sparq's own ingest computes
    both components from the same canonical bytes and **fails closed** on a parse mismatch, so
    honest sparq ingest cannot self-desync; (b) a named **canonical-issuance precondition**
    that, if conformed to, restores INV-VL. Neither binds an external malicious issuer
    committing off-sparq. This is **CR-G8 obligation (1)/(4)** for the auditor.
- **term identity: sound ONLY IF reject-list (v) is structurally enforced.** Identity
  operators must read the `lexical_component` **only**, never the many-to-one `value_component`
  (`-0.0`/`+0.0` collapse, NaN payloads, decimal `"5.0"`/`"5.00"` at fixed scale — design
  §3.3, `tests.nr:379-380`). If an identity operator ever read `value_component`, distinct
  terms would alias. The design makes this **structural** — the `(method, circuit)` resolver
  (sq-cfmv) refuses to bind an identity operator to the value lane — **not** prose. Until
  sq-cfmv lands and the auditor confirms it is fail-closed (CR-G8 obligation (3)), term
  identity on dual-leaf is **conditional, not established**.
- **in-range / single-encoding (B1/B4): sound ONLY IF instantiated in-circuit.** A
  value-bearing FILTER member is sound only if it **instantiates** (not merely names) the B1
  range-decomposition of the value handle (no modular wrap) and the B4 canonical-form bind. If
  the builder omits them, in-range-ness and single-encoding **also** downgrade from prover-side
  circuit guarantees to issuer/ingest assumptions — a strictly larger escalation than INV-VL
  alone (CR-G8 obligation (2)). The design's stance is to make B4's placement (in-circuit vs
  ingest) a **recorded per-method property** so this is stated truthfully, not left implicit.
- **desync residual: bounded to a malicious trusted issuer** (§3); sparq's own fail-closed
  ingest cannot self-desync; no untrusted party can exploit it.
- **recommendation:** the value-optimised method for new issuance that needs cheap value
  FILTERs **and** identity — **with the INV-VL downgrade accepted per #769 and the B1/B4 +
  reject-list (v) obligations carried to the audit.** `is_production_selectable()` returns
  `true`, which means **only** "not a research-only dial" — it is **not** an audit/soundness
  claim. Selecting it records the downgrade (CR-G8) explicitly.

### 4.3 (ii) `value-only` (`ValueOnlyV1`, `zk:poseidon2-valuehook-v1`) — research dial

A single value-first leaf with the **lexical hash dropped entirely**. Compiled out unless the
OFF-by-default `commitment-value-only` feature is on; never production-selectable
(`is_production_selectable()` returns `false`).

- **value↔lexical agreement (INV-VL): NOT APPLICABLE.** There is no lexical component, so
  INV-VL is meaningless and there is nothing for the value to agree *with*.
- **term identity: UNAVAILABLE — and identity ops MUST be rejected at plan time.** With no
  lexical component there is no per-term preimage; `sameTerm` / `DISTINCT` / `join` cannot be
  answered. The design requires they be **rejected at plan time**, never silently answered on
  a collapsing value handle (which would alias distinct terms). Identity desync is **total by
  construction**.
- **B1/B4:** the same in-circuit obligation as dual-leaf, with **no** lexical fallback for
  identity.
- **what it is FOR:** the cheapest and the **least safe** point — it earns its place purely so
  the benchmark + this table can show the **full cost/safety frontier** (the explicit purpose
  of "benchmarks with each configuration" in #769). It is comparison-only.
- **recommendation:** **never select for real issuance.** It is a research/benchmark dial,
  gated behind a feature whose name is the warning. The default-build rejection of its
  `zk:scheme` IRI is the structural enforcement of that recommendation.

### 4.4 One-line comparison

| Property | (i) string-canonical | (iii) dual-leaf | (ii) value-only |
|---|---|---|---|
| INV-VL (value = parse(committed lexical)) | machine-enforced in-circuit vs arbitrary committer | **REMOVED** → trusted-issuer honesty (#769 accepted; CR-G8) | not applicable (no lexical component) |
| term identity (`sameTerm`/`DISTINCT`/`join`) | sound (reads full token leaf) | sound **only if** reject-list (v) is structurally enforced | **unavailable** — reject at plan time |
| in-range / single-encoding (B1/B4) | machine-enforced (digit array) | sound **only if** B1/B4 instantiated in-circuit, else downgraded | same B1/B4 obligation, no lexical fallback |
| desync residual | none (single preimage) | bounded to a malicious trusted issuer | total by construction |
| implemented today | **yes** (end-to-end) | config only (registry); leaf/circuit pending | config only, **feature-gated off** |
| production-selectable | yes (default) | yes (downgrade accepted, NOT an audit claim) | **no** (research dial) |

## 5. The signature seam and VC-cryptosuite configurability (forward references)

These are the other two axes the maintainer asked to be configurable (#769). Both are
**partially seamed already** but **not yet generalised**; the security posture is stated for
the seam as it stands.

### 5.1 Signature-scheme seam (sq-1hsl, in flight)

- **Today:** `sig::SignatureScheme` is a **closed enum** with one variant,
  `Poseidon2SchnorrV1` (Schnorr over Baby-JubJub, Poseidon2 challenge). Baby-JubJub's base
  field **is** BN254's scalar field, so this signature is exactly the one an in-circuit
  verifier would check — that is why this scheme, not Ed25519, is the v1 choice. The
  `zk:cryptosuite` registry slot already records it.
- **In flight (sq-1hsl):** generalise the closed enum into an open
  `IssuerSignatureScheme` **trait** for the **off-circuit** seam (design §4.2; §10 Q4 default
  = open trait off-circuit). The trait surfaces the property that actually discriminates the
  schemes for this estate: `in_circuit_member()` — **whether a scheme is verifiable in-circuit
  at all.** Schnorr-over-Baby-JubJub is in-circuit-friendly by construction (the
  `hidden_issuer_d{depth}` member exists); a generic EdDSA/ECDSA over a non-embedded curve, or
  an RSA/lattice VC signature, is **not** without a dedicated, expensive new circuit member.
- **Security posture of the seam:**
  - `Poseidon2SchnorrV1` closes the unsigned-commitment hole **verifier-side**. v1 placement
    reveals **which** issuer signed each graph (the verifier checks `pk` in the clear); the
    in-circuit hidden-issuer upgrade removes that linkability leak but is itself unaudited
    (CR-G1) and is a privacy residual, not a soundness re-opening (CR-G6).
  - Signing is **NOT asserted constant-time** (the arkworks scalar-mul residual, CR-G5 /
    `sq-8jv7`); rated LOW **only because signing is issuance-side** (a trusted environment),
    and the relying party only ever runs `verify` over public data. This becomes load-bearing
    if signing moves to an exposed/online surface (e.g. the in-circuit hidden-key upgrade).
  - Any **added** scheme (EdDSA / ECDSA / BBS+) is verifier-side host-check only; **no**
    in-circuit member unless one is built and separately audited. The trait's
    `in_circuit_member()` returning `None` is the honest, machine-readable statement of that
    limit. None of this is a production guarantee.

### 5.2 W3C VC cryptosuite bridge (deferred — design-only)

- **Feasible scope (design §5.2):** an **ingest-time bridge** — verify a VC's Data-Integrity
  proof **off-circuit at the host** under its named cryptosuite (`eddsa-rdfc-2022`,
  `ecdsa-rdfc-2019`, and as a later seam `bbs-2023`), then RDFC10-canonicalise, re-commit
  under the selected `CommitmentMethod`, sparq-sign `C(G)`, and record a new
  `zk:sourceCryptosuite` provenance slot.
- **Two distinct signatures, kept distinct (security-critical):** the VC's own
  Data-Integrity proof (what makes it a valid VC) and sparq's commitment signature
  (`Poseidon2SchnorrV1`, what the query proof binds to) are **not the same**. sparq does
  **NOT** verify a VC's Ed25519/ECDSA proof **in-circuit**. Claiming in-circuit VC-proof
  verification would be an overclaim and is explicitly excluded.
- **Security posture:** `zk:sourceCryptosuite` is **provenance**, not a re-verifiable in-proof
  property; the query proof binds to sparq's commitment signature, not to the VC's proof. The
  selective-disclosure suites (`bbs-2023`, `ecdsa-sd-2023`) match sparq's per-leaf disclosure
  model more naturally; the plain suites are all-or-nothing at the VC layer. **No
  selective-disclosure soundness property is claimed for the bridge.** This axis is
  design-only and deferred; it is named here so the security frontier is complete, not because
  any of it is implemented.

## 6. Per-config benchmark plan (forward reference — sq-ot3x)

The cost half of "the comparison is the point" lives in sq-ot3x; this record carries only the
**security** half so the two can be read as one table per configuration.

- **Axis:** `(commitment-method × circuit-family × signature-scheme)`, plus the VC-bridge
  ingest sub-axis.
- **Ground-truth metric:** `bb gates -s ultra_honk` `circuit_size`, re-baselined into
  `crates/sparq-zk-compose/tests/gate_count_snapshot.json` + `bench/zk-compose/...` together
  (the parity test forces both), so a landed member's cost is a checked-in, regression-gated
  fact — **never a prose figure** in this or any markdown.
- **Honesty discipline:** gate counts are deterministic and **canonical once measured on the
  pinned toolchain** (the snapshot records the `bb`/`nargo` versions). The **VC-ingest
  wall-times** and any prover/verifier latency measured **on this EC2 work box are
  NON-CANONICAL** and must be labelled so. The benchmark MUST include the B1 range-decomposition
  and any B4 in-circuit re-derivation in the measured member, and MUST record the legacy
  blake3 family alongside the collapsed value-lane members, or the comparison is dishonest.
- **The deliverable is a table per configuration — cost AND the §4 security posture together**
  — so the maintainer sees the full cost/safety frontier, not a headline number.

## 7. Control CR-G9 — registered now (the configurable surface is a new fail-closed seam)

The design record (§8) proposed **CR-G9** and, per §10 Q6, deferred *registering* it until
the first implementation bead landed ("hold as proposal, register when the first impl bead
lands"). **The registry (sq-zzxt, #891) has landed**, so this record **registers CR-G9** on
the gap-register (`compliance/cryptoreview/gap-register.md`). The row:

> **CR-G9 — The commitment-method × circuit × signature compatibility matrix is a new
> fail-closed soundness surface.** Selectable commitment methods (string-canonical /
> dual-leaf / value-only) and a pluggable signature trait mean a verifier MUST refuse any
> `(zk:scheme, CircuitId)` or `(zk:scheme, zk:cryptosuite)` pair outside the legal matrix
> (design §3.1) — a value-bearing FILTER against a method with no value handle, an identity
> operator routed at a `value_component`, or an unknown method/cryptosuite IRI **defaulting
> instead of rejecting** are each a soundness break. **Partially seamed:** the
> `CommitmentMethod` registry is fail-closed on an unknown `zk:scheme` IRI today (#891 —
> `from_scheme_iri` returns `None`, `RegistryEntry::method()` returns `None`, never a
> default); the `(method, circuit)` dispatch (sq-cfmv) and the value-bearing members
> (sq-xojl) that the matrix governs are **not yet implemented**. **OPEN /
> EXTERNAL-REQUIRED**, folded into the sq-qhy4 pass. Design records:
> `research/zk-configurable-commitment-design.md` (the program) +
> `research/zk-configurable-commitment-security.md` (this write-up).

That registration is performed in the same change as this record (the gap-register edit is
the only `compliance/` change). It complements **CR-G8** (the dual-leaf INV-VL obligation),
which is already on `main` and unchanged: CR-G8 governs the *encoding's* removed invariant;
CR-G9 governs the *selection surface* that decides which encoding+circuit is legal.

## 8. What an external auditor MUST verify (carried, not re-decided)

The CR-G8 obligations (already on `main`, #794) apply unchanged to dual-leaf and are not
re-litigated here. CR-G9 adds, for the configurable surface: the auditor MUST verify the
`(method, circuit, signature)` compatibility matrix is **enforced fail-closed end-to-end** —
that **no proof verifies under a `CircuitId` illegal for its recorded `zk:scheme`**, that **no
identity operator can read a `value_component`**, and that an **unknown / mismatched** method
or cryptosuite IRI **rejects rather than defaults**. The registry half of this (unknown-IRI →
`None`) is in place and testable today (`commit.rs` tests
`method_parse_is_fail_closed_on_unknown`, `value_only_iri_is_rejected_without_the_feature`);
the dispatch half lands with sq-cfmv and is the part the auditor confirms is fail-closed.

## 9. Phased plan (future beads, ordered)

These are the security-relevant beads in the configurable-commitment build-out, ordered. All
are under epic **sq-1s2.5**, all carry the `[OPUS-4.8]` marker + the Opus 4.8
`Co-Authored-By` trailer, and all are **audit-gated behind sq-qhy4** for any soundness
reliance. Items 1–2 are landed/this-record; 3–8 are future.

1. **Commitment-method config registry** — `CommitmentMethod` over `zk:scheme`, fail-closed.
   **DONE** (sq-zzxt, #891).
2. **This security-properties write-up + register CR-G9** — `research/` doc + the gap-register
   row. **THIS RECORD** (sq-pkrl).
3. **Dual-leaf circuit member** — the value-FILTER over the value handle, with the B1/B4
   instantiation and the documented INV-VL downgrade. (sq-xojl; depends on the registry,
   which has landed; soundness reliance audit-gated.)
4. **Dual-leaf + value-only host leaf encoding** — the same-leaf co-binding at ingest,
   fail-closed; value-only flagged not-for-production. (sq-j506; aligns with item 3.)
5. **Fail-closed `(method, circuit)` dispatch matrix** — the CR-G9 surface: structurally
   refuse illegal pairs and identity-ops at the value lane. (sq-cfmv.)
6. **Pluggable signature-scheme seam** — the open `IssuerSignatureScheme` trait + a second
   verifier-side scheme so the cryptosuite comparison has a real second point. (sq-1hsl,
   in flight.)
7. **Per-config benchmark matrix** — sweep `(method × circuit × signature)`; emit the
   cost+security comparison table; honest canonical-vs-non-canonical labelling. (sq-ot3x.)
8. **W3C VC ingest cryptosuite bridge** — off-circuit verify + re-commit + record
   `zk:sourceCryptosuite`; `bbs-2023` as a later seam. (deferred; design-only today.)

## 10. Open questions that genuinely need the maintainer

These are not re-asks of the design record's §10; they are the security-specific decisions
this write-up surfaces:

1. **B4 placement, per method (the load-bearing one).** For dual-leaf and value-only, is the
   gate cost of an **in-circuit** canonicalising re-derivation of the value handle acceptable
   (keeping single-encoding a prover-side guarantee), or do you accept the **honest downgrade**
   to an issuer/ingest assumption? This must be `bb gates`-measured before deciding, but the
   *posture preference* is yours, and it directly changes the §4.2/§4.3 rows.
2. **Reject-list (v) enforcement point.** Confirm the structural enforcement of "no identity
   operator reads `value_component`" should live in the verifier's `(method, circuit)` resolver
   (sq-cfmv), as the design proposes, rather than at the planner — this decides where the
   term-identity-on-dual-leaf guarantee is *actually* made fail-closed.
3. **value-only retention.** Confirm keeping value-only purely as a feature-gated benchmark dial
   is still wanted, or whether it should be dropped from the program entirely (it adds a leaf
   shape + ingest path no real deployment should select).
4. **CR-G9 scope.** This record registers CR-G9 as a **partially-seamed OPEN** row now (the
   registry half is in place; dispatch/members pending). Confirm that framing, or whether you'd
   rather it stay a pure proposal until sq-cfmv lands.

## 11. Verdict

The configurable commitment surface is, as merged today, a **config-only registry** that makes
*how a graph was committed* an explicit, fail-closed, recorded property — with **no leaf-shape,
circuit, or encoding change yet**. Its security posture is honest and per-method: string-canonical
keeps the in-circuit INV-VL guarantee and is the safe default; **dual-leaf removes INV-VL on the
value-FILTER lane, downgrading value↔lexical agreement to trusted-issuer honesty — a documented
trust assumption the maintainer accepted at research grade in #769, not a proven property**; and
value-only is a feature-gated benchmark dial that loses term identity entirely and must never be
selected for real issuance. The whole estate is **remediated but NOT externally audited
(sq-qhy4, P0, OPEN)**; every "machine-enforced" property above is an **obligation an external
accredited cryptographer MUST confirm** before any production reliance, and **no production
security / privacy / soundness claim is made here.**

---

## Sources

In-repo (verified against `origin/main`, #891): `crates/sparq-zk/src/commit.rs`
(the `CommitmentMethod` enum + tests), `crates/sparq-zk/src/registry.rs`
(`method()`/`new_already_issued()` + the `zk:scheme` IRIs), `crates/sparq-zk/src/sig.rs`
(the `SignatureScheme` enum + the constant-time posture note),
`research/zk-configurable-commitment-design.md` (the program design + §8 CR-G9 proposal),
`research/zk-field-native-encoding.md` (#794, the dual-leaf FINAL design + INV-VL framing),
`research/zk-dual-leaf-issuer-desync-review.md` (the adversarial INV-VL finding),
`research/zk-soundness-audit.md` + `research/zk-verifier-reaudit.md` (the internal
audit/re-audit), `compliance/cryptoreview/gap-register.md` (CR-G1 / CR-G8 + the HEADLINE),
`SECURITY.md` (the "remediated but NOT externally audited" posture).

External prior art: W3C *Verifiable Credential Data Integrity 1.0*
(<https://www.w3.org/TR/vc-data-integrity/>), W3C *Data Integrity ECDSA Cryptosuites v1.0*
(<https://www.w3.org/TR/vc-di-ecdsa/>), W3C *Data Integrity EdDSA Cryptosuites v1.0*
(<https://www.w3.org/TR/vc-di-eddsa/>), W3C *Verifiable Credentials Data Model v2.0*
(<https://www.w3.org/TR/vc-data-model-2.0/>).
