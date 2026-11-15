//! Exact bounded table evaluation over supplied term cells.
//!
//! The credential adapter authenticates complete inputs before this reference
//! evaluator runs; the generated program checks them independently.

#[path = "interp_closed.rs"]
mod closed;

use crate::{plan::*, Error};
use oxrdf::Term;
use std::collections::BTreeSet;

/// Canonical field word, big endian. The suite adapter owns its derivation.
pub type Word = [u8; 32];

/// The term-cell contract, including the credential scope of a blank node.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cell {
    pub bound: bool,
    pub kind: u8,
    /// Authenticated key-independent RDF identity, or a result-local blank label.
    pub enc: Word,
    pub key_class: u8,
    pub key: Word,
    pub dt: Word,
    pub lang: Word,
    pub scope: u32,
}

impl Cell {
    pub fn same_term(self, other: Self) -> bool {
        self.bound == other.bound
            && (!self.bound
                || (self.kind == other.kind && self.enc == other.enc && self.scope == other.scope))
    }
    fn identity(self) -> (bool, Word, u8, u32) {
        (self.bound, self.enc, self.kind, self.scope)
    }
    pub fn validate(self) -> Result<(), Error> {
        if !self.bound {
            if self != Self::default() {
                return Err(Error::Invalid("noncanonical unbound cell".into()));
            }
        } else if !(1..=3).contains(&self.kind) || (self.kind != 2 && self.scope != 0) {
            return Err(Error::Invalid("term kind or blank-node scope".into()));
        }
        // Canonical BN254 words, never host-side modular reduction.
        const MODULUS: Word = [
            0x30, 0x64, 0x4e, 0x72, 0xe1, 0x31, 0xa0, 0x29, 0xb8, 0x50, 0x45, 0xb6, 0x81, 0x81,
            0x58, 0x5d, 0x28, 0x33, 0xe8, 0x48, 0x79, 0xb9, 0x70, 0x91, 0x43, 0xe1, 0xf5, 0x93,
            0xf0, 0x00, 0x00, 0x01,
        ];
        if [self.enc, self.key, self.dt, self.lang]
            .iter()
            .any(|w| w >= &MODULUS)
        {
            return Err(Error::Invalid("noncanonical field word".into()));
        }
        Ok(())
    }
}

/// The suite supplies encodings for request constants; it never selects plan structure.
pub trait EncodeTerm {
    fn encode(&self, term: &Term) -> Result<Cell, Error>;
}

pub type Row = Vec<Cell>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    pub rows: Vec<Row>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResultValue {
    Select {
        variables: Vec<String>,
        rows: Vec<Row>,
    },
    Ask(bool),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trace {
    pub tables: Vec<Table>,
    pub result: ResultValue,
}

/// A complete credential's cells, supplied in full to the opening contract.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CredentialCells {
    pub quads: Vec<[Cell; 4]>,
}

/// Complete default and named views. Counts refer to signed input quads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dataset {
    pub(crate) quads: Vec<[Cell; 4]>,
    pub(crate) counts: Vec<usize>,
}
impl Dataset {
    pub fn quads(&self) -> &[[Cell; 4]] {
        &self.quads
    }
    pub fn credential_counts(&self) -> &[usize] {
        &self.counts
    }
}

pub fn evaluate(
    plan: &Plan,
    credentials: &[CredentialCells],
    encoder: &impl EncodeTerm,
) -> Result<Trace, Error> {
    plan.bounds.check_credentials(
        &credentials
            .iter()
            .map(|c| c.quads.len())
            .collect::<Vec<_>>(),
    )?;
    if credentials
        .iter()
        .zip(plan.credential_quad_bounds())
        .any(|(c, b)| c.quads.len() > b as usize)
    {
        return Err(Error::Capacity("per-credential quad count".into()));
    }
    let counts = credentials.iter().map(|c| c.quads.len()).collect();
    let mut dataset = vec![];
    let mut seen = BTreeSet::new();
    for (credential_index, credential) in credentials.iter().enumerate() {
        for quad in &credential.quads {
            for cell in quad {
                cell.validate()?;
            }
            if quad[..3].iter().any(|c| !c.bound) || quad[0].kind == 3 || quad[1].kind != 1 {
                return Err(Error::Invalid("RDF triple positions".into()));
            }
            if quad[3].bound {
                return Err(Error::Unsupported("credential named graph".into()));
            }
            for c in &quad[..3] {
                if c.kind == 2 && c.scope != credential_index as u32 + 1 {
                    return Err(Error::Invalid("credential blank-node scope".into()));
                }
            }
            let key = std::array::from_fn::<_, 3, _>(|i| quad[i].identity());
            if seen.insert(key) {
                dataset.push(*quad);
            }
        }
    }
    evaluate_dataset(
        plan,
        &Dataset {
            quads: dataset,
            counts,
        },
        encoder,
    )
}

