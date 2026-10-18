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

/// The identity the marker of the store at `d` records (`root <id>` or `child <id>`).
fn store_id(d: &Path) -> String {
    let marker = std::fs::read_to_string(d.join("sparq-store")).unwrap();
    marker.trim_end().split_once(' ').unwrap().1.to_string()
}

/// Records in the store at `d` that it is `kind` staging of the store whose identity is `id`
/// (its marker then carries that identity too), as a swap does before its renames: the
/// on-disk state a crash mid-swap leaves.
fn stage(d: &Path, kind: &str, id: &str) {
    std::fs::write(d.join("sparq-store"), format!("root {id}\n")).unwrap();
    std::fs::write(d.join("sparq-staging"), format!("{kind} {id}\n")).unwrap();
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
    #[cfg_attr(not(unix), allow(unused_mut))] // a symbolic link is added on Unix only
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
    stage(&dir, "compact-old", &store_id(&dir));
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
    let id = "00112233445566778899aabbccddeeff";
    for kind in ["compact-new", "compact-old"] {
        let staged = root.join(format!("g.{kind}"));
        save_dataset(&root.join("staging"));
        std::fs::rename(root.join("staging"), &staged).unwrap();
        stage(&staged, kind, id);
    }
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
    // What is in the way is not this store's staging: opening refuses it and leaves it alone
    // until it is removed by hand.
    let err = Graph::open(&dir)
        .err()
        .expect("opened past foreign staging");
    assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
    assert!(root.join("g.compact-new").is_file());
    std::fs::remove_file(root.join("g.compact-new")).unwrap();
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
    let marker = std::fs::read_to_string(dir.join("sparq-store")).unwrap();
    assert!(marker.starts_with("root "), "{marker}");
    let marker = std::fs::read_to_string(dir.join("named/0/sparq-store")).unwrap();
    assert!(marker.starts_with("child "), "{marker}");
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

/// A graph rebuilt around another's dictionary (`into_dict` + `from_parts`, as a reasoner does)
/// keeps the directory locked while the mapped dictionary lives, writable or read-only.
#[test]
fn a_rebuilt_graph_keeps_its_dictionarys_lock() {
    let root = scratch("rebuilt");
    let dir = root.join("g");
    save_dataset(&dir);
    for read_only in [false, true] {
        let mut g = if read_only {
            Graph::open_read_only(&dir).unwrap()
        } else {
            Graph::open(&dir).unwrap()
        };
        let triples: Vec<_> = g.iter_ids().collect();
        g.named.clear();
        let rebuilt = Graph::from_parts(g.into_dict(), triples);
        assert!(
            would_block(Graph::open(&dir)),
            "the lock went with the graph the dictionary came from"
        );
        assert!(has(&rebuilt, "a"));
        drop(rebuilt);
        drop(Graph::open(&dir).expect("released with the dictionary"));
    }
    std::fs::remove_dir_all(&root).ok();
}

/// A whole-store write (`restore_into_durable`, `save`) never replaces a directory holding
/// another store it cannot tell is its own: an independently held root, a closed legacy store
/// without a marker, or a marker that cannot be parsed. Everything inside is left as it was.
#[test]
fn a_whole_store_write_refuses_what_it_cannot_tell_is_its_own() {
    let root = scratch("foreign");
    let fresh =
        || Graph::load_str("<http://ex/n> <http://ex/p> <http://ex/o> .", "ntriples").unwrap();

    // An independently locked root inside the target of a restore.
    let target = root.join("t");
    let nested = target.join("sub");
    save_dataset(&nested);
    let held = Graph::open(&nested).unwrap();
    let err = Graph::restore_into_durable(&target, fresh())
        .err()
        .expect("restored over a held store");
    assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
    assert!(has(&held, "a"));
    drop(held);
    assert!(
        Graph::open(&nested).is_ok(),
        "the held store was left alone"
    );

    // A closed legacy store (no markers) where the future root's first named graph would go.
    let later = root.join("later");
    let legacy = later.join("named").join("0");
    save_dataset(&legacy);
    for d in [
        legacy.clone(),
        legacy.join("named/0"),
        legacy.join("named/1"),
    ] {
        std::fs::remove_file(d.join("sparq-store")).unwrap();
    }
    let err = fresh().save(&later).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
    let err = Graph::restore_into_durable(&later, fresh())
        .err()
        .expect("restored over a legacy store");
    assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
    assert!(
        has(&Graph::open_read_only(&legacy).unwrap(), "a"),
        "the legacy store was left alone"
    );

    // A marker that is neither `root` nor `child`: refused, never guessed.
    let odd = root.join("odd");
    std::fs::create_dir_all(odd.join("x")).unwrap();
    std::fs::write(odd.join("x").join("sparq-store"), b"garbage").unwrap();
    let err = fresh().save(&odd).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
    assert!(odd.join("x").join("sparq-store").exists());
    // ... and a store whose own marker is garbage does not open writable.
    let own = root.join("own");
    save_dataset(&own);
    std::fs::write(own.join("sparq-store"), b"garbage").unwrap();
    let err = Graph::open(&own)
        .err()
        .expect("opened under a garbage marker");
    assert_eq!(err.kind(), ErrorKind::InvalidData, "{err}");

    // A restore over a directory a crash left mid-swap recovers it first.
    let swapped = root.join("swapped");
    save_dataset(&swapped);
    stage(&swapped, "compact-old", &store_id(&swapped));
    std::fs::rename(&swapped, swapped.with_extension("compact-old")).unwrap();
    let g = Graph::restore_into_durable(&swapped, fresh()).unwrap();
    assert!(has(&g, "n") && !has(&g, "a"));
    assert!(!swapped.with_extension("compact-old").exists());
    std::fs::remove_dir_all(&root).ok();
}

/// A save writes every file to a fresh sibling and renames it into place: a file already there
/// is replaced, never written through (a handle on the old file still reads the old bytes), no
/// temporary file is left behind, and a directory whose files are hard-linked elsewhere is
/// refused for saves and builds alike.
#[cfg(unix)]
#[test]
fn a_save_replaces_files_and_never_writes_through_them() {
    use std::io::Read;
    use std::os::unix::fs::MetadataExt;
    let root = scratch("fresh");
    let dir = root.join("g");
    save_dataset(&dir);
    let perm = dir.join("perm0.bin");
    let before = std::fs::read(&perm).unwrap();
    let inode = std::fs::metadata(&perm).unwrap().ino();
    let mut old = std::fs::File::open(&perm).unwrap();
    Graph::load_str("<http://ex/n> <http://ex/p> <http://ex/o> .", "ntriples")
        .unwrap()
        .save(&dir)
        .unwrap();
    assert_ne!(
        std::fs::metadata(&perm).unwrap().ino(),
        inode,
        "rewritten in place"
    );
    let mut still = Vec::new();
    old.read_to_end(&mut still).unwrap();
    assert_eq!(still, before, "the old file was written through");
    for entry in std::fs::read_dir(&dir).unwrap() {
        let name = entry.unwrap().file_name();
        assert!(
            !name.to_string_lossy().ends_with(".tmp"),
            "left {name:?} behind"
        );
    }
    let g = Graph::open(&dir).unwrap();
    assert!(has(&g, "n") && !has(&g, "a"));
    drop(g);

    // A file of the store hard-linked to one outside it: saves and builds are refused.
    let outside = root.join("outside.bin");
    std::fs::write(&outside, b"not a store file").unwrap();
    std::fs::remove_file(&perm).unwrap();
    std::fs::hard_link(&outside, &perm).unwrap();
    let err = Graph::new().save(&dir).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
    assert!(Graph::build_external("".as_bytes(), "ntriples", &dir, 1024).is_err());
    assert_eq!(std::fs::read(&outside).unwrap(), b"not a store file");
    std::fs::remove_dir_all(&root).ok();
}

/// A read-only open checks for links before it walks the store for residue, so a `named/` that
/// links back into itself is refused at once rather than walked forever.
#[cfg(unix)]
#[test]
fn a_read_only_open_does_not_walk_a_link_loop() {
    let root = scratch("loop");
    let dir = root.join("g");
    save_dataset(&dir);
    // Each of x's entries links back to x: a walk that follows links fans out without end.
    let x = root.join("x");
    for i in 0..3 {
        std::fs::create_dir_all(x.join(i.to_string())).unwrap();
        std::os::unix::fs::symlink(&x, x.join(i.to_string()).join("named")).unwrap();
    }
    std::fs::remove_dir_all(dir.join("named")).unwrap();
    std::os::unix::fs::symlink(&x, dir.join("named")).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let d = dir.clone();
    std::thread::spawn(move || {
        tx.send(Graph::open_read_only(&d).err().map(|e| e.kind()))
            .ok();
    });
    let kind = rx
        .recv_timeout(std::time::Duration::from_secs(30))
        .expect("the read-only open did not return");
    assert_eq!(kind, Some(ErrorKind::InvalidInput));
    std::fs::remove_dir_all(&root).ok();
}

/// Recovery removes or promotes only staging that records this store as its owner: another
/// store (or staging left by an older version, which records no owner) at a staging name is
/// refused by every save, build, restore and open, and left exactly as it was.
#[test]
fn recovery_never_removes_what_this_store_did_not_stage() {
    let root = scratch("foreign-staging");
    let fresh =
        || Graph::load_str("<http://ex/n> <http://ex/p> <http://ex/o> .", "ntriples").unwrap();
    let foreign = |at: &Path| {
        save_dataset(&root.join("elsewhere"));
        std::fs::rename(root.join("elsewhere"), at).unwrap();
    };
    // Checked under a neutral name (a store at a staging name reads as mid-swap), then put back.
    let intact = |at: &Path| {
        let check = root.join("check");
        std::fs::rename(at, &check).unwrap();
        assert!(
            has(&Graph::open_read_only(&check).unwrap(), "a"),
            "{} was touched",
            at.display()
        );
        std::fs::rename(&check, at).unwrap();
    };
    let refused = |what: &str, r: std::io::Result<()>| {
        let err = r
            .err()
            .unwrap_or_else(|| panic!("{what} went past foreign staging"));
        assert_eq!(err.kind(), ErrorKind::InvalidInput, "{what}: {err}");
    };

    // Beside a store with `named/`, an independent store at `named.drop-old`.
    let target = root.join("target");
    save_dataset(&target);
    let drop_old = target.join("named.drop-old");
    foreign(&drop_old);
    refused("save", fresh().save(&target));
    refused("build", {
        let nt = "<http://ex/s> <http://ex/p> <http://ex/o> .\n";
        Graph::build_external(nt.as_bytes(), "ntriples", &target, 1 << 16)
            .map_err(|e| std::io::Error::new(ErrorKind::InvalidInput, e))
    });
    refused(
        "restore",
        Graph::restore_into_durable(&target, fresh()).map(drop),
    );
    refused("open", Graph::open(&target).map(drop));
    std::fs::rename(&drop_old, root.join("moved")).unwrap();
    intact(&root.join("moved"));
    std::fs::remove_dir_all(root.join("moved")).unwrap();

    // An independent store at `<target>.compact-new`, with the target present ...
    let compact_new = target.with_extension("compact-new");
    foreign(&compact_new);
    refused("save", fresh().save(&target));
    refused("open", Graph::open(&target).map(drop));
    intact(&compact_new);
    // ... and staging that names another store as its owner.
    stage(
        &compact_new,
        "compact-new",
        "ffffffffffffffffffffffffffffffff",
    );
    refused("save", fresh().save(&target));
    assert!(compact_new.join("sparq-staging").exists());
    std::fs::remove_dir_all(&compact_new).unwrap();

    // With the target missing, an independent store at `<target>.compact-old` is not taken
    // for the target's old base.
    let gone = root.join("gone");
    let compact_old = gone.with_extension("compact-old");
    foreign(&compact_old);
    refused("save", fresh().save(&gone));
    refused("open", Graph::open(&gone).map(drop));
    assert!(!gone.exists());
    intact(&compact_old);
    std::fs::remove_dir_all(&root).ok();
}

/// A build keeps its side files in a private scratch directory it makes afresh: links already
/// in the store directory (here `tmp/p<n>`, where builds used to sort, each pointing into
/// another directory holding a `run0.bin`, the name a sort's first run takes) are never
/// followed, and no scratch directory is left behind.
#[cfg(unix)]
#[test]
fn a_build_never_writes_through_a_link_in_its_directory() {
    let root = scratch("build-scratch");
    let other = root.join("other");
    let dir = root.join("g");
    std::fs::create_dir_all(dir.join("tmp")).unwrap();
    for p in 1..6 {
        let target = other.join(format!("p{p}"));
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("run0.bin"), b"keep").unwrap();
        std::os::unix::fs::symlink(&target, dir.join("tmp").join(format!("p{p}"))).unwrap();
    }
    // More rows than a sibling sort holds in memory, so each one spills runs.
    let rows = 70_000;
    let nt: String = (0..rows)
        .map(|i| format!("<http://ex/s{i}> <http://ex/p{}> <http://ex/o> .\n", i % 7))
        .collect();
    Graph::build_external(nt.as_bytes(), "ntriples", &dir, 1 << 12).unwrap();
    for p in 1..6 {
        let target = other.join(format!("p{p}"));
        let names: Vec<_> = std::fs::read_dir(&target)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(
            names,
            ["run0.bin"],
            "the build wrote through {}",
            target.display()
        );
        assert_eq!(std::fs::read(target.join("run0.bin")).unwrap(), b"keep");
    }
    let left = std::fs::read_dir(&dir).unwrap().any(|e| {
        e.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".sparq-")
    });
    assert!(!left, "a scratch directory was left behind");
    assert_eq!(Graph::open(&dir).unwrap().len(), rows);
    std::fs::remove_dir_all(&root).ok();
}

