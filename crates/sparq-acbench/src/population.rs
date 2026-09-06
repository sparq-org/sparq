//! Stream personal-service histories and equivalent WAC/ACP policies one Pod at a time.
//!
//! [GPT-6] The generator never constructs a server-wide deployment. Its working memory
//! depends on one record, the configuration, and the caller's writer. Record counts
//! combine one empirical marginal (`MovieLens` retained ratings) with explicitly declared
//! service-history assumptions. This is a scenario model, not a validated population
//! of future Solid users. See `bench/ac/million/corpus-calibration.json`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{self, Write};

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const ACL: &str = "http://www.w3.org/ns/auth/acl#";
const ACP: &str = "http://www.w3.org/ns/solid/acp#";
const LDP: &str = "http://www.w3.org/ns/ldp#";
const VCARD: &str = "http://www.w3.org/2006/vcard/ns#";
/// Vocabulary identifying synthetic personal records and their relationships.
pub const VOCAB: &str = "https://sparq.dev/bench/personal#";

/// Policy serialization for the same intended effective rights.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PolicyModel {
    /// Web Access Control with nearest-ACL inheritance and native groups.
    Wac,
    /// Access Control Policy with cumulative inheritance and enumerated groups.
    Acp,
}

/// One synthetic personal-service record family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Service {
    /// Message envelope, synthetic subject, and text excerpt.
    Communication,
    /// Retained address-book entries, stored once per Pod.
    Contacts,
    /// Appointments and participant links.
    Calendar,
    /// Account transaction records with amount, currency, and merchant links.
    Transactions,
    /// Daily activity summaries, not clinical patient histories.
    Activity,
    /// Episodic locations, not continuous high-frequency GPS samples.
    Location,
    /// Photo/video/attachment metadata, excluding their binary payloads.
    Media,
    /// Retained media-rating records, calibrated to a historical service sample.
    Ratings,
}

impl Service {
    /// All services in stable serialization order.
    pub const ALL: [Self; 8] = [
        Self::Communication,
        Self::Contacts,
        Self::Calendar,
        Self::Transactions,
        Self::Activity,
        Self::Location,
        Self::Media,
        Self::Ratings,
    ];

    /// Return the stable service identifier used in resource IRIs.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Communication => "communication",
            Self::Contacts => "contacts",
            Self::Calendar => "calendar",
            Self::Transactions => "transactions",
            Self::Activity => "activity",
            Self::Location => "location",
            Self::Media => "media",
            Self::Ratings => "ratings",
        }
    }
}

/// Monthly records at unit intensity; every field is a modeling assumption.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MonthlyRecords {
    /// Messages retained per modeled month.
    pub communication: u32,
    /// Calendar entries per modeled month.
    pub calendar: u32,
    /// Transaction records per modeled month.
    pub transactions: u32,
    /// Activity summaries per modeled month.
    pub activity: u32,
    /// Episodic location records per modeled month.
    pub location: u32,
    /// Media metadata records per modeled month.
    pub media: u32,
}

/// One shared activity-intensity class, selected independently for each Pod.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VolumeClass {
    /// Relative population mass; these are assumptions, not measured prevalence.
    pub weight: u32,
    /// Numerator of the multiplier applied to non-rating service record counts.
    pub numerator: u32,
    /// Positive denominator of the multiplier.
    pub denominator: u32,
}

/// Versioned corpus parameters, serialized into every persisted corpus manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PopulationConfig {
    /// Configuration schema version, currently one.
    pub schema_version: u32,
    /// Explicit profile label; smoke profiles cannot support population claims.
    pub profile: String,
    /// Deterministic seed, independent of the number of hosted Pods.
    pub seed: u64,
    /// HTTPS namespace ending in `/`; Pod roots append `p/{id}/`.
    pub base_iri: String,
    /// Retention horizon in complete calendar months before January 2026.
    pub history_months: u32,
    /// Base service volume before the shared Pod intensity multiplier.
    pub monthly_records: MonthlyRecords,
    /// Base retained address-book size, stored once rather than every month.
    pub contacts_snapshot: u32,
    /// Population mixture, preserving cross-service intensity correlation.
    pub volume_classes: Vec<VolumeClass>,
    /// `(retained rating count, cumulative users)` empirical count distribution.
    pub rating_count_cdf: Vec<(u32, u32)>,
    /// Number of named members in the media-sharing group.
    pub group_members: u32,
    /// Every nth calendar/media document is private; zero disables exceptions.
    pub private_exception_every: u32,
    /// Enable the common owner/private/public/individual/group sharing scenarios.
    pub sharing_enabled: bool,
}