/// Evaluate the complete method dataset; ordinary scans use its default merge.
pub fn evaluate_dataset(
    plan: &Plan,
    input_dataset: &Dataset,
    encoder: &impl EncodeTerm,
) -> Result<Trace, Error> {
    plan.bounds.check_credentials(&input_dataset.counts)?;
    if input_dataset
        .counts
        .iter()
        .zip(plan.credential_quad_bounds())
        .any(|(n, b)| *n > b as usize)
    {
        return Err(Error::Capacity("per-credential quad count".into()));
    }
    let params: Vec<Option<Cell>> = plan
        .parameters
        .iter()
        .map(|p| match p {
            Parameter::Term(t) => encoder.encode(t).and_then(|c| {
                c.validate()?;
                if !c.bound || c.scope != 0 {
                    return Err(Error::Invalid("public constant".into()));
                }
                Ok(Some(c))
            }),
            Parameter::Index(_) => Ok(None),
        })
        .collect::<Result<_, _>>()?;
    let mut dataset = vec![];
    let mut seen = BTreeSet::new();
    for quad in &input_dataset.quads {
        for cell in quad {
            cell.validate()?;
        }
        if quad[..3].iter().any(|c| !c.bound) || quad[0].kind == 3 || quad[1].kind != 1 {
            return Err(Error::Invalid("RDF triple positions".into()));
        }
        let key = std::array::from_fn::<_, 3, _>(|i| quad[i].identity());
        if seen.insert(key) {
            dataset.push(*quad);
        }
    }
    let mut tables: Vec<Table> = vec![];
    for node in &plan.nodes {
        let input = |id: NodeId| &tables[id as usize].rows;
        let schema = |id: NodeId| &plan.nodes[id as usize].output.variables;
        let mut rows = match &node.operator {
            Operator::Closed(crate::plan_closed::ClosedOperator::Scan {
                pattern,
                context,
                credential: None,
            }) if !crate::credential_graphs::is_profile(plan) => scan_rows(
                pattern,
                &node.output.variables,
                &dataset,
                &params,
                Some((schema(*context), input(*context).first().map(Vec::as_slice))),
            ),
            Operator::Closed(op) => {
                closed::evaluate(op, node, plan, &tables, &dataset, input_dataset, &params)?
            }
            Operator::Unit => vec![vec![]],
            Operator::Scan(pattern) => {
                scan_rows(pattern, &node.output.variables, &dataset, &params, None)
            }
            Operator::Join(a, b) => {
                let mut rows = vec![];
                for left in input(*a) {
                    for right in input(*b) {
                        let mut row = vec![];
                        let mut compatible = true;
                        for v in &node.output.variables {
                            let l = lookup(schema(*a), left, *v);
                            let r = lookup(schema(*b), right, *v);
                            compatible &= !l.bound || !r.bound || l.same_term(r);
                            row.push(if l.bound { l } else { r });
                        }
                        if compatible {
                            rows.push(row);
                        }
                        if rows.len() > plan.bounds.rows_per_table as usize {
                            return Err(Error::Capacity("join output".into()));
                        }
                    }
                }
                rows
            }
            Operator::Filter(a, e) => {
                let mut rows = vec![];
                for row in input(*a) {
                    let (value, error) =
                        eval(e, schema(*a), row, &params, &plan.parameters)?.ebv()?;
                    if value && !error {
                        rows.push(row.clone());
                    }
                }
                rows
            }
            Operator::Project(a) => input(*a)
                .iter()
                .map(|r| {
                    node.output
                        .variables
                        .iter()
                        .map(|v| lookup(schema(*a), r, *v))
                        .collect()
                })
                .collect(),
            Operator::Distinct(a) => {
                let mut seen = BTreeSet::new();
                input(*a)
                    .iter()
                    .filter(|r| seen.insert(row_key(r)))
                    .cloned()
                    .collect()
            }
            Operator::Slice {
                input: a,
                offset,
                limit,
            } => {
                let index = |p: ParameterId| match plan.parameters[p as usize] {
                    Parameter::Index(n) => n as usize,
                    _ => unreachable!("index parameter"),
                };
                input(*a)
                    .iter()
                    .skip(index(*offset))
                    .take(limit.map(index).unwrap_or(usize::MAX))
                    .cloned()
                    .collect()
            }
            Operator::Values(rows) => rows
                .iter()
                .map(|r| {
                    r.iter()
                        .map(|p| {
                            p.map_or(Cell::default(), |p| {
                                params[p as usize].expect("term parameter")
                            })
                        })
                        .collect()
                })
                .collect(),
        };
        if rows.len() > plan.bounds.rows_per_table as usize {
            return Err(Error::Capacity("operator output".into()));
        }
        crate::plan_closed::check_row_bound(node, rows.len())?;
        rows.sort_by_key(|r| row_key(r));
        tables.push(Table { rows });
    }
    let output = tables[plan.result as usize].rows.clone();
    let result = match plan.form {
        ResultForm::Ask => ResultValue::Ask(!output.is_empty()),
        ResultForm::Select => ResultValue::Select {
            variables: plan
                .result_schema()
                .variables
                .iter()
                .map(|&v| plan.variable_names[v as usize].clone())
                .collect(),
            rows: release_rows(output),
        },
    };
    Ok(Trace { tables, result })
}

