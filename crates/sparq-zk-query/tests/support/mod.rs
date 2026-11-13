use oxrdf::{Literal, NamedNode, Term};
use sparq_zk_query::{
    interp::{Cell, CredentialCells, EncodeTerm, Word},
    Error,
};

/// Test-only supplied cells. This is not the credential suite's term encoder.
pub struct FixtureEncoder;
impl EncodeTerm for FixtureEncoder {
    fn encode(&self, term: &Term) -> Result<Cell, Error> {
        let mut enc = *blake3::hash(term.to_string().as_bytes()).as_bytes();
        enc[0] = 0;
        let mut c = Cell {
            bound: true,
            kind: match term {
                Term::NamedNode(_) => 1,
                Term::BlankNode(_) => 2,
                _ => 3,
            },
            enc,
            ..Cell::default()
        };
        if let Term::Literal(l) = term {
            let mut dt = *blake3::hash(l.datatype().as_str().as_bytes()).as_bytes();
            dt[0] = 0;
            c.dt = dt;
            match l
                .datatype()
                .as_str()
                .strip_prefix("http://www.w3.org/2001/XMLSchema#")
            {
                Some("integer" | "decimal") => {
                    let n = l
                        .value()
                        .parse::<f64>()
                        .map_err(|_| Error::Invalid("fixture number".into()))?;
                    let scaled = (n * 1e6).round() as i128 * 1_000_000_000_000;
                    c.key_class = 1;
                    c.key = decimal(scaled);
                }
                Some("boolean") => {
                    c.key_class = 5;
                    c.key[2] = u8::from(matches!(l.value(), "true" | "1"));
                }
                Some("string") => {
                    assert!(l.value().len() <= 30);
                    c.key_class = 6;
                    c.key[2..2 + l.value().len()].copy_from_slice(l.value().as_bytes());
                }
                _ => {}
            }
        }
        Ok(c)
    }
}

fn decimal(n: i128) -> Word {
    let mut key = [0; 32];
    key[2] = 0x80;
    let bytes = n.unsigned_abs().to_be_bytes();
    let mut carry = 0i16;
    for i in (0..32).rev() {
        let delta = if i >= 16 { i16::from(bytes[i - 16]) } else { 0 };
        let value = i16::from(key[i]) + if n < 0 { -delta } else { delta } + carry;
        key[i] = value.rem_euclid(256) as u8;
        carry = value.div_euclid(256);
    }
    key
}

pub fn iri(s: &str) -> Term {
    NamedNode::new(s).unwrap().into()
}
pub fn integer(n: i64) -> Term {
    Literal::new_typed_literal(
        n.to_string(),
        NamedNode::new("http://www.w3.org/2001/XMLSchema#integer").unwrap(),
    )
    .into()
}
pub fn quad(s: &Term, p: &Term, o: &Term) -> [Cell; 4] {
    let enc = FixtureEncoder;
    [
        enc.encode(s).unwrap(),
        enc.encode(p).unwrap(),
        enc.encode(o).unwrap(),
        Cell::default(),
    ]
}
pub fn fixture() -> Vec<CredentialCells> {
    vec![CredentialCells {
        quads: vec![
            quad(&iri("urn:a"), &iri("urn:p"), &integer(1)),
            quad(&iri("urn:b"), &iri("urn:p"), &integer(2)),
        ],
    }]
}
