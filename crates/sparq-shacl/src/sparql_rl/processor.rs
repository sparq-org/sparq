//! Preparation and engine-backed SPARQL-RL inference.
use super::{lexer, parse, Element, Statement};
use crate::{rule_schedule, view::GraphView};
use oxrdf::{Literal, Term, Triple, Variable};
use spargebra::{
    algebra::{
        AggregateExpression, AggregateFunction, Expression, GraphPattern, PropertyPathExpression,
    },
    term::{GroundTerm, NamedNodePattern, TermPattern, TriplePattern},
    Query, SparqlParser,
};
use sparq_core::Graph;
use sparq_engine::{PreparedQuery, QueryBudget, QueryResult};
use std::collections::{BTreeSet, HashMap};
use std::time::Duration;

type Vars = BTreeSet<String>;
type Result<T> = std::result::Result<T, RuleError>;

/// The processing stage that rejected a rule set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleErrorKind {
    /// Source syntax or lowering was rejected.
    Syntax,
    /// Variable inScope/inUse conditions do not hold.
    WellFormedness,
    /// A cycle contains a closed dependency.
    Stratification,
    /// IMPORTS is not supported by this processor.
    Imports,
    /// An operational budget was exhausted.
    Limit,
    /// Query evaluation failed.
    Evaluation,
}

/// A whole-operation failure; no partial inference is returned.
#[derive(Debug, PartialEq, Eq)]
pub struct RuleError {
    /// The processing stage.
    pub kind: RuleErrorKind,
    /// Diagnostic text.
    pub message: String,
}
impl std::fmt::Display for RuleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for RuleError {}
fn err(kind: RuleErrorKind, message: impl Into<String>) -> RuleError {
    RuleError {
        kind,
        message: message.into(),
    }
}
fn syntax(e: impl ToString) -> RuleError {
    err(RuleErrorKind::Syntax, e.to_string())
}
fn limit(message: &str) -> RuleError {
    err(RuleErrorKind::Limit, message)
}

/// Operational limits for preparation and inference. See the module defaults.
#[derive(Debug, Clone)]
pub struct Limits {
    /// Maximum rules in the source.
    pub max_rules: usize,
    /// Maximum pattern/template compatibility checks during dependency analysis.
    pub max_dependency_checks: usize,
    /// Maximum general-rule passes per stratum, including the settling pass.
    pub max_iterations: usize,
    /// Maximum engine calls across the operation.
    pub max_queries: usize,
    /// Maximum rows in an intermediate query or accumulated binding table.
    pub max_rows: usize,
    /// Maximum new inferred triples, including DATA facts.
    pub max_inferred_triples: usize,
    /// Maximum serialized bytes across new inferred triples.
    pub max_inferred_bytes: usize,
    /// Maximum template instances considered in one CONSTRUCT call.
    pub max_construct_triples: usize,
    /// Maximum conservative template-expansion byte estimate per CONSTRUCT.
    pub max_construct_bytes: usize,
    /// Engine working-set byte budget per query.
    pub max_query_bytes: usize,
    /// Deadline duration for the whole operation (native builds).
    pub timeout: Duration,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_rules: 256,
            max_dependency_checks: 1_000_000,
            max_iterations: 100,
            max_queries: 100_000,
            max_rows: 100_000,
            max_inferred_triples: 100_000,
            max_inferred_bytes: 64 * 1024 * 1024,
            max_construct_triples: 100_000,
            max_construct_bytes: 64 * 1024 * 1024,
            max_query_bytes: 64 * 1024 * 1024,
            timeout: Duration::from_secs(30),
        }
    }
}

#[derive(Clone)]
enum Op {
    Triples(Vec<TriplePattern>, Vars),
    Filter(Expression, Vars),
    Set(Variable, Expression, Vars),
    Not(bool, Vec<Op>),
    Group {
        keys: Vec<Variable>,
        body: Vec<Op>,
        aggregates: Vec<(Variable, AggregateExpression)>,
    },
}
struct CompiledRule {
    context: Query,
    head: Vec<TriplePattern>,
    head_vars: Vars,
    body: Vec<Op>,
    base_only: bool,
    once: bool,
}
struct Program {
    rules: Vec<CompiledRule>,
    seeds: Vec<Vec<TriplePattern>>,
    imports: bool,
}

