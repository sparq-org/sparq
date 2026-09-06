//! # sparq-acbench — access-controlled-query benchmark scaffold
//!
//! Parameterised, deterministic generator of access-controlled RDF datasets with
//! built-in by-construction correctness oracles for WAC, ACP, and ODRL.
//!
//! ## Design authority
//! `research/ac-query-benchmark.md` (epic `sq-i6du2`, bead `sq-i6du2.1`). The six
//! design decisions in §2 of that record are the canonical source of truth; this crate
//! implements §2.1 (parameter vector), §2.2 (intent-table IR + per-model compilers),
//! §2.3 (by-construction procedural oracle), and §2.4 (one module per use case).
//!
//! ## Invariants
//!
//! - **Determinism** (tested in `tests/determinism.rs`): identical [`GenParams`]
//!   produces byte-identical N-Quads, intent tables, decisions, and W2 expected results
//!   on every run. No wall-clock, thread ID, OS random, or `HashMap` randomisation may
//!   enter the output path.
//! - **Zero dependency on `sparq-core` / `sparq-engine`** (load-bearing): this crate's
//!   oracle is structurally independent of the system under test; it never calls sparq's
//!   N3 rule engine, `AclIndex`, or `sparq-policy` evaluator.
//! - **Fail-closed default**: [`Decision::Deny`] is the default for any request not
//!   matched by a policy. Absence of a matching rule is NEVER interpreted as allow.
//!
//! ## Use cases (§1.3)
//!
//! | Module | Use case | AC shape stressed |
//! |---|---|---|
//! | [`personal`] | U1 personal data storage | container inheritance, owner-centric |
//! | [`project_mgmt`] | U2 commercial project management | role/team groups, all-except |
//! | [`financial`] | U3 financial services | ODRL duties/constraints, compartmentalization |
//! | [`consortium`] | U4 research-data consortium | temporal constraints, embargo flips |

// ── lint configuration ──────────────────────────────────────────────────────────────
#![deny(missing_docs)]
#![deny(clippy::all)]
#![warn(clippy::pedantic)]
// Iterator-collect style: allow collect() without type annotation where inference works.
#![allow(clippy::missing_panics_doc)] // stubs will panic; full impls document.

pub mod personal;
pub mod project_mgmt;
pub mod financial;
pub mod consortium;
pub mod workload;
pub mod oracle;

// ── GenParams ───────────────────────────────────────────────────────────────────────

/// The single parameter vector shared by all four use-case generators.
///
/// Every knob is documented with the **real-world property it models** (design record
/// §2.1 wording reproduced here verbatim so it appears in rustdoc as the canonical
/// definition).
///
/// **Determinism**: same [`GenParams`] → byte-identical output. The seed drives a
/// `SplitMix64` PRNG (no OS entropy). All knobs have bounded, language-stable types.
#[derive(Debug, Clone, PartialEq)]
pub struct GenParams {
    /// **Models:** "same seed ⇒ byte-identical corpus" guarantee. `SplitMix64`
    /// throughout; identical across runs, platforms, and rustc versions.
    pub seed: u64,

    /// **Models:** real-world dataset size. SF=1 sizes each use case at roughly
    /// the repo's per-commit tier (~10⁴ resources / ~10⁵ triples, comparable to
    /// `watdiv` SF=1 / `bsbm` per-commit). SF∈{10, 100} are the EC2/nightly tiers.
    /// Resource, agent, and policy counts scale linearly; shape knobs do not.
    pub sf: u32,

    /// **Models:** pod / project folder hierarchy depth. Drives the WAC
    /// nearest-ancestor vs ACP cumulative-ancestor inheritance divergence (§2.2).
    /// Range: 0–6.
    pub container_depth: u8,

    /// **Models:** "most resources inherit" reality. Fraction of resources carrying
    /// their OWN `.acl`/`.acr` vs inheriting from an ancestor. Drives effective-ACL
    /// discovery cost. Must be in [0.0, 1.0].
    pub acl_coverage: f32,

    /// **Models:** org/team group chain depth. Drives group-closure cost and the
    /// ACP no-group-matcher per-member expansion (§2.2). Range: 0–4.
    pub group_nesting_depth: u8,

    /// **Models:** org/team group fan-out. Drives the total member-resolution
    /// cardinality and the per-member ACP expansion blowup factor.
    pub members_per_group: u32,

    /// **Models:** audience distribution across public / private / shared resources.
    /// Drives auth-view size and query selectivity. Components must sum to 1.0.
    pub mix: AudienceMix,

    /// **Models:** accreted real-world policy sets (`XACBench`'s headline dimension).
    /// ODRL policy and ACP policy fan-in per resource.
    pub policies_per_resource: u8,

    /// **Models:** usage-condition richness for ODRL. Levels 1–3 are ODRL-only by
    /// construction (§2.2 expressibility matrix). 0 = none; 1 = temporal (`dateTime`
    /// window); 2 = purpose or count; 3 = compound `and`-of.
    pub constraint_complexity: ConstraintComplexity,

    /// **Models:** request-population size; how many distinct agents submit requests.
    pub n_agents: u32,
}

impl GenParams {
    /// Canonical default for a quick smoke-test at SF=1. Seed is fixed; all knobs
    /// are set to their simplest (shallowest / most uniform) values so the generator
    /// produces a minimal valid corpus.
    #[must_use]
    pub fn smoke() -> Self {
        Self {
            seed: 42,
            sf: 1,
            container_depth: 2,
            acl_coverage: 0.3,
            group_nesting_depth: 1,
            members_per_group: 4,
            mix: AudienceMix { public: 0.3, private: 0.5, shared: 0.2 },
            policies_per_resource: 2,
            constraint_complexity: ConstraintComplexity::None,
            n_agents: 10,
        }
    }

