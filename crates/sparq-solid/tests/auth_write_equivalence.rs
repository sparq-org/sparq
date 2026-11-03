//! Replay the existing write suites through an observing store. Every default-budget
//! write is compared with committed snapshots captured from the pinned source revision.
//! Both the complete stored quad sets (including views) and per-session decisions enter
//! the snapshot; large sets use a length-framed digest, never a cardinality-only oracle.
//! Source: `5cfd3640a661d1bcaf994708f0d16a1e0c5adccc`.
//! Regenerate with `python3 crates/sparq-solid/tests/regenerate_auth_writes.py OUTPUT_DIR`.
//! Capture runs require AUTH_WRITE_GOLDEN_DIR and write outside the repository.
//! In-crate `src/*_tests.rs` suites use private fault hooks and lifecycle state that an
//! external observing store cannot intercept. They run in the unit-test suite; this
//! corpus covers the shared public write methods and their resulting store/view state.

extern crate self as sparq_solid;
extern crate sparq_solid as implementation;

pub use implementation::*;
use serde_json::{json, Value};
use sparq_core::Graph;
use sparq_engine::QueryBudget;
use std::ops::{Deref, DerefMut};
use std::sync::OnceLock;

pub struct PodStore {
    inner: implementation::PodStore,
    case: Option<String>,
    sequence: std::sync::atomic::AtomicUsize,
}

