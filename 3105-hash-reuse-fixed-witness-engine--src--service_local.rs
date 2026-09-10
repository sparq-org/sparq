//! Local (in-process) rows-returning `SERVICE` handlers — the table-function seam.
//! [OPUS-5] (sq-lsp7k.2.2)
//!
//! The [`FunctionRegistry`](crate::FunctionRegistry) extension seam is *scalar*: terms
//! in, ONE term out. Some extensions cannot be expressed that way — a lookup that
//! yields a whole RELATION (`?row ?value` pairs from an inverted index, a shape's
//! conforming focus nodes, a generator like "the integers 1..n"). SPARQL already has a
//! syntax for "evaluate this group somewhere else and join the resulting solutions
//! back": `SERVICE <iri> { … }`. This module reuses it for a LOCAL handler.
//!
//! A [`LocalServiceRegistry`] maps a SERVICE IRI to a handler closure. Install it for a
//! scope with [`with_local_services`](crate::with_local_services); the executor then
//! intercepts `SERVICE <iri> { … }` for a REGISTERED IRI **before** any HTTP transport
//! is reached, calls the handler, interns the returned rows against the query's
//! dictionaries exactly as `VALUES` does, and joins them into the surrounding query
//! through the ordinary join path. An IRI that is *not* registered is untouched: it
//! takes the existing `service`-feature HTTP path (or, with `service` off, the usual
//! "unsupported" error).
//!
//! ## Egress / SSRF
//!
//! Interception happens **before** the transport, so a handled SERVICE performs no
//! network I/O at all and the default-deny egress allowlist (`with_service_egress_allow`
//! — a `service`-feature item, so deliberately named here as a code span rather than an
//! intra-doc link) is never consulted: there is nothing to consult it about. This can
//! only ever REMOVE outbound reach,
//! never add it: a MISS in the registry falls through to the unchanged HTTP path where
//! the allowlist still applies in full. Registering an IRI therefore SHADOWS the remote
//! endpoint of the same IRI for the scope of the install.
//!
//! The trust boundary is the same as [`FunctionRegistry`](crate::FunctionRegistry)'s:
//! a handler is arbitrary in-process code supplied by the embedding application, so the
//! engine neither sandboxes it nor polices what IT does (a handler that opens its own
//! sockets is outside the engine's egress policy — that is the application's choice,
//! made when it linked the handler in, not something a query can cause).
//!
//! ## Scope-outs (v1)
//!
//! * Only a CONCRETE `SERVICE <iri>` is dispatched locally. `SERVICE ?ep { … }`
//!   (variable endpoint) is never routed to a handler even when `?ep` binds a
//!   registered IRI; it keeps its existing behaviour exactly.
//! * No bindings are pushed INTO the handler from the outer query. The SERVICE
//!   bind-join (`VALUES` pushdown) explicitly DECLINES for a registered IRI, so the
//!   handler is evaluated once, unbound, and the outer join happens locally — the same
//!   answer the verbatim remote path produces, with no half-specified pushdown
//!   protocol. The pre-bound terms the SERVICE group itself fixes ARE visible, as the
//!   constant slots of [`LocalServiceRequest::patterns`].

use oxrdf::{Term, Variable};

/// One slot of a [`LocalServicePattern`]: either a term the SERVICE group already
/// fixes (a pre-bound *argument*) or a variable the handler is expected to bind (an
/// *output*).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalServiceSlot {
    /// A concrete RDF term written in the SERVICE group.
    Term(Term),
    /// A variable of the SERVICE group.
    Var(Variable),
}

/// One triple pattern of the SERVICE group, in the engine's neutral form.
///
/// This is the table-function *argument* channel: the constant slots are the arguments
/// the query supplied, the variable slots name the columns the handler should return.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalServicePattern {
    /// The subject slot.
    pub subject: LocalServiceSlot,
    /// The predicate slot (always an IRI or a variable — never a literal).
    pub predicate: LocalServiceSlot,
    /// The object slot.
    pub object: LocalServiceSlot,
}

/// What the engine hands a local SERVICE handler.
///
/// Built by the executor and borrowed for the duration of the call; a handler that
/// needs to retain anything must clone it out.
#[derive(Debug, Clone, Copy)]
pub struct LocalServiceRequest<'a> {
    pub(crate) service: &'a str,
    pub(crate) query: &'a str,
    pub(crate) vars: &'a [Variable],
    pub(crate) patterns: &'a [LocalServicePattern],
}