fn scan_rows(
    pattern: &[Slot; 3],
    variables: &[Var],
    dataset: &[[Cell; 4]],
    parameters: &[Option<Cell>],
    context: Option<(&[Var], Option<&[Cell]>)>,
) -> Vec<Row> {
    if context.is_some_and(|(_, row)| row.is_none()) {
        return vec![];
    }
    let mut rows = vec![];
    for quad in dataset {
        let mut row = vec![Cell::default(); variables.len()];
        let mut matches = true;
        for (i, slot) in pattern.iter().enumerate() {
            match slot {
                Slot::Parameter(p) => {
                    matches &= quad[i].same_term(parameters[*p as usize].expect("term parameter"));
                }
                Slot::Variable(v) => {
                    let captured = context
                        .and_then(|(schema, row)| row.map(|row| lookup(schema, row, *v)))
                        .unwrap_or_default();
                    if captured.bound {
                        matches &= quad[i].same_term(captured);
                    } else {
                        let col = variables.iter().position(|x| x == v).expect("scan schema");
                        if row[col].bound {
                            matches &= row[col].same_term(quad[i]);
                        } else {
                            row[col] = quad[i];
                        }
                    }
                }
            }
        }
        if matches {
            rows.push(row);
        }
    }
    rows
}

// Result blank nodes have presentation-local identities, independent of the
// credential-local identities used by joins in private operator tables.
fn release_rows(mut rows: Vec<Row>) -> Vec<Row> {
    let mut labels = std::collections::BTreeMap::new();
    for row in &mut rows {
        for c in row {
            if c.bound && c.kind == 2 {
                let next = labels.len() as u32 + 1;
                let label = *labels.entry((c.enc, c.scope)).or_insert(next);
                let mut enc = [0; 32];
                enc[28..].copy_from_slice(&label.to_be_bytes());
                *c = Cell {
                    bound: true,
                    kind: 2,
                    enc,
                    ..Cell::default()
                };
            } else {
                *c = released_cell(*c);
            }
        }
    }
    rows.sort_by_key(|r| row_key(r));
    rows
}

// A released RDF term carries identity and RDF metadata, with no value-key claim.
pub(crate) fn released_cell(mut cell: Cell) -> Cell {
    cell.key_class = 0;
    cell.key = [0; 32];
    cell
}

pub(crate) fn row_key(row: &[Cell]) -> Vec<(bool, Word, u8, u32)> {
    row.iter().map(|c| c.identity()).collect()
}
fn lookup(schema: &[Var], row: &[Cell], v: Var) -> Cell {
    schema
        .iter()
        .position(|&a| a == v)
        .map_or(Cell::default(), |i| row[i])
}

#[derive(Clone, Copy)]
enum Value {
    Term(Cell),
    Boolean(bool, bool),
}
impl Value {
    fn ebv(self) -> Result<(bool, bool), Error> {
        Ok(match self {
            Self::Boolean(v, e) => (v, e),
            Self::Term(c) if !c.bound || c.kind != 3 => (false, true),
            Self::Term(c) => match c.key_class {
                0 if c.dt == *LANGUAGE_STRING_DATATYPE => {
                    return Err(Error::Unsupported(
                        "language-tagged effective boolean value".into(),
                    ));
                }
                0 if NUMERIC_BOOLEAN_DATATYPES.contains(&c.dt) => (false, false),
                1 => {
                    check_exact_magnitude(c)?;
                    let mut zero = [0; 32];
                    zero[2] = 0x80;
                    let mut full_zero = zero;
                    full_zero[1] = 1;
                    (c.key != full_zero, false)
                }
                5 | 6 => {
                    let mut zero = [0; 32];
                    zero[1] = c.key_class;
                    (c.key != zero, false)
                }
                7 => (true, false),
                2 => {
                    return Err(Error::Unsupported(
                        "floating-point effective boolean value".into(),
                    ))
                }
                0x7f => return Err(Error::Capacity("literal value range".into())),
                _ => (false, true),
            },
        })
    }
}