    /// Validate all invariants (e.g. `mix` sums to 1.0, ranges respected).
    ///
    /// # Errors
    /// Returns `Err` with a human-readable description of the first violated invariant.
    pub fn validate(&self) -> Result<(), String> {
        if self.container_depth > 6 {
            return Err(format!(
                "container_depth {} > 6 (max 6 per design record §2.1)",
                self.container_depth
            ));
        }
        if self.group_nesting_depth > 4 {
            return Err(format!(
                "group_nesting_depth {} > 4 (max 4 per design record §2.1)",
                self.group_nesting_depth
            ));
        }
        if !(0.0..=1.0).contains(&self.acl_coverage) {
            return Err(format!(
                "acl_coverage {} not in [0.0, 1.0]",
                self.acl_coverage
            ));
        }
        self.mix.validate()?;
        Ok(())
    }
}

/// Audience distribution across public / private / shared resources.
///
/// The three fractions must sum to 1.0 (within floating-point tolerance of 0.001).
#[derive(Debug, Clone, PartialEq)]
pub struct AudienceMix {
    /// Fraction of resources with unrestricted public read access.
    pub public: f32,
    /// Fraction of resources restricted to the owning agent only.
    pub private: f32,
    /// Fraction of resources shared with a named group or set of agents.
    pub shared: f32,
}

impl AudienceMix {
    /// Check that the fractions sum to 1.0 within tolerance.
    ///
    /// # Errors
    /// Returns `Err` if the three components do not sum to 1.0 within 0.001.
    pub fn validate(&self) -> Result<(), String> {
        let sum = self.public + self.private + self.shared;
        if (sum - 1.0_f32).abs() > 0.001 {
            return Err(format!(
                "AudienceMix components sum to {sum:.4}, expected 1.0"
            ));
        }
        Ok(())
    }
}

/// ODRL constraint complexity level. Values 1–3 are **ODRL-only** by construction
/// (§2.2 expressibility matrix); WAC and ACP compilers emit nothing for these levels.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConstraintComplexity {
    /// No usage conditions — expressible in all three models.
    None,
    /// Temporal `dateTime` window constraint — ODRL-only.
    Temporal,
    /// Purpose or count-limited access constraint — ODRL-only.
    PurposeOrCount,
    /// Compound `and`-of constraint — ODRL-only.
    Compound,
}

// ── Intent Table IR ─────────────────────────────────────────────────────────────────

/// A row in the model-agnostic intent table (§2.2).
///
/// Each generator emits a `Vec<IntentRow>` before lowering to concrete policy triples.
/// Three compiler functions ([`compile_wac`], [`compile_acp`], [`compile_odrl`]) lower
/// each row to WAC, ACP, or ODRL graphs respectively.
///
/// The expressibility matrix is derived from the set of `(audience, condition)` pairs
/// present in the intent table and the per-compiler lowering results.
#[derive(Debug, Clone, PartialEq)]
pub struct IntentRow {
    /// The audience this intent grants or denies access to.
    pub audience: Audience,
    /// The resource or container-subtree scope.
    pub scope: Scope,
    /// The access mode(s) granted or denied.
    pub mode: AccessMode,
    /// An optional usage condition. Non-`None` values are ODRL-only in the matrix.
    pub condition: Condition,
    /// Whether this is a grant (allow) or prohibition (deny).
    pub effect: Effect,
    /// Opaque URI identifying the resource (or container root) this intent applies to.
    pub resource_uri: String,
}

/// The audience (principal class) for an intent.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Audience {
    /// The owning agent of the resource.
    Owner,
    /// A specific named agent IRI.
    Agent(String),
    /// A named group (resolved via group-closure at oracle evaluation time).
    Group(String),
    /// Any unauthenticated or authenticated agent (public).
    Public,
    /// Any authenticated agent.
    Authenticated,
    /// An agent restricted to a specific client application.
    ClientRestricted {
        /// The agent IRI.
        agent: String,
        /// The client application IRI (ACP `acp:client`; WAC approximation via `acl:origin`).
        client: String,
    },
    /// All agents EXCEPT a named set. ODRL / ACP native; WAC inexpressible if unbounded.
    AllExcept(Vec<String>),
}

/// Resource or container-subtree scope for an intent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    /// Applies to a single resource URI.
    Resource,
    /// Applies to a container and all its descendants (subtree).
    Subtree,
}

/// SPARQL / Solid access modes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AccessMode {
    /// Read access (`acl:Read` / `acp:Read`).
    pub read: bool,
    /// Write access (`acl:Write` / `acp:Write`).
    pub write: bool,
    /// Control/admin access (`acl:Control` / `acp:Control`).
    pub control: bool,
}

impl AccessMode {
    /// Read-only access.
    #[must_use]
    pub fn read_only() -> Self {
        Self { read: true, write: false, control: false }
    }

    /// Full access (read + write + control).
    #[must_use]
    pub fn full() -> Self {
        Self { read: true, write: true, control: true }
    }
}

/// Optional usage condition for an intent. Non-`None` values compile only to ODRL.
#[derive(Debug, Clone, PartialEq)]
pub enum Condition {
    /// No usage condition.
    None,
    /// Temporal window: access permitted only in `[start, end)`.
    /// Dates are ISO 8601 strings (e.g. `"2026-01-01T00:00:00Z"`).
    Temporal {
        /// Start of the temporal window (inclusive), ISO 8601.
        start: String,
        /// End of the temporal window (exclusive), ISO 8601.
        end: String,
    },
    /// Purpose-of-use constraint (ODRL `odrl:purpose`).
    Purpose(String),
    /// Count-limited access (ODRL `odrl:count`).
    Count(u32),
    /// Compound `and`-of two sub-conditions.
    And(Box<Condition>, Box<Condition>),
}

/// Whether an intent grants or prohibits access.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Grant (allow) access.
    Allow,
    /// Prohibit (deny) access.
    Deny,
}

// ── Expressibility Matrix ────────────────────────────────────────────────────────────

