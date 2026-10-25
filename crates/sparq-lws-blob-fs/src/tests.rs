use super::*;

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("sparq-blob-fs-{}", nonce().unwrap()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

const KEY: &str = "b1-0123456789abcdef0123456789abcdef";
const OTHER: &str = "b1-abcdef0123456789abcdef0123456789";
const POOLED: &str = "$pool$b1-fedcba9876543210fedcba9876543210";

#[tokio::test]
async fn round_trip_layout_metadata_and_restart() {
    let dir = TestDir::new();
    let root = dir.0.join("nested/blobs");
    let store = FsBlobStore::open(&root).await.unwrap();
    let body = Bytes::from((0..=255).cycle().take(65_537).collect::<Vec<u8>>());
    let cases = [
        (KEY, body.clone(), format!("objects/01/23/{KEY}")),
        (OTHER, Bytes::new(), format!("objects/ab/cd/{OTHER}")),
        (
            POOLED,
            body,
            format!("pool/fe/{}", POOLED.replace('$', "%24")),
        ),
    ];
    for (key, body, path) in &cases {
        store.put(key, body.clone()).await.unwrap();
        assert_eq!(store.get(key).await.unwrap(), *body);
        assert_eq!(fs::read(root.join(path)).unwrap(), *body);
        assert!(store.exists(key).await.unwrap());
        let stat = store.stat(key).await.unwrap();
        assert_eq!(stat.key, *key);
        assert_eq!(stat.size, body.len() as u64);
        assert_eq!(stat.generation, None);
        assert_eq!(
            stat.last_modified,
            Some(fs::metadata(root.join(path)).unwrap().modified().unwrap())
        );
    }
    let mut listed = store.list().await.unwrap();
    listed.sort_by(|left, right| left.key.cmp(&right.key));
    let mut expected: Vec<_> = cases.iter().map(|(key, _, _)| *key).collect();
    expected.sort();
    assert_eq!(
        listed
            .iter()
            .map(|entry| entry.key.as_str())
            .collect::<Vec<_>>(),
        expected
    );
    assert!(listed
        .iter()
        .all(|entry| entry.last_modified.is_some() && entry.generation.is_none()));
    drop(store);
    let reopened = FsBlobStore::open(&root).await.unwrap();
    for (key, body, _) in cases {
        assert_eq!(reopened.get(key).await.unwrap(), body);
    }
    assert_eq!(fs::read_dir(root.join("tmp")).unwrap().count(), 0);
}

#[tokio::test]
async fn existing_key_is_rejected_without_replacing_bytes() {
    let dir = TestDir::new();
    let store = FsBlobStore::open(&dir.0).await.unwrap();
    for key in [KEY, POOLED] {
        store
            .put(key, Bytes::from_static(b"original"))
            .await
            .unwrap();
        assert!(matches!(
            store.put(key, Bytes::from_static(b"replacement")).await,
            Err(Error::AlreadyExists)
        ));
        assert_eq!(store.get(key).await.unwrap(), "original");
    }
    assert_eq!(fs::read_dir(dir.0.join("tmp")).unwrap().count(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_clones_cannot_overwrite_the_same_key() {
    let dir = TestDir::new();
    let store = FsBlobStore::open(&dir.0).await.unwrap();
    let mut jobs = Vec::new();
    for byte in 0..8 {
        let store = store.clone();
        jobs.push(tokio::spawn(async move {
            let body = Bytes::from(vec![byte; 8192]);
            (store.put(KEY, body.clone()).await, body)
        }));
    }
    let mut winners = Vec::new();
    for job in jobs {
        match job.await.unwrap() {
            (Ok(()), body) => winners.push(body),
            (Err(Error::AlreadyExists), _) => {}
            (result, _) => panic!("unexpected put result: {result:?}"),
        }
    }
    assert_eq!(winners.len(), 1);
    assert_eq!(store.get(KEY).await.unwrap(), winners[0]);
}

#[tokio::test]
async fn missing_keys_and_idempotent_delete() {
    let dir = TestDir::new();
    let store = FsBlobStore::open(&dir.0).await.unwrap();
    for key in [KEY, POOLED] {
        assert!(matches!(store.get(key).await, Err(Error::NotFound)));
        assert!(matches!(store.stat(key).await, Err(Error::NotFound)));
        assert!(!store.exists(key).await.unwrap());
        store.delete(key).await.unwrap();
        store.put(key, Bytes::from_static(b"bytes")).await.unwrap();
        store.delete(key).await.unwrap();
        store.delete(key).await.unwrap();
        assert!(!store.exists(key).await.unwrap());
        assert!(matches!(store.get(key).await, Err(Error::NotFound)));
    }
    assert!(store.list().await.unwrap().is_empty());
    let reopened = FsBlobStore::open(&dir.0).await.unwrap();
    assert!(reopened.list().await.unwrap().is_empty());
}

#[tokio::test]
async fn every_keyed_operation_rejects_disallowed_keys() {
    let dir = TestDir::new();
    let store = FsBlobStore::open(&dir.0).await.unwrap();
    let invalid = [
        "",
        ".",
        "..",
        "../objects/file",
        "/absolute",
        "b1-",
        "b2-0123456789abcdef0123456789abcdef",
        "b1-0123456789abcdef0123456789abcde",
        "b1-0123456789abcdef0123456789abcdef0",
        "b1-0123456789ABCDEF0123456789abcdef",
        "b1-0123456789abcdef0123456789abcdeg",
        "b1-0123456789abcdef0123456789abcdef/",
        "b1-0123456789abcdef0123456789abcdef\n",
        "b1-0123456789abcdef0123456789abcdef\0",
        "b1-0123456789abcdef0123456789abcdé",
        "$pool$$pool$b1-0123456789abcdef0123456789abcdef",
        "%24pool%24b1-0123456789abcdef0123456789abcdef",
        "$pool$b1-0123456789abcdef0123456789abcdeF",
        "b1-0123456789abcdef0123456789abcde\\",
    ];
    for key in invalid {
        assert!(
            matches!(store.get(key).await, Err(Error::InvalidKey)),
            "get {key:?}"
        );
        assert!(
            matches!(store.put(key, Bytes::new()).await, Err(Error::InvalidKey)),
            "put {key:?}"
        );
        assert!(
            matches!(store.exists(key).await, Err(Error::InvalidKey)),
            "exists {key:?}"
        );
        assert!(
            matches!(store.delete(key).await, Err(Error::InvalidKey)),
            "delete {key:?}"
        );
        assert!(
            matches!(store.stat(key).await, Err(Error::InvalidKey)),
            "stat {key:?}"
        );
    }
    assert!(store.list().await.unwrap().is_empty());
    assert_eq!(fs::read_dir(dir.0.join("tmp")).unwrap().count(), 0);
}

#[tokio::test]
async fn torn_writes_are_hidden_and_recovered_while_renamed_blobs_survive() {
    let dir = TestDir::new();
    let store = FsBlobStore::open(&dir.0).await.unwrap();
    let partial = dir.0.join("tmp").join(format!("{OTHER}.interrupted"));
    fs::write(&partial, b"partial").unwrap();
    fs::create_dir_all(dir.0.join("tmp/nested")).unwrap();
    fs::write(dir.0.join("tmp/nested/partial"), b"partial").unwrap();
    // Reproduce the crash window after a complete file was synced and renamed,
    // but before a caller could commit an index pointer or receive success.
    let complete = dir.0.join("tmp").join(format!("{KEY}.complete"));
    let mut file = File::create(&complete).unwrap();
    let body = vec![0xa5; 131_073];
    file.write_all(&body).unwrap();
    file.sync_all().unwrap();
    drop(file);
    let destination = dir.0.join(format!("objects/01/23/{KEY}"));
    create_durable_dir(destination.parent().unwrap()).unwrap();
    fs::rename(&complete, &destination).unwrap();
    sync_dir(destination.parent().unwrap()).unwrap();
    assert_eq!(
        store
            .list()
            .await
            .unwrap()
            .iter()
            .map(|entry| entry.key.as_str())
            .collect::<Vec<_>>(),
        vec![KEY]
    );
    assert!(!store.exists(OTHER).await.unwrap());
    drop(store);
    let reopened = FsBlobStore::open(&dir.0).await.unwrap();
    assert_eq!(reopened.get(KEY).await.unwrap(), body);
    assert!(matches!(reopened.get(OTHER).await, Err(Error::NotFound)));
    assert_eq!(reopened.list().await.unwrap().len(), 1);
    assert_eq!(fs::read_dir(dir.0.join("tmp")).unwrap().count(), 0);
}

#[tokio::test]
async fn listing_ignores_unexpected_files_and_noncanonical_shards() {
    let dir = TestDir::new();
    let store = FsBlobStore::open(&dir.0).await.unwrap();
    store.put(KEY, Bytes::new()).await.unwrap();
    store.put(POOLED, Bytes::new()).await.unwrap();
    for path in [
        "objects/unexpected",
        "objects/00/11",
        "objects/01/23/deeper",
        "objects/AA/BB",
        "pool/00",
    ] {
        fs::create_dir_all(dir.0.join(path)).unwrap();
    }
    for path in [
        "objects/file".to_owned(),
        "objects/01/file".to_owned(),
        "objects/01/23/invalid".to_owned(),
        format!("objects/00/11/{KEY}"),
        format!("objects/AA/BB/{OTHER}"),
        format!("objects/01/23/{POOLED}"),
        format!("pool/00/{}", encoded_key(POOLED)),
        format!("pool/fe/{POOLED}"),
        format!("pool/fe/{KEY}"),
        format!("tmp/{OTHER}.partial"),
    ] {
        fs::write(dir.0.join(path), b"ignored").unwrap();
    }
    let mut keys: Vec<_> = store
        .list()
        .await
        .unwrap()
        .into_iter()
        .map(|entry| entry.key)
        .collect();
    keys.sort();
    assert_eq!(keys, vec![POOLED, KEY]);
}

#[cfg(unix)]
#[tokio::test]
async fn listing_and_tmp_cleanup_do_not_follow_symlinks() {
    use std::os::unix::fs::symlink;
    let dir = TestDir::new();
    let outside = TestDir::new();
    fs::write(outside.0.join("keep"), b"keep").unwrap();
    let store = FsBlobStore::open(&dir.0).await.unwrap();
    create_durable_dir(&dir.0.join("objects/01/23")).unwrap();
    symlink(
        outside.0.join("keep"),
        dir.0.join(format!("objects/01/23/{KEY}")),
    )
    .unwrap();
    symlink(&outside.0, dir.0.join("objects/ab")).unwrap();
    symlink(&outside.0, dir.0.join("tmp/link")).unwrap();
    assert!(store.list().await.unwrap().is_empty());
    assert!(matches!(
        store.put(KEY, Bytes::new()).await,
        Err(Error::AlreadyExists)
    ));
    drop(store);
    FsBlobStore::open(&dir.0).await.unwrap();
    assert_eq!(fs::read(outside.0.join("keep")).unwrap(), b"keep");
    assert_eq!(fs::read_dir(dir.0.join("tmp")).unwrap().count(), 0);
}

#[tokio::test]
async fn filesystem_failures_propagate_and_failed_put_is_not_listed() {
    let dir = TestDir::new();
    let store = FsBlobStore::open(&dir.0).await.unwrap();
    fs::remove_dir(dir.0.join("tmp")).unwrap();
    fs::write(dir.0.join("tmp"), b"not a directory").unwrap();
    assert!(matches!(
        store.put(KEY, Bytes::new()).await,
        Err(Error::Io(_))
    ));
    assert!(store.list().await.unwrap().is_empty());
    assert!(matches!(FsBlobStore::open(&dir.0).await, Err(Error::Io(_))));
    fs::remove_dir_all(dir.0.join("objects")).unwrap();
    fs::write(dir.0.join("objects"), b"not a directory").unwrap();
    assert!(matches!(store.get(KEY).await, Err(Error::Io(_))));
    assert!(matches!(store.stat(KEY).await, Err(Error::Io(_))));
    assert!(matches!(store.exists(KEY).await, Err(Error::Io(_))));
    assert!(matches!(store.delete(KEY).await, Err(Error::Io(_))));
    assert!(matches!(store.list().await, Err(Error::Io(_))));
}