/// A store with only a marker (identity `id`) and the content of a saved dataset: the old or
/// new half of a swap caught between a rename and its staging record.
fn half(at: &Path, id: &str, tag: &str) {
    let nt = format!("<http://ex/{tag}> <http://ex/p> <http://ex/o> .");
    Graph::load_str(&nt, "ntriples").unwrap().save(at).unwrap();
    std::fs::write(at.join("sparq-store"), format!("root {id}\n")).unwrap();
}

/// Staging is per store and never a store of its own: `g.a` and `g.b` stage at
/// `g.a.compact-*` and `g.b.compact-*` (never a shared `g.compact-*`); a directory carrying a
/// staging record is refused by every claim and left as it is; staging at the names an
/// earlier version shared is removed only when it records this store.
#[test]
fn staging_is_per_store_and_never_claimed_on_its_own() {
    let root = scratch("staging-names");
    let (a, b) = (root.join("g.a"), root.join("g.b"));
    save_dataset(&a);
    save_dataset(&b);
    let mut g = Graph::open(&a).unwrap();
    g.compact().unwrap();
    drop(g);
    let names: Vec<String> = std::fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        names.iter().all(|n| !n.contains("compact")),
        "staging left after a swap: {names:?}"
    );
    assert!(has(&Graph::open(&b).unwrap(), "a"), "g.b was touched");

    // Legacy shared names: this store's staging goes, another store's stays.
    let (mine, theirs) = (root.join("g.compact-old"), root.join("g.compact-new"));
    save_dataset(&mine);
    stage(&mine, "compact-old", &store_id(&a));
    save_dataset(&theirs);
    stage(&theirs, "compact-new", &store_id(&b));
    drop(Graph::open(&a).unwrap());
    assert!(!mine.exists(), "this store's legacy staging was kept");
    assert!(
        theirs.join("sparq-staging").exists(),
        "another store's staging was touched"
    );

    // A staging directory is never claimed on its own, and nothing is removed for trying.
    let refused = |what: &str, r: std::io::Result<()>| {
        let err = r
            .err()
            .unwrap_or_else(|| panic!("{what} claimed a staging directory"));
        assert_eq!(err.kind(), ErrorKind::InvalidInput, "{what}: {err}");
    };
    refused("open", Graph::open(&theirs).map(drop));
    refused("read-only open", Graph::open_read_only(&theirs).map(drop));
    refused(
        "save",
        Graph::load_str("<http://ex/n> <http://ex/p> <http://ex/o> .", "ntriples")
            .unwrap()
            .save(&theirs),
    );
    assert!(theirs.join("sparq-staging").exists());
    assert!(has(&Graph::open(&a).unwrap(), "a") && has(&Graph::open(&b).unwrap(), "a"));
    std::fs::remove_dir_all(&root).ok();
}

