//! U2 — Commercial project-management generator (bead `sq-i6du2.3`).
//!
//! Generates a Graphmetrix-inspired dataset with org → project → site → document-set
//! hierarchy, cross-org subcontractor access, role/team group reuse, and all-except
//! (deny-shaped) access intents.
//!
//! # AC shape stressed
//! - Role/team groups with **cross-org group reuse** (same group IRI referenced by
//!   multiple container policies).
//! - Wide flat containers (many documents per project).
//! - "All-except" intents: ACP/ODRL native; WAC inexpressible → enum-allow blowup.
//! - Handover/revocation churn (W3 workload): grant-then-revoke sequences.
//! - `policies_per_resource` drives accreted policy fan-in.
//!
//! # Hierarchy
//! ```text
//! https://bench.sparq.dev/pm/org/{oi}/
//!   project/{pi}/
//!     site/{si}/
//!       doc/{di}
//! ```
//!
//! # File ownership
//! **Only bead `sq-i6du2.3` edits this file.**
//!
//! # Implementation notes
//! Counts (orgs, projects-per-org, sites-per-project, docs-per-site) all scale linearly
//! with `sf`. The PRNG is seeded once from `params.seed` and advanced in a fixed order so
//! the same [`GenParams`] always produces the same corpus.

use std::collections::{BTreeMap, BTreeSet};

use rand::RngCore;
use rand::SeedableRng;
use rand::rngs::SmallRng;

use crate::{
    AcModel, AccessMode, Audience, CompiledPolicy, Condition, Decision, Effect,
    ExpectedDecision, Expressibility, ExpressibilityEntry, GenParams, IntentRow,
    QueryClass, QueryFixture, Request, Scope, compile_acp, compile_odrl, compile_wac,
    oracle_acp, oracle_odrl, oracle_wac,
};

/// Output of the U2 commercial-project-management generator.
pub struct ProjectMgmtDataset {
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
    /// W3 churn steps: interleaved grant/revoke writes with expected decision deltas.
    pub churn_steps: Vec<ChurnStep>,
    /// Expressibility matrix entries. Each tuple is `(intent_index, entry)`.
    ///
    /// An entry is required for every all-except intent and every group-nesting intent
    /// (design record §2.2). The workload engine (B6) reads this to report the matrix.
    pub expressibility_matrix: Vec<(usize, ExpressibilityEntry)>,
}

/// A W3 churn step: one ACL write + the expected decision delta it produces.
#[derive(Debug, Clone)]
pub struct ChurnStep {
    /// Description of the write (for diagnostics).
    pub description: String,
    /// The grant or revoke operation (as N-Quads to add or remove).
    /// N-Quads triples to add (policy graph update).
    pub delta_add: Vec<String>,
    /// N-Quads triples to remove (policy graph update).
    pub delta_remove: Vec<String>,
    /// Expected decision changes: `(request, model, new_decision)`.
    pub expected_deltas: Vec<ExpectedDecision>,
}

// ── Namespace / IRI helpers ──────────────────────────────────────────────────────────────

const BASE: &str = "https://bench.sparq.dev/pm";
const PM_VOCAB: &str = "https://bench.sparq.dev/pm/vocab#";

const RDF_TYPE: &str = "<http://www.w3.org/1999/02/22-rdf-syntax-ns#type>";
const RDFS_LABEL: &str = "<http://www.w3.org/2000/01/rdf-schema#label>";
const LDP_CONTAINS: &str = "<http://www.w3.org/ns/ldp#contains>";
const LDP_BASIC_CONTAINER: &str = "http://www.w3.org/ns/ldp#BasicContainer";
const FOAF_DOCUMENT: &str = "http://xmlns.com/foaf/0.1/Document";
const VCARD_GROUP: &str = "http://www.w3.org/2006/vcard/ns#Group";
const VCARD_HAS_MEMBER: &str = "<http://www.w3.org/2006/vcard/ns#hasMember>";
const PM_ORG_TYPE: &str = "Organization";
const PM_PROJECT_TYPE: &str = "Project";
const PM_SITE_TYPE: &str = "Site";

