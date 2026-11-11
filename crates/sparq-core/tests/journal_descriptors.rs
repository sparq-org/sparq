//! Journal descriptors stay bounded across named-graph creation and crash recovery.
#![cfg(all(target_os = "linux", feature = "mmap"))]

use oxrdf::{NamedNode, Term};
use sparq_core::{Graph, TxnOp};
use std::collections::HashSet;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};

const GRAPHS: usize = 384;

fn name(i: usize) -> Term {
    NamedNode::new(format!("http://example/graph/{i}"))
        .unwrap()
        .into()
}

fn triple(i: usize) -> [Term; 3] {
    [name(i), name(GRAPHS), name(GRAPHS + 1)]
}

fn triples(graph: &Graph) -> HashSet<[Term; 3]> {
    graph
        .iter_ids()
        .map(|ids| ids.map(|id| graph.dict().term(id)))
        .collect()
}

fn descriptors() -> usize {
    std::fs::read_dir("/proc/self/fd").unwrap().count()
}

fn bounded(baseline: usize) {
    assert!(
        descriptors() <= baseline + 8,
        "journal descriptors grew with named graphs"
    );
}

fn create(dir: &Path, transactional: bool) {
    let mut graph = Graph::open_or_create(dir, &Graph::new()).unwrap();
    let baseline = descriptors();
    if transactional {
        // Keep all materialization pending together, including its dirty WAL state.
        let ops: Vec<_> = (0..GRAPHS)
            .map(|i| TxnOp::Insert {
                slot: Some(name(i)),
                triple: triple(i),
            })
            .collect();
        graph.commit_txn_ops(&ops).unwrap();
    }
    for i in 0..GRAPHS {
        let index = graph.ensure_named(&name(i)).unwrap();
        graph
            .apply_delta_to_named(index, &[triple(i)], &[])
            .unwrap();
        bounded(baseline);
    }
    if transactional {
        graph.finish_txn().unwrap();
        bounded(baseline);
    }
    // The parent kills us with the graph and its retained journal still live.
    println!("READY");
    std::io::stdout().flush().unwrap();
    loop {
        std::thread::park();
    }
}

fn verify(dir: &Path) {
    let baseline = descriptors();
    let mut graph = Graph::open(dir).unwrap();
    bounded(baseline);
    assert_eq!(graph.named().len(), GRAPHS);
    for i in 0..GRAPHS {
        let child = graph.named_graph(&name(i)).unwrap();
        assert_eq!(triples(child), HashSet::from([triple(i)]));
        bounded(baseline);
    }
    // Reopened logs can still append and checkpoint.
    let extra = [name(GRAPHS + 2), name(GRAPHS), name(GRAPHS + 1)];
    graph
        .commit_txn_ops(&[TxnOp::Insert {
            slot: Some(name(0)),
            triple: extra.clone(),
        }])
        .unwrap();
    graph
        .apply_delta_to_named(0, std::slice::from_ref(&extra), &[])
        .unwrap();
    graph.finish_txn().unwrap();
    graph.clear_txn().unwrap();
    graph
        .apply_delta_to_named(GRAPHS - 1, std::slice::from_ref(&extra), &[])
        .unwrap();
    bounded(baseline);
    graph.close().unwrap();
    let graph = Graph::open(dir).unwrap();
    for i in [0, GRAPHS - 1] {
        assert_eq!(
            triples(graph.named_graph(&name(i)).unwrap()),
            HashSet::from([triple(i), extra.clone()])
        );
    }
    bounded(baseline);
    graph.close().unwrap();
}

#[test]
fn directory_journal_descriptors_are_bounded() {
    if let Ok(mode) = std::env::var("SPARQ_DESCRIPTOR_CHILD") {
        let dir = std::env::var_os("SPARQ_DESCRIPTOR_DIR").unwrap();
        match mode.as_str() {
            "direct" => create(Path::new(&dir), false),
            "transactional" => create(Path::new(&dir), true),
            "verify" => verify(Path::new(&dir)),
            _ => panic!("unknown child mode"),
        }
        return;
    }
    let directory = std::env::temp_dir().join(format!(
        "sparq-descriptors-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    ));
    std::fs::create_dir(&directory).unwrap();
    for mode in ["direct", "transactional"] {
        let dir = directory.join(mode);
        let command = |mode: &str| {
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "directory_journal_descriptors_are_bounded",
                    "--nocapture",
                ])
                .env("SPARQ_DESCRIPTOR_CHILD", mode)
                .env("SPARQ_DESCRIPTOR_DIR", &dir);
            command
        };
        let mut child = command(mode).stdout(Stdio::piped()).spawn().unwrap();
        let ready = BufReader::new(child.stdout.take().unwrap())
            .lines()
            .any(|line| line.unwrap() == "READY");
        if !ready {
            panic!(
                "{mode} child exited before acknowledgement: {}",
                child.wait().unwrap()
            );
        }
        child.kill().unwrap();
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(child.wait().unwrap().signal(), Some(9));
        assert!(
            command("verify").status().unwrap().success(),
            "{mode} recovery failed"
        );
    }
    std::fs::remove_dir_all(directory).unwrap();
}
