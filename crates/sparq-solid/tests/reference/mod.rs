//! sq-t58w.7 — an **independent reference evaluator** for Solid WAC + ACP.
//!
//! This module is the second paradigm in the differential oracle
//! (`tests/differential_oracle.rs`, design record `research/solid-acp-differential-oracle-design.md`
//! §3-4). The engine ([`sparq_solid::materialize_wac`]/[`materialize_acp`] +
//! [`sparq_solid::AuthIndex::accessible`]) decides authorization by running **N3 rules**
//! (`rules/*.n3`) through `sparq-reason` over a dictionary-encoded `Graph`. This module
//! decides the SAME `(agent, client, mode, resource) → allow | deny` question by a
//! **completely different mechanism**: a hand-written, procedural reading of the spec over
//! a plain in-memory model, parsed by a tiny independent N-Quad reader.
//!
//! The whole value of this code is being a DIFFERENT implementation: it deliberately shares
//! **nothing** with the engine's decision path —
//!
//! - it does NOT call `materialize.rs`, `loader.rs`, `assemble_input`, `parent_iri`, or any
//!   `rules/*.n3` (a shared bug in those cannot hide in both deciders);
//! - it does NOT use `Graph::load_dataset` / `sparq-core` to parse the corpus — it parses
//!   the raw N-Quads the builders emit with its own line reader ([`parse_nquads`]);
//! - it re-derives containment ancestry, nearest-ACL resolution (WAC) vs. cumulative
//!   inheritance (ACP), the matcher logic, and deny-overrides from scratch.
//!
//! It reads only the scenario corpus's `nquads_str()` (the IDENTICAL bytes the engine
//! loads) and emits a [`RefDecision`]; the oracle then diffs the two deciders plus the
//! hand `Expect` table and asserts zero divergence.
//!
//! Scope (honest): this is a CORRECTNESS oracle over the parity corpus, not a security
//! audit and not a full WAC/ACP implementation — it implements exactly the constructs the
//! corpus exercises (and fails closed on anything it does not recognise, which the oracle
//! counts as a divergence).

#![allow(dead_code)]

pub mod acp;
pub mod wac;

/// The reference evaluator's verdict for one request. Mirrors
/// [`sparq_solid::conformance::Decision`] but is a distinct type so the two paradigms never
/// share a decision enum by accident.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefDecision {
    /// The access mode is granted on the resource for the requestor.
    Allow,
    /// The access mode is refused (no matching grant, a deny overrides, or fail-closed).
    Deny,
}

/// The four WAC/ACP access modes, named independently of the engine's `Mode` so the
/// reference reader does not borrow the engine's enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefMode {
    Read,
    Write,
    Append,
    Control,
}

impl RefMode {
    /// Parse an `acl:`-namespace mode IRI (`…#Read`/`Write`/`Append`/`Control`).
    pub fn from_acl_iri(iri: &str) -> Option<RefMode> {
        match iri {
            "http://www.w3.org/ns/auth/acl#Read" => Some(RefMode::Read),
            "http://www.w3.org/ns/auth/acl#Write" => Some(RefMode::Write),
            "http://www.w3.org/ns/auth/acl#Append" => Some(RefMode::Append),
            "http://www.w3.org/ns/auth/acl#Control" => Some(RefMode::Control),
            _ => None,
        }
    }
}

/// A parsed N-Quad: subject/predicate IRIs, an object that is either an IRI or a literal,
/// and the graph IRI. The reference reader only ever needs IRI subjects/predicates/graphs
/// and distinguishes an IRI object from a literal one (document placeholders are literals).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Quad {
    pub subject: String,
    pub predicate: String,
    pub object: Obj,
    pub graph: String,
}

/// An N-Quad object: a named node (IRI) or a literal value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Obj {
    Iri(String),
    Literal(String),
}

impl Obj {
    /// The IRI if this object is a named node, else `None`.
    pub fn iri(&self) -> Option<&str> {
        match self {
            Obj::Iri(s) => Some(s),
            Obj::Literal(_) => None,
        }
    }
}

/// A tiny, INDEPENDENT N-Quad line reader for exactly the shape the corpus builders emit:
/// `<s> <p> <o> <g> .` per line, where `<o>` is either `<iri>` or a `"…"` literal (the
/// document placeholders). It is deliberately NOT a general N-Quads parser and NOT
/// `sparq-core`'s loader — its only job is to read back the bytes
/// `AclBuilder`/`AcrBuilder` produced, by hand, so the reference decider shares no parsing
/// code with the engine.
///
/// Lines that do not match the four-`<>`/literal-then-`.` shape are skipped (blank lines);
/// a line that *looks* like a quad but fails to tokenize is returned as an error so the
/// oracle fails closed rather than silently dropping an authorization.
pub fn parse_nquads(src: &str) -> Result<Vec<Quad>, String> {
    let mut out = Vec::new();
    for (lineno, raw) in src.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let quad = parse_quad_line(line).ok_or_else(|| {
            format!(
                "reference n-quad parse failed at line {}: {:?}",
                lineno + 1,
                raw
            )
        })?;
        out.push(quad);
    }
    Ok(out)
}

