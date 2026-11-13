//! Byte-deterministic gadget wiring. No constraint expressions are emitted here.

use crate::{plan::*, Error};
use std::collections::BTreeMap;
use std::fmt::Write;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Package {
    pub name: String,
    /// Relative paths and UTF-8 source, in deterministic path order.
    pub files: BTreeMap<String, String>,
}

/// Generate the query program with complete suite authentication.
pub fn generate(plan: &Plan) -> Result<Package, Error> {
    generate_inner(plan, true)
}

/// Operator-relation fixture. It has no credential-authentication claim.
pub fn generate_operator_fixture(plan: &Plan) -> Result<Package, Error> {
    generate_inner(plan, false)
}

fn generate_inner(plan: &Plan, authenticated: bool) -> Result<Package, Error> {
    let hash: String = plan.hash().iter().map(|b| format!("{b:02x}")).collect();
    let name = format!("query_{hash}");
    let mut source = String::from(
        "// Research query program; the selected library entry point defines its credential contract.\n\
         use core::table::{TermCell, Table};\n\
         use core::{ops_core, ops_core_statement};\n\nfn main(\n",
    );
    if plan
        .nodes
        .iter()
        .any(|n| matches!(n.operator, Operator::Filter(..)))
    {
        source = source.replacen("\nfn main(", "\nuse core::ops_core_expr;\nfn main(", 1);
    }
    if plan
        .nodes
        .iter()
        .any(|n| matches!(n.operator, Operator::Scan(..) | Operator::Values(..)))
    {
        source = source.replacen("\nfn main(", "\nuse core::table::unbound;\nfn main(", 1);
    }
    let term_ids: Vec<_> = plan
        .parameters
        .iter()
        .enumerate()
        .filter_map(|(i, p)| matches!(p, Parameter::Term(_)).then_some(i as u32))
        .collect();
    let index_ids: Vec<_> = plan
        .parameters
        .iter()
        .enumerate()
        .filter_map(|(i, p)| matches!(p, Parameter::Index(_)).then_some(i as u32))
        .collect();
    let term = |p: u32| -> String {
        format!(
            "parameters[{}]",
            term_ids
                .iter()
                .position(|&id| id == p)
                .expect("term parameter")
        )
    };
    let index = |p: u32| -> String {
        format!(
            "indices[{}]",
            index_ids
                .iter()
                .position(|&id| id == p)
                .expect("index parameter")
        )
    };
    let cap = plan.capacity;
    let lexical = crate::credential::LEXICAL_CAPACITY;
    let layout = crate::statement::Layout::new(plan);
    source.push_str(&if authenticated {
        layout.declarations_authenticated()
    } else {
        layout.declarations()
    });
    if authenticated {
        for (c, n) in plan.credential_quad_capacities().into_iter().enumerate() {
            writeln!(source,"    credential_{c}: [[core::suite_entry::TermOpening<{lexical}>; 4]; {n}],\n    credential_proof_{c}: core::ops_core_suite::CredentialProof<{n}>,").unwrap();
        }
        writeln!(
            source,
            "    counts: [u32; {}],\n    credential_count: u32,\n    presentation_salt: [u8; 32],",
            cap.credentials
        )
        .unwrap();
    } else {
        writeln!(source,"    credentials: [[[TermCell; 4]; {}]; {}],\n    counts: [u32; {}],\n    credential_count: u32,\n    presentation_salt: [u8; 32],",cap.quads_per_credential,cap.credentials,cap.credentials).unwrap();
    }
    for (i, node) in plan.nodes.iter().enumerate() {
        writeln!(
            source,
            "    t{i}: Table<{}, {}>,",
            node.output.capacity,
            node.output.abi_width()
        )
        .unwrap();
    }
    writeln!(source,") {{\n    ops_core::check_parameters(parameters, {});\n    ops_core::check_indices(indices, {});",term_ids.len(),index_ids.len()).unwrap();
    let dataset_name = if plan
        .nodes
        .iter()
        .any(|n| matches!(n.operator, Operator::Scan(_)))
    {
        "dataset"
    } else {
        "_dataset"
    };
    if authenticated {
        let capacities = plan.credential_quad_capacities();
        let total: u32 = capacities.iter().sum();
        writeln!(source,"    let credential_flags = core::ops_core_suite::credential_flags::<{}>(credential_count, bounds[0], signature_mode);",cap.credentials).unwrap();
        let mut offset = 0;
        writeln!(
            source,
            "    let dataset_start = core::ops_core_suite::empty_dataset::<{total}>();"
        )
        .unwrap();
        for (c, n) in capacities.iter().enumerate() {
            writeln!(source,"    let opened_{c} = core::ops_core_suite::open_one(credential_{c}, credential_proof_{c}, counts[{c}], credential_flags[{c}], credential_quad_bounds[{c}], {c}, issuer_root);").unwrap();
            let prior = if c == 0 {
                "dataset_start".to_owned()
            } else {
                format!("dataset_{}", c - 1)
            };
            writeln!(source,"    let dataset_{c} = core::ops_core_suite::append_credential({prior}, opened_{c}, {offset});").unwrap();
            offset += n;
        }
        writeln!(
            source,
            "    let {dataset_name} = dataset_{};",
            capacities.len() - 1
        )
        .unwrap();
        let messages = array((0..capacities.len()).map(|c| format!("opened_{c}.message")));
        writeln!(source,"    core::ops_core_statement::check_suite_dataset({messages}, credential_count, presentation_salt, input_commitment);").unwrap();
    } else {
        writeln!(source,"    let {dataset_name} = ops_core::open_dataset::<{}, {}, {}>(credentials, counts, credential_count, bounds);",cap.credentials,cap.quads_per_credential,cap.credentials*cap.quads_per_credential).unwrap();
        source.push_str("    ops_core_statement::check_dataset(credentials, counts, credential_count, presentation_salt, input_commitment, signature_mode);\n");
    }
    for (i, node) in plan.nodes.iter().enumerate() {
        let schema = &node.output.variables;
        let columns = |id: NodeId| -> String {
            let input = &plan.nodes[id as usize].output.variables;
            if schema.is_empty() {
                return format!("[{}]", input.len().max(1));
            }
            array(schema.iter().map(|v| {
                input
                    .iter()
                    .position(|a| a == v)
                    .unwrap_or(input.len().max(1))
                    .to_string()
            }))
        };
        match &node.operator {
            Operator::Unit => writeln!(source, "    ops_core::unit(t{i});").unwrap(),
            Operator::Scan(slots) => {
                let mapping = array(slots.iter().map(|slot| {
                    match slot {
                        Slot::Variable(v) => schema
                            .iter()
                            .position(|a| a == v)
                            .expect("scan schema")
                            .to_string(),
                        Slot::Parameter(_) => node.output.abi_width().to_string(),
                    }
                }));
                let constants = array(slots.iter().map(|slot| match slot {
                    Slot::Variable(_) => "unbound()".into(),
                    Slot::Parameter(p) => term(*p),
                }));
                writeln!(source,"    ops_core::scan(dataset.rows, dataset.active, {mapping}, {constants}, t{i});").unwrap();
            }
            Operator::Join(a, b) => writeln!(
                source,
                "    ops_core::join(t{a}, t{b}, {}, {}, t{i});",
                columns(*a),
                columns(*b)
            )
            .unwrap(),
            Operator::Project(a) => writeln!(
                source,
                "    ops_core::project(t{a}, {}, t{i});",
                columns(*a)
            )
            .unwrap(),
            Operator::Distinct(a) => {
                writeln!(source, "    ops_core::distinct(t{a}, t{i});").unwrap()
            }
            Operator::Filter(a, expr) => {
                let mut steps = vec![];
                expression(
                    expr,
                    &plan.nodes[*a as usize].output.variables,
                    &term_ids,
                    &mut steps,
                );
                writeln!(
                    source,
                    "    let predicate_{i} = ops_core_expr::evaluate(t{a}, parameters, {});",
                    array(steps.iter().map(Step::source))
                )
                .unwrap();
                writeln!(source, "    ops_core::filter(t{a}, predicate_{i}, t{i});").unwrap();
            }
            Operator::Slice {
                input,
                offset,
                limit,
            } => writeln!(
                source,
                "    ops_core::slice(t{input}, {}, {}, {}, t{i});",
                index(*offset),
                limit.is_some(),
                limit.map(&index).unwrap_or_else(|| "0".into())
            )
            .unwrap(),
            Operator::Values(rows) => writeln!(
                source,
                "    ops_core::values({}, t{i});",
                array(rows.iter().map(|r| {
                    if r.is_empty() {
                        return "[unbound()]".into();
                    }
                    array(
                        r.iter()
                            .map(|p| p.map(&term).unwrap_or_else(|| "unbound()".into())),
                    )
                }))
            )
            .unwrap(),
        }
        writeln!(source, "    ops_core::check_bound(t{i}, bounds[2]);").unwrap();
    }
    let result = plan.result;
    match plan.form {
        ResultForm::Ask => writeln!(source, "    ops_core::ask(t{result}, result);").unwrap(),
        ResultForm::Select => writeln!(source, "    ops_core::select(t{result}, result);").unwrap(),
    }
    let binder = match plan.form {
        ResultForm::Ask => "bind_ask",
        ResultForm::Select => "bind_select",
    };
    let binder = if authenticated {
        format!("{binder}_authenticated")
    } else {
        binder.to_owned()
    };
    if authenticated {
        writeln!(
            source,
            "    let envelope = ops_core_statement::envelope_words({});",
            crate::statement::Layout::envelope_arguments()
        )
        .unwrap();
    }
    let issuer_argument = if authenticated {
        "envelope, credential_quad_bounds, "
    } else {
        ""
    };
    writeln!(source, "    ops_core_statement::{binder}({issuer_argument}request_digest, input_commitment, signature_mode, bounds, parameters, indices, result, statement_binding);\n}}\n").unwrap();
    let files=BTreeMap::from([
        ("Nargo.toml".into(),format!("[workspace]\nmembers = [\"{name}\"]\n")),
        (format!("{name}/Nargo.toml"),format!("[package]\nname = \"{name}\"\ntype = \"bin\"\nauthors = [\"sparq\"]\ncompiler_version = \">=1.0.0\"\n\n[dependencies]\ncore = {{ path = \"../zk/noir-query/core\" }}\n")),
        (format!("{name}/src/main.nr"),source),
    ]);
    Ok(Package { name, files })
}

