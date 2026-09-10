//! Named parameterized SPARQL templates (issue-class: GraphDB "SPARQL templates" /
//! Stardog "stored queries"; bead `sq-lsp7k.10`). NON-DEFAULT `templates` feature.
//!
//! A [`Template`] is a **named, server-storable** SPARQL query or UPDATE whose free
//! placeholders are bound with **typed** values at invocation time through the `params`
//! module's injection-safe algebra rewrite ([`crate::params`], #901): the template text is
//! parsed ONCE at registration, each declared parameter is validated to be a bindable free
//! placeholder, and an invocation substitutes typed [`oxrdf::Term`]s into the parsed
//! algebra — never string concatenation — so a hostile bound value can never change the
//! query structure. This module is the shared **definition + typed-JSON-binding** layer the
//! HTTP surface (`sparq-server`, feature `templates`) and the MCP tool surface
//! (`sparq-mcp`, feature `templates`) both consume; it executes nothing itself.
//!
//! ## Fail-closed invocation contract (the load-bearing rule)
//!
//! [`Template::bind_json`] refuses — it never guesses — when:
//! - an argument names a parameter the template does not declare (typo ⇒ error, not a
//!   silent no-op),
//! - a declared parameter is missing (every declared parameter is required),
//! - a value's JSON shape does not match the declared [`ParamType`] (no coercion),
//! - the underlying algebra bind rejects the slot (e.g. a literal into a predicate
//!   position — surfaced from [`crate::params`]).
//!
//! Registration ([`Template::new`]) is equally fail-closed: a template whose text does not
//! parse, or that declares a parameter which is not a free, slot-compatible placeholder in
//! the text, is rejected up front — a stored template is always invocable.
//!
//! When the `templates` feature is off, none of this compiles; the default build is
//! byte-identical. The only dependency delta is `serde_json` (already ubiquitous in the
//! workspace tree), pulled in behind the feature. [FABLE-5] sq-lsp7k.10

use std::collections::BTreeMap;

use oxrdf::{NamedNode, Term};
use serde_json::{json, Map, Value};

use crate::params::value;
use crate::{PreparedQuery, PreparedUpdate};

/// The declared type of one template parameter — how a JSON argument is converted into
/// the typed [`oxrdf::Term`] bound into the algebra. Declared per parameter in the
/// template definition (`"parameters": {"who": "iri", "age": "integer", …}`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParamType {
    /// Auto-typed from the JSON value shape: a JSON string ⇒ `xsd:string` literal, an
    /// integer number ⇒ `xsd:integer`, a non-integer number ⇒ `xsd:double`, a boolean ⇒
    /// `xsd:boolean`; or an explicit object `{"iri": …}` / `{"value": …, "datatype": …}` /
    /// `{"value": …, "lang": …}` for the shapes JSON cannot express natively.
    Auto,
    /// An IRI ([`oxrdf::NamedNode`]); the JSON argument must be a string holding a valid
    /// absolute IRI (validated by oxrdf's constructor before it ever reaches the algebra).
    Iri,
    /// A plain `xsd:string` literal; the JSON argument must be a string.
    String,
    /// An `xsd:boolean` literal; the JSON argument must be a JSON boolean.
    Boolean,
    /// An `xsd:integer` literal; the JSON argument must be a JSON integer number.
    Integer,
    /// An `xsd:decimal` literal; the JSON argument must be a JSON number.
    Decimal,
    /// An `xsd:double` literal; the JSON argument must be a JSON number.
    Double,
    /// A literal typed with the given datatype IRI; the JSON argument must be a string
    /// carrying the lexical form.
    Datatype(String),
}

const XSD: &str = "http://www.w3.org/2001/XMLSchema#";

impl ParamType {
    /// Parses a declared parameter-type token: one of the keywords `auto` / `iri` /
    /// `string` / `boolean` / `integer` / `decimal` / `double`, or a full datatype IRI
    /// (anything containing `:` — validated as an IRI). Fail-closed on anything else.
    pub fn parse(s: &str) -> Result<ParamType, std::string::String> {
        match s {
            "auto" => Ok(ParamType::Auto),
            "iri" => Ok(ParamType::Iri),
            "string" => Ok(ParamType::String),
            "boolean" => Ok(ParamType::Boolean),
            "integer" => Ok(ParamType::Integer),
            "decimal" => Ok(ParamType::Decimal),
            "double" => Ok(ParamType::Double),
            other if other.contains(':') => {
                NamedNode::new(other)
                    .map_err(|e| format!("invalid datatype IRI `{}`: {}", other, e))?;
                Ok(ParamType::Datatype(other.to_string()))
            }
            other => Err(format!(
                "unknown parameter type `{}` (expected auto|iri|string|boolean|integer|\
                 decimal|double or a datatype IRI)",
                other
            )),
        }
    }

