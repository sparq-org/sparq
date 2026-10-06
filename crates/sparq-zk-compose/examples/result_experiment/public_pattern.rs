// [OPUS-5.5] beadzkp-15.1.1: synthetic, NONcanonical v1/v4 public-pattern ablation.
//! Pairs the baseline version-one relation with the opt-in version-four relation.
//!
//! Both arms consume identical query bytes, released mappings, signed synthetic
//! credentials, verifier policy and prover options, and must realize the same
//! issuer-slot capacity. They share one semantic/disclosure acceptance digest,
//! while each sample retains its own relation version, package, proof and public
//! transcript. The legacy selected-support experiment is not eligible for version
//! four (hidden subject and private FILTER) and is never rewritten here.
use super::*;
use sparq_zk_compose::{
    planner::{DisclosureQuery, QueryKind, QuerySlot},
    result::{CredentialCapacity, PUBLIC_PATTERN_VERSION, prepare_result_public_pattern},
};
use std::collections::BTreeSet;

/// Manifest selector for this mode; schema 1/2 manifests keep the planner path.
pub(super) const SCHEMA_VERSION: u32 = 3;
const FIXTURE: &str = "synthetic_public_pattern_ablation_v1";
/// Every first-pattern slot is constant or projected; `?org` is a hidden join
/// between the remaining patterns. There is no FILTER.
const PATTERN_QUERY: &str = "SELECT DISTINCT ?person ?name WHERE { ?person <urn:name> ?name . ?person <urn:member> ?org . ?org <urn:accredited> <urn:yes> . }";
/// Paired arms in their canonical declaration order.
const ARMS: [Arm; 2] = [Arm::BaselineV1, Arm::PublicPatternV4];
/// Keeps an EC2 smoke or bounded run short; more warm-ups are not a cold-cache control.
const MAX_WARMUP_ROUNDS: u32 = 2;
/// Bounds measured paired rounds per profile (at most 40 samples per manifest).
const MAX_MEASURED_ROUNDS: u32 = 8;
/// Deterministic synthetic test challenges: `NONCE_BASE + ordinal`, disjoint from
/// the legacy `420000 + ordinal` series and the shared `TAMPER_NONCE`. Never a
/// production nonce source.
const NONCE_BASE: u64 = 430_000;

/// Realized issuer-slot capacity; each profile has its own synthetic wallet.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub(super) enum Profile {
    /// One credential supports every pattern.
    K1,
    /// The public pattern and the hidden join live in two distinct credentials.
    K2,
}
impl Profile {
    fn slots(self) -> usize {
        match self {
            Self::K1 => 1,
            Self::K2 => 2,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::K1 => "k1",
            Self::K2 => "k2",
        }
    }
}

/// One side of the paired comparison.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Arm {
    /// Default `prepare_result_with_options`, generic typed openings for every pattern.
    BaselineV1,
    /// Opt-in `prepare_result_public_pattern`, public triple table for pattern zero.
    PublicPatternV4,
}
impl Arm {
    fn name(self) -> &'static str {
        match self {
            Self::BaselineV1 => "baseline_v1",
            Self::PublicPatternV4 => "public_pattern_v4",
        }
    }
    fn version(self) -> u32 {
        match self {
            Self::BaselineV1 => 1,
            Self::PublicPatternV4 => PUBLIC_PATTERN_VERSION,
        }
    }
    fn other(self) -> Self {
        match self {
            Self::BaselineV1 => Self::PublicPatternV4,
            Self::PublicPatternV4 => Self::BaselineV1,
        }
    }
    fn entry_point(self) -> &'static str {
        match self {
            Self::BaselineV1 => "result::prepare_result_with_options",
            Self::PublicPatternV4 => "result::prepare_result_public_pattern",
        }
    }
    /// Adapter-declared member for this F0, depth-10 fixture; not a driver observation.
    fn package(self, profile: Profile) -> &'static str {
        match (self, profile) {
            (Self::BaselineV1, Profile::K1) => "result_v1_k1_n16_p3_r4_f0",
            (Self::BaselineV1, Profile::K2) => "result_v1_k2_n16_p3_r4_f0",
            (Self::PublicPatternV4, Profile::K1) => "result_v4_k1_n16_p3_r4_f0_d10",
            (Self::PublicPatternV4, Profile::K2) => "result_v4_k2_n16_p3_r4_f0_d10",
        }
    }
}

