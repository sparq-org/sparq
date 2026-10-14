//! (sq-0bxp) **Opt-in per-query access audit log** — CDMC CD-2 (access audit),
//! composes with ISO 27001 A.8.15 (logging) and the EU CRA logging expectations.
//!
//! The CDMC audit (#234) rated access-audit a **2** because the server emits only
//! AGGREGATE metrics (`GET /metrics`) — there is NO per-subject / per-query audit trail.
//! This module adds one: for every query / update / Graph-Store request the server emits a
//! single STRUCTURED `tracing` event under the dedicated `target: "sparq_server::audit"`,
//! carrying *who* asked, *what class* of operation, a *stable query fingerprint*, the
//! *access decision*, the *result size / status*, and the *duration*. Operators route that
//! target to whatever sink their compliance regime requires (a file, journald, an
//! OpenTelemetry collector, a SIEM) via the standard `RUST_LOG` / `EnvFilter` machinery the
//! `--verbose` request log already uses.
//!
//! ## Opt-in, lean, zero-overhead-when-off
//!
//! The whole module is gated behind the `audit-log` cargo feature **and** a runtime flag
//! ([`ServerConfig::audit_log`] / `--audit-log` / `SPARQ_AUDIT_LOG=1`). With the feature OFF
//! the module is not compiled and every call site is `#[cfg]`-stripped, so a request pays
//! exactly zero — no field is captured, no timer is read, no branch is taken. With the
//! feature ON but the runtime flag OFF, a single boolean check short-circuits before any
//! record is built. Only when both are on does the server assemble and emit a record.
//!
//! ## No-PII / no-info-leak posture (reuses the #241 lesson)
//!
//! This is a **server-side** log under the operator's control — it is NEVER written to the
//! HTTP response (that is the #241 information-leak contract: detail to the server log, a
//! generic class to the client). Critically it does **not** log the full query text by
//! default: the #241 fix established that raw SPARQL can disclose loaded-data fragments and
//! caller input, and a query body can itself carry PII (a patient IRI, an email in a
//! `FILTER`). Instead the record carries a **stable non-reversible fingerprint** (a FNV-1a
//! hash of the normalised query text, hex-encoded) plus the operation class — enough to
//! correlate repeated identical queries and spot anomalous access patterns without ever
//! persisting the query content. The requester identity is likewise a fingerprint of the
//! authenticated Bearer token, never the secret itself.

use std::time::Instant;

use axum::http::StatusCode;
use axum::response::Response;

use crate::http::{AuthPosture, Operation, ServerConfig};

/// The `tracing` target every audit event is emitted under. Operators filter on it
/// independently of the request log (e.g. `RUST_LOG=sparq_server::audit=info`) to route the
/// audit trail to its own sink. Kept as a `const` so the call sites and any test subscriber
/// agree on the exact string.
pub const AUDIT_TARGET: &str = "sparq_server::audit";

/// The operation CLASS recorded in the audit record. Coarser than the HTTP route — it names
/// what the request *does* to the dataset (read vs mutate) and, for reads, the protocol
/// surface — so an operator can audit "who issued UPDATEs" without parsing query text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditOp {
    /// A SPARQL query (SELECT / ASK / CONSTRUCT / DESCRIBE) on `/sparql`.
    Query,
    /// A SPARQL Update (mutates the dataset) on `/sparql`.
    Update,
    /// A Graph-Store-Protocol READ (`GET`/`HEAD`).
    GraphRead,
    /// A Graph-Store-Protocol WRITE (`PUT`/`POST`/`DELETE`/`PATCH`).
    GraphWrite,
}

impl AuditOp {
    /// The stable string recorded in the `op` field.
    fn as_str(self) -> &'static str {
        match self {
            AuditOp::Query => "query",
            AuditOp::Update => "update",
            AuditOp::GraphRead => "graph_read",
            AuditOp::GraphWrite => "graph_write",
        }
    }

    /// Derives the audit op from the auth-classifier [`Operation`] for the plain SPARQL
    /// surface (read => a query, write => an update).
    pub(crate) fn from_sparql(op: Operation) -> Self {
        match op {
            Operation::Read => AuditOp::Query,
            Operation::Write => AuditOp::Update,
        }
    }

    /// Derives the audit op from the auth-classifier [`Operation`] for the Graph-Store surface.
    pub(crate) fn from_graph(op: Operation) -> Self {
        match op {
            Operation::Read => AuditOp::GraphRead,
            Operation::Write => AuditOp::GraphWrite,
        }
    }
}