    /// The declared token this type round-trips to (the inverse of [`ParamType::parse`];
    /// [`ParamType::Datatype`] yields the datatype IRI itself).
    pub fn as_str(&self) -> &str {
        match self {
            ParamType::Auto => "auto",
            ParamType::Iri => "iri",
            ParamType::String => "string",
            ParamType::Boolean => "boolean",
            ParamType::Integer => "integer",
            ParamType::Decimal => "decimal",
            ParamType::Double => "double",
            ParamType::Datatype(dt) => dt,
        }
    }

    /// Converts a JSON argument into the typed [`Term`] this declaration admits.
    /// Fail-closed: a JSON shape that does not match the declaration is an error — no
    /// coercion (`"5"` is NOT an integer; `5` is NOT a string).
    fn term_for(&self, name: &str, v: &Value) -> Result<Term, std::string::String> {
        let wrong = |want: &str| {
            format!(
                "parameter `{}` expects {} (declared type `{}`)",
                name,
                want,
                self.as_str()
            )
        };
        match self {
            ParamType::Auto => auto_term(name, v),
            ParamType::Iri => match v.as_str() {
                Some(s) => value::iri(s).map_err(|e| format!("parameter `{}`: {}", name, e)),
                None => Err(wrong("a JSON string holding an IRI")),
            },
            ParamType::String => match v.as_str() {
                Some(s) => Ok(value::string(s)),
                None => Err(wrong("a JSON string")),
            },
            ParamType::Boolean => match v.as_bool() {
                Some(b) => typed(&b.to_string(), "boolean"),
                None => Err(wrong("a JSON boolean")),
            },
            ParamType::Integer => match v.as_i64() {
                Some(i) => typed(&i.to_string(), "integer"),
                None => Err(wrong("a JSON integer")),
            },
            ParamType::Decimal => match v.as_f64() {
                // serde_json's Number renders a canonical lexical form; reuse it so
                // `1.5` stays `1.5` (not a float re-render).
                Some(_) => typed(&v.to_string(), "decimal"),
                None => Err(wrong("a JSON number")),
            },
            ParamType::Double => match v.as_f64() {
                Some(_) => typed(&v.to_string(), "double"),
                None => Err(wrong("a JSON number")),
            },
            ParamType::Datatype(dt) => match v.as_str() {
                Some(s) => value::typed(s, dt).map_err(|e| format!("parameter `{}`: {}", name, e)),
                None => Err(wrong("a JSON string holding the lexical form")),
            },
        }
    }

    /// A probe term of this type, used at registration to prove the declared parameter is
    /// a free, slot-compatible placeholder (so a stored template is always invocable).
    /// `None` for [`ParamType::Auto`] — an auto parameter's slot compatibility depends on
    /// the invocation value, so registration probes it as an IRI OR a literal (either
    /// binding proves the placeholder is free).
    fn probe(&self) -> Option<Term> {
        match self {
            ParamType::Auto => None,
            ParamType::Iri => Some(value::iri("http://sparq.invalid/probe").expect("static IRI")),
            ParamType::String => Some(value::string("probe")),
            ParamType::Boolean => Some(xsd_term("true", "boolean")),
            ParamType::Integer => Some(xsd_term("0", "integer")),
            ParamType::Decimal => Some(xsd_term("0.0", "decimal")),
            ParamType::Double => Some(xsd_term("0.0e0", "double")),
            ParamType::Datatype(dt) => value::typed("probe", dt).ok(),
        }
    }
}

/// An `xsd:`-typed literal term; `suffix` is the local name under the XSD namespace.
fn xsd_term(lexical: &str, suffix: &str) -> Term {
    value::typed(lexical, &format!("{}{}", XSD, suffix)).expect("static XSD datatype IRI")
}

