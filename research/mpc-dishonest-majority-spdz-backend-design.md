<!-- [OPUS-4.8] Dishonest-majority (SPDZ/MASCOT/Overdrive) backend design record for sparq-mpc.
Design-for-review SPIKE (no code, doc-only), Opus 4.8 (Fable unavailable) — re-review when Fable
returns. Date: 2026-07-19. Bead sq-j5ok (parent epic #2629 / MPC epic sq-pwr). btype=spike. -->

# Dishonest-majority SPDZ/MASCOT/Overdrive backend: the trait-compatible slot-in, the preprocessing tax, and why it stays research-only

**Status:** Deep-research design record — **SPIKE, no implementation, doc-only** (`btype=spike`).
Author: Opus 4.8 (Fable unavailable — flag for re-review). Date: 2026-07-19.
Bead **sq-j5ok** (parent #2629; MPC epic **sq-pwr**).
Depends on **sq-mq8q** (3-axis descriptor refactor, CLOSED) and **sq-a6p1** (fail-closed selection
API, CLOSED) — both landed in `backend.rs`, so this record specs against real, shipped seams.

**Scope.** Pin the exact trait-compatible shape a **dishonest-majority, malicious-with-abort**
(SPDZ-family) backend would take behind the unchanged `MpcBackend` trait: the authenticated
additive `Share = value + MAC`, the input-independent Beaver-triple + MAC **preprocessing** phase
(the **MASCOT-OT** vs **Overdrive-AHE** choice for triple generation), and the batched **MAC-check
before open**. Spec the three **new `BackendInfo` fields** this regime forces
(`requires_preprocessing` + its cost, `pq_posture` per component, `trusted_setup`) so a federation
can budget the usually-hidden-dominant offline cost and reason about PQ / setup. Deliver an
**HONEST verdict**: this backend stays **research-only for the SPARQL query pipeline** (there are
**zero published dishonest-majority-malicious instances for any SPARQL/graph-query operator**), it
is **truthfully REFUSED fail-closed** by the shipped selection API today, and **nothing here is
implemented or audited**. This is the `DishonestMajority`-marker-made-concrete-on-paper deliverable
— the AXIS-3 (`SemiHonest/Honest → Dishonest` corruption-threshold) continuation of the AXIS-1
(`SemiHonest → Malicious`) work in [`mpc-malicious-security-design.md`](./mpc-malicious-security-design.md).

**This record EXTENDS, does not duplicate:**
- [`mpc-malicious-security-design.md`](./mpc-malicious-security-design.md) (sq-km34) — the IT-MAC /
  authenticated-secret-sharing construction for **honest-majority** malicious-with-abort. §2.4
  route (b) there already names *authenticated Beaver triples* as "the route that generalises to
  dishonest-majority (the triples come from OT/SHE then, not from honest-majority resharing) and is
  the natural home for the `requires_preprocessing` cost field" and explicitly defers to **this**
  bead (its §6 optional continuation, and §3/§sources "sq-j5ok, truthfully-refused"). This record
  is that continuation: same MAC machinery, but additive sharing + preprocessed triples + **no
  honest-majority reduction**, tolerating up to `n−1` corruptions.
- [`mpc-sparql-capability-matrix.md`](./mpc-sparql-capability-matrix.md) — names the whole
  `DishonestMajority` column: "SPDZ/MASCOT/Overdrive additive + IT-MAC; preprocessing-heavy
  (Beaver triples + MACs) — **no backend** — registry refuses (fail-closed, sq-a6p1)" (§ tables,
  ~L292/L307), §4.4 "the cost of going semi-honest → malicious" (DM column = "SPDZ MAC-check
  before open (adds the preprocessing tax)"), and open-item #11
  (`BackendInfo.requires_preprocessing` field, → sq-4i39). This record is the *construction +
  field spec* behind those cells.
- [`mpc-security-models-and-benchmarks.md`](./mpc-security-models-and-benchmarks.md) — the 3-axis
  framing (§1.2), the primitive table row "Additive + SPDZ/MASCOT/Overdrive | dishonest-majority
  malicious (abort) | online O(n); offline O(n²) channels | Beaver-triple-consumed; preprocessing
  dominates | … | no (documented slot-in behind the trait)" (~L185), and §6 (the preprocessing /
  PQ / trusted-setup gaps: "`BackendInfo` has NO `requires_preprocessing`/cost field … no
  per-component PQ-posture field exists anywhere"). This record turns those three named gaps into a
  field spec.
- [`mpc-zkp-build-out-delta.md`](./mpc-zkp-build-out-delta.md) §M-F (the dishonest-majority + WAN
  frontier) — done-definition: "design records + `BackendInfo.requires_preprocessing`/PQ/
  trusted-setup fields so a federation can budget the usually-hidden-dominant offline cost." **This
  record IS that design record for the DM-backend half of sq-j5ok.**
- The honest-SOTA verdict in the `mpc-protocols` skill and
  [`mpc-zkp-research-and-architecture.md`](./mpc-zkp-research-and-architecture.md) (§3.1: "Dishonest-
  majority malicious (SPDZ/MASCOT/Overdrive) is the realistic cross-org model but pays expensive
  input-independent preprocessing usually excluded from headline numbers").

**The achieved status, stated up front (and NOT over-claimed):** this is a **spike design record
only**. It delivers **no code, no backend, no crypto, and no audit**. It pins what a DM backend
*would* look like behind the trait and what fields `BackendInfo` *would* grow, and it confirms the
selection API **already** refuses the DM-malicious request fail-closed and will keep doing so until
a real backend is built **and externally audited**. SPDZ/MASCOT/Overdrive are well-studied for
*generic arithmetic circuits*; their instantiation over the **SPARQL operator pipeline** at
dishonest majority is **unbuilt and unpublished**, so this stays research, not engineering.

---

## 0. Ground truth: the seam this would slot into (verified against `origin/main`)

The trait and registry that a DM backend must fit are already shipped (all citations `origin/main`):

- **The trait absorbs the scheme change by design.** `MpcBackend` (`backend.rs:693`) has an
  associated `type Share` (`backend.rs:696`) and three unchanged method signatures
  (`share_private_input`, `run_secure`, `reconstruct_disclosed`, `backend.rs:724–783`). The
  module docs already spell out the DM slot-in (`backend.rs:47–62`): "a SPDZ-style
  dishonest-majority backend's share is an *authenticated* additive share (value + MAC tag) with a
  preprocessing (triples) phase … (1) an input-independent preprocessing step producing Beaver
  triples + MACs; (2) `run_secure` consuming triples for multiplications and tracking MACs; (3)
  `reconstruct_disclosed` doing a MAC-check before opening (abort on cheat)". **This record does
  not change the trait** — it fills in that documented shape.
- **The three-axis descriptor already NAMES the DM regime.**
  `CorruptionThreshold::DishonestMajority { t }` (`backend.rs:306–313`) exists, is `t < n`, and is
  the *strongest regime to operate under* (`threshold_rank`, `backend.rs:870–876`).
  `AdversaryModel::Malicious` and `OutputGuarantee::Abort(AbortKind::Unanimous)` exist. **Cleve is
  a type-level invariant** (`backend.rs:261–297`): `OutputGuarantee::fairness/guaranteed_output`
  return `None` under `DishonestMajority`, so a DM backend is *unable to construct* a
  fairness/GOD claim — the impossibility is enforced at construction, not asserted.
- **The selection API already refuses DM-malicious fail-closed.**
  `SecurityRequirement::dishonest_majority_malicious()` (`backend.rs:1014–1022`) is a named
  constructor for "the request we truthfully refuse"; `BackendRegistry::select`
  (`backend.rs:1152`) returns `MpcError::NoBackendSatisfies` when no DM backend is registered — and
  the test `select_fails_closed_on_dishonest_majority_malicious_over_shipped_set`
  (`backend.rs:1674–1702`) pins exactly that. The shipped set is one honest-majority semi-honest
  Shamir backend (`shamir.rs`); it is refused, never downgraded.
- **`BackendInfo` today has NO preprocessing / PQ / trusted-setup fields.** It carries `name`,
  `security`, `trust_model`, `malicious_security` (`backend.rs:617–641`). The three fields this
  record specs (sq-4i39 and its PQ/setup siblings) **do not exist yet** — that is the honesty gap
  the capability-matrix (§6, open-item #11) and security-models (§6) docs both flag.
- **The crate is an in-process simulation.** Every "party" is a function call; there is no real
  network, no real OT, no real AHE. So even a *built* DM backend would simulate the offline phase's
  *effect* (produce triples) and measure its *modelled cost* via `metrics.rs`/`transport.rs` — it
  would **not** exercise real MASCOT-OT / Overdrive-SHE wire crypto. This is a first-class honesty
  caveat: a DM backend here would be a *cost-and-shape model*, not a deployable protocol.
- **The MAC-check error path already exists.** `MpcError::MacCheckFailed { detail }`
  (`partial.rs:142`) is the DM/authenticated abort signal, distinct from the Reed–Solomon
  `Tampered` (`partial.rs:110`). A DM backend's `reconstruct_disclosed` would return it on a failed
  MAC-check.

---

## 1. The exact trait-compatible SPDZ shape

SPDZ (Damgård–Pastro–Smart–Zakarias, CRYPTO'12) secures arithmetic circuits over `F_p` against
**up to `n−1` malicious corruptions** (dishonest majority) with **abort**. sparq's field is already
`F_p`, `p = 2^61 − 1` (`field.rs`), so the arithmetic domain matches. Adapted to the trait:

### 1.1 The share: authenticated **additive** sharing (`type Share`)

Unlike honest-majority Shamir (a degree-`t` polynomial sharing, private only up to `t`), SPDZ uses
**additive** sharing plus a global information-theoretic MAC:

```text
Global MAC key   α ∈ F_p        additively shared:  α = Σ_i α_i   (no party knows α)
Authenticated
sharing of x:    [[x]] = ( x_i , m_i )_{i=1..n}    with  Σ_i x_i = x ,  Σ_i m_i = α · x
```

Each party `i` holds `(x_i, m_i)` — its additive share of the value **and** its additive share of
the MAC `α·x`. This is the DM backend's `type Share` (an `AuthenticatedAdditiveShare` /
`Vec` thereof for batched columns), and it hides behind the associated `type Share`
(`backend.rs:696`) exactly as the registry docs promise — the `join`/`proof`/`pipeline` layers
compose onto it **unmodified**.

**Contrast with the honest-majority authenticated Shamir of sq-km34 (§2.2 there):** that carries
*two degree-`t` Shamir sharings* `([x],[α·x])`, private up to `t` and relying on the honest
majority for degree-reduction. SPDZ carries *two additive sharings*, private up to `n−1` and
relying on **preprocessing** (not an honest majority) for multiplication. Same MAC idea, different
sharing and different multiplication engine — that is the whole AXIS-3 move.

### 1.2 Online phase (consumes preprocessing)

- **Linear ops — FREE, no interaction, MAC carried for free** (§ same algebra as sq-km34 §2.3):
  `[[x]]+[[y]]`, `c·[[x]]`, `[[x]]+c` are all local; the public-constant MAC term uses the shared
  `[[α]]` / a party-indexed convention. The zero-round cumulative-SUM aggregate (`run_secure`,
  `shamir.rs:794` analogue) stays zero-round.
- **Multiplication — Beaver's trick, consuming ONE authenticated triple per gate.** With a
  preprocessed authenticated triple `([[a]],[[b]],[[c]])`, `c = a·b`: open `ε = x−a` and `ρ = y−b`
  (both masked by fresh, input-independent preprocessed randomness, so they leak nothing), then
  `[[z]] = [[c]] + ε·[[b]] + ρ·[[a]] + ε·ρ` — all **linear** post-opening (the `ε·ρ` term is a
  public constant, added by the party-indexed public-constant convention), so the MAC is carried
  for free and the two openings are themselves MAC-checked. The identity checks out:
  `c + ε·b + ρ·a + ε·ρ = ab + (x−a)·b + (y−b)·a + (x−a)(y−b)
  = ab + (xb − ab) + (ya − ab) + (xy − xb − ay + ab) = x·y`. Concretely over `F_7` with
  `x=3, y=5, a=1, b=2, c=2`: `ε=2, ρ=3`, and `2 + 2·2 + 3·1 + 2·3 = 15 ≡ 1 = 3·5 (mod 7)`. ✓
  (NOTE: the prior record `mpc-malicious-security-design.md` §2.4 route (b) writes the erroneous form
  `[[w]] + ε·[[y]] + ρ·[[x]] + ε·ρ`, which double-counts the cross terms and does NOT expand to
  `x·y` — it needs `−ε·ρ` to be correct in that variable choice. This record supersedes it;
  reconciling the prior record is captured as a follow-up.) Every product **consumes one
  triple**; this is the resource the offline phase manufactures.
- **MAC-check before open (the catch-everything step).** Just before `reconstruct_disclosed`
  reveals any value (the equality verdict, the aggregate sum, the comparison boolean), run the
  SPDZ batched random-challenge `MAC-Check`: draw public `χ_j` *after* shares are fixed, form the
  local share of `σ = Σ_j χ_j·m_{y_j} − (Σ_j χ_j·y_j)·α_i`, open `σ`, **accept iff `σ = 0`**, else
  `MpcError::MacCheckFailed`-abort. Soundness `≈ 1 − 1/p ≈ 1 − 2^-61` per check over `F_p`
  (identical statistical parameter to sq-km34 §2.5). `σ` is zero for honest runs and leakage-free.

### 1.3 Output guarantee: **unanimous abort, NOT identifiable** (an honest nuance)

Plain SPDZ with a single global MAC gives **`Abort(AbortKind::Unanimous)`** — a cheat is detected
and all honest parties abort, but **no cheater is attributed**. **Identifiable abort (IA)** at
dishonest majority is a *strictly heavier* construction (pairwise/BDOZ-style MACs, or the SPDZ2k-IA
line — Baum et al. eprint 2020/767, Cunningham–Fuller-style pairwise-MAC IA eprint 2023/1548). This
matters for the selection API (§4): the shipped
`SecurityRequirement::dishonest_majority_malicious()` (`backend.rs:1014–1022`) demands
`min_output_guarantee = Abort(Identifiable)` **and** `require_cheater_attribution = true`, so **even
a hypothetical plain-SPDZ backend would still be refused** for that exact named requirement — it
delivers unanimous, not identifiable, abort. A DM backend must therefore report
`AbortKind::Unanimous` (not `Identifiable`) unless it actually builds the pairwise-MAC IA layer —
and must **not** advertise the heuristic `Tampered{cheaters}` set as IA (the same discipline
`backend.rs:194–199` already imposes on the Shamir backend).

### 1.4 The preprocessing: **MASCOT-OT vs Overdrive-AHE** (the one real design fork)

The online phase above is cheap and identical regardless of *how* the authenticated triples are
made. The entire cost — and the entire cryptographic-assumption / PQ / trusted-setup surface — lives
in the **input-independent offline phase** that manufactures the triples. Two published routes:

| Route | Triple source | Assumption / crypto | Setup | PQ posture | Cost shape |
|---|---|---|---|---|---|
| **MASCOT** (Keller–Orsini–Scholl, eprint 2016/505) | **OT extension** (correlated OT → authenticated triples) | symmetric-key + **base OTs**; no public-key per-triple | base OTs only (no CRS) | base OT can be instantiated PQ (lattice/isogeny); OT-extension is symmetric ⇒ **PQ-plausible** | communication-bound; O(n²) pairwise OT channels; more comm per triple |
| **Overdrive / LowGear / HighGear** (Keller–Pastro–Rotaru, eprint 2017/1230) | **AHE** (BGV additively-homomorphic encryption) | **lattice** (BGV) public-key + zero-knowledge proofs of correct ciphertexts | distributed key generation (a shared/threshold BGV key) — **not a trusted CRS**, but a distributed key-setup | lattice-based ⇒ **PQ-plausible**, but the ZK proofs of plaintext knowledge are the subtle part | computation-bound; fewer rounds/less comm at scale; heavier local crypto |

**Recommendation for a would-be sparq DM backend: default to MASCOT-OT** for the spike, because (a)
it needs **no distributed key setup** (base OTs only → cleanest `trusted_setup` story, §3.3), (b)
its assumptions are the simplest to state honestly (symmetric-key + OT), and (c) the OT layer is the
easier one to give a credible PQ instantiation. **Document Overdrive-AHE as the throughput
alternative** (better amortized cost at large batch sizes / many triples, the regime a real
federation would hit), gated behind the same trait with a different `PreprocessingMethod` tag in the
`requires_preprocessing` field (§3.1). Both are **out of scope to build here**; the fork is recorded
so the field spec can represent either.

---

## 2. What it ADDS vs Shamir (and vs honest-majority authenticated Shamir, sq-km34)

Precisely, holding the SPARQL operator pipeline fixed:

1. **Drops the honest-majority assumption entirely.** Privacy holds against up to `n−1` colluding
   parties (additive sharing) rather than `≤ t = ⌊(n−1)/2⌋` (Shamir). This is the *only* reason to
   pay for it: the realistic hostile cross-org federation where a majority of pods may collude.
2. **Replaces honest-majority degree-reduction with preprocessed triples.** Shamir multiplication
   uses the BGW/DN07 `degree_reduce` re-sharing (`shamir.rs:406`), which *assumes* an honest
   majority (sq-km34 Hole 2). SPDZ multiplication consumes an offline-generated authenticated
   Beaver triple — no honest-majority reduction, correctness enforced by the MAC.
3. **Adds an input-independent OFFLINE PHASE — the dominant, usually-excluded cost (§3).** HM Shamir
   needs *none* for its linear aggregate and single-equality-open (`mpc-security-models §6`: "Honest-
   majority Shamir needs NONE for its current ops"). SPDZ's online is cheap *because* the expensive
   part was moved offline. A federation that budgets only the online phase under-counts the true
   cost by (typically) an order of magnitude — the Ozdemir-Boneh / ORQ / S&S-2022-survey lesson.
4. **Adds computational + public-key/OT assumptions the Shamir core does not have.** Shamir over
   `F_p` is information-theoretic (PQ for free). MASCOT adds OT (symmetric + base OTs); Overdrive
   adds lattice AHE. The MPC *core* stops being unconditionally PQ; PQ now depends on the offline
   route's instantiation (§3.2). This is a genuine **PQ regression surface** that must be reported,
   not hidden.
5. **Adds O(n²)-channel offline communication.** Online is O(n); the offline triple generation is
   O(n²) pairwise channels (security-models primitive table). In the in-process sim this is
   *modelled*, not real — another honesty caveat.
6. **Does NOT add fairness or guaranteed output.** Cleve forbids GOD/fairness under a dishonest
   majority (already a type-level invariant, `backend.rs:261–297`); the ceiling is abort. A single
   malicious party can always force an abort.

**Net:** vs Shamir, SPDZ buys *dishonest-majority privacy* at the price of a *dominant offline
phase*, *computational assumptions*, and *a PQ-posture that now depends on the offline route*. Every
one of those three prices is exactly what the new `BackendInfo` fields (§3) exist to surface.

---

## 3. The three new `BackendInfo` fields (design-only spec — sq-4i39 and siblings)

The task is to **spec** (not implement) three additive-only, back-compatible fields on `BackendInfo`
(`backend.rs:617–641`) so a federation can budget the offline cost and reason about PQ/setup. All
three follow the sq-mq8q discipline: **source of truth is the richer field; back-compat is
preserved so every existing `BackendInfo::new` caller keeps compiling.** Concretely, keep
`BackendInfo::new(name, security, robust_cheaters)` as the common constructor that fills these three
with their **honest defaults** (no preprocessing, PQ-core, no trusted-setup — the shipped Shamir
backend's truthful posture), and add a builder (or a `BackendInfo::new_with_offline(...)`) that a DM
backend uses to set them. That way adding the fields does not break the ~30 existing
`BackendInfo::new` call sites in `backend.rs`/`shamir.rs`/tests.

### 3.1 `requires_preprocessing: Option<PreprocessingCost>` (sq-4i39)

`None` for a backend whose shipped operators need no offline phase (HM Shamir today). `Some(cost)`
for a DM/SPDZ backend. The cost is a **descriptor a federation reads to budget**, deliberately
*without* a hard-coded number (the actual figure is measured by `metrics.rs`/`transport.rs`, per the
crate's no-fabricated-numbers rule — capability-matrix §4.4):

```rust
/// How the offline (input-independent) preprocessing is produced and what it costs
/// PER unit of online work — so a federation can budget the usually-hidden-dominant
/// offline phase (the Ozdemir-Boneh / ORQ / S&S-2022 lesson). DESIGN SKETCH — not built.
pub struct PreprocessingCost {
    /// Which offline engine manufactures the authenticated triples (§1.4).
    pub method: PreprocessingMethod,        // MascotOt | OverdriveAhe | HonestMajorityResharing
    /// Authenticated Beaver triples consumed PER secure multiplication gate (SPDZ: 1).
    pub triples_per_multiplication: u32,
    /// Authenticated input masks consumed PER private input shared.
    pub input_masks_per_input: u32,
    /// The offline phase's channel complexity, as a documented enum (not a number):
    /// SPDZ offline is O(n^2) pairwise channels; online is O(n). Lets a federation see
    /// the scaling class without a fabricated timing.
    pub offline_channel_complexity: ChannelComplexity,   // Linear | Quadratic
    /// Honest free-text note: e.g. "offline dominates; excluded from headline online
    /// numbers — measure via metrics.rs before any perf claim".
    pub note: &'static str,
}
```

The key honesty property: a `Some(_)` here is the machine-readable statement "this backend has an
offline tax you must budget", which the shipped Shamir backend truthfully reports as `None`.

### 3.2 `pq_posture: PqPosture` — **per component** (security-models §6: "No per-component PQ-posture field exists anywhere")

Post-quantum status is **not** one bit — it differs per layer (capability-matrix §6): the SS core is
information-theoretically PQ, the offline route's crypto may or may not be, and the attestation/proof
layer is classically-broken today. Report each:

```rust
/// PQ status of ONE cryptographic component. DESIGN SKETCH — not built.
pub enum PqStatus {
    /// The COMPLETE concrete stack for this component has an established PQ posture
    /// (information-theoretic, or symmetric-key-only, or an audited PQ instantiation).
    /// Reserved: a component whose posture depends on an unresolved sub-primitive
    /// choice must use `Conditional`, never this.
    PostQuantum,
    /// Known classically-secure-only under a quantum adversary (e.g. EdDSA/Groth16).
    ClassicalOnly,
    /// The posture is route/instantiation-dependent and UNRESOLVED for this backend;
    /// carries the dependency so the conditional claim stays machine-readable
    /// without collapsing to an unconditional one.
    Conditional { depends_on: &'static str },
    /// The component is not present in this backend (e.g. no attestation layer).
    NotApplicable,
}

pub struct PqPosture {
    /// The secret-sharing core. Shamir/additive over F_p is information-theoretic ⇒ PostQuantum.
    pub secret_sharing_core: PqStatus,
    /// The integrity/authentication mechanism. IT-MAC over F_p / RS redundancy ⇒ PostQuantum.
    pub integrity: PqStatus,
    /// The masking CSPRNG. ChaCha20 ⇒ PostQuantum (symmetric).
    pub masking: PqStatus,
    /// The OFFLINE preprocessing crypto. MASCOT OT-extension ⇒
    /// Conditional { depends_on: "base-OT instantiation" }; Overdrive BGV-AHE ⇒
    /// Conditional { depends_on: "BGV parameters + ZK proofs of plaintext knowledge" }.
    /// PostQuantum here is reserved for a backend whose complete concrete
    /// preprocessing stack (base OTs / AHE params / proof components included) has an
    /// established PQ posture. No-offline backend ⇒ NotApplicable. The in-process
    /// simulation implements NO wire cryptography (a local dealer stands in for the
    /// offline protocol) ⇒ it must report
    /// Conditional { depends_on: "unimplemented wire preprocessing (in-process dealer \
    /// simulation — posture undetermined until a real MASCOT/Overdrive offline exists)" },
    /// never PostQuantum.
    pub preprocessing: PqStatus,
    /// The attestation / collaborative-proof layer. EdDSA/BBS+/Pedersen/Groth16 are
    /// pre-quantum ⇒ ClassicalOnly (no PQ collaborative-zkSNARK exists today).
    pub attestation: PqStatus,
}
```

Honest headline this field encodes for a DM backend: **PQ confidentiality + integrity in the MPC
core; route-dependent preprocessing reported as `Conditional` with its dependency named;
classical attestation** — never a single "PQ: yes/no" that would lie about one of the five
components, and never an unconditional `PostQuantum` standing in for an unresolved conditional
posture.

### 3.3 `trusted_setup: TrustedSetup` — **per component**

"Trusted-setup-free" is true for the SS core (a genuine trust-minimality advantage) and
false/dependent for other layers (capability-matrix §6). The MASCOT-vs-Overdrive fork (§1.4) is
exactly a `trusted_setup` difference, so report per component:

```rust
/// Setup requirement of ONE component. DESIGN SKETCH — not built.
pub enum SetupRequirement {
    /// No setup — information-theoretic / transparent (the SS core; a STARK/UltraHonk proof).
    None,
    /// Only base OTs — MASCOT. Not a CRS and no trusted party; a one-time cryptographic
    /// bootstrap (typically a public-key-style OT — PQ posture Conditional on that
    /// instantiation) from which OT extension proceeds symmetric-key.
    BaseOtBootstrap,
    /// A distributed / threshold key must be generated (Overdrive's BGV key) — no single
    /// trusted party, but a setup ceremony among the parties.
    DistributedKeyGen,
    /// A per-circuit common reference string from a trusted ceremony (a Groth16-style
    /// coSNARK). The heaviest; avoided by transparent proof systems.
    TrustedCrs,
}

pub struct TrustedSetup {
    pub secret_sharing_core: SetupRequirement,   // None (info-theoretic)
    pub preprocessing: SetupRequirement,         // MASCOT: BaseOtBootstrap | Overdrive: DistributedKeyGen
    pub attestation: SetupRequirement,           // transparent proof: None | Groth16 coSNARK: TrustedCrs
}
```

This makes the §1.4 recommendation legible in the type: MASCOT ⇒ `preprocessing:
BaseOtBootstrap` (cleaner), Overdrive ⇒ `preprocessing: DistributedKeyGen` (a setup ceremony), and
the SS core is `None` for both — the honest "trust-minimal core, setup at the boundaries" claim.

### 3.4 Back-compat + fail-closed discipline for the new fields

- **Additive, defaulted.** `BackendInfo::new(...)` fills all three with the shipped Shamir posture
  (`requires_preprocessing = None`, `pq_posture` all-PQ-core, `trusted_setup` all-`None`/`NotApplicable`),
  so no existing caller changes. A DM backend sets them via the builder.
- **These fields are DESCRIPTIVE, not gating (today).** `SecurityRequirement`/`satisfies`
  (`backend.rs:987–1052`) match on the three *security* axes; the new fields are for **budgeting and
  disclosure**, not selection — a federation reads `requires_preprocessing` to decide whether it can
  afford the offline tax, but the fail-closed refusal (§4) already happens on the corruption-threshold
  axis. (A future `SecurityRequirement::max_trusted_setup` / `require_pq` could gate on them; that is
  a *separate* bead, explicitly not this spike — see follow-ups.)

---

## 4. Fit to the fail-closed selection API — and why it stays REFUSED

The selection registry (sq-a6p1) already does the right thing, and this design **does not weaken
it**:

- **Today, a DM-malicious request is refused fail-closed.**
  `SecurityRequirement::dishonest_majority_malicious()` → `BackendRegistry::select` →
  `MpcError::NoBackendSatisfies` because no backend with `CorruptionThreshold::DishonestMajority`
  is registered (`select_fails_closed_on_dishonest_majority_malicious_over_shipped_set`,
  `backend.rs:1674–1702`). **This spike changes nothing about that** — it adds no backend.
- **If a DM backend were built**, it would report
  `SecurityDescriptor { adversary: Malicious, output_guarantee: Abort(Unanimous),
  threshold: DishonestMajority{t}, public_verifiability: false }` and `select` would match it for a
  *malicious / DM / unanimous-abort* requirement. It would **still be refused** for the named
  `dishonest_majority_malicious()` requirement (which demands `Abort(Identifiable)` +
  `require_cheater_attribution`) — because plain SPDZ gives unanimous, not identifiable, abort
  (§1.3). This is the honest, self-consistent behaviour: the registry judges the *actual* guarantee,
  not the marketing tier. Only a SPDZ-with-pairwise-MAC-IA backend would satisfy the full named ask
  (the `dishonest_malicious_ia_pvc` fixture, `backend.rs:1539–1546`, is exactly that hypothetical).
- **A new descriptor constructor** (`SecurityDescriptor::dishonest_majority_abort(n, t)`) would build
  the `Malicious + Abort(Unanimous) + DishonestMajority{t}` descriptor. Note it must **not** route
  through `shamir_degree_recon` (`backend.rs:395`), which *fails closed to semi-honest under a
  dishonest majority* — that helper is honest-majority-specific by construction (`backend.rs:411–413`).
  A genuine DM backend builds its descriptor directly, exactly as sq-km34 §4 already anticipated
  ("A genuine dishonest-majority active backend … is a *different* construction and would build its
  descriptor directly, not via this helper", `backend.rs:407–410`).

---

## 5. The honest verdict (the load-bearing conclusion)

**This backend stays research-only for the SPARQL query pipeline, and the selection API truthfully
refuses it today.** Precisely:

1. **Zero published dishonest-majority-malicious instances for any SPARQL/graph-query-eval
   operator.** SPDZ/MASCOT/Overdrive are textbook for *generic arithmetic circuits*, but the
   published relational/graph MPC systems that reach malicious security do so at **honest majority**
   (Senate USENIX'21 malicious n-party is honest-majority; ORQ's malicious path is 4PC Fantastic
   Four honest-majority; Falcon/ABY3 are honest-majority 3PC). There is **no published system** that
   delivers dishonest-majority-malicious correctness for the equality-join / aggregate / comparison /
   oblivious-shuffle operators a federated-SPARQL pipeline needs (capability-matrix §4.4, §6.3;
   security-models §primitive-table; zkp-federated-sparql-design §"NOT ACHIEVED IN THE LITERATURE —
   deliberately scoped out"). Building it would be **novel research**, not routine engineering — and
   its soundness would require a fresh external audit (the same discipline as the ZK verifier gate
   `sq-qhy4`, applied to the collaborative/DM path). **Nothing here may be labelled "sound":** it is
   an unbuilt, unaudited paper design.
2. **Truthfully REFUSED by the selection API.** The fail-closed registry (sq-a6p1) refuses the
   DM-malicious request rather than silently downgrading onto the semi-honest Shamir backend — the
   load-bearing honesty property, already tested (`backend.rs:1674–1702`). This spike **preserves**
   that: it adds no backend, so the refusal stands.
3. **The trait + fields are ready; the crypto is not.** The `MpcBackend` seam absorbs the additive
   authenticated share behind `type Share` with unchanged method signatures (`backend.rs:47–62`), and
   the three new `BackendInfo` fields (§3) let a federation *budget and disclose* the offline tax and
   PQ/setup posture. So the *architecture* is honestly ready for a DM backend; the *implementation and
   external audit* are the gate, and they are out of scope for this spike.
4. **Even built, it is abort-only and offline-dominated.** Cleve caps it at abort (a single cheater
   forces an abort); the offline phase dominates cost and is usually excluded from headline numbers;
   and in the in-process sim it would be a cost-and-shape *model*, not deployable wire crypto. A
   federation must understand all three before choosing it over honest-majority Shamir.

**One-paragraph recommendation.** Keep `TrustModel::DishonestMajority` and the DM registry cells as a
**truthfully-refused capability**, not a shipped one. When (if) the four-flatmates use case is shown
to genuinely need dishonest-majority *among holders* (open-Q 1, capability-matrix §9 — currently
*unresolved*, and the honest-majority-among-cooperating-holders model may well suffice), build the DM
backend as **MASCOT-OT triples + additive authenticated shares + batched MAC-check** behind the
unchanged trait, report it via `requires_preprocessing = Some(MascotOt, …)` / a per-component
`pq_posture` / `trusted_setup = {core: None, preprocessing: BaseOtBootstrap}`, wire a
`SecurityDescriptor::dishonest_majority_abort` constructor (built directly, not via the
honest-majority helper), and gate the whole thing on a fresh external soundness audit before any
"malicious" claim ships. Until then, this record is the design of record and the registry keeps
saying **no**.

---

## 6. Follow-up work → beads (dependency-ordered; NOT implemented here)

This is a spike; it produces a design and a field spec, not code. The concrete follow-ons (all
already beaded per build-out-delta §M-F, so **no new beads created by this record** — recorded for
traceability):

1. **sq-4i39 — implement the `requires_preprocessing` field** (§3.1) as an additive, defaulted
   `BackendInfo` field with the Shamir backend truthfully reporting `None`. The smallest, most
   independent slice; unblocks honest offline-cost budgeting even before any DM backend exists.
2. **pq_posture + trusted_setup fields** (§3.2/§3.3) — sibling additive fields; today the Shamir
   backend reports the honest all-PQ-core / no-trusted-setup posture. *(If not yet beaded under
   sq-4i39's umbrella, file as a sibling — flagged as an out-of-scope follow-up below.)*
3. **sq-ox16 — covert / PVC tier** — the genuine middle between semi-honest and DM-malicious;
   composes with `AdversaryModel::Covert` (already in the type system, `backend.rs:130`).
4. **sq-38zk — WAN constant-round family** — round-per-depth Shamir/SPDZ is WAN-wrong; a
   constant-round comparison / BMR Boolean backend is the WAN answer (a *different* family).
5. **The DM backend itself** — MASCOT-OT triples + additive authenticated shares + MAC-check behind
   the trait — remains **DEFERRED research**, gated on open-Q 1 (does the use case need DM among
   holders?) and on a fresh external soundness audit. No bead promotes it to build without that
   resolution; this spike deliberately does not.

*(Out-of-scope discovery for the worker's follow-up channel: if the `pq_posture`/`trusted_setup`
fields are not covered by an existing bead, they want their own; and a future
`SecurityRequirement::require_pq` / `max_trusted_setup` gating axis is a separate, un-beaded
enhancement. Neither is implemented here.)*

---

## Sources

SPDZ — Damgård–Pastro–Smart–Zakarias (CRYPTO'12); **MASCOT** OT-triples (Keller–Orsini–Scholl,
eprint 2016/505); **Overdrive / LowGear / HighGear** SHE-triples (Keller–Pastro–Rotaru, eprint
2017/1230); SPDZ2k (Cramer–Damgård–Escudero–Scholl–Xing, CRYPTO'18); identifiable-abort at dishonest
majority — Baum et al. (CRYPTO'20, eprint 2020/767), pairwise-MAC IA (eprint 2023/1548); Cleve
(STOC'86); Goyal–Song "malicious security comes free in honest majority" (eprint 2020/134) [the
*contrast* case — no free lunch at DM]; Senate (USENIX'21); ORQ / Fantastic Four (oblivious
relational, honest-majority malicious); S&S 2022 MPC survey (offline-dominates / usually-excluded-
from-headline-numbers — https://sands.edpsciences.org/articles/sands/full_html/2022/01/sands20210001/sands20210001.html,
IACR 2022/417); Ozdemir–Boneh (collaborative-zkSNARK preprocessing lesson); MP-SPDZ framework
(the standard DM benchmarking harness). **In-repo ground truth (`origin/main`):**
`crates/sparq-mpc/src/{backend,shamir,join,partial,field,robust,metrics,transport}.rs`;
`research/{mpc-malicious-security-design,mpc-sparql-capability-matrix,mpc-security-models-and-benchmarks,
mpc-zkp-build-out-delta,mpc-zkp-research-and-architecture,mpc-zkp-federated-sparql-design,
mpc-distributed-randomness-design}.md`. Beads: **sq-j5ok** (this spike), sq-mq8q (CLOSED, 3-axis
descriptor), sq-a6p1 (CLOSED, fail-closed registry), sq-km34 (honest-majority IT-MAC, `.1` CLOSED),
sq-4i39 (`requires_preprocessing` field), sq-38zk (WAN constant-round), sq-ox16 (covert/PVC),
sq-yyro (PRSS/dealer-less VSS), sq-qhy4 (external ZK audit, P0 — the soundness-gate discipline this
DM/collaborative path inherits).
