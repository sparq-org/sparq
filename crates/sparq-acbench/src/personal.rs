//! U1 — Personal data storage generator (bead `sq-i6du2.2`).
//!
//! Generates a Solid pod-shaped dataset with owner-centric access control,
//! deep container inheritance, friend/family group nesting, and app-restricted
//! access (ACP `acp:client`; WAC `acl:origin` approximation).
//!
//! # AC shape stressed
//! - Container inheritance depth (driven by [`GenParams::container_depth`]).
//! - Owner-centric: most resources are owned by one agent, with shared sub-trees.
//! - Friend/family groups: `vcard:Group` chains at depth ≤ [`GenParams::group_nesting_depth`].
//! - Public/private/shared mix driven by [`GenParams::mix`].
//! - App-restricted: a fraction of shared resources carry `acp:client` / `acl:origin`.
//!
//! # Canonical WAC use case
//! A personal Solid pod is the canonical WAC use case (WAC was designed for this):
//! owner-private by default, selective sharing with named agents/groups, public
//! profile subset, with selective app-restriction.
//!
//! # Oracle by construction
//! Expected decisions are computed directly from the intent table using the
//! procedural oracle in `oracle.rs`. The oracle is **structurally independent** of
//! any sparq evaluator — it reads only the intent table. Fail-closed: any request
//! for an unlisted resource returns [`Decision::Deny`].
//!
//! # Expressibility matrix entries
//! - [`Audience::ClientRestricted`] → WAC: [`Expressibility::Approximation`] (via
//!   `acl:origin`), ACP: [`Expressibility::Native`] (via `acp:client`), ODRL:
//!   [`Expressibility::Native`].
//! - Conditions ([`Condition::None`] only in U1) → all three models: `Native`.
//! - [`Audience::Group`] → WAC: `Native` (via `acl:agentGroup`), ACP:
//!   [`Expressibility::Expansion`]`(n)` (per-member matchers, n = group size),
//!   ODRL: `Native` (via `odrl:PartyCollection`).
//!
//! # File ownership
//! **Only bead `sq-i6du2.2` edits this file.**
//! All other beads in the `sq-i6du2` family leave it untouched.

use std::collections::{BTreeMap, BTreeSet};

use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};

use crate::{
    compile_acp, compile_odrl, compile_wac, oracle_acp, oracle_odrl, oracle_wac, AcModel,
    AccessMode, Audience, CompiledPolicy, Condition, Decision, Effect, ExpectedDecision,
    Expressibility, GenParams, IntentRow, QueryClass, QueryFixture, Request, Scope,
};

// ── Pod resource categories ─────────────────────────────────────────────────────────

/// A single resource in the generated pod.
#[derive(Debug, Clone)]
struct PodResource {
    /// Absolute URI of this resource.
    uri: String,
    /// The category determines default access audience.
    category: ResourceCategory,
    /// Index of the container this resource lives in (for inheritance).
    container_idx: usize,
}

/// High-level access category for a resource.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ResourceCategory {
    /// Public subset of the profile (foaf:Person metadata).
    PublicProfile,
    /// Private resource (owner-only access).
    Private,
    /// Shared with a named group or specific agents.
    Shared,
    /// App-restricted: shared but only via a specific client app.
    AppRestricted,
}

/// A `vcard:Group` node in the generated group closure graph.
#[derive(Debug, Clone)]
struct Group {
    /// IRI of this group.
    uri: String,
    /// Resolved member agent IRIs (leaf-level).
    members: Vec<String>,
    /// Parent group URI (for nesting chain), if any.
    parent: Option<String>,
}

// ── Output type ──────────────────────────────────────────────────────────────────────

/// Output of the U1 personal-data-storage generator.
///
/// All fields are deterministic for a given [`GenParams`] (same seed, same output).
pub struct PersonalDataset {
    /// Group IRIs mapped to their resolved agent members, including nested membership.
    pub group_memberships: BTreeMap<String, BTreeSet<String>>,
    /// N-Quads lines forming the data graph (resources, containers, metadata).
    pub data_nquads: Vec<String>,
    /// Compiled WAC policy graph.
    pub wac_policy: Vec<CompiledPolicy>,
    /// Compiled ACP policy graph.
    pub acp_policy: Vec<CompiledPolicy>,
    /// Compiled ODRL policy graph.
    pub odrl_policy: Vec<CompiledPolicy>,
    /// The model-agnostic intent table (one row per access-control decision point).
    pub intents: Vec<IntentRow>,
    /// Request tuples with expected decisions for W1 / W3 oracle checking.
    pub expected_decisions: Vec<ExpectedDecision>,
    /// W2 SPARQL queries with expected result sets (closed-form, as N-Quads row sets).
    pub queries: Vec<QueryFixture>,
}