/// An `xsd:`-typed literal, as a `Result` for the conversion paths.
fn typed(lexical: &str, suffix: &str) -> Result<Term, String> {
    Ok(xsd_term(lexical, suffix))
}

/// [`ParamType::Auto`]'s JSON-shape–driven conversion (see the variant doc).
fn auto_term(name: &str, v: &Value) -> Result<Term, String> {
    match v {
        Value::String(s) => Ok(value::string(s)),
        Value::Bool(b) => typed(&b.to_string(), "boolean"),
        Value::Number(n) => {
            if n.as_i64().is_some() {
                typed(&n.to_string(), "integer")
            } else {
                typed(&n.to_string(), "double")
            }
        }
        Value::Object(obj) => auto_object_term(name, obj),
        _ => Err(format!(
            "parameter `{}` (auto): expected a JSON string/number/boolean or an object \
             {{\"iri\"}} / {{\"value\",\"datatype\"}} / {{\"value\",\"lang\"}}",
            name
        )),
    }
}

/// The explicit-object shapes an `auto` parameter accepts: `{"iri": …}`,
/// `{"value": …, "datatype": …}`, `{"value": …, "lang": …}`, or bare `{"value": …}`
/// (a plain string literal). Unknown keys are rejected (fail-closed).
fn auto_object_term(name: &str, obj: &Map<String, Value>) -> Result<Term, String> {
    let known = ["iri", "value", "datatype", "lang"];
    if let Some(k) = obj.keys().find(|k| !known.contains(&k.as_str())) {
        return Err(format!("parameter `{}`: unknown key `{}` in value object", name, k));
    }
    let get = |k: &str| obj.get(k).and_then(Value::as_str);
    match (get("iri"), get("value"), get("datatype"), get("lang")) {
        (Some(iri), None, None, None) => {
            value::iri(iri).map_err(|e| format!("parameter `{}`: {}", name, e))
        }
        (None, Some(v), Some(dt), None) => {
            value::typed(v, dt).map_err(|e| format!("parameter `{}`: {}", name, e))
        }
        (None, Some(v), None, Some(lang)) => {
            value::lang_string(v, lang).map_err(|e| format!("parameter `{}`: {}", name, e))
        }
        (None, Some(v), None, None) => Ok(value::string(v)),
        _ => Err(format!(
            "parameter `{}`: value object must be exactly one of {{\"iri\"}}, \
             {{\"value\",\"datatype\"}}, {{\"value\",\"lang\"}} or {{\"value\"}} \
             (string fields)",
            name
        )),
    }
}

/// The parsed body of a template: a query or an update, prepared once at registration.
#[derive(Debug, Clone)]
enum Body {
    // Boxed: a PreparedQuery is much larger than a PreparedUpdate (clippy
    // large_enum_variant); templates are stored long-term, so keep the enum slim.
    Query(Box<PreparedQuery>),
    Update(PreparedUpdate),
}

/// A fully-bound template ready to execute: the invocation surface hands the prepared,
/// value-substituted algebra to whatever execution path it already uses.
#[derive(Debug, Clone)]
pub enum Bound {
    /// A bound SELECT / ASK / CONSTRUCT / DESCRIBE (boxed — see [`PreparedQuery`]'s size).
    Query(Box<PreparedQuery>),
    /// A bound SPARQL 1.1 UPDATE. The invocation surface MUST keep its update gate in
    /// front of executing this (the template layer stores and binds; it never widens
    /// write access).
    Update(PreparedUpdate),
}

impl Bound {
    /// Whether this bound template is a SPARQL UPDATE (the invocation surface's write
    /// gate keys off this).
    pub fn is_update(&self) -> bool {
        matches!(self, Bound::Update(_))
    }

    /// Whether this bound template is a graph-producing query form (CONSTRUCT /
    /// DESCRIBE) rather than a solution-producing one (SELECT / ASK) — the invocation
    /// surface picks its result serialization (N-Triples vs SPARQL-JSON) off this.
    /// `false` for an update.
    pub fn is_graph_form(&self) -> bool {
        match self {
            Bound::Query(q) => matches!(
                q.query(),
                spargebra::Query::Construct { .. } | spargebra::Query::Describe { .. }
            ),
            Bound::Update(_) => false,
        }
    }