fn iri(uri: &str) -> String {
    format!("<{uri}>")
}

fn triple(s: &str, p: &str, o: &str) -> String {
    format!("<{s}> {p} <{o}> .")
}

fn triple_lit(s: &str, p: &str, literal: &str) -> String {
    format!("<{s}> {p} {literal} .")
}

fn org_uri(oi: u32) -> String {
    format!("{BASE}/org/{oi}/")
}

fn project_uri(oi: u32, pi: u32) -> String {
    format!("{BASE}/org/{oi}/project/{pi}/")
}

fn site_uri(oi: u32, pi: u32, si: u32) -> String {
    format!("{BASE}/org/{oi}/project/{pi}/site/{si}/")
}

fn doc_uri(oi: u32, pi: u32, si: u32, di: u32) -> String {
    format!("{BASE}/org/{oi}/project/{pi}/site/{si}/doc/{di}")
}

fn owner_uri(oi: u32) -> String {
    format!("{BASE}/agents/org/{oi}/owner")
}

fn agent_uri(oi: u32, idx: u32) -> String {
    format!("{BASE}/agents/org/{oi}/agent/{idx}")
}

/// Role group IRI — cross-org reusable by convention (same IRI, multiple containers).
fn role_group_uri(oi: u32, role: &str) -> String {
    format!("{BASE}/groups/org/{oi}/{role}")
}

/// Shared subcontractor group between two adjacent orgs.
fn subcontractor_group_uri(oi: u32, oi2: u32) -> String {
    format!("{BASE}/groups/subcontractor/{oi}-{oi2}")
}

// ── Scale-factor mapping ─────────────────────────────────────────────────────────────────

fn n_orgs(sf: u32) -> u32 { (2 * sf).max(1) }
fn projects_per_org(sf: u32) -> u32 { (2 * sf).max(1) }
fn sites_per_project(sf: u32) -> u32 { (2 * sf).max(1) }
fn docs_per_site(sf: u32) -> u32 { (5 * sf).max(5) }
fn members_per_role(params: &GenParams) -> u32 { params.members_per_group.clamp(1, 8) }

// ── PRNG helpers ─────────────────────────────────────────────────────────────────────────

fn rand_bool(rng: &mut SmallRng, probability: f32) -> bool {
    // u32 in [0, 10^6) divided by 10^6.0 gives a uniform float in [0, 1).
    // The cast is intentional: 10^6 < 2^23 mantissa bits so values in [0, 10^6) are
    // represented exactly as f32; the division is an acceptable approximation.
    #[allow(clippy::cast_precision_loss)]
    let v = (rng.next_u32() % 1_000_000) as f32 / 1_000_000.0_f32;
    v < probability
}

fn next_u32_bounded(rng: &mut SmallRng, bound: u32) -> u32 {
    if bound == 0 { return 0; }
    rng.next_u32() % bound
}

// ── Group resolution helper ──────────────────────────────────────────────────────────────

/// Resolve member list for a group audience, by matching the deterministic IRI convention.
///
/// Used by the ACP compiler path: ACP has no `vcard:Group` matcher, so group intents
/// must be expanded to per-member `acp:agent` matchers. This function recreates the
/// member list from the IRI pattern without external state.
fn resolve_group_members(row: &IntentRow, params: &GenParams) -> Vec<String> {
    let Audience::Group(g) = &row.audience else { return vec![]; };
    let nm = members_per_role(params);
    let no = n_orgs(params.sf);

    // Subcontractor group: BASE/groups/subcontractor/{oi}-{oi2}
    let sub_prefix = format!("{BASE}/groups/subcontractor/");
    if let Some(rest) = g.strip_prefix(&sub_prefix) {
        if let Some((a, b)) = rest.split_once('-') {
            if let (Ok(oi), Ok(oi2)) = (a.parse::<u32>(), b.parse::<u32>()) {
                let mut members = Vec::new();
                for mi in 0..2_u32 {
                    if oi < no { members.push(agent_uri(oi, mi)); }
                    if oi2 < no { members.push(agent_uri(oi2, mi)); }
                }
                return members;
            }
        }
    }

    // Role group: BASE/groups/org/{oi}/{role}
    let role_prefix = format!("{BASE}/groups/org/");
    if let Some(rest) = g.strip_prefix(&role_prefix) {
        if let Some((oi_s, _role)) = rest.split_once('/') {
            if let Ok(oi) = oi_s.parse::<u32>() {
                return (0..nm).map(|mi| agent_uri(oi, mi)).collect();
            }
        }
    }

    vec![]
}