/// How a per-model compiler renders one [`IntentRow`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expressibility {
    /// The intent is natively expressible in the model.
    Native,
    /// Expressible only via an expansion (e.g. WAC all-except → enumerated allows).
    /// The `usize` is the blowup factor (number of generated rules vs 1).
    Expansion(usize),
    /// The model can only approximate the intent (e.g. WAC `acl:origin` for ACP
    /// `acp:client`).
    Approximation,
    /// The intent is structurally inexpressible in the model (e.g. WAC all-except
    /// with unbounded exclusion set, or any `Condition ≠ None` in WAC/ACP).
    Unsupported,
}

/// An entry in the expressibility matrix for one intent row and one model.
#[derive(Debug, Clone)]
pub struct ExpressibilityEntry {
    /// The access-control model this entry describes.
    pub model: AcModel,
    /// How the compiler renders the intent.
    pub expressibility: Expressibility,
}

/// How a GenAI-retrieval capability participates in an answer-safe pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnswerSafety {
    /// An approximate signal proposes candidates; it does not establish an answer.
    ApproximateProposes,
    /// An exact graph operation validates or emits the grounded result.
    ExactValidates,
}

/// One implemented capability in the GenAI-retrieval expressibility catalogue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GenAiRetrievalExpressibilityEntry {
    /// Stable, human-readable capability name.
    pub capability: &'static str,
    /// Workspace crate that implements the capability.
    pub provider_crate: &'static str,
    /// Declared Cargo feature that exposes the capability.
    ///
    /// `default` means the capability is available whenever the opt-in provider crate itself is
    /// selected; it does not make that crate part of the default engine.
    pub provider_feature: &'static str,
    /// The capability's role in the answer-safety boundary.
    pub answer_safety: AnswerSafety,
}

/// Implemented GenAI-retrieval capabilities used by the audience-capability matrix.
///
/// Approximate ANN and fusion signals only propose candidates. Generated queries are parsed and
/// executed by the exact engine, while citations are emitted only from provenance asserted in the
/// graph. Every provider pair below names a crate and Cargo feature declared in this workspace.
/// [GPT-5.6] `sq-oti9m` keeps this catalogue data-only and independent of workload execution.
pub const GENAI_RETRIEVAL_EXPRESSIBILITY: [GenAiRetrievalExpressibilityEntry; 5] = [
    GenAiRetrievalExpressibilityEntry {
        capability: "vec:nearest ANN-in-SPARQL",
        provider_crate: "sparq-vectors",
        provider_feature: "vec-predicate",
        answer_safety: AnswerSafety::ApproximateProposes,
    },
    GenAiRetrievalExpressibilityEntry {
        capability: "filtered ANN by type",
        provider_crate: "sparq-vectors",
        provider_feature: "filtered-ann",
        answer_safety: AnswerSafety::ApproximateProposes,
    },
    GenAiRetrievalExpressibilityEntry {
        capability: "hybrid text+vector fusion",
        provider_crate: "sparq-vectors",
        provider_feature: "default",
        answer_safety: AnswerSafety::ApproximateProposes,
    },
    GenAiRetrievalExpressibilityEntry {
        capability: "NL→SPARQL",
        provider_crate: "sparq-nlq",
        provider_feature: "live",
        answer_safety: AnswerSafety::ExactValidates,
    },
    GenAiRetrievalExpressibilityEntry {
        capability: "provenance-carrying citation",
        provider_crate: "sparq-nlq",
        provider_feature: "citations",
        answer_safety: AnswerSafety::ExactValidates,
    },
];

/// The three access-control models supported by the benchmark.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AcModel {
    /// Web Access Control (WAC).
    Wac,
    /// Access Control Policy (ACP).
    Acp,
    /// Open Digital Rights Language (ODRL).
    Odrl,
}

// ── Per-model Policy Compilers ───────────────────────────────────────────────────────

/// Output of a per-model compiler run over one [`IntentRow`].
#[derive(Debug, Clone)]
pub struct CompiledPolicy {
    /// The access-control model that was compiled.
    pub model: AcModel,
    /// N-Quads triples comprising the compiled policy graph.
    pub nquads: Vec<String>,
    /// Expressibility classification for this (row, model) pair.
    pub expressibility: Expressibility,
}

