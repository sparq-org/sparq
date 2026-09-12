<!-- [OPUS-4.8] Design-for-review authored by Opus 4.8 (1M context) (Fable unavailable) — re-review when Fable returns. -->
# Field-native ZK term encoding: the FINAL dual-leaf scheme (value handle AND lexical-identity hash)

Maintainer-review design record. This is the **finalized** encoding design for the
maintainer's `#769` decision, which selected the **dual-leaf** option (value +
lexical) over canonical-on-ingest alone. It **supersedes** the open-question
value-only `VALUE_HOOK` draft (PR #765) — that draft proposed a *single* value-first
leaf and left the dual-leaf question open (#765 §8 Q2); this record closes it with
the decided dual-component leaf and folds in the four adversarial review verdicts
(issuer-desync, term-identity, must-keep survival, gate-reality) that were run
against the draft. His verbatim direction (#769):

> "I guess we need both the value and the hash of the lexical representation then.
> This does create the risk that a malicious issuer could provide a value that does
> not conform to the lexical representation; but I don't know whether there are
> really any attacks that can be done based on that. Could you do the version where
> we include both so we can optimise using the value provided whilst also having the
> lexical representation for sameTerm type queries — and note the performance risk.
> It may be a good reason to force issuers to only be allowed to issue values in a
> canonical form in the long term."

This record builds directly on the gate-count attribution and the **§3.4 must-keep
constraint set** in [`research/zk-age-gatecount-reduction.md`](./zk-age-gatecount-reduction.md)
(read that first). Companion analyses:
[`research/zk-soundness-audit.md`](./zk-soundness-audit.md),
[`research/zk-verifier-reaudit.md`](./zk-verifier-reaudit.md),
[`research/zk-hidden-join-design.md`](./zk-hidden-join-design.md). The adversarial
issuer-desync review that drove §5's correction is in
[`research/zk-dual-leaf-issuer-desync-review.md`](./zk-dual-leaf-issuer-desync-review.md)
(PR #793).

Parent: epic **sq-1s2** ("ZK query-proof build-out + in-circuit privacy upgrades").
This is a **design-for-review**: it changes no `.nr` / `.rs` source, creates no bead
(the orchestrator owns bead structure; recommended children are in §11).

## 1. Honesty framing (load-bearing — read first)

sparq's v1 ZK query-proof verifier (`sparq-zk` / `sparq-zk-compose`) is
**remediated and internally re-audited but NOT externally audited**, and is
documented **NOT-yet-sound** for production reliance (beads **sq-qhy4**, **sq-9hrn**,
**sq-1s2**; `SECURITY.md`; `compliance/cryptoreview/gap-register.md` headline CR-G1).
An external accredited-cryptographer sign-off (**sq-qhy4**, P0) is **REQUIRED before
any ZK soundness / privacy / integrity property may be relied upon in production**.
The MPC estate is semi-honest-only and is not invoked here.

Nothing in this record is a security guarantee. Where this design says a constraint
is "preserved", that means the exact equalities, range-binds and canonical-form
asserts the current circuit makes are **relocated, not removed**, and their
preservation is itself an **audit obligation** — never an established fact. A
relocation is "safe to propose" only if **every** §3.4 must-keep demonstrably
survives with the cited constraint intact (not merely "equivalent"); even then the
framing stays "preserves the constraint set, pending sq-qhy4".

**The central honesty correction this record makes (driven by the adversarial
issuer-desync verdict, §5).** The value-only draft and an earlier dual-leaf draft
framed value↔lexical desync as "within the existing honest-issuer boundary — the
issuer could already lie about the value." **That framing is WRONG against the actual
code and is corrected here.** The current `filter_int` / `filter_signed` /
`filter_decimal` / `filter_float` members enforce, **in-circuit, against arbitrary
committers including a malicious trusted issuer**, the invariant that the compared
value IS the parse of the committed lexical bytes (call it **INV-VL**) — because the
value and the leaf binding are both derived from **one** witnessed digit/byte set
(verified, §2). The dual leaf witnesses the value handle and the lexical hash
**independently**, so it **REMOVES INV-VL**. That is a **trust-model regression for
the value-FILTER lane** (from machine-enforced to issuer-honesty-trusted), a NEW
capability — not the equivalent of a value lie that keeps value and lexical in
agreement. §5 states this plainly; it is not hand-waved away.

The privacy-claims CI gate (`scripts/check-privacy-claims.sh`) path-excludes
`research/**` (it defends the *outward* claim surface, not design records). The
caveat wording proposed for the ZK `SKILL.md`, the `sparq-zk` README, and the
`gap-register.md` CR-G8 row in §9 — which **is** on the scanned surface — is written
negated / obligation-framed (and inline-marked where the gate's predicate regex
over-matches) so it passes the live gate.

All gate counts here are the **measured `bb gates -s ultra_honk` `circuit_size`**
already snapshotted and regression-gated at
`crates/sparq-zk-compose/tests/gate_count_snapshot.json` — a circuit metric, not a
performance-marketing number. Toolchain: `nargo 1.0.0-beta.21`,
`bb 5.0.0-nightly.20260324` (the snapshot's baselined toolchain). Every *projected*
post-change figure is an **estimate bracketed by measured anchors** and is NOT a
claim until re-measured with `bb gates` on the actual changed member. All EC2 /
work-box timings are NON-canonical and none appear here.

## 2. What the leaf is today, what INV-VL is, and what identity ops consult (verified)

The host commitment path is `crates/sparq-zk/src/encode.rs`; the in-circuit mirror is
`zk/compose/compose_core/src/hashes.nr`. The current literal leaf is
(`encode.rs:52-55`):

```text
Enc_literal = h2(TYPE_CODE_LITERAL, blake3_field(literal.to_string()))
            = Poseidon2([2, field_from_hash_bytes(blake3("<lexical>"^^<dt> | @lang))], 2)
```

`oxrdf`'s `Literal::to_string()` passes the lexical form through **verbatim** — it
does not re-canonicalise — so the hashed token carries the **exact ingested bytes**
(`filter_signed.nr:9-12` confirms). `field_from_hash_bytes` (`field.rs`) truncates the
32-byte digest to its low 31 bytes (248 bits, bias-free). `TYPE_CODE_{IRI,LITERAL,
BLANK_NODE} = {1,2,3}` exist in both `encode.rs:33-35` and `hashes.nr:35-37`.

### 2.1 INV-VL: the value↔lexical invariant the current circuit enforces in-circuit

**This is the load-bearing fact the adversarial issuer-desync verdict surfaced, and
it corrects the earlier framing.** In every value-FILTER member today, the compared
numeric value AND the operand binding are derived from **the SAME witnessed digit
array** — there is exactly **one preimage**:

- `filter_int.nr:67-70` builds `value` by accumulating `digits[i]` into a `u64`;
  `filter_int.nr:72-92` rebuilds the canonical N-Triples token **from the same
  `digits`**, blake3-hashes it, and asserts `h2(LITERAL, hs) == operand_enc`. So a
  prover who commits lexical `"5"` literally **cannot** make the FILTER compare it as
  `18`: the circuit re-parses the digits `"5"` for both the value and the binding.
- `filter_signed.nr:150-180` (signed integer) and `filter_signed.nr:229-275`
  (decimal) do the same: `mag` / `mag_scaled` and the rebuilt token both consume the
  same `mag_digits` / `int_digits` / `frac_digits`.
- `filter_float.nr` derives the IEEE bits from witnessed digits the same way, then
  binds the rebuilt token.

Call this invariant **INV-VL**: *the compared value equals the parse of the committed
lexical bytes, enforced in-circuit against an arbitrary (even malicious, even trusted)
committer.* No value↔lexical desync state is reachable today. This is a **prover-side
circuit guarantee independent of issuer honesty** — it is the property the dual leaf
must be honest about (§5).

### 2.2 What identity ops consult (verified) — every one compares the FULL leaf

*Every identity-sensitive in-circuit operation already compares the full leaf,
because the leaf is the only per-term value the circuit holds.*

- **scan row-presence / attribution** (`scan.nr:143-149`, `:164-175`): equality is
  `enc[g][i][s] == rows[j][s]` and `enc[g][i][s] == pattern.const_enc[s]` — over the
  full per-slot term encoding `Enc_t`. The disclosed `rows` (`scan.nr:88`) are PUBLIC
  inputs (so the lexical side can be consumed by any downstream / non-ZK consumer —
  this matters in §5.2).
- **hidden join** (`join.nr:170-177`): `a_val = select_slot(row_a, slot_a)` over a row
  of `enc_a`, then `assert(a_val == b_val)`; the value is bound under a blinder into
  `join_commitment` (`join.nr:179+`). The equality is over the **full leaf** at the
  join slot, and is **cross-credential** (graph A vs graph B). `DISTINCT` / `sameTerm`
  over a ZK column, when added, reduce to the same full-leaf equality.
- **value FILTER** (`filter_int.nr:92`, `filter_signed.nr:114`, `filter_float.nr`):
  the *only* place that looks *inside* the leaf — it re-derives the value from the
  lexical bytes and re-hashes (the in-circuit blake3 the value handle removes).

So the design principle: **put the lexical-identity hash where the full-leaf equality
already lands (identity ops correct by construction, no change), and put the value
handle inside the same leaf as a cheap additional component (so the FILTER member can
bind the value with Poseidon2 instead of blake3).** But — per §2.1 — the value handle
and the lexical hash being *independent witnesses* is exactly what removes INV-VL, so
§5 and §4 must put back, or honestly account for, what the single-preimage construction
gave for free.

## 3. The dual-leaf encoding (mapped to `Poseidon2` / the actual code)

### 3.1 The leaf

```text
Enc_literal = Poseidon2([ value_component, lexical_component, TYPE_CODE_LITERAL ], 3)

  value_component   = Poseidon2([ VALUE_HOOK, DATATYPE_CONST, LANG_CONST ], 3)
                      // the cheap numeric handle (§3.3); a per-datatype field value
                      // + PRECOMPUTED datatype/lang constants for the known FILTER
                      // datatypes — this is what lets the FILTER avoid in-circuit blake3.

  lexical_component = field_from_hash(blake3(canonical N-Triples token))
                      // the identity-bearing part; computed OFF-circuit at ingest,
                      // EXACTLY today's blake3_field(literal.to_string()).
```

`Poseidon2::hash([·,·,·], 3)` is the existing `h3` arity (`hashes.nr:25` — already used
for triple leaves and `sig.rs` 3-input commitments), so no new permutation width is
introduced. The full leaf is one outer Poseidon2 over three fields; the inner
`value_component` is one more Poseidon2 over three fields.

This is a structural generalisation of today's leaf: `lexical_component` **is** today's
`blake3_field(to_string())` (byte-for-byte), so the identity content is unchanged; the
leaf gains a `value_component` sibling and moves `TYPE_CODE_LITERAL` to the
value-first / type-last slot (matching the maintainer's `hashFields([…, termType])`
convention).

### 3.2 Non-literals and string / opaque literals — value_component degenerates

For terms with **no numeric handle**, `value_component` collapses to a fixed
no-value sentinel and the **lexical_component is the sole binding** — in-circuit cost
UNCHANGED from today:

| Term | Leaf | Cost vs today |
| --- | --- | --- |
| IRI | `Poseidon2([NO_VALUE, lexical, TYPE_CODE_IRI], 3)`, `lexical = blake3(iri)` | one extra Poseidon2 layer at commit only; in-circuit unchanged |
| Blank node | `Poseidon2([NO_VALUE, lexical, TYPE_CODE_BLANK_NODE], 3)`, `lexical = Poseidon2([salt_G, blake3(label)], 2)` | salt-scoped inner retained (Q6); one extra layer at commit only |
| `xsd:string`, `rdf:langString`, opaque datatype | `Poseidon2([NO_VALUE, lexical, TYPE_CODE_LITERAL], 3)`, `lexical = blake3("<lexical>"^^<dt> or @lang)` | **string lane, no numeric handle, in-circuit cost UNCHANGED** |

`NO_VALUE` recommendation: a datatype-folded `value_component = Poseidon2([VALUE_NONE,
DATATYPE_CONST, LANG_CONST], 3)` with `VALUE_NONE` a reserved field tag distinct from a
real `VALUE_HOOK = 0`, so a degenerate value_component can never be confused with a real
zero value and `literal_shapes_are_distinguished` (`encode.rs:104-117`) stays true
without leaning on the lexical lane alone. (§10 Q1 confirms this against a single global
`NO_VALUE` alternative.)

### 3.3 VALUE_HOOK per datatype

The handle is **injective on value within a datatype**, range-bound, and is what a
FILTER compares against in-circuit without re-hashing a string. **Note (term-identity
verdict, §5.5): for double/float and decimal the handle is MANY-TO-ONE on the term** —
flagged below and is why reject-list (v) plus the expanded regression guard (§8) are
load-bearing.

| Datatype | VALUE_HOOK | Many-to-one on term? | Canonical-form obligation (B4) |
| --- | --- | --- | --- |
| `xsd:boolean` | `0` / `1` | yes (`"true"`/`"1"` → same) | injective on the two values |
| `xsd:integer` (incl. signed) | signed value in the `u64` magnitude + sign domain `filter_signed.nr` uses (NOT raw wrapping `Field`) | yes (`"05"`/`"5"` → same) | no leading zeros, no `-0` (`filter_int.nr:58-64`, `filter_signed.nr:142-158`) |
| `xsd:decimal` | canonical scaled integer at the member-fixed `FD` scale (matching `filter_decimal_check`) | **yes — `"5.0"`/`"5.00"` collide at a fixed `FD`** | `filter_decimal` digit-canonicality asserts (`filter_signed.nr:215-246`) |
| `xsd:double` / `xsd:float` | IEEE-754 bit pattern as a field | **yes — `-0.0`/`+0.0` compare EQUAL (`tests.nr:379-380`); NaN payloads** | canonical IEEE bits — §14.2 resolves §10 Q3: the fold is applied IN-CIRCUIT before the value component is formed, not ingest-only, and it does obsolete sq-mslu's RNE parser for the *comparison* |
| `xsd:dateTime` / `xsd:date` | signed scaled-epoch scalar at a member-fixed `FS` sub-second scale, timezoned-`Z` canonical lexicals ONLY (§13 — resolves §10 Q2) | not in the slice-1 `Z`-only domain; becomes yes if the §13.6 offset-normalisation widening lands | §13.4 fail-closed canonical predicate (bare / non-`Z` / `24:00:00` / over-`FS`-precision lexicals rejected); NOT in the first slice |
| `xsd:string`, `rdf:langString`, opaque | `VALUE_NONE` (no handle) | — | the §3.2 fallback; lexical lane only |

`DATATYPE_CONST` / `LANG_CONST` are `blake3(datatype IRI)` / `blake3(lang)` off-circuit
at ingest, and **precomputed field constants** in-circuit for the known FILTER
datatypes — the substitution that removes the in-circuit blake3 (§4).

### 3.4 Why the lexical_component cannot be dropped

The value-only draft (#765) committed a single value-first leaf and had to defend
value-collapse (`"05"` vs `"5"` → same leaf) for every operator. The maintainer rejected
canonicalise-on-ingest alone (#769) and chose to carry the lexical hash too, because:

- The engine receives **pre-committed** graphs from external issuers it does not
  canonicalise at ingest. A value-only leaf would silently make
  `join`/`DISTINCT`/`sameTerm` use **value identity** for those — wrong per RDF
  semantics.
- The lexical_component preserves **term identity** for every identity op with **zero
  mechanism change** to scan/join (those ops already compare the full leaf, §2.2, which
  now contains the lexical hash), **conditionally** on reject-list (v) being enforced so
  no identity op ever reads `value_component` (the many-to-one hazard of §3.3 / §5.5).

### 3.5 Leaf-tuple order and the one-time recommit (inherited migration)

Adopting `Poseidon2([value_component, lexical_component, TYPE_CODE], 3)` is value-first
(matches the maintainer's spec) and **re-bases every leaf**: the host
(`encode.rs`/`commit.rs`), the circuit (`hashes.nr` leaf recompute), every checked-in
leaf/commitment vector, every real-bb cross-vector, the `gate_count_snapshot.json`
baselines, and any persisted `<urn:sparq:zk>` commitment must be **recomputed and
re-committed atomically** in the same change, or the verifier byte-compare
(`verifier.rs` `PublicInputMismatch`) diverges silently. Same one-time migration #765
flagged; unchanged here (§10 Q4).

## 4. The gate win — proving a value-FILTER against `value_component`, AND putting back B1/B4

This is the (a) part of the brief. **The gate-reality adversarial verdict CONFIRMED
the win is real** (no in-circuit blake3 hidden in the dual-leaf reconstruction; the
only production in-circuit `std::hash::blake3` call sites are the three FILTER members,
`filter_int.nr:84` / `filter_signed.nr:107` / `filter_float.nr:137`, and scan/join have
**zero**). **But the must-keep adversarial verdict found the §4 pseudocode of an earlier
draft SILENTLY DROPPED the B1/B4 mechanism** — it witnessed VALUE_HOOK as a bare `Field`
and only asserted the leaf binding, with **no range-decomposition and no
canonical-digit asserts**. That is fixed here: the FILTER member MUST *instantiate*, not
merely name, B1 and B4.

```text
// witnesses: VALUE_HOOK (a Field), lexical_component (a Field).
// ---- B1: RANGE-DECOMPOSE the witnessed VALUE_HOOK into the typed comparison domain
//      BEFORE any comparison. A Field witness is NOT a u64; without this a prover can
//      supply VALUE_HOOK congruent to a small value mod the BN254 modulus (modular wrap,
//      reject-list (i)).
//   integer/signed : prove VALUE_HOOK's magnitude < 2^64 (explicit bit/byte decomposition),
//                    sign as a constrained bool; compare in the u64 + sign domain.
//   decimal        : prove scaled magnitude = int_part*10^FD + frac_part with ID+FD<=19
//                    fits u64; compare in the scaled-int + sign domain.
//   double/float   : prove the 64/32-bit IEEE pattern is well-formed; compare via
//                    sparq_ieee754 f64/f32 predicates (filter_f64 shape).
// ---- B4: CANONICAL-FORM bind on the range-decomposed value. Because the digit-string
//      asserts (no leading zero, no -0, canonical scale) cannot exist in a digit-free
//      member, the member MUST EITHER (i) re-introduce a constrained canonicalising
//      re-derivation of VALUE_HOOK from a canonical witness in-circuit (costs gates,
//      must be re-measured), OR (ii) the design must HONESTLY RE-CLASSIFY B4 (and the
//      no-modular-wrap half of B1 not covered by the range-decomposition) to an
//      issuer/ingest-side assumption — a STRICTLY LARGER trust escalation than §5's
//      value↔lexical point, because in-range-ness and single-encoding are TODAY
//      prover-side circuit guarantees independent of issuer honesty. §5.4 records which.
inner = Poseidon2([VALUE_HOOK, DATATYPE_CONST, LANG_CONST], 3)            // 1 perm
leaf  = Poseidon2([inner, lexical_component_witness, TYPE_CODE_LITERAL], 3) // 1 perm
assert_eq(leaf, operand_enc)                                             // the binding
// then: the typed comparison (B2) and assert verdict == expected (B3) — filter_f64 shape.
```

`lexical_component` is supplied to the FILTER member as a **witness** (the
scan-anchored leaf's other component; the FILTER does not need the lexical bytes, only
the field value of its hash, which is enough to reconstruct the full leaf and assert the
binding). The FILTER member **never reconstructs or hashes the lexical string** — that
is the saving — **but it does add the explicit VALUE_HOOK range-decomposition (B1) that
the earlier draft omitted**, whose cost must be measured (§10 Q5; the
noir-optimisation standing warning, PR #37 / `shr_sticky`, applies — inversion/
decomposition surprises intuition on this stack).

Cost accounting against the measured anchors (from `gate_count_snapshot.json`,
reproduced in `zk-age-gatecount-reduction.md` §6):

| Anchor | `circuit_size` | What it isolates |
| --- | ---: | --- |
| `filter_f64` (raw compare, no binding) | 3,113 | comparison verdict + UltraHonk/range-table floor |
| blake3-over-48B probe | 17,416 | the in-circuit string-hash binding **alone** |
| `filter_int_d{1..4}` / `filter_signed_int_d{2,4}` / `filter_decimal_i3_f2` / `filter_f64_d{1..4}` | 17,416 | the full blake3-bound FILTER members |

The blake3 binding (~14,300 gates = 17,416 − 3,113) is replaced by two Poseidon2 perms
(~74 gates each per the `hashes.nr:14` cost note ≈ ~150 gates) **plus** the VALUE_HOOK
range-decomposition and the typed re-derivation.

> **Projected dual-leaf numeric-FILTER member ≈ 3,200 gates** (3,113 comparison floor +
> ~150 Poseidon2 binding + the range-decomposition cost, which must NOT be assumed
> negligible). **This is an ESTIMATE bracketed by the two measured anchors above. It is
> NOT a claim.** It MUST be confirmed with `bb gates -s ultra_honk` on the actual
> dual-leaf FILTER member — *including* the B1 range-decomposition and any B4
> canonicalising re-derivation — and re-baselined into
> `crates/sparq-zk-compose/tests/gate_count_snapshot.json` (and
> `bench/zk-compose/gate_counts_latest.json`) before it is quoted anywhere. The net
> delta of moving the outer binding from arity-2 `h2` to arity-3 `h3` plus one inner
> perm (net +1 Poseidon2 perm over today's single binding perm) plus the range-check is
> a MEASUREMENT obligation, not an assumption.

## 5. The value↔lexical consistency analysis — corrected per the issuer-desync verdict

This is the crux (the maintainer's "malicious issuer could provide a value that does not
conform to the lexical representation" worry). The earlier dual-leaf framing was found
**FALSE against the code** by the issuer-desync verdict; the corrected, honest position
is below.

### 5.1 The structural fact (CONFIRMED by the verdict)

The dual leaf carries two independent components in the same leaf: `value_component`
(from VALUE_HOOK) and `lexical_component` (from blake3(lexical)). They are **independent
preimages**: `Poseidon2([value_component, lexical_component, TYPE_CODE])` binds *that the
leaf contains both*, but does **NOT** bind that `VALUE_HOOK == parse(lexical)`. To bind
them in-circuit, the circuit would have to re-derive the value from the lexical bytes —
witness the lexical string, parse it, assert it equals VALUE_HOOK — which is **exactly
the blake3-over-the-token + digit-parse the value handle exists to avoid** (~14,300
gates). The verdict CONFIRMED this structural claim is correct.

> **Value↔lexical consistency CANNOT be enforced in-circuit without giving back the gate
> win. The loss of INV-VL is therefore the IRREDUCIBLE PRICE of the gate win, not a
> negligible residual.**

### 5.2 The CORRECTED threat model — the dual leaf REMOVES the in-circuit invariant INV-VL

The earlier framing said desync is "not a new value-lie capability (the issuer could
already lie about the value)." **That is wrong, and the issuer-desync verdict refuted it
against the code:**

- **Today (§2.1), INV-VL holds in-circuit against ARBITRARY committers** — including a
  malicious trusted issuer. A malicious issuer who commits lexical `"5"` **cannot** make
  any FILTER compare it as `18`; the circuit re-parses `"5"` for both the value and the
  binding. A value lie today **keeps value and lexical in agreement** (the issuer must
  commit `"18"` lexically to get an `18` comparison, and then `sameTerm`/`join`/`DISTINCT`
  also see `18`).
- **Under the dual leaf, INV-VL is REMOVED.** A malicious trusted issuer can commit a
  leaf with `VALUE_HOOK = 18` but `lexical = "5"`, sign `C(G)`, and a holder proves
  `age ≥ 18` truthfully against `value_component` while the **same** credential answers a
  `sameTerm`/`DISTINCT`/`join` question as `"5"` via `lexical_component`. **That single
  signed credential answering a value question as 18 and an identity question as 5 is
  IMPOSSIBLE today.** It is a NEW capability, a strict trust-model regression for the
  value-FILTER lane: that lane goes from **machine-enforced** to **issuer-honesty-
  trusted**.
- **The exploit surface is wider than "one mixed query."** Disclosed scan rows are
  PUBLIC (`scan.nr:88`; `rows` are public inputs), so the lexical side can be consumed by
  any downstream / non-ZK consumer of the disclosed result, not just an in-proof
  identity op. And `join_eq` (`join.nr:170-177`, full-leaf equality) makes the
  inconsistency **cross-credential**: one malicious issuer's desynced leaf plus one
  honest issuer's `"5"` leaf can be joined on the lexical side while the malicious leaf
  passes a value-FILTER as `18`.

### 5.3 What still holds, honestly

- **No UNTRUSTED party gains anything.** The scan/issuer chain still binds the leaf to a
  trusted Schnorr signature over `C(G)` (`verifier.rs` `bind_issuer_attestations`;
  `issuer.nr`). An untrusted party cannot forge a desynced leaf; the attacker must **be
  or collude with a trusted issuer**.
- **No single-component operation weakens.** A pure value-FILTER over an honest issuer's
  leaf, or a pure identity op, is unaffected. The desync only bites where a value answer
  and an identity answer about the **same** committed term are both consumed.
- **An honest issuer never desyncs**, and §6's host-side same-leaf co-binding makes
  honest sparq ingest *structurally unable* to desync, turning desync into a detectable
  protocol violation for sparq-originated commitments.

### 5.4 Two trust escalations, named separately (do not conflate)

The dual leaf escalates trust in **two** distinct ways, both of which the docs must
state honestly:

1. **Value↔lexical agreement** (§5.2) — removed INV-VL; rests on issuer honesty for the
   value lane.
2. **In-range-ness and single-encoding** (the must-keep verdict, §4) — if the §4 member
   does **not** instantiate the B1 range-decomposition and a B4 canonicalising
   re-derivation in-circuit, then in-range-ness (no modular wrap) and single-encoding (no
   second encoding of a value) **also** migrate from prover-side circuit guarantees to
   issuer/ingest assumptions. This is a **strictly larger** escalation than (1). **The
   design's position: instantiate B1 in-circuit unconditionally (it is cheap relative to
   the comparison floor and a hard reject-list (i) requirement), and instantiate B4
   in-circuit if measurement allows; only if B4's in-circuit re-derivation proves too
   costly may it be downgraded — and then the docs/SKILL/README MUST say in-range single-
   encoding rests on issuer honesty too.** This choice is itself a sq-qhy4 audit
   obligation (§9).

### 5.5 Term-identity is CONDITIONAL, not a property of the encoding (term-identity verdict)

The term-identity verdict found that "preserves term identity end-to-end" rests on an
**unenforced convention** (reject-list (v): "value_component MUST NOT be consulted by any
identity op"), not a circuit invariant — and that VALUE_HOOK is **many-to-one on the
term** for two named datatypes (§3.3): IEEE `-0.0`/`+0.0` compare EQUAL
(`tests.nr:379-380`), NaN has many comparison-unordered patterns, and decimal `"5.0"`/
`"5.00"` collide at a fixed `FD`. So `value_component` is affirmatively a term-identity
**HAZARD**, not a neutral sibling. Term identity is preserved **only because every
current identity op reads the full leaf** (which today carries the lexical hash) — **not
as a property of the encoding**. The moment a DISTINCT/sameTerm/value-keyed member is
added (the design says they are coming), nothing *structural* stops it consulting
`value_component`. Fix: §8 promotes reject-list (v) to a structurally-enforced invariant
and §8/§11-bead-4 expands the regression guard to the many-to-one datatypes.

### 5.6 The long-term close — canonical issuance is a NAMED PRECONDITION, not just a future option

The maintainer's own suggestion — *force issuers to issue only canonical-form values* —
is, per the issuer-desync verdict, **elevated from a documented future option to a NAMED
PRECONDITION for the value lane.** If an issuer *actually* emits canonical lexical forms
AND `VALUE_HOOK = parse(canonical_lexical)`, then for that issuer class INV-VL holds as
an issuance invariant, value identity coincides with term identity for the datatypes
whose value handle is injective over canonical terms (NOT the many-to-one ones, §5.5),
and the dual leaf could collapse per-datatype toward a value-first leaf (§7). The
conformance mechanism itself is **designed at proposal grade** in
`research/zk-canonical-form-issuance.md` (sq-mtv7) — and that design establishes it is
**issuer self-attestation**: it scopes and attributes the honest-issuer assumption
(which keys claim the discipline, with attribution/revocation on breach) but supplies
**no verifier-checkable evidence of same-byte derivation**, so it does NOT make the
value lane sound against a malicious or compromised issuer. **The value-FILTER lane
over an adversarial issuer is sound only under the explicit honest-issuer-for-value
assumption named in §5.2 — both before and after the conformance mechanism exists.**

## 6. The host-side same-leaf co-binding at ingest (issuer-desync fix)

So that **honest sparq ingest never desyncs** and desync becomes a **detectable protocol
violation** for sparq-originated commitments, `encode.rs` / `commit.rs` MUST compute
`VALUE_HOOK = parse(canonical(lexical))` from the **same bytes** that are hashed into
`lexical_component`, and **fail closed** if the lexical form does not parse to a
canonical value of its datatype:

- For a hookable datatype, ingest parses the lexical bytes once, derives both the
  canonical `VALUE_HOOK` and the `lexical_component` blake3 from the same parse, and
  commits the dual leaf only if the parse succeeds and the lexical form is canonical
  (or it canonicalises and records that it did). A non-parseable / non-canonical lexical
  for a hookable datatype is a fail-closed ingest error, not a silent desync.
- This does **not** prevent a *malicious external issuer* from committing a desynced leaf
  off-sparq and signing it (only canonical-issuance conformance, §5.6/§7, closes that) —
  but it guarantees sparq's own commitments are INV-VL-consistent and makes any desynced
  leaf a deviation from sparq's published ingest behaviour.

This is off-circuit Rust; it is not a gate cost, but it is a real ingest cost and must be
measured, not assumed negligible (§10 Q5).

## 7. Performance honesty — dual-leaf is MORE expensive than value-only

Per the maintainer's "note the performance risk":

- **Commit time (host).** The dual leaf is two Poseidon2 perms per literal (inner
  value_component + outer leaf) **plus** the blake3 over the lexical token, **plus** the
  §6 ingest parse + fail-closed canonical check — versus today's one blake3 + one `h2`.
  For IRIs/strings it adds one extra Poseidon2 layer over today. Off-circuit, but real;
  must be measured.
- **In-circuit (gates).** The reduction applies to **value-comparison FILTER members
  ONLY**: 17,416 → est. ~3,200, to be re-measured **including the B1 range-
  decomposition**. It does **NOT** apply to scan / join / revoke / issuer / holder
  (one `h3` per leaf, unchanged), to identity ops (full-leaf equality — the dual leaf
  buys them **correctness**, conditional on §5.5, not speed), or to IRI/string/opaque
  FILTERs (string lane unchanged).
- **Net.** Same FILTER gate win as value-only, at MORE commit-time cost, while carrying
  the lexical hash forever, **and** at the price of the removed INV-VL on the value lane
  (§5.2). The maintainer accepted the commit-time trade explicitly (#769); §5.6's
  canonical issuance is the path to retire both the carried lexical hash and the INV-VL
  regression.

## 8. §3.4 must-keeps carried onto the DUAL leaf, with the verdict fixes folded in

Each `zk-age-gatecount-reduction.md` §3.4 must-keep, restated for the dual leaf.
File:line citations verified against the current checkout. None is asserted as proven.

**A. Operand bound to the signed/committed credential (not attacker-chosen).**

- **A1 / A4 — operand binding by RELOCATION.** Today `filter_int.nr:92` /
  `filter_signed.nr:114` assert `h2(LITERAL, blake3(token)) == operand_enc`. Under the
  dual leaf: `assert_eq(Poseidon2([Poseidon2([VALUE_HOOK, DT_CONST, LANG_CONST], 3),
  lexical_component_witness, TYPE_CODE_LITERAL], 3), operand_enc)`. The equality assert
  is kept. **CORRECTION (must-keep verdict):** A1 binds VALUE_HOOK as a **field element**,
  not as a range-bound `u64`; A1's survival is real **only if the SAME range-decomposed
  VALUE_HOOK feeds both the binding and the typed comparison** (§4). Without the §4 B1
  decomposition, A1 binds "the same field" but not "the same in-range value the
  comparison uses." **AUDIT OBLIGATION.**
- **A2 — scan anchors the new DUAL leaf.** `verifier.rs` (`scanned != operand` ⇒
  `BindingInconsistent`); `scan.nr:97-108` commit-recompute now carries the dual leaf.
  Mechanism unchanged; what flows through it changes. Must not weaken.
- **A3–A7 — scan binds witnessed graph to public commitment.** `scan.nr:97-108`
  `commit_fold == commitments[g]`; `scan.nr:119-124` strictly-increasing commitments
  (anti duplicate-inclusion / COUNT-forgery); row-present `scan.nr:139-149`;
  completeness + attribution `scan.nr:158-182`. Survives unchanged except the hashed leaf
  is the dual leaf.
- **A8 / A9 — issuer attestation / in-circuit Schnorr.** `verifier.rs`
  `bind_issuer_attestations`; `issuer.nr`. **Untouched** — and (per §5) exactly what makes
  the removed-INV-VL a matter of issuer trust, since the malicious-desync attacker must be
  a trusted issuer.

**B. Comparison `value ⋈ bound` correct over the field — NO modular wrap.**

- **B1 — range-checked integer domain, no signed-Field wrap.** Today the magnitude
  accumulates into `u64` from range-checked ASCII digits (`filter_int.nr:58-70`,
  `filter_signed.nr:142-153`, `filter_signed.nr:229-242`). **CORRECTION (must-keep
  verdict):** the §4 member has no digit arrays, so it MUST add an **explicit in-circuit
  range-decomposition** of the witnessed VALUE_HOOK proving it lies in the typed domain
  (magnitude `< 2^64`; scaled magnitude `ID+FD<=19`; well-formed IEEE pattern) **before**
  the comparison. NOT raw `Field` arithmetic — REJECTED (reject-list (i)). This is
  instantiated in §4, not merely named.
- **B2 — verdict correct and constant-shape.** `filter_int.nr:96-110`, `signed_verdict`
  (`filter_signed.nr:57-87`), `f64_verdict` (`filter_float.nr:29-43`). Unconditional over
  typed `u64`/`f64`; untouched.
- **B3 — verdict asserted equal to public `expected`.** `filter_int.nr:111` /
  `filter_signed.nr:183` / `filter_signed.nr:284` / `filter_float.nr:152`. Unchanged.
- **B4 — canonical-form bind carried onto VALUE_HOOK.** Today on digit arrays
  (`filter_int.nr:58-64`, `filter_signed.nr:142-158`, `filter_signed.nr:215-246`).
  **CORRECTION (must-keep verdict):** with no digit arrays in the member, the no-leading-
  zero / no-`-0` / canonical-scale asserts have nothing to attach to. The member MUST
  EITHER re-introduce a constrained canonicalising re-derivation of VALUE_HOOK from a
  canonical witness in-circuit (costs gates — re-measure), OR the docs MUST honestly
  re-classify B4 to an issuer/ingest assumption (the §5.4 (2) larger escalation). **AUDIT
  OBLIGATION;** §5.4 records which path is taken. *Distinct from §5.2:* B4 stops a
  **prover** binding a second VALUE_HOOK encoding of the same value; §5.2 is about a
  **malicious issuer** setting VALUE_HOOK ≠ parse(lexical).

**C. Public inputs bind correctly — no malleability.**

- **C1 — verifier-side reconstruction byte-matches the proof.** A dual-leaf FILTER member
  changes the `pub`/witness layout (witnessed VALUE_HOOK + lexical_component witness + the
  per-datatype constants), so `reconstruct_public_inputs` **and** the six real-bb
  cross-vectors (`reconstruct_filter_int_matches_real_bb_public_inputs` and the
  f64/signed/decimal siblings, `verifier.rs` ~5010/5041/5111/5146/5184/5222) **must be
  updated in lockstep** (verified against the checkout). MANDATORY CO-CHANGE.
- **C2 — canonical verifier-side vk.** `verifier.rs` `canonical_vk`; `derive_id` must pin
  the new member's parameters. CO-CHANGE.
- **C3 / C4 — nonce + query-correctness binding.** `verifier.rs` field-0 challenge;
  `bind_query_correctness`. Unchanged.

**D. Nullifier / uniqueness / replay.** `record_fresh`; holder PoP / `holder_pok`.
Untouched.

### Reject-list (carried from §3.4 — still REJECTED)

(i) lower the typed comparison to raw `Field` arithmetic, or **omit the VALUE_HOOK
range-decomposition** (modular wrap — B1); (ii) drop the canonical-form binds on
VALUE_HOOK without re-classifying them honestly (second encoding — A1/B4); (iii) take
VALUE_HOOK as a free witness not anchored to the scan-bound commitment (A1/A2); (iv) feed
Baby-JubJub coords to the Grumpkin native MSM black-box (wrong curve — A9). **Specific to
dual-leaf:** (v) consult `value_component` in **any** identity-sensitive operator
(scan-row equality / `join_eq` / DISTINCT / sameTerm) — that re-introduces value-collapse
on the identity lane, and (per §5.5) `value_component` is many-to-one on the term for
double/float/decimal, so this is an affirmative hazard. Identity ops MUST consult the
full leaf (identity carried by `lexical_component`). **(v) must be STRUCTURALLY enforced,
not left as prose** (§8 fix below). REJECTED.

### Structural enforcement of reject-list (v) (term-identity verdict fix)

(a) Any identity-sensitive in-circuit member (scan-row eq, `join_eq`, future
DISTINCT/sameTerm) MUST take the **full leaf `Enc`** as its equality operand and MUST be
**structurally prevented** from receiving `value_component`/`VALUE_HOOK` as an input —
e.g. type-segregate `value_component` so it is only addressable inside FILTER members and
is never a selectable row slot. (b) The §11-bead-4 regression guard MUST be **expanded
beyond the integer `"05"`/`"5"` fixture** to assert non-collision for EACH many-to-one
VALUE_HOOK datatype: IEEE `-0.0` vs `+0.0` (distinct lexical, value-equal), NaN payloads,
and decimal `"5.0"` vs `"5.00"` at a fixed `FD` — proving they do NOT
join/dedup/sameTerm. (c) §9/§12 state term-identity preservation is **conditional** on
(a) and only verified by (b), NOT a property of the dual-leaf encoding itself.

### What binds each component (collision argument)

A prover cannot find a `value_component' ≠ value_component` or `lexical_component' ≠
lexical_component` with the same leaf (Poseidon2 collision-resistance + the
`assert_eq(leaf, operand_enc)` against the scan-anchored leaf force both witnessed
components to be the committed ones). Cross-datatype value collisions (integer `5` vs
decimal `5` vs the double bits for `5.0`) are prevented by `DATATYPE_CONST` inside the
inner value_component. Term identity (`"05"` vs `"5"`) is preserved by the distinct
`lexical_component` **and** by reject-list (v) being structurally enforced. **Standard
collision-resistance arguments under the Poseidon2 / blake3 assumptions — design intent
the external audit must verify, NOT a proven property.**

## 9. Audit-obligation registration (exact wording)

Extends CR-G8 to the FINAL dual leaf, folding in the issuer-desync (removed INV-VL),
term-identity (many-to-one + unenforced reject-list (v)), and must-keep (B1/B4
instantiation) verdicts. Obligation-/negation-framed to pass
`scripts/check-privacy-claims.sh`. The full revised CR-G8 row text is in
`compliance/cryptoreview/gap-register.md` (this PR edits it). SKILL/README wording:

### 9.1 ZK `SKILL.md` (`skills/zk-query-proofs/SKILL.md`) — caveat

> A dual-leaf value+lexical literal encoding (research-grade, design in
> `research/zk-field-native-encoding.md`) is proposed to cut numeric-FILTER gate cost
> while carrying a separate lexical-identity hash for sameTerm/DISTINCT/join; it is NOT
> implemented and NOT audited. It REMOVES the in-circuit invariant that the compared
> value equals the parse of the committed lexical bytes (today enforced against arbitrary
> committers), so the value-FILTER lane becomes sound only under an unverified
> honest-issuer-for-value assumption; term-identity preservation is conditional on an
> enforced rule that no identity operator reads the value handle (the handle is
> many-to-one on the term for double/float/decimal); and in-range single-encoding rests
> on an in-circuit range-decomposition that must be instantiated, not assumed. All of
> this is registered as an open external-audit obligation (CR-G8 / sq-qhy4); it provides
> no soundness or privacy guarantee. <!-- privacy-claims-allow: negative/obligation framing — names the unaudited dual-leaf encoding only to flag removed invariants as open audit obligations; sq-qhy4 -->

### 9.2 `crates/sparq-zk/README.md` — caveat

> A proposed dual-leaf value+lexical term/literal encoding
> (`research/zk-field-native-encoding.md`, research-grade, NOT implemented) would re-base
> the commitment onto value-first leaves carrying both a per-datatype numeric hook and a
> lexical-identity hash; it is unaudited, it removes the in-circuit value-equals-parsed-
> lexical invariant the current FILTER members enforce (value-lane consistency then rests
> on issuer honesty), and it makes no soundness or privacy claim — it is an open external-
> audit obligation (sq-qhy4). <!-- privacy-claims-allow: negative/obligation framing — flags an unimplemented unaudited encoding as an open audit obligation, asserts no guarantee; sq-qhy4 -->

## 10. Open questions (need the maintainer)

1. **`value_component` degeneracy shape (§3.2):** global `NO_VALUE` vs datatype-folded
   `Poseidon2([VALUE_NONE, DT_CONST, LANG_CONST])`? Recommendation: the latter.
   **RESOLVED — §14.1** (proceed-and-document, sq-vvfte): datatype-folded, per this
   record's own recommendation, with `VALUE_NONE` constrained to fall outside the
   reachable handle band; maintainer can steer post-hoc via the steer-me issue opened
   by the sq-vvfte PR.
2. **dateTime/date VALUE_HOOK:** canonical epoch vs component tuple; timezone-
   normalisation rule. Deferred from the first slice. **RESOLVED — §13**
   (proceed-and-document, spike sq-fvztj): signed scaled-epoch scalar, timezoned-`Z`
   canonical lexicals only, everything else fail-closed; maintainer can steer
   post-hoc via the sq-fvztj steer-me issue.
3. **Double bits vs sq-mslu parser:** confirm against `filter_float.nr` /
   `sparq_ieee754` that the IEEE-bit VALUE_HOOK obsoletes the in-circuit RNE parser for
   the *comparison*, AND decide the `-0.0`/`+0.0`/NaN canonicalisation rule at ingest
   (§3.3 many-to-one), before closing/superseding sq-mslu. **RESOLVED — §14.2**: it
   does, for the comparison — `filter_value_dl_f64` commits the IEEE bits, so no
   in-circuit parse occurs at all; the canonicalisation is the in-circuit
   `canonical_f64_bits` fold (any NaN → the canonical quiet NaN, `-0.0` → `+0.0`);
   sq-mslu is **re-scoped** to the string-canonical residual for the single-leaf blake3
   lane and demoted **P4**, NOT closed.
4. **One-time recommit (§3.5):** confirm the value-first re-base of all persisted
   commitments/vectors/snapshot is acceptable now (research grade).
5. **B4 in-circuit vs ingest (§5.4):** is the cost of an in-circuit canonicalising
   re-derivation of VALUE_HOOK acceptable (keeping single-encoding a prover-side
   guarantee), or is the honest downgrade to an issuer/ingest assumption accepted? The
   §4/§6 ingest parse + the §4 B1 range-decomposition cost must be `bb gates`-measured
   first.
6. **Canonical-issuance conformance (§5.6):** is the canonical-issuance conformance
   mechanism — now a NAMED precondition that scopes the value lane's issuer-honesty
   assumption (issuer self-attestation; it does not bind a malicious issuer, §5.6) —
   wanted on the roadmap now (a §11 bead), or accepted as a stated precondition that
   bounds when the value lane may be relied upon?

## 11. Beads (orchestrator to create — ordered)

Recommended children of epic **sq-1s2**, gated on **sq-qhy4** for any production reliance
(research-grade / opt-in / NOT-yet-sound before sign-off). Existing related beads to
link, not duplicate: **sq-j506** (numeric lane in `encode`/`commit`), **sq-mslu**
(`xsd:double` RNE parser — likely refined). The orchestrator owns bead creation.

1. **Audit-obligation registration (CR-G8 revised + SKILL/README caveats, §9).** Doc-only;
   land FIRST so sq-qhy4 is forced to check the removed INV-VL, the B1/B4 instantiation,
   the structural reject-list (v) enforcement, the many-to-one handle hazard, and the
   value↔lexical issuer-honesty assumption. No code. *(This PR lands the CR-G8 + research
   doc; the SKILL/README edits are this bead's follow-on so they land WITH the impl.)*
2. **Host encoding overhaul + same-leaf co-binding at ingest (`encode.rs` + `commit.rs`).**
   Implement the dual leaf `Poseidon2([value_component, lexical_component, TYPE_CODE], 3)`
   with per-datatype VALUE_HOOK + the degenerate `value_component`; keep
   `lexical_component` = today's `blake3_field(to_string())`; **compute
   `VALUE_HOOK = parse(canonical(lexical))` from the SAME bytes, fail-closed** (§6).
   One-time atomic recommit of all leaf/commitment vectors. Extend/relink sq-j506. Tested.
   Audit-gated.
3. **Circuit + verifier co-change WITH B1/B4 instantiation.** Add dual-leaf FILTER
   member(s) (`filter_int`/`signed`/`decimal`/`float`) binding via the 2-Poseidon2
   constant-datatype path with `lexical_component` as a witness (NO in-circuit blake3),
   **including the explicit VALUE_HOOK range-decomposition (B1) and a B4 canonical bind or
   the honest §5.4 downgrade**; structurally type-segregate `value_component` so no
   identity op can read it (reject-list (v)); update `scan.nr`/`join.nr` leaf recompute to
   the dual leaf; update `reconstruct_public_inputs` + `derive_id`/`canonical_vk` + the six
   real-bb cross-vectors in lockstep (C1/C2); **re-measure with `bb gates` (including the
   range-decomposition)** and re-baseline `gate_count_snapshot.json` +
   `gate_counts_latest.json`. Depends on 2. Audit-gated.
4. **Identity-op + desync regression guard (EXPANDED).** Prove `value_component` is NEVER
   consulted by scan-row equality / `join_eq` / DISTINCT / sameTerm (reject-list (v)),
   with fixtures for EACH many-to-one datatype: integer `"05"`/`"5"`, IEEE `-0.0`/`+0.0`,
   NaN payloads, decimal `"5.0"`/`"5.00"` at fixed `FD` — asserting they do NOT
   join/dedup. Plus a desync-detection test: the §6 fail-closed ingest rejects a
   non-canonical hookable lexical. Depends on 3.
5. **Canonical-issuance conformance (NAMED PRECONDITION, §5.6).** Design + implement the
   conformance mechanism under which, for issuers that *actually follow* the discipline,
   INV-VL holds as an issuance invariant, and which lets the leaf collapse toward a
   value-first leaf per-datatype (injective-handle allow-list only — the many-to-one
   datatypes keep their identity handle). Second one-time recommit. Audit-gated. This is
   the named precondition that scopes/attributes the value lane's issuer-honesty
   assumption (issuer self-attestation, NOT an adversarial-issuer mechanism —
   `zk-canonical-form-issuance.md` §4), not merely a roadmap nicety. **The DESIGN half is now scoped in
   `research/zk-canonical-form-issuance.md` (sq-mtv7 / #3286); the implement half —
   the `secx:conformsTo` vocabulary, the attestation binding, and the fail-closed
   relying-party gate — is the audit-gated follow-on, dependent on sq-j506.**

Suggested ordering / deps: **1** (register first) → **2** → **3** (depends on 2) → **4**
(depends on 3) → **5** (depends on 4 + this draft's resolution). 2/3 are audit-gated
behind sq-qhy4 for production reliance; they may be implemented at research grade before
sign-off, consistent with the rest of the ZK estate.

## 12. Verdict

**The dual-leaf encoding is the correct realisation of the maintainer's #769 decision,
with two honesty corrections folded in from the adversarial review.** It gives the same
anchor-bracketed FILTER gate reduction as the value-only draft (17,416 → estimated
~3,200, to be re-measured **including the B1 range-decomposition**) by replacing one
in-circuit blake3 with two constant-datatype Poseidon2 permutations over the witnessed
VALUE_HOOK, while keeping the lexical-identity hash in the same leaf so identity ops stay
correct **conditionally** (see below). The cost, stated honestly, is that the lexical
hash is still carried (more commit-time cost than value-only) and that the value lane
loses an invariant it has today.

**Correction 1 (issuer-desync — the maintainer's malicious-issuer question, answered
straight):** binding value↔lexical in-circuit would re-derive the value from the lexical
bytes — exactly the blake3/parse the value handle exists to avoid — so consistency CANNOT
be enforced in-circuit without defeating the gate win. **This means the dual leaf REMOVES
the in-circuit invariant INV-VL (value = parse(committed lexical)) that the current
`filter_int`/`signed`/`decimal`/`float` members enforce against arbitrary committers
including a malicious trusted issuer.** A malicious *trusted* issuer can therefore commit
one signed credential that answers a value question as `18` and a sameTerm/DISTINCT/join
question as `5` — **impossible today**, a NEW capability, a strict trust-model regression
for the value-FILTER lane (not "the issuer could already lie about the value"; that lie
keeps value and lexical in agreement). No untrusted party can exploit it (the
scan/issuer chain still binds to a trusted signature), and a host-side same-leaf
co-binding at ingest (§6, fail-closed) makes honest sparq ingest unable to desync.
Canonical-issuance conformance (§5.6) is the NAMED PRECONDITION under which INV-VL holds
as an issuance invariant for issuers that *actually follow* the discipline (issuer
self-attestation — it does not bind a malicious issuer) and is the exit path that
retires the dual leaf per-datatype.

**Correction 2 (must-keep + term-identity):** the value-FILTER member MUST *instantiate*,
not merely name, an explicit in-circuit range-decomposition of VALUE_HOOK (B1) and a
canonical-form bind (B4), or in-range-ness and single-encoding silently downgrade to
issuer/ingest assumptions — a larger escalation than the value↔lexical point, which the
docs must state. And term-identity preservation is **conditional** on structurally
enforcing reject-list (v) (no identity op reads `value_component`, which is many-to-one on
the term for double/float/decimal) and is verified only by the expanded regression guard,
NOT a property of the encoding itself.

All of this — the removed INV-VL and its honest-issuer-for-value assumption, the B1/B4
in-circuit instantiation (or its honest downgrade), the structural reject-list (v)
enforcement, the many-to-one handle hazard, and the canonical-issuance precondition — is
**registered as an explicit external-audit obligation (CR-G8 / sq-qhy4)**; none is
presented as a settled property, and no soundness or privacy guarantee is claimed pending
the external sign-off.

## 13. §10 Q2 RESOLVED: the `xsd:dateTime` / `xsd:date` VALUE_HOOK (spike sq-fvztj, proceed-and-document)

Design-only resolution of the one §10 question left open by the first slice. Made under
the standing proceed-and-document rule (best-judgment call, maintainer steers post-hoc
via the sq-fvztj steer-me issue); it inherits every honesty caveat of this record — the
removed INV-VL, the issuer-honesty assumption on the value lane, and the CR-G8 / sq-qhy4
external-audit obligation. **Nothing below is a soundness claim**; the chosen rule is
itself registered as an OPEN audit obligation (§13.7).

### 13.1 Decision — a signed scaled-epoch SCALAR, not a component tuple

`VALUE_HOOK` for `xsd:dateTime` (and `xsd:date`, §13.3) is a **single signed scalar**:

```text
VALUE_HOOK = sign · ( |seconds_from_epoch| · 10^FS + fraction_scaled )
```

- the XSD proleptic-Gregorian `timeOnTimeline` mapping (NO leap seconds — XSD's
  timeline has none; `second = 60` is not a valid XSD lexical and is rejected),
  anchored at epoch `1970-01-01T00:00:00Z`;
- `FS` is a **member-fixed sub-second scale** (recommendation: `FS = 3`,
  milliseconds), folded into the lane's `DATATYPE_CONST` exactly like the decimal
  `FD` bind — `blake3("<xsd:dateTime IRI>@epochscale=<FS>")` — so a hook at one
  scale can never collide a hook at another (B4);
- carried **sign-split** `(value_neg, value_mag)` in the same `u64`-magnitude +
  sign domain `filter_signed.nr` / `filter_value_dl_decimal` already compare
  (`signed_scaled_verdict` is reused UNCHANGED; no `-0`; B1
  `assert_max_bit_size::<64>()` on the magnitude).

**Why the scalar beats the component tuple.** XSD ordering on *timezoned* values IS
timeline order, so one sign-aware scalar comparison decides every FILTER operator —
the exact machinery `filter_value_dl_decimal` ships today. A `(year, month, day, hour,
minute, second, tz)` tuple would need a multi-limb lexicographic comparator (a new
comparator to build AND audit), and still needs every component canonicalised before
the hook is injective — the tuple buys nothing within the slice-1 domain. The tuple's
one genuine advantage — representing *un-timezoned* values distinctly — is mooted
because slice 1 rejects them outright (§13.2); if bare lexicals are ever admitted it
is via a disjoint sub-lane constant (§13.6), not a tuple.

**Why not epoch-seconds unscaled:** `xsd:dateTime` admits arbitrary fractional-second
precision; dropping sub-seconds would collide distinct values (non-injective). The
member-fixed `FS` mirrors the decimal lane's fixed-point pattern
(`canonical_decimal_scaled`), with one deliberate difference: `FS` is fixed for the
WHOLE lane (canonical lexicals with 1..=`FS` fraction digits are scaled UP to `FS`,
exactly, never rounded), instead of decimal's per-lexical `fd`-in-the-const. Fixing
`FS` keeps every committed `xsd:dateTime` hook in ONE totally-ordered domain, so a
FILTER can compare operands of differing lexical precision — for decimal the
per-`fd` split was acceptable; for dateTime cross-precision comparison is the normal
case (`"…T12:00:00Z" < "…T12:00:00.5Z"` must be decidable in one member). A lexical
with MORE than `FS` fraction digits is rejected fail-closed (never rounded — rounding
would break injectivity AND desync the §6 co-binding); a higher-`FS` member is a
compatible future addition because `FS` is folded into the const.

**Injectivity (the invariant).** Within the slice-1 domain (timezoned-`Z` canonical
lexicals, §13.2/§13.4): equal XSD values have exactly one canonical lexical, and the
canonical lexical determines the scaled epoch exactly — so the hook is
**injective-on-value within the datatype**, and (a strictly stronger property, unique
to this lane's slice 1) injective on the TERM too: the `Z`-only canonical domain
admits one lexical per value, so no new row is added to the §3.3 many-to-one hazard
table until the §13.6 offset widening lands.

### 13.2 The timezone-normalisation rule — hookable domain = timezoned `Z` ONLY, everything else fail-closed

1. **Un-timezoned lexicals are NOT hookable — fail-closed reject** (`DualLeafError`,
   never a silent string-lane downgrade and never an implicit timezone):
   - XSD order between an un-timezoned and a timezoned value is PARTIAL
     (indeterminate inside the ±14:00 window). Both live under the same
     `xsd:dateTime` `DATATYPE_CONST`; if bare lexicals were mapped into the same
     scalar domain, the circuit would compare indeterminate pairs determinately —
     wrong against XSD semantics, and inconsistent with the engine's RELATIONAL
     comparison, which keeps that partial order as a type error. (sq-2k5py made
     only the ORDER BY / MIN-MAX **total order** decide those pairs — by instant,
     then timezone presence, a documented extension — and deliberately left the
     relational semantics alone.) The engine and the ZK lane must not disagree.
   - An implicit-timezone rule (SPARQL/XPath evaluation context) is
     context-dependent — the same lexical would hook differently per context, which
     is not injective-on-value and makes the §6 co-binding ill-defined.
2. **Non-`Z` offsets (`+01:00`, `-05:00`, and the `+00:00`/`-00:00` spellings) are
   rejected in slice 1** — the strict mirror of the boolean lane's canonical-only
   rule (`{"true","false"}` accepted, XSD-legal `"1"`/`"0"` rejected). XSD 1.1's
   canonical dateTime form is `Z`-normalised, so slice 1 = "canonical lexicals
   only". The compatible widening is §13.6, a follow-up decision, NOT slice 1.
3. **`24:00:00` (XSD-legal next-day-midnight endpoint) is rejected** — it is
   excluded from the canonical form (canonical spelling is `00:00:00` of the next
   day); accepting it would admit two lexicals for one value.

Fail-closed here means the §6 co-binding stays well-defined: `encode_datetime`
either derives BOTH the hook and the lexical hash from one successful strict
canonical parse of the same bytes, or commits nothing. A rejected lexical is an
ingest error for the dual-leaf lane, not a desynced leaf. (As everywhere in this
record, this binds only sparq's OWN ingest — a malicious external issuer is
unconstrained pending §5.6 canonical-issuance; issuer honesty on the value lane
remains an unverified assumption under CR-G8.)

### 13.3 `xsd:date`

Same rule, own lane: the hook is the scaled epoch of the date's **starting instant**
(midnight UTC — XSD orders dates by their starting moment on the timeline), at the
same `FS`, under its OWN constant `blake3("<xsd:date IRI>@epochscale=<FS>")` so a
date can never collide a dateTime (§3.3 cross-datatype separation). Slice-1 domain =
`YYYY-MM-DDZ` canonical lexicals only; bare dates — however common in real data —
are rejected fail-closed in slice 1 for exactly the §13.2(1) indeterminacy reason.
(`xsd:date` has no fractional part; sharing `FS` keeps the two lanes' host/circuit
code identical.)

### 13.4 The fail-closed canonical predicate (the §6 co-binding, exact)

`encode_datetime` / `encode_date` accept a lexical iff ALL of:

- strict XSD 1.1 grammar shape with an explicit `Z` timezone (no offset, no bare);
- structural component validity: month `01..=12`, day valid for month + proleptic
  Gregorian leap-year rule, hour `<= 23`, minute/second `<= 59` (no leap second —
  not an XSD lexical), no `24:00:00`;
- canonical year: no superfluous leading zeros beyond four digits, `-` sign
  permitted (proleptic), no `+`;
- fraction (dateTime only): `1..=FS` digits, no trailing zero (canonical minimal
  form), absent entirely rather than `.0`;
- range: the scaled-epoch magnitude fits `u64`, else reject (never wrap — the
  far-proleptic-year overflow is fail-closed, mirroring the integer lane).

The epoch conversion is pure integer arithmetic (days-from-civil), no floating
point, derived once from the same bytes that `lexical_component =
blake3_field(literal.to_string())` hashes verbatim.

### 13.5 The circuit member — `filter_value_dl_datetime` is the decimal member's shape

One new relation with the exact `filter_value_dl_decimal` structure: public
`(operand_enc, op, bound_neg, bound_scaled_epoch, expected, datatype_const)`,
private `(value_neg, value_hook_scaled, lexical_component)`; B1 range-decompose,
no `-0`, sign folded into the handle by field negation, two `h3` bindings, verdict
via the UNCHANGED `signed_scaled_verdict`, no in-circuit blake3. Because
`datatype_const` is a PUBLIC input, the SAME relation serves both the dateTime and
the date lane — the host selects the lane constant; no second Noir function. Gate
shape is therefore expected to match the decimal member's (to be `bb gates`-measured
when implemented, per the §7 rule; no number is claimed here).

### 13.6 Documented widenings (recorded, NOT slice 1)

- **Offset normalisation (canonicalise-and-record):** accept non-`Z` offsets by
  folding the offset into the epoch (UTC normalisation). Compatible: it only widens
  the accepted domain and maps the new lexicals onto the SAME hook values — no
  recommit. Cost: the hook becomes many-to-one on the term
  (`…T12:00:00+01:00` = `…T11:00:00Z`), joining the decimal/double row in the §3.3
  hazard table under reject-list (v) — which is exactly why it is deferred until the
  reject-list guard (§11 bead 4) exists to witness it.
- **Bare (un-timezoned) lexicals as a DISJOINT sub-lane:** bare values among
  themselves ARE totally ordered, so a separate constant
  (`…@epochscale=<FS>@tz=none`) gives sound within-sub-lane comparison while making
  a bare-vs-timezoned comparison structurally inexpressible (no member binds both
  constants) — the circuit-side analogue of the engine's sq-2k5py residual partial
  order. A design choice for later, kept out of slice 1.

### 13.7 Follow-on beads + audit registration

Mirrors the boolean-lane pair, inheriting its INV-VL / CR-G8 caveats verbatim:

1. **Host** `encode_datetime` + `encode_date` in `crates/sparq-zk/src/dual_leaf.rs`'s
   module family (new module, opt-in `dual-leaf` feature, default build unchanged) —
   the §13.4 predicate, fail-closed, lexical_component byte-identical to today's
   string-canonical `h_s`.
2. **Circuit** `filter_value_dl_datetime` member (+ member crate + the compose/verifier
   wiring the int/decimal/f64 members received), reusing `signed_scaled_verdict`;
   depends on 1.

The §13 rule set — `Z`-only domain, member-fixed `FS`, epoch mapping, the rejection
list, and both §13.6 widenings — is registered as an OPEN external-audit obligation
under CR-G8 / sq-qhy4 alongside the rest of this record. No soundness or privacy
property is claimed for the dateTime/date lane pending the external sign-off.

## 14. §10 Q1 + Q3 RESOLVED: the degenerate `value_component`, and the double lane vs the sq-mslu parser (proceed-and-document)

Design-only resolution of the two remaining §10 questions a spike could settle without
the maintainer. Made under the standing proceed-and-document rule (best-judgment call,
steerable post-hoc via the steer-me issue opened by the sq-vvfte PR); both inherit every
honesty caveat of this record — the removed INV-VL, the issuer-honesty assumption on the
value lane, and the CR-G8 / sq-qhy4 external-audit obligation. **Nothing below is a
soundness claim**; both rules are themselves registered as OPEN audit obligations
(§14.3).

### 14.1 Q1 — the degenerate `value_component` is DATATYPE-FOLDED, not a global `NO_VALUE`

**Decision: `value_component = h3(VALUE_NONE, DATATYPE_CONST, LANG_CONST)`** — the shape
§3.2 already recommended, now confirmed against the single-global alternative. The
implementation is carried by **sq-vvfte**.

**The deciding reason is that it is the SAME shape the value lanes already ship.** Every
dual-leaf member that exists today forms its value component identically —
`h3(VALUE_HOOK, DATATYPE_CONST, LANG_NONE)`, in the int / f64 / decimal / dateTime
members of `filter_value.nr` — so the degenerate case is that one permutation with a
reserved tag in the value slot, not a second encoding shape. A global `NO_VALUE` would
add a branch to leaf reconstruction that the host AND every circuit that recomputes a
leaf (`scan.nr` / `join.nr`, §11 bead 3) must agree on, for no structural gain. One
shape is one audit surface.

Two secondary points, at their real weight:

- **It keeps the datatype/lang separation in the value slot too.** Under a global
  `NO_VALUE` every non-hookable literal shares ONE value component, so
  `literal_shapes_are_distinguished` (`encode.rs`) rests on the lexical lane ALONE. The
  folded form is *redundant with* the lexical hash rather than load-bearing over it —
  defence in depth, NOT an independent security property, and it must not be written up
  as one.
- **It is the more expensive option, honestly.** A global `NO_VALUE` is a compile-time
  constant: no permutation at commit for strings / IRIs / blank nodes. The folded form
  costs one Poseidon2 per degenerate literal at commit. In-circuit cost is unchanged
  either way, because a degenerate value component is never an in-circuit operand:
  identity ops are structurally barred from the value slot (reject-list (v)) and a
  FILTER member has nothing to bind for a term with no handle. The choice is therefore
  shape uniformity bought with commit-time work, and §7's "dual-leaf is MORE expensive
  than value-only" framing stands unrevised.

**For non-literals the two alternatives coincide.** IRIs and blank nodes have no datatype
and no language, so with reserved constants in the unused slots the folded component is
a fixed value the host and circuit can precompute — the global `NO_VALUE` in all but
name. The choice therefore only bites for LITERALS; the §3.2 table's IRI and blank-node
rows are unaffected either way.

**The reserved-tag constraint — pick `VALUE_NONE` outside the reachable handle band.**
§3.2 motivates the tag as "distinct from a real `VALUE_HOOK = 0`". Sharpened: under the
datatype fold a degenerate component and a real one *already* differ in the
`DATATYPE_CONST` slot, and fail-closed ingest (§6, §13.2 — a non-hookable lexical on a
hookable datatype is REJECTED, never silently downgraded to the string lane) means no
term is ever degenerate under a *hookable* datatype const. So the tag's value is not
load-bearing today; it becomes load-bearing the moment any lane degrades instead of
rejecting. Since the tag is free to choose, choose it so the separation does not depend
on that rule holding everywhere, forever:

`LANG_NONE = 1` is separated from a real `LANG_CONST` only **probabilistically**, and this
record must not say otherwise. A real `LANG_CONST` is `blake3_field(lang)` — a hash
*reduced into the circuit field*, which CAN equal `1`. Picking a small reserved tag makes
that collision negligible, not impossible; the language slot is collision-resistant and
domain-separated, never provably disjoint. The enforceable sharpening is the same shape
§14.1 demands of the value slot — a fail-closed `assert(lang_const != LANG_NONE)` at
ingest, mirrored wherever a witnessed `LANG_CONST` enters a circuit — and until that
ships, "no committed language tag hashes to `LANG_NONE`" is a **retained assumption**,
registered as an OPEN audit obligation in §14.3. (The shipped host and Noir constants
today assert nothing and their doc comments overclaim; captured as follow-up work, not
fixed by this record.)

The value slot is worse than probabilistic, which is why it gets an assert and not a
caveat: its real occupants ARE small field elements, so a `0`/`1` tag collides with
certainty rather than negligibly. Every shipped member B1-range-decomposes its handle
(`assert_max_bit_size::<64>()`), and the signed lanes fold the sign by field negation
(`0 - magnitude`, in the decimal and dateTime members), so the reachable handle band is

```text
[0, 2^64)  ∪  (p − 2^64, p)          (p = the field modulus)
```

and a `VALUE_NONE` of `0` or `1` sits inside it. So: **derive `VALUE_NONE` as a
domain-separated `blake3_field` constant and ASSERT it falls in neither band** — a host
test plus a Noir compile-time check. Asserting the exclusion is what makes this a
checkable invariant rather than a probabilistic argument about where a hash output lands.

### 14.2 Q3 — `filter_value_dl_f64` obsoletes the in-circuit RNE parser FOR THE COMPARISON; sq-mslu keeps the string-canonical residual

**Confirmed against the code Q3 named.** `filter_float.nr`'s own status note records that
it is a gate-counted building block that cannot be composed, because binding a hidden
operand would require re-deriving the canonical `xsd:double` lexical form from the bit
pattern in-circuit (float-to-canonical-decimal printing, unbudgeted); and that the general
fractional/scientific fragment (sq-lxi7) additionally needs a decimal→IEEE
round-to-nearest-even parser whose cross-base big-integer interval check was never
`bb gates`-measured — so it was honestly left unshipped rather than approximated with a
shortcut that loses RNE.

**The dual leaf removes that requirement structurally, for the comparison.** Under
dual-leaf commitments the value handle IS the IEEE-754 bit pattern, so
`filter_value_dl_f64` (`zk/compose/filter_value_dl_f64`, relation
`sparq_zk_compose_core::filter_value::filter_value_dl_f64`) binds the operand through the
two Poseidon2 permutations over the witnessed bits and carries `lexical_component` as a
free witness. No decimal→IEEE parse, no canonical-lexical printing and no RNE interval check
occurs in-circuit at all: the parser is not made cheaper, it is made **unnecessary for
this lane**. The parse moves to the host (`dual_leaf::encode_double`, fail-closed per §6 —
handle and lexical hash derived from the same bytes, or nothing is committed), which is
exactly the §5.4 trade this record already names: an in-circuit obligation exchanged for
an ingest one, not eliminated.

**The `-0.0`/`+0.0`/NaN rule Q3 asked for is decided, and it is stronger than §3.3
recorded.** The canonicalisation is `canonical_f64_bits`: any NaN → the canonical quiet
NaN, `-0.0` → `+0.0`, everything else unchanged. It is applied **in-circuit, before the
value component is formed** (`filter_value.nr`) and mirrored host-side by
`dual_leaf::canonical_f64_bits`. B4 for the double lane is therefore an enforced
in-circuit fold, not the ingest-only assumption §3.3's original "canonical IEEE bits at
ingest" wording implied. What this does NOT change: the handle stays MANY-TO-ONE on the
term — that is the *purpose* of the fold — so the §3.3 hazard row and reject-list (v)
remain fully load-bearing: for the signed zeros, whose two canonical spellings `"0.0E0"`
and `"-0.0E0"` are both accepted by `parse_xsd_double_bits`, only the `lexical_component`
keeps them apart for an identity op. NaN is NOT symmetric with that and the §8 guard
list should not be read as if it were: canonical `xsd:double` has a single `NaN` lexical,
so distinct payloads are unreachable through sparq's own ingest and the lexical lane
cannot disambiguate them either. The payload fold defends only against a committer who
witnesses a non-canonical payload directly — an issuer-side capability, which lands it
back under the same unverified issuer-honesty assumption as the rest of the value lane.

**sq-mslu is re-scoped, not closed** (verdict recorded in the sq-mslu notes; **demoted
P4**). What survives is the **string-canonical residual**: reconstructing `xsd:double`'s
canonical lexical form in-circuit — and with it the RNE interval check — for the
SINGLE-leaf blake3 lane, which binds a hidden operand through the lexical token and so
still cannot express a fractional/scientific double filter. That residual matters only to
deployments that do not adopt the dual leaf, which is why it drops to P4 rather than being
superseded outright. `filter_float.nr`'s measure-then-ship-or-documented-reject discipline
applies to it unchanged; no affordability claim is made for it here.

### 14.3 What stays open, and the audit registration

§10 Q4 (the one-time recommit), Q5 (B1/B4 in-circuit vs ingest) and Q6 (canonical-issuance
conformance on the roadmap) are maintainer calls and remain OPEN. Neither resolution above
settles them: §14.2 narrows Q5 for the double lane's B4 only (the IEEE-bit fold is
in-circuit) and leaves the question standing for the other lanes' range and canonical-form
obligations.

Both §14 rules — the reserved-tag band exclusion on `VALUE_NONE`, and the double lane's
in-circuit canonical fold together with the residual it leaves to sq-mslu — are registered
as OPEN external-audit obligations under CR-G8 / sq-qhy4 alongside the rest of this
record. So is the §14.1 **`LANG_NONE` separation assumption**: unlike `VALUE_NONE`'s
exclusion it is today *unasserted*, resting on collision-resistance of `blake3_field` over
the language-tag domain rather than on a check, and it stays an assumption until the
fail-closed ingest/circuit assert §14.1 names actually ships. No soundness or privacy
property is claimed for any of the three, pending the external sign-off.