// ── Main generator ───────────────────────────────────────────────────────────────────────

/// Generate a U2 commercial-project-management dataset.
///
/// # Invariants
/// - Determinism: same `params` → same output.
/// - All-except intents in `intents` carry per-model [`ExpressibilityEntry`]
///   entries in `expressibility_matrix`.
/// - W3 churn steps have exact expected decision deltas by construction.
///
/// # Panics
/// Panics if `params.validate()` fails.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn generate(params: &GenParams) -> ProjectMgmtDataset {
    params.validate().expect("GenParams must be valid");

    let mut rng = SmallRng::seed_from_u64(params.seed);

    let mut data_nquads: Vec<String> = Vec::new();
    let mut group_memberships: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut intents: Vec<IntentRow> = Vec::new();
    let mut expressibility_matrix: Vec<(usize, ExpressibilityEntry)> = Vec::new();

    let no = n_orgs(params.sf);
    let np = projects_per_org(params.sf);
    let ns = sites_per_project(params.sf);
    let nd = docs_per_site(params.sf);
    let nm = members_per_role(params);

    // ── 1. Orgs and their role groups ──────────────────────────────────────────────────

    for oi in 0..no {
        let org = org_uri(oi);
        data_nquads.push(triple(&org, RDF_TYPE, LDP_BASIC_CONTAINER));
        data_nquads.push(triple(&org, RDF_TYPE, &format!("{PM_VOCAB}{PM_ORG_TYPE}")));
        data_nquads.push(triple_lit(&org, RDFS_LABEL, &format!("\"Org {oi}\"")));

        // Three role groups per org: owner, member, guest.
        // Using deterministic naming so any downstream code can reconstruct the
        // member list purely from the IRI (cf. resolve_group_members).
        for role in ["owner", "member", "guest"] {
            let group = role_group_uri(oi, role);
            data_nquads.push(triple(&group, RDF_TYPE, VCARD_GROUP));
            data_nquads.push(triple_lit(
                &group,
                RDFS_LABEL,
                &format!("\"Org {oi} {role} group\""),
            ));
            for mi in 0..nm {
                let member = if role == "owner" && mi == 0 {
                    owner_uri(oi)
                } else {
                    agent_uri(oi, mi)
                };
                group_memberships.entry(group.clone()).or_default().insert(member.clone());
                data_nquads.push(format!(
                    "{} {} {} .",
                    iri(&group),
                    VCARD_HAS_MEMBER,
                    iri(&member)
                ));
            }
        }
    }

    // ── 2. Subcontractor cross-org groups ──────────────────────────────────────────────

    // For each consecutive org pair, create a shared subcontractor group.
    // The same group IRI is referenced by both orgs' policies → cross-org group reuse.
    for oi in 0..no.saturating_sub(1) {
        let oi2 = oi + 1;
        let sub_group = subcontractor_group_uri(oi, oi2);
        data_nquads.push(triple(&sub_group, RDF_TYPE, VCARD_GROUP));
        data_nquads.push(triple_lit(
            &sub_group,
            RDFS_LABEL,
            &format!("\"Subcontractor group org {oi}↔{oi2}\""),
        ));
        // Two agents from each org (deterministic indices 0, 1).
        for mi in 0..2_u32 {
            group_memberships.entry(sub_group.clone()).or_default()
                .extend([agent_uri(oi, mi), agent_uri(oi2, mi)]);
            data_nquads.push(format!(
                "{} {} {} .",
                iri(&sub_group),
                VCARD_HAS_MEMBER,
                iri(&agent_uri(oi, mi))
            ));
            data_nquads.push(format!(
                "{} {} {} .",
                iri(&sub_group),
                VCARD_HAS_MEMBER,
                iri(&agent_uri(oi2, mi))
            ));
        }
    }

    // ── 3. Projects, sites, documents and their access intents ─────────────────────────

    for oi in 0..no {
        let owner_agent = owner_uri(oi);
        let member_group = role_group_uri(oi, "member");
        let guest_group = role_group_uri(oi, "guest");

        for pi in 0..np {
            let project = project_uri(oi, pi);
            data_nquads.push(triple(&project, RDF_TYPE, LDP_BASIC_CONTAINER));
            data_nquads.push(triple(&project, RDF_TYPE, &format!("{PM_VOCAB}{PM_PROJECT_TYPE}")));
            data_nquads.push(triple(&org_uri(oi), LDP_CONTAINS, &project));

            // Owner: full control on the project subtree.
            intents.push(IntentRow {
                audience: Audience::Agent(owner_agent.clone()),
                scope: Scope::Subtree,
                mode: AccessMode::full(),
                condition: Condition::None,
                effect: Effect::Allow,
                resource_uri: project.clone(),
            });

            // Member group: read on the project subtree (WAC: acl:agentGroup native;
            // ACP: expand to per-member matchers — expressibility matrix entry required).
            let member_intent_idx = intents.len();
            let member_list: Vec<String> = (0..nm).map(|mi| agent_uri(oi, mi)).collect();
            intents.push(IntentRow {
                audience: Audience::Group(member_group.clone()),
                scope: Scope::Subtree,
                mode: AccessMode::read_only(),
                condition: Condition::None,
                effect: Effect::Allow,
                resource_uri: project.clone(),
            });
            // ACP expansion entry for group intent (no-group-matcher → per-member blowup).
            expressibility_matrix.push((
                member_intent_idx,
                ExpressibilityEntry {
                    model: AcModel::Acp,
                    expressibility: Expressibility::Expansion(member_list.len()),
                },
            ));
            // WAC and ODRL: native for group (WAC acl:agentGroup, ODRL PartyCollection).
            expressibility_matrix.push((
                member_intent_idx,
                ExpressibilityEntry {
                    model: AcModel::Wac,
                    expressibility: Expressibility::Native,
                },
            ));
            expressibility_matrix.push((
                member_intent_idx,
                ExpressibilityEntry {
                    model: AcModel::Odrl,
                    expressibility: Expressibility::Native,
                },
            ));

            // Guest group: read-only on the project subtree.
            let guest_intent_idx = intents.len();
            intents.push(IntentRow {
                audience: Audience::Group(guest_group.clone()),
                scope: Scope::Subtree,
                mode: AccessMode::read_only(),
                condition: Condition::None,
                effect: Effect::Allow,
                resource_uri: project.clone(),
            });
            // ACP expansion for guest group.
            expressibility_matrix.push((
                guest_intent_idx,
                ExpressibilityEntry {
                    model: AcModel::Acp,
                    expressibility: Expressibility::Expansion(member_list.len()),
                },
            ));

            // Subcontractor group: read on this project (cross-org group reuse).
            // Only add for orgs that have a successor (i.e., not the last org).
            if oi + 1 < no {
                let sub_group = subcontractor_group_uri(oi, oi + 1);
                let sub_members: Vec<String> = (0..2)
                    .flat_map(|mi: u32| [agent_uri(oi, mi), agent_uri(oi + 1, mi)])
                    .collect();
                let sub_intent_idx = intents.len();
                intents.push(IntentRow {
                    audience: Audience::Group(sub_group),
                    scope: Scope::Subtree,
                    mode: AccessMode::read_only(),
                    condition: Condition::None,
                    effect: Effect::Allow,
                    resource_uri: project.clone(),
                });
                expressibility_matrix.push((
                    sub_intent_idx,
                    ExpressibilityEntry {
                        model: AcModel::Acp,
                        expressibility: Expressibility::Expansion(sub_members.len()),
                    },
                ));
            }

            // ── All-except intent (first project only per org) ──────────────────────
            //
            // Deny all agents EXCEPT the org members AND the org owner from reading the
            // first project. This exercises the key U2 expressibility asymmetry:
            //   - ACP:  acp:deny native (deny-wins semantics).
            //   - ODRL: Prohibition native.
            //   - WAC:  no deny → only expressible as bounded enumerated allow (blowup),
            //           and only if the exclusion set is finite.
            //
            // Exclusion set includes both the owner and all member agents so that
            // the org's principals are not inadvertently blocked by the deny.
            if pi == 0 {
                // Exclusion set = the org owner + member agents (deterministic).
                let mut excl: Vec<String> = vec![owner_agent.clone()];
                excl.extend((0..nm).map(|mi| agent_uri(oi, mi)));
                let ae_idx = intents.len();
                intents.push(IntentRow {
                    audience: Audience::AllExcept(excl.clone()),
                    scope: Scope::Resource,
                    mode: AccessMode::read_only(),
                    condition: Condition::None,
                    effect: Effect::Deny,
                    resource_uri: project.clone(),
                });
                // ACP: native (acp:deny)
                expressibility_matrix.push((
                    ae_idx,
                    ExpressibilityEntry {
                        model: AcModel::Acp,
                        expressibility: Expressibility::Native,
                    },
                ));
                // WAC: no deny → bounded enumerated allow or Unsupported if empty.
                let wac_exp = if excl.is_empty() {
                    Expressibility::Unsupported
                } else {
                    Expressibility::Expansion(excl.len())
                };
                expressibility_matrix.push((
                    ae_idx,
                    ExpressibilityEntry {
                        model: AcModel::Wac,
                        expressibility: wac_exp,
                    },
                ));
                // ODRL: native (Prohibition)
                expressibility_matrix.push((
                    ae_idx,
                    ExpressibilityEntry {
                        model: AcModel::Odrl,
                        expressibility: Expressibility::Native,
                    },
                ));
            }

            // ── Sites and documents ────────────────────────────────────────────────────

            for si in 0..ns {
                let site = site_uri(oi, pi, si);
                data_nquads.push(triple(&site, RDF_TYPE, LDP_BASIC_CONTAINER));
                data_nquads.push(triple(&site, RDF_TYPE, &format!("{PM_VOCAB}{PM_SITE_TYPE}")));
                data_nquads.push(triple(&project, LDP_CONTAINS, &site));

                for di in 0..nd {
                    let doc = doc_uri(oi, pi, si, di);
                    data_nquads.push(triple(&doc, RDF_TYPE, FOAF_DOCUMENT));
                    data_nquads.push(triple(&site, LDP_CONTAINS, &doc));

                    // Audience mix for this document (deterministic via PRNG).
                    let is_public = rand_bool(&mut rng, params.mix.public);
                    let is_shared = !is_public && rand_bool(&mut rng, params.mix.shared);

                    if is_public {
                        // Public read: any agent.
                        intents.push(IntentRow {
                            audience: Audience::Public,
                            scope: Scope::Resource,
                            mode: AccessMode::read_only(),
                            condition: Condition::None,
                            effect: Effect::Allow,
                            resource_uri: doc.clone(),
                        });
                    } else if is_shared {
                        // Shared: member group read on this specific document.
                        // ACP: must expand group to per-member matchers (no group-matcher).
                        let shared_intent_idx = intents.len();
                        let doc_member_list: Vec<String> =
                            (0..nm).map(|mi| agent_uri(oi, mi)).collect();
                        intents.push(IntentRow {
                            audience: Audience::Group(member_group.clone()),
                            scope: Scope::Resource,
                            mode: AccessMode::read_only(),
                            condition: Condition::None,
                            effect: Effect::Allow,
                            resource_uri: doc.clone(),
                        });
                        // ACP expansion entry for this doc-level group intent.
                        expressibility_matrix.push((
                            shared_intent_idx,
                            ExpressibilityEntry {
                                model: AcModel::Acp,
                                expressibility: Expressibility::Expansion(
                                    doc_member_list.len(),
                                ),
                            },
                        ));
                    }
                    // Private: no additional per-doc intent; project-level subtree covers owner.

                    // policies_per_resource: accrete additional read intents (XACBench dimension).
                    for _extra in 1..u32::from(params.policies_per_resource) {
                        let mi = next_u32_bounded(&mut rng, nm);
                        intents.push(IntentRow {
                            audience: Audience::Agent(agent_uri(oi, mi)),
                            scope: Scope::Resource,
                            mode: AccessMode::read_only(),
                            condition: Condition::None,
                            effect: Effect::Allow,
                            resource_uri: doc.clone(),
                        });
                    }
                }
            }
        }
    }

    // ── 4. Compile intents to per-model policy graphs ───────────────────────────────────

    let wac_policy: Vec<CompiledPolicy> = intents.iter().map(compile_wac).collect();

    let acp_policy: Vec<CompiledPolicy> = intents
        .iter()
        .map(|row| {
            let members = resolve_group_members(row, params);
            compile_acp(row, &members)
        })
        .collect();

    let odrl_policy: Vec<CompiledPolicy> = intents.iter().map(compile_odrl).collect();

    // ── 5. Expected decisions (by-construction oracle) ──────────────────────────────────

    let request_tuples = build_request_tuples(params, no, np, ns, nd);
    let mut expected_decisions: Vec<ExpectedDecision> = Vec::new();
    for req in &request_tuples {
        for (model, decision) in [
            (AcModel::Wac, oracle_wac(req, &intents)),
            (AcModel::Acp, oracle_acp(req, &intents)),
            (AcModel::Odrl, oracle_odrl(req, &intents)),
        ] {
            expected_decisions.push(ExpectedDecision {
                request: req.clone(),
                model,
                decision,
            });
        }
    }

    // ── 6. W2 query fixtures ─────────────────────────────────────────────────────────────

    let queries = build_query_fixtures(no, np, ns, nd, &intents);

    // ── 7. W3 churn steps ────────────────────────────────────────────────────────────────

    let churn_steps = build_churn_steps(no, np, &intents);

    ProjectMgmtDataset {
        group_memberships,
        data_nquads,
        wac_policy,
        acp_policy,
        odrl_policy,
        intents,
        expected_decisions,
        queries,
        churn_steps,
        expressibility_matrix,
    }
}

