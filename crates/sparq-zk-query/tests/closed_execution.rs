mod closed_support;
#[path = "../../sparq-zk-compose/tests/support/mod.rs"]
mod toolchain;

use closed_support::*;
use sparq_zk_query::{
    interp::{Cell, Table, Word},
    interp_closed_group::*,
    interp_closed_order::{order_by, OrderKey},
    Error,
};

// A deterministic test codec with an identical constrained Noir implementation.
// It is deliberately local to this fixture, not a suite term encoder.
struct Codec;
impl NumericCodec for Codec {
    fn types(&self) -> NumericTypes {
        IntegerCodec::new(Encoder).unwrap().types()
    }
    fn encode_numeric(&self, key: Word, decimal: bool) -> Result<Cell, Error> {
        let mut enc = key;
        if decimal {
            for b in enc.iter_mut().rev() {
                let (next, carry) = b.overflowing_add(1);
                *b = next;
                if !carry {
                    break;
                }
            }
        }
        Ok(Cell {
            bound: true,
            kind: 3,
            enc,
            key_class: 1,
            key,
            dt: if decimal {
                self.types().decimal_dt
            } else {
                self.types().integer_dt
            },
            ..Cell::default()
        })
    }
}

fn field(w: Word) -> String {
    format!(
        "\"0x{}\"",
        w.iter().map(|b| format!("{b:02x}")).collect::<String>()
    )
}
fn cell(c: Cell) -> String {
    format!("{{ bound = {}, kind = {}, enc = {}, key_class = {}, key = {}, dt = {}, lang = {}, scope = {} }}",
    c.bound, c.kind, field(c.enc), c.key_class, field(c.key), field(c.dt), field(c.lang), c.scope)
}
fn table(t: &Table, capacity: usize, width: usize) -> String {
    let mut rows: Vec<_> = t
        .rows
        .iter()
        .map(|r| {
            format!(
                "[{}]",
                r.iter().copied().map(cell).collect::<Vec<_>>().join(",")
            )
        })
        .collect();
    rows.resize(
        capacity,
        format!("[{}]", vec![cell(Cell::default()); width].join(",")),
    );
    format!("{{ rows = [{}], len = {} }}", rows.join(","), t.rows.len())
}
fn package(source: &str) -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join("src")).unwrap();
    let core = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../zk/noir-query/core")
        .canonicalize()
        .unwrap();
    std::fs::write(tmp.path().join("Nargo.toml"), format!("[package]\nname = \"closed_fixture\"\ntype = \"bin\"\nauthors = [\"sparq\"]\ncompiler_version = \">=1.0.0\"\n[dependencies]\ncore = {{ path = {:?} }}\n", core)).unwrap();
    std::fs::write(tmp.path().join("src/main.nr"), source).unwrap();
    tmp
}
fn execute(
    gate: &toolchain::LiveToolchain,
    root: &std::path::Path,
    witness: &str,
) -> std::process::Output {
    std::fs::write(root.join("Prover.toml"), witness).unwrap();
    toolchain::live_tool(gate, "nargo")
        .current_dir(root)
        .arg("execute")
        .output()
        .unwrap()
}

#[test]
fn host_group_and_order_witnesses_execute_and_reject_altered_cells() {
    let Some(gate) = toolchain::gate(
        "host_group_and_order_witnesses_execute_and_reject_altered_cells",
        &["nargo"],
    ) else {
        return;
    };
    let input = canonical(
        engine(
            "",
            "SELECT ?g ?x { VALUES (?g ?x) { (1 1) (1 4) (2 2) (2 2) } }",
            &["g", "x"],
        )
        .rows,
    );
    let kinds = [
        AggregateKind::CountAll,
        AggregateKind::Count,
        AggregateKind::Sum,
        AggregateKind::Min,
        AggregateKind::Max,
        AggregateKind::Avg,
        AggregateKind::Sample,
    ];
    let spec = GroupSpec {
        input_width: 2,
        columns: vec![0],
        aggregates: kinds
            .into_iter()
            .map(|kind| Aggregate {
                kind,
                column: (kind != AggregateKind::CountAll).then_some(1),
                distinct: false,
            })
            .collect(),
        having: None,
        capacity: 4,
    };
    let output = group(&input, &errors(&input, 7), &spec, &Codec).unwrap();
    let types = Codec.types();
    let source = format!("use core::table::{{TermCell, Table}};\nuse core::ops_closed_group::{{group, Aggregate, NumericTypes, Having}};\n\
        fn main(input: Table<4,2>, out: Table<4,8>) {{\n\
        let types = NumericTypes {{ integer_dt: {}, decimal_dt: {} }};\n\
        let encode = |key: Field, decimal: bool| TermCell {{ bound: true, kind: 3, enc: key + decimal as Field, key_class: 1, key, dt: if decimal {{ types.decimal_dt }} else {{ types.integer_dt }}, lang: 0, scope: 0 }};\n\
        group(input, out, [0], [{}], [[false;7];4], types, Having {{ enabled: false, column: 0, comparison: 0, rhs: core::table::unbound() }}, encode);\n}}\n",
        field(types.integer_dt).trim_matches('"'), field(types.decimal_dt).trim_matches('"'),
        (0..7).map(|kind| format!("Aggregate {{ kind: {kind}, column: {}, distinct: false }}", if kind == 0 { 2 } else { 1 })).collect::<Vec<_>>().join(","));
    let tmp = package(&source);
    let witness = |out: &Table| {
        format!(
            "input = {}\nout = {}\n",
            table(&input, 4, 2),
            table(out, 4, 8)
        )
    };
    let honest = execute(&gate, tmp.path(), &witness(&output));
    assert!(
        honest.status.success(),
        "{}",
        String::from_utf8_lossy(&honest.stderr)
    );
    for column in 1..8 {
        let mut altered = output.clone();
        altered.rows[0][column].enc[31] ^= 1;
        let rejected = execute(&gate, tmp.path(), &witness(&altered));
        assert!(
            !rejected.status.success(),
            "altered aggregate column {column}"
        );
        assert!(String::from_utf8_lossy(&rejected.stderr).contains("operator rows"));
    }
    let keys = [
        OrderKey {
            column: 1,
            descending: true,
        },
        OrderKey {
            column: 0,
            descending: false,
        },
    ];
    let output = order_by(&input, 2, &keys, 4).unwrap();
    let tmp = package("use core::table::Table;\nuse core::ops_closed_order::{order_by, OrderKey};\nfn main(input: Table<4,2>, out: Table<4,2>) { order_by(input, out, [OrderKey { column: 1, descending: true }, OrderKey { column: 0, descending: false }]); }\n");
    let witness = |out: &Table| {
        format!(
            "input = {}\nout = {}\n",
            table(&input, 4, 2),
            table(out, 4, 2)
        )
    };
    let honest = execute(&gate, tmp.path(), &witness(&output));
    assert!(
        honest.status.success(),
        "{}",
        String::from_utf8_lossy(&honest.stderr)
    );
    let mut altered = output.clone();
    altered.rows.swap(0, 3);
    let rejected = execute(&gate, tmp.path(), &witness(&altered));
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("ORDER BY rows"));
    toolchain::complete(gate);
}
