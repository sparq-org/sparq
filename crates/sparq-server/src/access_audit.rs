//! [OPUS-4.8] (sq-gos8, epic sq-toze) **Opt-in RICHER structured access-audit sink** — a
//! pluggable sink that records every enforced access decision as a structured, typed event
//! for a security / compliance audit trail. Maps to ASVS V7 (logging) / ISO 27001 A.8.15
//! (logging) / CDMC CD-2 (access audit).
//!
//! This is the heavier sibling of the `crate::audit` module (`audit-log`). Where `audit-log`
//! emits a single flat `tracing` event per request (requester fingerprint, op class, query
//! fingerprint, decision, status, duration) for operators who route a `tracing` target to a
//! sink, THIS module emits a **typed, self-describing access RECORD** through a **pluggable
//! sink trait** (`AuditSink`) — capturing more of the access-decision context that a
//! compliance audit trail needs:
//!
//!   * **actor** — the authenticated subject (a WebID / agent IRI when known, else a Bearer
//!     token fingerprint, else `anonymous`),
//!   * **action** — read / write / query / update / graph-read / graph-write,
//!   * **resource** — the dataset / named-graph IRI the request touched (when the surface
//!     names one), else the request path,
//!   * **decision** — allow / deny, with the **policy basis** that produced it (the actual
//!     enforcement reason: the Bearer-auth gate, etc.),
//!   * **timestamp** — RFC-3339-ish UTC wall-clock, plus a monotonic `duration_us`,
//!   * a **request fingerprint** — the SAME FNV-1a construction the redaction / `audit-log`
//!     fingerprints use (`fingerprint`), so an operator can correlate this record with the
//!     `audit-log` trail and with redacted error logs.
//!
//! ## Opt-in, lean, zero-cost-when-off
//!
//! The whole module is gated behind the `access-audit` cargo feature **and** a runtime sink:
//! [`AppState`](crate::http::AppState) carries an `Option<Arc<dyn AuditSink>>` that is `None`
//! unless the operator configured one (`--access-audit <target>` / `SPARQ_ACCESS_AUDIT`).
//! With the feature OFF the module is not compiled and every call site is `#[cfg]`-stripped,
//! so a request pays exactly zero. With the feature ON but no sink configured, a single
//! `Option` check short-circuits before any record is built.
//!
//! ## Privacy boundary — STATED EXPLICITLY
//!
//! An audit trail exists **precisely to record WHO accessed WHAT and whether it was allowed**,
//! so — by design, and unlike the request log — this sink **DOES record identities and
//! resources**: the actor (WebID / agent IRI / token fingerprint) and the resource IRI are
//! first-class fields. That is the point of an audit trail and is the operator's deliberate,
//! opt-in choice.
//!
//! What it **does NOT** record is query **CONTENT**. The query / update text is recorded only
//! as its non-reversible `fingerprint` — never the raw SPARQL — because (per the #241
//! info-leak posture and the sq-toze.34 redaction work) a query body can itself carry PII (a
//! patient IRI in a `FILTER`, an email literal) or disclose loaded-data fragments. The sink
//! therefore does **not** double-log the content the redaction work just protected. The
//! boundary in one line: **identities + resources are logged (that is the audit trail);
//! content stays fingerprinted.**

use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// A 64-bit FNV-1a hash, hex-encoded — the SAME construction used by `crate::audit` and the
/// error-redaction fingerprints, so a fingerprint produced here correlates byte-for-byte with
/// those trails. Fixed + dependency-free (not [`std::hash::DefaultHasher`], whose output is
/// unspecified across builds), so the same input yields the same fingerprint in every process
/// and across restarts. One-way: it records *that the same thing recurred*, never *what it was*.
fn fnv1a_hex(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

/// The non-reversible fingerprint of a query / update body — leading/trailing ASCII whitespace
/// trimmed (light normalisation), then FNV-1a-hashed. Deliberately NOT the raw text: that is
/// the privacy boundary (see the module docs). Public so call sites and tests agree on it.
pub fn fingerprint(sparql: &str) -> String {
    fnv1a_hex(sparql.trim().as_bytes())
}

/// The CLASS of action the request performs against the dataset — coarser than the HTTP route,
/// finer than read/write, so an audit reviewer can answer "who issued UPDATEs / wrote graphs"
/// without parsing query text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// A SPARQL query (SELECT / ASK / CONSTRUCT / DESCRIBE).
    Query,
    /// A SPARQL Update (mutates the dataset).
    Update,
    /// A Graph-Store-Protocol READ (`GET`/`HEAD`).
    GraphRead,
    /// A Graph-Store-Protocol WRITE (`PUT`/`POST`/`DELETE`/`PATCH`).
    GraphWrite,
}

