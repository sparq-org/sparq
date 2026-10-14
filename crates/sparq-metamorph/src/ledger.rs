//! **Found-bug ledger** — the machine-checked record format for bugs the campaign
//! finds, with a REQUIRED upstream issue link and developer-confirmation status per
//! entry. sq-gum8.6
//!
//! The ledger is the paper's evidence artifact: a bug only counts once it is filed
//! upstream and its confirmation state is tracked honestly (the SQLancer/GDsmith
//! papers count *developer-confirmed* bugs, not raw oracle alarms). The format
//! enforces that discipline structurally — [`validate_entry`] rejects an entry with no
//! issue URL, and [`ConfirmationStatus`] is a required field with no default, so a
//! serialised entry cannot omit it.
//!
//! # Schema (JSON Lines: one [`FoundBug`] JSON object per line)
//!
//! | field | type | meaning |
//! |-------|------|---------|
//! | `id` | string, non-empty, unique | campaign-local identifier (e.g. `sparq-tlp-0001`) |
//! | `engine` | string, non-empty | engine name as reported by the driver |
//! | `engine_version` | string, non-empty | exact version/commit tested |
//! | `oracle` | `"tlp"` / `"norec"` / `"differential"` | which oracle flagged it |
//! | `seed` | u64 or null | generator seed reproducing the case (null for hand-reduced cases) |
//! | `query` | string, non-empty | the (reduced) failing query |
//! | `data_ref` | string | the dataset: inline N-Triples or a repo-relative path |
//! | `classification` | `"wrong-result"` / `"engine-error"` | strict class split — see below |
//! | `summary` | string, non-empty | one-line description of the disagreement |
//! | `upstream_issue` | string, **required `http(s)` URL** | the filed upstream issue |
//! | `confirmation` | see [`ConfirmationStatus`] | required, no default |
//! | `notes` | string, optional | triage notes |
//!
//! `classification` mirrors the verdict-level distinction ([`crate::verdict::Verdict`]):
//! `wrong-result` is a broken metamorphic/differential relation with every query
//! evaluating successfully; `engine-error` is an internal error / crash / rejection of
//! a valid query. The two are never conflated — a paper claim about "logic bugs" may
//! count only the `wrong-result` class.

use serde::{Deserialize, Serialize};

use crate::verdict::OracleKind;

/// Strict bug classification (see the module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BugClass {
    /// Silently incorrect results (the logic-bug class).
    WrongResult,
    /// Internal error / crash / rejection of a valid query.
    EngineError,
}

/// Developer-confirmation lifecycle of a filed bug. Required on every entry; there is
/// deliberately no `Unfiled` state — an unfiled observation is not a ledger entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConfirmationStatus {
    /// Filed upstream, awaiting developer response.
    Reported,
    /// A developer acknowledged the behaviour as a bug.
    DeveloperConfirmed,
    /// A fix landed / was released upstream.
    Fixed,
    /// Developers rejected the report (e.g. documented/intended behaviour). Kept in
    /// the ledger for honesty — rejected reports are part of the method's error rate.
    Rejected,
    /// Duplicate of an existing upstream issue.
    Duplicate,
}

/// One found-bug ledger entry. Field semantics: see the module-level schema table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FoundBug {
    pub id: String,
    pub engine: String,
    pub engine_version: String,
    pub oracle: OracleKind,
    pub seed: Option<u64>,
    pub query: String,
    pub data_ref: String,
    pub classification: BugClass,
    pub summary: String,
    pub upstream_issue: String,
    pub confirmation: ConfirmationStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

/// A ledger-format violation (which entry, what is wrong).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerError(pub String);

impl std::fmt::Display for LedgerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ledger error: {}", self.0)
    }
}

impl std::error::Error for LedgerError {}

/// Validate one entry against the schema's non-structural requirements: non-empty
/// `id`/`engine`/`engine_version`/`query`/`summary`, and an `upstream_issue` that is a
/// non-empty `http(s)` URL. (The structural requirements — field presence, enum
/// values — are enforced by serde deserialisation itself.)
pub fn validate_entry(bug: &FoundBug) -> Result<(), LedgerError> {
    for (field, value) in [
        ("id", &bug.id),
        ("engine", &bug.engine),
        ("engine_version", &bug.engine_version),
        ("query", &bug.query),
        ("summary", &bug.summary),
    ] {
        if value.trim().is_empty() {
            return Err(LedgerError(format!("entry {:?}: empty field `{field}`", bug.id)));
        }
    }
    if !(bug.upstream_issue.trim().starts_with("https://") || bug.upstream_issue.trim().starts_with("http://")) {
        return Err(LedgerError(format!(
            "entry {:?}: `upstream_issue` must be an http(s) URL to the filed upstream issue, got {:?}",
            bug.id, bug.upstream_issue
        )));
    }
    Ok(())
}

