//! One holder per graph directory (`sparq_core::dir_lock`): every path that opens, writes,
//! replaces or recovers a directory holds its one lock, and every graph opened from it (its
//! named graphs, a vacuumed replacement) keeps that lock while it lives.

#![cfg(feature = "mmap")]

use oxrdf::{NamedNode, Term};
use sparq_core::Graph;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sparq-dirlock-it-{tag}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn iri(s: &str) -> Term {
    Term::NamedNode(NamedNode::new_unchecked(format!("http://ex/{s}")))
}

fn triple(s: &str) -> [Term; 3] {
    [iri(s), iri("p"), iri("o")]
}

fn has(g: &Graph, s: &str) -> bool {
    let ids: Option<Vec<_>> = triple(s).iter().map(|t| g.id_of(t)).collect();
    ids.is_some_and(|ids| g.iter_ids().any(|t| t[..] == ids[..]))
}

/// A dataset with a default graph and two named graphs, saved at `dir`.
fn save_dataset(dir: &Path) {
    let nq = "<http://ex/a> <http://ex/p> <http://ex/o> .\n\
              <http://ex/b> <http://ex/p> <http://ex/o> <http://ex/g0> .\n\
              <http://ex/c> <http://ex/p> <http://ex/o> <http://ex/g1> .\n";
    Graph::load_dataset(nq, "nquads")
        .unwrap()
        .save(dir)
        .unwrap();
}

fn would_block(r: std::io::Result<Graph>) -> bool {
    matches!(r, Err(e) if e.kind() == ErrorKind::WouldBlock)
}

/// The directory of a named graph inside a graph is its root's: opening it directly, under
/// any spelling, is refused, and so is saving or building into it.
#[test]
fn a_named_graph_directory_cannot_be_opened_on_its_own() {
    let root = scratch("child-open");
    let dir = root.join("g");
    save_dataset(&dir);
    let child = dir.join("named").join("0");
    for alias in [
        child.clone(),
        dir.join("named").join("..").join("named").join("0"),
    ] {
        let err = Graph::open(&alias)
            .err()
            .expect("a named graph's directory opened");
        assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
    }
    let err = Graph::new().save(&child).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
    assert!(Graph::build_external("".as_bytes(), "ntriples", &child, 1024).is_err());
    // The root itself opens, with its named graphs.
    assert_eq!(Graph::open(&dir).unwrap().named.len(), 2);
    std::fs::remove_dir_all(&root).ok();
}

/// Every named graph holds its root's lock: one taken out of the root keeps the directory
/// locked after the root is gone, including named graphs created, compacted or renumbered
/// since the open.
#[test]
fn a_named_graph_keeps_its_roots_lock() {
    let root = scratch("child-lock");
    let dir = root.join("g");
    save_dataset(&dir);
    // As opened.
    let mut g = Graph::open(&dir).unwrap();
    let child = g.named.pop().unwrap();
    drop(g);
    assert!(
        would_block(Graph::open(&dir)),
        "an opened named graph lost the root's lock"
    );
    drop(child);
    // Created, compacted and renumbered since the open.
    let mut g = Graph::open(&dir).unwrap();
    let fresh = g.ensure_named(&iri("g2")).unwrap();
    g.named[fresh].1.apply_delta(&[triple("d")], &[]).unwrap();
    g.compact().unwrap();
    g.drop_named_durable(&iri("g0")).unwrap();
    let mut named = std::mem::take(&mut g.named);
    drop(g);
    for (_, child) in &named {
        assert!(child.is_directory_backed());
    }
    assert!(
        would_block(Graph::open(&dir)),
        "the root's lock went with the root"
    );
    named.truncate(1);
    assert!(
        would_block(Graph::open(&dir)),
        "the lock went before its last holder"
    );
    drop(named);
    let g = Graph::open(&dir).expect("released with the last named graph");
    assert_eq!(g.named.len(), 2, "g1 and g2 survive the drop of g0");
    std::fs::remove_dir_all(&root).ok();
}