// ── Request tuple generation ─────────────────────────────────────────────────────────────

/// Build the W1 request population for this corpus.
///
/// Covers: org owners, org members (indices 0 and 1), subcontractors, an unknown external
/// agent (expect deny — fail-closed), and per-document reads. Request count is bounded by
/// `params.n_agents` to avoid blowing up at large SF.
fn build_request_tuples(
    params: &GenParams,
    no: u32,
    np: u32,
    ns: u32,
    nd: u32,
) -> Vec<Request> {
    let mut tuples: Vec<Request> = Vec::new();
    let max_orgs = no.min(params.n_agents.max(1));
    let max_projects = np.min(2);
    let max_sites = ns.min(1);
    let max_docs = nd.min(3);

    for oi in 0..max_orgs {
        let project0 = project_uri(oi, 0);

        // Owner reads project 0
        tuples.push(Request {
            agent: owner_uri(oi),
            client: None,
            resource: project0.clone(),
            mode: AccessMode::read_only(),
        });

        // Member reads project 0
        tuples.push(Request {
            agent: agent_uri(oi, 1),
            client: None,
            resource: project0.clone(),
            mode: AccessMode::read_only(),
        });

        // Member attempts write on project 0 (expect Deny — only read granted to members)
        tuples.push(Request {
            agent: agent_uri(oi, 1),
            client: None,
            resource: project0.clone(),
            mode: AccessMode { read: false, write: true, control: false },
        });

        // Unknown external agent reads project 0 (expect Deny — fail-closed)
        tuples.push(Request {
            agent: format!("{BASE}/agents/external/unknown"),
            client: None,
            resource: project0.clone(),
            mode: AccessMode::read_only(),
        });

        // Subcontractor from adjacent org reads project 0 (cross-org group reuse)
        if oi + 1 < no {
            tuples.push(Request {
                agent: agent_uri(oi + 1, 0),
                client: None,
                resource: project0.clone(),
                mode: AccessMode::read_only(),
            });
        }

        // Per-document requests within the first few projects/sites.
        for pi in 0..max_projects {
            for si in 0..max_sites {
                for di in 0..max_docs {
                    let doc = doc_uri(oi, pi, si, di);
                    // Member reads doc
                    tuples.push(Request {
                        agent: agent_uri(oi, 1),
                        client: None,
                        resource: doc.clone(),
                        mode: AccessMode::read_only(),
                    });
                    // Owner reads doc
                    tuples.push(Request {
                        agent: owner_uri(oi),
                        client: None,
                        resource: doc.clone(),
                        mode: AccessMode::read_only(),
                    });
                }
            }
        }
    }

    tuples
}

