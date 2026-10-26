use std::cell::RefCell;

use super::*;

#[derive(Default)]
struct SyncProbe {
    paths: Vec<PathBuf>,
    failure: Option<(PathBuf, usize)>,
    shared: Option<Arc<Mutex<Vec<PathBuf>>>>,
}

thread_local! {
    static SYNCS: RefCell<SyncProbe> = RefCell::default();
}

pub(super) fn before_sync(path: &Path) -> io::Result<()> {
    SYNCS.with_borrow_mut(|probe| {
        probe.paths.push(path.to_path_buf());
        if let Some(shared) = &probe.shared {
            shared.lock().unwrap().push(path.to_path_buf());
        }
        if let Some((target, skip)) = &mut probe.failure {
            if path == target {
                if *skip == 0 {
                    probe.failure = None;
                    return Err(io::Error::other("injected sync failure"));
                }
                *skip -= 1;
            }
        }
        Ok(())
    })
}

fn take_syncs() -> Vec<PathBuf> {
    SYNCS.with_borrow_mut(|probe| std::mem::take(&mut probe.paths))
}

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        loop {
            let root = std::env::temp_dir().join(format!(
                "lws-blob-sync-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&root) {
                Ok(()) => return Self(root),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("test directory: {e}"),
            }
        }
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn second_put_in_same_leaf_only_syncs_file_and_parent_across_clones() {
    for marker in ["", "$pool$"] {
        let dir = Directory::new();
        let store = FsBlobStore::open(&dir.0).unwrap();
        let clone = store.clone();
        let parent = dir.0.join(if marker.is_empty() {
            "objects/01/23"
        } else {
            "pool/01"
        });
        take_syncs();
        store
            .put(
                &format!("{marker}b1-0123456789abcdef0123456789abcdef"),
                b"first",
            )
            .unwrap();
        take_syncs();
        let second = format!("{marker}b1-0123456789abcdef0123456789abcdee");
        clone.put(&second, b"second").unwrap();
        let syncs = take_syncs();
        assert_eq!(syncs.len(), 2, "{marker}: {syncs:?}");
        assert_eq!(syncs[0].parent().unwrap(), dir.0.join("tmp"));
        assert_eq!(syncs[1], parent);
        assert_eq!(clone.get(&second).unwrap(), b"second");
    }
}

#[test]
fn preexisting_fanout_is_synced_on_first_use_and_after_reopen() {
    let dir = Directory::new();
    for suffix in ["objects/01/23", "objects/01/24", "pool/01"] {
        fs::create_dir_all(dir.0.join(suffix)).unwrap();
    }
    for tail in ['a', 'b'] {
        let store = FsBlobStore::open(&dir.0).unwrap();
        for (prefix, marker, expected) in [
            (
                "0123",
                "",
                vec!["objects/01", "objects", "objects/01/23", "objects/01"],
            ),
            ("0124", "", vec!["objects/01/24", "objects/01"]),
            ("0123", "$pool$", vec!["pool/01", "pool"]),
        ] {
            take_syncs();
            let key = format!("{marker}b1-{prefix}{tail:0>28}");
            store.put(&key, b"body").unwrap();
            let syncs = take_syncs();
            let expected: Vec<_> = expected.iter().map(|part| dir.0.join(part)).collect();
            assert_eq!(&syncs[..syncs.len() - 2], expected);
            assert_eq!(syncs[syncs.len() - 2].parent().unwrap(), dir.0.join("tmp"));
            assert_eq!(
                syncs.last().unwrap(),
                store.path(&key).unwrap().parent().unwrap()
            );
        }
    }
}

#[test]
fn fanout_bits_are_independent_across_layouts_and_hex_pairs() {
    let dir = Directory::new();
    let store = FsBlobStore::open(&dir.0).unwrap();
    let clone = store.clone();
    for prefix in ["0000", "0001", "003f", "0040", "00ff", "0100", "ffff"] {
        for marker in ["", "$pool$"] {
            let first = format!("{marker}b1-{prefix}{:028x}", 0);
            store.put(&first, b"first").unwrap();
            let second = format!("{marker}b1-{prefix}{:028x}", 1);
            take_syncs();
            clone.put(&second, b"second").unwrap();
            assert_eq!(take_syncs().len(), 2, "{marker}{prefix}");
            assert_eq!(clone.get(&second).unwrap(), b"second");
        }
    }
}

#[test]
fn concurrent_clones_initialize_each_fanout_directory_once() {
    for marker in ["", "$pool$"] {
        let dir = Directory::new();
        let store = FsBlobStore::open(&dir.0).unwrap();
        let syncs = Arc::new(Mutex::new(Vec::new()));
        let barrier = std::sync::Barrier::new(8);
        std::thread::scope(|scope| {
            for i in 0..8 {
                let store = store.clone();
                let syncs = syncs.clone();
                let barrier = &barrier;
                scope.spawn(move || {
                    SYNCS.with_borrow_mut(|probe| probe.shared = Some(syncs));
                    barrier.wait();
                    let key = format!("{marker}b1-0123{i:028x}");
                    store.put(&key, b"body").unwrap();
                    assert_eq!(store.get(&key).unwrap(), b"body");
                });
            }
        });
        let mut expected = if marker.is_empty() {
            vec!["objects/01", "objects", "objects/01/23", "objects/01"]
        } else {
            vec!["pool/01", "pool"]
        };
        let parent = if marker.is_empty() {
            "objects/01/23"
        } else {
            "pool/01"
        };
        expected.extend(std::iter::repeat_n(parent, 8));
        let mut expected: Vec<_> = expected.iter().map(|part| dir.0.join(part)).collect();
        expected.sort();
        let mut directories: Vec<_> = syncs
            .lock()
            .unwrap()
            .iter()
            .filter(|path| !path.starts_with(dir.0.join("tmp")))
            .cloned()
            .collect();
        directories.sort();
        assert_eq!(directories, expected);
    }
}

#[test]
fn failed_fanout_syncs_are_retried_before_publishing() {
    for (marker, failed, skip, expected) in [
        (
            "",
            "objects/01",
            0,
            vec!["objects/01", "objects", "objects/01/23", "objects/01"],
        ),
        (
            "",
            "objects",
            0,
            vec!["objects/01", "objects", "objects/01/23", "objects/01"],
        ),
        ("", "objects/01/23", 0, vec!["objects/01/23", "objects/01"]),
        ("", "objects/01", 1, vec!["objects/01/23", "objects/01"]),
        ("$pool$", "pool/01", 0, vec!["pool/01", "pool"]),
        ("$pool$", "pool", 0, vec!["pool/01", "pool"]),
    ] {
        let dir = Directory::new();
        let store = FsBlobStore::open(&dir.0).unwrap();
        let clone = store.clone();
        SYNCS.with_borrow_mut(|probe| probe.failure = Some((dir.0.join(failed), skip)));
        let first = format!("{marker}b1-0123456789abcdef0123456789abcdef");
        assert!(store.put(&first, b"unpublished").is_err());
        assert!(!store.exists(&first).unwrap());
        SYNCS.with_borrow(|probe| assert!(probe.failure.is_none()));
        take_syncs();
        let retry = format!("{marker}b1-0123456789abcdef0123456789abcdee");
        clone.put(&retry, b"retry").unwrap();
        let syncs = take_syncs();
        let expected: Vec<_> = expected.iter().map(|part| dir.0.join(part)).collect();
        assert_eq!(
            &syncs[..syncs.len() - 2],
            expected,
            "{marker} {failed} {skip}"
        );
        assert_eq!(clone.get(&retry).unwrap(), b"retry");
    }
}