/// Serialise a ledger to JSON Lines, validating every entry first (fail-closed: an
/// invalid entry aborts the write rather than being silently persisted).
pub fn to_jsonl(entries: &[FoundBug]) -> Result<String, LedgerError> {
    let mut out = String::new();
    for bug in entries {
        validate_entry(bug)?;
        let line = serde_json::to_string(bug).map_err(|e| LedgerError(e.to_string()))?;
        out.push_str(&line);
        out.push('\n');
    }
    Ok(out)
}

/// Parse a JSON Lines ledger, validating every entry (fail-closed: a malformed or
/// schema-violating line is an error, never skipped).
pub fn from_jsonl(text: &str) -> Result<Vec<FoundBug>, LedgerError> {
    let mut entries = Vec::new();
    for (line_no, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let bug: FoundBug = serde_json::from_str(line)
            .map_err(|e| LedgerError(format!("line {}: {}", line_no + 1, e)))?;
        validate_entry(&bug)?;
        entries.push(bug);
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_entry() -> FoundBug {
        FoundBug {
            id: "sparq-tlp-0001".into(),
            engine: "exampledb".into(),
            engine_version: "1.2.3".into(),
            oracle: OracleKind::Tlp,
            seed: Some(42),
            query: "SELECT * WHERE { ?s ?p ?o FILTER(?o < 5) }".into(),
            data_ref: "inline: <http://example.org/s> <http://example.org/p> \"1\"^^<http://www.w3.org/2001/XMLSchema#integer> .".into(),
            classification: BugClass::WrongResult,
            summary: "TLP partition union differs from base".into(),
            upstream_issue: "https://github.com/example/exampledb/issues/1".into(),
            confirmation: ConfirmationStatus::Reported,
            notes: None,
        }
    }

    #[test]
    fn validate_entry_accepts_a_complete_entry() {
        assert_eq!(validate_entry(&valid_entry()), Ok(()));
    }

    #[test]
    fn validate_entry_requires_an_upstream_issue_url() {
        let mut bug = valid_entry();
        bug.upstream_issue = String::new();
        assert!(validate_entry(&bug).is_err());
        bug.upstream_issue = "filed by email".into();
        assert!(validate_entry(&bug).is_err());
        bug.upstream_issue = "http://tracker.example.org/1".into();
        assert!(validate_entry(&bug).is_ok());
    }

    #[test]
    fn validate_entry_rejects_empty_required_strings() {
        for field in ["id", "engine", "engine_version", "query", "summary"] {
            let mut bug = valid_entry();
            match field {
                "id" => bug.id = " ".into(),
                "engine" => bug.engine = String::new(),
                "engine_version" => bug.engine_version = String::new(),
                "query" => bug.query = String::new(),
                _ => bug.summary = String::new(),
            }
            assert!(validate_entry(&bug).is_err(), "field {field} must be required");
        }
    }

    #[test]
    fn to_jsonl_round_trips_and_fails_closed_on_invalid_entries() {
        let entries = vec![valid_entry()];
        let text = to_jsonl(&entries).unwrap();
        assert_eq!(text.lines().count(), 1);
        let back = from_jsonl(&text).unwrap();
        assert_eq!(back, entries);

        let mut bad = valid_entry();
        bad.upstream_issue = String::new();
        assert!(to_jsonl(&[bad]).is_err(), "invalid entry must abort the write");
    }

    #[test]
    fn from_jsonl_rejects_a_line_missing_the_confirmation_field() {
        let text = to_jsonl(&[valid_entry()]).unwrap();
        let stripped = text.replace(",\"confirmation\":\"reported\"", "");
        assert_ne!(text, stripped, "test setup: the field must have been present");
        assert!(
            from_jsonl(&stripped).is_err(),
            "confirmation status is required with no default"
        );
    }
}