    /// Renders the bound algebra as canonical SPARQL text for the surface's existing
    /// string-driven execution path. Safe by construction: the parameter values were
    /// substituted STRUCTURALLY (the #901 algebra rewrite) and spargebra's serializer
    /// escapes every term, so the rendered text carries them as data — a hostile value
    /// cannot re-enter as syntax.
    pub fn render(&self) -> String {
        match self {
            Bound::Query(q) => q.query().to_string(),
            Bound::Update(u) => u.update().to_string(),
        }
    }
}

/// A named, storable, parameterized SPARQL query or UPDATE (see the module docs).
#[derive(Debug, Clone)]
pub struct Template {
    name: String,
    text: String,
    description: Option<String>,
    params: BTreeMap<String, ParamType>,
    body: Body,
}

impl Template {
    /// Parses + validates a template. `name` is the identifier the store keys it by
    /// (an IRI by convention, but any non-empty token works); `text` is the SPARQL
    /// query/update; `params` declares every invocation parameter and its type.
    ///
    /// Fail-closed registration: the text must parse (as a query, else as an update),
    /// and every declared parameter must be a FREE, slot-compatible placeholder in the
    /// parsed algebra (probed by a trial bind), so a stored template can always be
    /// invoked. A declared parameter the text never mentions, a result/aggregate/BIND
    /// output variable, or a literal-typed parameter in an IRI-only slot is rejected
    /// here, not at invocation time.
    pub fn new(
        name: &str,
        text: &str,
        params: BTreeMap<String, ParamType>,
        description: Option<String>,
    ) -> Result<Template, String> {
        if name.trim().is_empty() {
            return Err("template name must be non-empty".to_string());
        }
        let body = match PreparedQuery::parse(text) {
            Ok(q) => Body::Query(Box::new(q)),
            Err(query_err) => match PreparedUpdate::parse(text) {
                Ok(u) => Body::Update(u),
                Err(update_err) => {
                    return Err(format!(
                        "template text is neither a query ({}) nor an update ({})",
                        query_err, update_err
                    ))
                }
            },
        };
        // Trial-bind each declared parameter individually against the ORIGINAL body so
        // every declaration is proven free + slot-compatible (binds are immutable).
        for (pname, ptype) in &params {
            let probes: Vec<Term> = match ptype.probe() {
                Some(t) => vec![t],
                // Auto: an IRI or a literal binding proves the placeholder free.
                None => vec![
                    value::iri("http://sparq.invalid/probe").expect("static IRI"),
                    value::string("probe"),
                ],
            };
            let mut last_err = String::new();
            let ok = probes.iter().any(|probe| match &body {
                Body::Query(q) => match q.bind(pname, probe.clone()) {
                    Ok(_) => true,
                    Err(e) => {
                        last_err = e;
                        false
                    }
                },
                Body::Update(u) => match u.bind(pname, probe.clone()) {
                    Ok(_) => true,
                    Err(e) => {
                        last_err = e;
                        false
                    }
                },
            });
            if !ok {
                return Err(format!(
                    "declared parameter `{}` is not bindable in the template: {}",
                    pname, last_err
                ));
            }
        }
        Ok(Template {
            name: name.to_string(),
            text: text.to_string(),
            description,
            params,
            body,
        })
    }

    /// The template's identifier (an IRI by convention).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The stored SPARQL text, verbatim as registered.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The optional human/agent-facing description.
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// The declared parameters (name → type), in stable (sorted) order.
    pub fn params(&self) -> &BTreeMap<String, ParamType> {
        &self.params
    }

    /// Whether this template is a SPARQL UPDATE (`true`) or a query (`false`). The
    /// invocation surface uses this to apply its WRITE gate before any work.
    pub fn is_update(&self) -> bool {
        matches!(self.body, Body::Update(_))
    }