impl Action {
    /// The stable string recorded in the `action` field.
    pub fn as_str(self) -> &'static str {
        match self {
            Action::Query => "query",
            Action::Update => "update",
            Action::GraphRead => "graph_read",
            Action::GraphWrite => "graph_write",
        }
    }
}

/// The enforced access decision recorded in the `decision` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessDecision {
    /// The request passed the enforcement gate and was processed.
    Allow,
    /// The request was refused at the enforcement gate.
    Deny,
}

impl AccessDecision {
    /// The stable string recorded in the `decision` field.
    pub fn as_str(self) -> &'static str {
        match self {
            AccessDecision::Allow => "allow",
            AccessDecision::Deny => "deny",
        }
    }
}

/// The authenticated subject. Recorded by design (an audit trail attributes access to a
/// subject) — but a Bearer token is a shared secret, so it is recorded as its FNV-1a
/// fingerprint, never the secret itself. A WebID / agent IRI is an identifier (not a secret),
/// so it is recorded verbatim — that is the whole value of the trail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Actor {
    /// No credential was presented / required — the request was processed anonymously, OR it
    /// was denied (a denied request never authenticated).
    Anonymous,
    /// A Bearer token was presented — recorded as its non-reversible fingerprint, never the
    /// secret. Distinct tokens get distinct fingerprints (subject correlation).
    TokenFingerprint(String),
    /// An authenticated WebID / agent IRI (e.g. from a Solid / ACP identity layer). Recorded
    /// verbatim: it is an identifier the audit trail is meant to attribute access to.
    WebId(String),
}

impl Actor {
    /// Derives the actor from a presented Bearer token (the channel available on the plain
    /// HTTP / GSP surface today). `None`/empty => [`Actor::Anonymous`]. A future Solid / ACP
    /// identity layer constructs [`Actor::WebId`] directly at its enforcement seam (see
    /// [`from_session`](Actor::from_session)).
    pub fn from_token(presented: Option<&str>) -> Self {
        match presented {
            Some(t) if !t.is_empty() => Actor::TokenFingerprint(fnv1a_hex(t.as_bytes())),
            _ => Actor::Anonymous,
        }
    }

    /// [OPUS-4.8] (sq-ljfz, sq-gos8 follow-up) Derives the actor from the *authenticated
    /// session* of a fronting authorization layer, falling back to the local Bearer gate.
    ///
    /// The deployment pattern this serves is the one the bind-time warnings already name: a
    /// trusted front — `sparq-solid`, or any Solid/WAC reverse-proxy / identity gateway —
    /// authenticates the user (resolves their WebID from a WAC/ACP session) and forwards that
    /// resolved WebID to this upstream server as a request header. When the operator has
    /// configured the server to trust such a header, the header's value IS the authenticated
    /// subject, so the audit trail records [`Actor::WebId`] (the actual identity) instead of
    /// the coarse Bearer-token fingerprint — which is the whole point of the bead: attribute
    /// access to *who*, not to *which shared secret*.
    ///
    /// **Trust precondition (audit-integrity critical).** A forwarded header is
    /// client-controllable on the wire, so honouring it unconditionally would let any caller
    /// spoof an arbitrary WebID into the audit trail. The caller therefore passes the
    /// forwarded value ONLY when the operator has explicitly opted into trusting a named
    /// header (`--audit-webid-header` / `SPARQ_AUDIT_WEBID_HEADER`), asserting that a trusted
    /// front sets/overwrites it. With no trusted header configured the caller passes `None`
    /// here and this is byte-identical to [`from_token`](Actor::from_token).
    ///
    /// `forwarded_webid`: the value of the trusted forwarded-identity header (already
    /// extracted by the caller; `None` when no trusted header is configured or it is absent).
    /// A present, non-empty, whitespace-trimmed value becomes [`Actor::WebId`]; an empty /
    /// whitespace-only value is treated as "no identity forwarded" and falls through to the
    /// Bearer gate, so a front that emits an empty header never silently records a blank WebID.
    pub fn from_session(forwarded_webid: Option<&str>, bearer: Option<&str>) -> Self {
        match forwarded_webid.map(str::trim) {
            Some(w) if !w.is_empty() => Actor::WebId(w.to_string()),
            _ => Actor::from_token(bearer),
        }
    }