// Lane D's exact numeric bucket applies equally to the host and circuit.
fn check_exact_magnitude(c: Cell) -> Result<(), Error> {
    let mut zero = [0; 32];
    zero[1] = 1;
    zero[2] = 0x80;
    let a = crate::statement::field(c.key)?;
    let b = crate::statement::field(zero)?;
    let magnitude = if c.key < zero { b - a } else { a - b };
    let bytes = sparq_zk::field::field_to_be_bytes_32(&magnitude);
    if bytes[..18].iter().any(|b| *b != 0) {
        return Err(Error::Capacity("exact numeric expression range".into()));
    }
    Ok(())
}

fn datatype_id(iri: &[u8]) -> Word {
    sparq_merkle_suite::poseidon::field_bytes(sparq_merkle_suite::poseidon::hash_bytes(
        sparq_merkle_suite::poseidon::DATATYPE_DOMAIN,
        iri,
    ))
}
static STRING_DATATYPE: std::sync::LazyLock<Word> =
    std::sync::LazyLock::new(|| datatype_id(b"http://www.w3.org/2001/XMLSchema#string"));
static LANGUAGE_STRING_DATATYPE: std::sync::LazyLock<Word> = std::sync::LazyLock::new(|| {
    datatype_id(b"http://www.w3.org/1999/02/22-rdf-syntax-ns#langString")
});
fn plain_string(c: Cell) -> bool {
    c.lang == [0; 32] || c.dt == *STRING_DATATYPE
}
static NUMERIC_BOOLEAN_DATATYPES: std::sync::LazyLock<Vec<Word>> = std::sync::LazyLock::new(|| {
    [
        "integer",
        "decimal",
        "double",
        "float",
        "boolean",
        "nonPositiveInteger",
        "negativeInteger",
        "long",
        "int",
        "short",
        "byte",
        "nonNegativeInteger",
        "unsignedLong",
        "unsignedInt",
        "unsignedShort",
        "unsignedByte",
        "positiveInteger",
    ]
    .iter()
    .map(|name| datatype_id(format!("http://www.w3.org/2001/XMLSchema#{name}").as_bytes()))
    .collect()
});
fn string_cell(cell: Cell) -> bool {
    cell.kind == 3 && matches!(cell.key_class, 6 | 7)
}

