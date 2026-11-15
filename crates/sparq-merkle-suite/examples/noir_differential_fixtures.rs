//! Emit the same raw witness fixtures that the live nargo integration executes.
#[path = "../tests/support/cases.rs"]
mod cases;
fn main() {
    let groups: Vec<_> = cases::fixtures().into_iter().map(|((datatype,bucket),inputs)| serde_json::json!({
        "datatype":datatype,"bucket":bucket,"source":cases::program(&datatype,bucket),"inputs":inputs,
    })).collect();
    println!("{}", serde_json::to_string(&groups).unwrap());
}