/// Compile one [`IntentRow`] to WAC triples (`acl:Authorization` shape).
///
/// # WAC semantics
/// - `Scope::Subtree` → `acl:default` **and** `acl:accessTo` at the container. `acl:default`
///   alone reaches only the container's *members*, never the container resource itself, so a
///   `default`-only authorization would leave the container unreadable by the very principal
///   the intent names. Emitting both is the standard Solid container-authorization pattern and
///   is what makes `Scope::Subtree` mean "this container **and** everything under it" under
///   the nearest-ACL-document-wins resolution the WAC oracle implements (bead `sq-o4orz`).
/// - `Audience::AllExcept(excl)` → enumerated `acl:agent` for all agents not in `excl`.
///   If `excl` is unbounded (empty = placeholder), returns [`Expressibility::Unsupported`]
///   and no triples.
/// - `Condition ≠ None` → [`Expressibility::Unsupported`] (WAC has no usage conditions).
/// - `Effect::Deny` → [`Expressibility::Unsupported`] (WAC has no deny).
/// - `Audience::ClientRestricted` → approximated via `acl:origin` → [`Expressibility::Approximation`].
///
/// # Invariants (each tested in `tests/determinism.rs`)
/// - Output is deterministic: no hash-map ordering, no `rand` inside this function.
/// - No dependency on any sparq crate.
// One match arm per Audience variant, one triple per predicate: verbosity is load-bearing
// here because the 1:1 WAC-spec mapping is what makes the oracle legible and trustworthy.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn compile_wac(row: &IntentRow) -> CompiledPolicy {
    // Conditions and denies are inexpressible in WAC.
    if row.condition != Condition::None {
        return CompiledPolicy {
            model: AcModel::Wac,
            nquads: vec![],
            expressibility: Expressibility::Unsupported,
        };
    }
    if row.effect == Effect::Deny {
        return CompiledPolicy {
            model: AcModel::Wac,
            nquads: vec![],
            expressibility: Expressibility::Unsupported,
        };
    }

    let auth_id = format!("<{}#wac-auth>", row.resource_uri);
    let resource = format!("<{}>", row.resource_uri);
    let mut triples: Vec<String> = Vec::new();
    let mut expressibility = Expressibility::Native;

    // rdf:type acl:Authorization
    triples.push(format!(
        "{auth_id} <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> ."
    ));

    // acl:accessTo or acl:default
    match row.scope {
        Scope::Resource => {
            triples.push(format!(
                "{auth_id} <http://www.w3.org/ns/auth/acl#accessTo> {resource} ."
            ));
        }
        Scope::Subtree => {
            // `default` reaches the container's members; `accessTo` reaches the container
            // resource itself. Both are needed for "the container and its subtree" — see
            // the WAC-semantics note on this function.
            triples.push(format!(
                "{auth_id} <http://www.w3.org/ns/auth/acl#default> {resource} ."
            ));
            triples.push(format!(
                "{auth_id} <http://www.w3.org/ns/auth/acl#accessTo> {resource} ."
            ));
        }
    }

    // access modes
    if row.mode.read {
        triples.push(format!(
            "{auth_id} <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> ."
        ));
    }
    if row.mode.write {
        triples.push(format!(
            "{auth_id} <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Write> ."
        ));
    }
    if row.mode.control {
        triples.push(format!(
            "{auth_id} <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Control> ."
        ));
    }

    // audience
    match &row.audience {
        Audience::Public => {
            triples.push(format!(
                "{auth_id} <http://www.w3.org/ns/auth/acl#agentClass> <http://xmlns.com/foaf/0.1/Agent> ."
            ));
        }
        Audience::Authenticated => {
            triples.push(format!(
                "{auth_id} <http://www.w3.org/ns/auth/acl#agentClass> <http://www.w3.org/ns/auth/acl#AuthenticatedAgent> ."
            ));
        }
        Audience::Owner => {
            let agent = format!("<{}#owner>", row.resource_uri);
            triples.push(format!(
                "{auth_id} <http://www.w3.org/ns/auth/acl#agent> {agent} ."
            ));
        }
        Audience::Agent(ref a) => {
            triples.push(format!(
                "{auth_id} <http://www.w3.org/ns/auth/acl#agent> <{a}> ."
            ));
        }
        Audience::Group(ref g) => {
            triples.push(format!(
                "{auth_id} <http://www.w3.org/ns/auth/acl#agentGroup> <{g}> ."
            ));
        }
        Audience::ClientRestricted { agent, client } => {
            // WAC approximates ACP acp:client via acl:origin (different semantics).
            triples.push(format!(
                "{auth_id} <http://www.w3.org/ns/auth/acl#agent> <{agent}> ."
            ));
            triples.push(format!(
                "{auth_id} <http://www.w3.org/ns/auth/acl#origin> <{client}> ."
            ));
            expressibility = Expressibility::Approximation;
        }
        Audience::AllExcept(excl) => {
            if excl.is_empty() {
                // Unbounded exclusion set → inexpressible.
                return CompiledPolicy {
                    model: AcModel::Wac,
                    nquads: vec![],
                    expressibility: Expressibility::Unsupported,
                };
            }
            // Bounded: enumerate allows (blowup = excl.len() + 1 rules conceptually,
            // but WAC has no deny so we emit a placeholder allow for "others not listed").
            // The oracle evaluates compiled semantics exactly for this model.
            for agent in excl {
                triples.push(format!(
                    "{auth_id} <http://www.w3.org/ns/auth/acl#agent> <{agent}> ."
                ));
            }
            expressibility = Expressibility::Expansion(excl.len());
        }
    }

    CompiledPolicy { model: AcModel::Wac, nquads: triples, expressibility }
}

