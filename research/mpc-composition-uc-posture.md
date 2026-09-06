<!-- [OPUS-4.8] MPC composition / UC posture design record for sparq-mpc: the standalone/UC
statement the architecture (a composition) currently lacks — the masked-opening leakage lemma for
the mid-protocol `secure_equal` open; honest-majority UC without a CRS/PKI, GIVEN the UC theorems'
communication-model resources (Canetti FOCS'01), as the design-target argument for the
default; carry the coZK 2025/1026 composition-security caveat on the collaborative-proof layer.
Design-for-review (no code, doc-only), Opus 4.8 (Fable unavailable) — re-review when Fable
returns. Date: 2026-07-19. Bead sq-wj4k (parent #2629 → epic sq-pwr). -->

# MPC composition / UC posture for sparq-mpc

**Status:** Deep-research design record (no implementation; **doc-only**). Author: Opus 4.8
(Fable unavailable — flag for re-review). Date: 2026-07-19. Bead **sq-wj4k**
(parent **#2629**; sibling **sq-aaop**; epic **sq-pwr** — MPC over federated SPARQL + ZKP of
correctness + attested-source derivation).

**Scope.** sparq's MPC layer is not one protocol — it is a **composition** of sub-protocols
(holder → share → join → secure-aggregate → reconstruct → proof, driven by
`pipeline.rs::run_federated`). Yet no **standalone / UC (universally-composable) statement**
records *which* security results the pieces enjoy, *which* composition theorem carries them into
the whole, and *what obligations* each open/reveal step imposes on the stages downstream of it.
This record captures that posture. It is the honest framing artifact the security-models synthesis
named as a gap ([`mpc-security-models-and-benchmarks.md`](./mpc-security-models-and-benchmarks.md)
§ "Composability / UC"; [`mpc-sparql-capability-matrix.md`](./mpc-sparql-capability-matrix.md)
§ "Composability / UC"), and item 6 of the architecture roadmap
([`mpc-zkp-federated-sparql-design.md`](./mpc-zkp-federated-sparql-design.md) §8).

**This record EXTENDS, does not duplicate:**
- [`mpc-security-models-and-benchmarks.md`](./mpc-security-models-and-benchmarks.md) — the 3-axis
  `AdversaryModel × OutputGuarantee × CorruptionThreshold` taxonomy, the fail-closed registry, and
  the leakage analysis (L2 join match-structure leak). That doc *names* the composability gap; **this**
  doc is the composition/UC treatment.
- [`mpc-cozk-reaudit.md`](./mpc-cozk-reaudit.md) — the adversarial coZK re-audit vs eprint 2025/1026.
  That doc is the *soundness verdict* on the (unbuilt) collaborative-proof path; **this** doc places its
  witness-extension pitfall inside the composition framing (why it is a *composition-security* failure,
  and which composition obligation encodes the fix).
- [`mpc-malicious-security-design.md`](./mpc-malicious-security-design.md) — the IT-MAC upgrade to
  malicious-with-abort. That doc raises the *per-operator adversary tier*; **this** doc is orthogonal —
  it is about *how the operators compose*, at whatever tier each is realized.
- [`mpc-distributed-randomness-design.md`](./mpc-distributed-randomness-design.md) — which already
  cites Canetti honest-majority UC (FOCS'01) for the randomness beacon; **this** doc states the
  *conditional* version of that argument for the *whole pipeline's* default trust model, including
  the communication-model assumptions the citation carries (§1).

**One-line posture (stated up front, not over-claimed):** sparq-mpc's default —
**honest-majority Shamir, semi-honest today** — is a defensible *composition-minded design target*
because honest-majority protocols can achieve **UC security without a CRS or PKI** (Canetti, FOCS'01)
*given* the communication resources those theorems assume (ideal authenticated — and, for the
information-theoretic protocols, private — channels; broadcast where the protocol requires it; the
UC session/scheduling model), so a future networked realization need not import a CRS/PKI trust
root. Today's code is an in-process, single-session simulation with none of those resources, so the
composition statement is **conditional**: *if* each stage is realized as a distributed protocol and
proven standalone-secure under a precisely defined communication and corruption model, *then*
sequential modular composition carries the pieces into the whole. What is established now is
narrower: the one mid-protocol reveal (`secure_equal`'s open) is covered by a **masked-opening
distribution lemma** — the honestly-computed opened value is perfectly simulatable from the equality
bit alone (§3) — which is the ingredient the eventual stage simulator needs, **not** a realization
of `F_join`; and the collaborative-proof stage inherits a **composition-security obligation**
(validate the extended witness before proving, per coZK 2025/1026) that is **named but not yet
encoded** — so that stage stays fail-closed (`NotYetImplemented`) and no composed soundness claim
may be made until `sq-qhy4` (external audit) lands. **Nothing here is a production security claim;
this is the design posture, not a proof of security.**

---

## 0. Ground truth: the composition, verified against `origin/main`

The pipeline driver `crates/sparq-mpc/src/pipeline.rs` (`run_federated`, the four-flatmates worked
example) composes six stages, each already existing in isolation:

| # | Stage | Code | What it computes | Reveals mid-protocol? |
|---|-------|------|------------------|-----------------------|
| 1 | **Holder local eval** | `holder::Holder::evaluate_local` | each party evaluates a query fragment over its OWN graph via `sparq-engine`; raw graphs never leave | no (local only) |
| 2 | **Share** | `MpcBackend::share_private_input` (Shamir, `shamir.rs`) | secret-share each private value (e.g. salary) as a **degree-`t` Shamir** sharing over `n` parties, `t=⌊(n−1)/2⌋` — any `t` shares reveal nothing about the secret, `t+1` valid shares reconstruct it | no |
| 3 | **Join** | `join::DisclosedKeyJoin` (clear IRIs) / `join::HiddenValueJoin` (`secure_equal`) | equi-join; hidden-value variant opens `m=(a−b)·r` per pair | **YES — the danger open (§3)** |
| 4 | **Secure aggregate** | `MpcBackend::run_secure` (`shamir.rs`, zero-round local add) | cumulative SUM over hidden shares | no (linear, local) — **but the threshold DISCLOSURE that follows it in `pipeline.rs` step 4 (`compare::disclose_threshold_verdict`) does open, see below** |
| 5 | **Reconstruct** | open of the DISCLOSED result only, from `t+1` valid degree-`t` shares | open the final aggregate / verdict bit | yes — the *intended* output open |
| 6 | **Proof** | `proof::ProofStatement` / `CollaborativeProof` | attach a ZK/collaborative proof of correct evaluation | **stub — `NotYetImplemented` (§4)** |

**Correction carried by the sibling record (`sq-aaop`).** The stage table above is accurate for
`run_secure` itself, but it **understates the pipeline's mid-protocol opens**: `pipeline.rs` step 4
also runs `compare::disclose_threshold_verdict`, which opens **64** field elements per verdict (61
square-protocol opens for the Rabbit mask's solved bits, one Rabbit masked open `c = (x+r) mod p`,
one masked-product zero-test inside the in-protocol range proof, and the verdict bit). Each is
individually well-masked and one of them is only *statistically* simulatable. The open inventory for
that path, a simulator per open, and the composed error budget are in
[`mpc-simulator-sketch.md`](./mpc-simulator-sketch.md) §2–§4; §3 of *this* record covers one of the
four distinct open shapes it enumerates.

The same sibling record corrects stage **5** in the other direction: in `pipeline::run_federated`
step 5 reconstructs **nothing**. The disclosed join result comes from the crypto-free
`DisclosedKeyJoin` (never secret-shared) and the verdict bit was already opened in step 4, so that
path never calls `MpcBackend::reconstruct_disclosed`. The stage-5 row above describes the *general*
disclosed-output shape — real, and reached by other callers — not a step of `run_federated`. Note
also that the sibling's inventory is scoped to `run_federated` specifically, **not** to every
production semi-honest API; its §2.4 lists the reconstruction surfaces it excludes.

Load-bearing facts (all `origin/main`): the crate is an **in-process multi-party simulation** (every
party is a function call — NO real network, NO concurrent sessions, NO broadcast, NO round counter);
the default is **honest-majority Shamir, semi-honest**; `secure_equal` opens exactly one masked field
element per pair (`join.rs`, `secure_equal_leaks_full_bipartite_match_graph` pins the leak); every
`proof.rs` collaborative method returns `MpcError::NotYetImplemented`. **The composition is real code;
its security is the design question this record frames — it is not asserted as proven.**

---

## 1. Why a standalone/UC statement is needed (and which flavor)

A protocol that is secure *in isolation* need not stay secure when its output feeds another protocol,
or when run alongside other sessions. Two composition frameworks bound this:

- **Sequential modular composition (Canetti, J. Cryptology 2000; Goldreich Vol. 2 §7).** If each
  sub-protocol securely realizes an ideal functionality `F_i` in the standalone (stand-alone-simulation)
  sense, then the composed protocol that calls them **sequentially** (one finishes before the next
  starts, no concurrency) securely realizes the composed functionality. This is the *weakest* theorem
  that covers a straight-line pipeline like `run_federated`. **Precondition that bites sparq:** every
  value a sub-protocol OPENS mid-computation must be part of `F_i`'s *defined output* — the simulator
  for stage `i` must be able to produce that opened value's distribution from `F_i`'s output alone.
  An open that is NOT in `F_i` is unsimulatable and breaks the modular-composition premise (§3).
- **Universal Composability (Canetti, FOCS'01, *Universally Composable Security*).** The strong
  framework: security is preserved under **arbitrary concurrent composition** with any environment.
  UC is what sparq would *want* the day the in-process sim becomes a real networked protocol running
  concurrently with other federation queries. The pivotal result for the default:

> **Honest-majority protocols (`n ≥ 3`, `t < n/2`) achieve UC security without a CRS or PKI**
> (Canetti FOCS'01; Canetti–Lindell–Ostrovsky–Sahai STOC'02) — but NOT "without any assumptions":
> the theorems are stated relative to a communication model the protocol receives as ideal
> resources — authenticated (and, for the information-theoretic constructions, private/secure)
> point-to-point channels, a broadcast channel where the protocol/threshold requires one, and the
> UC framework's session-identifier and adversarial-scheduling conventions. Dishonest-majority UC
> additionally requires *cryptographic* setup (a CRS / PKI / correlated randomness) even given
> those communication resources.

**This is the composition argument FOR the honest-majority default.** Choosing HM is not only a
threshold choice — it is the choice that lets a future networked pipeline aspire to UC *without a
CRS/PKI trust root*, modulo the communication resources above, which that realization must actually
provide or instantiate. Today's in-process sim has no channels, authentication, broadcast, session
identifiers, or concurrency model at all (§0), so the cited result cannot be applied to it as-is. A
dishonest-majority backend (the truthfully-refused `sq-j5ok` slot) would drag cryptographic setup
back in even given those channels. So the default is composition-*motivated*, and this record is the
place that says so on the record.

**Which flavor sparq claims today: none unconditionally.** The artifact is an in-process,
single-session, straight-line sim in which one central driver executes every party's steps. That
means there are no per-party adversarial views to define, and the sharing / multiplication /
reconstruction / all-pairs-join stages do not exist as *distributed protocols* that could be
standalone-secure — a centrally-driven simulation is not itself a standalone-secure protocol. The
honest statement is therefore **conditional**: *if* each stage is realized as a distributed protocol
and proven standalone-secure — with a precisely defined communication model, corruption model,
per-party views, and simulators — *then* sequential modular composition (the weakest theorem,
matching the straight-line single-session shape) carries them into the composed pipeline.
UC-without-CRS is the further **target** the HM default keeps reachable, not a claim on today's code
(no networked concurrent protocol exists to be UC-secure). What is actually established today is the
§3 masked-opening distribution lemma. Stating more would be overclaim.

---

## 2. The composition obligations, per stage

For each stage: the ideal functionality it must realize, the standalone result that applies, and the
**residual obligation** the composition imposes. The "standalone result" column records what a
*distributed realization* of the stage could invoke; per §1, none of these stages currently exists as
a distributed protocol with defined adversarial views, so the table states obligations and applicable
results, **not** established realizations.

| Stage | Ideal functionality `F` | Standalone result | Composition obligation (residual) |
|-------|-------------------------|-------------------|-----------------------------------|
| 1 Holder eval | `F_local`: emit a fragment result over the party's own graph | trivial (local computation, no interaction) | outputs feeding stage 2/3 must be exactly what `F_local` defines — no side-channel from engine timing (out of model) |
| 2 Share | `F_share`: distribute a degree-`t` Shamir sharing over `n` parties (`t=⌊(n−1)/2⌋`) | Shamir secrecy: any `t` shares are independent of the secret, while `t+1` valid shares reconstruct it (Shamir'79); a distributed dealing protocol + view simulation is future work | randomness must be a CSPRNG (`rng.rs`, `SecureRng::from_os`) — a deterministic PRNG breaks privacy; `insecure-test-rng` off by default |
| 3 Join | `F_join`: output the join, **leaking the match bit per pair** (L2) | masked-opening lemma: the honest opened `m` is simulatable from the match bit (§3); realization of `F_join` pending a distributed protocol + simulator | **the leak MUST be in `F_join`** — the downstream aggregate composes against a functionality that *already leaked the match structure*; a `LeakageProfile` should surface it |
| 4 Aggregate | `F_sum`: threshold sum, open nothing | linear ops are local & perfectly private (no interaction) | none new for the SUM (zero-round); the **threshold disclosure** that follows it opens (see the §0 correction) and its `F_thresh` must leak BOTH the `> τ` verdict and the in-range clause of the in-protocol range proof ([`mpc-simulator-sketch.md`](./mpc-simulator-sketch.md) §3.5); any *chained* multiplication (degree reduction) adds an open → re-enters §3's obligation |
| 5 Reconstruct | `F_open`: open the DISCLOSED result only | opening from `t+1` valid degree-`t` shares is the intended output | the reconstructed value must be a *function of the ideal outputs*, not of intermediate shares |
| 6 Proof | `F_prove`: prove correct evaluation, leak nothing about honest witnesses | **UNBUILT + coZK-gated (§4)** | **validate the extended witness before proving** (2025/1026) — the unfilled composition obligation |

The pattern: **every mid-protocol open transfers a leakage obligation to the ideal functionality that
the next stage composes against.** Composition is sound iff each downstream stage is proven secure
*relative to a functionality that already accounts for the upstream leak*. sparq's job is to keep those
functionalities honest — the danger is a stage silently composing against a leak-free `F` that the real
upstream protocol does not deliver.

---

## 3. Justifying the `secure_equal` mid-protocol open

The `HiddenValueJoin::secure_equal` operator (`join.rs`) is the one built operator that **opens a value
mid-computation**: to test `a == b` it computes `d = a − b`, draws a uniform nonzero mask `r` (one
Shamir multiplication), and **opens `m = d·r`**. Then `m == 0 ⇔ a == b`.

**Why naive sequential composition does not *automatically* justify it.** Modular composition requires
the opened `m` be simulatable from `F_join`'s output. If `F_join` were defined to output *only the join
result* (leaking nothing else), the simulator would have to produce `m` from nothing — impossible when
`m` is a real field element the environment observes. So a leak-free `F_join` is the **wrong**
functionality; composing against it is unsound.

**The justification (and its exact precondition).** Model `F_join` to leak **exactly the match bit**
per pair. Then:
- when `a ≠ b`: `d ≠ 0`, and `r` uniform nonzero ⇒ `m = d·r` is **uniform over the nonzero field**,
  independent of `d` — so a simulator holding the bit "unequal" samples a uniform nonzero element,
  a perfect simulation;
- when `a = b`: `d = 0` ⇒ `m = 0` deterministically — the simulator holding "equal" outputs `0`.

Hence — **as a masked-opening distribution/leakage lemma** — the honestly-computed opened value `m`,
conditioned on the equality bit, is perfectly simulatable from that bit alone: the open adds nothing
beyond the bit in an honest execution. **This lemma is NOT a proof that `secure_equal` realizes the
bit-leaking `F_join`.** A realization claim would further require defining the real distributed
protocol — per-party adversarial views for the sharing, the masked multiplication, and the
reconstruction, over a stated communication and corruption model — and exhibiting a simulator for a
corrupted party's *entire view*, not just the opened value; none of that exists for the current
in-process sim, where one central driver plays every party (§1). The lemma is the ingredient that
eventual simulator would use to handle the open. Two obligations fall out, both already
partly encoded:

1. **The leak must be carried, not hidden.** `secure_equal_leaks_full_bipartite_match_graph`
   (`join.rs`) pins that `join` opens the WHOLE bipartite match graph over `O(|L|·|R|)` pairs — i.e.
   the join-key fan-out/multiplicity (leak L2). Any downstream stage (aggregate, proof) must compose
   against a functionality that *already* leaked this. The bit-vector variant `secure_equal_to_bit`
   (`compare.rs`) keeps the per-pair bit **secret-shared, never opened** — that is the composition-safe
   primitive to prefer when the bit itself must not surface (e.g. inside a larger circuit).
2. **The open must be consistency-checked against a malicious opener.** `m = d·r` is a degree-`2t`
   Reed–Solomon codeword over `n` party points; at the minimal honest-majority `n = 2t+1` it has **zero
   RS redundancy** (`shamir.rs reconstruct_degree`, `sq-7q9i`/WI-2 consistency-checked open), so a
   forged product share can silently flip a verdict and is information-theoretically undetectable at
   minimal `n`. Under semi-honest this is out of model; the **composition-with-malicious-security**
   story needs the IT-MAC upgrade ([`mpc-malicious-security-design.md`](./mpc-malicious-security-design.md),
   §8 step 5) so the open MAC-checks before revealing — the SPDZ discipline of *check-then-open* is
   itself a composition-safety mechanism.

**Bottom line for §3:** what is established is the **leakage characterization** — for honest
executions the open reveals nothing beyond the per-pair match bit (the lemma above), so the
bit-leaking `F_join` is the right functionality for downstream stages to compose against. Full
composition *justification* of the open remains conditional on §1's realization proofs (distributed
protocol + views + simulators); and the open is **not** safe against a malicious opener
at minimal `n` without the authenticated-open upgrade. Operators that open NO value mid-chain (e.g. the
bounded-path design's no-mid-chain-open property,
[`mpc-bounded-property-path-design.md`](./mpc-bounded-property-path-design.md)) sidestep this obligation
entirely and are the composition-cleanest building blocks.

---

## 4. The collaborative-proof layer: a composition-security failure the UC lens catches

The proof stage (stage 6) is where composition security bites hardest, and it is why this record and
the coZK re-audit are twinned. Per **eprint 2025/1026** (Garg–Goel–Jain–Roberts–Sekar, CRYPTO'25,
*Malicious Security in Collaborative zk-SNARKs*), a collaborative prover that proves on an
**inconsistent / maliciously-extended witness** can **LEAK the honest parties' inputs**. In the
composition framing this is precisely a **composition-security failure**: the proof sub-protocol, when
composed after the MPC evaluation, does NOT realize a leak-free `F_prove` — a corrupted prover uses the
proving step as a channel to exfiltrate honest witness data. A stand-alone "the SNARK is
zero-knowledge" claim is exactly the kind of local reasoning a UC/composition treatment **refutes**: ZK
of the proof object does not imply zero leakage *of the composed protocol* when the witness can be
adversarially extended between the MPC output and the prover input.

**The composition obligation this imposes:** the extended witness must be **validated (consistency-checked)
before proving** — the "fix" 2025/1026 names. sparq has **named but not encoded** this obligation
([`mpc-cozk-reaudit.md`](./mpc-cozk-reaudit.md) §3, RE-OPEN), and every collaborative-proof method is a
fail-closed `MpcError::NotYetImplemented` stub. That fail-closed state is the **correct** composition
posture today: since the validate-before-prove obligation is not enforceable, the only honest move is to
NOT compose the proof stage at all. The `secure_equal` mid-pipeline open (§3) is the *live in-crate
instance* of the same family — a mid-computation reveal whose downstream composition must account for
what it exposed (coZK re-audit Lens 1).

**No production/attestation claim over the composed proof path may be made** until (a) the path is
built, (b) validate-before-prove is encoded and enforced, and (c) the **external** cryptographer audit
**`sq-qhy4`** covers the *multi-prover* construction (not only the single-prover verifier). This is the
LIVE privacy-claims gate; any ZK/MPC soundness statement stays research-grade / not-externally-audited
(`sq-qhy4`), MPC semi-honest-only.

---

## 5. Which results apply — the summary map

| Question | Result / posture | Applies to sparq how |
|----------|------------------|----------------------|
| Does the straight-line pipeline compose? | **Sequential modular composition** (Canetti J.Crypto'00; Goldreich) | **CONDITIONAL** — applies once each stage is realized as a distributed protocol proven standalone-secure under a stated communication/corruption model, with each open in its stage's `F` (§1–§3); NOT established for the in-process sim |
| Why is honest-majority the default? | **UC without a CRS/PKI for honest majority, GIVEN ideal authenticated/private channels (+ broadcast where required)** (Canetti FOCS'01; CLOS STOC'02) | keeps a no-CRS/PKI trust root reachable for a future networked realization; DM would need cryptographic setup even given those channels (`sq-j5ok` refused) |
| Is the `secure_equal` open OK? | **Masked-opening distribution lemma** — the honest opened value is simulatable from the match bit (§3); realization of the bit-leaking `F_join` pending a distributed protocol + simulator | leak L2 must be carried downstream + a checked open; NOT malicious-safe at minimal `n` without IT-MACs |
| Concurrent / networked composition? | **UC (FOCS'01)** is the target framework, with its communication-model assumptions (§1) | ASPIRATIONAL — no networked concurrent protocol exists yet; do not claim UC on today's sim |
| Collaborative proof composition? | **coZK 2025/1026** — validate extended witness before proving | OBLIGATION named, NOT encoded; stage stays `NotYetImplemented`; gated by `sq-qhy4` |
| Malicious-with-abort composition | check-then-open (SPDZ/IT-MAC) is a composition-safety mechanism | the [`mpc-malicious-security-design.md`](./mpc-malicious-security-design.md) upgrade closes the §3 checked-open hole |

---

## 6. Non-goals, honesty caveats, and follow-ups

- **This is a posture record, not a proof.** No formal simulator is constructed here — neither UC
  nor standalone: no per-party adversarial views are defined for the sharing / multiplication /
  reconstruction / join stages (the sim is centrally driven), and the only distribution argument
  made is the §3 masked-opening lemma. The record states *which* theorems would apply to a future
  distributed realization and *what obligations* each imposes. A machine-checkable or
  paper-grade composition proof is a heavier, separate, audit-gated deliverable — and any UC/composition
  soundness statement inherits the `sq-qhy4` external-audit gate.
- **In-process sim ≠ networked protocol.** Everything about concurrent/UC composition is aspirational
  until a real networked, multi-session protocol exists. Claiming UC on the current single-process sim
  would be overclaim.
- **Semi-honest default.** The §3 justification is semi-honest; the malicious-opener hole at minimal
  `n = 2t+1` is real and is closed only by the IT-MAC upgrade, not by this record.
- **Sibling bead `sq-aaop` — LANDED as [`mpc-simulator-sketch.md`](./mpc-simulator-sketch.md).**
  Listed with `sq-wj4k` in the roadmap as "the composition / UC posture design records." This record
  covers the composition-obligations + which-results-apply framing for the MPC pipeline; `sq-aaop`
  took the distinct slice reserved for it — the *formal simulator sketch / paper-grade write-up*
  (the model, the ideal functionalities with their leakage written out, the open inventory of one
  fixed entry point — `pipeline::run_federated` — with a simulator and simulation quality per open,
  the excluded-surface list that bounds that scope, the hybrid composition with a concrete error
  budget, the unfilled-obligation ledger, and `F_prove^{val}` as the functionality-level statement
  of validate-before-prove). One source of truth: it extends this record and does not re-state it.
- **Surface a `LeakageProfile`.** §2/§3's "carry the leak downstream" obligation wants a machine-readable
  per-operator leakage descriptor so a federation can reason about residual composed leakage; that is an
  implementation follow-up, tracked separately (out of scope for this doc-only record).

---

## References

Canetti, *Security and Composition of Multiparty Cryptographic Protocols*, J. Cryptology 2000 (sequential
modular composition). Canetti, *Universally Composable Security: A New Paradigm for Cryptographic
Protocols*, FOCS'01 (UC; honest-majority UC without a CRS/PKI, given the framework's communication
resources — §1). Canetti–Lindell–Ostrovsky–Sahai, *Universally
Composable Two-Party and Multi-Party Secure Computation*, STOC'02. Goldreich, *Foundations of Cryptography
Vol. 2*, §7 (modular composition). Shamir, *How to Share a Secret*, CACM 1979. Garg–Goel–Jain–Roberts–Sekar,
*Malicious Security in Collaborative zk-SNARKs: More than Meets the Eye*, eprint 2025/1026 (CRYPTO'25).
In-crate: `crates/sparq-mpc/src/{pipeline,join,compare,shamir,proof,rng}.rs`; companions
[`mpc-cozk-reaudit.md`](./mpc-cozk-reaudit.md), [`mpc-security-models-and-benchmarks.md`](./mpc-security-models-and-benchmarks.md),
[`mpc-sparql-capability-matrix.md`](./mpc-sparql-capability-matrix.md),
[`mpc-malicious-security-design.md`](./mpc-malicious-security-design.md),
[`mpc-distributed-randomness-design.md`](./mpc-distributed-randomness-design.md),
[`mpc-zkp-federated-sparql-design.md`](./mpc-zkp-federated-sparql-design.md). Gates: `sq-qhy4` (external
audit), collaborative proof `NotYetImplemented` fail-closed.