/// Semantic and disclosure acceptance shared by both arms; no relation version.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct AblationContract {
    meaning: String,
    exact_dataset_scope: String,
    holder_binding: String,
    query: String,
    released_terms: Vec<BTreeMap<String, String>>,
    authority: String,
    signature_suite: String,
    status_regime: String,
    disclosure_regime: String,
    capacity_policy: String,
}
impl AblationContract {
    fn expected() -> Self {
        Self {
            meaning: "experimental_selected_successful_support_unsigned_v1".into(),
            exact_dataset_scope: "not_applicable_no_completeness_claim".into(),
            holder_binding: "none_no_holder_identity_claim".into(),
            query: PATTERN_QUERY.into(),
            released_terms: vec![BTreeMap::from([
                ("name".into(), "\"Alice\"".into()),
                ("person".into(), "<urn:alice>".into()),
            ])],
            authority: "synthetic_babyjubjub_issuer_seed_1".into(),
            signature_suite: "sparq_schnorr_babyjubjub_poseidon2".into(),
            status_regime: "verifier_owned_snapshot_epoch_7_hidden_reference_depth_10".into(),
            disclosure_regime:
                "public_query_result_issuer_slots_private_roots_hidden_join_and_status".into(),
            capacity_policy: "smallest_realized_k_equal_within_pair_n16_p3_r4_f0_d10".into(),
        }
    }
}

/// Strict schema-3 manifest for the paired public-pattern ablation.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Ablation {
    schema_version: u32,
    fixture: String,
    contract: AblationContract,
    profiles: Vec<Profile>,
    arms: Vec<Arm>,
    warmup_rounds_per_profile: u32,
    measured_rounds_per_profile: u32,
}
impl Ablation {
    /// Rejects any other fixture, contract, arm inventory, profile list or budget.
    pub(super) fn validate(&self) -> Fallible<()> {
        if self.schema_version != SCHEMA_VERSION
            || self.fixture != FIXTURE
            || self.contract != AblationContract::expected()
        {
            return Err(
                "unsupported public-pattern fixture, contract or signature/status/disclosure regime"
                    .into(),
            );
        }
        eligible(&self.contract.query)?;
        let unique: BTreeSet<_> = self.profiles.iter().collect();
        if self.profiles.is_empty() || unique.len() != self.profiles.len() {
            return Err("profiles must list k1 and/or k2 once each".into());
        }
        if self.arms != ARMS {
            return Err("arms must be exactly baseline_v1 then public_pattern_v4".into());
        }
        if self.warmup_rounds_per_profile > MAX_WARMUP_ROUNDS
            || !(1..=MAX_MEASURED_ROUNDS).contains(&self.measured_rounds_per_profile)
        {
            return Err("invalid paired-round budget".into());
        }
        Ok(())
    }
}

/// Checks version-four eligibility without relying on library pattern reordering.
fn eligible(query: &str) -> Fallible<()> {
    let q = DisclosureQuery::parse(query)?;
    if !matches!(q.kind, QueryKind::SelectDistinct) {
        return Err("public-pattern ablation requires SELECT DISTINCT".into());
    }
    let public = |slot: &QuerySlot| match slot {
        QuerySlot::Constant(_) => true,
        QuerySlot::Variable(v) => q.projection.contains(v),
    };
    let (first, rest) = q.patterns.split_first().ok_or("empty pattern list")?;
    if !first.iter().all(public) {
        return Err("first pattern must be constant or projected in every slot".into());
    }
    if q.filters
        .iter()
        .any(|f| !q.projection.contains(&f.variable))
    {
        return Err("hidden FILTER is outside the public-pattern contract".into());
    }
    if !rest.iter().flatten().any(|slot| !public(slot)) {
        return Err("ablation requires a hidden remaining join".into());
    }
    Ok(())
}

/// Builds the signed synthetic wallet and verifier policy for one profile.
///
/// Bob's organization is not accredited, so the hidden join fails for him.
/// Carla is supported but withheld: selected support is not completeness.
fn fixture(profile: Profile) -> Fallible<Fixture> {
    let iri = |s: &str| NamedNode::new(s.to_owned());
    let mut names = Vec::new();
    let mut joins = Vec::new();
    for (person, name, org) in [
        ("urn:alice", "Alice", "urn:org:acme"),
        ("urn:bob", "Bob", "urn:org:shell"),
        ("urn:carla", "Carla", "urn:org:acme"),
    ] {
        names.push(Triple::new(
            iri(person)?,
            iri("urn:name")?,
            Literal::new_simple_literal(name),
        ));
        joins.push(Triple::new(iri(person)?, iri("urn:member")?, iri(org)?));
    }
    for (org, status) in [("urn:org:acme", "urn:yes"), ("urn:org:shell", "urn:no")] {
        joins.push(Triple::new(iri(org)?, iri("urn:accredited")?, iri(status)?));
    }
    let inputs = match profile {
        Profile::K1 => vec![names.into_iter().chain(joins).collect()],
        Profile::K2 => vec![names, joins],
    };
    // Reuse the legacy issuer, salts, status list and policy; only rows differ.
    let mut f = authenticate(inputs)?;
    f.rows = vec![BTreeMap::from([
        (
            "name".into(),
            Term::Literal(Literal::new_simple_literal("Alice")),
        ),
        ("person".into(), Term::NamedNode(iri("urn:alice")?)),
    ])];
    Ok(f)
}

