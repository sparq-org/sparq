//! Journal-first group commit. No materialized WAL bytes escape before the barrier.

use super::*;
use std::collections::BTreeMap;
use std::path::PathBuf;

pub(super) struct Group {
    owner: PathBuf,
    journal: Option<TxnJournal>,
    frames: Vec<u8>,
    wals: BTreeMap<PathBuf, (std::fs::File, Vec<u8>)>,
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
            journal: None,
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
            if group.journal.is_none() {
                group.journal = Some(TxnJournal {
                    file: journal.file.try_clone()?,
                    needs_entry_sync: std::sync::Arc::clone(&journal.needs_entry_sync),
                    dir: journal.dir.clone(),
                    poisoned: false,
                    format_marked: true,
                });
            }
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
                    entry.insert((wal.file.try_clone()?, bytes));
                }
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    entry.get_mut().1.extend_from_slice(&bytes);
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
        let mut group = self
            .group
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .ok_or_else(|| std::io::Error::other("no group is active"))?;
        let result = (|| {
            if let Some(journal) = group.journal.as_mut() {
                journal.write_frames(&group.frames)?;
            }
            for (path, (file, bytes)) in &mut group.wals {
                Wal::write_frame(file, path, bytes)?;
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
