//! STAND-IN: lane D owns replacement of this typed integer expression seam.
//! Values use signed integer keys; errors are explicit, overflow is a refusal.
use crate::{
    interp::{Cell, Word},
    plan::{Compare, Slot, Var},
    Error,
};
use sparq_zk::field::{field_from_hex_str_strict, field_to_be_bytes_32, Fr};
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Integer {
    Term(Slot),
    Add(Box<Integer>, Box<Integer>),
    Subtract(Box<Integer>, Box<Integer>),
    Multiply(Box<Integer>, Box<Integer>),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Comparison {
    pub op: Compare,
    pub left: Integer,
    pub right: Integer,
}
fn field(w: Word) -> Fr {
    field_from_hex_str_strict(&w.iter().map(|b| format!("{b:02x}")).collect::<String>())
        .expect("canonical cell")
}
fn offset() -> Fr {
    "883423532389192164791648750371459257913741948437809479060803100646309888"
        .parse()
        .unwrap()
}
fn checked(n: Fr) -> Result<Fr, Error> {
    let key = field_to_be_bytes_32(&(n * Fr::from(1_000_000_000_000_000_000u64) + offset()));
    if key[..2] != [0; 2] {
        return Err(Error::Capacity("lane D integer arithmetic range".into()));
    }
    Ok(n)
}
impl Integer {
    fn evaluate(
        &self,
        vars: &[Var],
        row: &[Cell],
        params: &[Option<Cell>],
    ) -> Result<(Fr, bool), Error> {
        Ok(match self {
            Self::Term(s) => {
                let c = match s {
                    Slot::Variable(v) => vars
                        .iter()
                        .position(|a| a == v)
                        .map_or(Cell::default(), |i| row[i]),
                    Slot::Parameter(p) => params[*p as usize].expect("term"),
                };
                if !c.bound {
                    return Ok((Fr::from(0), true));
                }
                let types =
                    crate::interp_closed_group::IntegerCodec::new(crate::credential::SuiteEncoder)?;
                use crate::interp_closed_group::NumericCodec;
                if c.key_class == 127 {
                    return Err(Error::Capacity("lane D integer arithmetic range".into()));
                }
                if c.kind != 3 || c.key_class != 1 || c.dt != types.types().integer_dt {
                    return Err(Error::Unsupported(
                        "lane D integer arithmetic operands".into(),
                    ));
                }
                let mut key = c.key;
                if key[0] != 0 || key[1] != 1 {
                    return Err(Error::Invalid("integer key class".into()));
                }
                key[1] = 0;
                let scaled = field(key) - offset();
                let n = scaled / Fr::from(1_000_000_000_000_000_000u64);
                let magnitude = if key < field_to_be_bytes_32(&offset()) {
                    -n
                } else {
                    n
                };
                let bytes = field_to_be_bytes_32(&magnitude);
                if bytes[..9] != [0; 9] || bytes[9] >= 16 {
                    return Err(Error::Invalid("fractional integer key".into()));
                }
                (n, false)
            }
            Self::Add(a, b) | Self::Subtract(a, b) | Self::Multiply(a, b) => {
                let (a, ae) = a.evaluate(vars, row, params)?;
                let (b, be) = b.evaluate(vars, row, params)?;
                if ae || be {
                    (Fr::from(0), true)
                } else {
                    if matches!(self, Self::Multiply(..)) {
                        let negative = |n: Fr| {
                            field_to_be_bytes_32(
                                &(n * Fr::from(1_000_000_000_000_000_000u64) + offset()),
                            ) < field_to_be_bytes_32(&offset())
                        };
                        let am = if negative(a) { -a } else { a };
                        let bm = if negative(b) { -b } else { b };
                        if field_to_be_bytes_32(&am)[..17] != [0; 17]
                            || field_to_be_bytes_32(&bm)[..17] != [0; 17]
                        {
                            return Err(Error::Capacity(
                                "lane D multiplication operand range".into(),
                            ));
                        }
                        let magnitude = field_to_be_bytes_32(&(am * bm));
                        if magnitude[..9] != [0; 9] || magnitude[9] >= 16 {
                            return Err(Error::Capacity(
                                "lane D multiplication result range".into(),
                            ));
                        }
                    }
                    let n = match self {
                        Self::Add(..) => a + b,
                        Self::Subtract(..) => a - b,
                        _ => a * b,
                    };
                    (checked(n)?, false)
                }
            }
        })
    }
    fn encode(&self, out: &mut Vec<u8>) {
        match self {
            Self::Term(s) => {
                out.push(0);
                match s {
                    Slot::Variable(v) => {
                        out.push(0);
                        out.extend(v.to_le_bytes());
                    }
                    Slot::Parameter(p) => {
                        out.push(1);
                        out.extend(p.to_le_bytes());
                    }
                }
            }
            Self::Add(a, b) | Self::Subtract(a, b) | Self::Multiply(a, b) => {
                out.push(match self {
                    Self::Add(..) => 1,
                    Self::Subtract(..) => 2,
                    _ => 3,
                });
                a.encode(out);
                b.encode(out);
            }
        }
    }
}
impl Comparison {
    pub(crate) fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = vec![self.op as u8];
        self.left.encode(&mut out);
        self.right.encode(&mut out);
        out
    }
    pub(crate) fn evaluate(
        &self,
        vars: &[Var],
        row: &[Cell],
        params: &[Option<Cell>],
    ) -> Result<(bool, bool), Error> {
        let (a, ae) = self.left.evaluate(vars, row, params)?;
        let (b, be) = self.right.evaluate(vars, row, params)?;
        let key =
            |n: Fr| field_to_be_bytes_32(&(n * Fr::from(1_000_000_000_000_000_000u64) + offset()));
        let (a, b) = (key(a), key(b));
        Ok((
            match self.op {
                Compare::Equal => a == b,
                Compare::Less => a < b,
                Compare::LessOrEqual => a <= b,
                Compare::Greater => a > b,
                Compare::GreaterOrEqual => a >= b,
            },
            ae || be,
        ))
    }
}