impl<'a> LocalServiceRequest<'a> {
    /// The SERVICE IRI this request was dispatched under — the registry key. A handler
    /// registered for several IRIs uses it to tell them apart.
    pub fn service(&self) -> &'a str {
        self.service
    }

    /// The in-scope variables of the SERVICE group, in first-occurrence order. A
    /// handler MUST return rows over a subset of these — exactly what a remote endpoint
    /// answering the group could produce. A returned column that is NOT in scope here is
    /// rejected as an invalid relation: it would otherwise join against an
    /// identically-named variable of the SURROUNDING query, which is not the SERVICE
    /// group's semantics.
    pub fn vars(&self) -> &'a [Variable] {
        self.vars
    }

    /// The SERVICE group rendered as the standalone SPARQL query
    /// `SELECT * WHERE { … }` — byte-for-byte the string the HTTP transport would have
    /// POSTed for this SERVICE. The general escape hatch for a handler that wants to
    /// re-parse or forward the pattern itself; prefer [`patterns`](Self::patterns) for
    /// the table-function shape.
    pub fn query(&self) -> &'a str {
        self.query
    }

    /// The SERVICE group decomposed into triple patterns — non-empty ONLY when the
    /// group is a flat basic graph pattern (the table-function shape). For any richer
    /// group (`OPTIONAL`, `UNION`, `FILTER`, a sub-`SELECT`, …) this is EMPTY and the
    /// handler must work from [`query`](Self::query) instead. Decomposing is
    /// deliberately all-or-nothing: a handler must never see a partial view of a group
    /// it did not fully understand and answer as though it had.
    pub fn patterns(&self) -> &'a [LocalServicePattern] {
        self.patterns
    }
}

/// The solution rows a local SERVICE handler returns.
///
/// `vars` names the columns; each row carries exactly `vars.len()` cells, `None` for an
/// UNBOUND cell (the SPARQL-Results-JSON "variable absent from this binding" case). The
/// engine validates the arity and the uniqueness of `vars` ([`validate`](Self::validate))
/// AND that every column is in scope in the SERVICE group
/// ([`LocalServiceRequest::vars`]) — a scope check the executor makes, since only it
/// knows the group. A violation is reported the same way any other SERVICE failure is —
/// a hard query error, or the join identity under `SILENT`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocalServiceRows {
    /// The column names, in row order.
    pub vars: Vec<Variable>,
    /// The rows; each is `vars.len()` cells wide.
    pub rows: Vec<Vec<Option<Term>>>,
}

impl LocalServiceRows {
    /// A relation with the given header and rows.
    pub fn new(vars: Vec<Variable>, rows: Vec<Vec<Option<Term>>>) -> Self {
        Self { vars, rows }
    }

    /// The empty relation over `vars` — zero rows, which makes the surrounding join
    /// empty (NOT the join identity; see the `SILENT` note on the module docs).
    pub fn empty(vars: Vec<Variable>) -> Self {
        Self { vars, rows: Vec::new() }
    }

    /// Checks the header/arity contract — unique column names, every row exactly
    /// `vars.len()` cells wide — returning the offending detail as an `Err` message.
    /// Called by the executor on every handler result; exposed so a handler (or its
    /// tests) can assert the same contract itself. The executor additionally checks the
    /// columns against the SERVICE group's in-scope variables, which this method cannot
    /// see; see [`LocalServiceRequest::vars`].
    pub fn validate(&self) -> Result<(), String> {
        for (i, v) in self.vars.iter().enumerate() {
            if self.vars[..i].contains(v) {
                return Err(format!("duplicate variable ?{} in the returned header", v.as_str()));
            }
        }
        let width = self.vars.len();
        for (i, row) in self.rows.iter().enumerate() {
            if row.len() != width {
                return Err(format!(
                    "row {} has {} cells but the header names {} variables",
                    i,
                    row.len(),
                    width
                ));
            }
        }
        Ok(())
    }
}