fn array(items: impl IntoIterator<Item = String>) -> String {
    format!("[{}]", items.into_iter().collect::<Vec<_>>().join(", "))
}

#[derive(Default)]
struct Step {
    op: u8,
    a: usize,
    b: usize,
    parameter: usize,
    comparison: u8,
}
impl Step {
    fn source(&self) -> String {
        format!(
            "ops_core_expr::Step {{ op: {}, a: {}, b: {}, parameter: {}, comparison: {} }}",
            self.op, self.a, self.b, self.parameter, self.comparison
        )
    }
}

fn expression(e: &Expr, schema: &[Var], parameters: &[u32], steps: &mut Vec<Step>) -> usize {
    let step = match e {
        Expr::Term(Slot::Variable(v)) => Step {
            op: 0,
            parameter: schema.iter().position(|a| a == v).unwrap_or(schema.len()),
            ..Step::default()
        },
        Expr::Term(Slot::Parameter(p)) => Step {
            op: 1,
            parameter: parameters
                .iter()
                .position(|a| a == p)
                .expect("term parameter"),
            ..Step::default()
        },
        Expr::Compare(op, a, b) => Step {
            op: 2,
            a: expression(a, schema, parameters, steps),
            b: expression(b, schema, parameters, steps),
            comparison: match op {
                Compare::Equal => 0,
                Compare::Less => 1,
                Compare::LessOrEqual => 2,
                Compare::Greater => 3,
                Compare::GreaterOrEqual => 4,
            },
            ..Step::default()
        },
        Expr::And(a, b) | Expr::Or(a, b) => Step {
            op: if matches!(e, Expr::And(..)) { 3 } else { 4 },
            a: expression(a, schema, parameters, steps),
            b: expression(b, schema, parameters, steps),
            ..Step::default()
        },
        Expr::Not(a) => {
            let a = expression(a, schema, parameters, steps);
            Step {
                op: 5,
                a,
                b: a,
                ..Step::default()
            }
        }
        Expr::In(a, bs) => {
            let operand = expression(a, schema, parameters, steps);
            let mut accumulator = steps.len();
            steps.push(Step {
                op: 7,
                ..Step::default()
            });
            for b in bs {
                let rhs = expression(b, schema, parameters, steps);
                let comparison = steps.len();
                steps.push(Step {
                    op: 6,
                    a: operand,
                    b: rhs,
                    ..Step::default()
                });
                let next = steps.len();
                steps.push(Step {
                    op: 4,
                    a: accumulator,
                    b: comparison,
                    ..Step::default()
                });
                accumulator = next;
            }
            return accumulator;
        }
    };
    let index = steps.len();
    steps.push(step);
    index
}