struct Compiler {
    parser: SparqlParser,
    occupied: Vars,
    blanks: HashMap<String, Variable>,
    serial: usize,
}
impl Compiler {
    fn fresh(&mut self) -> Variable {
        loop {
            let name = format!("__srl_internal_{}", self.serial);
            self.serial += 1;
            if self.occupied.insert(name.clone()) {
                return Variable::new_unchecked(name);
            }
        }
    }
    fn blank(&mut self, name: &str) -> Variable {
        if let Some(v) = self.blanks.get(name) {
            return v.clone();
        }
        let v = self.fresh();
        self.blanks.insert(name.into(), v.clone());
        v
    }
    fn query(&self, text: &str) -> Result<Query> {
        self.parser.clone().parse_query(text).map_err(syntax)
    }
    fn expression(&self, text: &str) -> Result<Expression> {
        let Query::Select { pattern, .. } =
            self.query(&format!("SELECT ({text} AS ?__srl_result) WHERE {{}}"))?
        else {
            unreachable!()
        };
        let GraphPattern::Project { inner, .. } = pattern else {
            return Err(syntax("Expected expression projection"));
        };
        let GraphPattern::Extend { expression, .. } = *inner else {
            return Err(syntax("Expected expression"));
        };
        Ok(expression)
    }
    fn template(&self, text: &str) -> Result<Vec<TriplePattern>> {
        let Query::Construct { template, .. } =
            self.query(&format!("CONSTRUCT {text} WHERE {{}}"))?
        else {
            unreachable!()
        };
        Ok(template)
    }
    fn body_text(&mut self, text: &str) -> Result<String> {
        let mut out = String::new();
        let mut start = 0;
        for token in lexer::lex(text).map_err(syntax)? {
            if token.kind == lexer::Kind::Blank {
                out.push_str(&text[start..token.start]);
                out.push_str(&self.blank(token.text).to_string());
                start = token.end;
            }
        }
        out.push_str(&text[start..]);
        Ok(out)
    }
    fn term(&mut self, term: &mut TermPattern) {
        match term {
            TermPattern::BlankNode(n) => {
                *term = TermPattern::Variable(self.blank(&format!("anon:{}", n.as_str())))
            }
            TermPattern::Triple(t) => {
                self.term(&mut t.subject);
                self.term(&mut t.object);
            }
            _ => {}
        }
    }
    fn flatten(&mut self, pattern: GraphPattern, out: &mut Vec<TriplePattern>) -> Result<()> {
        match pattern {
            GraphPattern::Bgp { mut patterns } => {
                for t in &mut patterns {
                    self.term(&mut t.subject);
                    self.term(&mut t.object);
                }
                out.extend(patterns);
            }
            GraphPattern::Join { left, right } => {
                self.flatten(*left, out)?;
                self.flatten(*right, out)?;
            }
            GraphPattern::Path {
                mut subject,
                path,
                mut object,
            } => {
                self.term(&mut subject);
                self.term(&mut object);
                self.path(subject, path, object, out)?;
            }
            _ => return Err(syntax("Unsupported triple-block lowering")),
        }
        Ok(())
    }
    fn path(
        &mut self,
        subject: TermPattern,
        path: PropertyPathExpression,
        object: TermPattern,
        out: &mut Vec<TriplePattern>,
    ) -> Result<()> {
        match path {
            PropertyPathExpression::NamedNode(predicate) => out.push(TriplePattern {
                subject,
                predicate: predicate.into(),
                object,
            }),
            PropertyPathExpression::Reverse(p) => self.path(object, *p, subject, out)?,
            PropertyPathExpression::Sequence(a, b) => {
                let middle = TermPattern::Variable(self.fresh());
                self.path(subject, *a, middle.clone(), out)?;
                self.path(middle, *b, object, out)?;
            }
            _ => return Err(syntax("SPARQL-RL allows only sequence and inverse paths")),
        }
        Ok(())
    }
    fn ops(&mut self, body: &[Element<'_>]) -> Result<Vec<Op>> {
        body.iter()
            .map(|element| {
                Ok(match element {
                    Element::Triples(text) => {
                        let names = variables(text)?;
                        let text = self.body_text(text)?;
                        let Query::Select { pattern, .. } =
                            self.query(&format!("SELECT * WHERE {{{text}}}"))?
                        else {
                            unreachable!()
                        };
                        let pattern = match pattern {
                            GraphPattern::Project { inner, .. } => *inner,
                            other => other,
                        };
                        let mut triples = Vec::new();
                        self.flatten(pattern, &mut triples)?;
                        Op::Triples(triples, names)
                    }
                    Element::Filter(text) => Op::Filter(self.expression(text)?, variables(text)?),
                    Element::Assignment {
                        variable: v,
                        expression,
                    } => Op::Set(
                        variable(v),
                        self.expression(expression)?,
                        variables(expression)?,
                    ),
                    Element::Negation { base_only, body } => Op::Not(*base_only, self.ops(body)?),
                    Element::Group {
                        keys,
                        body,
                        assignments,
                    } => {
                        let aggregates = assignments
                            .iter()
                            .map(|a| {
                                let tokens = lexer::lex(a.function).map_err(syntax)?;
                                let distinct = tokens
                                    .iter()
                                    .any(|t| t.text.eq_ignore_ascii_case("DISTINCT"));
                                let aggregate = if tokens.iter().any(|t| t.text == "*") {
                                    AggregateExpression::CountSolutions { distinct }
                                } else {
                                    let name = match tokens[0].text.to_ascii_uppercase().as_str() {
                                        "COUNT" => AggregateFunction::Count,
                                        "MIN" => AggregateFunction::Min,
                                        "MAX" => AggregateFunction::Max,
                                        "SUM" => AggregateFunction::Sum,
                                        _ => return Err(syntax("Unknown aggregate")),
                                    };
                                    let arg = tokens
                                        .iter()
                                        .find(|t| t.kind == lexer::Kind::Var)
                                        .ok_or_else(|| syntax("Missing aggregate argument"))?;
                                    AggregateExpression::FunctionCall {
                                        name,
                                        expr: Expression::Variable(variable(arg.text)),
                                        distinct,
                                    }
                                };
                                Ok((variable(a.variable), aggregate))
                            })
                            .collect::<Result<Vec<_>>>()?;
                        let mut key_vars = Vec::new();
                        for key in keys {
                            let key = variable(key);
                            if !key_vars.contains(&key) {
                                key_vars.push(key);
                            }
                        }
                        Op::Group {
                            keys: key_vars,
                            body: self.ops(body)?,
                            aggregates,
                        }
                    }
                })
            })
            .collect()
    }
}
fn variable(text: &str) -> Variable {
    Variable::new_unchecked(&text[1..])
}
fn variables(text: &str) -> Result<Vars> {
    Ok(lexer::lex(text)
        .map_err(syntax)?
        .into_iter()
        .filter(|t| t.kind == lexer::Kind::Var)
        .map(|t| t.text[1..].into())
        .collect())
}
fn compile(source: &str, base: &str, limits: &Limits) -> Result<Program> {
    let doc = parse(source, base).map_err(|e| {
        err(
            if e.kind == super::ErrorKind::Limit {
                RuleErrorKind::Limit
            } else {
                RuleErrorKind::Syntax
            },
            e.to_string(),
        )
    })?;
    let mut compiler = Compiler {
        parser: SparqlParser::new().with_base_iri(base).map_err(syntax)?,
        occupied: variables(source)?,
        blanks: HashMap::new(),
        serial: 0,
    };
    let mut program = Program {
        rules: Vec::new(),
        seeds: Vec::new(),
        imports: false,
    };
    for statement in doc.statements {
        match statement {
            Statement::Base(iri) | Statement::Prefix { iri, .. } => {
                let query = compiler.query(&format!("BASE {iri} SELECT * WHERE {{}}"))?;
                let resolved = query.base_iri().unwrap().as_str();
                compiler.parser = if let Statement::Prefix { prefix, .. } = statement {
                    compiler
                        .parser
                        .with_prefix(&prefix[..prefix.len() - 1], resolved)
                        .map_err(syntax)?
                } else {
                    compiler.parser.with_base_iri(resolved).map_err(syntax)?
                };
            }
            Statement::Version(text) => {
                if !matches!(compiler.expression(text)?, Expression::Literal(l) if l.value() == "1.2")
                {
                    return Err(err(
                        RuleErrorKind::Evaluation,
                        "Unsupported SPARQL-RL version",
                    ));
                }
            }
            Statement::Imports(_) => program.imports = true,
            Statement::Data(text) => program.seeds.push(compiler.template(text)?),
            Statement::Rule(rule) => {
                if program.rules.len() >= limits.max_rules {
                    return Err(limit("Rule budget exceeded"));
                }
                compiler.blanks.clear();
                let head = compiler.template(rule.head)?;
                let body = compiler.ops(&rule.body)?;
                let once = head.iter().any(generates_terms)
                    || body
                        .iter()
                        .any(|o| matches!(o, Op::Set(..) | Op::Group { .. }));
                program.rules.push(CompiledRule {
                    context: compiler.query("SELECT * WHERE {}")?,
                    head,
                    head_vars: variables(rule.head)?,
                    body,
                    base_only: rule.base_only,
                    once,
                });
            }
        }
    }
    Ok(program)
}
fn term_vars(t: &TermPattern, vars: &mut Vars) {
    match t {
        TermPattern::Variable(v) => {
            vars.insert(v.as_str().into());
        }
        TermPattern::Triple(t) => {
            term_vars(&t.subject, vars);
            term_vars(&t.object, vars);
            if let NamedNodePattern::Variable(v) = &t.predicate {
                vars.insert(v.as_str().into());
            }
        }
        _ => {}
    }
}
fn generates_terms(t: &TriplePattern) -> bool {
    fn generates(t: &TermPattern) -> bool {
        match t {
            TermPattern::BlankNode(_) => true,
            TermPattern::Triple(triple) => {
                let mut vars = Vars::new();
                term_vars(t, &mut vars);
                !vars.is_empty() || generates(&triple.subject) || generates(&triple.object)
            }
            _ => false,
        }
    }
    generates(&t.subject) || generates(&t.object)
}
fn well_formed_ops(ops: &[Op], mut scope: Vars, mut used: Vars) -> Result<(Vars, Vars)> {
    let failure = || {
        err(
            RuleErrorKind::WellFormedness,
            "Variable inScope/inUse condition violated",
        )
    };
    for op in ops {
        match op {
            Op::Triples(_, vars) => {
                if !vars.is_disjoint(&used) {
                    return Err(failure());
                }
                scope.extend(vars.iter().cloned());
            }
            Op::Filter(_, vars) => {
                if !vars.is_subset(&scope) {
                    return Err(failure());
                }
            }
            Op::Set(target, _, vars) => {
                if !vars.is_subset(&scope)
                    || scope.contains(target.as_str())
                    || used.contains(target.as_str())
                {
                    return Err(failure());
                }
                scope.insert(target.as_str().into());
            }
            Op::Not(_, inner) => {
                let (v, u) = well_formed_ops(inner, scope.clone(), used.clone())?;
                used.extend(v.union(&u).filter(|v| !scope.contains(*v)).cloned());
            }
            Op::Group {
                keys,
                body,
                aggregates,
            } => {
                let keys: Vars = keys.iter().map(|v| v.as_str().into()).collect();
                let initial = scope.intersection(&keys).cloned().collect();
                let mut inner_used = used.clone();
                inner_used.extend(scope.difference(&keys).cloned());
                let (v, u) = well_formed_ops(body, initial, inner_used)?;
                if !keys.is_subset(&v) {
                    return Err(failure());
                }
                let mut assigned = Vars::new();
                for (target, aggregate) in aggregates {
                    let name = target.as_str();
                    if scope.contains(name)
                        || used.contains(name)
                        || v.contains(name)
                        || u.contains(name)
                        || !assigned.insert(name.into())
                    {
                        return Err(failure());
                    }
                    if let AggregateExpression::FunctionCall {
                        expr: Expression::Variable(arg),
                        ..
                    } = aggregate
                    {
                        if !v.contains(arg.as_str()) {
                            return Err(failure());
                        }
                    }
                }
                scope.extend(keys);
                scope.extend(assigned);
                used.extend(v.union(&u).filter(|v| !scope.contains(*v)).cloned());
            }
        }
    }
    Ok((scope, used))
}
fn reject_imports(program: &Program) -> Result<()> {
    if program.imports {
        Err(err(RuleErrorKind::Imports, "IMPORTS is unsupported"))
    } else {
        Ok(())
    }
}
fn well_formed(program: &Program) -> Result<()> {
    for rule in &program.rules {
        let (scope, _) = well_formed_ops(&rule.body, Vars::new(), Vars::new())?;
        if !rule.head_vars.is_subset(&scope) {
            return Err(err(
                RuleErrorKind::WellFormedness,
                "Head variable is not in scope",
            ));
        }
    }
    Ok(())
}

/// Check the rule-set variable well-formedness conditions, without evaluating it.
pub fn check_well_formed(source: &str, base: &str) -> Result<()> {
    let program = compile(source, base, &Limits::default())?;
    reject_imports(&program)?;
    well_formed(&program)
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct UVar {
    side: u8,
    name: String,
    blank: bool,
}
#[derive(Clone)]
enum UTerm {
    Variable(UVar),
    Constant(Term),
    Triple(Box<[UTerm; 3]>),
}
fn unify_term(t: &TermPattern, side: u8) -> UTerm {
    match t {
        TermPattern::Variable(v) => UTerm::Variable(UVar {
            side,
            name: v.as_str().into(),
            blank: false,
        }),
        TermPattern::BlankNode(b) => UTerm::Variable(UVar {
            side,
            name: b.as_str().into(),
            blank: true,
        }),
        TermPattern::NamedNode(n) => UTerm::Constant(n.clone().into()),
        TermPattern::Literal(l) => UTerm::Constant(l.clone().into()),
        TermPattern::Triple(t) => UTerm::Triple(Box::new(unify_triple(t, side))),
    }
}
fn unify_triple(t: &TriplePattern, side: u8) -> [UTerm; 3] {
    [
        unify_term(&t.subject, side),
        unify_term(&TermPattern::from(t.predicate.clone()), side),
        unify_term(&t.object, side),
    ]
}
fn resolve(t: UTerm, bindings: &HashMap<UVar, UTerm>) -> UTerm {
    if let UTerm::Variable(v) = &t {
        if let Some(t) = bindings.get(v) {
            return resolve(t.clone(), bindings);
        }
    }
    t
}
fn occurs(v: &UVar, t: UTerm, bindings: &HashMap<UVar, UTerm>) -> bool {
    match resolve(t, bindings) {
        UTerm::Variable(w) => *v == w,
        UTerm::Triple(t) => t.iter().any(|t| occurs(v, t.clone(), bindings)),
        _ => false,
    }
}
fn unify(a: UTerm, b: UTerm, bindings: &mut HashMap<UVar, UTerm>) -> bool {
    let a = resolve(a, bindings);
    let b = resolve(b, bindings);
    match (a, b) {
        (UTerm::Variable(a), UTerm::Variable(b)) if a == b => true,
        (UTerm::Variable(a), UTerm::Variable(b)) => {
            // Preserve the more restrictive blank-node domain through aliases.
            if a.blank {
                bindings.insert(b, UTerm::Variable(a));
            } else {
                bindings.insert(a, UTerm::Variable(b));
            }
            true
        }
        (UTerm::Variable(v), t) | (t, UTerm::Variable(v)) => {
            if v.blank && !matches!(&t, UTerm::Constant(Term::BlankNode(_)))
                || occurs(&v, t.clone(), bindings)
            {
                return false;
            }
            bindings.insert(v, t);
            true
        }
        (UTerm::Constant(a), UTerm::Constant(b)) => a == b,
        (UTerm::Triple(a), UTerm::Triple(b)) => a
            .iter()
            .zip(b.iter())
            .all(|(a, b)| unify(a.clone(), b.clone(), bindings)),
        _ => false,
    }
}
fn possible_triple(triple: [UTerm; 3], bindings: &HashMap<UVar, UTerm>) -> bool {
    let [subject, predicate, object] = triple.map(|t| resolve(t, bindings));
    let subject_ok = matches!(
        subject,
        UTerm::Variable(_) | UTerm::Constant(Term::NamedNode(_) | Term::BlankNode(_))
    );
    let predicate_ok = match predicate {
        UTerm::Variable(v) => !v.blank,
        UTerm::Constant(Term::NamedNode(_)) => true,
        _ => false,
    };
    let object_ok = match object {
        UTerm::Triple(t) => possible_triple(*t, bindings),
        _ => true,
    };
    subject_ok && predicate_ok && object_ok
}
fn compatible(a: &TriplePattern, b: &TriplePattern) -> bool {
    let mut bindings = HashMap::new();
    let left = unify_triple(a, 0);
    let right = unify_triple(b, 1);
    left.iter()
        .cloned()
        .zip(right.iter().cloned())
        .all(|(a, b)| unify(a, b, &mut bindings))
        // A common substitution must still describe an RDF triple. Repeated
        // variables can otherwise force a literal into subject/predicate position.
        && possible_triple(left, &bindings)
        && possible_triple(right, &bindings)
}
fn dependencies(ops: &[Op], closed: bool, out: &mut Vec<(TriplePattern, bool)>) {
    for op in ops {
        match op {
            Op::Triples(triples, _) => out.extend(triples.iter().cloned().map(|t| (t, closed))),
            Op::Not(false, inner) => dependencies(inner, true, out),
            Op::Group { body, .. } => dependencies(body, true, out),
            _ => {}
        }
    }
}
fn strata(program: &Program, limits: &Limits) -> Result<Vec<usize>> {
    let n = program.rules.len();
    let mut edges = Vec::new();
    let mut checks = 0;
    for (i, rule) in program.rules.iter().enumerate() {
        if rule.base_only {
            continue;
        }
        let mut patterns = Vec::new();
        dependencies(&rule.body, rule.once, &mut patterns);
        for (j, producer) in program.rules.iter().enumerate() {
            let mut label = None;
            for (pattern, closed) in &patterns {
                for head in &producer.head {
                    checks += 1;
                    if checks > limits.max_dependency_checks {
                        return Err(limit("Dependency analysis budget exceeded"));
                    }
                    if compatible(pattern, head) {
                        label = Some(label.unwrap_or(false) || *closed);
                    }
                }
            }
            if let Some(closed) = label {
                edges.push((i, j, usize::from(closed)));
            }
        }
    }
    let mut levels = vec![0; n];
    for _ in 0..=n {
        let mut changed = false;
        for &(from, to, weight) in &edges {
            let required = levels[to] + weight;
            if levels[from] < required {
                if required >= n {
                    return Err(err(
                        RuleErrorKind::Stratification,
                        "Closed dependency cycle",
                    ));
                }
                levels[from] = required;
                changed = true;
            }
        }
        if !changed {
            return Ok(levels);
        }
    }
    Err(err(
        RuleErrorKind::Stratification,
        "Closed dependency cycle",
    ))
}

/// Compute a stratum number for each rule in source order, without evaluating it.
/// This stage is independent of the variable well-formedness check.
pub fn stratify(source: &str, base: &str) -> Result<Vec<usize>> {
    let limits = Limits::default();
    let program = compile(source, base, &limits)?;
    reject_imports(&program)?;
    strata(&program, &limits)
}

#[derive(Clone)]
struct Rows {
    vars: Vec<Variable>,
    rows: Vec<Vec<Option<Term>>>,
}
impl Rows {
    fn unit() -> Self {
        Self {
            vars: Vec::new(),
            rows: vec![Vec::new()],
        }
    }
    fn table(&self) -> GraphPattern {
        let mut ground = Vec::new();
        let mut tables = Vec::new();
        for row in &self.rows {
            let converted = row
                .iter()
                .map(|t| t.clone().map(GroundTerm::try_from).transpose())
                .collect::<std::result::Result<Vec<_>, _>>();
            if let Ok(row) = converted {
                ground.push(row);
            } else {
                let bindings: Vec<_> = self
                    .vars
                    .iter()
                    .zip(row)
                    .filter_map(|(v, t)| t.as_ref().map(|t| (v.as_str(), t)))
                    .collect();
                tables.push(crate::sparql::pre_bind_values(&bindings).expect("RDF binding table"));
            }
        }
        if !ground.is_empty() || tables.is_empty() {
            tables.push(GraphPattern::Values {
                variables: self.vars.clone(),
                bindings: ground,
            });
        }
        // Balance unions so a large blank-node table does not create linear AST depth.
        while tables.len() > 1 {
            let mut next = Vec::new();
            let mut iter = tables.into_iter();
            while let Some(left) = iter.next() {
                next.push(if let Some(right) = iter.next() {
                    GraphPattern::Union {
                        left: Box::new(left),
                        right: Box::new(right),
                    }
                } else {
                    left
                });
            }
            tables = next;
        }
        tables.pop().unwrap()
    }
    fn one(&self, row: &[Option<Term>]) -> Self {
        Self {
            vars: self.vars.clone(),
            rows: vec![row.to_vec()],
        }
    }
    fn keys(&self, row: &[Option<Term>], keys: &[Variable]) -> Self {
        let mut vars = Vec::new();
        let mut values = Vec::new();
        for key in keys {
            if let Some(i) = self.vars.iter().position(|v| v == key) {
                vars.push(key.clone());
                values.push(row[i].clone());
            }
        }
        Self {
            vars,
            rows: vec![values],
        }
    }
    fn append(&mut self, other: Self, max_rows: usize) -> Result<()> {
        if self.rows.len().saturating_add(other.rows.len()) > max_rows {
            return Err(limit("Solution row budget exceeded"));
        }
        for row in other.rows {
            self.rows.push(
                self.vars
                    .iter()
                    .map(|v| {
                        other
                            .vars
                            .iter()
                            .position(|w| w == v)
                            .and_then(|i| row[i].clone())
                    })
                    .collect(),
            );
        }
        Ok(())
    }
}
fn join(left: GraphPattern, right: GraphPattern) -> GraphPattern {
    GraphPattern::Join {
        left: Box::new(left),
        right: Box::new(right),
    }
}
fn select(pattern: GraphPattern) -> Query {
    Query::Select {
        dataset: None,
        pattern,
        base_iri: None,
    }
}
fn construct(head: &[TriplePattern], pattern: GraphPattern) -> Query {
    Query::Construct {
        template: head.to_vec(),
        dataset: None,
        pattern,
        base_iri: None,
    }
}
fn filter(expr: Expression, pattern: GraphPattern) -> GraphPattern {
    GraphPattern::Filter {
        expr,
        inner: Box::new(pattern),
    }
}
fn simple(ops: &[Op], mut pattern: GraphPattern) -> GraphPattern {
    for op in ops {
        pattern = match op {
            Op::Triples(triples, _) => join(
                pattern,
                GraphPattern::Bgp {
                    patterns: triples.clone(),
                },
            ),
            Op::Filter(expr, _) => filter(expr.clone(), pattern),
            Op::Set(variable, expression, _) => filter(
                Expression::Bound(variable.clone()),
                GraphPattern::Extend {
                    inner: Box::new(pattern),
                    variable: variable.clone(),
                    expression: expression.clone(),
                },
            ),
            _ => unreachable!("Restricted inner pattern"),
        };
    }
    pattern
}
// Resource estimation only; actual substitution and blank allocation stay in
// the engine's CONSTRUCT implementation. Include ample fresh-label/punctuation
// allowance, and count every candidate before deduplication.
fn iri_bytes(node: &oxrdf::NamedNode) -> usize {
    node.as_str().len().saturating_mul(6).saturating_add(16)
}
fn literal_bytes(literal: &Literal) -> usize {
    literal
        .value()
        .len()
        .saturating_add(literal.datatype().as_str().len())
        .saturating_add(literal.language().map_or(0, str::len))
        .saturating_mul(6)
        .saturating_add(64)
}
fn value_bytes(term: &Term) -> usize {
    match term {
        Term::NamedNode(n) => iri_bytes(n),
        Term::BlankNode(n) => n.as_str().len().saturating_mul(6).saturating_add(16),
        Term::Literal(l) => literal_bytes(l),
        Term::Triple(t) => {
            let subject = match &t.subject {
                oxrdf::NamedOrBlankNode::NamedNode(n) => iri_bytes(n),
                oxrdf::NamedOrBlankNode::BlankNode(n) => {
                    n.as_str().len().saturating_mul(6).saturating_add(16)
                }
            };
            subject
                .saturating_add(iri_bytes(&t.predicate))
                .saturating_add(value_bytes(&t.object))
                .saturating_add(32)
        }
    }
}
fn template_variable_bytes(
    variable: &Variable,
    columns: &HashMap<&str, usize>,
    row: &[Option<Term>],
) -> usize {
    columns
        .get(variable.as_str())
        .and_then(|&i| row[i].as_ref())
        .map_or(0, value_bytes)
}
fn template_predicate_bytes(
    term: &NamedNodePattern,
    columns: &HashMap<&str, usize>,
    row: &[Option<Term>],
) -> usize {
    match term {
        NamedNodePattern::NamedNode(n) => iri_bytes(n),
        NamedNodePattern::Variable(v) => template_variable_bytes(v, columns, row),
    }
}
fn template_term_bytes(
    term: &TermPattern,
    columns: &HashMap<&str, usize>,
    row: &[Option<Term>],
) -> usize {
    match term {
        TermPattern::Variable(v) => template_variable_bytes(v, columns, row),
        TermPattern::BlankNode(n) => n.as_str().len().saturating_mul(6).saturating_add(128),
        TermPattern::Triple(t) => 32_usize
            .saturating_add(template_term_bytes(&t.subject, columns, row))
            .saturating_add(template_predicate_bytes(&t.predicate, columns, row))
            .saturating_add(template_term_bytes(&t.object, columns, row)),
        TermPattern::NamedNode(n) => iri_bytes(n),
        TermPattern::Literal(l) => literal_bytes(l),
    }
}
struct Runtime<'a> {
    limits: &'a Limits,
    queries: usize,
    budget: QueryBudget,
    now: Literal,
    context: Option<Query>,
}
impl<'a> Runtime<'a> {
    fn new(limits: &'a Limits) -> Result<Self> {
        let budget = QueryBudget {
            #[cfg(not(target_arch = "wasm32"))]
            deadline: Some(
                std::time::Instant::now()
                    .checked_add(limits.timeout)
                    .ok_or_else(|| limit("Deadline duration exceeds clock range"))?,
            ),
            max_rows: Some(limits.max_rows),
            max_bytes: Some(limits.max_query_bytes),
            ..QueryBudget::default()
        };
        let mut runtime = Self {
            limits,
            queries: 0,
            budget,
            now: Literal::new_simple_literal(""),
            context: None,
        };
        Ok({
            let result = runtime.query(
                &Graph::default(),
                SparqlParser::new()
                    .parse_query("SELECT (NOW() AS ?now) WHERE {}")
                    .map_err(syntax)?,
            )?;
            let Some(Some(Term::Literal(now))) = result.rows.first().and_then(|r| r.first()) else {
                return Err(err(RuleErrorKind::Evaluation, "NOW unavailable"));
            };
            runtime.now = now.clone();
            runtime
        })
    }
    fn check_deadline(&self) -> Result<()> {
        #[cfg(not(target_arch = "wasm32"))]
        if self
            .budget
            .deadline
            .is_some_and(|d| std::time::Instant::now() >= d)
        {
            return Err(limit("Inference deadline exceeded"));
        }
        Ok(())
    }
    fn prepared(&mut self, mut query: Query) -> Result<PreparedQuery> {
        self.queries += 1;
        if self.queries > self.limits.max_queries {
            return Err(limit("Query count budget exceeded"));
        }
        self.check_deadline()?;
        if let Some(context) = &self.context {
            let base = context.base_iri().cloned();
            match &mut query {
                Query::Select { base_iri, .. }
                | Query::Construct { base_iri, .. }
                | Query::Ask { base_iri, .. }
                | Query::Describe { base_iri, .. } => *base_iri = base,
            }
        }
        // The clock probe itself must evaluate NOW rather than the placeholder.
        if self.queries > 1 {
            rule_schedule::clock::freeze(&mut query, &self.now);
        }
        PreparedQuery::from_query_with_versions(query, vec!["1.2".into()])
            .map_err(|e| err(RuleErrorKind::Evaluation, e))
    }
    fn failure(error: sparq_engine::QueryFailure) -> RuleError {
        let kind = match &error {
            sparq_engine::QueryFailure::Budget(_) | sparq_engine::QueryFailure::Capacity(_) => {
                RuleErrorKind::Limit
            }
            sparq_engine::QueryFailure::Evaluation(_) => RuleErrorKind::Evaluation,
        };
        err(kind, error.to_string())
    }
    fn query(&mut self, graph: &Graph, query: Query) -> Result<QueryResult> {
        let prepared = self.prepared(query)?;
        sparq_engine::query_prepared_with_budget_detailed(graph, &prepared, &self.budget)
            .map_err(Self::failure)
    }
    fn rows(&mut self, graph: &Graph, pattern: GraphPattern) -> Result<Rows> {
        let result = self.query(graph, select(pattern))?;
        if result.rows.len() > self.limits.max_rows {
            return Err(limit("Solution row budget exceeded"));
        }
        Ok(Rows {
            vars: result.vars,
            rows: result.rows,
        })
    }
    fn template_work(&self, head: &[TriplePattern], rows: &Rows) -> Result<()> {
        let instances = head
            .len()
            .checked_mul(rows.rows.len())
            .ok_or_else(|| limit("Template instance capacity exceeded"))?;
        if instances > self.limits.max_construct_triples {
            return Err(limit("Template instance budget exceeded"));
        }
        let columns: HashMap<_, _> = rows
            .vars
            .iter()
            .enumerate()
            .map(|(i, v)| (v.as_str(), i))
            .collect();
        let mut bytes = 0_usize;
        for row in &rows.rows {
            self.check_deadline()?;
            for triple in head {
                bytes = bytes
                    .saturating_add(64)
                    .saturating_add(template_term_bytes(&triple.subject, &columns, row))
                    .saturating_add(template_predicate_bytes(&triple.predicate, &columns, row))
                    .saturating_add(template_term_bytes(&triple.object, &columns, row));
                if bytes > self.limits.max_construct_bytes {
                    return Err(limit("Template expansion byte budget exceeded"));
                }
            }
        }
        Ok(())
    }
    fn construct(
        &mut self,
        graph: &Graph,
        head: &[TriplePattern],
        rows: &Rows,
    ) -> Result<Vec<Triple>> {
        // Bound candidate allocation before the engine instantiates a template.
        // This conservative admission can reject work that would later deduplicate.
        self.template_work(head, rows)?;
        let prepared = self.prepared(construct(head, rows.table()))?;
        sparq_engine::construct_prepared_with_budget_detailed(graph, &prepared, &self.budget)
            .map_err(Self::failure)
    }
    fn body(&mut self, graph: &Graph, base: &Graph, ops: &[Op], mut rows: Rows) -> Result<Rows> {
        for op in ops {
            if rows.rows.is_empty() {
                break;
            }
            rows = match op {
                Op::Triples(..) | Op::Filter(..) | Op::Set(..) => {
                    self.rows(graph, simple(std::slice::from_ref(op), rows.table()))?
                }
                Op::Not(base_only, inner) => {
                    let expr = Expression::Not(Box::new(Expression::Exists(Box::new(simple(
                        inner,
                        GraphPattern::default(),
                    )))));
                    self.rows(
                        if *base_only { base } else { graph },
                        filter(expr, rows.table()),
                    )?
                }
                Op::Group {
                    keys,
                    body,
                    aggregates,
                } => {
                    let mut vars = rows.vars.clone();
                    for v in keys.iter().chain(aggregates.iter().map(|(v, _)| v)) {
                        if !vars.contains(v) {
                            vars.push(v.clone());
                        }
                    }
                    let mut output = Rows {
                        vars,
                        rows: Vec::new(),
                    };
                    for row in &rows.rows {
                        let inner = self.body(graph, base, body, rows.keys(row, keys))?;
                        let mut group = GraphPattern::Group {
                            inner: Box::new(inner.table()),
                            variables: keys.clone(),
                            aggregates: aggregates.clone(),
                        };
                        for (target, _) in aggregates {
                            group = filter(Expression::Bound(target.clone()), group);
                        }
                        let grouped = self.rows(graph, group)?;
                        let merged =
                            self.rows(graph, join(rows.one(row).table(), grouped.table()))?;
                        output.append(merged, self.limits.max_rows)?;
                    }
                    output
                }
            };
        }
        Ok(rows)
    }
}

#[cfg(test)]
thread_local! { static WORKING_COPIES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
fn working_copy(base: &Graph) -> Graph {
    #[cfg(test)]
    WORKING_COPIES.with(|c| c.set(c.get() + 1));
    base.fork_scoped(&[])
}
fn publish(
    graph: &mut Graph,
    output: &mut Vec<Triple>,
    output_bytes: &mut usize,
    triples: Vec<Triple>,
    limits: &Limits,
) -> Result<bool> {
    let mut changed = false;
    for triple in triples {
        let subject: Term = triple.subject.clone().into();
        if GraphView::new(graph).contains(&subject, triple.predicate.as_str(), &triple.object) {
            continue;
        }
        if output.len() >= limits.max_inferred_triples {
            return Err(limit("Inferred triple budget exceeded"));
        }
        let bytes = output_bytes.saturating_add(triple.to_string().len());
        if bytes > limits.max_inferred_bytes {
            return Err(limit("Inferred byte budget exceeded"));
        }
        *output_bytes = bytes;
        graph
            .insert_triple(subject, triple.predicate.clone(), triple.object.clone())
            .map_err(|e| err(RuleErrorKind::Evaluation, e))?;
        output.push(triple);
        changed = true;
    }
    Ok(changed)
}

/// Infer only triples absent from the immutable base graph, using default limits.
/// IMPORTS is refused. Preparation/evaluation exhaustion returns an error.
pub fn infer(base: &Graph, source: &str, base_iri: &str) -> Result<Vec<Triple>> {
    infer_with_limits(base, source, base_iri, &Limits::default())
}

/// Infer with explicit operational limits and no successful partial result.
pub fn infer_with_limits(
    base: &Graph,
    source: &str,
    base_iri: &str,
    limits: &Limits,
) -> Result<Vec<Triple>> {
    crate::functions::with_rule_bindings(|| {
        let mut runtime = Runtime::new(limits)?;
        let program = compile(source, base_iri, limits)?;
        reject_imports(&program)?;
        well_formed(&program)?;
        let levels = strata(&program, limits)?;
        runtime.check_deadline()?;
        let mut working = working_copy(base);
        let mut output = Vec::new();
        let mut output_bytes = 0;
        for seed in &program.seeds {
            let triples = runtime.construct(&working, seed, &Rows::unit())?;
            publish(
                &mut working,
                &mut output,
                &mut output_bytes,
                triples,
                limits,
            )?;
        }
        for layer in 0..=levels.iter().copied().max().unwrap_or(0) {
            for (i, rule) in program.rules.iter().enumerate() {
                if levels[i] != layer || !rule.once {
                    continue;
                }
                runtime.context = Some(rule.context.clone());
                let rows = runtime.body(
                    if rule.base_only { base } else { &working },
                    base,
                    &rule.body,
                    Rows::unit(),
                )?;
                let triples = runtime.construct(&working, &rule.head, &rows)?;
                publish(
                    &mut working,
                    &mut output,
                    &mut output_bytes,
                    triples,
                    limits,
                )?;
            }
            let mut failure = None;
            let settled = rule_schedule::saturate(
                limits.max_iterations,
                "SPARQL-RL iteration budget exceeded",
                || {
                    let result = (|| {
                        let mut changed = false;
                        for (i, rule) in program.rules.iter().enumerate() {
                            if levels[i] != layer || rule.once {
                                continue;
                            }
                            runtime.context = Some(rule.context.clone());
                            let rows = runtime.body(
                                if rule.base_only { base } else { &working },
                                base,
                                &rule.body,
                                Rows::unit(),
                            )?;
                            let triples = runtime.construct(&working, &rule.head, &rows)?;
                            changed |= publish(
                                &mut working,
                                &mut output,
                                &mut output_bytes,
                                triples,
                                limits,
                            )?;
                        }
                        Ok(changed)
                    })();
                    result.map_err(|e: RuleError| {
                        let message = e.to_string();
                        failure = Some(e);
                        message
                    })
                },
            );
            if let Err(message) = settled {
                return Err(failure.unwrap_or_else(|| limit(&message)));
            }
        }
        output.sort_by_key(ToString::to_string);
        runtime.check_deadline()?;
        Ok(output)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn one_working_copy_for_many_rules_and_iterations() {
        let base = Graph::load_str(
            "<urn:a> <urn:p> <urn:b>. <urn:b> <urn:p> <urn:c>. <urn:c> <urn:p> <urn:d>.",
            "turtle",
        )
        .unwrap();
        let source = "RULE {?a <urn:q> ?b} WHERE {?a <urn:p> ?b} RULE {?a <urn:q> ?c} WHERE {?a <urn:q> ?b. ?b <urn:p> ?c} RULE {?a <urn:r> ?b} WHERE {?a <urn:q> ?b}";
        for repeats in [1, 4, 12] {
            let source = source.repeat(repeats);
            let before = WORKING_COPIES.with(std::cell::Cell::get);
            let out = infer(&base, &source, "urn:rules:").unwrap();
            assert_eq!(out.len(), 12);
            assert!(WORKING_COPIES.with(std::cell::Cell::get) - before <= 1);
            assert_eq!(base.len(), 3);
        }
    }
    #[test]
    fn query_error_categories_use_typed_causes() {
        use sparq_engine::{BudgetExceeded, QueryFailure};
        assert_eq!(
            Runtime::failure(QueryFailure::Budget(BudgetExceeded::Rows)).kind,
            RuleErrorKind::Limit
        );
        assert_eq!(
            Runtime::failure(QueryFailure::Evaluation(
                "query budget exceeded (max-rows)".into()
            ))
            .kind,
            RuleErrorKind::Evaluation
        );
    }
}