impl PopulationConfig {
    /// Construct a tiny controlled fixture, explicitly unsuitable for capacity claims.
    #[must_use]
    pub fn smoke() -> Self {
        Self {
            schema_version: 1,
            profile: "controlled-smoke-v1".into(),
            seed: 20_260_906,
            base_iri: "https://pods.example/".into(),
            history_months: 2,
            monthly_records: MonthlyRecords {
                communication: 4,
                calendar: 2,
                transactions: 3,
                activity: 2,
                location: 4,
                media: 3,
            },
            contacts_snapshot: 4,
            volume_classes: vec![VolumeClass {
                weight: 1,
                numerator: 1,
                denominator: 1,
            }],
            rating_count_cdf: vec![(8, 1)],
            group_members: 4,
            private_exception_every: 2,
            sharing_enabled: true,
        }
    }

    /// Construct the central history scenario with an empirical ratings marginal.
    ///
    /// The histogram is a statistical aggregate; original `MovieLens` rows are not
    /// redistributed. Other volumes, correlations, retention and sharing frequencies
    /// remain declared assumptions. The constructor does not imply representativeness.
    #[must_use]
    pub fn service_history() -> Self {
        Self {
            profile: "service-history-central-v1".into(),
            history_months: 60,
            monthly_records: MonthlyRecords {
                communication: 300,
                calendar: 8,
                transactions: 41,
                activity: 30,
                location: 300,
                media: 40,
            },
            contacts_snapshot: 250,
            volume_classes: vec![
                VolumeClass {
                    weight: 50,
                    numerator: 1,
                    denominator: 2,
                },
                VolumeClass {
                    weight: 40,
                    numerator: 1,
                    denominator: 1,
                },
                VolumeClass {
                    weight: 9,
                    numerator: 4,
                    denominator: 1,
                },
                VolumeClass {
                    weight: 1,
                    numerator: 20,
                    denominator: 1,
                },
            ],
            rating_count_cdf: serde_json::from_str(include_str!("population_ratings_cdf.json"))
                .expect("bundled empirical aggregate is validated by population tests"),
            private_exception_every: 10,
            ..Self::smoke()
        }
    }

    /// Validate parameters before emitting any corpus bytes.
    ///
    /// # Errors
    /// Rejects invalid IRIs, unsupported versions, empty populations, and unbounded
    /// arithmetic inputs. Limits are safety bounds, not empirically justified volumes.
    pub fn validate(&self) -> io::Result<()> {
        let invalid = |message| io::Error::new(io::ErrorKind::InvalidInput, message);
        if self.schema_version != 1 || self.history_months == 0 || self.history_months > 1200 {
            return Err(invalid(
                "schema_version must be 1; history_months must be 1..=1200",
            ));
        }
        if !self.base_iri.starts_with("https://")
            || !self.base_iri.ends_with('/')
            || self.base_iri.bytes().any(|b| {
                b.is_ascii_whitespace() || matches!(b, b'<' | b'>' | b'"' | b'\\' | b'#' | b'?')
            })
        {
            return Err(invalid(
                "base_iri must be an absolute HTTPS directory IRI without query/fragment",
            ));
        }
        if self.volume_classes.is_empty() || self.volume_classes.len() > 1000 {
            return Err(invalid("volume_classes must contain 1..=1000 classes"));
        }
        for class in &self.volume_classes {
            if class.weight == 0
                || class.numerator == 0
                || class.denominator == 0
                || class.numerator > 1_000_000
                || class.weight > 1_000_000
            {
                return Err(invalid(
                    "volume classes require bounded positive weights and multipliers",
                ));
            }
        }
        if self.contacts_snapshot == 0
            || self.contacts_snapshot > 1_000_000
            || self.group_members == 0
            || self.group_members > 10_000
        {
            return Err(invalid(
                "contacts_snapshot/group_members outside positive bounded range",
            ));
        }
        for service in Service::ALL {
            if !matches!(service, Service::Contacts | Service::Ratings)
                && monthly(self, service) > 1_000_000
            {
                return Err(invalid("monthly records must not exceed 1000000"));
            }
        }
        if self.rating_count_cdf.is_empty() {
            return Err(invalid("rating_count_cdf must not be empty"));
        }
        let mut previous = (0, 0);
        for &(count, cumulative) in &self.rating_count_cdf {
            if count <= previous.0 || cumulative <= previous.1 {
                return Err(invalid(
                    "rating_count_cdf must have increasing positive counts and frequencies",
                ));
            }
            previous = (count, cumulative);
        }
        Ok(())
    }
}

