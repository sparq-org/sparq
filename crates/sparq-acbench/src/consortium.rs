//! U4 — Research-data consortium generator (bead `sq-i6du2.5`).
//!
//! Generates a dataset modelling a research consortium: datasets, papers-in-progress,
//! instruments, and consortium membership rolls with temporal embargo constraints,
//! very large flat groups, public-after-embargo flips (churn workload), and
//! authenticated-agent-wide grants.
//!
//! # AC shape stressed
//! - **Temporal ODRL constraints** (`odrl:dateTime` embargo-until): every dataset resource
//!   carries an embargo window that gates read access. WAC and ACP cannot express these
//!   constraints; all embargo intents are ODRL-only in the expressibility matrix.
//! - **Very large flat groups**: `members_per_group` drives consortium membership roll size
//!   (no nesting depth needed — flat consortium list).
//! - **Authenticated-agent-wide grants**: instruments and membership rolls are visible to
//!   all authenticated consortium members (no resource-level enumeration needed).
//! - **Embargo flips** (W3 churn): each flip removes the embargo constraint from one
//!   dataset and turns the ODRL permission into an unconditional public allow; the oracle
//!   emits exact pre-flip and post-flip decision deltas by construction.
//!
//! # Clock discipline
//! The generator uses a **pinned epoch** (`PINNED_EPOCH_SECS`) in place of any
//! wall-clock call. All temporal embargo windows are expressed as offsets from this
//! epoch so the output is deterministic across platforms and runs.
//!
//! # File ownership
//! **Only bead `sq-i6du2.5` edits this file.**
//!
//! # Oracle independence
//! Expected decisions are computed by a small procedural evaluator over the intent table
//! (`oracle_odrl_with_embargo`) — it never calls sparq's N3 rule engine, `AclIndex`,
//! or `sparq-policy` evaluator.

use std::collections::{BTreeMap, BTreeSet};

use rand::prelude::SmallRng;
use rand::{Rng, SeedableRng};

use crate::project_mgmt::ChurnStep;
use crate::{
    AcModel, AccessMode, Audience, CompiledPolicy, Condition, Decision, Effect,
    ExpectedDecision, Expressibility, GenParams, IntentRow, QueryClass, QueryFixture, Request,
    Scope, compile_acp, compile_odrl, compile_wac,
};

// ── Pinned epoch ─────────────────────────────────────────────────────────────────────

/// Pinned epoch used for all temporal embargo deltas (Unix seconds).
///
/// Fixed at 2026-01-01T00:00:00Z — chosen to pre-date the benchmark publication
/// window and to make all embargo dates easy to read in tests. No wall-clock call
/// is made anywhere in this module; all dates are offsets from this constant.
const PINNED_EPOCH_SECS: i64 = 1_767_225_600; // 2026-01-01T00:00:00Z

/// One day in seconds (used for embargo-window arithmetic).
const DAY_SECS: i64 = 86_400;

// ── Internal dataset structures ───────────────────────────────────────────────────────

/// A single research dataset resource in the consortium.
struct DatasetResource {
    /// Base URI for this dataset.
    uri: String,
    /// Which institution owns it.
    institution_idx: usize,
    /// Embargo end time (epoch seconds, pinned clock). `None` = no embargo (public).
    embargo_end_secs: Option<i64>,
    /// The consortium membership group IRI that may read this after embargo.
    group_uri: String,
}

/// A paper-in-progress resource.
struct PaperResource {
    uri: String,
    /// Only the owner and consortium group can read.
    group_uri: String,
}

/// An instrument resource (shared read for all authenticated agents).
struct InstrumentResource {
    uri: String,
}

/// A consortium membership roll.
struct MembershipRoll {
    /// Group IRI (vcard:Group / flat list).
    group_uri: String,
    /// Member IRIs.
    members: Vec<String>,
}

// ── Output struct ─────────────────────────────────────────────────────────────────────

/// Output of the U4 research-data-consortium generator.
pub struct ConsortiumDataset {
    /// Group IRIs mapped to their resolved agent members, including nested membership.
    pub group_memberships: BTreeMap<String, BTreeSet<String>>,
    /// N-Quads lines forming the data graph.
    pub data_nquads: Vec<String>,
    /// Compiled WAC policy graph.
    pub wac_policy: Vec<CompiledPolicy>,
    /// Compiled ACP policy graph.
    pub acp_policy: Vec<CompiledPolicy>,
    /// Compiled ODRL policy graph.
    pub odrl_policy: Vec<CompiledPolicy>,
    /// Model-agnostic intent table.
    pub intents: Vec<IntentRow>,
    /// Request tuples with expected decisions.
    pub expected_decisions: Vec<ExpectedDecision>,
    /// W2 SPARQL query fixtures.
    pub queries: Vec<QueryFixture>,
    /// W3 embargo-flip churn steps with exact expected decision deltas.
    pub embargo_flips: Vec<ChurnStep>,
}