fn compare(op: Compare, a: Value, b: Value) -> Result<Value, Error> {
    let (mut a, mut b) = match (a, b) {
        (Value::Term(a), Value::Term(b)) => (a, b),
        (Value::Boolean(a, ae), Value::Boolean(b, be)) => {
            return Ok(Value::Boolean(
                match op {
                    Compare::Equal => a == b,
                    Compare::Less => !a && b,
                    Compare::LessOrEqual => !a || b,
                    Compare::Greater => a && !b,
                    Compare::GreaterOrEqual => a || !b,
                },
                ae || be,
            ))
        }
        _ => {
            return Err(Error::Unsupported(
                "comparison of boolean expression and term".into(),
            ))
        }
    };
    if !a.bound || !b.bound {
        return Ok(Value::Boolean(false, true));
    }
    if op == Compare::Equal
        && [a, b]
            .iter()
            .all(|c| c.kind != 3 || c.dt == *STRING_DATATYPE)
    {
        return Ok(Value::Boolean(a.same_term(b), false));
    }
    if op == Compare::Equal {
        for c in [&mut a, &mut b] {
            if c.kind == 3 && c.dt == *STRING_DATATYPE {
                c.key_class = 6;
                c.key = [0; 32];
                c.key[1] = 6;
            }
        }
    }
    for c in [a, b] {
        if c.kind == 3 && c.key_class == 1 {
            check_exact_magnitude(c)?;
        }
    }
    if a.key_class == 0x7f || b.key_class == 0x7f {
        return Err(Error::Capacity("literal comparison range".into()));
    }
    if a.kind == 3 && b.kind == 3 && (a.key_class == 2 || b.key_class == 2) {
        return Err(Error::Unsupported("floating-point comparison".into()));
    }
    if a.kind == 3 && b.kind == 3 {
        if matches!(a.key_class, 3 | 4 | 8) && matches!(b.key_class, 3 | 4 | 8) && a.dt != b.dt {
            return Err(Error::Unsupported("mixed temporal datatypes".into()));
        }
        if op != Compare::Equal
            && string_cell(a)
            && string_cell(b)
            && ((!plain_string(a)) || (!plain_string(b)))
        {
            return Err(Error::Unsupported("language-tagged string ordering".into()));
        }
        if matches!(a.key_class, 3 | 4 | 8)
            && matches!(b.key_class, 3 | 4 | 8)
            && a.key_class != b.key_class
        {
            return Err(Error::Unsupported("mixed timezone comparison".into()));
        }
        if op == Compare::Equal && string_cell(a) && string_cell(b) {
            return Ok(Value::Boolean(a.same_term(b), false));
        }
        if op == Compare::Equal
            && (a.dt == *LANGUAGE_STRING_DATATYPE || b.dt == *LANGUAGE_STRING_DATATYPE)
        {
            return Ok(Value::Boolean(a.same_term(b), !a.same_term(b)));
        }
        if op == Compare::Equal && (string_cell(a) || string_cell(b)) {
            return Ok(Value::Boolean(false, true));
        }
        if string_cell(a) && string_cell(b) && (a.key_class == 7 || b.key_class == 7) {
            return Err(Error::Unsupported("string comparison capacity".into()));
        }
        if op == Compare::Equal && (a.key_class == 0 || b.key_class == 0) {
            return Err(Error::Unsupported(
                "literal equality without a supported value key".into(),
            ));
        }
    }
    if a.kind == 3
        && b.kind == 3
        && a.key_class == b.key_class
        && matches!(a.key_class, 1 | 3 | 4 | 8 | 5 | 6)
    {
        return Ok(Value::Boolean(
            match op {
                Compare::Equal => a.key == b.key,
                Compare::Less => a.key < b.key,
                Compare::LessOrEqual => a.key <= b.key,
                Compare::Greater => a.key > b.key,
                Compare::GreaterOrEqual => a.key >= b.key,
            },
            false,
        ));
    }
    if op == Compare::Equal {
        if a.kind != 3 || b.kind != 3 {
            return Ok(Value::Boolean(a.same_term(b), false));
        }
        if a.same_term(b) {
            return Ok(Value::Boolean(true, false));
        }
    }
    Ok(Value::Boolean(false, true))
}

fn eval(
    e: &Expr,
    schema: &[Var],
    row: &[Cell],
    params: &[Option<Cell>],
    public: &[Parameter],
) -> Result<Value, Error> {
    Ok(match e {
        Expr::Term(slot) => Value::Term(match slot {
            Slot::Variable(v) => lookup(schema, row, *v),
            Slot::Parameter(p) => params[*p as usize].expect("term parameter"),
        }),
        Expr::Compare(op, a, b) => compare(
            *op,
            eval(a, schema, row, params, public)?,
            eval(b, schema, row, params, public)?,
        )?,
        Expr::Not(a) => {
            let (v, e) = eval(a, schema, row, params, public)?.ebv()?;
            Value::Boolean(!v, e)
        }
        Expr::And(a, b) | Expr::Or(a, b) => {
            let (a, ae) = eval(a, schema, row, params, public)?.ebv()?;
            let (b, be) = eval(b, schema, row, params, public)?.ebv()?;
            if matches!(e, Expr::And(..)) {
                Value::Boolean(a && b, (ae || be) && !(!a && !ae || !b && !be))
            } else {
                Value::Boolean(a || b, (ae || be) && !(a && !ae || b && !be))
            }
        }
        Expr::InList {
            value,
            start,
            count,
        } => {
            let Parameter::Index(count) = public[*count as usize] else {
                return Err(Error::Invalid("IN list length parameter".into()));
            };
            if count as usize > crate::plan::IN_CAPACITY {
                return Err(Error::Capacity("IN list".into()));
            }
            let a = eval(value, schema, row, params, public)?;
            let mut matched = false;
            let mut error = false;
            for i in 0..count {
                let b = params[(*start + i) as usize]
                    .ok_or_else(|| Error::Invalid("IN term parameter".into()))?;
                let (v, e) = compare(Compare::Equal, a, Value::Term(b))?.ebv()?;
                matched |= v && !e;
                error |= e;
            }
            Value::Boolean(matched, error && !matched)
        }
        Expr::In(a, bs) => {
            let a = eval(a, schema, row, params, public)?;
            let mut matched = false;
            let mut error = false;
            for b in bs {
                let (v, e) =
                    compare(Compare::Equal, a, eval(b, schema, row, params, public)?)?.ebv()?;
                matched |= v && !e;
                error |= e;
            }
            Value::Boolean(matched, error && !matched)
        }
    })
}