    /// The stable string recorded in the `actor` field.
    pub fn as_field(&self) -> String {
        match self {
            Actor::Anonymous => "anonymous".to_string(),
            Actor::TokenFingerprint(fp) => format!("token:{fp}"),
            Actor::WebId(iri) => format!("webid:{iri}"),
        }
    }
}

/// The resource the request addressed. For the Graph-Store surface this is the named-graph
/// IRI (or `default`); for the plain SPARQL surface the dataset itself (no per-graph IRI is
/// known at the request boundary), recorded as the request path. Recorded by design — a
/// compliance reviewer needs to know WHAT was accessed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resource {
    /// The default graph / whole dataset (the plain `/sparql` surface, or GSP `?default`).
    Dataset(String),
    /// A specific named graph, identified by its IRI.
    NamedGraph(String),
}

impl Resource {
    /// The stable string recorded in the `resource` field.
    pub fn as_field(&self) -> String {
        match self {
            Resource::Dataset(p) => p.clone(),
            Resource::NamedGraph(iri) => iri.clone(),
        }
    }

    /// The kind discriminator recorded in the `resource_kind` field.
    pub fn kind(&self) -> &'static str {
        match self {
            Resource::Dataset(_) => "dataset",
            Resource::NamedGraph(_) => "named_graph",
        }
    }
}

/// A fully-assembled, structured access-audit event — the typed record the [`AuditSink`]
/// persists. Every field is either a non-secret identifier (logged by design) or a
/// non-reversible fingerprint (content protected). Built at the real enforcement seam so the
/// `decision` + `policy_basis` are the ones ACTUALLY enforced.
#[derive(Debug, Clone)]
pub struct AccessEvent {
    /// UTC wall-clock, RFC-3339-ish (`YYYY-MM-DDTHH:MM:SS.mmmZ`), for the audit timeline.
    pub timestamp: String,
    /// The authenticated subject (WebID / token fingerprint / anonymous).
    pub actor: Actor,
    /// The action class (query / update / graph read / graph write).
    pub action: Action,
    /// The resource addressed (dataset / named-graph IRI).
    pub resource: Resource,
    /// The enforced decision (allow / deny).
    pub decision: AccessDecision,
    /// The policy basis that produced the decision — the ACTUAL enforcement reason (e.g.
    /// `"bearer-auth: allowed"` / `"bearer-auth: missing or invalid token"`). A machine-stable,
    /// caller-input-free string.
    pub policy_basis: String,
    /// Non-reversible fingerprint of the query / update body (`"-"` when there is none, e.g. a
    /// GSP read). NEVER the raw text — that is the privacy boundary.
    pub fingerprint: String,
    /// The HTTP status the request resolved to.
    pub status: u16,
    /// Wall-clock the handler took, microseconds.
    pub duration_us: u64,
}

impl AccessEvent {
    /// Serialises the event as a single JSON object (one JSON-Lines record). Uses `serde_json`
    /// (already a server dependency) to escape field values safely — a resource IRI or
    /// policy-basis string can contain characters that must be escaped to keep the line valid
    /// JSON. The keys are stable (a compliance parser binds to them).
    pub fn to_json_line(&self) -> String {
        let obj = serde_json::json!({
            "ts": self.timestamp,
            "actor": self.actor.as_field(),
            "action": self.action.as_str(),
            "resource": self.resource.as_field(),
            "resource_kind": self.resource.kind(),
            "decision": self.decision.as_str(),
            "policy_basis": self.policy_basis,
            "fingerprint": self.fingerprint,
            "status": self.status,
            "duration_us": self.duration_us,
        });
        // `to_string` on a serde_json::Value never fails.
        obj.to_string()
    }
}