/// A crash can leave one half of a swap without its staging record (the old base renamed
/// aside but not yet marked, or the new base unmarked just before its rename): recovery takes
/// that half only when its own marker carries the identity the other half's record names.
#[test]
fn recovery_takes_an_unmarked_half_only_of_the_same_store() {
    let root = scratch("unmarked-half");
    let id = "00112233445566778899aabbccddeeff";
    let other = "ffeeddccbbaa99887766554433221100";
    let swap = |name: &str, old_marked: bool, new_marked: bool, new_id: &str| {
        let (old, new) = (
            root.join(format!("{name}.compact-old")),
            root.join(format!("{name}.compact-new")),
        );
        half(&old, id, "old");
        half(&new, new_id, "new");
        if old_marked {
            std::fs::write(old.join("sparq-staging"), format!("compact-old {id}\n")).unwrap();
        }
        if new_marked {
            std::fs::write(new.join("sparq-staging"), format!("compact-new {new_id}\n")).unwrap();
        }
        (root.join(name), old, new)
    };
    // Old base aside, not yet marked: the new base is promoted.
    let (dir, old, new) = swap("p", false, true, id);
    let g = Graph::open(&dir).unwrap();
    assert!(has(&g, "new") && !old.exists() && !new.exists());
    assert!(
        !dir.join("sparq-staging").exists(),
        "the live store carries a staging record"
    );
    drop(g);
    // New base unmarked just before its rename: promoted.
    let (dir, old, new) = swap("q", true, false, id);
    let g = Graph::open(&dir).unwrap();
    assert!(has(&g, "new") && !old.exists() && !new.exists());
    assert!(
        !dir.join("sparq-staging").exists(),
        "the live store carries a staging record"
    );
    drop(g);
    // An unmarked "new base" of another store: refused, both left alone.
    let (dir, old, new) = swap("r", true, false, other);
    let err = Graph::open(&dir)
        .err()
        .expect("took another store's directory");
    assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
    assert!(!dir.exists() && old.join("sparq-staging").exists() && new.exists());
    std::fs::remove_dir_all(&root).ok();
}

