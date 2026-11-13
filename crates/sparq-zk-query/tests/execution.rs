#[path = "support/mod.rs"]
mod fixture_support;
#[path = "../../sparq-zk-compose/tests/support/mod.rs"]
mod support;

use fixture_support::{fixture, FixtureEncoder};
use sparq_zk_compose::driver::ProgramProver;
use sparq_zk_query::{
    capacity::Bounds,
    codegen::generate,
    interp::{evaluate, ResultValue},
    lower::lower,
    witness::prover_toml,
};

struct Workspace(std::path::PathBuf);
impl Workspace {
    fn new(package: &sparq_zk_query::codegen::Package) -> Self {
        let root = std::env::temp_dir().join(format!(
            "sparq_query_execution_{}_{}",
            std::process::id(),
            package.name
        ));
        std::fs::create_dir(&root).unwrap();
        for (name, source) in &package.files {
            let p = root.join(name);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, source).unwrap();
        }
        let core = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../zk/noir-query/core")
            .canonicalize()
            .unwrap();
        std::os::unix::fs::symlink(core, root.join("core")).unwrap();
        Self(root)
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn generated_driver_refuses_path_packages_before_io() {
    for name in [
        "",
        ".",
        "..",
        "../query",
        "a/b",
        "--query",
        "1query",
        "query.name",
        "query-name",
        "q%00",
    ] {
        assert!(ProgramProver::new("/unused", name).is_err(), "{name:?}");
    }
    assert!(ProgramProver::new("/unused", "query_abc").is_ok());
}

#[test]
#[ignore = "requires the pinned native query toolchain"]
fn generated_program_executes_trace_and_refuses_changed_rows_and_results() {
    let Some(gate) = support::gate(
        "generated_program_executes_trace_and_refuses_changed_rows_and_results",
        &["nargo", "bb"],
    ) else {
        return;
    };
    let plan = lower(
        "SELECT ?s WHERE {?s <urn:p> ?x FILTER(?x >= 1 && ?x < 3)}",
        Bounds {
            credentials: 1,
            quads_per_credential: 4,
            rows_per_table: 4,
        },
    )
    .unwrap();
    let data = fixture();
    let trace = evaluate(&plan, &data, &FixtureEncoder).unwrap();
    let package = generate(&plan).unwrap();
    let workspace = Workspace::new(&package);
    let driver = ProgramProver::new(&workspace.0, &package.name).unwrap();
    let input = prover_toml(&plan, &data, &FixtureEncoder, &trace, [7; 32]).unwrap();
    driver.execute(&input, "honest").unwrap();
    for (i, t) in trace.tables.iter().enumerate() {
        for (r, row) in t.rows.iter().enumerate() {
            for v in 0..row.len() {
                let mut altered = trace.clone();
                altered.tables[i].rows[r][v].enc[31] ^= 1;
                let input = prover_toml(&plan, &data, &FixtureEncoder, &altered, [7; 32]).unwrap();
                assert!(
                    driver.execute(&input, "changed_table").is_err(),
                    "table {i}, row {r}, column {v}"
                );
            }
        }
    }
    let mut altered = trace.clone();
    if let ResultValue::Select { rows, .. } = &mut altered.result {
        rows[0][0].enc[31] ^= 1;
    } else {
        panic!("SELECT")
    }
    let input = prover_toml(&plan, &data, &FixtureEncoder, &altered, [7; 32]).unwrap();
    assert!(driver.execute(&input, "changed_result").is_err());
    support::complete(gate);
}
