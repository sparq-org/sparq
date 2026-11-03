//! Test-only stable log images. Unsynced tails may persist independently per file.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Default)]
struct Images {
    // Each image records the synced byte range 0..bytes.len(); log syncs cover the whole file.
    stable: BTreeMap<PathBuf, Vec<u8>>,
    syncs: usize,
}
thread_local! {
    static IMAGES: RefCell<Option<Images>> = const { RefCell::new(None) };
}

pub(super) fn synced(path: &Path) {
    IMAGES.with(|images| {
        if let Some(images) = images.borrow_mut().as_mut() {
            if matches!(
                path.file_name().and_then(|s| s.to_str()),
                Some("wal.log" | "txn.log")
            ) {
                images
                    .stable
                    .insert(path.to_path_buf(), std::fs::read(path).unwrap());
            }
            images.syncs += 1;
        }
    });
}

#[cfg(test)]
pub(super) fn start(root: &Path) {
    fn visit(root: &Path, images: &mut Images) {
        for entry in std::fs::read_dir(root).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                visit(&entry.path(), images);
            } else if matches!(entry.file_name().to_str(), Some("wal.log" | "txn.log")) {
                images
                    .stable
                    .insert(entry.path(), std::fs::read(entry.path()).unwrap());
            }
        }
    }
    let mut images = Images::default();
    visit(root, &mut images);
    IMAGES.with(|state| *state.borrow_mut() = Some(images));
}

pub(super) fn syncs() -> usize {
    IMAGES.with(|state| state.borrow().as_ref().unwrap().syncs)
}

pub(super) fn synced_len(path: &Path) -> Option<usize> {
    IMAGES.with(|state| {
        state
            .borrow()
            .as_ref()
            .and_then(|images| images.stable.get(path).map(Vec::len))
    })
}

pub(super) fn stop() {
    IMAGES.with(|state| *state.borrow_mut() = None);
}

pub(super) fn crash(root: &Path, mut seed: u64) {
    fn logs(root: &Path, paths: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(root).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                logs(&entry.path(), paths);
            } else if matches!(entry.file_name().to_str(), Some("wal.log" | "txn.log")) {
                paths.push(entry.path());
            }
        }
    }
    let images = IMAGES.with(|state| state.borrow_mut().take().unwrap());
    let mut paths = Vec::new();
    logs(root, &mut paths);
    for path in paths {
        let stable = images.stable.get(&path).map(Vec::as_slice).unwrap_or(&[]);
        let bytes = std::fs::read(&path).unwrap();
        let mut boundaries = vec![stable.len()];
        let mut end = stable.len();
        while end + 12 <= bytes.len() {
            let len = u32::from_le_bytes(bytes[end + 8..end + 12].try_into().unwrap()) as usize;
            end += 24 + len;
            if end <= bytes.len() {
                boundaries.push(end);
            }
        }
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let keep = boundaries[seed as usize % boundaries.len()];
        let mut persisted = stable.to_vec();
        if keep > stable.len() {
            persisted.extend_from_slice(&bytes[stable.len()..keep]);
        }
        std::fs::write(path, persisted).unwrap();
    }
}