/// A graph never saves into the directory it is open from, however the path is spelled: it
/// still maps and logs into the files a save truncates. Its files are left as they were.
#[test]
fn a_graph_does_not_save_into_its_own_directory() {
    let root = scratch("own-save");
    let dir = root.join("g");
    save_dataset(&dir);
    let mut g = Graph::open(&dir).unwrap();
    g.apply_delta(&[triple("logged")], &[]).unwrap();
    let mut spellings = vec![
        dir.clone(),
        PathBuf::from(format!("{}/", dir.display())),
        root.join("g").join("..").join("g"),
    ];
    #[cfg(unix)]
    {
        let link = root.join("link");
        std::os::unix::fs::symlink(&dir, &link).unwrap();
        spellings.push(link);
    }
    for spelling in &spellings {
        assert!(g.is_backed_by(spelling), "{}", spelling.display());
        for err in [
            g.save(spelling).unwrap_err(),
            g.save_compressed(spelling).unwrap_err(),
        ] {
            assert_eq!(
                err.kind(),
                ErrorKind::InvalidInput,
                "{}: {err}",
                spelling.display()
            );
        }
    }
    // A named graph cannot save over its root either.
    let err = g.named[0].1.save(&dir).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
    // Elsewhere is fine, and the graph's own directory still holds its state.
    let copy = root.join("copy");
    assert!(!g.is_backed_by(&copy));
    g.save(&copy).unwrap();
    drop(g);
    for d in [&dir, &copy] {
        let g = Graph::open(d).unwrap();
        assert!(has(&g, "a") && has(&g, "logged"), "{}", d.display());
        assert_eq!(g.named.len(), 2, "{}", d.display());
    }
    std::fs::remove_dir_all(&root).ok();
}

/// A graph opened through a symbolic link works on the directory it resolves to: its log,
/// recovery and swaps use that one path, so a compaction rewrites the real directory and
/// leaves the link a link.
#[cfg(unix)]
#[test]
fn a_graph_opened_through_a_link_swaps_the_real_directory() {
    let root = scratch("alias-swap");
    let dir = root.join("real");
    save_dataset(&dir);
    let link = root.join("link");
    std::os::unix::fs::symlink(&dir, &link).unwrap();
    let mut g = Graph::open(&link).unwrap();
    assert_eq!(g.persist_dir().unwrap(), dir.canonicalize().unwrap());
    g.apply_delta(&[triple("new")], &[]).unwrap();
    g.compact().unwrap();
    assert!(
        link.symlink_metadata().unwrap().file_type().is_symlink(),
        "the link was replaced"
    );
    assert!(!root.join("link.compact-old").exists() && !root.join("link.compact-new").exists());
    drop(g);
    let g = Graph::open(&dir).unwrap();
    assert!(
        has(&g, "new"),
        "the real directory holds the compacted base"
    );
    drop(g);

    // A swap interrupted after its first rename leaves the real directory missing: the link
    // dangles and is refused rather than taken for a directory to create, while the real path
    // recovers the store.
    std::fs::rename(&dir, root.join("real.compact-old")).unwrap();
    let err = Graph::open(&link).err().expect("a dangling link opened");
    assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
    let err = Graph::open_or_create(&link, &Graph::new())
        .err()
        .expect("a dangling link seeded");
    assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
    let g = Graph::open(&dir).unwrap();
    assert!(has(&g, "new"), "recovered by the real path");
    std::fs::remove_dir_all(&root).ok();
}

/// A store whose swap a crash interrupted (between the two renames: the directory missing,
/// the new base beside it) is recovered before `open_or_create` decides whether to seed: it
/// is opened with its data, not replaced by the seed.
#[test]
fn open_or_create_recovers_a_swap_before_seeding() {
    let root = scratch("seed-recover");
    let dir = root.join("g");
    save_dataset(&root.join("g.compact-new"));
    save_dataset(&root.join("g.compact-old"));
    assert!(!dir.exists());
    let seed =
        Graph::load_str("<http://ex/seed> <http://ex/p> <http://ex/o> .", "ntriples").unwrap();
    let g = Graph::open_or_create(&dir, &seed).unwrap();
    assert!(has(&g, "a"), "the swapped-in store survives");
    assert!(!has(&g, "seed"), "a store mid-swap is not re-seeded");
    assert_eq!(g.named.len(), 2);
    std::fs::remove_dir_all(&root).ok();
}

