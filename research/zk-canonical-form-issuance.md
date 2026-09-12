<!-- [OPUS-4.8] Proposal authored by Opus 4.8 (1M context) for the Fable collaboration tier — re-review when Fable returns. -->
# Proposal: canonical-form issuance conformance — issuer self-attestation that scopes the value-lane honest-issuer assumption (NOT an adversarial-issuer mechanism)

> 🤖 SPARQL/ZK research agent record (sq-mtv7, #3286). A **design-for-review
> PROPOSAL / long-term item**, not an implementation. It changes no `.nr` / `.rs`
> circuit or encoder and asserts **no** soundness or privacy property. Parent
> **sq-1s2.5** (#2621); the whole ZK estate is remediated + internally re-audited
> but **NOT externally audited**, and external accredited-cryptographer sign-off
> (**sq-qhy4**, P0) is REQUIRED before any ZK soundness/privacy/integrity property
> may be relied upon. Nothing here is an established property — it is a scoped
> mechanism proposal and an **open external-audit obligation**.

## 0. Status and scope (read first)

This record designs the **canonical-form issuance conformance** mechanism at
**proposal grade**. It exists because the mechanism is already *named* as a
load-bearing precondition in three places without ever being *designed*:

- `research/zk-field-native-encoding.md` §5.6 elevates "force issuers to issue only
  canonical-form values" from a future option to a **NAMED PRECONDITION** for the
  dual-leaf value-FILTER lane, and §11 bead 5 asks to "design + implement the
  conformance mechanism".
- `research/zk-dual-leaf-issuer-desync-review.md` §6 fix 4 / §8 bead 4 elevates it
  to "a named value-lane precondition (the value-FILTER lane is honest-issuer-only
  until a canonical-issuance conformance mechanism exists)".
- `compliance/cryptoreview/gap-register.md` **CR-G8 obligation (4)** requires the
  external auditor to confirm "canonical-issuance conformance is named as the
  precondition that restores INV-VL", and `research/zk-configurable-commitment-security.md`
  (§ honest mitigants) records it as **CR-G8 obligation (1)/(4)**.

None of those *design* the mechanism: what a "conforming issuer" is, how conformance
is expressed and attested, how a relying party checks it, in what precise sense it
can be said to restore INV-VL, and what it eventually buys (the leaf collapse). This
record fills that gap so the auditor (sq-qhy4) and the maintainer have one place that
scopes it.

**Designing it sharpened the characterization, and this record CORRECTS the inherited
shorthand.** The mechanism specified here is **issuer self-attestation**: it gives a
relying party attribution and governance evidence (which keys claim the discipline,
signed and revocable on breach), but **no check that
`value_component = parse(committed_lexical)`** for any leaf. It therefore does
**NOT** restore INV-VL against a malicious or compromised issuer (§4). "Restores
INV-VL" is accurate only in the conditional sense *"for an issuer that actually
follows the discipline"* — i.e. it **scopes and attributes** the honest-issuer
assumption; it does not discharge it. The "against an adversarial issuer" framing in
the three sources above must be read (and is amended in
`zk-field-native-encoding.md` §5.6) as this scoped, self-attested precondition, not
as an adversarial-issuer conformance mechanism.

**This is a proposal, not a build.** It depends on the unbuilt dual-leaf encoder /
circuit (`sq-j506`, audit-gated) and on the `(method, circuit)` dispatch resolver
(`sq-cfmv`); it does not pre-empt them. It is implementable at research grade before
sign-off, consistent with the rest of the estate, but the INV-VL *restoration* it
describes is a **claimed** property conditional on the mechanism being built,
enforced, attested, and audited — never a resolved or safe claim today.

## 1. What is being restored, and why it was lost (recap)

`INV-VL` (defined in `research/zk-dual-leaf-issuer-desync-review.md` §1) is the
invariant that, for any literal consumed by a value-FILTER, **the compared numeric
value equals `parse(committed_lexical)`**. Today's `string-canonical` method
enforces INV-VL **in-circuit against an arbitrary committer** (including a malicious
*trusted* issuer), because the compared value and the committed lexical token derive
from ONE witnessed digit array (`filter_int.nr:67-92`, `filter_signed.nr:150-180`).
`CommitmentMethod::StringCanonicalV1.removes_inv_vl()` is `false`.

The `#769` **dual-leaf** method
(`Poseidon2([value_component, lexical_component, TYPE_CODE])`) witnesses the value
handle and the lexical-identity hash **independently**, with nothing tying them, so
it **REMOVES INV-VL** (`CommitmentMethod::DualLeafV1.removes_inv_vl()` is `true`,
recorded as the `secx:IssuerHonesty` assumption on its `secx:Soundness` property in
`crates/sparq-zk/ontologies/secprop-methods.ttl`). Binding them in-circuit would
re-derive the value from the lexical bytes — exactly the ~14.3k-gate blake3+parse
the value handle exists to delete — so consistency **cannot** be enforced in-circuit
without erasing the gate win (`zk-field-native-encoding.md` §5.1, confirmed against
the checkout). After the change the value-FILTER lane is sound only under an
explicit **honest-issuer-for-value** assumption.

The host-side same-leaf co-binding at ingest
(`zk-field-native-encoding.md` §6, `crates/sparq-zk/src/dual_leaf.rs`
`NonCanonicalValue` fail-closed path) makes **honest sparq ingest** unable to
self-desync, but it does **not** bind a malicious *external* issuer that commits a
desynced leaf off-sparq and signs it. Canonical-form issuance conformance is aimed
at *that* residual, but it does not **close** it: for an issuer that actually
follows the discipline the desync state is unreachable, while against a malicious
or compromised issuer it yields attribution and governance evidence only (§4).

## 2. What "canonical-form issuance conformance" means

Define a **conforming issuer** as a signing authority (a key in the trusted set `K`)
that is bound, by a checkable commitment, to the **canonical-issuance discipline**:

> **CI-DISCIPLINE.** For every hookable-datatype literal it commits under the
> dual-leaf method, the issuer emits the **canonical lexical form** of that
> datatype AND sets `value_component = parse(canonical_lexical)` derived from the
> *same* bytes it hashes into `lexical_component`. Equivalently: the issuer runs the
> §6 same-leaf co-binding fail-closed check that honest sparq ingest already runs,
> and never signs a leaf that fails it.

For an issuer that actually follows CI-DISCIPLINE, a desynced leaf
(`value_component = 18`, `lexical = "5"`) is **unreachable by construction**, so
INV-VL holds as an **issuance invariant** for that issuer's credentials — recovered
not in-circuit but at the trust boundary, exactly where the dual-leaf moved it.
The load-bearing qualifier is *actually follows*: no verifier-side check
establishes it (§3.2, §4), so this conditional **is** the honest-issuer assumption,
scoped to attested keys — not a mechanism that discharges it.

The key design question this record answers: **conformance is a property of an
issuer key — attested, with the *attestation* (not the conformance itself)
checkable — not a property the circuit can re-derive.** The mechanism therefore
lives at the issuance / registry / trust-graph layer, not in the Noir circuit. That
is intrinsic (§5.1): if the circuit could check it, we would not need the
precondition at all.

## 3. The mechanism (proposal)

Three layers, each grounded in an existing seam so this is additive, not a new
subsystem:

### 3.1 Expressing conformance — a security-property annotation

The estate already carries per-*method* security properties as `secx:` annotations
(`crates/sparq-zk/ontologies/secprop-methods.ttl`, the `sparq-zk::secprop` model).
Conformance is a property of an **issuer key**, so it is expressed as a
per-issuer-key assertion, proposed vocabulary:

```turtle
# The issuer <K> is bound to the canonical-issuance discipline for the dual-leaf
# method. PROPOSAL vocabulary — not minted until the mechanism is built + audited.
<issuer-key-K> secx:conformsTo zk:canonicalIssuanceConformance-v1 .
zk:canonicalIssuanceConformance-v1
    secx:appliesToMethod zk:poseidon2-dualleaf-v1 ;
    secx:discipline "value_component = parse(canonical_lexical), same-bytes, fail-closed"@en .
```

This mirrors the existing `secx:hasProperty` / `secx:assumption secx:IssuerHonesty`
shape: conformance turns the **dual-leaf's `secx:IssuerHonesty` assumption on the
value lane from an unconstrained trust into a *scoped, attested* one** for the
conforming key. It stays `secx:assurance secx:Claimed` (never `Proven`) and carries
`secx:auditStatus secx:ExternalSignOffPending` until sq-qhy4.

### 3.2 Attesting conformance — binding the claim to the signature

An annotation alone is cheap talk. To be load-bearing the conformance claim must be
**bound to the issuer key that signs commitments**, so a relying party cannot be
fooled by a conformance assertion made by someone other than the signer. Two
candidate bindings (decide at build time, audit-gated):

1. **Registry-level (lighter).** The conformance assertion is recorded against the
   issuer key in the same trust-graph / registry surface that already records
   `zk:cryptosuite` and the trusted-key set `K` (`sparq-zk::sig::resolve_signature_scheme`,
   the `K`-membership check on the verify path). The relying party's trust policy
   decides which keys in `K` it will accept for a value-FILTER — i.e. it treats
   "conforms to `zk:canonicalIssuanceConformance-v1`" as an *additional* membership
   predicate beyond "in `K`". This reuses the existing key-trust plumbing; its
   weakness is that conformance is a policy assertion *about* a key, not something
   the key itself signed.
2. **Signature-covered (stronger, preferred long-term).** The issuer includes a
   conformance marker in the signed commitment message
   (`commitment_message_with_status`, the digest the issuer Schnorr-signs), so the
   claim "this credential was issued under CI-DISCIPLINE" is **unforgeable and
   un-omittable** on the verify path — the same construction audit #12 used to make
   the status reference un-omittable (`registry.rs:60-65`). A relying party that
   requires the value lane then rejects any dual-leaf commitment whose signed message
   does not carry the conformance marker. This does not make a *lying* issuer honest
   — a malicious issuer can still sign a desynced leaf AND the marker — but it makes
   the conformance claim a **signed, attributable** statement, so desync by a
   marker-bearing key is a provable breach of the issuer's own signed attestation
   (a legal/governance hook, and a revocation trigger), not a silent capability.

Neither binding closes the irreducible gap that a *malicious* trusted issuer can
still choose to violate its own attestation (§4), and neither gives the verifier
any evidence of same-byte derivation — no check that
`value_component = parse(committed_lexical)` for the leaf at hand. Both are
therefore **issuer self-attestation**: attribution and governance evidence, not
conformance verification. An *enforceable* alternative — e.g. an audited
per-credential issuance proof that the leaf was derived from the committed lexical
bytes, or a trusted enforcement boundary the issuer cannot bypass — would be a
separate, audit-gated design with its own cost story (the in-circuit variant is
exactly the ~14.3k-gate bind §5.1 rules out) and is out of scope here. What
conformance buys is precise (§4), and the docs must not overstate it.

### 3.3 Checking conformance — a fail-closed relying-party gate

The relying party's value-FILTER acceptance becomes conditional, fail-closed:

- Accept a value-FILTER verdict over a dual-leaf commitment **only if** the signing
  key is attested `secx:conformsTo zk:canonicalIssuanceConformance-v1` under the
  chosen binding (§3.2) **and** the commitment was issued at-or-after that key's
  conformance epoch (§6 — under the signature-covered binding this is exactly "the
  signed message carries the marker"). Otherwise **reject** (or downgrade to
  "honest-issuer-only, not conformance-backed", per policy) — never silently
  accept, matching the
  closed-enum fail-closed discipline the `CommitmentMethod` registry already uses
  (`from_scheme_iri -> None`, `method_parse_is_fail_closed_on_unknown`).
- `string-canonical` commitments are unaffected (they retain in-circuit INV-VL and
  need no conformance attestation). This keeps the conservative default fully
  backward-compatible.

## 4. What conformance does and does NOT buy (it does not restore INV-VL against an adversarial issuer)

State the recovered guarantee precisely, because it is easy to overclaim:

- **Recovered (conditional):** for a credential issued by a conforming key that
  **actually follows** CI-DISCIPLINE, `value_component = parse(committed_lexical)` —
  INV-VL holds as an **issuance invariant**, and the desync state (§1) is
  unreachable *for that issuer*. The condition "actually follows" is not
  verifier-checkable (§3.2), so this bullet is the honest-issuer assumption scoped
  to attested keys, restated — not a new guarantee.
- **Value vs term identity (per-datatype, NOT general):** even under CI-DISCIPLINE,
  SPARQL value equality and RDF term identity coincide **only** for datatypes whose
  value handle is **injective over canonical RDF terms** — an explicitly
  enumerated, audited allow-list, never "hooked datatypes" generally. For the
  many-to-one datatypes (IEEE `-0.0`/`+0.0`, NaN payloads, decimal `"5.0"`/`"5.00"`
  at fixed scale — `zk-field-native-encoding.md` §3.3/§5.5) canonical issuance does
  NOT make the two interchangeable; they keep the distinct `lexical_component`
  identity handle and the structural reject-list (v) regardless of conformance, and
  any leaf-collapse eligibility (§5) is a **fail-closed per-datatype property**,
  not a consequence of conformance.
- **NOT recovered / irreducible:** conformance is a *trust* property, not a circuit
  constraint. A conforming key that is **compromised, coerced, or dishonest** can
  still sign a desynced leaf while claiming conformance; the verifier cannot detect
  post-commit desync (`zk-field-native-encoding.md` §6) and gains **no check that
  `value_component = parse(committed_lexical)`**. What the verifier gains is
  **attribution** (a signed, revocable claim, under the §3.2 signature-covered
  binding) — a *governance* mitigation, not a soundness mitigation. So conformance
  moves the value lane from "unconditional in-circuit soundness
  (string-canonical)" to "sound iff the attested conforming issuer was honest at
  commit time" — a **strictly weaker** guarantee than string-canonical's, and the
  **same** issuer-honesty assumption as the bare dual-leaf's, now scoped to
  attested keys and attributable on breach. It is defence bounding, not defence
  restoration to the string-canonical level.
- **Consequence for framing:** in the adversarial-issuer threat model the value
  lane is **honest-issuer-only both before AND after this mechanism exists** —
  "conformance-attested-issuer-only" narrows *which* issuers a relying party
  accepts and makes breach attributable; it does not remove the honesty
  requirement. The lane is never "sound against a malicious issuer" the way
  string-canonical is, and any inherited shorthand ("restores INV-VL",
  "precondition for relying on the value lane against an adversarial issuer") is
  accurate only in the conditional honest-conforming-issuer sense above. The
  gap-register (CR-G8) and any SKILL/README caveat must say exactly this;
  `check-privacy-claims.sh` passes because the statement is an obligation/negation,
  not a positive guarantee.

## 5. The eventual payoff — leaf collapse (retire the carried lexical hash)

Canonical-issuance conformance is also the **exit path** that retires the dual leaf
(`zk-field-native-encoding.md` §5.6 / §7 / §11 bead 5). Once a deployment relies on
conformance for its value lane, then **for a datatype on an explicitly enumerated,
audited allow-list whose value handle is injective over canonical RDF terms** (§4),
the carried `lexical_component` hash is redundant, and the leaf can collapse back
toward a single value-first leaf for those datatypes — recovering the commit-time
cost the dual leaf pays over value-only (`zk-field-native-encoding.md` §7). This is
a **second one-time recommit** and a separate audit-gated deliverable; it is out of
scope for this proposal beyond naming it as the payoff. It is NOT safe for the
many-to-one datatypes (`-0.0`/`+0.0`, NaN payloads, decimal `"5.0"`/`"5.00"` at
fixed scale — `zk-field-native-encoding.md` §3.3 / §5.5) without preserving a
distinct term-identity handle, so **leaf-collapse eligibility is a fail-closed
per-datatype property** — a datatype not affirmatively shown injective (and
audited as such) keeps its identity handle — never a generic consequence of
conformance, and never blanket.

## 6. Migration / recommit

If built, conformance is introduced without breaking existing credentials:

- Existing `string-canonical` commitments are untouched and remain the conservative
  default (they never needed conformance).
- Existing `dual-leaf` commitments (once `sq-j506` exists): conformance is
  effective **only from a registry epoch/version** — the point at which the key's
  attestation is recorded (§3.2 binding 1) or at which the issuer starts signing
  the conformance marker (§3.2 binding 2). A present-day attestation about a key
  is **not evidence that previously issued leaves were produced under
  CI-DISCIPLINE** (and under the signature-covered binding the marker is simply
  absent from their already-signed messages), so commitments issued **before** the
  key's conformance epoch stay **outside the conformance-backed lane**: the §3.3
  gate rejects them, or a policy **explicitly grandfathers** them under the
  labelled "honest-issuer-only, not conformance-backed" posture — never a silent
  extension of the present policy claim to historical issuance. Under the
  signature-covered binding, bringing a pre-epoch credential into the lane
  requires **reissuance/re-signing with the marker**; old credentials do not
  acquire that evidence retroactively. No leaf recommit is implied by attestation
  itself (only by *collapse*, §5, which is a separate deliverable).
- The relying-party gate (§3.3) is opt-in per policy, fail-closed, so a deployment
  adopts it deliberately.

## 7. Dependencies, non-goals, and ordering

- **Depends on:** `sq-j506` (dual-leaf host encoder + circuit — the thing whose
  INV-VL loss this restores; unbuilt, audit-gated) and `sq-cfmv` (the `(method,
  circuit)` dispatch resolver that structurally enforces reject-list (v)). This
  proposal does not pre-empt either.
- **Non-goals:** it does NOT make a malicious issuer honest (§4 irreducible); it does
  NOT add any in-circuit constraint (that is impossible per §5.1 and would erase the
  gate win); it does NOT change `string-canonical`; it does NOT itself perform the
  leaf collapse (§5, separate).
- **Ordering:** this is `zk-field-native-encoding.md` §11 **bead 5**, after beads
  1–4 (register CR-G8 → host encoding + same-leaf co-binding → circuit + B1/B4 →
  identity/desync regression guard). All audit-gated behind **sq-qhy4** for
  production reliance.

## 8. Honesty framing (load-bearing)

Nothing here is a security guarantee. sparq's v1 ZK verifier is remediated and
internally re-audited but **NOT externally audited**, documented **NOT-yet-sound**
for production reliance (`SECURITY.md`, `compliance/cryptoreview/gap-register.md`
CR-G1). External accredited-cryptographer sign-off (**sq-qhy4**, P0) is REQUIRED
before any ZK soundness/privacy/integrity property — including the scoped,
conditional INV-VL recovery this proposal describes (§4; issuer self-attestation,
not an adversarial-issuer guarantee) — may be relied upon. The dual-leaf method,
its removed INV-VL, and this conformance mechanism are **claimed** properties
(`secx:assurance secx:Claimed`, `secx:auditStatus secx:ExternalSignOffPending`),
never `Proven`. The MPC estate is semi-honest-only and not invoked here. No gate
counts or timings appear here; any that later do are `bb gates` snapshots, and EC2 /
work-box timings are NON-canonical. This proposal is an **open external-audit
obligation** (CR-G8 obligation (4)), not an established property.

## 9. Open questions for the maintainer

1. **Attestation binding (§3.2):** registry-level (lighter, reuses `K` trust) or
   signature-covered (stronger, un-omittable, revocation-triggerable on breach)?
   The proposal prefers signature-covered long-term but registry-level is a valid
   first step.
2. **Default relying-party posture (§3.3):** should a non-conforming dual-leaf
   value-FILTER **reject**, or **downgrade** to an explicitly-labelled
   "honest-issuer-only, not conformance-backed" verdict the policy layer can accept?
3. **Roadmap now or stated-precondition (mirrors `zk-field-native-encoding.md` §10
   Q6):** build the mechanism on the roadmap, or keep it as a named stated
   precondition that bounds *when* the value lane may be relied upon, until a
   deployment actually needs conformance-backed value FILTERs?
4. **Leaf collapse (§5):** is retiring the carried lexical hash for the
   one-to-one hooked datatypes wanted as a follow-on, given the many-to-one
   datatypes must keep a distinct identity handle?

## 10. Next steps (orchestrator owns bead structure)

This record IS the "design" half of `zk-field-native-encoding.md` §11 bead 5
(sq-mtv7). The "implement" half — the `secx:conformsTo` vocabulary + the §3.2
attestation binding + the §3.3 fail-closed relying-party gate — is a follow-on,
audit-gated behind sq-qhy4, and depends on `sq-j506` landing first. No code is
created here; per the shared contract the orchestrator owns any bead creation.