/// A local SERVICE handler: a SERVICE group in, named-variable solution rows out.
///
/// Returning `Err` is a SERVICE *failure*, handled exactly as a remote failure is: a
/// hard query error, or — under `SERVICE SILENT` — swallowed, yielding the join
/// identity so the surrounding solutions survive unchanged. The message only needs to
/// be useful to a human debugging the handler.
pub type LocalServiceFn =
    std::sync::Arc<dyn Fn(&LocalServiceRequest<'_>) -> Result<LocalServiceRows, String> + Send + Sync>;

/// A map from SERVICE IRIs to [`LocalServiceFn`] handlers, consulted by the executor
/// for `SERVICE <iri> { … }` BEFORE any HTTP transport. Installed for a scope by
/// [`with_local_services`](crate::with_local_services); with none installed the engine
/// keeps its exact pre-registry SERVICE behaviour and hot-path cost.
///
/// Cloning is cheap (the handlers are `Arc`-shared), so a long-lived registry can be
/// built once and reused across queries and threads.
#[derive(Clone, Default)]
pub struct LocalServiceRegistry {
    map: std::collections::HashMap<String, LocalServiceFn>,
}

impl LocalServiceRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `handler` under the SERVICE IRI (replacing any previous registration).
    pub fn register(
        &mut self,
        iri: impl Into<String>,
        handler: impl Fn(&LocalServiceRequest<'_>) -> Result<LocalServiceRows, String> + Send + Sync + 'static,
    ) {
        self.map.insert(iri.into(), std::sync::Arc::new(handler));
    }

    /// The handler registered for `iri`, if any.
    pub fn get(&self, iri: &str) -> Option<&LocalServiceFn> {
        self.map.get(iri)
    }

    /// Whether `iri` has a handler.
    pub fn contains(&self, iri: &str) -> bool {
        self.map.contains_key(iri)
    }

    /// Iterates the registered SERVICE IRIs (unspecified order) — lets a caller
    /// advertise EXACTLY the handlers actually installed instead of hand-maintaining a
    /// parallel list that could drift.
    pub fn iris(&self) -> impl Iterator<Item = &str> {
        self.map.keys().map(String::as_str)
    }

    /// The number of registered handlers.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Whether no handler is registered.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

impl std::fmt::Debug for LocalServiceRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut iris: Vec<&str> = self.iris().collect();
        iris.sort_unstable();
        f.debug_struct("LocalServiceRegistry").field("iris", &iris).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::NamedNode;

    fn var(n: &str) -> Variable {
        Variable::new(n).unwrap()
    }

    fn rows_ok() -> LocalServiceRows {
        LocalServiceRows::new(
            vec![var("a"), var("b")],
            vec![vec![Some(Term::from(NamedNode::new("urn:x").unwrap())), None]],
        )
    }

    #[test]
    fn rows_new_and_empty() {
        let r = rows_ok();
        assert_eq!(r.vars.len(), 2);
        assert_eq!(r.rows.len(), 1);
        let e = LocalServiceRows::empty(vec![var("a")]);
        assert!(e.rows.is_empty());
        assert_eq!(e.vars, vec![var("a")]);
        assert_eq!(LocalServiceRows::default(), LocalServiceRows::new(Vec::new(), Vec::new()));
    }

    #[test]
    fn validate_accepts_and_rejects() {
        assert!(rows_ok().validate().is_ok());
        let dup = LocalServiceRows::new(vec![var("a"), var("a")], Vec::new());
        assert!(dup.validate().unwrap_err().contains("duplicate variable ?a"));
        let ragged = LocalServiceRows::new(vec![var("a"), var("b")], vec![vec![None]]);
        let err = ragged.validate().unwrap_err();
        assert!(err.contains("row 0"), "{}", err);
        assert!(err.contains("1 cells"), "{}", err);
    }

    #[test]
    fn registry_register_get_contains_iris_len() {
        let mut reg = LocalServiceRegistry::new();
        assert!(reg.is_empty());
        assert_eq!(reg.len(), 0);
        assert!(reg.get("urn:h").is_none());
        assert!(!reg.contains("urn:h"));
        reg.register("urn:h", |_| Ok(rows_ok()));
        assert!(!reg.is_empty());
        assert_eq!(reg.len(), 1);
        assert!(reg.contains("urn:h"));
        assert_eq!(reg.iris().collect::<Vec<_>>(), vec!["urn:h"]);
        // Re-registering the same IRI replaces, it does not accumulate.
        reg.register("urn:h", |_| Err("nope".to_string()));
        assert_eq!(reg.len(), 1);
        let h = reg.get("urn:h").expect("registered").clone();
        let pats: Vec<LocalServicePattern> = Vec::new();
        let vars: Vec<Variable> = Vec::new();
        let req = LocalServiceRequest {
            service: "urn:h",
            query: "SELECT * WHERE { }",
            vars: &vars,
            patterns: &pats,
        };
        assert_eq!(h(&req).unwrap_err(), "nope");
        assert_eq!(format!("{:?}", reg), "LocalServiceRegistry { iris: [\"urn:h\"] }");
    }

    #[test]
    fn request_accessors_and_slots() {
        let pats = vec![LocalServicePattern {
            subject: LocalServiceSlot::Var(var("s")),
            predicate: LocalServiceSlot::Term(Term::from(NamedNode::new("urn:p").unwrap())),
            object: LocalServiceSlot::Var(var("o")),
        }];
        let vars = vec![var("s"), var("o")];
        let req = LocalServiceRequest {
            service: "urn:h",
            query: "SELECT * WHERE { ?s <urn:p> ?o . }",
            vars: &vars,
            patterns: &pats,
        };
        assert_eq!(req.service(), "urn:h");
        assert_eq!(req.query(), "SELECT * WHERE { ?s <urn:p> ?o . }");
        assert_eq!(req.vars(), &vars[..]);
        assert_eq!(req.patterns().len(), 1);
        assert_eq!(req.patterns()[0].subject, LocalServiceSlot::Var(var("s")));
        assert_eq!(
            req.patterns()[0].predicate,
            LocalServiceSlot::Term(Term::from(NamedNode::new("urn:p").unwrap()))
        );
        assert_eq!(req.patterns()[0].object, LocalServiceSlot::Var(var("o")));
        // Cheap to copy (the accessors hand back the original borrows).
        let copy = req;
        assert_eq!(copy.service(), req.service());
    }
}