/// A 64-bit FNV-1a hash, hex-encoded. Used for BOTH the query fingerprint and the requester
/// identity fingerprint: a fixed, dependency-free algorithm (not [`std::hash::DefaultHasher`],
/// whose output is unspecified and may differ across builds) so the same input yields the same
/// fingerprint in every process — letting an operator correlate repeated identical queries (or
/// a recurring caller) across requests and even across server restarts. It is one-way: the
/// fingerprint cannot be reversed to the query text or the token, which is the whole point —
/// the audit trail records *that the same thing recurred*, never *what it was*.
fn fnv1a_hex(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

/// A stable fingerprint of the query / update text — a hash of the text with leading and
/// trailing ASCII whitespace trimmed (a light normalisation so trivially-reformatted but
/// byte-different submissions of "the same" query still fingerprint identically at the
/// edges). Deliberately NOT the full text: per the #241 lesson, raw SPARQL can disclose
/// loaded-data fragments and caller-supplied PII, so only the non-reversible hash is recorded.
pub fn query_fingerprint(sparql: &str) -> String {
    fnv1a_hex(sparql.trim().as_bytes())
}

/// The requester identity recorded in the `requester` field — derived WITHOUT logging the
/// Bearer token (a shared secret) or any caller-supplied text.
///
///   * No token configured for this surface (`posture`/`op` leaves it ungated), OR a gated
///     request that presented no/invalid credential but was nonetheless processed — recorded
///     as `anonymous`. (A *denied* request is recorded as `anonymous` too: it never
///     authenticated.)
///   * A gated request that presented a token — recorded as `token:<fingerprint>`, the
///     FNV-1a hash of the presented token. Distinct tokens get distinct fingerprints (so an
///     operator can attribute activity to a subject / correlate a caller across requests)
///     while the secret itself is never written to the log.
pub fn requester_identity(presented_token: Option<&str>) -> String {
    match presented_token {
        Some(tok) if !tok.is_empty() => format!("token:{}", fnv1a_hex(tok.as_bytes())),
        _ => "anonymous".to_string(),
    }
}

/// The access decision recorded in the `decision` field, with a reason on a denial.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// The request passed the auth gate and was processed.
    Allowed,
    /// The request was refused at the auth gate (401) — missing or invalid Bearer token.
    DeniedAuth,
}

impl Decision {
    fn as_str(self) -> &'static str {
        match self {
            Decision::Allowed => "allowed",
            Decision::DeniedAuth => "denied",
        }
    }

    /// The machine-stable reason string for a denial (`""` when allowed). Kept generic — it
    /// never carries caller input — so it is safe even though this is an operator log.
    fn reason(self) -> &'static str {
        match self {
            Decision::Allowed => "",
            Decision::DeniedAuth => "auth: missing or invalid Bearer token",
        }
    }
}

/// A per-request audit record, assembled at the request boundary and emitted ONCE via
/// [`AuditRecord::emit`]. Captured fields are all non-reversible / non-PII (see the module
/// docs). The timer starts at construction so `duration` measures the whole handler.
pub struct AuditRecord {
    started: Instant,
    op: AuditOp,
    requester: String,
    fingerprint: String,
}

impl AuditRecord {
    /// Begins an audit record: snapshots the start instant and the (non-reversible) requester
    /// and query fingerprints. Cheap (two small hashes); only ever called when the feature AND
    /// the runtime flag are on (the caller guards with [`enabled`]).
    pub fn begin(op: AuditOp, presented_token: Option<&str>, sparql: Option<&str>) -> Self {
        Self {
            started: Instant::now(),
            op,
            requester: requester_identity(presented_token),
            fingerprint: sparql
                .map(query_fingerprint)
                .unwrap_or_else(|| "-".to_string()),
        }
    }

