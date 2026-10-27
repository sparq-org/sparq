//! Shared checkpoint state, including slots not yet published in the manifest.

use super::*;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Seek, SeekFrom};
use std::path::{Path, PathBuf};

// Bound recovery work without imposing a checkpoint on ordinary small updates.
// This is a policy limit, independent of the size of any one atomic frame.
pub(crate) const JOURNAL_CHECKPOINT_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Default)]
pub(super) struct Pending {
    wals: BTreeMap<PathBuf, DirtyWal>,
    journals: BTreeMap<PathBuf, std::fs::File>,
    manifests: BTreeMap<PathBuf, NamedManifest>,
    created: BTreeSet<PathBuf>,
    removed: BTreeSet<PathBuf>,
}

struct DirtyWal {
    file: std::fs::File,
    needs_entry_sync: std::sync::Arc<std::sync::Mutex<bool>>,
}

impl TreeState {
    pub(super) fn prepare_journal(&self, journal: &TxnJournal) -> std::io::Result<()> {
        let path = TxnJournal::path(&journal.dir);
        let different = self
            .checkpoint
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .journals
            .keys()
            .any(|pending| *pending != path);
        if different {
            if self.materializing() {
                return Err(std::io::Error::other(
                    "finish the earlier tree transaction first",
                ));
            }
            // Child journals replay before their parent's. Checkpoint when switching
            // journals so recovery cannot reverse the order of two writers in one tree.
            self.checkpoint()?;
        }
        Ok(())
    }

    pub(super) fn materializing(&self) -> bool {
        self.materializing
            .load(std::sync::atomic::Ordering::Acquire)
    }

    pub(super) fn dirty_wal(&self, wal: &Wal) -> std::io::Result<()> {
        let mut pending = self.checkpoint.lock().unwrap_or_else(|e| e.into_inner());
        let path = Wal::path(&wal.dir);
        if let std::collections::btree_map::Entry::Vacant(entry) = pending.wals.entry(path) {
            entry.insert(DirtyWal {
                file: wal.file.try_clone()?,
                needs_entry_sync: std::sync::Arc::clone(&wal.needs_entry_sync),
            });
        }
        Ok(())
    }

    pub(super) fn retain_journal(&self, journal: &TxnJournal) -> std::io::Result<()> {
        let mut pending = self.checkpoint.lock().unwrap_or_else(|e| e.into_inner());
        let path = TxnJournal::path(&journal.dir);
        if let std::collections::btree_map::Entry::Vacant(entry) = pending.journals.entry(path) {
            entry.insert(journal.file.try_clone()?);
        }
        Ok(())
    }

    pub(super) fn defer_create(
        &self,
        dir: &Path,
        slot: u32,
        name: &Term,
        next_slot: u32,
    ) -> std::io::Result<()> {
        let mut pending = self.checkpoint.lock().unwrap_or_else(|e| e.into_inner());
        let manifest = pending.manifest(dir)?;
        manifest.entries.push((slot, name.clone()));
        manifest.next_slot = next_slot;
        pending
            .created
            .insert(dir.join(NAMED_SUBDIR).join(slot.to_string()));
        Ok(())
    }

    pub(super) fn defer_drop(&self, dir: &Path, slot: u32, next_slot: u32) -> std::io::Result<()> {
        let path = dir.join(NAMED_SUBDIR).join(slot.to_string());
        check_dir_tree_removable(&path)?;
        let mut pending = self.checkpoint.lock().unwrap_or_else(|e| e.into_inner());
        let manifest = pending.manifest(dir)?;
        manifest.entries.retain(|(id, _)| *id != slot);
        manifest.next_slot = next_slot;
        pending.removed.insert(path.clone());
        self.record_replacement(&path);
        Ok(())
    }

    pub(super) fn checkpoint_if_due(&self) -> std::io::Result<()> {
        let due = self
            .checkpoint
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .journals
            .values()
            .try_fold(false, |due, file| {
                Ok::<_, std::io::Error>(due || file.metadata()?.len() >= JOURNAL_CHECKPOINT_BYTES)
            })?;
        if due {
            self.checkpoint()?;
        }
        Ok(())
    }

