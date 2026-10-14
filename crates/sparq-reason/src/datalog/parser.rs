//! sq-6tykl.3 — hand-rolled recursive-descent parser for the native
//! stratified-Datalog fragment (see the module docs in [`super`] for the grammar and
//! the design record `research/stratified-datalog-rules.md` §2 for the dialect
//! decision). Constants are interned into the caller's [`Dict`] as they are parsed;
//! every out-of-fragment construct is a loud error naming the construct.

use super::{AggAtom, AggFunc, Atom, CmpOp, DTerm, Filter, Program, Rule};
use rustc_hash::{FxHashMap, FxHashSet};
use sparq_core::dict::Dict;

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const XSD_INTEGER: &str = "http://www.w3.org/2001/XMLSchema#integer";
const XSD_STRING: &str = "http://www.w3.org/2001/XMLSchema#string";

/// Parse `src` into a validated [`Program`], interning ground terms into `dict`.
pub(super) fn parse(dict: &mut Dict, src: &str) -> Result<Program, String> {
    let mut p = Parser {
        dict,
        src: src.as_bytes(),
        pos: 0,
        prefixes: FxHashMap::default(),
        rules: Vec::new(),
    };
    p.program()?;
    Ok(Program { rules: p.rules })
}

/// Rule-scope variable table: name → slot, in first-seen order.
#[derive(Default)]
struct Scope {
    slots: FxHashMap<String, u32>,
    names: Vec<String>,
}

impl Scope {
    fn slot(&mut self, name: &str) -> u32 {
        if let Some(&s) = self.slots.get(name) {
            return s;
        }
        let s = self.slots.len() as u32;
        self.slots.insert(name.to_string(), s);
        self.names.push(name.to_string());
        s
    }
}

struct Parser<'a> {
    dict: &'a mut Dict,
    src: &'a [u8],
    pos: usize,
    prefixes: FxHashMap<String, String>,
    rules: Vec<Rule>,
}