// ── Generator ────────────────────────────────────────────────────────────────────────

/// Generate a U1 personal-data-storage dataset.
///
/// # Dataset structure
/// The generator produces:
/// - One Solid pod with a root container and nested sub-containers (depth driven by
///   `GenParams::container_depth`).
/// - Resources distributed across public-profile, private, shared, and app-restricted
///   categories according to `GenParams::mix`.
/// - Friend/family groups with nesting up to `GenParams::group_nesting_depth`.
/// - Per-model policy graphs (WAC, ACP, ODRL) compiled from the intent table.
/// - A full set of expected decisions (owner, friend-group member, public, unknown).
/// - W2 query fixtures (Q-point, Q-scan, Q-join, Q-agg) with closed-form expected results.
///
/// # Invariants
/// - **Determinism**: same `params` → same `PersonalDataset` every call.
/// - **Fail-closed oracle**: every `ExpectedDecision` with no matching allow rule
///   has `decision = Deny`.
/// - **Independent oracle**: expected decisions are computed from the intent table
///   without calling any sparq evaluator.
///
/// # Panics
/// Panics if `params.validate()` fails.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn generate(params: &GenParams) -> PersonalDataset {
    params.validate().expect("GenParams validation failed");

    // Deterministic PRNG seeded from params.seed.
    let mut rng = SmallRng::seed_from_u64(params.seed);

    // ── Pod and agent URIs ────────────────────────────────────────────────────────
    let pod_base = format!("https://pod.example/user{}/", params.seed % 1000);
    let owner_uri = format!("{pod_base}profile/card#me");

    // Scale: resources_per_container × container count, floored at 1.
    let n_containers = base_containers(params.container_depth);
    let resources_per_container = (params.sf as usize).max(1) * 5;
    let total_resources = n_containers * resources_per_container;

    // ── Agent pool ────────────────────────────────────────────────────────────────
    let n_agents = (params.n_agents as usize).max(4);
    let agents: Vec<String> = (0..n_agents)
        .map(|i| format!("https://pod.example/agent{i}/profile/card#me"))
        .collect();

    // ── Group pool (friend/family groups with nesting) ────────────────────────────
    let groups = build_groups(
        &pod_base,
        params.group_nesting_depth,
        params.members_per_group,
        &agents,
        &mut rng,
    );

    // ── Container tree ────────────────────────────────────────────────────────────
    let containers = build_containers(&pod_base, params.container_depth);

    // ── Resources: assign categories ─────────────────────────────────────────────
    let mix = &params.mix;
    // Fraction of shared resources that are also app-restricted.
    let app_restricted_fraction = 0.25_f32;

    let resources: Vec<PodResource> = (0..total_resources)
        .map(|i| {
            let container_idx = i / resources_per_container;
            let roll: f32 = rng.gen();
            let category = if roll < mix.public {
                ResourceCategory::PublicProfile
            } else if roll < mix.public + mix.private {
                ResourceCategory::Private
            } else {
                // shared bucket: sub-divide into app-restricted and plain shared
                let roll2: f32 = rng.gen();
                if roll2 < app_restricted_fraction {
                    ResourceCategory::AppRestricted
                } else {
                    ResourceCategory::Shared
                }
            };

            let container_uri = &containers[container_idx];
            let uri = format!("{container_uri}resource{i}");
            PodResource { uri, category, container_idx }
        })
        .collect();

    // Choose a client-app URI for app-restricted resources (fixed per generator run).
    let app_uri = format!("https://app.example/solidapp-{}", params.seed % 100);

    // Pick one "friend group" for shared resources (the first group's URI).
    let friend_group_uri = groups.first().map_or_else(
        || format!("{pod_base}groups/friends"),
        |g| g.uri.clone(),
    );

    // ── Intent table ─────────────────────────────────────────────────────────────
    // Invariant: the FIRST intent for each resource is an owner-full-control entry
    // (owner-centric Solid pod shape). Additional intents vary by category.
    let mut intents: Vec<IntentRow> = Vec::new();

    // Subtree-level owner grant at pod root (covers inheritance for private resources).
    intents.push(IntentRow {
        audience: Audience::Agent(owner_uri.clone()),
        scope: Scope::Subtree,
        mode: AccessMode::full(),
        condition: Condition::None,
        effect: Effect::Allow,
        resource_uri: pod_base.clone(),
    });

    // Public profile container: subtree grant to Public.
    let profile_container = format!("{pod_base}profile/");
    intents.push(IntentRow {
        audience: Audience::Public,
        scope: Scope::Subtree,
        mode: AccessMode::read_only(),
        condition: Condition::None,
        effect: Effect::Allow,
        resource_uri: profile_container,
    });

    // Per-resource intents.
    for resource in &resources {
        match resource.category {
            ResourceCategory::PublicProfile => {
                // Already covered by the profile-container subtree intent; emit an
                // explicit per-resource intent for clarity and for narrower scoped
                // oracle lookup.
                intents.push(IntentRow {
                    audience: Audience::Public,
                    scope: Scope::Resource,
                    mode: AccessMode::read_only(),
                    condition: Condition::None,
                    effect: Effect::Allow,
                    resource_uri: resource.uri.clone(),
                });
            }
            ResourceCategory::Private => {
                // Private: owner-only (covered by pod-root subtree intent above;
                // emit explicit resource-level for precision in the oracle).
                intents.push(IntentRow {
                    audience: Audience::Agent(owner_uri.clone()),
                    scope: Scope::Resource,
                    mode: AccessMode::full(),
                    condition: Condition::None,
                    effect: Effect::Allow,
                    resource_uri: resource.uri.clone(),
                });
            }
            ResourceCategory::Shared => {
                // Shared with the friend group.
                intents.push(IntentRow {
                    audience: Audience::Group(friend_group_uri.clone()),
                    scope: Scope::Resource,
                    mode: AccessMode::read_only(),
                    condition: Condition::None,
                    effect: Effect::Allow,
                    resource_uri: resource.uri.clone(),
                });
            }
            ResourceCategory::AppRestricted => {
                // App-restricted: one friend, only via the specific client app.
                // Pick the first agent as the restricted agent.
                let restricted_agent = agents[0].clone();
                intents.push(IntentRow {
                    audience: Audience::ClientRestricted {
                        agent: restricted_agent,
                        client: app_uri.clone(),
                    },
                    scope: Scope::Resource,
                    mode: AccessMode::read_only(),
                    condition: Condition::None,
                    effect: Effect::Allow,
                    resource_uri: resource.uri.clone(),
                });
            }
        }
    }

    // Resolve the nesting chain from leaves to parents; export agent members only.
    let mut group_memberships: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for group in groups.iter().rev() {
        let members = group_memberships.entry(group.uri.clone()).or_default();
        members.extend(group.members.iter().cloned());
        let members = members.clone();
        if let Some(parent) = &group.parent {
            group_memberships.entry(parent.clone()).or_default().extend(members);
        }
    }

    // ── Group closure map ─────────────────────────────────────────────────────────
    // Build a closure: set of agents allowed by group membership (for ACP expansion).
    let friend_group_members: Vec<String> = groups
        .first()
        .map(|g| g.members.clone())
        .unwrap_or_default();

    // ── Compile per-model policies ────────────────────────────────────────────────
    let wac_policy: Vec<CompiledPolicy> = intents.iter().map(compile_wac).collect();

    let acp_policy: Vec<CompiledPolicy> = intents
        .iter()
        .map(|row| compile_acp(row, &friend_group_members))
        .collect();

    let odrl_policy: Vec<CompiledPolicy> = intents.iter().map(compile_odrl).collect();

    // ── N-Quads data graph ────────────────────────────────────────────────────────
    let data_nquads = build_data_nquads(&pod_base, &owner_uri, &containers, &resources, &groups);

    // ── Expected decisions ────────────────────────────────────────────────────────
    // For each resource, generate W1 request tuples for:
    //   - owner (should always be Allow)
    //   - first agent (agent0, may be in friend group → Shared/AppRestricted might Allow)
    //   - unknown agent (not in any group → Deny unless Public)
    //   - public (no agent IRI → Allow only for Public resources)
    let unknown_agent = "https://pod.example/unknown/profile/card#me";
    let friend_agent = agents.first().cloned().unwrap_or_default();

    let mut expected_decisions: Vec<ExpectedDecision> = Vec::new();

    for resource in &resources {
        let req_mode = AccessMode::read_only();

        let owner_req = Request {
            agent: owner_uri.clone(),
            client: None,
            resource: resource.uri.clone(),
            mode: req_mode.clone(),
        };
        let friend_req = Request {
            agent: friend_agent.clone(),
            client: None,
            resource: resource.uri.clone(),
            mode: req_mode.clone(),
        };
        let app_req = Request {
            agent: friend_agent.clone(),
            client: Some(app_uri.clone()),
            resource: resource.uri.clone(),
            mode: req_mode.clone(),
        };
        let unknown_req = Request {
            agent: unknown_agent.to_string(),
            client: None,
            resource: resource.uri.clone(),
            mode: req_mode.clone(),
        };
        let public_req = Request {
            agent: String::new(), // unauthenticated
            client: None,
            resource: resource.uri.clone(),
            mode: req_mode.clone(),
        };

        for model in [AcModel::Wac, AcModel::Acp, AcModel::Odrl] {
            let evaluate = |req: &Request| -> Decision {
                match model {
                    AcModel::Wac => oracle_wac(req, &intents),
                    AcModel::Acp => oracle_acp(req, &intents),
                    AcModel::Odrl => oracle_odrl(req, &intents),
                }
            };

            expected_decisions.push(ExpectedDecision {
                request: owner_req.clone(),
                model: model.clone(),
                decision: evaluate(&owner_req),
            });
            expected_decisions.push(ExpectedDecision {
                request: friend_req.clone(),
                model: model.clone(),
                decision: evaluate(&friend_req),
            });
            expected_decisions.push(ExpectedDecision {
                request: app_req.clone(),
                model: model.clone(),
                decision: evaluate(&app_req),
            });
            expected_decisions.push(ExpectedDecision {
                request: unknown_req.clone(),
                model: model.clone(),
                decision: evaluate(&unknown_req),
            });
            expected_decisions.push(ExpectedDecision {
                request: public_req.clone(),
                model: model.clone(),
                decision: evaluate(&public_req),
            });
        }
    }

    // ── W2 query fixtures ─────────────────────────────────────────────────────────
    let queries = build_query_fixtures(&resources, &owner_uri, &friend_agent, &intents);

    PersonalDataset {
        group_memberships,
        data_nquads,
        wac_policy,
        acp_policy,
        odrl_policy,
        intents,
        expected_decisions,
        queries,
    }
}