    pub(super) fn checkpoint(&self) -> std::io::Result<()> {
        let _permit = self.write_permit();
        if self.materializing() {
            return Err(std::io::Error::other(
                "finish transaction materialization before checkpoint",
            ));
        }
        if self.poisoned() {
            return Err(std::io::Error::other(
                "checkpoint refused: reopen the frozen tree",
            ));
        }
        let result = self
            .checkpoint
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .flush();
        if result.is_err() {
            self.poison();
        } else {
            self.checkpoint_on_drop
                .store(true, std::sync::atomic::Ordering::Release);
        }
        result
    }
}

impl Pending {
    fn manifest(&mut self, dir: &Path) -> std::io::Result<&mut NamedManifest> {
        // FreshFile cannot publish through a directory at its temporary path. Detect
        // that obstruction while materializing, preserving the caller's fail-closed
        // response even though the actual manifest publication is deferred.
        let temporary = dir.join(format!("{NAMED_MANIFEST}.tmp"));
        match std::fs::symlink_metadata(&temporary) {
            Ok(meta) if meta.is_dir() => {
                return Err(std::io::Error::other(format!(
                    "manifest publication refused: {} is a directory",
                    temporary.display()
                )))
            }
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error),
            _ => {}
        }
        if !self.manifests.contains_key(dir) {
            self.manifests.insert(
                dir.to_path_buf(),
                read_named_manifest(dir)?.unwrap_or(NamedManifest {
                    entries: Vec::new(),
                    next_slot: 0,
                }),
            );
        }
        Ok(self.manifests.get_mut(dir).expect("inserted manifest"))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if self.journals.is_empty() {
            return Ok(());
        }
        // New slots have WALs but no published base yet. The empty base plus the WAL
        // represents the complete slot; never save the live overlay into that base.
        for dir in &self.created {
            if !self.removed.iter().any(|removed| dir.starts_with(removed)) {
                let empty = Graph::new();
                write_store_marker(dir, Role::Child)?;
                empty.store.save(dir)?;
                empty.dict.save_mmap(dir)?;
                stream_write_numerics(&dir.join(NUMERIC_CACHE_FILE), 0, &empty.numerics)?;
                stream_write_temporals(&dir.join(TEMPORAL_CACHE_FILE), 0, &empty.temporals)?;
                fsync_files_in(dir)?;
            }
        }
        for (path, wal) in &self.wals {
            sync_log_data(&wal.file, path)?;
            sync_log_entry(
                &wal.file,
                path,
                &mut wal
                    .needs_entry_sync
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()),
            )?;
        }
        // Children first: no parent manifest may publish an incomplete child tree.
        let mut manifests: Vec<_> = self.manifests.iter().collect();
        manifests.sort_by_key(|(dir, _)| std::cmp::Reverse(dir.components().count()));
        for (dir, manifest) in manifests {
            publish_named_manifest(dir, &[], &manifest.entries, manifest.next_slot)?;
        }
        // Release cloned WAL descriptors before removing slots (also required on Windows).
        self.wals.clear();
        for dir in &self.removed {
            if dir.exists() {
                remove_dir_tree_checked(dir)?;
                fsync_dir(dir.parent().expect("slot parent"))?;
            }
        }
        // All materialization is durable. Only now may the recovery records disappear.
        for (path, file) in &mut self.journals {
            let bytes = std::fs::read(path)?;
            file.set_len(0)?;
            file.seek(SeekFrom::Start(0))?;
            if let Err(error) = sync_log_data(file, path) {
                // Publish a complete copy through a fresh descriptor. Rewriting this
                // truncated file in place could expose only an old frame-prefix after
                // another crash, overwriting the already-checkpointed WAL state.
                // Never sync the failed descriptor again.
                write_fresh(path, &bytes)?;
                return Err(error);
            }
            fault_point("journal-truncated", path.parent().expect("journal parent"))?;
        }
        *self = Self::default();
        Ok(())
    }
}

impl Drop for TreeState {
    fn drop(&mut self) {
        if !self.poisoned()
            && !self.materializing()
            && !std::thread::panicking()
            && self
                .checkpoint_on_drop
                .load(std::sync::atomic::Ordering::Acquire)
        {
            // Explicit Graph::close reports errors; drop keeps the journal on failure.
            let _ = self.checkpoint();
        }
    }
}