/// Identical prover options for both arms; the smallest bucket realizes K.
fn options() -> ResultOptions {
    ResultOptions {
        credential_capacity: CredentialCapacity::Smallest,
        witness_selection: WitnessSelection::Optimize,
        ..ResultOptions::default()
    }
}

fn prepare(arm: Arm, f: &Fixture, nonce: &VerifierNonce) -> Result<PreparedResult, ResultError> {
    let (query, credentials, rows, policy) = (PATTERN_QUERY, &f.credentials, &f.rows, &f.policy);
    match arm {
        Arm::BaselineV1 => {
            prepare_result_with_options(query, credentials, rows, policy, nonce, options())
        }
        Arm::PublicPatternV4 => {
            prepare_result_public_pattern(query, credentials, rows, policy, nonce, options())
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Role {
    Warmup,
    Measurement,
}

/// One attempted proof in the fixed execution order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Sample {
    profile: Profile,
    arm: Arm,
    role: Role,
    round: u32,
    position: usize,
    ordinal: u64,
    nonce_id: u64,
}
impl Sample {
    /// Tamper controls run once per arm and profile, in the first measured round.
    fn controls_required(&self) -> bool {
        self.role == Role::Measurement && self.round == 0
    }
}

/// Lists every sample: profiles independently, warm-ups before measurements.
///
/// Pair order alternates by round index within each role, so measured round
/// zero is always baseline-first regardless of the warm-up count.
fn schedule(exp: &Ablation) -> Vec<Sample> {
    let mut samples = Vec::new();
    for &profile in &exp.profiles {
        for (role, rounds) in [
            (Role::Warmup, exp.warmup_rounds_per_profile),
            (Role::Measurement, exp.measured_rounds_per_profile),
        ] {
            for round in 0..rounds {
                let order = if round % 2 == 0 {
                    ARMS
                } else {
                    [ARMS[1], ARMS[0]]
                };
                for (position, arm) in order.into_iter().enumerate() {
                    let ordinal = samples.len() as u64 + 1;
                    samples.push(Sample {
                        profile,
                        arm,
                        role,
                        round,
                        position,
                        ordinal,
                        nonce_id: NONCE_BASE + ordinal,
                    });
                }
            }
        }
    }
    samples
}

/// Public transcript fields without the proof bytes.
fn public_fields(p: &ResultPresentation) -> Fallible<serde_json::Map<String, Value>> {
    let Value::Object(mut fields) = serde_json::to_value(p)? else {
        return Err("presentation object".into());
    };
    fields.remove("proof");
    Ok(fields)
}

/// Names the public transcript fields that differ between two presentations.
fn transcript_difference(a: &ResultPresentation, b: &ResultPresentation) -> Fallible<Vec<String>> {
    let (a, b) = (public_fields(a)?, public_fields(b)?);
    let keys: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
    Ok(keys
        .into_iter()
        .filter(|k| a.get(*k) != b.get(*k))
        .cloned()
        .collect())
}

/// Checks that a completed pair differs only in relation version and challenge.
///
/// Byte-identical transcripts are deliberately not asserted.
fn pair_check(
    first: (&ResultPresentation, &Value),
    second: (&ResultPresentation, &Value),
) -> Fallible<Value> {
    let differing = transcript_difference(first.0, second.0)?;
    if differing != ["challenge", "version"] {
        return Err(format!(
            "paired public transcripts must differ only in challenge and version: {differing:?}"
        )
        .into());
    }
    if first.1 != second.1 {
        return Err("paired arms selected different prover work".into());
    }
    Ok(
        json!({"status":"success","differing_public_fields":differing,
        "shared_public_fields":["query","rows","issuer_slots","integer_capacity"],
        "equal_prover_work":true,"byte_identical_transcripts":"not_asserted"}),
    )
}

/// Rejects a sample whose realized issuer-slot capacity differs from its profile.
fn check_realized_capacity(p: &ResultPresentation, work: &Value, profile: Profile) -> Fallible<()> {
    let k = profile.slots();
    if p.issuer_slots.len() != k
        || work["selected_credentials"] != json!(k)
        || work["signature_checks"] != json!(k)
    {
        return Err("realized capacity differs from the declared profile".into());
    }
    Ok(())
}

/// Identifies the declared relation and hashes its package sources.
fn relation_identity(arm: Arm, profile: Profile, root: &Path) -> Fallible<Value> {
    let package = arm.package(profile);
    let dir = root.join("zk/compose").join(package);
    let nargo_toml = hash(&fs::read(dir.join("Nargo.toml"))?);
    let main_nr = hash(&fs::read(dir.join("src/main.nr"))?);
    Ok(
        json!({"entry_point":arm.entry_point(),"version":arm.version(),"package":package,
        "package_source_blake3":{"Nargo.toml":nargo_toml,"src/main.nr":main_nr},
        "package_provenance":"adapter-declared from the public version and realized issuer-slot count; shared compose_core sources are identified by the recorded git tree",
        "public_abi":{"value":null,"reason":"public-input bytes are reconstructed inside the verifier API and not exported; version and package identify the ABI"}}),
    )
}

fn stage_definitions() -> Value {
    json!({
        "prepare_api":"host witness search, authentication checks and public-statement construction; no subprocess",
        "prove_api_inclusive":"PreparedResult::prove: toolchain check, workspace lock, nargo_compile of the member package, private ACIR copy, nargo_execute and bb_prove_and_write_vk",
        "verify_api_inclusive":"verify_result: statement reconstruction, toolchain check, workspace lock, nargo_compile of the canonical member package, private ACIR copy, independent bb_write_vk and bb_verify",
        "compilation":"the nargo_compile driver child events inside prove_api_inclusive and verify_api_inclusive; every call compiles, no ACIR snapshot reuse; Rust compilation is outside every timer",
        "replay_control_excluded_from_timings":"second verification with the consumed challenge; must reject before cryptographic work",
        "tamper_controls_excluded_from_timings":"first measured round only; proof bytes, released term, challenge on both sides, requested query, accepted status snapshot and version relabeling"})
}

/// Runs the paired ablation, retaining every attempted sample in `report.json`.
///
/// # Errors
/// Returns an error before creating output for a dirty checkout or an output
/// inside the source tree; later failures are recorded and yield `Ok(false)`.
pub(super) fn execute(exp: &Ablation, out: &Path) -> Fallible<bool> {
    let (root, source) = open_output(out)?;
    let mut report = json!({"schema_version":SCHEMA_VERSION,"canonical":false,"status":"failure",
        "mode":"public_pattern_ablation_baseline_v1_vs_opt_in_v4","experiment":exp,"source":source,
        "backend":{"name":"barretenberg","target":"noir-recursive","zk_mode_requested":true},
        "signature_suites_unavailable":["BBS+","ECDSA","EdDSA"],
        "comparison_contract":{
            "shared_within_pair":"identical query bytes, released mappings, signed credential inputs and salts, issuer/status policy, prover options and realized issuer-slot capacity",
            "separately_visible":"relation version, entry point, package and package sources, proof bytes, public transcript and driver events per sample",
            "byte_identical_transcripts":"not_asserted",
            "claims":"experimental selected support only; no completeness, absence or holder identity",
            "aggregation":"none; raw individual timings only; no cross-contract speedup is computed or implied"},
        "stage_definitions":stage_definitions(),
        "cache_contract":{"os_cache":"uncontrolled","nargo_dependency_cache":"uncontrolled","rust_build":"outside timing",
            "warmup_definition":"complete paired rounds (both arms, full prepare/prove/verify/replay) labelled warmup and never counted as measurements; no OS cold-cache assertion",
            "pair_order":"alternates by round index within each role; measured round zero is baseline first",
            "nargo_internal_cache_hit":{"value":null,"reason":"Nargo does not expose reliable per-invocation cache-hit telemetry"},
            "nonce_convention":"distinct deterministic synthetic test challenge per attempted sample; excluded from the acceptance digest and retained in each transcript"},
        "timing_contract":TIMING_CONTRACT,
        "unavailable_stages":unavailable_stages(),
        "profiles":{},"pairs":[],"runs":[]});
    let outcome = (|| -> Fallible<()> {
        record_tools(&mut report, &root)?;
        let mut fixtures = BTreeMap::new();
        for &profile in &exp.profiles {
            let start = Instant::now();
            let f = fixture(profile)?;
            let setup = start.elapsed().as_secs_f64();
            let binding = fixture_binding(&f);
            if binding["released_terms"] != json!(exp.contract.released_terms) {
                return Err("fixture rows differ from contract released terms".into());
            }
            let digest = json_hash(&json!({"acceptance":exp.contract,"synthetic_inputs":binding}))?;
            let mut relations = json!({});
            for arm in ARMS {
                relations[arm.name()] = relation_identity(arm, profile, &root)?;
            }
            report["profiles"][profile.name()] = json!({"realized_issuer_slots":profile.slots(),
                "fixture_setup_seconds":setup,"acceptance_contract_blake3":digest,
                "synthetic_input_binding":binding,"relations":relations});
            fixtures.insert(profile, (f, digest));
        }
        let prover = CircuitProver::new(root.join("zk/compose")).with_stage_metrics();
        let mut completed_warmups: BTreeMap<Profile, u32> = BTreeMap::new();
        let mut pending: Option<(ResultPresentation, Value)> = None;
        for sample in schedule(exp) {
            let (f, digest) = fixtures.get(&sample.profile).ok_or("profile fixture")?;
            let dir = out.join(format!("run-{}", sample.ordinal));
            fs::create_dir(&dir)?;
            let mut r = json!({"ordinal":sample.ordinal,"profile":sample.profile,"arm":sample.arm,
                "relation_version":sample.arm.version(),"package":sample.arm.package(sample.profile),
                "role":sample.role,"round":sample.round,"position_in_pair":sample.position,
                "completed_warmup_rounds":completed_warmups.get(&sample.profile).copied().unwrap_or(0),
                "prior_runs_in_process":sample.ordinal - 1,"nonce_id":sample.nonce_id,"status":"failure",
                "acceptance_contract_blake3":digest,"stages":{}});
            let altered_query =
                PATTERN_QUERY.replace("<urn:accredited> <urn:yes>", "<urn:accredited> <urn:no>");
            let relation = Relation {
                query: PATTERN_QUERY,
                altered_query,
                version: sample.arm.version(),
                relabel_version: Some(sample.arm.other().version()),
                prepare: &|nonce: &VerifierNonce| prepare(sample.arm, f, nonce),
            };
            let result = run_relation(
                &relation,
                f,
                sample.nonce_id,
                &prover,
                &dir,
                sample.controls_required(),
                &mut r,
            )
            .and_then(|p| {
                check_realized_capacity(&p, &r["work"], sample.profile)?;
                Ok(p)
            });
            if let Err(ref e) = result {
                r["status"] = json!("failure");
                r["error"] = json!(e.to_string());
                r["error_class"] = json!(error_class(e.as_ref()));
            }
            let work = r["work"].clone();
            report["runs"].as_array_mut().ok_or("run array")?.push(r);
            let p = result?;
            // A controlled ablation cannot mutate its inputs or acceptance policy.
            if json_hash(&json!({"acceptance":exp.contract,"synthetic_inputs":fixture_binding(f)}))?
                != *digest
            {
                return Err("ablation changed acceptance contract".into());
            }
            if sample.position == 0 {
                pending = Some((p, work));
                continue;
            }
            let (first, first_work) = pending.take().ok_or("pair without a first arm")?;
            let mut pair = json!({"profile":sample.profile,"role":sample.role,"round":sample.round,
                "ordinals":[sample.ordinal - 1, sample.ordinal]});
            let checked = pair_check((&first, &first_work), (&p, &work));
            match &checked {
                Ok(check) => pair["check"] = check.clone(),
                Err(e) => pair["check"] = json!({"status":"failure","error":e.to_string()}),
            }
            report["pairs"]
                .as_array_mut()
                .ok_or("pair array")?
                .push(pair);
            checked?;
            if sample.role == Role::Warmup {
                *completed_warmups.entry(sample.profile).or_default() += 1;
            }
        }
        report["status"] = json!("success");
        Ok(())
    })();
    if let Err(e) = outcome.as_ref() {
        report["error"] = json!(e.to_string());
    }
    fs::write(out.join("report.json"), serde_json::to_vec_pretty(&report)?)?;
    Ok(outcome.is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke manifest body: both profiles, no warm-up, one measured paired round.
    const SMOKE: &str = r#"{
      "schema_version": 3,
      "fixture": "synthetic_public_pattern_ablation_v1",
      "contract": {
        "meaning": "experimental_selected_successful_support_unsigned_v1",
        "exact_dataset_scope": "not_applicable_no_completeness_claim",
        "holder_binding": "none_no_holder_identity_claim",
        "query": "SELECT DISTINCT ?person ?name WHERE { ?person <urn:name> ?name . ?person <urn:member> ?org . ?org <urn:accredited> <urn:yes> . }",
        "released_terms": [{"name": "\"Alice\"", "person": "<urn:alice>"}],
        "authority": "synthetic_babyjubjub_issuer_seed_1",
        "signature_suite": "sparq_schnorr_babyjubjub_poseidon2",
        "status_regime": "verifier_owned_snapshot_epoch_7_hidden_reference_depth_10",
        "disclosure_regime": "public_query_result_issuer_slots_private_roots_hidden_join_and_status",
        "capacity_policy": "smallest_realized_k_equal_within_pair_n16_p3_r4_f0_d10"
      },
      "profiles": ["k1", "k2"],
      "arms": ["baseline_v1", "public_pattern_v4"],
      "warmup_rounds_per_profile": 0,
      "measured_rounds_per_profile": 1
    }"#;

    /// Parses the smoke body; `bounded` adds one warm-up and four measured rounds.
    fn manifest(name: &str) -> Ablation {
        let mut exp: Ablation = serde_json::from_str(SMOKE).unwrap();
        if name == "bounded" {
            exp.warmup_rounds_per_profile = 1;
            exp.measured_rounds_per_profile = 4;
        }
        exp.validate().unwrap();
        exp
    }

    #[test]
    fn inline_validated_modes_dispatch_to_their_own_schema() {
        for name in ["smoke", "bounded"] {
            let exp = manifest(name);
            let bytes = serde_json::to_vec(&exp).unwrap();
            assert!(matches!(
                parse_manifest(&bytes).unwrap(),
                Manifest::PublicPattern(_)
            ));
        }
        let legacy = include_bytes!(
            "../../../../bench/zk-compose/experiments/synthetic-selected-support.json"
        );
        assert!(matches!(
            parse_manifest(legacy).unwrap(),
            Manifest::Planner(_)
        ));
        // A legacy body relabeled as schema 3 is not accepted by either schema.
        let mut relabeled: Value = serde_json::from_slice(legacy).unwrap();
        relabeled["schema_version"] = json!(SCHEMA_VERSION);
        assert!(parse_manifest(&serde_json::to_vec(&relabeled).unwrap()).is_err());
        // An ablation body relabeled as schema 1 or 2 is rejected, never truncated.
        for version in [1, 2] {
            let mut value = serde_json::to_value(manifest("smoke")).unwrap();
            value["schema_version"] = json!(version);
            assert!(parse_manifest(&serde_json::to_vec(&value).unwrap()).is_err());
        }
        // Duplicate keys cannot survive dispatch.
        let smoke = serde_json::to_string(&manifest("smoke")).unwrap();
        let duplicate = smoke.replacen('{', "{\"warmup_rounds_per_profile\":2,", 1);
        assert!(parse_manifest(duplicate.as_bytes()).is_err());
    }

    #[test]
    fn contract_and_profile_tampering_is_rejected() {
        let valid = serde_json::to_value(manifest("smoke")).unwrap();
        for field in [
            "meaning",
            "exact_dataset_scope",
            "holder_binding",
            "query",
            "released_terms",
            "authority",
            "signature_suite",
            "status_regime",
            "disclosure_regime",
            "capacity_policy",
        ] {
            let mut value = valid.clone();
            value["contract"][field] = if field == "released_terms" {
                json!([{"name":"\"Alice\""}])
            } else {
                json!("changed")
            };
            let exp: Ablation = serde_json::from_value(value).unwrap();
            assert!(exp.validate().is_err(), "{field}");
        }
        assert!(rejected(&valid, |v| v["extra"] = json!(1)));
        assert!(rejected(&valid, |v| v["contract"]["extra"] = json!(1)));
        assert!(rejected(&valid, |v| v["fixture"] =
            json!("synthetic_shared_alternative_v1")));
        assert!(rejected(&valid, |v| v["profiles"] = json!(["k3"])));
        assert!(rejected(&valid, |v| v["profiles"] = json!([])));
        assert!(rejected(&valid, |v| v["profiles"] = json!(["k1", "k1"])));
        assert!(rejected(&valid, |v| v["arms"] = json!(["baseline_v1"])));
        assert!(rejected(&valid, |v| v["arms"] =
            json!(["public_pattern_v4", "baseline_v1"])));
        assert!(rejected(&valid, |v| v["arms"] = json!(["baseline_v1", "signed_v3"])));
        assert!(rejected(&valid, |v| v["warmup_rounds_per_profile"] =
            json!(MAX_WARMUP_ROUNDS + 1)));
        assert!(rejected(&valid, |v| v["measured_rounds_per_profile"] = json!(0)));
        assert!(rejected(&valid, |v| v["measured_rounds_per_profile"] =
            json!(MAX_MEASURED_ROUNDS + 1)));
        assert!(!rejected(&valid, |v| v["profiles"] = json!(["k2"])));
    }

    /// Applies one edit to a valid manifest and reports parse-or-validation rejection.
    fn rejected(valid: &Value, edit: impl FnOnce(&mut Value)) -> bool {
        let mut value = valid.clone();
        edit(&mut value);
        match serde_json::from_value::<Ablation>(value) {
            Ok(exp) => exp.validate().is_err(),
            Err(_) => true,
        }
    }

    #[test]
    fn only_the_new_query_shape_is_version_four_eligible() {
        eligible(PATTERN_QUERY).unwrap();
        // The legacy experiment hides ?s and filters a hidden ?age.
        assert!(eligible(super::super::QUERY).is_err());
        let hidden_filter = "SELECT DISTINCT ?person ?name WHERE { ?person <urn:name> ?name . ?person <urn:age> ?age . FILTER(?age >= 18) }";
        assert!(eligible(hidden_filter).is_err());
        // A public pattern that is not first would need library reordering.
        let late = "SELECT DISTINCT ?person WHERE { ?person <urn:member> ?org . ?person <urn:name> \"Alice\" . }";
        assert!(eligible(late).is_err());
        let no_join = "SELECT DISTINCT ?person ?name WHERE { ?person <urn:name> ?name . }";
        assert!(eligible(no_join).is_err());
    }

    #[test]
    fn k1_and_k2_preparations_are_equal_across_arms() {
        let mut nonce = 7u64;
        for profile in [Profile::K1, Profile::K2] {
            let f = fixture(profile).unwrap();
            assert_eq!(f.credentials.len(), profile.slots());
            let before = fixture_binding(&f);
            assert_eq!(
                before["released_terms"],
                json!(AblationContract::expected().released_terms)
            );
            let mut works = Vec::new();
            for arm in ARMS {
                nonce += 1;
                let prepared =
                    prepare(arm, &f, &VerifierNonce::from_field(Fr::from(nonce))).unwrap();
                let w = prepared.work();
                assert_eq!(
                    w.selected_credentials,
                    profile.slots(),
                    "{profile:?} {arm:?}"
                );
                assert_eq!(w.signature_checks, profile.slots(), "{profile:?} {arm:?}");
                assert_eq!(w.private_predicates, 0);
                works.push(work(w));
                assert_eq!(fixture_binding(&f), before);
            }
            assert_eq!(works[0], works[1], "{profile:?}");
        }
    }

    #[test]
    fn version_four_rejects_the_legacy_fixture_without_fallback() {
        let f = super::super::fixture().unwrap();
        let n = VerifierNonce::from_field(Fr::from(42u64));
        let err = prepare_result_public_pattern(
            super::super::QUERY,
            &f.credentials,
            &f.rows,
            &f.policy,
            &n,
            options(),
        )
        .err()
        .unwrap();
        assert!(matches!(err, ResultError::Rejected(_)), "{err}");
        // The ablation's own wallet is not substituted into the legacy request either.
        let own = fixture(Profile::K1).unwrap();
        assert!(
            prepare_result_public_pattern(
                super::super::QUERY,
                &own.credentials,
                &f.rows,
                &own.policy,
                &n,
                options(),
            )
            .is_err()
        );
    }

    #[test]
    fn versions_packages_and_outputs_stay_separately_visible() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let mut packages = BTreeSet::new();
        for profile in [Profile::K1, Profile::K2] {
            for arm in ARMS {
                let id = relation_identity(arm, profile, &root).unwrap();
                assert_eq!(id["version"], json!(arm.version()));
                let package = id["package"].as_str().unwrap();
                assert!(package.contains(&format!("_{}_", profile.name())));
                packages.insert(arm.package(profile));
            }
            assert_ne!(
                relation_identity(ARMS[0], profile, &root).unwrap()["package_source_blake3"],
                relation_identity(ARMS[1], profile, &root).unwrap()["package_source_blake3"]
            );
        }
        assert_eq!(packages.len(), 4);
        assert_eq!(Arm::BaselineV1.version(), 1);
        assert_eq!(Arm::PublicPatternV4.version(), 4);
        assert_eq!(Arm::BaselineV1.other(), Arm::PublicPatternV4);
        // Outputs must live outside the checkout.
        assert!(require_outside_source(&root, &root.join("new-output")).is_err());
        assert!(require_outside_source(&root, &root.join("bench/new-output")).is_err());
        let outside = std::env::temp_dir().canonicalize().unwrap();
        if !outside.starts_with(&root) {
            require_outside_source(&root, &outside.join("new-output")).unwrap();
        }
    }

    #[test]
    fn challenges_are_distinct_and_pair_order_alternates() {
        let exp = manifest("bounded");
        let samples = schedule(&exp);
        let rounds = exp.warmup_rounds_per_profile + exp.measured_rounds_per_profile;
        assert_eq!(
            samples.len(),
            exp.profiles.len() * ARMS.len() * rounds as usize
        );
        let nonces: BTreeSet<_> = samples.iter().map(|s| s.nonce_id).collect();
        assert_eq!(nonces.len(), samples.len());
        assert!(!nonces.contains(&TAMPER_NONCE));
        assert!(
            samples
                .iter()
                .enumerate()
                .all(|(i, s)| s.ordinal == i as u64 + 1)
        );
        for pair in samples.chunks(2) {
            assert_eq!(
                (pair[0].profile, pair[0].role, pair[0].round),
                (pair[1].profile, pair[1].role, pair[1].round)
            );
            assert_eq!((pair[0].position, pair[1].position), (0, 1));
            assert_eq!(pair[0].arm.other(), pair[1].arm);
            let expected = if pair[0].round % 2 == 0 {
                ARMS[0]
            } else {
                ARMS[1]
            };
            assert_eq!(pair[0].arm, expected);
        }
        for profile in [Profile::K1, Profile::K2] {
            let own: Vec<_> = samples.iter().filter(|s| s.profile == profile).collect();
            let first_measured = own
                .iter()
                .position(|s| s.role == Role::Measurement)
                .unwrap();
            assert!(own[..first_measured].iter().all(|s| s.role == Role::Warmup));
            assert!(
                own[first_measured..]
                    .iter()
                    .all(|s| s.role == Role::Measurement)
            );
            let controls: Vec<_> = own.iter().filter(|s| s.controls_required()).collect();
            assert_eq!(controls.len(), ARMS.len());
            assert_ne!(controls[0].arm, controls[1].arm);
        }
        // The largest admitted budget still keeps challenges distinct from the tamper nonce.
        let mut largest = manifest("bounded");
        largest.warmup_rounds_per_profile = MAX_WARMUP_ROUNDS;
        largest.measured_rounds_per_profile = MAX_MEASURED_ROUNDS;
        largest.validate().unwrap();
        let all = schedule(&largest);
        assert_eq!(all.len(), 40);
        let highest = all.iter().map(|s| s.nonce_id).max().unwrap();
        let lowest = all.iter().map(|s| s.nonce_id).min().unwrap();
        // Legacy planner runs use 420000 + ordinal with at most 14 ordinals.
        assert!(highest < TAMPER_NONCE && lowest > 420_000 + 14);
        let smoke = schedule(&manifest("smoke"));
        assert_eq!(smoke.len(), 4);
        assert!(
            smoke
                .iter()
                .all(|s| s.role == Role::Measurement && s.controls_required())
        );
    }

    #[test]
    fn pair_check_requires_equal_semantics_but_not_equal_bytes() {
        let base = ResultPresentation {
            version: 1,
            query: PATTERN_QUERY.into(),
            rows: Vec::new(),
            challenge: VerifierNonce::from_field(Fr::from(1u64)).as_field_hex(),
            issuer_slots: vec!["a".into()],
            integer_capacity: PrivateIntegerCapacity::TwoDigits,
            proof: vec![1],
        };
        let mut v4 = base.clone();
        v4.version = PUBLIC_PATTERN_VERSION;
        v4.challenge = VerifierNonce::from_field(Fr::from(2u64)).as_field_hex();
        v4.proof = vec![2, 3];
        let work = json!({"selected_credentials":1});
        let check = pair_check((&base, &work), (&v4, &work)).unwrap();
        assert_eq!(check["byte_identical_transcripts"], "not_asserted");
        let mut slots = v4.clone();
        slots.issuer_slots.push("a".into());
        assert!(pair_check((&base, &work), (&slots, &work)).is_err());
        let mut query = v4.clone();
        query.query.push(' ');
        assert!(pair_check((&base, &work), (&query, &work)).is_err());
        let mut same_challenge = v4.clone();
        same_challenge.challenge = base.challenge.clone();
        assert!(pair_check((&base, &work), (&same_challenge, &work)).is_err());
        let other_work = json!({"selected_credentials":2});
        assert!(pair_check((&base, &work), (&v4, &other_work)).is_err());
        let one = json!({"selected_credentials":1,"signature_checks":1});
        let mixed = json!({"selected_credentials":1,"signature_checks":2});
        assert!(check_realized_capacity(&base, &one, Profile::K1).is_ok());
        assert!(check_realized_capacity(&base, &one, Profile::K2).is_err());
        assert!(check_realized_capacity(&base, &mixed, Profile::K1).is_err());
    }
}
