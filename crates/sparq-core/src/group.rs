//! Journal-first group commit. No materialized WAL bytes escape before the barrier.

use super::*;
use std::collections::BTreeMap;
use std::path::PathBuf;

pub(super) struct Group {
    owner: PathBuf,
    needs_entry_sync: std::sync::Arc<std::sync::atomic::AtomicBool>,
    frames: Vec<u8>,
    wals: BTreeMap<PathBuf, Vec<u8>>,
}

impl TreeState {
    pub(super) fn grouping(&self) -> bool {
        self.group_active.load(std::sync::atomic::Ordering::Acquire)
    }

    pub(super) fn begin_group(&self, journal: &TxnJournal) -> std::io::Result<()> {
        if self.materializing() || self.grouping() {
            return Err(std::io::Error::other(
                "finish the earlier transaction or group first",
            ));
        }
        self.prepare_journal(journal)?;
        *self.group.lock().unwrap_or_else(|e| e.into_inner()) = Some(Group {
            owner: TxnJournal::path(&journal.dir),
            needs_entry_sync: std::sync::Arc::clone(&journal.needs_entry_sync),
            frames: Vec::new(),
            wals: BTreeMap::new(),
        });
        self.group_active
            .store(true, std::sync::atomic::Ordering::Release);
        Ok(())
    }

    pub(super) fn append_journal(
        &self,
        journal: &mut TxnJournal,
        ops: &[TxnOp],
    ) -> std::io::Result<()> {
        if !self.grouping() {
            return journal.append(ops);
        }
        let mut group = self.group.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(group) = group.as_mut() {
            if group.owner != TxnJournal::path(&journal.dir) {
                return Err(std::io::Error::other(
                    "a group must use its owning graph's journal",
                ));
            }
            let frame = journal.frame(ops)?;
            if group.frames.is_empty() {
                group.frames = frame;
            } else {
                group.frames.extend_from_slice(&frame);
            }
            Ok(())
        } else {
            journal.append(ops)
        }
    }

    pub(super) fn append_wal(
        &self,
        wal: &mut Wal,
        inserts: &[[Term; 3]],
        deletes: &[[Term; 3]],
    ) -> std::io::Result<()> {
        if !self.grouping() {
            return wal.append_batch_deferred(inserts, deletes);
        }
        let mut group = self.group.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(group) = group.as_mut() {
            let bytes = Wal::frame(inserts, deletes)?;
            let path = Wal::path(&wal.dir);
            match group.wals.entry(path) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(bytes);
                }
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    entry.get_mut().extend_from_slice(&bytes);
                }
            }
            Ok(())
        } else {
            wal.append_batch_deferred(inserts, deletes)
        }
    }

    pub(super) fn commit_group(&self) -> std::io::Result<()> {
        if self.poisoned() || self.materializing() {
            self.poison();
            return Err(std::io::Error::other(
                "group commit refused: reopen the frozen tree",
            ));
        }
        let group = self
            .group
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .ok_or_else(|| std::io::Error::other("no group is active"))?;
        let result = (|| {
            TxnJournal::write_frames(&group.owner, &group.needs_entry_sync, &group.frames)?;
            for (path, bytes) in &group.wals {
                Wal::write_frame(&mut open_log(path)?, path, bytes)?;
            }
            self.group_active
                .store(false, std::sync::atomic::Ordering::Release);
            self.checkpoint_if_due()
        })();
        if result.is_err() {
            self.poison();
        }
        result
    }
}