/// The pluggable audit sink. The DEFAULT implementation ([`WriterSink`]) writes JSON-Lines to
/// a configured target (a file or stderr). Heavy / external sinks (a SIEM client, an
/// OpenTelemetry exporter, Kafka) stay OUT of core — an embedder implements this trait and
/// installs their own. `record` takes `&self` so the sink can be shared (`Arc<dyn AuditSink>`)
/// across the concurrent request handlers; implementations own their synchronisation.
pub trait AuditSink: Send + Sync {
    /// Persist one access event. Best-effort: a sink failure (a full disk, a closed pipe) must
    /// NOT fail the request — the audit trail is observational. Implementations log/swallow
    /// their own I/O errors.
    fn record(&self, event: &AccessEvent);
}

/// Where the default [`WriterSink`] writes.
#[derive(Debug, Clone)]
pub enum SinkTarget {
    /// Append JSON-Lines to a file at this path (created if absent).
    File(std::path::PathBuf),
    /// Write JSON-Lines to standard error (the process's diagnostic stream — kept distinct
    /// from stdout, which carries the server's own startup banner).
    Stderr,
}

impl SinkTarget {
    /// Parses a `--access-audit <target>` / `SPARQ_ACCESS_AUDIT` value: the literal `stderr`
    /// (case-insensitive) selects [`SinkTarget::Stderr`]; anything else is a file PATH.
    pub fn parse(s: &str) -> Self {
        let t = s.trim();
        if t.eq_ignore_ascii_case("stderr") {
            SinkTarget::Stderr
        } else {
            SinkTarget::File(std::path::PathBuf::from(t))
        }
    }
}

/// The default sink: serialises each event to one JSON line and writes it to the configured
/// target under a mutex (so concurrent handlers never interleave a line). Best-effort — an I/O
/// error is reported once to stderr and otherwise swallowed, never failing the request.
pub struct WriterSink {
    inner: std::sync::Mutex<WriterInner>,
}

enum WriterInner {
    File(std::fs::File),
    Stderr,
}

impl WriterSink {
    /// Opens the sink for the given target. A [`SinkTarget::File`] is opened in append mode
    /// (created if absent); a failure to open is returned so the binary can surface a clean
    /// startup error rather than silently dropping the audit trail.
    pub fn open(target: &SinkTarget) -> std::io::Result<Self> {
        let inner = match target {
            SinkTarget::File(path) => {
                let f = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)?;
                WriterInner::File(f)
            }
            SinkTarget::Stderr => WriterInner::Stderr,
        };
        Ok(Self {
            inner: std::sync::Mutex::new(inner),
        })
    }
}

impl AuditSink for WriterSink {
    fn record(&self, event: &AccessEvent) {
        use std::io::Write as _;
        let mut line = event.to_json_line();
        line.push('\n');
        // A poisoned mutex (a prior panic while holding it) must not propagate into the
        // request path; recover the guard and keep recording.
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let res = match &mut *guard {
            WriterInner::File(f) => f.write_all(line.as_bytes()),
            WriterInner::Stderr => std::io::stderr().write_all(line.as_bytes()),
        };
        if let Err(e) = res {
            // Best-effort: never fail the request because the audit sink hiccuped.
            eprintln!("sparq-server: access-audit sink write failed: {e}");
        }
    }
}

/// A running access-audit record: snapshots the start instant + the (non-reversible) request
/// fingerprint at the enforcement seam, then [`finish`](AuditPending::finish)es into an
/// [`AccessEvent`] once the decision is known. Cheap (one hash); only built when a sink is
/// installed.
pub struct AuditPending {
    started: Instant,
    action: Action,
    actor: Actor,
    resource: Resource,
    fingerprint: String,
}

impl AuditPending {
    /// Begins a record. `sparql` is the query/update body to fingerprint (`None` for a GSP
    /// read — there is no body to fingerprint).
    pub fn begin(
        action: Action,
        actor: Actor,
        resource: Resource,
        sparql: Option<&str>,
    ) -> Self {
        Self {
            started: Instant::now(),
            action,
            actor,
            resource,
            fingerprint: sparql.map(fingerprint).unwrap_or_else(|| "-".to_string()),
        }
    }