// ── Internal helpers ─────────────────────────────────────────────────────────────────

/// Compute the number of containers for a given `container_depth`.
///
/// At depth 0: 1 container (the pod root).
/// At depth d: binary-tree layout gives `2^(d+1) - 1` containers.
/// Capped at 64 to keep SF=1 corpus sizes reasonable.
fn base_containers(depth: u8) -> usize {
    let n = (1_usize << (depth + 1)).saturating_sub(1);
    n.clamp(1, 64)
}

/// Build the list of container URIs (BFS order, binary tree layout).
fn build_containers(pod_base: &str, depth: u8) -> Vec<String> {
    let n = base_containers(depth);
    let mut containers: Vec<String> = Vec::with_capacity(n);
    containers.push(pod_base.to_string()); // root
    for i in 1..n {
        // Parent index in BFS-ordered binary tree: (i-1)/2.
        let parent = containers[(i - 1) / 2].clone();
        let child_name = if i % 2 == 1 { "a/" } else { "b/" };
        containers.push(format!("{parent}{child_name}"));
    }
    containers
}

/// Build friend/family groups with nesting.
///
/// Groups form a linear nesting chain: group0 → group1 → … → group(depth-1).
/// Leaf groups have `members_per_group` members drawn from the agent pool.
/// Each parent group contains the child group's members (flattened for oracle use).
///
/// # Determinism
/// All agent indices are computed arithmetically from group index × `members_per_group`;
/// `_rng` is accepted to reserve the parameter slot for future shape randomisation
/// without breaking the function signature.
fn build_groups(
    pod_base: &str,
    nesting_depth: u8,
    members_per_group: u32,
    agents: &[String],
    _rng: &mut SmallRng,
) -> Vec<Group> {
    let depth = (nesting_depth as usize).max(1);
    let mpg = (members_per_group as usize).max(1);
    let mut groups: Vec<Group> = Vec::new();

    for level in 0..depth {
        let group_uri = format!("{pod_base}groups/level{level}/");
        // Members at each level are consecutive agents starting at level * mpg.
        let members: Vec<String> = (0..mpg)
            .map(|j| {
                if agents.is_empty() {
                    format!("https://pod.example/member{}/profile/card#me", level * mpg + j)
                } else {
                    let agent_idx = (level * mpg + j) % agents.len();
                    agents[agent_idx].clone()
                }
            })
            .collect();

        let parent = if level > 0 {
            Some(groups[level - 1].uri.clone())
        } else {
            None
        };

        groups.push(Group { uri: group_uri, members, parent });
    }

    groups
}

