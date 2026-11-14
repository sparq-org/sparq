//! Host mirror of the shared exact expression bucket over checked suite keys.
//! Values use signed scaled magnitudes; errors and capacity refusals remain distinct.
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
const SCALE: i128 = 1_000_000_000_000_000_000;
const LIMIT: i128 = 1i128 << 112;
fn checked(n: i128) -> Result<i128, Error> {
    if n.unsigned_abs() >= LIMIT as u128 {
        Err(Error::Capacity("numeric capacity".into()))
    } else {
        Ok(n)
    }
}
fn signed(n: Fr, negative: bool) -> Result<i128, Error> {
    let bytes = field_to_be_bytes_32(&n);
    if bytes[..18] != [0; 18] {
        return Err(Error::Capacity(
            "numeric capacity or exact precision".into(),
        ));
    }
    let magnitude = u128::from_be_bytes(bytes[16..].try_into().unwrap()) as i128;
    checked(if negative { -magnitude } else { magnitude })
}
impl Integer {
    fn evaluate(
        &self,
        vars: &[Var],
        row: &[Cell],
        params: &[Option<Cell>],
    ) -> Result<(i128, bool), Error> {
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
                    return Ok((0, true));
                }
                if c.key_class == 127 {
                    return Err(Error::Capacity("numeric capacity".into()));
                }
                let types =
                    crate::interp_closed_group::IntegerCodec::new(crate::credential::SuiteEncoder)?;
                use crate::interp_closed_group::NumericCodec;
                if c.kind != 3
                    || c.key_class != 1
                    || ![types.types().integer_dt, types.types().decimal_dt].contains(&c.dt)
                {
                    return Ok((0, true));
                }
                let mut key = c.key;
                if key[0] != 0 || key[1] != 1 {
                    return Err(Error::Invalid("numeric key class".into()));
                }
                key[1] = 0;
                let negative = key < field_to_be_bytes_32(&offset());
                let n = field(key) - offset();
                let n = signed(if negative { -n } else { n }, negative)?;
                if c.dt == types.types().integer_dt && n % SCALE != 0 {
                    return Err(Error::Invalid("fractional integer key".into()));
                }
                (n, false)
            }
            Self::Add(a, b) | Self::Subtract(a, b) | Self::Multiply(a, b) => {
                let (a, ae) = a.evaluate(vars, row, params)?;
                let (b, be) = b.evaluate(vars, row, params)?;
                if ae || be {
                    (0, true)
                } else {
                    let value = match self {
                        Self::Add(..) => checked(a + b)?,
                        Self::Subtract(..) => checked(a - b)?,
                        _ => {
                            let magnitude = Fr::from(a.unsigned_abs()) * Fr::from(b.unsigned_abs())
                                / Fr::from(SCALE as u128);
                            signed(magnitude, (a < 0) ^ (b < 0))?
                        }
                    };
                    (value, false)
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