// ── Generator entry point ─────────────────────────────────────────────────────────────

/// Generate a U4 research-data-consortium dataset.
///
/// # Invariants
/// - **Determinism**: same `params` → same output on every call, platform, and rustc
///   version. Only `SmallRng::seed_from_u64(params.seed)` is used for randomness; no
///   wall-clock, thread ID, or OS entropy enters the output path.
/// - **Pinned clock**: embargo windows use the pinned epoch constant (`PINNED_EPOCH_SECS`,
///   2026-01-01T00:00:00Z) as the reference, not `SystemTime::now()`. This guarantees
///   byte-identical N-Quads and decisions.
/// - **Embargo flips** in `embargo_flips` produce exact decision deltas by construction
///   (the oracle evaluates the post-flip intent table directly; no sparq evaluator is
///   called).
/// - **Fail-closed default**: any request not matched by a policy is `Decision::Deny`.
///
/// # AC shape produced
/// - All embargo constraints are `Condition::Temporal` and compile only to ODRL
///   (`Expressibility::Unsupported` for WAC and ACP).
/// - Consortium-member read grants use `Audience::Group` (large flat groups).
/// - Instrument access uses `Audience::Authenticated` (wide grants).
/// - Owner write/control uses `Audience::Owner` (narrow, per-resource).
///
/// # Panics
/// Panics if `params.validate()` fails.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn generate(params: &GenParams) -> ConsortiumDataset {
    params.validate().expect("GenParams must be valid");

    let mut rng = SmallRng::seed_from_u64(params.seed);

    // ── 1. Derive counts from scale factor ────────────────────────────────────────────
    let n_institutions = (2 + params.sf as usize).min(16);
    let datasets_per_institution = (4 * params.sf as usize).max(4);
    let papers_per_institution = (2 * params.sf as usize).max(2);
    let n_instruments = (2 + params.sf as usize).max(2);
    let members_per_group = (params.members_per_group as usize).max(2);
    let n_agents = (params.n_agents as usize).max(4);

    let base = "https://consortium.example/";

    // ── 2. Build membership rolls (flat groups, one per institution) ───────────────────
    let mut rolls: Vec<MembershipRoll> = Vec::with_capacity(n_institutions);
    let mut all_agents: Vec<String> = Vec::with_capacity(n_agents);

    for inst_idx in 0..n_institutions {
        let group_uri = format!("{base}inst{inst_idx}/members");
        let mut members: Vec<String> = Vec::with_capacity(members_per_group);
        for m_idx in 0..members_per_group {
            let agent = format!("{base}agents/inst{inst_idx}-m{m_idx}");
            members.push(agent.clone());
            if all_agents.len() < n_agents {
                all_agents.push(agent);
            }
        }
        rolls.push(MembershipRoll { group_uri, members });
    }
    // Pad all_agents to n_agents with cross-institution agents.
    while all_agents.len() < n_agents {
        let idx = all_agents.len();
        all_agents.push(format!("{base}agents/extra{idx}"));
    }

    let group_memberships = rolls.iter()
        .map(|roll| (roll.group_uri.clone(), roll.members.iter().cloned().collect()))
        .collect();

    // ── 3. Build dataset resources ────────────────────────────────────────────────────
    let mut datasets: Vec<DatasetResource> = Vec::new();
    for (inst_idx, roll) in rolls.iter().enumerate().take(n_institutions) {
        let group_uri = roll.group_uri.clone();
        for ds_idx in 0..datasets_per_institution {
            let uri = format!("{base}inst{inst_idx}/datasets/ds{ds_idx}");
            // Deterministically decide embargo: ~60% have embargo.
            let has_embargo = rng.gen_bool(0.6);
            let embargo_end_secs = if has_embargo {
                // Embargo windows: some already expired (before pinned epoch),
                // some in the future (after). The churn workload flips future ones.
                let offset_days: i64 = rng.gen_range(-30_i64..180_i64);
                Some(PINNED_EPOCH_SECS + offset_days * DAY_SECS)
            } else {
                None // No embargo — public data.
            };
            datasets.push(DatasetResource {
                uri,
                institution_idx: inst_idx,
                embargo_end_secs,
                group_uri: group_uri.clone(),
            });
        }
    }

    // ── 4. Build paper resources ──────────────────────────────────────────────────────
    let mut papers: Vec<PaperResource> = Vec::new();
    for (inst_idx, roll) in rolls.iter().enumerate().take(n_institutions) {
        let group_uri = roll.group_uri.clone();
        for p_idx in 0..papers_per_institution {
            let uri = format!("{base}inst{inst_idx}/papers/p{p_idx}");
            papers.push(PaperResource { uri, group_uri: group_uri.clone() });
        }
    }

    // ── 5. Build instrument resources ─────────────────────────────────────────────────
    let instruments: Vec<InstrumentResource> = (0..n_instruments)
        .map(|i| InstrumentResource { uri: format!("{base}instruments/inst{i}") })
        .collect();

    // ── 6. Build N-Quads data graph ───────────────────────────────────────────────────
    let mut data_nquads: Vec<String> = Vec::new();
    let data_graph = "<https://consortium.example/data>";
    let rdf_type = "<http://www.w3.org/1999/02/22-rdf-syntax-ns#type>";
    let dcterms_title = "<http://purl.org/dc/terms/title>";
    let schema_dataset = "<https://schema.org/Dataset>";
    let schema_article = "<https://schema.org/Article>";
    let schema_instrument = "<https://schema.org/Instrument>";
    let vcard_group = "<http://www.w3.org/2006/vcard/ns#Group>";
    let vcard_has_member = "<http://www.w3.org/2006/vcard/ns#hasMember>";
    let schema_member_of = "<https://schema.org/memberOf>";

    // Datasets.
    for ds in &datasets {
        let r = format!("<{}>", ds.uri);
        data_nquads.push(format!("{r} {rdf_type} {schema_dataset} {data_graph} ."));
        data_nquads.push(format!("{r} {dcterms_title} \"Dataset {}\" {data_graph} .", ds.uri));
    }

    // Papers.
    for paper in &papers {
        let r = format!("<{}>", paper.uri);
        data_nquads.push(format!("{r} {rdf_type} {schema_article} {data_graph} ."));
        data_nquads.push(format!("{r} {dcterms_title} \"Paper {}\" {data_graph} .", paper.uri));
    }

    // Instruments.
    for instr in &instruments {
        let r = format!("<{}>", instr.uri);
        data_nquads.push(format!("{r} {rdf_type} {schema_instrument} {data_graph} ."));
    }

    // Membership rolls (vcard:Group with hasMember).
    for roll in &rolls {
        let g = format!("<{}>", roll.group_uri);
        data_nquads.push(format!("{g} {rdf_type} {vcard_group} {data_graph} ."));
        for member in &roll.members {
            let m = format!("<{member}>");
            data_nquads.push(format!("{g} {vcard_has_member} {m} {data_graph} ."));
            data_nquads.push(format!("{m} {schema_member_of} {g} {data_graph} ."));
        }
    }

    // ── 7. Build intent table ─────────────────────────────────────────────────────────
    let mut intents: Vec<IntentRow> = Vec::new();

    // Dataset intents.
    for ds in &datasets {
        // 7a. Owner gets full access (no condition — expressible in all models).
        intents.push(IntentRow {
            audience: Audience::Owner,
            scope: Scope::Resource,
            mode: AccessMode::full(),
            condition: Condition::None,
            effect: Effect::Allow,
            resource_uri: ds.uri.clone(),
        });

        match ds.embargo_end_secs {
            None => {
                // No embargo: public read.
                intents.push(IntentRow {
                    audience: Audience::Public,
                    scope: Scope::Resource,
                    mode: AccessMode::read_only(),
                    condition: Condition::None,
                    effect: Effect::Allow,
                    resource_uri: ds.uri.clone(),
                });
                // Consortium members can also write (no condition).
                intents.push(IntentRow {
                    audience: Audience::Group(ds.group_uri.clone()),
                    scope: Scope::Resource,
                    mode: AccessMode { read: true, write: true, control: false },
                    condition: Condition::None,
                    effect: Effect::Allow,
                    resource_uri: ds.uri.clone(),
                });
            }
            Some(embargo_end) => {
                // Embargoed: consortium members can read BUT only within a temporal
                // window [PINNED_EPOCH, embargo_end). ODRL-only (WAC/ACP unsupported).
                let embargo_start_iso = epoch_secs_to_iso(PINNED_EPOCH_SECS);
                let embargo_end_iso = epoch_secs_to_iso(embargo_end);
                intents.push(IntentRow {
                    audience: Audience::Group(ds.group_uri.clone()),
                    scope: Scope::Resource,
                    mode: AccessMode::read_only(),
                    condition: Condition::Temporal {
                        start: embargo_start_iso,
                        end: embargo_end_iso,
                    },
                    effect: Effect::Allow,
                    resource_uri: ds.uri.clone(),
                });
            }
        }
    }

    // Paper intents: owner + consortium-group read.
    for paper in &papers {
        // Owner full access.
        intents.push(IntentRow {
            audience: Audience::Owner,
            scope: Scope::Resource,
            mode: AccessMode::full(),
            condition: Condition::None,
            effect: Effect::Allow,
            resource_uri: paper.uri.clone(),
        });
        // Consortium group read.
        intents.push(IntentRow {
            audience: Audience::Group(paper.group_uri.clone()),
            scope: Scope::Resource,
            mode: AccessMode::read_only(),
            condition: Condition::None,
            effect: Effect::Allow,
            resource_uri: paper.uri.clone(),
        });
    }

    // Instrument intents: authenticated read for all.
    for instr in &instruments {
        intents.push(IntentRow {
            audience: Audience::Authenticated,
            scope: Scope::Resource,
            mode: AccessMode::read_only(),
            condition: Condition::None,
            effect: Effect::Allow,
            resource_uri: instr.uri.clone(),
        });
    }

    // ── 8. Compile per-model policies ────────────────────────────────────────────────
    let mut wac_policy: Vec<CompiledPolicy> = Vec::new();
    let mut acp_policy: Vec<CompiledPolicy> = Vec::new();
    let mut odrl_policy: Vec<CompiledPolicy> = Vec::new();

    for row in &intents {
        // Group members for ACP expansion: flatten the roll for this group.
        let group_members: Vec<String> = if let Audience::Group(ref g) = row.audience {
            rolls.iter()
                .find(|r| &r.group_uri == g)
                .map(|r| r.members.clone())
                .unwrap_or_default()
        } else {
            vec![]
        };

        wac_policy.push(compile_wac(row));
        acp_policy.push(compile_acp(row, &group_members));
        odrl_policy.push(compile_odrl(row));
    }

    // ── 9. Build expected decisions ───────────────────────────────────────────────────
    let mut expected_decisions: Vec<ExpectedDecision> = Vec::new();

    // For each dataset, generate oracle decisions for representative agents.
    for ds in &datasets {
        let owner_agent = format!("{}#owner", ds.uri);
        let owner_req = Request {
            agent: owner_agent.clone(),
            client: None,
            resource: ds.uri.clone(),
            mode: AccessMode::read_only(),
        };
        // Owner always allowed (unconditional full intent).
        for model in [AcModel::Wac, AcModel::Acp, AcModel::Odrl] {
            expected_decisions.push(ExpectedDecision {
                request: owner_req.clone(),
                model: model.clone(),
                decision: oracle_for_model(&owner_req, &intents, &model),
            });
        }

        // A consortium member from the owning institution.
        if let Some(first_member) = rolls[ds.institution_idx].members.first() {
            let member_req = Request {
                agent: first_member.clone(),
                client: None,
                resource: ds.uri.clone(),
                mode: AccessMode::read_only(),
            };
            for model in [AcModel::Wac, AcModel::Acp, AcModel::Odrl] {
                expected_decisions.push(ExpectedDecision {
                    request: member_req.clone(),
                    model: model.clone(),
                    decision: oracle_for_model(&member_req, &intents, &model),
                });
            }
        }

        // An external / anonymous agent (always Deny for embargoed datasets).
        let ext_req = Request {
            agent: format!("{base}external/stranger"),
            client: None,
            resource: ds.uri.clone(),
            mode: AccessMode::read_only(),
        };
        for model in [AcModel::Wac, AcModel::Acp, AcModel::Odrl] {
            expected_decisions.push(ExpectedDecision {
                request: ext_req.clone(),
                model: model.clone(),
                decision: oracle_for_model(&ext_req, &intents, &model),
            });
        }
    }

    // For instruments: authenticated agents should be allowed.
    for instr in &instruments {
        if let Some(agent) = all_agents.first() {
            let req = Request {
                agent: agent.clone(),
                client: None,
                resource: instr.uri.clone(),
                mode: AccessMode::read_only(),
            };
            for model in [AcModel::Wac, AcModel::Acp, AcModel::Odrl] {
                expected_decisions.push(ExpectedDecision {
                    request: req.clone(),
                    model: model.clone(),
                    decision: oracle_for_model(&req, &intents, &model),
                });
            }
        }
    }

    // ── 10. Build W2 query fixtures ───────────────────────────────────────────────────
    let mut queries: Vec<QueryFixture> = Vec::new();

    // Q-point: a member reads a specific embargoed dataset via ODRL model.
    if let (Some(ds), Some(agent)) = (
        datasets.iter().find(|d| d.embargo_end_secs.is_some()),
        all_agents.first(),
    ) {
        let sparql = format!(
            "SELECT ?p ?o WHERE {{ GRAPH <{}> {{ <{}> ?p ?o . }} }}",
            "https://consortium.example/data", ds.uri
        );
        let req = Request {
            agent: agent.clone(),
            client: None,
            resource: ds.uri.clone(),
            mode: AccessMode::read_only(),
        };
        let dec_odrl = oracle_for_model(&req, &intents, &AcModel::Odrl);
        let expected_rows = if dec_odrl == Decision::Allow {
            vec![
                format!(
                    "<http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://schema.org/Dataset>"
                ),
                format!("<http://purl.org/dc/terms/title> \"Dataset {}\"", ds.uri),
            ]
        } else {
            vec![]
        };
        queries.push(QueryFixture {
            class: QueryClass::Point,
            sparql,
            expected_rows,
            agent: agent.clone(),
            model: AcModel::Odrl,
        });
    }

    // Q-scan: list all public datasets (WAC model — only public intents expressed).
    {
        let public_uris: Vec<String> = datasets
            .iter()
            .filter(|d| d.embargo_end_secs.is_none())
            .map(|d| d.uri.clone())
            .collect();
        let sparql = format!(
            "SELECT ?dataset WHERE {{ GRAPH <{}> {{ ?dataset <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://schema.org/Dataset> . }} }}",
            "https://consortium.example/data"
        );
        let expected_rows = public_uris
            .iter()
            .map(|u| format!("?dataset=<{u}>"))
            .collect();
        queries.push(QueryFixture {
            class: QueryClass::Scan,
            sparql,
            expected_rows,
            agent: String::new(), // public query
            model: AcModel::Wac,
        });
    }

    // Q-join: join papers with instruments (ACP model — authenticated read for instruments).
    if let Some(agent) = all_agents.first() {
        let sparql = format!(
            "SELECT ?paper ?instr WHERE {{ GRAPH <{}> {{ ?paper <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://schema.org/Article> . ?instr <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://schema.org/Instrument> . }} }}",
            "https://consortium.example/data"
        );
        let expected_rows: Vec<String> = instruments.iter().flat_map(|instr| {
            papers.iter().filter_map(|paper| {
                let instr_req = Request {
                    agent: agent.clone(),
                    client: None,
                    resource: instr.uri.clone(),
                    mode: AccessMode::read_only(),
                };
                let paper_req = Request {
                    agent: agent.clone(),
                    client: None,
                    resource: paper.uri.clone(),
                    mode: AccessMode::read_only(),
                };
                if oracle_for_model(&instr_req, &intents, &AcModel::Acp) == Decision::Allow
                    && oracle_for_model(&paper_req, &intents, &AcModel::Acp) == Decision::Allow
                {
                    Some(format!("?paper=<{}> ?instr=<{}>", paper.uri, instr.uri))
                } else {
                    None
                }
            }).collect::<Vec<_>>()
        }).collect();
        queries.push(QueryFixture {
            class: QueryClass::Join,
            sparql,
            expected_rows,
            agent: agent.clone(),
            model: AcModel::Acp,
        });
    }

    // Q-agg: count datasets readable by a consortium member (ODRL).
    if let Some(agent) = all_agents.first() {
        let sparql = format!(
            "SELECT (COUNT(?dataset) AS ?cnt) WHERE {{ GRAPH <{}> {{ ?dataset <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://schema.org/Dataset> . }} }}",
            "https://consortium.example/data"
        );
        let readable_count: usize = datasets
            .iter()
            .filter(|d| {
                let req = Request {
                    agent: agent.clone(),
                    client: None,
                    resource: d.uri.clone(),
                    mode: AccessMode::read_only(),
                };
                oracle_for_model(&req, &intents, &AcModel::Odrl) == Decision::Allow
            })
            .count();
        queries.push(QueryFixture {
            class: QueryClass::Aggregate,
            sparql,
            expected_rows: vec![format!("?cnt={readable_count}")],
            agent: agent.clone(),
            model: AcModel::Odrl,
        });
    }

    // ── 11. Build embargo-flip churn steps (W3) ───────────────────────────────────────
    //
    // For each embargoed dataset whose embargo has NOT yet expired
    // (embargo_end > PINNED_EPOCH), generate one flip step that:
    //  - removes the temporal ODRL policy triples
    //  - adds a new unconditional public-read permission across all models
    //  - records exact decision deltas (agents that change from Deny to Allow).
    let mut embargo_flips: Vec<ChurnStep> = Vec::new();

    for ds in &datasets {
        if let Some(embargo_end) = ds.embargo_end_secs {
            if embargo_end <= PINNED_EPOCH_SECS {
                // Already expired — not a meaningful churn (would produce no delta).
                continue;
            }

            // The ODRL permission row that will be removed (the temporal-conditioned one).
            let old_row = IntentRow {
                audience: Audience::Group(ds.group_uri.clone()),
                scope: Scope::Resource,
                mode: AccessMode::read_only(),
                condition: Condition::Temporal {
                    start: epoch_secs_to_iso(PINNED_EPOCH_SECS),
                    end: epoch_secs_to_iso(embargo_end),
                },
                effect: Effect::Allow,
                resource_uri: ds.uri.clone(),
            };
            let old_odrl = compile_odrl(&old_row);

            // The new row: unconditional public read (embargo lifted).
            let new_row = IntentRow {
                audience: Audience::Public,
                scope: Scope::Resource,
                mode: AccessMode::read_only(),
                condition: Condition::None,
                effect: Effect::Allow,
                resource_uri: ds.uri.clone(),
            };
            let new_wac = compile_wac(&new_row);
            let new_acp = compile_acp(&new_row, &[]);
            let new_odrl = compile_odrl(&new_row);

            let delta_remove = old_odrl.nquads.clone();
            let mut delta_add = new_wac.nquads.clone();
            delta_add.extend(new_acp.nquads.clone());
            delta_add.extend(new_odrl.nquads.clone());

            // Post-flip intent table: replace temporal row with unconditional public row.
            let post_flip_intents: Vec<IntentRow> = intents
                .iter()
                .filter(|r| {
                    !(r.resource_uri == ds.uri
                        && matches!(&r.condition, Condition::Temporal { .. }))
                })
                .cloned()
                .chain(std::iter::once(new_row.clone()))
                .collect();

            // Expected deltas: agents whose decision changes after the flip.
            let mut expected_deltas: Vec<ExpectedDecision> = Vec::new();

            let ext_req = Request {
                agent: format!("{base}external/stranger"),
                client: None,
                resource: ds.uri.clone(),
                mode: AccessMode::read_only(),
            };
            for model in [AcModel::Wac, AcModel::Acp, AcModel::Odrl] {
                let pre_dec = oracle_for_model(&ext_req, &intents, &model);
                let post_dec = oracle_for_model(&ext_req, &post_flip_intents, &model);
                if pre_dec != post_dec {
                    expected_deltas.push(ExpectedDecision {
                        request: ext_req.clone(),
                        model: model.clone(),
                        decision: post_dec,
                    });
                }
            }

            if let Some(member) = rolls[ds.institution_idx].members.first() {
                let member_req = Request {
                    agent: member.clone(),
                    client: None,
                    resource: ds.uri.clone(),
                    mode: AccessMode::read_only(),
                };
                for model in [AcModel::Wac, AcModel::Acp, AcModel::Odrl] {
                    let pre_dec = oracle_for_model(&member_req, &intents, &model);
                    let post_dec = oracle_for_model(&member_req, &post_flip_intents, &model);
                    if pre_dec != post_dec {
                        expected_deltas.push(ExpectedDecision {
                            request: member_req.clone(),
                            model: model.clone(),
                            decision: post_dec,
                        });
                    }
                }
            }

            // Always record at least one delta (post-flip external-agent ODRL state).
            if expected_deltas.is_empty() {
                expected_deltas.push(ExpectedDecision {
                    request: ext_req.clone(),
                    model: AcModel::Odrl,
                    decision: oracle_for_model(&ext_req, &post_flip_intents, &AcModel::Odrl),
                });
            }

            embargo_flips.push(ChurnStep {
                description: format!("Embargo lifted on <{}>", ds.uri),
                delta_add,
                delta_remove,
                expected_deltas,
            });
        }
    }

    // Guarantee at least one embargo flip exists (design record §3, B5 invariant).
    // If the RNG produced no future-embargo datasets, synthesise one deterministically.
    if embargo_flips.is_empty() {
        let synth_uri = format!("{base}synthetic/embargoed-ds");
        let synth_end = PINNED_EPOCH_SECS + 90 * DAY_SECS; // 90 days in the future
        let synth_group = format!("{base}synthetic/group");

        let old_row = IntentRow {
            audience: Audience::Group(synth_group.clone()),
            scope: Scope::Resource,
            mode: AccessMode::read_only(),
            condition: Condition::Temporal {
                start: epoch_secs_to_iso(PINNED_EPOCH_SECS),
                end: epoch_secs_to_iso(synth_end),
            },
            effect: Effect::Allow,
            resource_uri: synth_uri.clone(),
        };
        let old_odrl = compile_odrl(&old_row);

        let new_row = IntentRow {
            audience: Audience::Public,
            scope: Scope::Resource,
            mode: AccessMode::read_only(),
            condition: Condition::None,
            effect: Effect::Allow,
            resource_uri: synth_uri.clone(),
        };
        let new_wac = compile_wac(&new_row);
        let new_acp = compile_acp(&new_row, &[]);
        let new_odrl = compile_odrl(&new_row);

        let ext_req = Request {
            agent: format!("{base}external/stranger"),
            client: None,
            resource: synth_uri.clone(),
            mode: AccessMode::read_only(),
        };
        let expected_deltas = vec![ExpectedDecision {
            request: ext_req.clone(),
            model: AcModel::Odrl,
            decision: Decision::Allow,
        }];

        let mut delta_add = new_wac.nquads;
        delta_add.extend(new_acp.nquads);
        delta_add.extend(new_odrl.nquads);

        embargo_flips.push(ChurnStep {
            description: format!("Embargo lifted on <{synth_uri}> (synthetic)"),
            delta_add,
            delta_remove: old_odrl.nquads,
            expected_deltas,
        });
    }

    ConsortiumDataset {
        group_memberships,
        data_nquads,
        wac_policy,
        acp_policy,
        odrl_policy,
        intents,
        expected_decisions,
        queries,
        embargo_flips,
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────────────

/// Convert a pinned-epoch Unix timestamp (seconds) to an ISO 8601 dateTime string.
///
/// Uses fixed-arithmetic conversion (no `chrono`, no `SystemTime`) to avoid any
/// non-deterministic or platform-specific behaviour.
///
/// # Format
/// `"YYYY-MM-DDTHH:MM:SSZ"` (always UTC, no fractional seconds).
fn epoch_secs_to_iso(secs: i64) -> String {
    // Civil calendar from days since Unix epoch.
    // Reference: https://howardhinnant.github.io/date_algorithms.html
    let (days, time_secs) = if secs >= 0 {
        (secs / 86_400, secs % 86_400)
    } else {
        // Floor division for negative values.
        let d = (secs - 86_399) / 86_400;
        let t = secs - d * 86_400;
        (d, t)
    };

    let z = days + 719_468;
    let era: i64 = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    let hh = time_secs / 3600;
    let mm = (time_secs % 3600) / 60;
    let ss = time_secs % 60;

    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

/// Dispatch an oracle call to the appropriate model.
///
/// This is the by-construction procedural oracle: it reads only the intent table
/// and never calls any sparq crate. For ODRL it uses [`oracle_odrl_with_embargo`]
/// which evaluates temporal conditions against [`PINNED_EPOCH_SECS`].
fn oracle_for_model(request: &Request, intents: &[IntentRow], model: &AcModel) -> Decision {
    match model {
        AcModel::Wac => crate::oracle_wac(request, intents),
        AcModel::Acp => crate::oracle_acp(request, intents),
        AcModel::Odrl => oracle_odrl_with_embargo(request, intents),
    }
}

/// Evaluate ODRL semantics with embargo-aware temporal condition evaluation.
///
/// The scaffold [`crate::oracle_odrl`] treats all `Condition::Temporal` as
/// always-satisfied (scaffold stub). For U4, a temporal window `[start, end)` is
/// satisfied if and only if [`PINNED_EPOCH_SECS`] falls within `[start, end)`.
///
/// This is the U4 oracle's **correctness-critical** function: the W3 churn tests
/// verify that pre-flip decisions match this evaluation.
fn oracle_odrl_with_embargo(request: &Request, intents: &[IntentRow]) -> Decision {
    use crate::Scope;

    let mut any_permission = false;

    for intent in intents {
        let resource_matches = match intent.scope {
            Scope::Resource => request.resource == intent.resource_uri,
            Scope::Subtree => request.resource.starts_with(&intent.resource_uri),
        };
        if !resource_matches {
            continue;
        }

        if !mode_matches_local(&request.mode, &intent.mode) {
            continue;
        }

        if !audience_matches_odrl_u4(request, &intent.audience) {
            continue;
        }

        // Evaluate condition with pinned clock.
        if !evaluate_condition_pinned(&intent.condition) {
            continue;
        }

        if intent.effect == Effect::Allow {
            any_permission = true;
        }
    }

    if any_permission { Decision::Allow } else { Decision::Deny }
}

/// Check that the requested modes are all covered by the intent's modes.
///
/// Duplicated from `oracle.rs`'s private `mode_matches` (which is `pub(crate)`)
/// to avoid depending on the internal implementation — file-ownership rule forbids
/// editing `oracle.rs`.
fn mode_matches_local(requested: &AccessMode, granted: &AccessMode) -> bool {
    (!requested.read || granted.read)
        && (!requested.write || granted.write)
        && (!requested.control || granted.control)
}

/// Audience matching for the U4 ODRL oracle.
fn audience_matches_odrl_u4(request: &Request, audience: &Audience) -> bool {
    match audience {
        Audience::Public => true,
        Audience::Authenticated => !request.agent.is_empty(),
        Audience::Owner => request.agent == format!("{}#owner", request.resource),
        Audience::Agent(a) => &request.agent == a,
        Audience::Group(g) => {
            // Group membership: member IRIs are generated as
            // `{base}agents/inst{i}-m{j}` and the group URI is
            // `{base}inst{i}/members`. Rather than a prefix test (which would
            // be imprecise), we check whether the agent is an exact member of
            // any institution whose group URI matches.
            // For the U4 corpus the prefix `{base}agents/inst{i}-` uniquely
            // identifies institution `i`, and the group URI is
            // `{base}inst{i}/members`. We extract the institution index from
            // both and compare.
            let base = "https://consortium.example/";
            let group_prefix = format!("{base}inst");
            if let Some(rest) = g.strip_prefix(&group_prefix) {
                // rest = "{i}/members"
                if let Some(idx_str) = rest.strip_suffix("/members") {
                    let agent_prefix = format!("{base}agents/inst{idx_str}-");
                    return request.agent.starts_with(&agent_prefix);
                }
            }
            // Fallback for synthetic groups: exact prefix match on the group URI.
            request.agent.starts_with(g.as_str())
        }
        Audience::ClientRestricted { agent, client } => {
            &request.agent == agent
                && request.client.as_deref() == Some(client.as_str())
        }
        Audience::AllExcept(excl) => {
            !excl.iter().any(|e| e == &request.agent)
        }
    }
}

/// Evaluate an ODRL condition against the pinned epoch.
///
/// - `Condition::None` → always satisfied.
/// - `Condition::Temporal { start, end }` → satisfied iff
///   `PINNED_EPOCH_SECS ∈ [parse(start), parse(end))`.
/// - Other conditions → delegated to a conservative `true` (B6 supplies full eval).
fn evaluate_condition_pinned(condition: &Condition) -> bool {
    match condition {
        Condition::Temporal { start, end } => {
            let start_secs = iso_to_epoch_secs(start);
            let end_secs = iso_to_epoch_secs(end);
            PINNED_EPOCH_SECS >= start_secs && PINNED_EPOCH_SECS < end_secs
        }
        Condition::Count(n) => *n > 0,
        Condition::And(a, b) => {
            evaluate_condition_pinned(a) && evaluate_condition_pinned(b)
        }
        // None and Purpose are always satisfied in the U4 oracle.
        Condition::None | Condition::Purpose(_) => true,
    }
}

/// Parse an ISO 8601 dateTime string (`"YYYY-MM-DDTHH:MM:SSZ"`) to Unix epoch seconds.
///
/// Inverse of [`epoch_secs_to_iso`]. Deterministic, no external crates.
/// Only handles UTC dates in the format produced by this module.
fn iso_to_epoch_secs(s: &str) -> i64 {
    // Expected format: "YYYY-MM-DDTHH:MM:SSZ" (20+ bytes)
    let bytes = s.as_bytes();
    if bytes.len() < 20 {
        return 0;
    }
    let year: i64 = parse_dec(&bytes[0..4]);
    let month: i64 = parse_dec(&bytes[5..7]);
    let day: i64 = parse_dec(&bytes[8..10]);
    let hour: i64 = parse_dec(&bytes[11..13]);
    let minute: i64 = parse_dec(&bytes[14..16]);
    let second: i64 = parse_dec(&bytes[17..19]);

    // Days since Unix epoch via civil calendar algorithm.
    // Reference: https://howardhinnant.github.io/date_algorithms.html
    let y = if month <= 2 { year - 1 } else { year };
    let m = month;
    let d = day;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;

    days * 86_400 + hour * 3600 + minute * 60 + second
}

/// Parse decimal digits from ASCII bytes.
fn parse_dec(bytes: &[u8]) -> i64 {
    bytes.iter().fold(0_i64, |acc, &b| acc * 10 + i64::from(b - b'0'))
}

// ── Expressibility matrix note ────────────────────────────────────────────────────────

/// Return the expressibility classification for a U4 intent row and model.
///
/// Documented asymmetries (design record §2.2):
/// - `Condition::Temporal` → [`Expressibility::Unsupported`] for WAC and ACP;
///   [`Expressibility::Native`] for ODRL.
/// - `Audience::Group` with `Condition::None` → [`Expressibility::Native`] for WAC
///   (uses `acl:agentGroup`); [`Expressibility::Expansion`] for ACP
///   (per-member expansion); [`Expressibility::Native`] for ODRL (`PartyCollection`).
#[must_use]
pub fn expressibility_for(row: &IntentRow, model: &AcModel) -> Expressibility {
    match model {
        AcModel::Wac => compile_wac(row).expressibility,
        AcModel::Acp => compile_acp(row, &[]).expressibility,
        AcModel::Odrl => compile_odrl(row).expressibility,
    }
}