/// Build N-Quads for the data graph (resources, containers, groups, profile metadata).
fn build_data_nquads(
    pod_base: &str,
    owner_uri: &str,
    containers: &[String],
    resources: &[PodResource],
    groups: &[Group],
) -> Vec<String> {
    let mut nquads: Vec<String> = Vec::new();
    let graph = format!("<{pod_base}data>");

    // Pod root type
    nquads.push(format!(
        "<{pod_base}> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/ldp#BasicContainer> {graph} ."
    ));
    // Pod owner
    nquads.push(format!(
        "<{pod_base}> <http://www.w3.org/ns/solid/terms#owner> <{owner_uri}> {graph} ."
    ));

    // Containers: type + containment chain
    for (i, c_uri) in containers.iter().enumerate() {
        nquads.push(format!(
            "<{c_uri}> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/ldp#BasicContainer> {graph} ."
        ));
        if i > 0 {
            let parent_uri = &containers[(i - 1) / 2];
            nquads.push(format!(
                "<{parent_uri}> <http://www.w3.org/ns/ldp#contains> <{c_uri}> {graph} ."
            ));
        }
    }

    // Resources: type + containment + category metadata
    for resource in resources {
        let container_uri = &containers[resource.container_idx];
        nquads.push(format!(
            "<{}> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/ldp#Resource> {graph} .",
            resource.uri
        ));
        nquads.push(format!(
            "<{container_uri}> <http://www.w3.org/ns/ldp#contains> <{}> {graph} .",
            resource.uri
        ));
        let cat_iri = category_iri(&resource.category);
        nquads.push(format!(
            "<{}> <https://sparq.dev/vocab/bench#category> <{cat_iri}> {graph} .",
            resource.uri
        ));
    }

    // Groups: vcard:Group + vcard:hasMember
    for group in groups {
        nquads.push(format!(
            "<{}> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/2006/vcard/ns#Group> {graph} .",
            group.uri
        ));
        if let Some(ref parent) = group.parent {
            nquads.push(format!(
                "<{parent}> <http://www.w3.org/2006/vcard/ns#hasMember> <{}> {graph} .",
                group.uri
            ));
        }
        for member in &group.members {
            nquads.push(format!(
                "<{}> <http://www.w3.org/2006/vcard/ns#hasMember> <{member}> {graph} .",
                group.uri
            ));
        }
    }

    // Sort for determinism.
    nquads.sort();
    nquads
}

