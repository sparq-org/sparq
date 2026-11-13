//! Standalone research grouping and aggregates, not externally audited.
//!
//! Groups use RDF term identity, including blank-node credential scope, and
//! cover the whole supplied table. Expression errors are an explicit input
//! lane; the plan integration must wire it from checked expression outputs.
//! Integer SUM and integer-input AVG use exact scaled keys. Decimal and double
//! input arithmetic and GROUP_CONCAT receive named refusals.

use crate::interp::{row_key, Cell, EncodeTerm, Row, Table, Word};
use crate::interp_closed_order::{order_cell, order_pair, table_shape, term_relation};
use crate::Error;
use oxrdf::{Literal, NamedNode};
use sparq_zk::field::{field_from_hex_str_strict, field_to_be_bytes_32, Fr};
use std::cmp::Ordering;

const SCALE: u64 = 1_000_000_000_000_000_000;
const XSD: &str = "http://www.w3.org/2001/XMLSchema#";

fn offset() -> Word {
    let mut w = [0; 32];
    w[2] = 0x80;
    w
}

fn field(w: Word) -> Fr {
    let hex: String = w.iter().map(|b| format!("{b:02x}")).collect();
    field_from_hex_str_strict(&hex).expect("validated field word")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AggregateKind {
    CountAll,
    Count,
    Sum,
    Min,
    Max,
    Avg,
    Sample,
    GroupConcat,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Aggregate {
    pub kind: AggregateKind,
    /// COUNT(*) has no operand column.
    pub column: Option<usize>,
    pub distinct: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NumericTypes {
    pub integer_dt: Word,
    pub decimal_dt: Word,
}

/// The matching Noir callback must CONSTRAIN the returned encoding from the
/// canonical numeric lexical form. It is circuit code, never a witness lookup.
pub trait NumericCodec {
    fn types(&self) -> NumericTypes;
    fn encode_numeric(&self, key: Word, decimal: bool) -> Result<Cell, Error>;
}

/// Host adapter for an existing term encoder; its in-circuit partner belongs to
/// the suite lane. This adapter does not authenticate supplied comparison keys.
pub struct IntegerCodec<E> {
    encoder: E,
    types: NumericTypes,
}

impl<E: EncodeTerm> IntegerCodec<E> {
    pub fn new(encoder: E) -> Result<Self, Error> {
        let int = encoder.encode(&literal("0", false).into())?;
        let dec = encoder.encode(&literal("0.0", true).into())?;
        int.validate()?;
        dec.validate()?;
        Ok(Self {
            encoder,
            types: NumericTypes {
                integer_dt: int.dt,
                decimal_dt: dec.dt,
            },
        })
    }
}

fn literal(value: &str, decimal: bool) -> Literal {
    Literal::new_typed_literal(
        value,
        NamedNode::new(format!(
            "{XSD}{}",
            if decimal { "decimal" } else { "integer" }
        ))
        .unwrap(),
    )
}

impl<E: EncodeTerm> NumericCodec for IntegerCodec<E> {
    fn types(&self) -> NumericTypes {
        self.types
    }

    fn encode_numeric(&self, key: Word, decimal: bool) -> Result<Cell, Error> {
        let lexical = key_lexical(key, decimal)?;
        let cell = self.encoder.encode(&literal(&lexical, decimal).into())?;
        numeric_contract(cell, key, decimal, self.types)?;
        Ok(cell)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HavingComparison {
    Equal,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Local numeric-comparison HAVING seam; general expressions wire from lane D.
pub struct Having {
    pub column: usize,
    pub comparison: HavingComparison,
    pub rhs: Cell,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupSpec {
    /// Compiled schema width, retained even for an empty input table.
    pub input_width: usize,
    pub columns: Vec<usize>,
    pub aggregates: Vec<Aggregate>,
    pub having: Option<Having>,
    pub capacity: usize,
}

fn numeric_contract(c: Cell, key: Word, decimal: bool, types: NumericTypes) -> Result<(), Error> {
    c.validate()?;
    if !c.bound
        || c.kind != 3
        || c.key_class != 1
        || c.key != key
        || c.dt
            != if decimal {
                types.decimal_dt
            } else {
                types.integer_dt
            }
        || c.lang != [0; 32]
        || c.scope != 0
    {
        return Err(Error::Invalid("numeric encoding contract".into()));
    }
    Ok(())
}

fn numeric(key: Fr, decimal: bool, codec: &impl NumericCodec) -> Result<Cell, Error> {
    let key = field_to_be_bytes_32(&key);
    if key[..2] != [0; 2] {
        return Err(Error::Capacity(
            "aggregate numeric range or AVG precision".into(),
        ));
    }
    let cell = codec.encode_numeric(key, decimal)?;
    numeric_contract(cell, key, decimal, codec.types())?;
    Ok(cell)
}

fn integer_key(c: Cell, types: NumericTypes) -> Result<Fr, Error> {
    if c.kind != 3 || c.key_class != 1 || c.dt != types.integer_dt {
        return Err(Error::Unsupported(
            "integer aggregate arithmetic required".into(),
        ));
    }
    if c.key[..2] != [0; 2] {
        return Err(Error::Capacity("aggregate integer key".into()));
    }
    let n = field(c.key) - field(offset());
    let magnitude = if c.key < offset() { -n } else { n };
    let quotient = field_to_be_bytes_32(&(magnitude / Fr::from(SCALE)));
    if quotient[..9] != [0; 9] || quotient[9] >= 16 {
        return Err(Error::Invalid("aggregate integer key is fractional".into()));
    }
    Ok(n)
}

fn same_group(a: &[Cell], b: &[Cell], columns: &[usize]) -> bool {
    columns.iter().all(|&c| a[c].same_term(b[c]))
}

fn aggregate(
    input: &Table,
    members: &[usize],
    errors: &[Vec<bool>],
    index: usize,
    spec: Aggregate,
    codec: &impl NumericCodec,
) -> Result<Cell, Error> {
    use AggregateKind as K;
    let value = |i: usize| spec.column.map_or(Cell::default(), |c| input.rows[i][c]);
    let mut selected = vec![];
    for &i in members {
        if !spec.distinct
            || !selected.iter().any(|&j| {
                if spec.kind == K::CountAll {
                    input.rows[i]
                        .iter()
                        .zip(&input.rows[j])
                        .all(|(&a, &b)| a.same_term(b))
                } else {
                    value(i).same_term(value(j)) && errors[i][index] == errors[j][index]
                }
            })
        {
            selected.push(i);
        }
    }
    let vals: Vec<_> = selected
        .iter()
        .copied()
        .filter(|&i| value(i).bound && !errors[i][index])
        .map(value)
        .collect();
    if matches!(spec.kind, K::CountAll | K::Count) {
        let count = if spec.kind == K::CountAll {
            selected.len()
        } else {
            vals.len()
        };
        return numeric(
            field(offset()) + Fr::from(count as u64) * Fr::from(SCALE),
            false,
            codec,
        );
    }
    if matches!(spec.kind, K::Sum | K::Avg) {
        let mut sum = Fr::from(0);
        let mut error = selected
            .iter()
            .any(|&i| errors[i][index] || !value(i).bound);
        for &c in &vals {
            if c.kind == 3 && matches!(c.key_class, 1 | 2 | 127) {
                sum += integer_key(c, codec.types())?;
            } else {
                error = true;
            }
        }
        // The Recommendation explicitly defines AVG(M)=0 when Count(M)=0.
        if spec.kind == K::Avg && vals.is_empty() {
            return numeric(field(offset()), false, codec);
        }
        if error {
            return Ok(Cell::default());
        }
        let decimal = spec.kind == K::Avg && !vals.is_empty();
        let divisor = if decimal { vals.len() } else { 1 };
        return numeric(
            field(offset()) + sum / Fr::from(divisor as u64),
            decimal,
            codec,
        );
    }
    if matches!(spec.kind, K::Min | K::Max) {
        for &v in &vals {
            order_cell(v)?;
            for &other in &vals {
                order_pair(v, other)?;
            }
        }
        // Use the error-propagating ToList reading of the published algebra.
        if selected
            .iter()
            .any(|&i| errors[i][index] || !value(i).bound)
        {
            return Ok(Cell::default());
        }
    }
    let direction = if spec.kind == K::Min {
        Ordering::Less
    } else {
        Ordering::Greater
    };
    Ok(vals
        .iter()
        .copied()
        .filter(|&v| {
            spec.kind == K::Sample
                || !vals
                    .iter()
                    .any(|&other| term_relation(other, v) == Some(direction))
        })
        .min_by_key(|&c| row_key(&[c]))
        .unwrap_or_default())
}

fn having_keeps(row: &[Cell], having: Having) -> Result<bool, Error> {
    let a = row[having.column];
    let b = having.rhs;
    b.validate()?;
    let compatible = a.bound
        && b.bound
        && a.kind == 3
        && b.kind == 3
        && a.key_class == b.key_class
        && matches!(a.key_class, 1 | 5);
    if !compatible {
        if a.bound && b.bound {
            return Err(Error::Unsupported(
                "HAVING numeric comparison required".into(),
            ));
        }
        return Ok(false);
    }
    if a.key[..2] != [0; 2] || b.key[..2] != [0; 2] {
        return Err(Error::Capacity("HAVING comparison capacity".into()));
    }
    let cmp = a.key.cmp(&b.key);
    Ok(match having.comparison {
        HavingComparison::Equal => cmp == Ordering::Equal,
        HavingComparison::Less => cmp == Ordering::Less,
        HavingComparison::LessOrEqual => cmp != Ordering::Greater,
        HavingComparison::Greater => cmp == Ordering::Greater,
        HavingComparison::GreaterOrEqual => cmp != Ordering::Less,
    })
}

pub fn group(
    input: &Table,
    errors: &[Vec<bool>],
    spec: &GroupSpec,
    codec: &impl NumericCodec,
) -> Result<Table, Error> {
    use AggregateKind as K;
    let width = spec.input_width;
    table_shape(input, width)?;
    if input.rows.len() > 32 {
        return Err(Error::Capacity("integer accumulator capacity".into()));
    }
    if errors.len() != input.rows.len() || errors.iter().any(|r| r.len() != spec.aggregates.len()) {
        return Err(Error::Invalid("aggregate error lane shape".into()));
    }
    if spec.columns.iter().any(|&c| c >= width) {
        return Err(Error::Invalid("GROUP BY column".into()));
    }
    for (m, agg) in spec.aggregates.iter().enumerate() {
        if agg.kind == K::GroupConcat {
            return Err(Error::Unsupported(
                "GROUP_CONCAT lexical opening required".into(),
            ));
        }
        if (agg.kind == K::CountAll) != agg.column.is_none()
            || agg.column.is_some_and(|c| c >= width)
        {
            return Err(Error::Invalid("aggregate column".into()));
        }
        for (i, err) in errors.iter().enumerate() {
            if err[m] && (agg.kind == K::CountAll || input.rows[i][agg.column.unwrap()].bound) {
                return Err(Error::Invalid("aggregate error cell".into()));
            }
        }
    }
    let output_width = spec.columns.len() + spec.aggregates.len();
    if spec.having.is_some_and(|h| h.column >= output_width) {
        return Err(Error::Invalid("HAVING column".into()));
    }
    if let Some(h) = spec.having {
        h.rhs.validate()?;
    }
    let mut groups: Vec<Vec<usize>> = vec![];
    for i in 0..input.rows.len() {
        if let Some(members) = groups
            .iter_mut()
            .find(|g| same_group(&input.rows[i], &input.rows[g[0]], &spec.columns))
        {
            members.push(i);
        } else {
            groups.push(vec![i]);
        }
    }
    if input.rows.is_empty() && spec.columns.is_empty() {
        groups.push(vec![]);
    }
    let mut rows = vec![];
    for members in groups {
        let mut row: Row = spec
            .columns
            .iter()
            .map(|&c| input.rows[members[0]][c])
            .collect();
        for (m, &agg) in spec.aggregates.iter().enumerate() {
            row.push(aggregate(input, &members, errors, m, agg, codec)?);
        }
        if spec
            .having
            .map(|h| having_keeps(&row, h))
            .transpose()?
            .unwrap_or(true)
        {
            if rows.len() >= spec.capacity {
                return Err(Error::Capacity("group output".into()));
            }
            rows.push(row);
        }
    }
    rows.sort_by_key(|r| row_key(r));
    Ok(Table { rows })
}

// Integer long division stays outside the circuit. The circuit enforces exact
// field division and range checks; the codec converts the resulting scaled key
// into a canonical lexical form for the existing term encoder.
fn div_small(w: Word, divisor: u64) -> (Word, u64) {
    let mut out = [0; 32];
    let mut remainder = 0u128;
    for (i, b) in w.into_iter().enumerate() {
        let n = remainder * 256 + u128::from(b);
        out[i] = (n / u128::from(divisor)) as u8;
        remainder = n % u128::from(divisor);
    }
    (out, remainder as u64)
}

pub(crate) fn key_lexical(key: Word, decimal: bool) -> Result<String, Error> {
    if key[..2] != [0; 2] {
        return Err(Error::Capacity("aggregate numeric range".into()));
    }
    let negative = key < offset();
    let value = field(key) - field(offset());
    let magnitude = field_to_be_bytes_32(&if negative { -value } else { value });
    let (mut integer, fraction) = div_small(magnitude, SCALE);
    if !decimal && fraction != 0 {
        return Err(Error::Invalid("aggregate integer key is fractional".into()));
    }
    let mut digits = vec![];
    while integer != [0; 32] {
        let (q, r) = div_small(integer, 10);
        digits.push((b'0' + r as u8) as char);
        integer = q;
    }
    if digits.is_empty() {
        digits.push('0');
    }
    let mut lexical: String = digits.into_iter().rev().collect();
    if negative {
        lexical.insert(0, '-');
    }
    if decimal {
        let fraction = format!("{fraction:018}");
        lexical.push('.');
        lexical.push_str(if fraction.trim_end_matches('0').is_empty() {
            "0"
        } else {
            fraction.trim_end_matches('0')
        });
    }
    Ok(lexical)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_lexical_conversion_and_integrality() {
        let zero = field(offset());
        let scaled = Fr::from(SCALE);
        assert_eq!(
            key_lexical(field_to_be_bytes_32(&(zero - scaled * Fr::from(12))), false).unwrap(),
            "-12"
        );
        assert_eq!(
            key_lexical(field_to_be_bytes_32(&(zero - scaled / Fr::from(2))), true).unwrap(),
            "-0.5"
        );
        assert_eq!(key_lexical(offset(), true).unwrap(), "0.0");
        assert!(key_lexical(field_to_be_bytes_32(&(zero + Fr::from(1))), false).is_err());
    }
}
