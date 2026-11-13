//! Live checks against lane A's immutable source snapshot, without merging its files.
use super::expr_test_support::Scratch;
use std::path::Path;
use std::process::Command;

const SUITE_COMMIT: &str = "6c44aa1db6ee99fa7dcabacf8971f5cbc9d576b1";
fn snapshot_file(name: &str) -> Vec<u8> {
    let file = format!("zk/noir-query/core/src/{name}");
    let local = Command::new("git")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args(["show", &format!("{SUITE_COMMIT}:{file}")])
        .output()
        .unwrap();
    if local.status.success() {
        return local.stdout;
    }
    let url = format!("https://raw.githubusercontent.com/sparq-org/sparq/{SUITE_COMMIT}/{file}");
    let remote = Command::new("curl")
        .args(["--fail", "--silent", "--show-error", "--location", &url])
        .output()
        .expect("read pinned suite source");
    assert!(
        remote.status.success(),
        "{}",
        String::from_utf8_lossy(&remote.stderr)
    );
    remote.stdout
}
const ADAPTER: &str = r#"
use std::collections::bounded_vec::BoundedVec;
use expressions::expr_core::{Lane, Scalar, lane, exact, SCALE, INTEGER, DECIMAL, truth, logical_and, as_numeric};
use expressions::expr_compare::{value_compare, numeric_compare, in_list, EQ, GE, LE};
use expressions::expr_arith::{binary, MUL};
use expressions::expr_term_contract::{TermCell, CheckedLexicalOpening, LexicalOpening, DT_INTEGER, DT_DECIMAL, DT_DATE, DT_BOOLEAN, DT_STRING};
use expressions::expr_to_term::{ComputedTermEncoder, numeric_lexical};
struct Encoder {}
fn convert(c:credential_terms::term::TermCell)->TermCell {
 TermCell {bound:c.bound(),kind:c.kind(),enc:c.enc(),key_class:c.key_class(),key:c.key(),dt:c.datatype(),lang:c.language(),credential:c.credential()}
}
impl ComputedTermEncoder for Encoder {
 fn literal(lexical:BoundedVec<u8,64>,datatype:BoundedVec<u8,64>,key:[u8;31])->TermCell {
  convert(credential_terms::term::open_term(3,lexical,datatype,BoundedVec::<u8,0>::new(),key,0))
 }
}
fn narrow(value:BoundedVec<u8,64>)->BoundedVec<u8,32> {assert(value.len()<=32,"sample literal bucket");let mut out=BoundedVec::new();for i in 0..32 {if i<value.len() {out.push(value.get_unchecked(i));}}out}
struct Opening {cell:credential_terms::term::TermCell,lexical:BoundedVec<u8,32>,datatype:BoundedVec<u8,128>,expected_dt:Field}
impl CheckedLexicalOpening<32,128,64> for Opening {
 fn open_lexical(self)->LexicalOpening<32,128,64> {
  let empty=BoundedVec::<u8,64>::new();
  if self.cell.bound() {checked_key(self.cell,self.lexical,self.datatype,self.expected_dt);}
  else {self.cell.assert_padding();}
  LexicalOpening {cell:convert(self.cell),lexical:self.lexical,datatype:self.datatype,language:empty}
 }
}
struct IriOpening {cell:credential_terms::term::TermCell,lexical:BoundedVec<u8,128>}
impl CheckedLexicalOpening<128,128,64> for IriOpening {
 fn open_lexical(self)->LexicalOpening<128,128,64> {
  assert_eq(self.cell.kind(),1);
  let datatype=BoundedVec::<u8,128>::new();let language=BoundedVec::<u8,64>::new();
  credential_terms::term::open_lexical(self.cell,self.lexical,datatype,language);
  LexicalOpening {cell:convert(self.cell),lexical:self.lexical,datatype,language}
 }
}
// Literal expected datatypes remain fixed at each suite parser call.
fn checked_key<let L:u32,let D:u32>(c:credential_terms::term::TermCell,l:BoundedVec<u8,L>,d:BoundedVec<u8,D>,dt:Field) {
 let g=BoundedVec::<u8,64>::new();
 if dt==DT_INTEGER {let _=credential_terms::term::recompute_key_for(c,l,d,g,DT_INTEGER);}
 else if dt==DT_DECIMAL {let _=credential_terms::term::recompute_key_for(c,l,d,g,DT_DECIMAL);}
 else if dt==DT_BOOLEAN {let _=credential_terms::term::recompute_key_for(c,l,d,g,DT_BOOLEAN);}
 else if dt==DT_STRING {let _=credential_terms::term::recompute_key_for(c,l,d,g,DT_STRING);}
 else {assert_eq(dt,DT_DATE);let _=credential_terms::term::recompute_key_for(c,l,d,g,DT_DATE);}
}
fn bytes<let N:u32>(s:[u8;N])->BoundedVec<u8,32> {let mut b=BoundedVec::new();for c in s {b.push(c);}b}
fn metadata<let N:u32>(s:[u8;N])->BoundedVec<u8,128> {let mut b=BoundedVec::new();for c in s {b.push(c);}b}
fn widened(input:(Lane<Scalar<32>>,bool))->(Lane<Scalar<128>>,bool) {
 let (v,e)=input;let s=v.value;let mut bytes=[0;128];for i in 0..32 {bytes[i]=s.string.bytes[i];}
 (Lane {value:Scalar {tag:s.tag,valid:s.valid,boolean:s.boolean,number:s.number,string:expressions::expr_core::StringLane {bytes,len:s.string.len,lang:s.string.lang},temporal:s.temporal,term:s.term},capacity:v.capacity},e)
}
fn opened(lexical:BoundedVec<u8,32>,datatype:BoundedVec<u8,128>,key:Field,expected_dt:Field)->(Lane<Scalar<128>>,bool) {
 let cell=credential_terms::term::open_term(3,lexical,datatype,BoundedVec::<u8,64>::new(),key.to_be_bytes::<31>(),0);
 widened(expressions::expr_open::variable(Opening {cell,lexical,datatype,expected_dt}))
}
fn number(magnitude:Field,decimal:bool)->(Lane<Scalar<128>>,bool) {
 let n=exact(if decimal {DECIMAL}else{INTEGER},false,magnitude);
 let key:Field=0x1800000000000000000000000000000000000000000000000000000000000+magnitude;
 if decimal {opened(narrow(numeric_lexical(n)),metadata("http://www.w3.org/2001/XMLSchema#decimal".as_bytes()),key,DT_DECIMAL)}
 else {opened(narrow(numeric_lexical(n)),metadata("http://www.w3.org/2001/XMLSchema#integer".as_bytes()),key,DT_INTEGER)}
}
fn integer(value:Field)->(Lane<Scalar<128>>,bool) {number(value*SCALE,false)}
fn boolean(value:bool)->(Lane<Scalar<128>>,bool) {
 let lexical=if value {bytes("true".as_bytes())}else{bytes("false".as_bytes())};
 let key:Field=0x5000000000000000000000000000000000000000000000000000000000000+if value {0x10000000000000000000000000000000000000000000000000000000000}else{0};
 opened(lexical,metadata("http://www.w3.org/2001/XMLSchema#boolean".as_bytes()),key,DT_BOOLEAN)
}
fn text<let N:u32>(value:[u8;N])->(Lane<Scalar<128>>,bool) {
 let mut payload:Field=0;for i in 0..30 {payload=payload*256+if i<N {value[i] as Field}else{0};}
 let key=(if N>30 {7}else{6}) as Field*0x1000000000000000000000000000000000000000000000000000000000000+payload;
 opened(bytes(value),metadata("http://www.w3.org/2001/XMLSchema#string".as_bytes()),key,DT_STRING)
}
fn iri(value:Field)->(Lane<Scalar<128>>,bool) {
 let lexical=if value==1 {metadata("urn:fictional:recycled-content:tier".as_bytes())}
 else if value==2 {metadata("urn:fictional:recycled-content:tier-2".as_bytes())}
 else if value==3 {metadata("urn:fictional:recycled-content:tier-3".as_bytes())}
 else {metadata("urn:other".as_bytes())};
 let cell=credential_terms::term::open_term(1,lexical,BoundedVec::<u8,128>::new(),BoundedVec::<u8,64>::new(),[0;31],0);
 expressions::expr_open::variable(IriOpening {cell,lexical})
}
fn date(day:u64)->(Lane<Scalar<128>>,bool) {
 let lexical=if day==18944 {"2021-11-13".as_bytes()}else if day==18945 {"2021-11-14".as_bytes()}
 else if day==18943 {"2021-11-12".as_bytes()}else if day==20758 {"2026-11-01".as_bytes()}
 else if day==20759 {"2026-11-02".as_bytes()}else if day==20769 {"2026-11-12".as_bytes()}else {"2026-11-13".as_bytes()};
 let key:Field=0x8000000000000000000000000000000000000000000000000000000000000
  +(0x8000000000000000+day as Field)*0x100000000000000000000000000000000000000000000;
 opened(bytes(lexical),metadata("http://www.w3.org/2001/XMLSchema#date".as_bytes()),key,DT_DATE)
}
fn missing()->(Lane<Scalar<128>>,bool) {
 widened(expressions::expr_open::variable(Opening {cell:credential_terms::term::padding(),lexical:BoundedVec::new(),datatype:BoundedVec::new(),expected_dt:0}))
}
fn bind(result:(Lane<bool>,bool),expected:bool,error:bool) {
 check(result,expected,error);
 let (cell,e)=expressions::expr_to_term::boolean::<Encoder>(result);assert_eq(e,error);assert_eq(cell.value.bound(),!error);
 if !error {let (expected,_)=boolean(expected);assert_eq(cell.value.enc(),expected.value.term.enc());assert_eq(cell.value.key(),expected.value.term.key());}
 else {assert_eq(cell.value.enc(),0);assert_eq(cell.value.key(),0);}
}
"#;
#[test]
fn noir_target_queries_match_host_mirror() {
    let Some(gate) =
        crate::support::gate("noir_target_queries_match_host_mirror", &["nargo", "bb"])
    else {
        return;
    };
    for name in ["nargo", "bb"] {
        let version = crate::support::live_tool(&gate, name)
            .arg("--version")
            .output()
            .unwrap();
        assert!(version.status.success());
        assert!(super::expr_tests::toolchain::pinned_version_output(
            name,
            &version.stdout
        ));
    }
    let core = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../zk/noir-query/core")
        .canonicalize()
        .unwrap();
    let compose = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../zk/compose/compose_core")
        .canonicalize()
        .unwrap();
    let scratch = Scratch::new("sparq_noir_targets");
    for sub in ["src", "suite/src"] {
        std::fs::create_dir_all(scratch.0.join(sub)).unwrap();
    }
    for file in [
        "term.nr",
        "term_key.nr",
        "term_float.nr",
        "suite_constants.nr",
        "term_bytes.nr",
        "term_ids.nr",
    ] {
        std::fs::write(scratch.0.join("suite/src").join(file), snapshot_file(file)).unwrap();
    }
    std::fs::write(
        scratch.0.join("suite/src/lib.nr"),
        "pub mod term;\nmod term_key;\nmod term_float;\nmod term_bytes;\nmod term_ids;\npub mod suite_constants;\n",
    )
    .unwrap();
    std::fs::write(scratch.0.join("suite/Nargo.toml"),format!("[package]\nname=\"nd_credential_snapshot\"\ntype=\"lib\"\n[dependencies]\nposeidon={{git=\"https://github.com/noir-lang/poseidon\",tag=\"v0.3.0\"}}\ncompose={{path={:?}}}\n",compose)).unwrap();
    std::fs::write(scratch.0.join("Nargo.toml"),format!("[package]\nname=\"nd_target_samples\"\ntype=\"lib\"\n[dependencies]\nexpressions={{path={:?}}}\ncredential_terms={{path=\"suite\"}}\n",core)).unwrap();
    let cases = include_str!("../../../../zk/noir-query/core/src/expr_target_tests.nr")
        .split_once("// CASES:")
        .unwrap()
        .1;
    let helpers = cases[cases.find("fn all").unwrap()..cases.find("#[test]").unwrap()].to_owned();
    let tests = r#"
#[test] fn target_1a_actual_openings_and_binding() {
 bind(stated(integer(12),integer(0),integer(0),boolean(true)),true,false);
 bind(stated(missing(),integer(0),integer(0),boolean(true)),false,true);
}
#[test] fn target_1b_actual_openings_and_binding() {
 bind(closed(integer(12),integer(0),date(18944),date(20770),truth(true,false)),true,false);
 bind(closed(integer(12),integer(0),missing(),date(20770),truth(true,false)),false,true);
}
#[test] fn target_2_actual_openings_and_binding() {
 bind(trial(integer(45),text("condition-a".as_bytes()),date(20758),date(20770),truth(true,false)),true,false);
 check(in_list(text("excluded-x".as_bytes()),[text("excluded-x".as_bytes()),text("excluded-y".as_bytes())]),true,false);
 bind(trial(integer(45),missing(),date(20758),date(20770),truth(true,false)),false,true);
}
#[test] fn target_3_actual_openings_and_binding() {
 bind(recycled(number(350000000000000000,true),integer(3),integer(3)),true,false);
 check(in_list(iri(2),[iri(1),iri(2),iri(3)]),true,false);
 bind(recycled(missing(),integer(3),integer(3)),false,true);
}
#[test] fn target_sum_actual_openings_and_binding() {
 bind(totals(integer(500),integer(3)),true,false);
 bind(totals(missing(),integer(3)),false,true);
 check(numeric_compare(binary(as_numeric(integer(150)),as_numeric(integer(10)),MUL),binary(as_numeric(integer(500)),as_numeric(integer(3)),MUL),GE),true,false);
}
#[test] fn numeric_terms_match_real_constructor() {
 let values=[exact(INTEGER,false,281474976710656*SCALE),exact(INTEGER,true,42*SCALE),exact(DECIMAL,false,SCALE+1),exact(DECIMAL,true,300000000000000000)];
 for n in values {
  let (cell,e)=expressions::expr_to_term::numeric::<Encoder>((lane(n),false));assert(!e);assert(!cell.capacity);
  let datatype=if n.ty==INTEGER {metadata("http://www.w3.org/2001/XMLSchema#integer".as_bytes())}else{metadata("http://www.w3.org/2001/XMLSchema#decimal".as_bytes())};
  let lexical=numeric_lexical(n);
  let actual=credential_terms::term::open_term(3,lexical,datatype,BoundedVec::<u8,0>::new(),cell.value.key().to_be_bytes::<31>(),0);
  checked_key(actual,narrow(lexical),datatype,if n.ty==INTEGER {DT_INTEGER}else{DT_DECIMAL});
  assert_eq(cell.value.enc(),actual.enc());
 }
}
#[test] fn canonical_boolean_is_distinct_from_alias() {
 let (computed,e)=expressions::expr_to_term::boolean::<Encoder>(truth(true,false));assert(!e);
 let alias=credential_terms::term::open_term(3,bytes("1".as_bytes()),metadata("http://www.w3.org/2001/XMLSchema#boolean".as_bytes()),BoundedVec::<u8,0>::new(),computed.value.key().to_be_bytes::<31>(),0);
 assert_eq(computed.value.key(),alias.key());assert(computed.value.enc()!=alias.enc(),"canonical lexical identity");
}
#[test(should_fail_with="computed encoding changed")]
fn computed_encoding_changed() {
 let (computed,_)=expressions::expr_to_term::boolean::<Encoder>(truth(true,false));
 let other=credential_terms::term::open_term(3,bytes("false".as_bytes()),metadata("http://www.w3.org/2001/XMLSchema#boolean".as_bytes()),BoundedVec::<u8,0>::new(),(0x5000000000000000000000000000000000000000000000000000000000000 as Field).to_be_bytes::<31>(),0);
 assert_eq(computed.value.enc(),other.enc(),"computed encoding changed");
}
#[test(should_fail_with="computed key changed")]
fn computed_key_changed() {
 let (computed,_)=expressions::expr_to_term::numeric::<Encoder>((lane(exact(INTEGER,false,5*SCALE)),false));
 assert_eq(computed.value.key(),0x1800000000000000000000000000000000000000000000000000000000000+6*SCALE,"computed key changed");
}
#[test(should_fail_with="key disagrees with lexical form")]
fn changed_checked_numeric_key() {
 let lexical=bytes("5".as_bytes());let datatype=metadata("http://www.w3.org/2001/XMLSchema#integer".as_bytes());
 let _=opened(lexical,datatype,0x1800000000000000000000000000000000000000000000000000000000000+6*SCALE,DT_INTEGER);
}
#[test(should_fail_with="lexical opening differs")]
fn changed_opened_bytes() {
 let lexical=bytes("5".as_bytes());let datatype=metadata("http://www.w3.org/2001/XMLSchema#integer".as_bytes());
 let c=credential_terms::term::open_term(3,lexical,datatype,BoundedVec::<u8,0>::new(),(0x1800000000000000000000000000000000000000000000000000000000000+5*SCALE).to_be_bytes::<31>(),0);
 let _=expressions::expr_open::variable(Opening {cell:c,lexical:bytes("6".as_bytes()),datatype,expected_dt:DT_INTEGER});
}
"#;
    std::fs::write(
        scratch.0.join("src/lib.nr"),
        format!("{ADAPTER}\n{helpers}\n{tests}"),
    )
    .unwrap();
    // Separate compiler processes bound retained compiler memory while preserving
    // every positive case and every negative control.
    for name in [
        "canonical_boolean_is_distinct_from_alias",
        "changed_checked_numeric_key",
        "changed_opened_bytes",
        "computed_encoding_changed",
        "computed_key_changed",
        "numeric_terms_match_real_constructor",
        "target_1a_actual_openings_and_binding",
        "target_1b_actual_openings_and_binding",
        "target_2_actual_openings_and_binding",
        "target_3_actual_openings_and_binding",
        "target_sum_actual_openings_and_binding",
    ] {
        let out = crate::support::live_tool(&gate, "nargo")
            .args([
                "test",
                "--exact",
                name,
                "--test-threads",
                "1",
                "--program-dir",
            ])
            .arg(&scratch.0)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{name}, status: {}\n{}{}",
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            String::from_utf8_lossy(&out.stdout).contains("1 test passed"),
            "{name}: {}",
            String::from_utf8_lossy(&out.stdout)
        );
        eprintln!("Noir target sample {name} completed");
    }
    crate::support::complete(gate);
}