/// Streaming statistics for a fully emitted Pod, including actual serialized bytes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PodSummary {
    /// Pod identifier, independent of generation order or population size.
    pub pod_id: u64,
    /// Base namespace of this Pod.
    pub root: String,
    /// Number of synthetic service records, excluding containers and policy triples.
    pub records: u64,
    /// Record counts by service.
    pub records_by_service: BTreeMap<String, u64>,
    /// Content record triples, including type and cross-service relationship triples.
    pub content_triples: u64,
    /// All emitted quads, including containment, group membership and policies.
    pub quads: u64,
    /// Non-control graphs, including service containers and the sharing group graph.
    pub content_graphs: u64,
    /// Number of physically serialized ACL/ACR graphs.
    pub policy_graphs: u64,
    /// Bytes written, before optional outer compression.
    pub bytes: u64,
    /// Shared non-rating intensity multiplier numerator.
    pub intensity_numerator: u32,
    /// Shared non-rating intensity multiplier denominator.
    pub intensity_denominator: u32,
}

/// A query with a stable name and intended benchmark purpose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PopulationQuery {
    /// Stable family identifier.
    pub id: String,
    /// SPARQL text, scoped by the server to one Pod's authorized dataset.
    pub sparql: String,
}

/// Return the independently addressable Pod root.
#[must_use]
pub fn pod_root(config: &PopulationConfig, pod_id: u64) -> String {
    format!("{}p/{pod_id}/", config.base_iri)
}

/// Return the synthetic owner's `WebID`.
#[must_use]
pub fn owner_webid(config: &PopulationConfig, pod_id: u64) -> String {
    format!("{}profile/card#me", pod_root(config, pod_id))
}

/// Return one named recipient's `WebID`; recipient zero also receives calendar access.
#[must_use]
pub fn recipient_webid(config: &PopulationConfig, pod_id: u64, recipient: u32) -> String {
    format!("{}people/{recipient}#me", pod_root(config, pod_id))
}

/// Decide intended record visibility without inspecting policy RDF or engine state.
#[must_use]
pub fn can_read(
    config: &PopulationConfig,
    pod_id: u64,
    service: Service,
    month: u32,
    agent: Option<&str>,
) -> bool {
    if agent.is_some_and(|a| a == owner_webid(config, pod_id)) {
        return true;
    }
    if !config.sharing_enabled || private_exception(config, service, month) {
        return false;
    }
    match service {
        Service::Ratings => true,
        Service::Calendar => agent.is_some_and(|a| a == recipient_webid(config, pod_id, 0)),
        Service::Media => agent.is_some_and(|a| {
            (0..config.group_members).any(|i| a == recipient_webid(config, pod_id, i))
        }),
        _ => false,
    }
}