/// Return the category benchmark IRI for a [`ResourceCategory`].
fn category_iri(cat: &ResourceCategory) -> &'static str {
    match cat {
        ResourceCategory::PublicProfile => "https://sparq.dev/vocab/bench#PublicProfile",
        ResourceCategory::Private => "https://sparq.dev/vocab/bench#Private",
        ResourceCategory::Shared => "https://sparq.dev/vocab/bench#Shared",
        ResourceCategory::AppRestricted => "https://sparq.dev/vocab/bench#AppRestricted",
    }
}

/// Build W2 query fixtures with closed-form expected result sets.
///
/// Produces four query classes:
/// - Q-point: explicit `GRAPH` lookup for a single known resource (owner can read).
/// - Q-scan: list of resources owner can read in the pod root.
/// - Q-join: list of resources the first friend agent can read.
/// - Q-agg: COUNT of resources owner can read (WAC model).
///
/// All expected results are computed from the intent table by the same procedural
/// oracle used for W1 — no sparq evaluator is called.
fn build_query_fixtures(
    resources: &[PodResource],
    owner_uri: &str,
    friend_agent: &str,
    intents: &[IntentRow],
) -> Vec<QueryFixture> {
    let mut fixtures: Vec<QueryFixture> = Vec::new();

    // ── Q-point: first public resource the owner can read ─────────────────────────
    let public_res = resources
        .iter()
        .find(|r| r.category == ResourceCategory::PublicProfile);

    if let Some(res) = public_res {
        let sparql = format!(
            "SELECT ?s ?p ?o WHERE {{ GRAPH <{}> {{ ?s ?p ?o }} }}",
            res.uri
        );
        // Expected result rows: type and category triples for this resource.
        let mut expected_rows = vec![
            format!(
                "<{}> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/ldp#Resource>",
                res.uri
            ),
            format!(
                "<{}> <https://sparq.dev/vocab/bench#category> <{}>",
                res.uri,
                category_iri(&res.category)
            ),
        ];
        expected_rows.sort();
        fixtures.push(QueryFixture {
            class: QueryClass::Point,
            sparql,
            expected_rows,
            agent: owner_uri.to_string(),
            model: AcModel::Wac,
        });
    }

    // ── Q-scan: owner-accessible resource URIs ────────────────────────────────────
    let mut owner_read_uris: Vec<String> = resources
        .iter()
        .filter(|r| {
            let req = Request {
                agent: owner_uri.to_string(),
                client: None,
                resource: r.uri.clone(),
                mode: AccessMode::read_only(),
            };
            oracle_wac(&req, intents) == Decision::Allow
        })
        .map(|r| format!("<{}>", r.uri))
        .collect();
    owner_read_uris.sort();

    fixtures.push(QueryFixture {
        class: QueryClass::Scan,
        sparql: "SELECT ?res WHERE { ?pod <http://www.w3.org/ns/ldp#contains> ?res }".to_string(),
        expected_rows: owner_read_uris.clone(),
        agent: owner_uri.to_string(),
        model: AcModel::Wac,
    });

    // ── Q-join: friend agent's accessible resources ───────────────────────────────
    let mut friend_accessible: Vec<String> = resources
        .iter()
        .filter(|r| {
            let req = Request {
                agent: friend_agent.to_string(),
                client: None,
                resource: r.uri.clone(),
                mode: AccessMode::read_only(),
            };
            oracle_wac(&req, intents) == Decision::Allow
        })
        .map(|r| format!("<{}>", r.uri))
        .collect();
    friend_accessible.sort();

    let join_sparql = format!(
        "SELECT ?res WHERE {{ ?res <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/ldp#Resource> . FILTER(?res IN ({})) }}",
        if friend_accessible.is_empty() {
            "<urn:sparq:empty>".to_string()
        } else {
            friend_accessible.join(", ")
        }
    );
    fixtures.push(QueryFixture {
        class: QueryClass::Join,
        sparql: join_sparql,
        expected_rows: friend_accessible,
        agent: friend_agent.to_string(),
        model: AcModel::Wac,
    });

    // ── Q-agg: COUNT of owner-accessible resources ────────────────────────────────
    let owner_count = owner_read_uris.len();
    fixtures.push(QueryFixture {
        class: QueryClass::Aggregate,
        sparql: "SELECT (COUNT(?res) AS ?count) WHERE { ?res <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/ldp#Resource> }".to_string(),
        expected_rows: vec![format!("{owner_count}")],
        agent: owner_uri.to_string(),
        model: AcModel::Wac,
    });

    fixtures
}