    /// Emits the audit event under [`AUDIT_TARGET`], deriving the decision + status from the
    /// finished [`Response`]: a `401` is a denial (`denied`), anything else is `allowed`. The
    /// HTTP status code is recorded as the outcome (`status`); for a successful SELECT the
    /// engine's row count is not cheaply available post-hoc, so the status class is the
    /// authoritative outcome field (an operator correlates the fingerprint with the request
    /// log for finer detail). `tracing::info!` so the record sits at a routable level distinct
    /// from the `debug` request log.
    pub fn emit(self, resp: &Response) {
        let status = resp.status();
        let decision = if status == StatusCode::UNAUTHORIZED {
            Decision::DeniedAuth
        } else {
            Decision::Allowed
        };
        self.emit_with(decision, status, None);
    }

    /// Emits the audit event with an explicit decision / status / optional row count — used by
    /// the SELECT path, which knows its exact result-row count, and by call sites that refuse a
    /// request before a `Response` exists. `row_count` is `None` when not applicable (an
    /// update, a denial, a non-SELECT form); when present it records the exact number of result
    /// rows returned.
    pub fn emit_with(self, decision: Decision, status: StatusCode, row_count: Option<u64>) {
        tracing::info!(
            target: AUDIT_TARGET,
            requester = %self.requester,
            op = self.op.as_str(),
            fingerprint = %self.fingerprint,
            decision = decision.as_str(),
            reason = decision.reason(),
            status = status.as_u16(),
            rows = row_count,
            duration_us = self.started.elapsed().as_micros() as u64,
            "access audit",
        );
    }
}

/// Whether the audit log should run for this request: the runtime flag (the feature is
/// already proven on by the fact this function compiles). A single boolean read — when the
/// flag is off the caller skips ALL record construction, so audit-disabled requests pay only
/// this branch.
#[inline]
pub fn enabled(config: &ServerConfig) -> bool {
    config.audit_log
}

/// The auth posture is needed only to decide whether a presented token is meaningful; exposed
/// for the call sites that already hold a [`ServerConfig`]. (Kept as a thin accessor so the
/// http module's call sites stay symmetric with the auth gate.)
#[allow(dead_code)]
pub(crate) fn posture(config: &ServerConfig) -> AuthPosture {
    AuthPosture::from_config(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_stable_and_trims() {
        // Same logical query, different edge whitespace => identical fingerprint.
        assert_eq!(
            query_fingerprint("SELECT * WHERE {?s ?p ?o}"),
            query_fingerprint("  SELECT * WHERE {?s ?p ?o}\n")
        );
        // Different queries => different fingerprints (overwhelmingly likely).
        assert_ne!(
            query_fingerprint("SELECT * WHERE {?s ?p ?o}"),
            query_fingerprint("ASK {?s ?p ?o}")
        );
        // Hex, 16 chars (64-bit).
        let fp = query_fingerprint("anything");
        assert_eq!(fp.len(), 16);
        assert!(fp.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn requester_identity_never_logs_the_token() {
        let id = requester_identity(Some("super-secret-token"));
        assert!(id.starts_with("token:"));
        assert!(
            !id.contains("super-secret-token"),
            "the raw token must never appear in the identity"
        );
        // Anonymous when no / empty token.
        assert_eq!(requester_identity(None), "anonymous");
        assert_eq!(requester_identity(Some("")), "anonymous");
        // Distinct tokens => distinct fingerprints (subject attribution).
        assert_ne!(
            requester_identity(Some("token-a")),
            requester_identity(Some("token-b"))
        );
    }

    #[test]
    fn op_strings_are_stable() {
        assert_eq!(AuditOp::Query.as_str(), "query");
        assert_eq!(AuditOp::Update.as_str(), "update");
        assert_eq!(AuditOp::GraphRead.as_str(), "graph_read");
        assert_eq!(AuditOp::GraphWrite.as_str(), "graph_write");
        assert_eq!(AuditOp::from_sparql(Operation::Read), AuditOp::Query);
        assert_eq!(AuditOp::from_sparql(Operation::Write), AuditOp::Update);
        assert_eq!(AuditOp::from_graph(Operation::Read), AuditOp::GraphRead);
        assert_eq!(AuditOp::from_graph(Operation::Write), AuditOp::GraphWrite);
    }
}