// ── W2 query fixtures ────────────────────────────────────────────────────────────────────

/// Build W2 query fixtures (Q-point, Q-scan, Q-join, Q-agg).
///
/// All expected result sets are derived from the intent table by the same
/// by-construction oracle used for decisions — no sparq evaluator is consulted.
#[allow(clippy::too_many_lines)]
fn build_query_fixtures(
    no: u32,
    np: u32,
    _ns: u32,
    nd: u32,
    intents: &[IntentRow],
) -> Vec<QueryFixture> {
    if no == 0 || np == 0 {
        return vec![];
    }

    let oi = 0u32;
    let pi = 0u32;
    let project = project_uri(oi, pi);
    let site = site_uri(oi, pi, 0);
    let owner = owner_uri(oi);
    let member = agent_uri(oi, 1);

    // Q-point: owner looks up a known project container.
    let owner_req = Request {
        agent: owner.clone(),
        client: None,
        resource: project.clone(),
        mode: AccessMode::read_only(),
    };
    let owner_allowed = oracle_wac(&owner_req, intents) == Decision::Allow;
    let qpoint = QueryFixture {
        class: QueryClass::Point,
        sparql: format!("SELECT ?s ?p ?o WHERE {{ GRAPH <{project}> {{ ?s ?p ?o }} }}"),
        expected_rows: if owner_allowed {
            vec![format!("GRAPH <{project}>")]
        } else {
            vec![]
        },
        agent: owner.clone(),
        model: AcModel::Wac,
    };

    // Q-scan: member lists all accessible documents in site 0.
    let mut scan_rows: Vec<String> = (0..nd)
        .filter_map(|di| {
            let doc = doc_uri(oi, pi, 0, di);
            let req = Request {
                agent: member.clone(),
                client: None,
                resource: doc.clone(),
                mode: AccessMode::read_only(),
            };
            if oracle_wac(&req, intents) == Decision::Allow {
                Some(format!("<{doc}>"))
            } else {
                None
            }
        })
        .collect();
    scan_rows.sort();
    let qscan = QueryFixture {
        class: QueryClass::Scan,
        sparql: format!(
            "SELECT ?doc WHERE {{ <{site}> <http://www.w3.org/ns/ldp#contains> ?doc }}"
        ),
        expected_rows: scan_rows,
        agent: member.clone(),
        model: AcModel::Wac,
    };

    // Q-join: owner retrieves all accessible projects in their org.
    let mut join_rows: Vec<String> = (0..np)
        .filter_map(|p2| {
            let proj = project_uri(oi, p2);
            let req = Request {
                agent: owner.clone(),
                client: None,
                resource: proj.clone(),
                mode: AccessMode::read_only(),
            };
            if oracle_wac(&req, intents) == Decision::Allow {
                Some(format!("<{proj}>"))
            } else {
                None
            }
        })
        .collect();
    join_rows.sort();
    let qjoin = QueryFixture {
        class: QueryClass::Join,
        sparql: format!(
            "SELECT ?proj WHERE {{ \
             <{org}> <http://www.w3.org/ns/ldp#contains> ?proj . \
             ?proj <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> \
             <{PM_VOCAB}Project> \
             }}",
            org = org_uri(oi),
        ),
        expected_rows: join_rows,
        agent: owner.clone(),
        model: AcModel::Wac,
    };

    // Q-agg: member counts accessible documents in site 0.
    let agg_count = (0..nd)
        .filter(|&di| {
            let doc = doc_uri(oi, pi, 0, di);
            let req = Request {
                agent: member.clone(),
                client: None,
                resource: doc,
                mode: AccessMode::read_only(),
            };
            oracle_wac(&req, intents) == Decision::Allow
        })
        .count();
    let qagg = QueryFixture {
        class: QueryClass::Aggregate,
        sparql: format!(
            "SELECT (COUNT(?doc) AS ?n) WHERE {{ \
             <{site}> <http://www.w3.org/ns/ldp#contains> ?doc \
             }}"
        ),
        expected_rows: vec![format!(
            "\"{agg_count}\"^^<http://www.w3.org/2001/XMLSchema#integer>"
        )],
        agent: member.clone(),
        model: AcModel::Wac,
    };

    vec![qpoint, qscan, qjoin, qagg]
}