/// A writable open checks a store's internals before recovery touches anything: with
/// `named/` a link to a directory outside the store (here one carrying a staging record) and
/// drop staging beside it, the open is refused and the outside directory keeps its record.
#[cfg(unix)]
#[test]
fn recovery_never_follows_a_link_out_of_the_store() {
    let root = scratch("recover-link");
    let outside = root.join("A");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(
        outside.join("sparq-staging"),
        b"drop-old 00112233445566778899aabbccddeeff\n",
    )
    .unwrap();
    let dir = root.join("B");
    save_dataset(&dir);
    let id = store_id(&dir);
    std::fs::rename(dir.join("named"), root.join("named-real")).unwrap();
    std::os::unix::fs::symlink(&outside, dir.join("named")).unwrap();
    let staged = dir.join("named.drop-new");
    std::fs::create_dir(&staged).unwrap();
    std::fs::write(staged.join("sparq-staging"), format!("drop-new {id}\n")).unwrap();
    let seed = Graph::load_str("<http://ex/n> <http://ex/p> <http://ex/o> .", "ntriples").unwrap();
    for (what, r) in [
        ("open", Graph::open(&dir).map(drop)),
        (
            "open_or_create",
            Graph::open_or_create(&dir, &seed).map(drop),
        ),
    ] {
        assert!(r.is_err(), "{what} went through a linked named/");
        assert!(
            outside.join("sparq-staging").exists(),
            "{what} cleared a record outside the store"
        );
        assert!(
            staged.exists(),
            "{what} removed staging past a linked named/"
        );
    }
    std::fs::remove_dir_all(&root).ok();
}
