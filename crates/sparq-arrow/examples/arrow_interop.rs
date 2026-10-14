// sq-vwnzh — thin, deliberately machine-readable Arrow interop driver.
#![cfg(feature = "arrow")]

use arrow_array::cast::AsArray;
use arrow_array::{Array, StringArray};
use sparq_arrow::{to_record_batch, FIELD_DATATYPE, FIELD_KIND, FIELD_LANGUAGE, FIELD_VALUE};
use sparq_core::Graph;
use sparq_engine::query;

fn field<'a>(column: &'a arrow_array::StructArray, name: &str) -> &'a StringArray {
    column.column_by_name(name).unwrap().as_string::<i32>()
}

fn main() {
    let mut args = std::env::args().skip(1);
    let data = std::fs::read_to_string(args.next().expect("usage: arrow_interop DATA QUERY"))
        .expect("read data");
    let query_text =
        std::fs::read_to_string(args.next().expect("missing query")).expect("read query");
    let graph = Graph::load_str(&data, "turtle").expect("parse data");
    let result = query(&graph, &query_text).expect("execute query");
    let batch = to_record_batch(&result).expect("export Arrow RecordBatch");

    for row in 0..batch.num_rows() {
        let mut cells = Vec::with_capacity(batch.num_columns());
        for column in batch.columns() {
            if column.is_null(row) {
                cells.push(String::new());
                continue;
            }
            let term = column.as_struct();
            let kind = field(term, FIELD_KIND).value(row);
            let value = field(term, FIELD_VALUE).value(row);
            let encoded = match kind {
                "uri" => format!("<{value}>"),
                "bnode" => format!("_:{value}"),
                "literal" => {
                    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
                    let lang = field(term, FIELD_LANGUAGE);
                    let datatype = field(term, FIELD_DATATYPE);
                    if !lang.is_null(row) {
                        format!("\"{escaped}\"@{}", lang.value(row))
                    } else if !datatype.is_null(row) {
                        format!("\"{escaped}\"^^<{}>", datatype.value(row))
                    } else {
                        format!("\"{escaped}\"")
                    }
                }
                other => panic!("unexpected Arrow term kind: {other}"),
            };
            cells.push(encoded);
        }
        println!("{}", cells.join("\t"));
    }
}