impl Deref for PodStore {
    type Target = implementation::PodStore;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
impl DerefMut for PodStore {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

std::thread_local! {
    static SEQUENCE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn compact(mut rows: Vec<String>) -> Value {
    rows.sort();
    if rows.iter().map(String::len).sum::<usize>() <= 1024 {
        return json!({"rows": rows});
    }
    let mut digest = 0x6c62272e07bb014262b821756295c58du128;
    for row in &rows {
        for byte in (row.len() as u64)
            .to_le_bytes()
            .iter()
            .chain(row.as_bytes())
        {
            digest ^= u128::from(*byte);
            digest = digest.wrapping_mul(0x0000000001000000000000000000013bu128);
        }
    }
    json!({"count":rows.len(), "fnv1a128":format!("{digest:032x}")})
}

impl PodStore {
    pub fn new(graph: Graph) -> Self {
        let thread = std::thread::current();
        let index = SEQUENCE.with(|n| {
            let v = n.get();
            n.set(v + 1);
            v
        });
        Self {
            inner: implementation::PodStore::new(graph),
            case: thread.name().map(|name| format!("{name}/{index}")),
            sequence: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn observe<T: std::fmt::Debug>(
        &self,
        operation: &str,
        result: &Result<T, String>,
        default: bool,
    ) {
        if !default {
            return;
        }
        let mut graphs = Vec::new();
        for (name, graph) in std::iter::once((String::new(), self.graph()))
            .chain(self.graph().named().iter().map(|(n, g)| (n.to_string(), g)))
        {
            let scan = graph.store().scan(&[None, None, None]);
            graphs.push(format!("graph {name}"));
            graphs.extend(scan.rows.iter().map(|r| {
                let [s, p, o] = scan.to_spo(r);
                format!(
                    "{name} {} {} {}",
                    graph.dict().term(s),
                    graph.dict().term(p),
                    graph.dict().term(o)
                )
            }));
        }
        let mut decisions = Vec::new();
        for agent in [
            None,
            Some(fixture::ALICE),
            Some(fixture::BOB),
            Some(fixture::CAROL),
            Some(fixture::DAVE),
            Some("https://admin.ex/card#me"),
        ] {
            for client in [None, Some(fixture::APP)] {
                let session = Session {
                    agent,
                    client,
                    ..Session::default()
                };
                for mode in [Mode::Read, Mode::Write, Mode::Append, Mode::Control] {
                    let mut allowed: Vec<_> = self
                        .accessible(&session, mode)
                        .iter()
                        .map(ToString::to_string)
                        .collect();
                    allowed.sort();
                    decisions.push(format!("{agent:?}/{client:?}/{mode:?}/{allowed:?}"));
                }
            }
        }
        let result = match result {
            Ok(value) => json!({"ok": format!("{value:?}")}),
            Err(error) => {
                json!({"refused":true,"allowance":auth_allowance_refusal(error).map(|k| format!("{k:?}"))})
            }
        };
        let actual = json!({"operation":operation,"result":result,"graphs":compact(graphs),"decisions":compact(decisions)});
        let Some(case) = &self.case else {
            return;
        };
        let sequence = self
            .sequence
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let key = format!("{case}/{sequence}");
        if let Some(dir) = std::env::var_os("AUTH_WRITE_GOLDEN_DIR") {
            let dir = std::path::PathBuf::from(dir);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join(format!("{}.json", key.replace('/', "__"))),
                serde_json::to_vec_pretty(&json!({"key":key,"snapshot":actual})).unwrap(),
            )
            .unwrap();
        } else {
            static GOLDEN: OnceLock<Value> = OnceLock::new();
            let expected = GOLDEN.get_or_init(|| {
                serde_json::from_str(include_str!("fixtures/auth-write-ledger.json")).unwrap()
            });
            assert_eq!(
                &actual, &expected["writes"][&key],
                "write differs from pinned source: {key}"
            );
        }
    }

    fn default_budget(&self) -> bool {
        format!("{:?}", self.auth_budget()) == format!("{:?}", default_auth_budget())
    }

    pub fn put_acl(
        &mut self,
        doc: &str,
        body: &str,
        format: &str,
    ) -> Result<AclWriteOutcome, String> {
        let default = self.default_budget();
        let result = self.inner.put_acl(doc, body, format);
        self.observe("put_acl", &result, default);
        result
    }
    pub fn put_acl_acp(
        &mut self,
        doc: &str,
        body: &str,
        format: &str,
    ) -> Result<AclWriteOutcome, String> {
        let default = self.default_budget();
        let result = self.inner.put_acl_acp(doc, body, format);
        self.observe("put_acl_acp", &result, default);
        result
    }
    pub fn delete_acl(&mut self, doc: &str) -> Result<AclWriteOutcome, String> {
        let default = self.default_budget();
        let result = self.inner.delete_acl(doc);
        self.observe("delete_acl", &result, default);
        result
    }
    pub fn delete_acl_acp(&mut self, doc: &str) -> Result<AclWriteOutcome, String> {
        let default = self.default_budget();
        let result = self.inner.delete_acl_acp(doc);
        self.observe("delete_acl_acp", &result, default);
        result
    }
    pub fn update_as(&mut self, session: &Session, query: &str) -> Result<(), String> {
        let default = self.default_budget();
        let result = self.inner.update_as(session, query);
        self.observe("update_as", &result, default);
        result
    }
    pub fn update_as_acp(&mut self, session: &Session, query: &str) -> Result<(), String> {
        let default = self.default_budget();
        let result = self.inner.update_as_acp(session, query);
        self.observe("update_as_acp", &result, default);
        result
    }
    pub fn update_as_with_budget(
        &mut self,
        session: &Session,
        query: &str,
        budget: &QueryBudget,
    ) -> Result<(), String> {
        let default = self.default_budget();
        let result = self.inner.update_as_with_budget(session, query, budget);
        self.observe("update_as_with_budget", &result, default);
        result
    }
    pub fn update_as_acp_with_budget(
        &mut self,
        session: &Session,
        query: &str,
        budget: &QueryBudget,
    ) -> Result<(), String> {
        let default = self.default_budget();
        let result = self.inner.update_as_acp_with_budget(session, query, budget);
        self.observe("update_as_acp_with_budget", &result, default);
        result
    }
}

#[path = "acp.rs"]
mod acp;
#[path = "acp_empty_matcher.rs"]
mod acp_empty_matcher;
#[path = "acp_matcher_scope.rs"]
mod acp_matcher_scope;
#[path = "acp_trusted_context.rs"]
mod acp_trusted_context;
#[path = "acp_unresolved_reference.rs"]
mod acp_unresolved_reference;
#[path = "auth_allowance.rs"]
mod auth_allowance;
#[path = "auth_store_enumeration.rs"]
mod auth_store_enumeration;
#[path = "concurrent_readers.rs"]
mod concurrent_readers;
#[path = "decide.rs"]
mod decide;
#[path = "group_doc_remat.rs"]
mod group_doc_remat;
#[path = "incremental_remat.rs"]
mod incremental_remat;
#[path = "odrl_bridge.rs"]
mod odrl_bridge_writes;
#[path = "odrl_request_decide.rs"]
mod odrl_request_decide;
#[path = "origin_root.rs"]
mod origin_root;
#[path = "remat_failure_fails_closed.rs"]
mod remat_failure_fails_closed;
#[path = "update.rs"]
mod update;
#[path = "update_entry_points.rs"]
mod update_entry_points;
#[path = "update_read_scope.rs"]
mod update_read_scope;
#[path = "view_model_entry_points.rs"]
mod view_model_entry_points;
#[path = "wac.rs"]
mod wac;
#[path = "write_through.rs"]
mod write_through;

#[path = "write_admission_parity.rs"]
mod write_admission_parity;