/// Parse one `<s> <p> <o> <g> .` line into a [`Quad`], or `None` if it does not match.
fn parse_quad_line(line: &str) -> Option<Quad> {
    // Must end with " ." (N-Triples/N-Quads statement terminator).
    let body = line.strip_suffix('.')?.trim_end();
    let mut rest = body;

    let subject = take_iri(&mut rest)?;
    let predicate = take_iri(&mut rest)?;
    let object = take_obj(&mut rest)?;
    let graph = take_iri(&mut rest)?;
    // Nothing should remain after the graph token.
    if !rest.trim().is_empty() {
        return None;
    }
    Some(Quad {
        subject,
        predicate,
        object,
        graph,
    })
}

/// Consume a leading `<iri>` token from `rest` (skipping leading whitespace), returning the
/// IRI text and advancing `rest` past it.
fn take_iri(rest: &mut &str) -> Option<String> {
    let s = rest.trim_start();
    let s = s.strip_prefix('<')?;
    let end = s.find('>')?;
    let iri = &s[..end];
    *rest = &s[end + 1..];
    Some(iri.to_owned())
}

/// Consume a leading object token — either `<iri>` or `"literal"` — from `rest`.
fn take_obj(rest: &mut &str) -> Option<Obj> {
    let s = rest.trim_start();
    if s.starts_with('<') {
        let iri = take_iri(rest)?;
        Some(Obj::Iri(iri))
    } else if let Some(after) = s.strip_prefix('"') {
        // The corpus emits only simple `"x"` placeholder literals (no escapes, no
        // datatype/lang). Read up to the closing quote.
        let end = after.find('"')?;
        let val = &after[..end];
        *rest = &after[end + 1..];
        Some(Obj::Literal(val.to_owned()))
    } else {
        None
    }
}

/// The Solid slash-semantics parent container of an IRI, derived INDEPENDENTLY (this is the
/// reference reader's own ancestry walk — it does not call the engine's
/// `loader::parent_iri`). `None` at or above the authority root.
///
/// Rule: strip a trailing `/` (so a container is parented like its own path), then cut at
/// the last `/` that is still in the path (after the `scheme://host` authority). A fragment
/// or query is treated as part of the final path segment (the corpus never parents through
/// one).
pub fn parent_container(iri: &str) -> Option<String> {
    let scheme_end = iri.find("://")? + 3;
    let after_authority = iri.get(scheme_end..)?;
    let first_slash = after_authority.find('/')?;
    let host_end = scheme_end + first_slash; // index of the first '/' after the host
                                             // Trim exactly one trailing slash so `…/c/` parents to `…/`, not to itself.
    let trimmed = iri.strip_suffix('/').unwrap_or(iri);
    if trimmed.len() <= host_end {
        return None; // already the root container `scheme://host/`
    }
    let cut = trimmed.rfind('/')?;
    if cut < host_end {
        return None;
    }
    Some(iri[..cut + 1].to_owned())
}

/// All structural ancestors of `iri`, nearest-first (its parent container, then that
/// container's parent, … up to the root). Excludes `iri` itself.
pub fn ancestors_nearest_first(iri: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = iri.to_owned();
    while let Some(parent) = parent_container(&cur) {
        out.push(parent.clone());
        cur = parent;
    }
    out
}

/// Is `descendant` an IRI-structural descendant of `container`? (A container's own IRI is
/// NOT a descendant of itself — `acl:default` / `acp:memberAccessControl` reach members,
/// not the container resource.) Independent of any engine code.
pub fn is_structural_descendant(container: &str, descendant: &str) -> bool {
    container != descendant
        && descendant.starts_with(container)
        // The container IRI ends in `/` for a real container; require the descendant to be
        // strictly under it (the prefix relation over a slash boundary). The corpus always
        // names containers with a trailing slash, so a plain prefix check after the
        // equality guard is the structural-descendant test.
        && container.ends_with('/')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_simple_quad() {
        let q =
            parse_quad_line("<https://a/s> <https://a/p> <https://a/o> <https://a/g> .").unwrap();
        assert_eq!(q.subject, "https://a/s");
        assert_eq!(q.object, Obj::Iri("https://a/o".to_owned()));
        assert_eq!(q.graph, "https://a/g");
    }

    #[test]
    fn parses_a_literal_object() {
        let q = parse_quad_line("<https://a/s#it> <https://ex.dev/ns#prop> \"x\" <https://a/s> .")
            .unwrap();
        assert_eq!(q.object, Obj::Literal("x".to_owned()));
    }

    #[test]
    fn parent_and_descendants() {
        assert_eq!(
            parent_container("https://pod.ex/a/b/d1").as_deref(),
            Some("https://pod.ex/a/b/")
        );
        assert_eq!(
            parent_container("https://pod.ex/a/b/").as_deref(),
            Some("https://pod.ex/a/")
        );
        assert_eq!(parent_container("https://pod.ex/"), None);
        assert!(is_structural_descendant(
            "https://pod.ex/c/",
            "https://pod.ex/c/sub/d1"
        ));
        assert!(!is_structural_descendant(
            "https://pod.ex/c/",
            "https://pod.ex/c/"
        ));
    }
}