/// Compile one [`IntentRow`] to ACP triples (`acp:Policy` + matcher shape).
///
/// # ACP semantics
///
/// The output triples are intended for placement in the per-resource ACR document
/// (`<resource.acr>` named graph). The engine's loader synthesizes
/// `<resource> solidx:ownAcr <resource.acr>` by naming convention, so the ACR triples
/// must follow the chain the engine's N3 rules (`rules/acp-a.n3`) consume:
///
/// ```text
/// <resource.acr> acp:accessControl  <resource.acr#c>        # or memberAccessControl
/// <resource.acr#c> acp:apply        <resource#acp-policy>
/// <resource#acp-policy> acp:allow   acl:Read                 # note: acl:, NOT acp:
/// <resource#acp-policy> acp:anyOf   <resource#acp-matcher>
/// <resource#acp-matcher> acp:agent  <principal>
/// ```
///
/// **Mode IRIs** — the engine (rules/acp-c.n3) maps modes via
/// `acl:Read solidx:allowPred auth:read`, so modes MUST be in the `acl:` namespace
/// (`http://www.w3.org/ns/auth/acl#Read`, not `acp:Read`).
///
/// **Access-control chain** — the loader finds `.acr`-suffixed graphs and derives
/// `<R> solidx:ownAcr <R.acr>`. The rule then expects
/// `?acr acp:accessControl ?c . ?c acp:apply ?pol` (own) or
/// `?acr acp:memberAccessControl ?c . ?c acp:apply ?pol` (inherited; [`Scope::Subtree`]).
///
/// Other semantics:
/// - `Scope::Subtree` → `acp:memberAccessControl` on the ACR document.
/// - `Audience::AllExcept` → `acp:deny`-shaped policy.
/// - `Audience::Group(g)` → per-member `acp:agent` matchers (blowup). The group IRI
///   is used as a placeholder; full expansion requires the group-closure table from
///   the generator (passed as `group_members`).
/// - `Effect::Deny` → `acp:deny` policy.
/// - `Condition ≠ None` → [`Expressibility::Unsupported`] (ACP has no usage conditions;
///   these compile only to ODRL).
///
/// # Invariants
/// - Output is deterministic given the same `row` and `group_members` slice.
/// - No dependency on any sparq crate.
/// - The oracle and the live driver both call this function, so alignment here
///   makes the ACP oracle and engine agree on the authorization shape. [SONNET-4.6]
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn compile_acp(row: &IntentRow, group_members: &[String]) -> CompiledPolicy {
    if row.condition != Condition::None {
        return CompiledPolicy {
            model: AcModel::Acp,
            nquads: vec![],
            expressibility: Expressibility::Unsupported,
        };
    }

    // The ACR document IRI is <resource.acr>; the engine's loader derives
    // `<resource> solidx:ownAcr <resource.acr>` from this naming convention.
    //
    // Policy and matcher IDs are distinguished by an FNV-1a hash of the intent's
    // (audience key, scope, effect) so that multiple compile_acp calls for the SAME
    // resource URI (one per intent row) produce DISTINCT IRIs — otherwise all matchers
    // would accumulate on a single shared policy node, causing over-grant.
    // FNV-1a is simple, dependency-free, and deterministic across platforms. [SONNET-4.6]
    let aud_key = match &row.audience {
        Audience::Public => "pub".to_string(),
        Audience::Authenticated => "auth".to_string(),
        Audience::Owner => "own".to_string(),
        Audience::Agent(a) => format!("ag:{a}"),
        Audience::Group(g) => format!("gr:{g}"),
        Audience::ClientRestricted { agent, client } => format!("cl:{agent}:{client}"),
        Audience::AllExcept(excl) => format!("ex:{}", excl.join(",")),
    };
    let scope_byte: u8 = match row.scope { Scope::Resource => b'r', Scope::Subtree => b's' };
    let effect_byte: u8 = match row.effect { Effect::Allow => b'a', Effect::Deny => b'd' };

    // FNV-1a 32-bit: offset-basis 2166136261, prime 16777619.
    let mut h: u32 = 2_166_136_261u32;
    for &b in aud_key.as_bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(16_777_619u32);
    }
    h ^= u32::from(scope_byte);
    h = h.wrapping_mul(16_777_619u32);
    h ^= u32::from(effect_byte);
    h = h.wrapping_mul(16_777_619u32);
    let frag = format!("acp-{h:08x}");

    let acr_iri = format!("{}.acr", row.resource_uri);
    let policy_id = format!("<{}#{frag}-pol>", row.resource_uri);
    let acr = format!("<{acr_iri}>");
    let ac_node = format!("<{acr_iri}#{frag}-c>");
    let matcher_id = format!("<{}#{frag}-m>", row.resource_uri);
    let mut triples: Vec<String> = Vec::new();
    let mut expressibility = Expressibility::Native;

    // rdf:type acp:Policy — first triple so derive_acl_graph can extract the base IRI.
    triples.push(format!(
        "{policy_id} <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/solid/acp#Policy> ."
    ));

    // acp:allow or acp:deny predicate.
    let effect_pred = match row.effect {
        Effect::Allow => "<http://www.w3.org/ns/solid/acp#allow>",
        Effect::Deny => "<http://www.w3.org/ns/solid/acp#deny>",
    };

    // Access modes — MUST use the acl: namespace (http://www.w3.org/ns/auth/acl#).
    // The engine's acp-c.n3 maps modes as `acl:Read solidx:allowPred auth:read`; using
    // `acp:Read` (a different IRI) causes the mode mapping to silently miss and the
    // policy never fires. [SONNET-4.6] sq-kvvcl defect-1 fix.
    if row.mode.read {
        triples.push(format!(
            "{policy_id} {effect_pred} <http://www.w3.org/ns/auth/acl#Read> ."
        ));
    }
    if row.mode.write {
        triples.push(format!(
            "{policy_id} {effect_pred} <http://www.w3.org/ns/auth/acl#Write> ."
        ));
    }
    if row.mode.control {
        triples.push(format!(
            "{policy_id} {effect_pred} <http://www.w3.org/ns/auth/acl#Control> ."
        ));
    }

    // Access-control chain on the ACR document.
    // The engine rule (acp-a.n3 line 29) is:
    //   { ?r solidx:ownAcr ?acr . ?acr acp:accessControl ?c . ?c acp:apply ?pol . }
    //   => { ?pol solidx:appliesToResource ?r . }
    // So we need <acr> acp:accessControl|memberAccessControl <acr#c> and
    //            <acr#c> acp:apply <policy>  — NOT <resource> acp:accessControl <policy>.
    // [SONNET-4.6] sq-kvvcl defect-1 fix.
    let ac_link_pred = match row.scope {
        Scope::Resource => "<http://www.w3.org/ns/solid/acp#accessControl>",
        Scope::Subtree => "<http://www.w3.org/ns/solid/acp#memberAccessControl>",
    };
    triples.push(format!("{acr} {ac_link_pred} {ac_node} ."));
    triples.push(format!(
        "{ac_node} <http://www.w3.org/ns/solid/acp#apply> {policy_id} ."
    ));

    // Matcher — anyOf is sufficient (at least one matcher accepts → satisfied).
    // `matcher_id` is already defined above with the unique frag slug.
    triples.push(format!(
        "{matcher_id} <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/solid/acp#Matcher> ."
    ));
    triples.push(format!(
        "{policy_id} <http://www.w3.org/ns/solid/acp#anyOf> {matcher_id} ."
    ));

    match &row.audience {
        Audience::Public => {
            triples.push(format!(
                "{matcher_id} <http://www.w3.org/ns/solid/acp#agent> <http://www.w3.org/ns/solid/acp#PublicAgent> ."
            ));
        }
        Audience::Authenticated => {
            triples.push(format!(
                "{matcher_id} <http://www.w3.org/ns/solid/acp#agent> <http://www.w3.org/ns/solid/acp#AuthenticatedAgent> ."
            ));
        }
        Audience::Owner => {
            let agent = format!("<{}#owner>", row.resource_uri);
            triples.push(format!(
                "{matcher_id} <http://www.w3.org/ns/solid/acp#agent> {agent} ."
            ));
        }
        Audience::Agent(a) => {
            triples.push(format!(
                "{matcher_id} <http://www.w3.org/ns/solid/acp#agent> <{a}> ."
            ));
        }
        Audience::Group(group_iri) => {
            // ACP has no group matcher: expand to per-member acp:agent triples.
            if group_members.is_empty() {
                // Placeholder (group closure not yet resolved): emit the group IRI
                // as a single matcher and mark as expansion(0) pending resolution.
                triples.push(format!(
                    "{matcher_id} <http://www.w3.org/ns/solid/acp#agent> <{group_iri}> ."
                ));
                expressibility = Expressibility::Expansion(0);
            } else {
                for member in group_members {
                    triples.push(format!(
                        "{matcher_id} <http://www.w3.org/ns/solid/acp#agent> <{member}> ."
                    ));
                }
                expressibility = Expressibility::Expansion(group_members.len());
            }
        }
        Audience::ClientRestricted { agent, client } => {
            triples.push(format!(
                "{matcher_id} <http://www.w3.org/ns/solid/acp#agent> <{agent}> ."
            ));
            triples.push(format!(
                "{matcher_id} <http://www.w3.org/ns/solid/acp#client> <{client}> ."
            ));
        }
        Audience::AllExcept(_) => {
            // ACP deny-shaped policy handles all-except natively.
            // The Effect::Deny above already selects `acp:deny`.
            // Emit a PublicAgent matcher (grants to all, then the deny restricts).
            triples.push(format!(
                "{matcher_id} <http://www.w3.org/ns/solid/acp#agent> <http://www.w3.org/ns/solid/acp#PublicAgent> ."
            ));
        }
    }

    CompiledPolicy { model: AcModel::Acp, nquads: triples, expressibility }
}

