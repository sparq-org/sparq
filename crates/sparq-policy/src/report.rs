//! Deterministic summaries of already-computed policy decisions.
//!
//! This module is read-only: it counts the result of [`crate::evaluate`] and never
//! evaluates, changes, or re-derives an authorization decision. Callers pair each
//! decision with its request action because [`crate::Decision`] intentionally stores
//! only the verdict and its audit evidence.

use crate::Decision;
use std::collections::BTreeMap;
use std::fmt::Write;

/// Counts for one requested action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionReport {
    /// Requested action IRI (or another caller-selected stable action identifier).
    pub action: String,
    /// Number of permitted requests for this action.
    pub permitted: usize,
    /// Number of denied requests for this action.
    pub denied: usize,
}

/// A deterministic aggregate of a batch of policy decisions. sq-mu4au.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionReport {
    /// Number of decisions in the batch.
    pub total: usize,
    /// Number of decisions that permitted the request.
    pub permitted: usize,
    /// Number of decisions that denied the request.
    pub denied: usize,
    /// Number of denials caused by a matching prohibition.
    pub conflicts: usize,
    /// Per-action counts, sorted lexicographically by `action`.
    pub per_action: Vec<ActionReport>,
}

impl DecisionReport {
    /// Summarises `(action, decision)` pairs without re-running policy evaluation.
    ///
    /// `conflicts` counts decisions carrying the canonical audit explanation emitted
    /// by `evaluate` when a prohibition overrides a request. This classification only
    /// observes the completed decision; it cannot affect its verdict.
    pub fn summarize<'a, I>(decisions: I) -> Self
    where
        I: IntoIterator<Item = (&'a str, &'a Decision)>,
    {
        let mut report = Self {
            total: 0,
            permitted: 0,
            denied: 0,
            conflicts: 0,
            per_action: Vec::new(),
        };
        let mut actions = BTreeMap::<&str, (usize, usize)>::new();

        for (action, decision) in decisions {
            report.total += 1;
            let counts = actions.entry(action).or_default();
            if decision.allow {
                report.permitted += 1;
                counts.0 += 1;
            } else {
                report.denied += 1;
                counts.1 += 1;
                if decision.unmet_constraints.iter().any(|reason| {
                    reason.starts_with("prohibition ") && reason.ends_with(" matches the request")
                }) {
                    report.conflicts += 1;
                }
            }
        }

        report.per_action = actions
            .into_iter()
            .map(|(action, (permitted, denied))| ActionReport {
                action: action.to_owned(),
                permitted,
                denied,
            })
            .collect();
        report
    }

    /// Serialises the report to byte-stable compact JSON.
    pub fn to_json(&self) -> String {
        let mut out = format!(
            "{{\"total\":{},\"permitted\":{},\"denied\":{},\"conflicts\":{},\"per_action\":[",
            self.total, self.permitted, self.denied, self.conflicts
        );
        for (index, action) in self.per_action.iter().enumerate() {
            if index != 0 {
                out.push(',');
            }
            out.push_str("{\"action\":\"");
            write_json_string(&mut out, &action.action);
            write!(
                out,
                "\",\"permitted\":{},\"denied\":{}}}",
                action.permitted, action.denied
            )
            .expect("writing to a String cannot fail");
        }
        out.push_str("]}");
        out
    }
}

fn write_json_string(out: &mut String, value: &str) {
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            character if character <= '\u{1f}' => {
                write!(out, "\\u{:04x}", u32::from(character))
                    .expect("writing to a String cannot fail");
            }
            character => out.push(character),
        }
    }
}
