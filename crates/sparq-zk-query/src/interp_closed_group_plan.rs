//! Suite-key bridge for the standalone grouping relation.
use crate::{
    credential::SuiteEncoder,
    interp::{Cell, EncodeTerm, Table, Word},
    interp_closed_group::*,
    plan::Var,
    plan_closed::GroupAggregate,
    Error,
};
use sparq_zk::field::{field_from_hex_str_strict, field_to_be_bytes_32, Fr};
fn field(w: Word) -> Fr {
    field_from_hex_str_strict(&w.iter().map(|b| format!("{b:02x}")).collect::<String>())
        .expect("canonical word")
}
fn prefix() -> Fr {
    let mut w = [0; 32];
    w[1] = 1;
    field(w)
}
fn normalize(mut c: Cell) -> Result<Cell, Error> {
    if c.bound && c.kind == 3 && c.key_class == 1 {
        if c.key[0] != 0 || c.key[1] != 1 {
            return Err(Error::Invalid("suite numeric key prefix".into()));
        }
        c.key[1] = 0;
        c.lang = [0; 32];
    }
    Ok(c)
}
struct Codec;
impl NumericCodec for Codec {
    fn types(&self) -> NumericTypes {
        IntegerCodec::new(SuiteEncoder)
            .expect("suite numeric datatypes")
            .types()
    }
    fn encode_numeric(&self, key: Word, decimal: bool) -> Result<Cell, Error> {
        let lexical = key_lexical(key, decimal)?;
        // Mirror the exact scaled capacity used by the shared expression encoder.
        let offset: Fr = "883423532389192164791648750371459257913741948437809479060803100646309888"
            .parse()
            .unwrap();
        let n = field(key) - offset;
        let magnitude = field_to_be_bytes_32(&if key < field_to_be_bytes_32(&offset) {
            -n
        } else {
            n
        });
        if magnitude[..18] != [0; 18] {
            return Err(Error::Capacity("aggregate numeric capacity".into()));
        }
        let datatype = if decimal { "decimal" } else { "integer" };
        normalize(
            SuiteEncoder.encode(
                &oxrdf::Literal::new_typed_literal(
                    lexical,
                    oxrdf::NamedNode::new_unchecked(format!(
                        "http://www.w3.org/2001/XMLSchema#{datatype}"
                    )),
                )
                .into(),
            )?,
        )
    }
}
pub(crate) fn evaluate(
    input: &Table,
    schema: &[Var],
    keys: &[Var],
    aggregates: &[GroupAggregate],
    capacity: usize,
) -> Result<Table, Error> {
    let column = |v: &Var| schema.iter().position(|x| x == v).expect("group schema");
    let normalized = Table {
        rows: input
            .rows
            .iter()
            .map(|r| r.iter().copied().map(normalize).collect())
            .collect::<Result<_, _>>()?,
    };
    let spec = GroupSpec {
        input_width: schema.len(),
        columns: keys.iter().map(column).collect(),
        aggregates: aggregates
            .iter()
            .map(|a| Aggregate {
                kind: a.kind,
                column: a.operand.as_ref().map(column),
                distinct: a.distinct,
            })
            .collect(),
        having: None,
        capacity,
    };
    let errors = vec![vec![false; aggregates.len()]; input.rows.len()];
    let mut result = group(&normalized, &errors, &spec, &Codec)?;
    let lang = SuiteEncoder.encode(&oxrdf::Literal::from(0).into())?.lang;
    for r in &mut result.rows {
        for c in r {
            if c.bound && c.kind == 3 && c.key_class == 1 {
                c.key = field_to_be_bytes_32(&(field(c.key) + prefix()));
                c.lang = lang;
            }
        }
    }
    Ok(result)
}