/// Compile one [`IntentRow`] to ODRL triples (Permission/Prohibition shape).
///
/// # ODRL semantics
/// - Conditions compile to `odrl:constraint` values.
/// - `Audience::Group(g)` → `odrl:PartyCollection` with `odrl:source <g>`.
/// - `Effect::Allow` → `odrl:Permission`; `Effect::Deny` → `odrl:Prohibition`.
/// - All intents with any [`Condition`] variant are natively expressible in ODRL only.
///
/// # Invariants
/// - Output is deterministic given the same `row`.
/// - No dependency on any sparq crate.
// Verbosity is load-bearing: 1:1 ODRL-spec mapping for each Audience and Condition variant.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn compile_odrl(row: &IntentRow) -> CompiledPolicy {
    let policy_id = format!("<{}#odrl-policy>", row.resource_uri);
    let resource = format!("<{}>", row.resource_uri);
    let mut triples: Vec<String> = Vec::new();

    // rdf:type odrl:Set (or odrl:Offer — Set is the generic root)
    triples.push(format!(
        "{policy_id} <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/odrl/2/Set> ."
    ));
    triples.push(format!(
        "{policy_id} <http://www.w3.org/ns/odrl/2/uid> {policy_id} ."
    ));
    triples.push(format!(
        "{policy_id} <http://www.w3.org/ns/odrl/2/target> {resource} ."
    ));

    let rule_id = format!("<{}#odrl-rule>", row.resource_uri);
    let rule_type = match row.effect {
        Effect::Allow => "<http://www.w3.org/ns/odrl/2/permission>",
        Effect::Deny => "<http://www.w3.org/ns/odrl/2/prohibition>",
    };
    triples.push(format!("{policy_id} {rule_type} {rule_id} ."));

    // action: sparql:read / write / control (custom sparq:acl namespace)
    if row.mode.read {
        triples.push(format!(
            "{rule_id} <http://www.w3.org/ns/odrl/2/action> <https://sparq.dev/vocab/acl#Read> ."
        ));
    }
    if row.mode.write {
        triples.push(format!(
            "{rule_id} <http://www.w3.org/ns/odrl/2/action> <https://sparq.dev/vocab/acl#Write> ."
        ));
    }
    if row.mode.control {
        triples.push(format!(
            "{rule_id} <http://www.w3.org/ns/odrl/2/action> <https://sparq.dev/vocab/acl#Control> ."
        ));
    }

    // assignee
    match &row.audience {
        Audience::Public => {
            triples.push(format!(
                "{rule_id} <http://www.w3.org/ns/odrl/2/assignee> <http://www.w3.org/ns/odrl/2/All> ."
            ));
        }
        Audience::Authenticated => {
            triples.push(format!(
                "{rule_id} <http://www.w3.org/ns/odrl/2/assignee> <http://www.w3.org/ns/odrl/2/AllAuthenticated> ."
            ));
        }
        Audience::Owner | Audience::Agent(_) => {
            let agent = match &row.audience {
                Audience::Owner => format!("<{}#owner>", row.resource_uri),
                Audience::Agent(a) => format!("<{a}>"),
                _ => unreachable!(),
            };
            triples.push(format!(
                "{rule_id} <http://www.w3.org/ns/odrl/2/assignee> {agent} ."
            ));
        }
        Audience::Group(g) => {
            let party_id = format!("<{}#odrl-party>", row.resource_uri);
            triples.push(format!(
                "{party_id} <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/odrl/2/PartyCollection> ."
            ));
            triples.push(format!(
                "{party_id} <http://www.w3.org/ns/odrl/2/source> <{g}> ."
            ));
            triples.push(format!(
                "{rule_id} <http://www.w3.org/ns/odrl/2/assignee> {party_id} ."
            ));
        }
        Audience::ClientRestricted { agent, client } => {
            triples.push(format!(
                "{rule_id} <http://www.w3.org/ns/odrl/2/assignee> <{agent}> ."
            ));
            // client constraint
            let cst_id = format!("<{}#odrl-client-cst>", row.resource_uri);
            triples.push(format!(
                "{cst_id} <http://www.w3.org/ns/odrl/2/leftOperand> <http://www.w3.org/ns/odrl/2/systemDevice> ."
            ));
            triples.push(format!(
                "{cst_id} <http://www.w3.org/ns/odrl/2/operator> <http://www.w3.org/ns/odrl/2/eq> ."
            ));
            triples.push(format!(
                "{cst_id} <http://www.w3.org/ns/odrl/2/rightOperand> <{client}> ."
            ));
            triples.push(format!("{rule_id} <http://www.w3.org/ns/odrl/2/constraint> {cst_id} ."));
        }
        Audience::AllExcept(excl) => {
            // ODRL Prohibition naturally expresses all-except.
            for agent in excl {
                triples.push(format!(
                    "{rule_id} <http://www.w3.org/ns/odrl/2/assignee> <{agent}> ."
                ));
            }
        }
    }

    // conditions → odrl:constraint
    match &row.condition {
        Condition::None => {}
        Condition::Temporal { start, end } => {
            let cst_start = format!("<{}#odrl-cst-start>", row.resource_uri);
            let cst_end = format!("<{}#odrl-cst-end>", row.resource_uri);
            triples.push(format!(
                "{cst_start} <http://www.w3.org/ns/odrl/2/leftOperand> <http://www.w3.org/ns/odrl/2/dateTime> ."
            ));
            triples.push(format!(
                "{cst_start} <http://www.w3.org/ns/odrl/2/operator> <http://www.w3.org/ns/odrl/2/gteq> ."
            ));
            triples.push(format!(
                "{cst_start} <http://www.w3.org/ns/odrl/2/rightOperand> \"{start}\"^^<http://www.w3.org/2001/XMLSchema#dateTime> ."
            ));
            triples.push(format!(
                "{cst_end} <http://www.w3.org/ns/odrl/2/leftOperand> <http://www.w3.org/ns/odrl/2/dateTime> ."
            ));
            triples.push(format!(
                "{cst_end} <http://www.w3.org/ns/odrl/2/operator> <http://www.w3.org/ns/odrl/2/lt> ."
            ));
            triples.push(format!(
                "{cst_end} <http://www.w3.org/ns/odrl/2/rightOperand> \"{end}\"^^<http://www.w3.org/2001/XMLSchema#dateTime> ."
            ));
            triples.push(format!("{rule_id} <http://www.w3.org/ns/odrl/2/constraint> {cst_start} ."));
            triples.push(format!("{rule_id} <http://www.w3.org/ns/odrl/2/constraint> {cst_end} ."));
        }
        Condition::Purpose(purpose) => {
            let cst_id = format!("<{}#odrl-cst-purpose>", row.resource_uri);
            triples.push(format!(
                "{cst_id} <http://www.w3.org/ns/odrl/2/leftOperand> <http://www.w3.org/ns/odrl/2/purpose> ."
            ));
            triples.push(format!(
                "{cst_id} <http://www.w3.org/ns/odrl/2/operator> <http://www.w3.org/ns/odrl/2/eq> ."
            ));
            triples.push(format!(
                "{cst_id} <http://www.w3.org/ns/odrl/2/rightOperand> <{purpose}> ."
            ));
            triples.push(format!("{rule_id} <http://www.w3.org/ns/odrl/2/constraint> {cst_id} ."));
        }
        Condition::Count(n) => {
            let cst_id = format!("<{}#odrl-cst-count>", row.resource_uri);
            triples.push(format!(
                "{cst_id} <http://www.w3.org/ns/odrl/2/leftOperand> <http://www.w3.org/ns/odrl/2/count> ."
            ));
            triples.push(format!(
                "{cst_id} <http://www.w3.org/ns/odrl/2/operator> <http://www.w3.org/ns/odrl/2/lteq> ."
            ));
            triples.push(format!(
                "{cst_id} <http://www.w3.org/ns/odrl/2/rightOperand> \"{n}\"^^<http://www.w3.org/2001/XMLSchema#integer> ."
            ));
            triples.push(format!("{rule_id} <http://www.w3.org/ns/odrl/2/constraint> {cst_id} ."));
        }
        Condition::And(a, b) => {
            // Compound constraint: emit both sub-conditions as separate constraints.
            // Full nested-and encoding is left for B6 (workload/oracle engine).
            let sub_a = IntentRow {
                condition: *a.clone(),
                ..row.clone()
            };
            let sub_b = IntentRow {
                condition: *b.clone(),
                ..row.clone()
            };
            let compiled_a = compile_odrl(&sub_a);
            let compiled_b = compile_odrl(&sub_b);
            triples.extend(compiled_a.nquads);
            triples.extend(compiled_b.nquads);
        }
    }

    CompiledPolicy { model: AcModel::Odrl, nquads: triples, expressibility: Expressibility::Native }
}

