//! Shared component witnesses for genuine V2 scan and join fixtures.
use oxrdf::{NamedNode, Term, Triple};
use serde::Serialize;
use sparq_zk::commit::commit_triples;
use sparq_zk::field::{field_from_hash_bytes, Fr};
use sparq_zk_compose::manifest::{CircuitId, FieldHex, ProofInputs};

fn iri(s: &str) -> NamedNode { NamedNode::new(s).unwrap() }

pub(crate) struct Graph {
    pub(crate) commitment: FieldHex,
    pub(crate) method_commitment: sparq_zk::sig::MethodCommitment,
    pub(crate) salt: Fr,
    pub(crate) enc: [FieldHex; 3],
    identities: Vec<[FieldHex; 3]>,
}

impl Graph {
    pub(crate) fn new(v2: bool, predicate: &str, salt: u64) -> Self {
        let salt = Fr::from(salt);
        let terms = ["http://ex/s", predicate, "http://ex/o"].map(|s| Term::NamedNode(iri(s)));
        let triple = Triple::new(iri("http://ex/s"), iri(predicate), iri("http://ex/o"));
        let method_commitment = if v2 {
            sparq_zk::dual_leaf_v2::commit_triples(&[triple], salt).unwrap().method_commitment()
        } else {
            commit_triples(&[triple], salt).unwrap().method_commitment()
        };
        let enc = terms.map(|t| FieldHex::from_field(&if v2 {
            sparq_zk::dual_leaf_v2::encode_term(&t, &salt).unwrap().unwrap()
        } else {
            sparq_zk::encode::encode_term(&t, &salt).unwrap()
        }));
        let mut identities = vec![zero_row(); 16];
        identities[0] = ["http://ex/s", predicate, "http://ex/o"].map(|s|
            FieldHex::from_field(&field_from_hash_bytes(blake3::hash(s.as_bytes()).as_bytes())));
        Self { commitment: FieldHex::from_field(&method_commitment.commitment),
            method_commitment, salt, enc, identities }
    }

    pub(crate) fn scan(&self, v2: bool) -> ProofInputs {
        ProofInputs::Scan {
            id: if v2 { CircuitId::scan_v2_dl(1, 16, 4).unwrap() }
                else { CircuitId::Scan { k: 1, n: 16, r: 4 } },
            commitments: vec![self.commitment.clone()],
            pattern_is_const: [false, true, false],
            pattern_const_enc: [fh0(), self.enc[1].clone(), fh0()],
            rows: vec![self.enc.clone()], row_count: 1, attribution: vec![true],
        }
    }

    pub(crate) fn components(&self, out: &mut String, suffix: &str, scan: bool) {
        let mut tc = vec![zero_row(); 16];
        let mut vs = tc.clone();
        let mut lc = tc.clone();
        tc[0] = std::array::from_fn(|_| FieldHex("0x1".into()));
        vs[0] = std::array::from_fn(|_| FieldHex::from_field(&sparq_zk::dual_leaf_v2::value_none()));
        lc[0] = std::array::from_fn(|_| FieldHex("0x1".into()));
        let zeros = vec![zero_row(); 16];
        let flags = vec![[false; 3]; 16];
        for (key, rows) in [("type_codes", &tc), ("value_slot0", &vs),
            ("datatype_consts", &zeros), ("lang_consts", &lc), ("identities", &self.identities)] {
            if scan { put(out, &format!("{key}{suffix}"), &[rows]); }
            else { put(out, &format!("{key}{suffix}"), rows); }
        }
        for key in ["has_value", "has_lang"] {
            if scan { put(out, &format!("{key}{suffix}"), &[&flags]); }
            else { put(out, &format!("{key}{suffix}"), &flags); }
        }
    }
}

fn fh0() -> FieldHex { FieldHex("0x0".into()) }
fn zero_row() -> [FieldHex; 3] { std::array::from_fn(|_| fh0()) }
pub(crate) fn put(out: &mut String, key: &str, value: &impl Serialize) {
    out.push_str(&format!("{key} = {}\n", serde_json::to_string(value).unwrap()));
}

pub(crate) fn scan_toml(g: &Graph, inputs: &ProofInputs, components: bool) -> String {
    let ProofInputs::Scan { commitments, pattern_is_const, pattern_const_enc, rows,
        row_count, attribution, .. } = inputs else { unreachable!() };
    let mut out = String::new();
    put(&mut out, "challenge", &FieldHex("0x2a".into()));
    put(&mut out, "commitments", commitments);
    put(&mut out, "pattern_is_const", pattern_is_const);
    put(&mut out, "pattern_const_enc", pattern_const_enc);
    let mut padded = rows.clone(); padded.resize(4, zero_row());
    put(&mut out, "rows", &padded);
    put(&mut out, "row_count", row_count);
    put(&mut out, "attribution", attribution);
    put(&mut out, "counts", &[1u32]);
    if components { g.components(&mut out, "", true); }
    else {
        let mut enc = vec![zero_row(); 16]; enc[0] = g.enc.clone();
        put(&mut out, "enc", &[enc]);
    }
    out
}