// ── Expressibility matrix helpers ────────────────────────────────────────────────────

/// Return the [`Expressibility`] classification for a given intent row in the WAC model.
///
/// This is the per-intent entry in the U1 expressibility matrix; recorded as a documented
/// artifact (§2.2 of the design record).
#[must_use]
pub fn wac_expressibility(row: &IntentRow) -> Expressibility {
    compile_wac(row).expressibility
}

/// Return the [`Expressibility`] classification for a given intent row in the ACP model.
///
/// For `Audience::Group`, `group_members` must be the resolved member list; an empty
/// slice marks the entry as `Expansion(0)` (pending resolution).
#[must_use]
pub fn acp_expressibility(row: &IntentRow, group_members: &[String]) -> Expressibility {
    compile_acp(row, group_members).expressibility
}

/// Return the [`Expressibility`] classification for a given intent row in the ODRL model.
#[must_use]
pub fn odrl_expressibility(row: &IntentRow) -> Expressibility {
    compile_odrl(row).expressibility
}

// ── Smoke-test helper ────────────────────────────────────────────────────────────────

/// Smoke-test helper: generate U1 at `GenParams::smoke()` and return the first
/// `n` expected decisions (WAC model).
///
/// # Panics
/// Panics if `GenParams::smoke()` validation fails (it should not).
#[doc(hidden)]
#[must_use]
pub fn smoke_decisions(n: usize) -> Vec<(Request, Decision)> {
    let params = GenParams::smoke();
    let ds = generate(&params);
    ds.expected_decisions
        .iter()
        .filter(|ed| ed.model == AcModel::Wac)
        .take(n)
        .map(|ed| (ed.request.clone(), ed.decision.clone()))
        .collect()
}