impl Parser<'_> {
    // ---- lexical helpers -------------------------------------------------

    fn skip_ws(&mut self) {
        loop {
            while self.pos < self.src.len() && self.src[self.pos].is_ascii_whitespace() {
                self.pos += 1;
            }
            if self.pos < self.src.len() && self.src[self.pos] == b'#' {
                while self.pos < self.src.len() && self.src[self.pos] != b'\n' {
                    self.pos += 1;
                }
            } else {
                return;
            }
        }
    }

    fn line(&self) -> usize {
        1 + self.src[..self.pos].iter().filter(|&&c| c == b'\n').count()
    }

    fn err(&self, msg: &str) -> String {
        format!("datalog parse error (line {}): {msg}", self.line())
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn eat(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, c: u8) -> Result<(), String> {
        if self.eat(c) {
            Ok(())
        } else {
            Err(self.err(&format!("expected `{}`", c as char)))
        }
    }

    /// Consume `word` if it is the next token (ASCII, followed by a non-word byte).
    fn eat_word(&mut self, word: &str) -> bool {
        let w = word.as_bytes();
        if self.src[self.pos..].starts_with(w) {
            let after = self.src.get(self.pos + w.len());
            if after.is_none_or(|c| !c.is_ascii_alphanumeric() && *c != b'_') {
                self.pos += w.len();
                return true;
            }
        }
        false
    }

    fn name(&mut self) -> Result<String, String> {
        let start = self.pos;
        while self
            .peek()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-' || c == b'.')
        {
            self.pos += 1;
        }
        // A trailing `.` is the statement terminator, not part of the name.
        let mut end = self.pos;
        while end > start && self.src[end - 1] == b'.' {
            end -= 1;
        }
        self.pos = end;
        if end == start {
            return Err(self.err("expected a name"));
        }
        Ok(String::from_utf8_lossy(&self.src[start..end]).into_owned())
    }

    // ---- grammar ---------------------------------------------------------

    fn program(&mut self) -> Result<(), String> {
        loop {
            self.skip_ws();
            if self.pos >= self.src.len() {
                return Ok(());
            }
            if self.eat_word("@prefix") {
                self.prefix_decl()?;
            } else {
                self.rule()?;
            }
        }
    }

    fn prefix_decl(&mut self) -> Result<(), String> {
        self.skip_ws();
        let name = if self.peek() == Some(b':') {
            String::new()
        } else {
            self.name()?
        };
        self.expect(b':')?;
        self.skip_ws();
        let iri = self.iri_ref()?;
        self.skip_ws();
        self.expect(b'.')?;
        self.prefixes.insert(name, iri);
        Ok(())
    }

    fn iri_ref(&mut self) -> Result<String, String> {
        self.expect(b'<')?;
        let start = self.pos;
        while self.peek().is_some_and(|c| c != b'>') {
            self.pos += 1;
        }
        let iri = String::from_utf8_lossy(&self.src[start..self.pos]).into_owned();
        self.expect(b'>')?;
        Ok(iri)
    }

    fn rule(&mut self) -> Result<(), String> {
        let mut scope = Scope::default();
        // Head: one or more atoms, comma-separated.
        let mut head = vec![self.atom(&mut scope, "head")?];
        loop {
            self.skip_ws();
            if self.eat(b',') {
                head.push(self.atom(&mut scope, "head")?);
            } else {
                break;
            }
        }
        self.skip_ws();
        if !(self.eat(b':') && self.eat(b'-')) {
            return Err(self.err("expected `:-` after the rule head"));
        }
        let mut positive = Vec::new();
        let mut negated = Vec::new();
        let mut aggregates: Vec<(AggAtom, Vec<String>)> = Vec::new();
        let mut filters = Vec::new();
        loop {
            self.skip_ws();
            if self.eat_word("NOT") {
                self.skip_ws();
                // `NOT EXISTS { ... }` is an accepted spelling of the
                // same grouped existential absence test as `NOT { ... }`.
                self.eat_word("EXISTS");
                negated.push(self.negated_group(&mut scope)?);
            } else if self.eat_word("AGGREGATE") {
                aggregates.push(self.aggregate(&mut scope)?);
            } else if self.eat_word("FILTER") {
                filters.push(self.filter(&mut scope)?);
            } else {
                positive.push(self.atom(&mut scope, "body")?);
            }
            self.skip_ws();
            if self.eat(b',') {
                continue;
            }
            self.expect(b'.')?;
            break;
        }
        let rule = Rule {
            head,
            positive,
            aggregates: aggregates.iter().map(|(a, _)| a.clone()).collect(),
            negated,
            filters,
            n_slots: scope.slots.len(),
        };
        self.validate(&rule, &scope, &aggregates)?;
        self.rules.push(rule);
        Ok(())
    }

    /// `NOT atom`, `NOT { atom (,|.) atom }`, or `NOT EXISTS { ... }` after
    /// the caller has consumed the leading keyword(s).
    fn negated_group(&mut self, scope: &mut Scope) -> Result<Vec<Atom>, String> {
        self.skip_ws();
        if !self.eat(b'{') {
            return Ok(vec![self.atom(scope, "NOT")?]);
        }
        let mut atoms = Vec::new();
        loop {
            self.skip_ws();
            if self.eat(b'}') {
                if atoms.is_empty() {
                    return Err(self.err("NOT group requires at least one atom"));
                }
                return Ok(atoms);
            }
            atoms.push(self.atom(scope, "NOT group")?);
            self.skip_ws();
            if self.eat(b',') || self.eat(b'.') {
                continue;
            }
            if self.peek() != Some(b'}') {
                return Err(self.err("expected `,`, `.`, or `}` in NOT group"));
            }
        }
    }

    /// `AGGREGATE( atom (, atom)* [ON ?g (,? ?g)*] BIND FUNC(?v) AS ?out )`
    fn aggregate(&mut self, outer: &mut Scope) -> Result<(AggAtom, Vec<String>), String> {
        self.skip_ws();
        self.expect(b'(')?;
        let mut local = Scope::default();
        let mut body = vec![self.atom(&mut local, "AGGREGATE body")?];
        loop {
            self.skip_ws();
            if self.eat(b',') {
                body.push(self.atom(&mut local, "AGGREGATE body")?);
            } else {
                break;
            }
        }
        self.skip_ws();
        let mut on = Vec::new();
        if self.eat_word("ON") {
            loop {
                self.skip_ws();
                if self.peek() != Some(b'?') {
                    break;
                }
                self.pos += 1;
                let n = self.name()?;
                let Some(&ls) = local.slots.get(&n) else {
                    return Err(self.err(&format!(
                        "AGGREGATE `ON ?{n}`: the variable does not occur in the aggregate body"
                    )));
                };
                on.push((ls, outer.slot(&n)));
                self.skip_ws();
                self.eat(b','); // optional separator
            }
            if on.is_empty() {
                return Err(self.err("AGGREGATE `ON` requires at least one variable"));
            }
        }
        self.skip_ws();
        if !self.eat_word("BIND") {
            return Err(self.err("expected `BIND` in AGGREGATE"));
        }
        self.skip_ws();
        let func_name = self.name()?;
        let func = match func_name.to_ascii_uppercase().as_str() {
            "COUNT" => AggFunc::Count,
            "SUM" => AggFunc::Sum,
            "MIN" => AggFunc::Min,
            "MAX" => AggFunc::Max,
            "AVG" => AggFunc::Avg,
            _ => return Err(self.err(&format!("unknown AGGREGATE function `{func_name}`"))),
        };
        self.skip_ws();
        self.expect(b'(')?;
        self.skip_ws();
        let distinct = self.eat_word("DISTINCT");
        if distinct && func != AggFunc::Count {
            return Err(self.err("DISTINCT is currently supported only by COUNT"));
        }
        self.skip_ws();
        if !self.eat(b'?') {
            return Err(self.err("expected a `?variable` as the AGGREGATE function argument"));
        }
        let cname = self.name()?;
        let Some(&value_slot) = local.slots.get(&cname) else {
            return Err(self.err(&format!(
                "AGGREGATE `{func_name}(?{cname})`: the variable does not occur in the aggregate body"
            )));
        };
        let value = (func != AggFunc::Count || distinct).then_some(value_slot);
        self.skip_ws();
        self.expect(b')')?;
        self.skip_ws();
        if !self.eat_word("AS") {
            return Err(self.err("expected `AS` after the AGGREGATE function"));
        }
        self.skip_ws();
        if !self.eat(b'?') {
            return Err(self.err("expected a `?variable` after `AS`"));
        }
        let oname = self.name()?;
        let out = outer.slot(&oname);
        self.skip_ws();
        self.expect(b')')?;
        // Local names that are NOT `ON` variables must not collide with outer-scope
        // names (silent capture would be ambiguous) — checked in `validate` once the
        // whole rule scope is known; record them here.
        let on_locals: FxHashSet<u32> = on.iter().map(|&(l, _)| l).collect();
        let locals: Vec<String> = local
            .names
            .iter()
            .enumerate()
            .filter(|(i, _)| !on_locals.contains(&(*i as u32)))
            .map(|(_, n)| n.clone())
            .collect();
        let n_slots = local.slots.len();
        Ok((
            AggAtom {
                func,
                distinct,
                value,
                body,
                on,
                out,
                n_slots,
            },
            locals,
        ))
    }

    /// `FILTER( operand op operand )`
    fn filter(&mut self, scope: &mut Scope) -> Result<Filter, String> {
        self.skip_ws();
        self.expect(b'(')?;
        let a = self.operand(scope)?;
        self.skip_ws();
        let op = self.cmp_op()?;
        let b = self.operand(scope)?;
        self.skip_ws();
        self.expect(b')')?;
        Ok(Filter { a, op, b })
    }

    fn cmp_op(&mut self) -> Result<CmpOp, String> {
        let two = |p: &mut Self| {
            p.pos += 2;
        };
        match (self.peek(), self.src.get(self.pos + 1).copied()) {
            (Some(b'>'), Some(b'=')) => {
                two(self);
                Ok(CmpOp::Ge)
            }
            (Some(b'<'), Some(b'=')) => {
                two(self);
                Ok(CmpOp::Le)
            }
            (Some(b'!'), Some(b'=')) => {
                two(self);
                Ok(CmpOp::Ne)
            }
            (Some(b'>'), _) => {
                self.pos += 1;
                Ok(CmpOp::Gt)
            }
            (Some(b'<'), _) => {
                self.pos += 1;
                Ok(CmpOp::Lt)
            }
            (Some(b'='), _) => {
                self.pos += 1;
                Ok(CmpOp::Eq)
            }
            _ => Err(self.err("expected a comparison operator (= != < <= > >=)")),
        }
    }

    /// A FILTER operand: a variable or a numeric constant.
    fn operand(&mut self, scope: &mut Scope) -> Result<DTerm, String> {
        self.skip_ws();
        if self.eat(b'?') {
            let n = self.name()?;
            return Ok(DTerm::Var(scope.slot(&n)));
        }
        match self.term(scope, "FILTER")? {
            DTerm::Const(id) => Ok(DTerm::Const(id)),
            DTerm::Var(_) => unreachable!("variables are consumed above"),
        }
    }

    /// `[ term , term , term ]`.
    fn atom(&mut self, scope: &mut Scope, ctx: &str) -> Result<Atom, String> {
        self.skip_ws();
        self.expect(b'[')?;
        let s = self.term(scope, ctx)?;
        self.skip_ws();
        self.expect(b',')?;
        let p = self.term(scope, ctx)?;
        self.skip_ws();
        self.expect(b',')?;
        let o = self.term(scope, ctx)?;
        self.skip_ws();
        self.expect(b']')?;
        let pred = match p {
            DTerm::Const(id) => Some(id),
            DTerm::Var(_) => None,
        };
        Ok(Atom { t: [s, p, o], pred })
    }

    fn term(&mut self, scope: &mut Scope, ctx: &str) -> Result<DTerm, String> {
        self.skip_ws();
        match self.peek() {
            Some(b'?') => {
                self.pos += 1;
                let n = self.name()?;
                Ok(DTerm::Var(scope.slot(&n)))
            }
            Some(b'<') => {
                let iri = self.iri_ref()?;
                Ok(DTerm::Const(self.dict.intern_iri(&iri)))
            }
            Some(b'"') => {
                self.pos += 1;
                let start = self.pos;
                while self.peek().is_some_and(|c| c != b'"') {
                    self.pos += 1;
                }
                let val = String::from_utf8_lossy(&self.src[start..self.pos]).into_owned();
                self.expect(b'"')?;
                let dt = if self.eat(b'^') {
                    self.expect(b'^')?;
                    if self.peek() == Some(b'<') {
                        self.iri_ref()?
                    } else {
                        self.pname()?
                    }
                } else {
                    XSD_STRING.to_string()
                };
                Ok(DTerm::Const(self.dict.intern_lit(&val, &dt, None)))
            }
            Some(c) if c.is_ascii_digit() || c == b'-' || c == b'+' => {
                let start = self.pos;
                self.pos += 1;
                while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                    self.pos += 1;
                }
                let lex = String::from_utf8_lossy(&self.src[start..self.pos]).into_owned();
                Ok(DTerm::Const(self.dict.intern_lit(&lex, XSD_INTEGER, None)))
            }
            Some(b'a')
                if self
                    .src
                    .get(self.pos + 1)
                    .is_none_or(|c| !c.is_ascii_alphanumeric() && *c != b'_' && *c != b':') =>
            {
                self.pos += 1;
                Ok(DTerm::Const(self.dict.intern_iri(RDF_TYPE)))
            }
            Some(c) if c.is_ascii_alphabetic() || c == b':' => {
                let iri = self.pname()?;
                Ok(DTerm::Const(self.dict.intern_iri(&iri)))
            }
            _ => Err(self.err(&format!("expected a term ({ctx} atom)"))),
        }
    }

    /// A prefixed name `prefix:local`, expanded against `@prefix` declarations.
    fn pname(&mut self) -> Result<String, String> {
        let pfx = if self.peek() == Some(b':') {
            String::new()
        } else {
            self.name()?
        };
        self.expect(b':')?;
        let local = if self
            .peek()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_')
        {
            self.name()?
        } else {
            String::new()
        };
        let Some(base) = self.prefixes.get(&pfx) else {
            return Err(self.err(&format!("undeclared prefix `{pfx}:`")));
        };
        Ok(format!("{base}{local}"))
    }

    // ---- validation (safety / range restriction) --------------------------

    fn validate(
        &self,
        rule: &Rule,
        scope: &Scope,
        aggs: &[(AggAtom, Vec<String>)],
    ) -> Result<(), String> {
        let bound = super::bound_slots(rule);
        let name_of = |slot: u32| scope.names[slot as usize].clone();
        for a in &rule.head {
            for t in &a.t {
                if let DTerm::Var(v) = t {
                    if !bound.contains(v) {
                        return Err(self.err(&format!(
                            "unsafe rule: head variable `?{}` is not bound by a positive body \
                             atom or an AGGREGATE",
                            name_of(*v)
                        )));
                    }
                }
            }
        }
        for f in &rule.filters {
            for t in [&f.a, &f.b] {
                if let DTerm::Var(v) = t {
                    if !bound.contains(v) {
                        return Err(self.err(&format!(
                            "FILTER variable `?{}` is not bound by a positive body atom or an \
                             AGGREGATE",
                            name_of(*v)
                        )));
                    }
                }
            }
        }
        for (agg, locals) in aggs {
            // The `AS` output must be fresh w.r.t. the positive atoms (joining ON a
            // count value elsewhere would silently change meaning — loud instead).
            for a in &rule.positive {
                for t in &a.t {
                    if *t == DTerm::Var(agg.out) {
                        return Err(self.err(&format!(
                            "AGGREGATE output `?{}` also occurs in a positive body atom — the \
                             output variable must be fresh",
                            name_of(agg.out)
                        )));
                    }
                }
            }
            // Aggregate-local variables (non-ON) must not shadow outer names.
            for n in locals {
                if scope.slots.contains_key(n) {
                    return Err(self.err(&format!(
                        "aggregate-local variable `?{n}` also occurs outside the AGGREGATE — \
                         name it apart or list it under ON"
                    )));
                }
            }
        }
        let outs: Vec<u32> = rule.aggregates.iter().map(|a| a.out).collect();
        for (i, o) in outs.iter().enumerate() {
            if outs[..i].contains(o) {
                return Err(self.err(&format!(
                    "two AGGREGATE atoms bind the same output variable `?{}`",
                    name_of(*o)
                )));
            }
        }
        Ok(())
    }
}