// ── Decision Oracle ──────────────────────────────────────────────────────────────────

/// A request tuple for the by-construction oracle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// The agent IRI making the request.
    pub agent: String,
    /// The client application IRI (may be empty for WAC/ACP without client restriction).
    pub client: Option<String>,
    /// The resource URI being accessed.
    pub resource: String,
    /// The access mode requested.
    pub mode: AccessMode,
}

/// The outcome of an oracle evaluation.
///
/// **Fail-closed invariant**: the default when no matching rule exists is [`Decision::Deny`].
/// This is tested in `tests/determinism.rs` via the negative fixture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Access is granted.
    Allow,
    /// Access is denied. This is the **default** (fail-closed).
    Deny,
}

impl Default for Decision {
    /// Default is [`Decision::Deny`] — fail-closed.
    fn default() -> Self {
        Decision::Deny
    }
}

/// A single expected-decision row, as produced by the generators and consumed by the
/// workload engine (B6).
#[derive(Debug, Clone)]
pub struct ExpectedDecision {
    /// The request this decision applies to.
    pub request: Request,
    /// The access-control model for which this decision was computed.
    pub model: AcModel,
    /// The expected decision (fail-closed: `Deny` if no rule matches).
    pub decision: Decision,
}

/// A W2 query fixture: a SPARQL query string and its expected result set.
///
/// Shared by all four use-case generators. Each fixture bundles the query, the
/// agent under whose authorization it runs, the model, and the closed-form expected
/// result set.
#[derive(Debug, Clone)]
pub struct QueryFixture {
    /// The W2 query class.
    pub class: QueryClass,
    /// SPARQL query string.
    pub sparql: String,
    /// Expected result rows as sorted N-Quads strings (one per solution).
    pub expected_rows: Vec<String>,
    /// The requesting agent this query is evaluated under.
    pub agent: String,
    /// The access-control model to use for evaluation.
    pub model: AcModel,
}