/// Count visible records from the neutral scenario, without an RDF query engine.
///
/// # Errors
/// Returns an error for invalid configuration.
pub fn expected_record_count(
    config: &PopulationConfig,
    pod_id: u64,
    agent: Option<&str>,
) -> io::Result<u64> {
    config.validate()?;
    let counts = record_counts(config, pod_id);
    let mut count = 0;
    for service in Service::ALL {
        for month in 0..months(config, service) {
            if can_read(config, pod_id, service, month, agent) {
                count += records_in_month(config, service, counts[&service], month);
            }
        }
    }
    Ok(count)
}

/// Return service record counts without generating RDF, for large population planning.
///
/// # Errors
/// Returns an error for invalid configuration.
pub fn planned_record_counts(
    config: &PopulationConfig,
    pod_id: u64,
) -> io::Result<BTreeMap<Service, u64>> {
    config.validate()?;
    Ok(record_counts(config, pod_id))
}

/// Emit one populated Pod as deterministic named-graph N-Quads.
///
/// Bytes are written incrementally; no server-wide deployment or per-Pod RDF string
/// is retained. Persist completed output before counting the Pod as hosted. A failed
/// write can leave a partial stream, which the caller must discard.
///
/// # Errors
/// Returns invalid-configuration and underlying writer errors.
pub fn write_pod(
    config: &PopulationConfig,
    pod_id: u64,
    model: PolicyModel,
    writer: impl Write,
) -> io::Result<PodSummary> {
    config.validate()?;
    let root = pod_root(config, pod_id);
    let owner = owner_webid(config, pod_id);
    let class = intensity(config, pod_id);
    let mut out = Output {
        writer,
        bytes: 0,
        quads: 0,
    };
    let mut summary = PodSummary {
        pod_id,
        root: root.clone(),
        content_graphs: 2,
        intensity_numerator: class.numerator,
        intensity_denominator: class.denominator,
        ..PodSummary::default()
    };
    out.iri(&root, RDF_TYPE, &format!("{LDP}Container"), &root)?;
    write_policy(&mut out, config, pod_id, model, &root, None, false, true)?;
    summary.policy_graphs += 1;
    let group_doc = format!("{root}groups.ttl");
    let group = format!("{group_doc}#trusted");
    out.iri(&root, &format!("{LDP}contains"), &group_doc, &root)?;
    out.iri(&group, RDF_TYPE, &format!("{VCARD}Group"), &group_doc)?;
    for member in 0..config.group_members {
        out.iri(
            &group,
            &format!("{VCARD}hasMember"),
            &recipient_webid(config, pod_id, member),
            &group_doc,
        )?;
    }
    let counts = record_counts(config, pod_id);
    for service in Service::ALL {
        let container = format!("{root}{}/", service.name());
        out.iri(&root, &format!("{LDP}contains"), &container, &root)?;
        out.iri(&container, RDF_TYPE, &format!("{LDP}Container"), &container)?;
        summary.content_graphs += 1;
        write_policy(
            &mut out,
            config,
            pod_id,
            model,
            &container,
            Some(service),
            false,
            true,
        )?;
        summary.policy_graphs += 1;
        let count = counts[&service];
        summary.records += count;
        summary
            .records_by_service
            .insert(service.name().into(), count);
        let mut record = 0;
        for month in 0..months(config, service) {
            let doc = format!("{container}m{month:04}.ttl");
            let n = records_in_month(config, service, count, month);
            if n == 0 {
                continue;
            }
            out.iri(&container, &format!("{LDP}contains"), &doc, &container)?;
            summary.content_graphs += 1;
            if private_exception(config, service, month) {
                write_policy(
                    &mut out,
                    config,
                    pod_id,
                    model,
                    &doc,
                    Some(service),
                    true,
                    false,
                )?;
                summary.policy_graphs += 1;
            }
            for local in 0..n {
                let before = out.quads;
                write_record(
                    &mut out,
                    config,
                    &root,
                    &owner,
                    &doc,
                    service,
                    record,
                    month,
                    local,
                    counts[&Service::Contacts],
                )?;
                summary.content_triples += out.quads - before;
                record += 1;
            }
        }
    }
    summary.bytes = out.bytes;
    summary.quads = out.quads;
    Ok(summary)
}

