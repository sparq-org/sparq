//! Reproduce the signed native envelope vectors on stdout.
use oxrdf::{Literal, Term};
use sha2::{Digest, Sha256};
use sparq_merkle_suite::{metadata, poseidon as p};
use sparq_zk::{field::field_to_hex, sig};
use sparq_zk_query::envelope_policy::{
    holder_message, snapshot_digest, status_leaf, trust_member_digest, TrustMember,
};
use std::fmt::Write;

fn raw(term: Option<&Term>) -> String {
    let Some(term) = term else {
        return "absent()".into();
    };
    let (kind, text, datatype, language, key) = match term {
        Term::NamedNode(n) => (1, n.as_str(), "", "", [0; 31]),
        Term::BlankNode(n) => (2, n.as_str(), "", "", [0; 31]),
        Term::Literal(l) => (
            3,
            l.value(),
            l.datatype().as_str(),
            l.language().unwrap_or(""),
            sparq_merkle_suite::comparison_key(l),
        ),
        Term::Triple(_) => panic!("triple term outside the profile"),
    };
    format!(
        "raw({kind},{text:?}.as_bytes(),{datatype:?}.as_bytes(),{language:?}.as_bytes(),{key:?})"
    )
}
#[derive(Default)]
struct Cells(std::collections::BTreeMap<String, usize>);
impl Cells {
    fn raw(&mut self, term: Option<&Term>) -> String {
        let body = raw(term);
        if body == "absent()" {
            return body;
        }
        let next = self.0.len();
        let id = *self.0.entry(body).or_insert(next);
        format!("term_{id}()")
    }
}
fn signature(key: &sig::SecretKey, message: &sparq_merkle_suite::Fr) -> String {
    let sig = sig::sign_deterministic(key, message);
    let w = sig::in_circuit_witness(&key.public_key(), message, &sig).unwrap();
    format!(
        "Signature {{present:true,r:Point {{x:{},y:{}}},s:{},e:{},e_k:{}}}",
        field_to_hex(&w.r_x),
        field_to_hex(&w.r_y),
        field_to_hex(&w.s),
        field_to_hex(&w.e),
        field_to_hex(&w.e_k)
    )
}
fn key(n: u8) -> sig::SecretKey {
    p::keypair_from_seed(&[n; 32])
}
fn point(key: &sig::SecretKey) -> String {
    let (x, y) = key.public_key().coords().unwrap();
    format!("Point {{x:{},y:{}}}", field_to_hex(&x), field_to_hex(&y))
}
fn document(
    name: &str,
    issuer: u8,
    holder: Option<u8>,
    status: Option<&str>,
    bounds: Option<(&str, &str)>,
) -> String {
    let mut rows=vec![String::from("<urn:credential> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://www.w3.org/2018/credentials#VerifiableCredential>"),
        format!("<urn:credential> <https://www.w3.org/2018/credentials#issuer> <urn:issuer:{issuer}>"),
        "<urn:subject> <urn:shared> <urn:value>".into()];
    rows.push(format!(
        "<urn:subject> <urn:fact:issuer> <urn:fact:{issuer}>"
    ));
    if name == "missing_issuer" {
        rows.remove(1);
    }
    if let Some(h) = holder {
        rows.extend(["<urn:credential> <https://www.w3.org/2018/credentials#credentialSubject> <urn:subject>".into(),
            "<urn:subject> <https://www.w3.org/2018/credentials#confidenceMethod> <urn:holder>".into(),
            format!("<urn:holder> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <{}>",if name=="bad_holder_type" {"urn:other"} else {"https://w3id.org/security#Multikey"}),
            format!("<urn:holder> <https://w3id.org/security#publicKeyMultibase> \"{}\"",metadata::multikey(&key(h).public_key()).unwrap())]);
        if name == "ambiguous_holder" {
            rows.push(format!(
                "<urn:holder> <https://w3id.org/security#publicKeyMultibase> \"{}\"",
                metadata::multikey(&key(12).public_key()).unwrap()
            ));
        }
    }
    if let Some((from, until)) = bounds {
        if !from.is_empty() {
            rows.push(format!(
                "<urn:credential> <https://www.w3.org/2018/credentials#validFrom> {from}"
            ));
        }
        if !until.is_empty() {
            rows.push(format!(
                "<urn:credential> <https://www.w3.org/2018/credentials#validUntil> {until}"
            ));
        }
    }
    if name == "duplicate_until" {
        rows.push("<urn:credential> <https://www.w3.org/2018/credentials#validUntil> \"1970-01-01T00:00:00.301Z\"^^<http://www.w3.org/2001/XMLSchema#dateTime>".into());
    }
    if name == "unrelated_bounds" {
        rows.push("<urn:other> <https://www.w3.org/2018/credentials#validUntil> \"1970-01-01T00:00:00.001Z\"^^<http://www.w3.org/2001/XMLSchema#dateTime>".into());
    }
    if name == "additional_type" {
        rows.push(
            "<urn:credential> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <urn:Additional>"
                .into(),
        );
    }
    if let Some(purpose) = status {
        let entries = if name == "two_status" { 2 } else { 1 };
        for n in 0..entries {
            rows.extend([format!("<urn:credential> <https://www.w3.org/2018/credentials#credentialStatus> <urn:status:{n}>"),
                format!("<urn:status:{n}> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <{}>",if name=="bad_status_type" {"urn:Other"} else {"https://www.w3.org/ns/credentials/status#BitstringStatusListEntry"}),
                format!("<urn:status:{n}> <https://www.w3.org/ns/credentials/status#statusListCredential> <urn:list:{purpose}>"),
                format!("<urn:status:{n}> <https://www.w3.org/ns/credentials/status#statusPurpose> \"{purpose}\"")]);
            if name != "missing_index" {
                rows.push(format!("<urn:status:{n}> <https://www.w3.org/ns/credentials/status#statusListIndex> {}",if name=="bad_index" {"\"one\""} else {"\"0001\""}));
            }
            if name == "size_two" || name == "size_one" {
                rows.push(format!("<urn:status:{n}> <https://www.w3.org/ns/credentials/status#statusSize> \"{}\"^^<http://www.w3.org/2001/XMLSchema#integer>",if name=="size_two" {2} else {1}));
            }
        }
    }
    if name == "reserved" {
        rows.push(
            "<urn:subject> <https://w3id.org/sparq/vcq#verifiedIssuer> <urn:issuer:1>".into(),
        );
    }
    // The combined fixtures carry enough facts to exercise the actual Solid bucket.
    if name.starts_with("complete") || name == "last_status" || name == "last_until" {
        for n in 0..32 {
            rows.push(format!(
                "<urn:subject> <urn:fact:{n}> \"{}\"^^<http://www.w3.org/2001/XMLSchema#integer>",
                n + 1
            ));
        }
    }
    if name == "complete_other_issuer" {
        rows = rows
            .into_iter()
            .map(|r| r.replace("<urn:issuer:3>", "<urn:issuer:4>"))
            .collect();
    }
    if name == "blank_scope" {
        rows = rows
            .into_iter()
            .map(|r| {
                r.replace("<urn:subject>", "_:subject")
                    .replace("<urn:holder>", "_:holder")
            })
            .collect();
    }
    rows.into_iter()
        .map(|r| {
            format!(
                "{r}{} .\n",
                if name == "named" { " <urn:graph>" } else { "" }
            )
        })
        .collect()
}
fn time(text: &str) -> String {
    format!("\"{text}\"^^<http://www.w3.org/2001/XMLSchema#dateTime>")
}
fn main() {
    let mut cells = Cells::default();
    let mut out=String::from("// Generated by cargo run -p sparq-zk-query --example envelope_fixtures.\n// Signed seeded credentials; values are fixed cross-language controls.\nuse envelope_core::envelope::{CredentialOpening,Point,Signature};\nuse envelope_core::suite_entry::TermOpening;\nuse std::collections::bounded_vec::BoundedVec;\nfn absent<let L:u32>() -> TermOpening<L> {TermOpening {bound:false,kind:0,lexical:[0;L],lexical_len:0,datatype:[0;128],datatype_len:0,language:[0;64],language_len:0,key:[0;31]}}\nfn raw<let L:u32,let T:u32,let D:u32,let G:u32>(kind:u8,text:[u8;T],dt:[u8;D],lang:[u8;G],key:[u8;31]) -> TermOpening<L> {\nlet mut r=absent::<L>();r.bound=true;r.kind=kind;r.lexical_len=T;r.datatype_len=D;r.language_len=G;r.key=key;\nfor i in 0..T {r.lexical[i]=text[i];} for i in 0..D {r.datatype[i]=dt[i];} for i in 0..G {r.language[i]=lang[i];} r }\n");
    for n in [1, 2, 3, 11, 12] {
        writeln!(out, "pub fn key_{n}() -> Point {{{}}}", point(&key(n))).unwrap();
    }
    let from = time("1970-01-01T00:00:00.101Z");
    let until = time("1970-01-01T00:00:00.201Z");
    let equal = time("1970-01-01T00:00:00.101Z");
    let reverse = time("1970-01-01T00:00:00.100Z");
    let late = time("1970-01-01T00:00:00.149Z");
    let variants = vec![
        (
            "created_submillis",
            1,
            None,
            None,
            None,
            Some("1970-01-01T00:00:00.1001Z"),
        ),
        (
            "second_period",
            1,
            None,
            None,
            Some((
                "\"1970-01-01T00:00:01.101Z\"^^<http://www.w3.org/2001/XMLSchema#dateTime>",
                "\"1970-01-01T00:00:02.201Z\"^^<http://www.w3.org/2001/XMLSchema#dateTime>",
            )),
            Some("1970-01-01T00:00:00Z"),
        ),
        (
            "complete_other_issuer",
            3,
            None,
            Some("revocation"),
            Some((from.as_str(), until.as_str())),
            Some("1970-01-01T00:00:00.101Z"),
        ),
        (
            "complete_no_status",
            3,
            None,
            None,
            Some((from.as_str(), until.as_str())),
            Some("1970-01-01T00:00:00.101Z"),
        ),
        (
            "blank_scope",
            1,
            Some(11),
            None,
            None,
            Some("1970-01-01T00:00:00Z"),
        ),
        ("base", 1, None, None, None, Some("1970-01-01T00:00:00Z")),
        (
            "missing_issuer",
            1,
            None,
            None,
            None,
            Some("1970-01-01T00:00:00Z"),
        ),
        (
            "issuer_two",
            2,
            None,
            None,
            None,
            Some("1970-01-01T00:00:00Z"),
        ),
        (
            "holder",
            1,
            Some(11),
            None,
            None,
            Some("1970-01-01T00:00:00Z"),
        ),
        (
            "holder_two",
            1,
            Some(12),
            None,
            None,
            Some("1970-01-01T00:00:00Z"),
        ),
        ("ambiguous_holder", 1, Some(11), None, None, None),
        ("bad_holder_type", 1, Some(11), None, None, None),
        (
            "validity",
            1,
            None,
            None,
            Some((from.as_str(), until.as_str())),
            Some("1970-01-01T00:00:00.100Z"),
        ),
        ("from_only", 1, None, None, Some((from.as_str(), "")), None),
        (
            "until_only",
            1,
            None,
            None,
            Some(("", until.as_str())),
            None,
        ),
        (
            "equal_bounds",
            1,
            None,
            None,
            Some((from.as_str(), equal.as_str())),
            None,
        ),
        (
            "reversed",
            1,
            None,
            None,
            Some((from.as_str(), reverse.as_str())),
            None,
        ),
        (
            "bad_until",
            1,
            None,
            None,
            Some((from.as_str(), "<urn:not-a-time>")),
            None,
        ),
        (
            "wrong_datatype",
            1,
            None,
            None,
            Some((
                from.as_str(),
                "\"201\"^^<http://www.w3.org/2001/XMLSchema#integer>",
            )),
            None,
        ),
        (
            "duplicate_until",
            1,
            None,
            None,
            Some((from.as_str(), until.as_str())),
            None,
        ),
        (
            "unrelated_bounds",
            1,
            None,
            None,
            None,
            Some("1970-01-01T00:00:00Z"),
        ),
        ("unknown_created", 1, None, None, None, None),
        (
            "created",
            1,
            None,
            None,
            None,
            Some("1970-01-01T00:00:00.101Z"),
        ),
        (
            "earlier_created",
            1,
            None,
            None,
            None,
            Some("1970-01-01T00:00:00.100Z"),
        ),
        (
            "additional_type",
            1,
            None,
            None,
            None,
            Some("1970-01-01T00:00:00Z"),
        ),
        ("named", 1, None, None, None, None),
        ("reserved", 1, None, None, None, None),
        ("status", 1, None, Some("revocation"), None, None),
        ("suspension", 1, None, Some("suspension"), None, None),
        ("two_status", 1, None, Some("revocation"), None, None),
        ("missing_index", 1, None, Some("revocation"), None, None),
        ("bad_index", 1, None, Some("revocation"), None, None),
        ("bad_purpose", 1, None, Some("other"), None, None),
        ("size_one", 1, None, Some("revocation"), None, None),
        ("size_two", 1, None, Some("revocation"), None, None),
        ("bad_status_type", 1, None, Some("revocation"), None, None),
        ("last_status", 1, None, Some("revocation"), None, None),
        (
            "last_until",
            1,
            None,
            None,
            Some(("", until.as_str())),
            None,
        ),
        (
            "complete",
            1,
            Some(11),
            Some("revocation"),
            Some((from.as_str(), until.as_str())),
            Some("1970-01-01T00:00:00.101Z"),
        ),
        (
            "complete_shared",
            2,
            Some(11),
            Some("suspension"),
            Some((from.as_str(), until.as_str())),
            Some("1970-01-01T00:00:00.101Z"),
        ),
        (
            "complete_distinct",
            1,
            Some(12),
            Some("suspension"),
            Some((from.as_str(), until.as_str())),
            Some("1970-01-01T00:00:00.101Z"),
        ),
        (
            "complete_unlisted",
            3,
            None,
            Some("revocation"),
            Some((from.as_str(), until.as_str())),
            Some("1970-01-01T00:00:00.101Z"),
        ),
        (
            "complete_old",
            3,
            None,
            Some("revocation"),
            Some((from.as_str(), until.as_str())),
            Some("1970-01-01T00:00:00.100Z"),
        ),
        (
            "complete_expired",
            3,
            None,
            Some("revocation"),
            Some((from.as_str(), late.as_str())),
            Some("1970-01-01T00:00:00.101Z"),
        ),
    ];
    for (name, issuer, holder, status, bounds, created) in variants {
        let mut config=format!("<urn:proof> <https://w3id.org/security#cryptosuite> \"schnorr-poseidon2-merkle-2026\" .\n<urn:proof> <https://w3id.org/security#verificationMethod> <urn:method:{issuer}> .\n");
        if let Some(t) = created {
            writeln!(
                config,
                "<urn:proof> <http://purl.org/dc/terms/created> {} .",
                time(t)
            )
            .unwrap();
        }
        let issuer_key = key(issuer);
        let mut document = document(name, issuer, holder, status, bounds);
        if name == "last_status" || name == "last_until" {
            let original = document.clone();
            let predicate = if name == "last_status" {
                "credentialStatus"
            } else {
                "validUntil"
            };
            let mut found = false;
            for nonce in 0..4096 {
                document =
                    original.replace("<urn:credential>", &format!("<urn:credential:{nonce}>"));
                let mut rows = oxttl::NQuadsParser::new()
                    .for_slice(document.as_bytes())
                    .map(Result::unwrap)
                    .collect::<Vec<_>>();
                rows.sort_by_key(|q| p::field_bytes(p::quad_hash(q).unwrap()));
                if rows.last().unwrap().predicate.as_str().ends_with(predicate) {
                    found = true;
                    break;
                }
            }
            assert!(found, "last-row fixture construction");
        }
        let credential =
            p::issue_deterministic(&document, &config, &issuer_key, &[31; 32]).unwrap();
        assert!(p::verify(&credential, &issuer_key.public_key()));
        let opened = p::verify_and_open(&credential, &issuer_key.public_key()).unwrap();
        let digest: [u8; 32] = Sha256::digest(credential.proof_config.as_bytes()).into();
        let message = p::signed_message(
            credential.salt,
            credential.count,
            credential.root,
            &digest,
            opened.digest,
        );
        writeln!(out,"pub fn {name}<let N:u32>() -> CredentialOpening<N,160> {{\nlet mut rows=[[absent::<160>();4];N];let mut salts=[0;N];").unwrap();
        for (i, q) in oxttl::NQuadsParser::new()
            .for_slice(credential.document.as_bytes())
            .map(Result::unwrap)
            .enumerate()
        {
            let graph = match q.graph_name {
                oxrdf::GraphName::DefaultGraph => {
                    "raw(0,\"\".as_bytes(),\"\".as_bytes(),\"\".as_bytes(),[0;31])".into()
                }
                oxrdf::GraphName::NamedNode(n) => cells.raw(Some(&Term::NamedNode(n))),
                _ => panic!("graph"),
            };
            writeln!(
                out,
                "rows[{i}]=[{},{},{},{}];salts[{i}]={};",
                cells.raw(Some(&Term::from(q.subject))),
                cells.raw(Some(&Term::NamedNode(q.predicate))),
                cells.raw(Some(&q.object)),
                graph,
                field_to_hex(&p::leaf_salt(credential.salt, i as u32))
            )
            .unwrap();
        }
        writeln!(out,"CredentialOpening {{raw:rows,salts,salt:{},count:{},configuration:[{},{}],created:{},method:{},pk:{},signature:{}}}\n}}",field_to_hex(&credential.salt),credential.count,u128::from_be_bytes(digest[..16].try_into().unwrap()),u128::from_be_bytes(digest[16..].try_into().unwrap()),cells.raw(opened.created.as_ref()),cells.raw(opened.verification_method.as_ref()),point(&issuer_key),signature(&issuer_key,&message)).unwrap();
        if name == "complete" {
            let member = TrustMember {
                issuer: oxrdf::NamedNode::new_unchecked("urn:issuer:1"),
                verification_method: match opened.verification_method.unwrap() {
                    Term::NamedNode(n) => n,
                    _ => panic!("method"),
                },
                key: issuer_key.public_key(),
            };
            writeln!(
                out,
                "pub fn issued_trust_digest() -> Field {{0x{}}}",
                trust_member_digest(&member)
                    .unwrap()
                    .map(|b| format!("{b:02x}"))
                    .concat()
            )
            .unwrap();
        }
    }
    let digest = [7u8; 32];
    let message = holder_message(&digest);
    writeln!(
        out,
        "pub fn holder_message_vector() -> Field {{{}}}",
        field_to_hex(&message)
    )
    .unwrap();
    for bit in [false, true] {
        writeln!(
            out,
            "pub fn status_leaf_{bit}_vector() -> Field {{{}}}",
            field_to_hex(&status_leaf(bit))
        )
        .unwrap();
    }
    let left = sparq_zk::poseidon2::hash(&[status_leaf(true), status_leaf(false)]);
    let right = sparq_zk::poseidon2::hash(&[status_leaf(false), status_leaf(true)]);
    let root = sparq_zk::poseidon2::hash(&[left, right]);
    for purpose in ["revocation", "suspension"] {
        let list = p::term_cell(
            &Term::NamedNode(oxrdf::NamedNode::new_unchecked(format!(
                "urn:list:{purpose}"
            ))),
            0,
        )
        .unwrap();
        let purpose_cell =
            p::term_cell(&Term::Literal(Literal::new_simple_literal(purpose)), 0).unwrap();
        let digest = snapshot_digest(
            p::field_bytes(list.enc),
            p::field_bytes(purpose_cell.enc),
            4,
            2,
            p::field_bytes(root),
        )
        .unwrap();
        writeln!(
            out,
            "pub fn snapshot_{purpose}_vector() -> Field {{0x{}}}",
            digest.map(|b| format!("{b:02x}")).concat()
        )
        .unwrap();
    }
    for n in [11, 12] {
        writeln!(
            out,
            "pub fn holder_signature_{n}() -> Signature {{{}}}",
            signature(&key(n), &message)
        )
        .unwrap();
    }
    // Pin the real literal constructors used by status snapshot identity controls.
    for purpose in ["revocation", "suspension", "other"] {
        writeln!(out,"pub fn purpose_{purpose}() -> envelope_core::term::TermCell {{envelope_core::suite_entry::open_cell::<160>({},0)}}",cells.raw(Some(&Term::Literal(Literal::new_simple_literal(purpose))))).unwrap();
    }
    out.push_str("pub fn empty_vec<let N:u32>() -> BoundedVec<u8,N> {BoundedVec::new()}\n");
    for (body, id) in cells.0 {
        writeln!(
            out,
            "fn term_{id}<let L:u32>() -> TermOpening<L> {{{body}}}"
        )
        .unwrap();
    }
    print!("{out}");
}
