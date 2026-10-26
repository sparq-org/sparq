use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};

use sparq_lws_blob_fs::FsBlobStore;

const KEY: &str = "b1-0123456789abcdef0123456789abcdef";
const POOLED: &str = "$pool$b1-fedcba9876543210fedcba9876543210";

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        loop {
            let root = std::env::temp_dir().join(format!(
                "lws-blob-fs-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&root) {
                Ok(()) => return Self(root),
                Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("create test directory: {e}"),
            }
        }
    }

    fn open(&self) -> FsBlobStore {
        FsBlobStore::open(&self.0).unwrap()
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn invalid_keys_are_refused_by_every_operation_before_creating_paths() {
    let dir = Directory::new();
    let store = dir.open();
    let mut invalid = vec![
        String::new(),
        "../x".into(),
        dir.0.join("escape").to_str().unwrap().into(),
        "b1-../0123456789abcdef0123456789abc".into(),
        "b1-0123456789abcde/0123456789abcdef".into(),
        "b1-0123456789abcde\\0123456789abcdef".into(),
        "b1-0123456789abcdef0123456789abcde\0".into(),
        "b1-0123456789abcdef0123456789abcdeg".into(),
        "$pool$../x".into(),
        format!("$pool$$pool${KEY}"),
        KEY.to_uppercase(),
        KEY.replace('a', "A"),
        KEY.replace("b1-", "b2-"),
        KEY[..KEY.len() - 1].into(),
        format!("{KEY}0"),
        format!("{KEY}/child"),
        format!("{KEY}é"),
    ];
    invalid.extend(
        invalid
            .clone()
            .into_iter()
            .map(|key| format!("$pool${key}")),
    );
    for key in invalid {
        assert_eq!(
            store.put(&key, b"bad").unwrap_err().kind(),
            ErrorKind::InvalidInput,
            "{key:?}"
        );
        assert_eq!(
            store.get(&key).unwrap_err().kind(),
            ErrorKind::InvalidInput,
            "{key:?}"
        );
        assert_eq!(
            store.exists(&key).unwrap_err().kind(),
            ErrorKind::InvalidInput,
            "{key:?}"
        );
        assert_eq!(
            store.delete(&key).unwrap_err().kind(),
            ErrorKind::InvalidInput,
            "{key:?}"
        );
        assert_eq!(
            store.stat(&key).unwrap_err().kind(),
            ErrorKind::InvalidInput,
            "{key:?}"
        );
    }
    for name in ["objects", "pool", "tmp"] {
        assert_eq!(fs::read_dir(dir.0.join(name)).unwrap().count(), 0);
    }
    assert!(!dir.0.join("escape").exists());
}

#[test]
fn layout_reopen_and_delete() {
    let dir = Directory::new();
    let store = dir.open();
    assert!(!store.exists(KEY).unwrap());
    assert_eq!(store.get(KEY).unwrap_err().kind(), ErrorKind::NotFound);
    store.put(KEY, b"persistent\0bytes").unwrap();
    store.put(POOLED, b"pooled").unwrap();
    let object = dir.0.join("objects/01/23").join(KEY);
    let pooled = dir.0.join("pool/fe").join(POOLED);
    assert_eq!(fs::read(&object).unwrap(), b"persistent\0bytes");
    assert_eq!(fs::read(&pooled).unwrap(), b"pooled");
    assert!(store.exists(KEY).unwrap());
    drop(store);
    let store = dir.open();
    assert_eq!(store.get(KEY).unwrap(), b"persistent\0bytes");
    assert_eq!(store.get(POOLED).unwrap(), b"pooled");
    let entries = store.list().unwrap();
    assert_eq!(entries.len(), 2);
    for (key, path) in [(KEY, object), (POOLED, pooled)] {
        let entry = entries.iter().find(|entry| entry.key == key).unwrap();
        assert_eq!(entry.generation, None);
        assert_eq!(
            entry.last_modified,
            Some(fs::metadata(path).unwrap().modified().unwrap())
        );
        assert_eq!(
            store.stat(key).unwrap().unwrap().last_modified,
            entry.last_modified
        );
    }
    store.delete(KEY).unwrap();
    store.delete(KEY).unwrap();
    store.delete(POOLED).unwrap();
    assert!(!store.exists(KEY).unwrap());
    assert!(store.stat(KEY).unwrap().is_none());
    drop(store);
    assert!(dir.open().list().unwrap().is_empty());
}

#[test]
fn existing_key_is_refused_without_changing_bytes_or_mtime() {
    let dir = Directory::new();
    let store = dir.open();
    for key in [KEY, POOLED] {
        store.put(key, b"original").unwrap();
        let modified = store.stat(key).unwrap().unwrap().last_modified;
        for bytes in [b"replacement".as_slice(), b"original"] {
            assert_eq!(
                store.put(key, bytes).unwrap_err().kind(),
                ErrorKind::AlreadyExists
            );
            assert_eq!(store.get(key).unwrap(), b"original");
            assert_eq!(store.stat(key).unwrap().unwrap().last_modified, modified);
        }
    }
    assert_eq!(fs::read_dir(dir.0.join("tmp")).unwrap().count(), 0);
}

#[test]
fn open_sweeps_temporary_leftovers_and_list_skips_them() {
    let dir = Directory::new();
    let store = dir.open();
    store.put(KEY, b"keep").unwrap();
    let temporary = dir.0.join("tmp");
    fs::write(temporary.join("abandoned"), b"partial").unwrap();
    fs::write(temporary.join(POOLED), b"unpublished").unwrap();
    fs::create_dir(temporary.join("leftover-directory")).unwrap();
    fs::write(temporary.join("leftover-directory/file"), b"partial").unwrap();
    assert_eq!(
        store
            .list()
            .unwrap()
            .iter()
            .map(|entry| entry.key.as_str())
            .collect::<Vec<_>>(),
        [KEY]
    );
    drop(store);
    let reopened = dir.open();
    assert_eq!(fs::read_dir(temporary).unwrap().count(), 0);
    assert_eq!(reopened.get(KEY).unwrap(), b"keep");
    assert!(!reopened.exists(POOLED).unwrap());
}

#[test]
fn concurrent_distinct_puts_share_fanout_without_losing_bytes() {
    let dir = Directory::new();
    let store = Arc::new(dir.open());
    let barrier = Arc::new(Barrier::new(16));
    std::thread::scope(|scope| {
        for i in 0..16 {
            let (store, barrier) = (store.clone(), barrier.clone());
            scope.spawn(move || {
                let key = format!("{}b1-{i:032x}", if i % 2 == 0 { "" } else { "$pool$" });
                barrier.wait();
                store.put(&key, &[i]).unwrap();
                assert_eq!(store.get(&key).unwrap(), [i]);
            });
        }
    });
    assert_eq!(store.list().unwrap().len(), 16);
    assert_eq!(fs::read_dir(dir.0.join("tmp")).unwrap().count(), 0);
}

#[test]
fn concurrent_same_key_has_one_winner_across_independent_handles() {
    let dir = Directory::new();
    let store = dir.open();
    let barrier = Barrier::new(12);
    let winners = std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for i in 0..12 {
            let (store, barrier) = (store.clone(), &barrier);
            handles.push(scope.spawn(move || {
                barrier.wait();
                match store.put(KEY, &[i]) {
                    Ok(()) => Some(i),
                    Err(e) => {
                        assert_eq!(e.kind(), ErrorKind::AlreadyExists);
                        None
                    }
                }
            }));
        }
        handles
            .into_iter()
            .filter_map(|h| h.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(winners.len(), 1);
    assert_eq!(store.get(KEY).unwrap(), winners);
    assert_eq!(store.list().unwrap().len(), 1);
    assert_eq!(fs::read_dir(dir.0.join("tmp")).unwrap().count(), 0);
}

#[test]
fn nested_root_is_created_and_io_errors_propagate() {
    let dir = Directory::new();
    let root = dir.0.join("nested/root");
    let store = FsBlobStore::open(&root).unwrap();
    fs::write(root.join("objects/01"), b"blocks fanout").unwrap();
    assert!(store.put(KEY, b"body").is_err());
    assert_eq!(fs::read_dir(root.join("tmp")).unwrap().count(), 0);
    drop(store);
    assert!(FsBlobStore::open(root.join("objects/01")).is_err());
}