/// A restore that fails before its renames leaves the directory as it was, and the graph keeps
/// logging to it: an update acknowledged after the failure survives a reopen.
#[test]
fn a_failed_restore_in_place_keeps_logging() {
    let root = scratch("restore-fail");
    let dir = root.join("g");
    save_dataset(&dir);
    let mut g = Graph::open(&dir).unwrap();
    // A file where the swap stages the new base makes writing it fail.
    std::fs::write(root.join("g.compact-new"), b"in the way").unwrap();
    let fresh = Graph::load_str("<http://ex/r> <http://ex/p> <http://ex/o> .", "ntriples").unwrap();
    assert!(g.restore_in_place(fresh).is_err());
    g.apply_delta(&[triple("after")], &[]).unwrap();
    assert!(would_block(Graph::open(&dir)), "the graph keeps its lock");
    drop(g);
    let g = Graph::open(&dir).unwrap();
    assert!(
        has(&g, "a") && has(&g, "after"),
        "the update after the failed restore is durable"
    );
    assert!(!has(&g, "r"));
    std::fs::remove_dir_all(&root).ok();
}

/// Read-only opens share the lock with each other and never with a writable holder; they write
/// nothing into the directory and refuse every write.
#[test]
fn read_only_opens_share_the_directory_and_exclude_a_writer() {
    let root = scratch("read-only");
    let dir = root.join("g");
    save_dataset(&dir);
    let first = Graph::open_read_only(&dir).unwrap();
    let mut second = Graph::open_read_only(&root.join("g").join("..").join("g")).unwrap();
    assert!(has(&first, "a") && has(&second, "a"));
    assert_eq!(second.named.len(), 2);
    assert!(!first.is_directory_backed());
    assert!(
        would_block(Graph::open(&dir)),
        "a writer got in beside readers"
    );
    assert!(
        Graph::new().save(&dir).is_err(),
        "a save got in beside readers"
    );
    assert!(
        !dir.join("wal.log").exists() && !dir.join("txn.log").exists(),
        "a read-only open wrote"
    );
    assert!(second.apply_delta(&[triple("x")], &[]).is_err());
    assert!(second.named[0].1.apply_delta(&[triple("x")], &[]).is_err());
    assert!(second.ensure_named(&iri("g9")).is_err());
    assert!(second.compact().is_err() && second.vacuum().is_err());
    assert_eq!(
        second.save(&dir).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    second
        .save(&root.join("copy"))
        .expect("a read-only graph saves elsewhere");
    drop((first, second));

    let mut writer = Graph::open(&dir).unwrap();
    assert!(
        would_block(Graph::open_read_only(&dir)),
        "a reader got in beside a writer"
    );
    writer.apply_delta(&[triple("logged")], &[]).unwrap();
    drop(writer);
    // A log to replay needs a writable open (and a compaction) first.
    let err = Graph::open_read_only(&dir)
        .err()
        .expect("opened read-only over a log");
    assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
    Graph::open(&dir).unwrap().compact().unwrap();
    let g = Graph::open_read_only(&dir).unwrap();
    assert!(has(&g, "logged"));
    // The directory of a named graph is refused read-only too.
    assert!(Graph::open_read_only(&dir.join("named").join("0")).is_err());
    drop(g);

    // So is the residue of an interrupted swap.
    save_dataset(&root.join("g.compact-new"));
    let err = Graph::open_read_only(&dir)
        .err()
        .expect("opened read-only over a swap's residue");
    assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
    std::fs::remove_dir_all(&root).ok();
}

/// A fork (and a snapshot) shares the base's memory-mapped files, so it holds the directory's
/// lock after the graph it came from is gone, and cannot be saved over that directory.
#[test]
fn a_fork_keeps_the_lock_and_cannot_save_over_its_source() {
    let root = scratch("fork");
    let dir = root.join("g");
    save_dataset(&dir);
    for read_only in [false, true] {
        let reader = if read_only {
            Graph::open_read_only(&dir).unwrap()
        } else {
            Graph::open(&dir).unwrap()
        };
        let fork = reader.fork();
        let snapshot = reader.snapshot();
        drop(reader);
        assert!(
            would_block(Graph::open(&dir)),
            "the lock went with the reader"
        );
        let err = fork.save(&dir).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
        assert!(has(&fork, "a") && has(&snapshot, "a"));
        drop(fork);
        assert!(would_block(Graph::open(&dir)), "the snapshot keeps it too");
        drop(snapshot);
        drop(Graph::open(&dir).expect("released with the last fork"));
    }
    std::fs::remove_dir_all(&root).ok();
}

/// Ownership is marked, not inferred from names: nothing can be opened, saved or built inside a
/// store, a save into a directory refuses another store inside it, and a refused claim creates
/// no directory.
#[test]
fn a_store_owns_everything_under_it() {
    let root = scratch("ownership");
    let dir = root.join("g");
    save_dataset(&dir);
    assert!(dir.join("sparq-store").exists() && dir.join("named/0/sparq-store").exists());
    for inside in [
        dir.join("other"),
        dir.join("named").join("7"),
        dir.join("a").join("b"),
    ] {
        let err = Graph::open(&inside).err().expect("opened inside a store");
        assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
        let err = Graph::new().save(&inside).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
        assert!(Graph::open_read_only(&inside).is_err());
    }
    assert!(
        !dir.join("a").exists(),
        "a refused claim created a directory"
    );

    // A store made where a root will later be saved: the save refuses to delete it.
    let later = root.join("later");
    let squatter = later.join("named").join("0");
    save_dataset(&squatter);
    let held = Graph::open(&squatter).unwrap();
    let err = Graph::load_dataset(
        "<http://ex/x> <http://ex/p> <http://ex/o> <http://ex/g> .",
        "nquads",
    )
    .unwrap()
    .save(&later)
    .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
    assert!(has(&held, "a"));
    drop(held);
    assert!(
        Graph::open(&squatter).is_ok(),
        "the store inside was left alone"
    );
    std::fs::remove_dir_all(&root).ok();
}

/// A store saved before markers is a store all the same: refused as an ancestor, opened as
/// before, and marked on its next writable open (a read-only open writes nothing).
#[test]
fn a_legacy_store_without_markers_is_still_a_store() {
    let root = scratch("legacy");
    let dir = root.join("g");
    save_dataset(&dir);
    for d in [dir.clone(), dir.join("named/0"), dir.join("named/1")] {
        std::fs::remove_file(d.join("sparq-store")).unwrap();
    }
    assert!(Graph::open(&dir.join("named").join("0")).is_err());
    drop(Graph::open_read_only(&dir).unwrap());
    assert!(
        !dir.join("sparq-store").exists(),
        "a read-only open wrote a marker"
    );
    let g = Graph::open(&dir).unwrap();
    assert_eq!(g.named.len(), 2);
    assert_eq!(std::fs::read(dir.join("sparq-store")).unwrap(), b"root\n");
    assert_eq!(
        std::fs::read(dir.join("named/0/sparq-store")).unwrap(),
        b"child\n"
    );
    std::fs::remove_dir_all(&root).ok();
}

/// A store's internals are its own: a named graph that is a symbolic link to another store, or
/// a file hard-linked with another store's, is refused (writable and read-only).
#[cfg(unix)]
#[test]
fn linked_internals_are_refused() {
    let root = scratch("links");
    let a = root.join("a");
    let b = root.join("b");
    save_dataset(&a);
    save_dataset(&b);
    // b's first named graph is a link to a's.
    std::fs::remove_dir_all(b.join("named/0")).unwrap();
    std::os::unix::fs::symlink(a.join("named/0"), b.join("named/0")).unwrap();
    for err in [Graph::open(&b).err(), Graph::open_read_only(&b).err()] {
        let err = err.expect("opened over a linked named graph");
        assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
    }
    // c shares a's permutation file through a hard link.
    let c = root.join("c");
    save_dataset(&c);
    std::fs::remove_file(c.join("perm0.bin")).unwrap();
    std::fs::hard_link(a.join("perm0.bin"), c.join("perm0.bin")).unwrap();
    for d in [&a, &c] {
        let err = Graph::open(d)
            .err()
            .expect("opened over a hard-linked file");
        assert_eq!(
            err.kind(),
            ErrorKind::InvalidInput,
            "{}: {err}",
            d.display()
        );
    }
    std::fs::remove_dir_all(&root).ok();
}

/// A read-only graph refuses the durable clear and the rebuilding update alike.
#[test]
fn a_read_only_graph_refuses_every_mutator() {
    let root = scratch("ro-mutators");
    let dir = root.join("g");
    save_dataset(&dir);
    let mut g = Graph::open_read_only(&dir).unwrap();
    assert!(g.check_writable().is_err());
    assert!(g.clear_default_durable().is_err());
    assert!(g.clear_named_durable(&iri("g0")).is_err());
    assert!(g.drop_all_named_durable().is_err());
    assert!(has(&g, "a"), "a refused clear changed the graph");
    assert!(
        g.fork().check_writable().is_ok(),
        "a fork is an in-memory copy"
    );
    std::fs::remove_dir_all(&root).ok();
}