    /// Completes the record with the ACTUALLY-enforced decision + its policy basis + the HTTP
    /// status, producing the structured [`AccessEvent`] to hand to the sink.
    pub fn finish(self, decision: AccessDecision, policy_basis: &str, status: u16) -> AccessEvent {
        AccessEvent {
            timestamp: now_rfc3339(),
            actor: self.actor,
            action: self.action,
            resource: self.resource,
            decision,
            policy_basis: policy_basis.to_string(),
            fingerprint: self.fingerprint,
            status,
            duration_us: self.started.elapsed().as_micros() as u64,
        }
    }
}

/// A dependency-free UTC RFC-3339-ish timestamp (`YYYY-MM-DDTHH:MM:SS.mmmZ`). Computed from the
/// wall clock; avoids pulling `chrono`/`time` into core for one timestamp. Civil-date
/// conversion via the standard days-since-epoch algorithm (proleptic Gregorian).
fn now_rfc3339() -> String {
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = dur.as_secs();
    let millis = dur.subsec_millis();
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (hh, mm, ss) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}T{hh:02}:{mm:02}:{ss:02}.{millis:03}Z")
}

use sparq_core::civil::civil_from_days;

/// A no-op sink used in tests / by an embedder that wants the call sites live but no output.
#[derive(Debug, Default)]
pub struct NullSink;

impl AuditSink for NullSink {
    fn record(&self, _event: &AccessEvent) {}
}

