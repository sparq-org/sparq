#![allow(dead_code)]

use oxrdf::{BlankNode, Literal, NamedNode, Term};
use sparq_zk::field::{field_to_be_bytes_32, Fr};
use sparq_zk_query::{
    interp::{Cell, EncodeTerm, Table, Word},
    Error,
};

pub struct Encoder;

pub fn hash(s: &str) -> Word {
    let mut w = *blake3::hash(s.as_bytes()).as_bytes();
    w[0] = 0;
    w
}

pub fn key(s: &str) -> Word {
    let negative = s.starts_with('-');
    let s = s.trim_start_matches(['-', '+']);
    let (whole, fraction) = s.split_once('.').unwrap_or((s, ""));
    assert!(fraction.len() <= 18);
    let scaled: Fr = format!("{whole}{fraction:0<18}").parse().unwrap();
    let offset: Fr = "883423532389192164791648750371459257913741948437809479060803100646309888"
        .parse()
        .unwrap();
    field_to_be_bytes_32(&if negative {
        offset - scaled
    } else {
        offset + scaled
    })
}

impl EncodeTerm for Encoder {
    fn encode(&self, term: &Term) -> Result<Cell, Error> {
        let mut c = Cell {
            bound: true,
            enc: hash(&term.to_string()),
            kind: match term {
                Term::NamedNode(_) => 1,
                Term::BlankNode(_) => 2,
                _ => 3,
            },
            ..Cell::default()
        };
        match term {
            Term::NamedNode(n) => {
                if n.as_str().len() > 30 {
                    return Err(Error::Unsupported("fixture IRI lexical key".into()));
                }
                c.key_class = 6;
                c.key[2..2 + n.as_str().len()].copy_from_slice(n.as_str().as_bytes());
            }
            Term::Literal(l) => {
                c.dt = hash(l.datatype().as_str());
                if let Some(lang) = l.language() {
                    c.lang = hash(lang);
                }
                match l
                    .datatype()
                    .as_str()
                    .strip_prefix("http://www.w3.org/2001/XMLSchema#")
                {
                    Some("integer" | "decimal") => {
                        c.key_class = 1;
                        c.key = key(l.value());
                    }
                    Some("boolean") => {
                        c.key_class = 5;
                        c.key[2] = u8::from(matches!(l.value(), "true" | "1"));
                    }
                    Some("string") => {
                        c.key_class = if l.value().len() <= 30 { 6 } else { 7 };
                        if c.key_class == 6 {
                            c.key[2..2 + l.value().len()].copy_from_slice(l.value().as_bytes());
                        }
                    }
                    Some("double" | "float") => c.key_class = 2,
                    _ => {}
                }
            }
            _ => {}
        }
        c.validate()?;
        Ok(c)
    }
}

pub fn int(n: i64) -> Cell {
    Encoder.encode(&Literal::from(n).into()).unwrap()
}
pub fn iri(s: &str) -> Cell {
    Encoder.encode(&NamedNode::new(s).unwrap().into()).unwrap()
}
pub fn canonical(mut rows: Vec<Vec<Cell>>) -> Table {
    rows.sort_by_key(|r| {
        r.iter()
            .map(|c| (c.bound, c.enc, c.scope))
            .collect::<Vec<_>>()
    });
    Table { rows }
}

pub fn engine(rdf: &str, query: &str, variables: &[&str]) -> Table {
    let graph = sparq_core::Graph::load_str(rdf, "ntriples").unwrap();
    let json: serde_json::Value =
        serde_json::from_str(&sparq_engine::query_json(&graph, query).unwrap()).unwrap();
    let rows = json["results"]["bindings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            variables
                .iter()
                .map(|v| {
                    let b = &row[*v];
                    let term: Term = match b["type"].as_str() {
                        None => return Cell::default(),
                        Some("uri") => NamedNode::new(b["value"].as_str().unwrap()).unwrap().into(),
                        Some("bnode") => {
                            BlankNode::new(b["value"].as_str().unwrap()).unwrap().into()
                        }
                        Some("literal" | "typed-literal") => {
                            let value = b["value"].as_str().unwrap();
                            if let Some(lang) = b["xml:lang"].as_str() {
                                Literal::new_language_tagged_literal(value, lang)
                                    .unwrap()
                                    .into()
                            } else {
                                Literal::new_typed_literal(
                                    value,
                                    NamedNode::new(
                                        b["datatype"]
                                            .as_str()
                                            .unwrap_or("http://www.w3.org/2001/XMLSchema#string"),
                                    )
                                    .unwrap(),
                                )
                                .into()
                            }
                        }
                        other => panic!("binding {other:?}"),
                    };
                    Encoder.encode(&term).unwrap()
                })
                .collect()
        })
        .collect();
    Table { rows }
}

pub fn errors(table: &Table, aggregates: usize) -> Vec<Vec<bool>> {
    vec![vec![false; aggregates]; table.rows.len()]
}