/// Return eight queries covering point, joins, aggregation, filtering and bounded lists.
#[must_use]
pub fn benchmark_queries(config: &PopulationConfig, pod_id: u64) -> Vec<PopulationQuery> {
    let root = pod_root(config, pod_id);
    let ns = format!("PREFIX p: <{VOCAB}> ");
    [
        ("q1-point", format!("SELECT ?kind WHERE {{ GRAPH <{root}communication/m0000.ttl> {{ <{root}communication/m0000.ttl#r0> p:service ?kind }} }}")),
        ("q2-count", "SELECT (COUNT(?s) AS ?count) WHERE { ?s a p:Record }".into()),
        ("q3-star", "SELECT ?s ?created ?value WHERE { ?s p:service p:transactions; p:created ?created; p:value ?value } ORDER BY DESC(?created) LIMIT 20".into()),
        ("q4-join", "SELECT ?message ?name WHERE { ?message p:service p:communication; p:contact ?contact . ?contact p:name ?name } LIMIT 20".into()),
        ("q5-aggregate", "SELECT ?service (COUNT(?s) AS ?count) WHERE { ?s p:service ?service } GROUP BY ?service".into()),
        ("q6-filter", "SELECT ?s ?v WHERE { ?s p:service p:activity; p:value ?v . FILTER(?v > 5000) } LIMIT 20".into()),
        ("q7-optional", "SELECT ?s ?caption WHERE { ?s p:service p:media . OPTIONAL { ?s p:caption ?caption } } LIMIT 20".into()),
        ("q8-union", "SELECT ?s WHERE { { ?s p:service p:calendar } UNION { ?s p:service p:media } } LIMIT 20".into()),
    ].into_iter().map(|(id, sparql)| PopulationQuery { id: id.into(), sparql: ns.clone() + &sparql }).collect()
}