// ── W3 churn steps ───────────────────────────────────────────────────────────────────────

/// Build W3 churn steps: a grant → revoke cycle with exact decision deltas.
///
/// Uses org 0, project 1 (if available) as the target. The new agent is identified
/// by a fixed IRI so the step is deterministic regardless of SF.
fn build_churn_steps(no: u32, np: u32, intents: &[IntentRow]) -> Vec<ChurnStep> {
    if no == 0 || np == 0 {
        return vec![];
    }

    let oi = 0u32;
    let pi = 1_u32.min(np.saturating_sub(1));
    let project = project_uri(oi, pi);
    let new_agent = format!("{BASE}/agents/org/{oi}/churn-agent");

    let churn_req = Request {
        agent: new_agent.clone(),
        client: None,
        resource: project.clone(),
        mode: AccessMode::read_only(),
    };

    // Pre-grant decisions (by-construction oracle, no sparq).
    let pre = [
        (AcModel::Wac, oracle_wac(&churn_req, intents)),
        (AcModel::Acp, oracle_acp(&churn_req, intents)),
        (AcModel::Odrl, oracle_odrl(&churn_req, intents)),
    ];

    // Post-grant intent table: add a direct allow for the new agent.
    let grant_intent = IntentRow {
        audience: Audience::Agent(new_agent.clone()),
        scope: Scope::Resource,
        mode: AccessMode::read_only(),
        condition: Condition::None,
        effect: Effect::Allow,
        resource_uri: project.clone(),
    };
    let post_intents: Vec<IntentRow> = intents
        .iter()
        .cloned()
        .chain(std::iter::once(grant_intent))
        .collect();
    let post = [
        (AcModel::Wac, oracle_wac(&churn_req, &post_intents)),
        (AcModel::Acp, oracle_acp(&churn_req, &post_intents)),
        (AcModel::Odrl, oracle_odrl(&churn_req, &post_intents)),
    ];

    // N-Quads delta for the WAC grant.
    let auth_id = format!("{project}#churn-wac-auth");
    let acl = "http://www.w3.org/ns/auth/acl#";
    let delta_add = vec![
        format!("<{auth_id}> <{acl}type> <{acl}Authorization> ."),
        format!("<{auth_id}> <{acl}accessTo> <{project}> ."),
        format!("<{auth_id}> <{acl}mode> <{acl}Read> ."),
        format!("<{auth_id}> <{acl}agent> <{new_agent}> ."),
    ];
    let delta_remove = delta_add.clone();

    vec![
        ChurnStep {
            description: format!("Grant read on <{project}> to <{new_agent}>"),
            delta_add,
            delta_remove: vec![],
            expected_deltas: post
                .into_iter()
                .map(|(model, decision)| ExpectedDecision {
                    request: churn_req.clone(),
                    model,
                    decision,
                })
                .collect(),
        },
        ChurnStep {
            description: format!("Revoke read on <{project}> from <{new_agent}>"),
            delta_add: vec![],
            delta_remove,
            expected_deltas: pre
                .into_iter()
                .map(|(model, decision)| ExpectedDecision {
                    request: churn_req.clone(),
                    model,
                    decision,
                })
                .collect(),
        },
    ]
}