/// Convenience: build a default [`WriterSink`] from a parsed target as an `Arc<dyn AuditSink>`
/// for installation into [`AppState`](crate::http::AppState).
pub fn make_sink(target: &SinkTarget) -> std::io::Result<Arc<dyn AuditSink>> {
    Ok(Arc::new(WriterSink::open(target)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// An in-memory test sink capturing the JSON lines it is handed.
    #[derive(Default)]
    struct VecSink(Mutex<Vec<String>>);
    impl AuditSink for VecSink {
        fn record(&self, event: &AccessEvent) {
            self.0.lock().unwrap().push(event.to_json_line());
        }
    }

    #[test]
    fn fingerprint_matches_audit_construction_and_trims() {
        // Same FNV-1a construction as crate::audit (basis offset 0xcbf29ce484222325).
        assert_eq!(fingerprint("SELECT * WHERE {?s ?p ?o}"), fingerprint("  SELECT * WHERE {?s ?p ?o}\n"));
        assert_ne!(fingerprint("SELECT * WHERE {?s ?p ?o}"), fingerprint("ASK {?s ?p ?o}"));
        let fp = fingerprint("anything");
        assert_eq!(fp.len(), 16);
        assert!(fp.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn actor_never_logs_the_raw_token_and_distinguishes_subjects() {
        let a = Actor::from_token(Some("super-secret"));
        assert!(matches!(a, Actor::TokenFingerprint(_)));
        let field = a.as_field();
        assert!(field.starts_with("token:"));
        assert!(!field.contains("super-secret"), "raw token must never appear");
        assert_eq!(Actor::from_token(None).as_field(), "anonymous");
        assert_eq!(Actor::from_token(Some("")).as_field(), "anonymous");
        assert_ne!(
            Actor::from_token(Some("a")).as_field(),
            Actor::from_token(Some("b")).as_field()
        );
        // WebID is recorded verbatim (an identifier, not a secret).
        assert_eq!(
            Actor::WebId("https://alice.example/profile#me".into()).as_field(),
            "webid:https://alice.example/profile#me"
        );
    }

    #[test]
    fn from_session_prefers_a_forwarded_webid_else_falls_back_to_the_bearer_gate() {
        // [OPUS-4.8] sq-ljfz: a trusted front's forwarded WebID becomes Actor::WebId (the
        // actual subject), recorded verbatim — that is the whole point of the enrichment.
        let webid = "https://alice.example/profile#me";
        assert_eq!(
            Actor::from_session(Some(webid), Some("the-shared-token")),
            Actor::WebId(webid.to_string()),
        );
        // The forwarded value is trimmed (a front may pad the header).
        assert_eq!(
            Actor::from_session(Some("  https://bob.example/#me \t"), None),
            Actor::WebId("https://bob.example/#me".to_string()),
        );
        // No forwarded identity (no trusted header configured / header absent) => byte-identical
        // to the Bearer gate: a present token fingerprints, an absent one is anonymous.
        assert_eq!(
            Actor::from_session(None, Some("tok")),
            Actor::from_token(Some("tok")),
        );
        assert_eq!(Actor::from_session(None, None), Actor::Anonymous);
        // An EMPTY / whitespace-only forwarded value is "no identity forwarded" — it must NOT
        // record a blank WebID; it falls through to the Bearer gate.
        assert_eq!(
            Actor::from_session(Some(""), Some("tok")),
            Actor::from_token(Some("tok")),
        );
        assert_eq!(Actor::from_session(Some("   "), None), Actor::Anonymous);
    }

    #[test]
    fn allow_event_has_the_structured_fields() {
        let sink = VecSink::default();
        let ev = AuditPending::begin(
            Action::Query,
            Actor::WebId("https://alice.example/#me".into()),
            Resource::Dataset("/sparql".into()),
            Some("SELECT * WHERE {?s ?p ?o}"),
        )
        .finish(AccessDecision::Allow, "bearer-auth: allowed", 200);
        sink.record(&ev);
        let line = &sink.0.lock().unwrap()[0];
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(v["actor"], "webid:https://alice.example/#me");
        assert_eq!(v["action"], "query");
        assert_eq!(v["resource"], "/sparql");
        assert_eq!(v["resource_kind"], "dataset");
        assert_eq!(v["decision"], "allow");
        assert_eq!(v["policy_basis"], "bearer-auth: allowed");
        assert_eq!(v["status"], 200);
        // Content is fingerprinted, NOT raw: the raw query text must not appear anywhere.
        assert!(!line.contains("SELECT"), "query content must never be logged raw");
        assert_eq!(v["fingerprint"], fingerprint("SELECT * WHERE {?s ?p ?o}"));
    }

    #[test]
    fn deny_event_records_the_enforced_basis() {
        let sink = VecSink::default();
        let ev = AuditPending::begin(
            Action::GraphWrite,
            Actor::Anonymous,
            Resource::NamedGraph("https://example.org/g1".into()),
            None,
        )
        .finish(
            AccessDecision::Deny,
            "bearer-auth: missing or invalid token",
            401,
        );
        sink.record(&ev);
        let v: serde_json::Value =
            serde_json::from_str(&sink.0.lock().unwrap()[0]).unwrap();
        assert_eq!(v["decision"], "deny");
        assert_eq!(v["action"], "graph_write");
        assert_eq!(v["resource"], "https://example.org/g1");
        assert_eq!(v["resource_kind"], "named_graph");
        assert_eq!(v["status"], 401);
        assert_eq!(v["fingerprint"], "-");
    }

    #[test]
    fn writer_sink_appends_json_lines_to_a_file() {
        let dir = std::env::temp_dir().join(format!("sparq-aa-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("audit.jsonl");
        let sink = WriterSink::open(&SinkTarget::File(path.clone())).unwrap();
        let ev = AuditPending::begin(
            Action::Update,
            Actor::from_token(Some("tok")),
            Resource::Dataset("/sparql".into()),
            Some("INSERT DATA { <a> <b> <c> }"),
        )
        .finish(AccessDecision::Allow, "bearer-auth: allowed", 204);
        sink.record(&ev);
        let contents = std::fs::read_to_string(&path).unwrap();
        assert_eq!(contents.lines().count(), 1);
        let v: serde_json::Value = serde_json::from_str(contents.trim()).unwrap();
        assert_eq!(v["action"], "update");
        assert!(v["actor"].as_str().unwrap().starts_with("token:"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn timestamp_is_rfc3339_z() {
        let ts = now_rfc3339();
        assert!(ts.ends_with('Z'));
        assert_eq!(ts.len(), 24, "YYYY-MM-DDTHH:MM:SS.mmmZ = 24 chars, got {ts}");
        // A known epoch day: 2021-01-01 is day 18628.
        assert_eq!(civil_from_days(18628), (2021, 1, 1));
        assert_eq!(civil_from_days(0), (1970, 1, 1));
    }

    #[test]
    fn sink_target_parses_stderr_vs_file() {
        assert!(matches!(SinkTarget::parse("stderr"), SinkTarget::Stderr));
        assert!(matches!(SinkTarget::parse("STDERR"), SinkTarget::Stderr));
        assert!(matches!(SinkTarget::parse("/var/log/a.jsonl"), SinkTarget::File(_)));
    }
}