/// W2 query class (§2.1 workload W2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryClass {
    /// Q-point: explicit `GRAPH` lookup for a known resource.
    Point,
    /// Q-scan: container listing via `FROM <…#union-default-graph>`.
    Scan,
    /// Q-join: cross-pod / cross-container join.
    Join,
    /// Q-agg: `COUNT` over the authorized subset.
    Aggregate,
}

/// Evaluate one [`Request`] against a slice of [`IntentRow`]s using the
/// by-construction procedural oracle for WAC semantics.
///
/// This oracle is **structurally independent** of sparq's N3 rule engine and
/// `AclIndex` — it reads only the intent table and applies WAC semantics directly.
///
/// **Fail-closed**: returns [`Decision::Deny`] if no matching `Allow` intent is found,
/// even if an explicit `Deny` is also absent.
///
/// **Determinism**: pure function of `request` and `intents`; no random, no time.
#[must_use]
pub fn oracle_wac(request: &Request, intents: &[IntentRow]) -> Decision {
    oracle::evaluate_wac(request, intents)
}

/// Evaluate one [`Request`] against a slice of [`IntentRow`]s using the
/// by-construction procedural oracle for ACP semantics.
///
/// ACP semantics: any explicit `Deny` overrides any `Allow` (deny-wins).
/// Fail-closed: no matching rule → [`Decision::Deny`].
#[must_use]
pub fn oracle_acp(request: &Request, intents: &[IntentRow]) -> Decision {
    oracle::evaluate_acp(request, intents)
}

/// Evaluate one [`Request`] against a slice of [`IntentRow`]s using the
/// by-construction procedural oracle for ODRL semantics.
///
/// ODRL semantics: Permission overrides Prohibition; Prohibition without a matching
/// Permission is a deny. Usage conditions are evaluated procedurally.
/// Fail-closed: no matching rule → [`Decision::Deny`].
#[must_use]
pub fn oracle_odrl(request: &Request, intents: &[IntentRow]) -> Decision {
    oracle::evaluate_odrl(request, intents)
}

#[cfg(test)]
mod expressibility_tests {
    use super::{AnswerSafety, GenAiRetrievalExpressibilityEntry, GENAI_RETRIEVAL_EXPRESSIBILITY};

    const VECTORS_MANIFEST: &str = include_str!("../../sparq-vectors/Cargo.toml");
    const NLQ_MANIFEST: &str = include_str!("../../sparq-nlq/Cargo.toml");

    fn feature_is_declared(manifest: &str, feature: &str) -> bool {
        manifest
            .lines()
            .skip_while(|line| line.trim() != "[features]")
            .skip(1)
            .take_while(|line| !line.trim_start().starts_with('['))
            .filter_map(|line| line.split_once('=').map(|(name, _)| name.trim()))
            .any(|name| name == feature)
    }

    #[test]
    fn genai_retrieval_expressibility_catalogue_is_complete_and_grounded() {
        const EXPECTED: [GenAiRetrievalExpressibilityEntry; 5] = [
            GenAiRetrievalExpressibilityEntry {
                capability: "vec:nearest ANN-in-SPARQL",
                provider_crate: "sparq-vectors",
                provider_feature: "vec-predicate",
                answer_safety: AnswerSafety::ApproximateProposes,
            },
            GenAiRetrievalExpressibilityEntry {
                capability: "filtered ANN by type",
                provider_crate: "sparq-vectors",
                provider_feature: "filtered-ann",
                answer_safety: AnswerSafety::ApproximateProposes,
            },
            GenAiRetrievalExpressibilityEntry {
                capability: "hybrid text+vector fusion",
                provider_crate: "sparq-vectors",
                provider_feature: "default",
                answer_safety: AnswerSafety::ApproximateProposes,
            },
            GenAiRetrievalExpressibilityEntry {
                capability: "NL→SPARQL",
                provider_crate: "sparq-nlq",
                provider_feature: "live",
                answer_safety: AnswerSafety::ExactValidates,
            },
            GenAiRetrievalExpressibilityEntry {
                capability: "provenance-carrying citation",
                provider_crate: "sparq-nlq",
                provider_feature: "citations",
                answer_safety: AnswerSafety::ExactValidates,
            },
        ];

        assert_eq!(
            GENAI_RETRIEVAL_EXPRESSIBILITY.len(),
            5,
            "the catalogue count pins the five GenAI-retrieval additions"
        );
        assert_eq!(
            GENAI_RETRIEVAL_EXPRESSIBILITY, EXPECTED,
            "every authoritative GenAI-retrieval capability must be present exactly once"
        );

        for entry in GENAI_RETRIEVAL_EXPRESSIBILITY {
            let manifest = match entry.provider_crate {
                "sparq-vectors" => VECTORS_MANIFEST,
                "sparq-nlq" => NLQ_MANIFEST,
                other => panic!("catalogue references an unknown provider crate: {other}"),
            };
            assert!(
                feature_is_declared(manifest, entry.provider_feature),
                "{}/{} is not a declared workspace crate/feature pair",
                entry.provider_crate,
                entry.provider_feature
            );
        }
    }
}