fn mix(mut n: u64) -> u64 {
    // SplitMix64 finalizer: fixed arithmetic makes each Pod independent of iteration order.
    n = (n ^ (n >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    n = (n ^ (n >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    n ^ (n >> 31)
}

fn intensity(config: &PopulationConfig, pod_id: u64) -> &VolumeClass {
    let sum: u64 = config
        .volume_classes
        .iter()
        .map(|c| u64::from(c.weight))
        .sum();
    let mut target = mix(config.seed ^ pod_id) % sum;
    for class in &config.volume_classes {
        if target < u64::from(class.weight) {
            return class;
        }
        target -= u64::from(class.weight);
    }
    unreachable!("validated positive class weights cover the sampled index")
}

fn monthly(config: &PopulationConfig, service: Service) -> u32 {
    let m = &config.monthly_records;
    match service {
        Service::Communication => m.communication,
        Service::Calendar => m.calendar,
        Service::Transactions => m.transactions,
        Service::Activity => m.activity,
        Service::Location => m.location,
        Service::Media => m.media,
        Service::Contacts | Service::Ratings => 0,
    }
}

fn record_counts(config: &PopulationConfig, pod_id: u64) -> BTreeMap<Service, u64> {
    let class = intensity(config, pod_id);
    let scaled = |n: u64| {
        if n == 0 {
            0
        } else {
            (n * u64::from(class.numerator) / u64::from(class.denominator)).max(1)
        }
    };
    let total_users = config
        .rating_count_cdf
        .last()
        .expect("validated nonempty CDF")
        .1;
    let user = mix(config.seed ^ pod_id ^ 0x5241_5449_4e47_5301) % u64::from(total_users);
    let rating_count = config
        .rating_count_cdf
        .iter()
        .find(|&&(_, cumulative)| user < u64::from(cumulative))
        .expect("validated cumulative population contains sampled user")
        .0;
    Service::ALL
        .into_iter()
        .map(|s| {
            (
                s,
                match s {
                    Service::Contacts => scaled(u64::from(config.contacts_snapshot)),
                    Service::Ratings => u64::from(rating_count),
                    _ => scaled(u64::from(monthly(config, s)) * u64::from(config.history_months)),
                },
            )
        })
        .collect()
}

fn months(config: &PopulationConfig, service: Service) -> u32 {
    if service == Service::Contacts {
        1
    } else {
        config.history_months
    }
}

fn records_in_month(config: &PopulationConfig, service: Service, total: u64, month: u32) -> u64 {
    let n = u64::from(months(config, service));
    total / n + u64::from(u64::from(month) < total % n)
}

fn private_exception(config: &PopulationConfig, service: Service, month: u32) -> bool {
    config.sharing_enabled
        && matches!(service, Service::Calendar | Service::Media)
        && config.private_exception_every > 0
        && (month + 1).is_multiple_of(config.private_exception_every)
}

struct Output<W> {
    writer: W,
    bytes: u64,
    quads: u64,
}

impl<W: Write> Output<W> {
    fn quad(&mut self, s: &str, p: &str, object: &str, g: &str) -> io::Result<()> {
        let line = format!("<{s}> <{p}> {object} <{g}> .\n");
        self.writer.write_all(line.as_bytes())?;
        self.bytes += line.len() as u64;
        self.quads += 1;
        Ok(())
    }
    fn iri(&mut self, s: &str, p: &str, o: &str, g: &str) -> io::Result<()> {
        self.quad(s, p, &format!("<{o}>"), g)
    }
    fn text(&mut self, s: &str, p: &str, value: &str, g: &str) -> io::Result<()> {
        self.quad(s, p, &format!("\"{value}\""), g)
    }
    fn integer(&mut self, s: &str, p: &str, value: u64, g: &str) -> io::Result<()> {
        self.quad(
            s,
            p,
            &format!("\"{value}\"^^<http://www.w3.org/2001/XMLSchema#integer>"),
            g,
        )
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "record writer carries scalar streaming state, not a retained record collection"
)]
fn write_record<W: Write>(
    out: &mut Output<W>,
    config: &PopulationConfig,
    root: &str,
    owner: &str,
    doc: &str,
    service: Service,
    record: u64,
    month: u32,
    local: u64,
    contacts: u64,
) -> io::Result<()> {
    let subject = format!("{doc}#r{record}");
    let contact = format!("{root}contacts/m0000.ttl#r{}", record % contacts);
    let absolute_month = 2026 * 12 - config.history_months + month;
    let date = format!(
        "{:04}-{:02}-{:02}T12:00:00Z",
        absolute_month / 12,
        absolute_month % 12 + 1,
        local % 28 + 1
    );
    out.iri(&subject, RDF_TYPE, &format!("{VOCAB}Record"), doc)?;
    out.iri(
        &subject,
        &format!("{VOCAB}service"),
        &format!("{VOCAB}{}", service.name()),
        doc,
    )?;
    out.iri(&subject, &format!("{VOCAB}owner"), owner, doc)?;
    out.integer(&subject, &format!("{VOCAB}sequence"), record, doc)?;
    out.quad(
        &subject,
        &format!("{VOCAB}created"),
        &format!("\"{date}\"^^<http://www.w3.org/2001/XMLSchema#dateTime>"),
        doc,
    )?;
    out.iri(&subject, &format!("{VOCAB}contact"), &contact, doc)?;
    let value = mix(config.seed ^ record ^ u64::from(month));
    out.integer(&subject, &format!("{VOCAB}value"), value % 10000, doc)?;
    write_record_details(out, root, &subject, doc, service, record, value)
}

fn write_record_details<W: Write>(
    out: &mut Output<W>,
    root: &str,
    subject: &str,
    doc: &str,
    service: Service,
    record: u64,
    value: u64,
) -> io::Result<()> {
    match service {
        Service::Communication => {
            out.text(
                subject,
                &format!("{VOCAB}subject"),
                &format!("Synthetic conversation {}", record / 5),
                doc,
            )?;
            out.text(subject, &format!("{VOCAB}text"), "Synthetic message excerpt; full message-body volume is not calibrated in this scenario.", doc)?;
            out.iri(
                subject,
                &format!("{VOCAB}thread"),
                &format!("{root}communication/thread/{}", record / 5),
                doc,
            )?;
        }
        Service::Contacts => {
            out.text(
                subject,
                &format!("{VOCAB}name"),
                &format!("Synthetic contact {record}"),
                doc,
            )?;
            out.text(
                subject,
                &format!("{VOCAB}email"),
                &format!("contact{record}@example.invalid"),
                doc,
            )?;
        }
        Service::Calendar => {
            out.text(
                subject,
                &format!("{VOCAB}title"),
                &format!("Synthetic appointment {record}"),
                doc,
            )?;
            out.integer(
                subject,
                &format!("{VOCAB}durationMinutes"),
                30 + (value % 4) * 30,
                doc,
            )?;
        }
        Service::Transactions => {
            out.text(subject, &format!("{VOCAB}currency"), "GBP", doc)?;
            out.iri(
                subject,
                &format!("{VOCAB}account"),
                &format!("{root}accounts/{}", record % 3),
                doc,
            )?;
            out.text(
                subject,
                &format!("{VOCAB}direction"),
                if record.is_multiple_of(10) {
                    "credit"
                } else {
                    "debit"
                },
                doc,
            )?;
        }
        Service::Activity => {
            out.text(subject, &format!("{VOCAB}unit"), "steps", doc)?;
            out.integer(subject, &format!("{VOCAB}activeMinutes"), value % 180, doc)?;
        }
        Service::Location => {
            out.text(subject, &format!("{VOCAB}sampling"), "episodic", doc)?;
            out.integer(
                subject,
                &format!("{VOCAB}latitudeMicrodegrees"),
                51_000_000 + value % 1_000_000,
                doc,
            )?;
            out.integer(
                subject,
                &format!("{VOCAB}longitudeMicrodegrees"),
                value % 1_000_000,
                doc,
            )?;
        }
        Service::Media | Service::Ratings => {
            write_media_details(out, subject, doc, service, record, value)?;
        }
    }
    Ok(())
}

fn write_media_details<W: Write>(
    out: &mut Output<W>,
    subject: &str,
    doc: &str,
    service: Service,
    record: u64,
    value: u64,
) -> io::Result<()> {
    match service {
        Service::Media => {
            out.text(
                subject,
                &format!("{VOCAB}mimeType"),
                if record.is_multiple_of(10) {
                    "video/mp4"
                } else {
                    "image/jpeg"
                },
                doc,
            )?;
            out.text(
                subject,
                &format!("{VOCAB}filename"),
                &format!("synthetic-{record}.bin"),
                doc,
            )?;
            if record.is_multiple_of(3) {
                out.text(
                    subject,
                    &format!("{VOCAB}caption"),
                    "Synthetic retained media description",
                    doc,
                )?;
            }
        }
        Service::Ratings => {
            out.iri(
                subject,
                &format!("{VOCAB}item"),
                &format!("https://catalog.example/movie/{}", record % 3952),
                doc,
            )?;
            out.integer(subject, &format!("{VOCAB}stars"), 1 + value % 5, doc)?;
        }
        _ => unreachable!("media detail writer only receives media or ratings"),
    }
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "policy compiler uses explicit resource and scope, independent of the neutral decision oracle"
)]
fn write_policy<W: Write>(
    out: &mut Output<W>,
    config: &PopulationConfig,
    pod_id: u64,
    model: PolicyModel,
    resource: &str,
    service: Option<Service>,
    exception: bool,
    descendants: bool,
) -> io::Result<()> {
    let owner = owner_webid(config, pod_id);
    let shared = config.sharing_enabled
        && matches!(
            service,
            Some(Service::Calendar | Service::Media | Service::Ratings)
        );
    let suffix = if model == PolicyModel::Wac {
        "acl"
    } else {
        "acr"
    };
    let graph = format!("{resource}.{suffix}");
    if model == PolicyModel::Wac {
        wac_grant(
            out,
            &graph,
            "owner",
            resource,
            descendants,
            "agent",
            &owner,
            &["Read", "Write", "Append", "Control"],
        )?;
        if shared && !exception {
            let (predicate, audience) = match service {
                Some(Service::Calendar) => ("agent", recipient_webid(config, pod_id, 0)),
                Some(Service::Media) => (
                    "agentGroup",
                    format!("{}groups.ttl#trusted", pod_root(config, pod_id)),
                ),
                _ => ("agentClass", "http://xmlns.com/foaf/0.1/Agent".into()),
            };
            wac_grant(
                out,
                &graph,
                "reader",
                resource,
                descendants,
                predicate,
                &audience,
                &["Read"],
            )?;
        }
    } else {
        out.iri(
            &graph,
            RDF_TYPE,
            &format!("{ACP}AccessControlResource"),
            &graph,
        )?;
        out.iri(&graph, &format!("{ACP}resource"), resource, &graph)?;
        acp_policy(
            out,
            &graph,
            "owner",
            descendants,
            &[owner],
            false,
            &["Read", "Write", "Append", "Control"],
        )?;
        if shared {
            let agents = match service {
                Some(Service::Calendar) => vec![recipient_webid(config, pod_id, 0)],
                Some(Service::Media) => (0..config.group_members)
                    .map(|i| recipient_webid(config, pod_id, i))
                    .collect(),
                _ => vec![format!("{ACP}PublicAgent")],
            };
            // A private exception shadows WAC inheritance. ACP instead denies only
            // the inherited named recipients; owner access is preserved in both.
            acp_policy(
                out,
                &graph,
                "reader",
                descendants,
                &agents,
                exception,
                &["Read"],
            )?;
        }
    }
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "serialization helper receives all explicit authorization components"
)]
fn wac_grant<W: Write>(
    out: &mut Output<W>,
    graph: &str,
    name: &str,
    resource: &str,
    descendants: bool,
    predicate: &str,
    audience: &str,
    modes: &[&str],
) -> io::Result<()> {
    let node = format!("{graph}#{name}");
    out.iri(&node, RDF_TYPE, &format!("{ACL}Authorization"), graph)?;
    out.iri(&node, &format!("{ACL}accessTo"), resource, graph)?;
    if descendants {
        out.iri(&node, &format!("{ACL}default"), resource, graph)?;
    }
    out.iri(&node, &format!("{ACL}{predicate}"), audience, graph)?;
    for mode in modes {
        out.iri(&node, &format!("{ACL}mode"), &format!("{ACL}{mode}"), graph)?;
    }
    Ok(())
}

fn acp_policy<W: Write>(
    out: &mut Output<W>,
    graph: &str,
    name: &str,
    descendants: bool,
    agents: &[String],
    deny: bool,
    modes: &[&str],
) -> io::Result<()> {
    let control = format!("{graph}#{name}-control");
    let policy = format!("{graph}#{name}-policy");
    let matcher = format!("{graph}#{name}-matcher");
    out.iri(graph, &format!("{ACP}accessControl"), &control, graph)?;
    if descendants {
        out.iri(graph, &format!("{ACP}memberAccessControl"), &control, graph)?;
    }
    out.iri(&control, &format!("{ACP}apply"), &policy, graph)?;
    out.iri(&policy, RDF_TYPE, &format!("{ACP}Policy"), graph)?;
    out.iri(&policy, &format!("{ACP}anyOf"), &matcher, graph)?;
    out.iri(&matcher, RDF_TYPE, &format!("{ACP}Matcher"), graph)?;
    for agent in agents {
        out.iri(&matcher, &format!("{ACP}agent"), agent, graph)?;
    }
    let effect = if deny { "deny" } else { "allow" };
    for mode in modes {
        out.iri(
            &policy,
            &format!("{ACP}{effect}"),
            &format!("{ACL}{mode}"),
            graph,
        )?;
    }
    Ok(())
}