    /// `"update"` or `"query"` — the wire token for [`Template::is_update`].
    pub fn kind(&self) -> &'static str {
        if self.is_update() {
            "update"
        } else {
            "query"
        }
    }

    /// Binds a JSON argument object (`{"who": "http://ex/alice", "age": 30, …}`) into
    /// the template, returning the fully-bound algebra. Fail-closed (see the module
    /// docs): unknown argument names, missing declared parameters, JSON shapes that do
    /// not match the declared type, and slot-incompatible values are all errors.
    pub fn bind_json(&self, args: &Value) -> Result<Bound, String> {
        let empty = Map::new();
        let args = match args {
            Value::Object(m) => m,
            Value::Null => &empty,
            _ => return Err("template arguments must be a JSON object".to_string()),
        };
        // Fail-closed direction 1: an argument the template does not declare.
        if let Some(unknown) = args.keys().find(|k| !self.params.contains_key(*k)) {
            return Err(format!(
                "unknown parameter `{}` (declared: {})",
                unknown,
                self.declared_names()
            ));
        }
        // Fail-closed direction 2: a declared parameter with no argument.
        if let Some(missing) = self.params.keys().find(|k| !args.contains_key(*k)) {
            return Err(format!(
                "missing required parameter `{}` (declared: {})",
                missing,
                self.declared_names()
            ));
        }
        let mut bound = self.body.clone();
        for (pname, ptype) in &self.params {
            let term = ptype.term_for(pname, &args[pname])?;
            bound = match bound {
                Body::Query(q) => Body::Query(Box::new(q.bind(pname, term)?)),
                Body::Update(u) => Body::Update(u.bind(pname, term)?),
            };
        }
        Ok(match bound {
            Body::Query(q) => Bound::Query(q),
            Body::Update(u) => Bound::Update(u),
        })
    }

    fn declared_names(&self) -> String {
        if self.params.is_empty() {
            "none".to_string()
        } else {
            self.params
                .keys()
                .map(|k| format!("`{}`", k))
                .collect::<Vec<_>>()
                .join(", ")
        }
    }

    /// Parses a template from its JSON wire definition:
    ///
    /// ```json
    /// { "name": "http://ex/tpl/rename",
    ///   "text": "DELETE { … } INSERT { … } WHERE { … }",
    ///   "parameters": { "who": "iri", "newName": "string" },
    ///   "description": "optional" }
    /// ```
    ///
    /// `sparql` is accepted as an alias for `text`. Unknown top-level keys are rejected
    /// (fail-closed), and the definition is fully validated via [`Template::new`].
    pub fn from_json(v: &Value) -> Result<Template, String> {
        let obj = v
            .as_object()
            .ok_or_else(|| "template definition must be a JSON object".to_string())?;
        let known = ["name", "text", "sparql", "parameters", "description", "kind"];
        if let Some(k) = obj.keys().find(|k| !known.contains(&k.as_str())) {
            return Err(format!("unknown key `{}` in template definition", k));
        }
        let name = obj
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| "template definition requires a string `name`".to_string())?;
        let text = obj
            .get("text")
            .or_else(|| obj.get("sparql"))
            .and_then(Value::as_str)
            .ok_or_else(|| "template definition requires a string `text` (or `sparql`)".to_string())?;
        let description = match obj.get("description") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.clone()),
            Some(_) => return Err("template `description` must be a string".to_string()),
        };
        let mut params = BTreeMap::new();
        match obj.get("parameters") {
            None | Some(Value::Null) => {}
            Some(Value::Object(m)) => {
                for (pname, ptype) in m {
                    let ptype = ptype.as_str().ok_or_else(|| {
                        format!("parameter `{}` type must be a string", pname)
                    })?;
                    params.insert(pname.clone(), ParamType::parse(ptype)?);
                }
            }
            Some(_) => {
                return Err("template `parameters` must be an object of name → type".to_string())
            }
        }
        let t = Template::new(name, text, params, description)?;
        // `kind`, when present, must agree with what the text parses as (fail-closed —
        // a definition that says "query" but carries an UPDATE is a mistake).
        if let Some(k) = obj.get("kind").and_then(Value::as_str) {
            if k != t.kind() {
                return Err(format!(
                    "template `kind` says `{}` but the text is a {}",
                    k,
                    t.kind()
                ));
            }
        }
        Ok(t)
    }

    /// The JSON wire definition this template round-trips to (the inverse of
    /// [`Template::from_json`], plus the derived `kind`).
    pub fn to_json(&self) -> Value {
        let params: Map<String, Value> = self
            .params
            .iter()
            .map(|(k, v)| (k.clone(), Value::String(v.as_str().to_string())))
            .collect();
        let mut out = json!({
            "name": self.name,
            "kind": self.kind(),
            "text": self.text,
            "parameters": Value::Object(params),
        });
        if let Some(d) = &self.description {
            out["description"] = Value::String(d.clone());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{query_prepared, update_prepared};
    use sparq_core::Graph;

    const DATA: &str = r#"
        @prefix ex: <http://ex/> .
        ex:alice ex:knows ex:bob ; ex:name "Alice" ; ex:age 30 .
        ex:bob   ex:name "Bob" ; ex:age 41 .
    "#;

    fn g() -> Graph {
        Graph::load_str(DATA, "turtle").unwrap()
    }

    fn decl(pairs: &[(&str, ParamType)]) -> BTreeMap<String, ParamType> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    // ParamType::parse — every keyword + a datatype IRI + fail-closed unknown.
    #[test]
    fn param_type_parse_round_trips() {
        for tok in ["auto", "iri", "string", "boolean", "integer", "decimal", "double"] {
            assert_eq!(ParamType::parse(tok).unwrap().as_str(), tok);
        }
        let dt = "http://www.w3.org/2001/XMLSchema#date";
        assert_eq!(ParamType::parse(dt).unwrap(), ParamType::Datatype(dt.to_string()));
        assert_eq!(ParamType::parse(dt).unwrap().as_str(), dt);
        assert!(ParamType::parse("intt").is_err());
        assert!(ParamType::parse("not an iri:").is_err());
    }

    // Template::new — parses a query, records name/text/description/params/kind.
    #[test]
    fn new_query_template_accessors() {
        let t = Template::new(
            "http://ex/tpl/friends",
            "SELECT ?f WHERE { ?who <http://ex/knows> ?f }",
            decl(&[("who", ParamType::Iri)]),
            Some("friends of ?who".to_string()),
        )
        .unwrap();
        assert_eq!(t.name(), "http://ex/tpl/friends");
        assert!(t.text().starts_with("SELECT ?f"));
        assert_eq!(t.description(), Some("friends of ?who"));
        assert_eq!(t.params().len(), 1);
        assert!(!t.is_update());
        assert_eq!(t.kind(), "query");
    }

    // Template::new — an UPDATE text is classified as an update.
    #[test]
    fn new_update_template_is_update() {
        let t = Template::new(
            "tpl-insert",
            "INSERT { <http://ex/m> <http://ex/note> ?note } WHERE { }",
            decl(&[("note", ParamType::String)]),
            None,
        )
        .unwrap();
        assert!(t.is_update());
        assert_eq!(t.kind(), "update");
        assert_eq!(t.description(), None);
    }

    // Fail-closed registration: unparseable text, undeclared-free param, empty name.
    #[test]
    fn new_rejects_bad_definitions() {
        assert!(Template::new("t", "NOT SPARQL", BTreeMap::new(), None)
            .unwrap_err()
            .contains("neither a query"));
        // Declared parameter that is not free in the text.
        let err = Template::new(
            "t",
            "SELECT ?s WHERE { ?s ?p ?o }",
            decl(&[("missing", ParamType::Iri)]),
            None,
        )
        .unwrap_err();
        assert!(err.contains("`missing`"), "{}", err);
        // A literal-typed parameter in an IRI-only (predicate) slot is caught at
        // registration, not at invocation.
        let err = Template::new(
            "t",
            "SELECT ?o WHERE { <http://ex/alice> ?prop ?o }",
            decl(&[("prop", ParamType::String)]),
            None,
        )
        .unwrap_err();
        assert!(err.contains("`prop`"), "{}", err);
        assert!(Template::new("  ", "ASK { ?s ?p ?o }", BTreeMap::new(), None).is_err());
    }

    // bind_json — typed binding produces the same rows as the hand-written constant query.
    #[test]
    fn bind_json_query_matches_constant_query() {
        let t = Template::new(
            "t",
            "SELECT ?f WHERE { ?who <http://ex/knows> ?f }",
            decl(&[("who", ParamType::Iri)]),
            None,
        )
        .unwrap();
        let bound = t.bind_json(&json!({"who": "http://ex/alice"})).unwrap();
        let Bound::Query(pq) = bound else {
            panic!("query template must bind to Bound::Query")
        };
        let rows = query_prepared(&g(), &pq).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows.rows[0][0].as_ref().unwrap().to_string(), "<http://ex/bob>");
    }

    // bind_json — fail-closed: unknown / missing / wrong-shape / non-object arguments.
    #[test]
    fn bind_json_fail_closed() {
        let t = Template::new(
            "t",
            "SELECT ?s WHERE { ?s <http://ex/age> ?age }",
            decl(&[("age", ParamType::Integer)]),
            None,
        )
        .unwrap();
        let err = t.bind_json(&json!({"age": 30, "typo": 1})).unwrap_err();
        assert!(err.contains("unknown parameter `typo`"), "{}", err);
        let err = t.bind_json(&json!({})).unwrap_err();
        assert!(err.contains("missing required parameter `age`"), "{}", err);
        // Wrong JSON shape: a string is NOT an integer (no coercion).
        let err = t.bind_json(&json!({"age": "30"})).unwrap_err();
        assert!(err.contains("expects a JSON integer"), "{}", err);
        assert!(t.bind_json(&json!(["age"])).is_err());
        // The typed binding matches typed data.
        let Bound::Query(pq) = t.bind_json(&json!({"age": 30})).unwrap() else {
            panic!()
        };
        assert_eq!(query_prepared(&g(), &pq).unwrap().len(), 1);
    }

    // bind_json — auto typing: string/number/boolean + the explicit object shapes.
    #[test]
    fn bind_json_auto_shapes() {
        let t = Template::new(
            "t",
            "SELECT ?s WHERE { ?s <http://ex/name> ?v }",
            decl(&[("v", ParamType::Auto)]),
            None,
        )
        .unwrap();
        // Plain string ⇒ xsd:string literal, matches "Alice".
        let Bound::Query(pq) = t.bind_json(&json!({"v": "Alice"})).unwrap() else {
            panic!()
        };
        assert_eq!(query_prepared(&g(), &pq).unwrap().len(), 1);
        // Explicit IRI object binds an IRI (matches nothing here, but binds cleanly).
        assert!(t.bind_json(&json!({"v": {"iri": "http://ex/alice"}})).is_ok());
        // Explicit datatype + lang objects.
        assert!(t
            .bind_json(&json!({"v": {"value": "30", "datatype": "http://www.w3.org/2001/XMLSchema#integer"}}))
            .is_ok());
        assert!(t.bind_json(&json!({"v": {"value": "chat", "lang": "fr"}})).is_ok());
        // Fail-closed object shapes: unknown key / conflicting keys / bad IRI.
        assert!(t.bind_json(&json!({"v": {"iri": "http://ex/a", "value": "x"}})).is_err());
        assert!(t.bind_json(&json!({"v": {"wat": "x"}})).is_err());
        assert!(t.bind_json(&json!({"v": {"iri": "not an iri"}})).is_err());
        // Auto numbers/booleans type themselves.
        assert!(t.bind_json(&json!({"v": 30})).is_ok());
        assert!(t.bind_json(&json!({"v": 1.5})).is_ok());
        assert!(t.bind_json(&json!({"v": true})).is_ok());
        // A JSON array is not a bindable value.
        assert!(t.bind_json(&json!({"v": [1]})).is_err());
    }

    // bind_json — an UPDATE template binds and applies; the hostile-literal injection
    // scenario stays data (the #901 invariant carried through the template layer).
    #[test]
    fn bind_json_update_applies_and_is_injection_safe() {
        let t = Template::new(
            "t",
            "INSERT { <http://ex/m> <http://ex/note> ?note } WHERE { }",
            decl(&[("note", ParamType::String)]),
            None,
        )
        .unwrap();
        let hostile = r#"x" } ; DROP ALL ; INSERT DATA { <http://ex/evil> <http://ex/p> <http://ex/o> } # "#;
        let Bound::Update(pu) = t.bind_json(&json!({ "note": hostile })).unwrap() else {
            panic!("update template must bind to Bound::Update")
        };
        // Exactly ONE operation — no injected DROP ALL.
        assert_eq!(pu.update().operations.len(), 1);
        let updated = update_prepared(&g(), &pu).unwrap();
        assert!(crate::ask(&updated, "ASK { <http://ex/alice> <http://ex/name> ?n }").unwrap());
        assert!(crate::ask(&updated, "ASK { <http://ex/m> <http://ex/note> ?n }").unwrap());
        assert!(!crate::ask(&updated, "ASK { <http://ex/evil> ?p ?o }").unwrap());
    }

    // from_json / to_json — full round trip, aliases, and fail-closed definitions.
    #[test]
    fn from_json_to_json_round_trip() {
        let def = json!({
            "name": "http://ex/tpl/rename",
            "text": "DELETE { ?s <http://ex/name> ?old } INSERT { ?s <http://ex/name> ?new } \
                     WHERE { ?s <http://ex/name> ?old . FILTER(?s = ?who) }",
            "parameters": { "who": "iri", "new": "string" },
            "description": "rename ?who"
        });
        let t = Template::from_json(&def).unwrap();
        assert_eq!(t.kind(), "update");
        let back = t.to_json();
        assert_eq!(back["name"], def["name"]);
        assert_eq!(back["text"], def["text"]);
        assert_eq!(back["parameters"], def["parameters"]);
        assert_eq!(back["description"], def["description"]);
        assert_eq!(back["kind"], "update");
        // Round-trip parses again (to_json output is a valid definition).
        assert_eq!(Template::from_json(&back).unwrap().name(), t.name());
        // `sparql` alias for `text`.
        assert!(Template::from_json(&json!({
            "name": "t", "sparql": "ASK { ?s ?p ?o }"
        }))
        .is_ok());
        // Fail-closed: unknown key, missing name/text, bad parameter type, kind mismatch.
        assert!(Template::from_json(&json!({"name": "t", "text": "ASK { ?s ?p ?o }", "extra": 1}))
            .unwrap_err()
            .contains("unknown key `extra`"));
        assert!(Template::from_json(&json!({"text": "ASK { ?s ?p ?o }"})).is_err());
        assert!(Template::from_json(&json!({"name": "t"})).is_err());
        assert!(Template::from_json(&json!({
            "name": "t", "text": "ASK { ?s ?p ?o }", "parameters": {"x": "intt"}
        }))
        .is_err());
        assert!(Template::from_json(&json!({
            "name": "t", "text": "ASK { ?s ?p ?o }", "kind": "update"
        }))
        .unwrap_err()
        .contains("kind"));
        assert!(Template::from_json(&json!("nope")).is_err());
    }

    // Bound is Clone + Debug (the surfaces move it across a spawn boundary).
    #[test]
    fn bound_is_clone_debug() {
        let t = Template::new("t", "ASK { ?s ?p ?o }", BTreeMap::new(), None).unwrap();
        let b = t.bind_json(&json!({})).unwrap();
        let b2 = b.clone();
        assert!(format!("{:?}", b2).contains("Query"));
    }

    // Bound::is_update / is_graph_form / render — the surface-facing classification +
    // canonical rendering helpers (render round-trips through a fresh parse, and the
    // bound value survives as data).
    #[test]
    fn bound_classification_and_render() {
        let ask = Template::new("t", "ASK { ?s ?p ?o }", BTreeMap::new(), None)
            .unwrap()
            .bind_json(&json!({}))
            .unwrap();
        assert!(!ask.is_update());
        assert!(!ask.is_graph_form());
        let construct = Template::new(
            "t",
            "CONSTRUCT { ?s <http://ex/p> ?o } WHERE { ?s <http://ex/knows> ?o }",
            BTreeMap::new(),
            None,
        )
        .unwrap()
        .bind_json(&json!({}))
        .unwrap();
        assert!(construct.is_graph_form());
        let upd = Template::new(
            "t",
            "INSERT { <http://ex/m> <http://ex/note> ?note } WHERE { }",
            decl(&[("note", ParamType::String)]),
            None,
        )
        .unwrap()
        .bind_json(&json!({"note": "hi \"there\""}))
        .unwrap();
        assert!(upd.is_update());
        assert!(!upd.is_graph_form());
        // render() is executable canonical SPARQL: it re-parses, and the bound literal
        // (with its quote) is carried escaped as data.
        let rendered = upd.render();
        assert!(PreparedUpdate::parse(&rendered).is_ok(), "{}", rendered);
        assert!(rendered.contains(r#"hi \"there\""#), "{}", rendered);
        assert!(PreparedQuery::parse(&ask.render()).is_ok());
    }
}
