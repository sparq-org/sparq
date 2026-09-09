// Exact crates/sparq-core/src/lib.rs:1384-1386
    pub fn from_parts(dict: Dict, triples: Vec<[Id; 3]>) -> Graph {
        Self::build(dict, triples)
    }


// Exact crates/sparq-core/src/lib.rs:1891-1988
    pub fn open(dir: &std::path::Path) -> std::io::Result<Graph> {
        // [OPUS-4.8] (review 1593) Finish or roll back any compaction directory swap that a
        // crash interrupted, so `dir` is always present (never lost in the rename window).
        recover_compaction(dir)?;
        // [OPUS-4.8] (sq-glw2) Finish or roll back any named-graph DROP sub-tree swap a crash
        // interrupted, so `named/` is always consistent with the manifest before `open_named`.
        recover_named_drop(dir)?;
        let store = TripleStore::open(dir)?;
        // [OPUS-4.8] (review 1325, sub-finding 2) Prefer the mmap dictionary format
        // (`dict-meta.bin`, written by `save_mmap`); fall back to the LEGACY single-file
        // `dict.bin` (written by the older `Dict::save`) so graph directories saved before
        // the mmap dictionary format still open. The legacy dict loads fully into RAM (no
        // mmap), which is the only difference — every term still round-trips.
        let dict = if dir.join("dict-meta.bin").exists() {
            Dict::open_mmap(dir)?
        } else {
            let legacy = dir.join("dict.bin");
            if legacy.exists() {
                Dict::open(&legacy)?
            } else {
                // Neither format present — surface the original mmap error (NotFound on
                // dict-meta.bin), matching the previous behaviour for a corrupt directory.
                Dict::open_mmap(dir)?
            }
        };
        let np = dir.join("numerics.bin");
        let numerics = match std::fs::File::open(&np) {
            Ok(f) if f.metadata()?.len() as usize == dict.len() * std::mem::size_of::<f64>() => {
                // SAFETY: the file is owned by this graph for its lifetime and not mutated.
                NumData::Mapped(unsafe { memmap2::Mmap::map(&f)? }, rustc_hash::FxHashMap::default())
            }
            _ => NumData::Owned(numerics_of(&dict)),
        };
        let tp = dir.join("temporals.bin");
        let temporals = match std::fs::File::open(&tp) {
            Ok(f) if f.metadata()?.len() as usize == dict.len() * 9 => {
                // SAFETY: the file is owned by this graph for its lifetime and not mutated.
                TempData::Mapped(unsafe { memmap2::Mmap::map(&f)? }, rustc_hash::FxHashMap::default())
            }
            // Absent or stale (a graph saved before this cache existed): recompute —
            // backward compatible, like the numerics cache.
            _ => TempData::Owned(temporals_of(&dict)),
        };
        // [OPUS-4.8] (sq-3ui0, gh-45) Restore the named graphs (each opened memory-mapped
        // with its own per-graph WAL) so a save→open round-trip is lossless for the whole
        // dataset. Empty for a default-graph-only / pre-named-graph directory.
        let named = Self::open_named(dir)?;
        let mut g = Graph {
            dict,
            store,
            numerics,
            temporals,
            high_precision_decimal: std::sync::atomic::AtomicU8::new(0),
            named,
            graph_prefix_index: std::sync::Mutex::new(None),
            wal: None,
            txn: None,
        };
        // Replay any write-ahead log into the delta-overlay (recovery after a crash or a
        // plain not-yet-compacted close), stopping cleanly at the first torn record and
        // truncating the log there — then keep the log open for further appends.
        for (insert, t) in Wal::replay(dir)? {
            if insert {
                g.apply_delta_mem(&[t], &[]);
            } else {
                g.apply_delta_mem(&[], &[t]);
            }
        }
        g.wal = Some(Wal::open(dir)?);
        // [OPUS-4.8] (sq-ycle) AFTER the per-graph WALs are replayed (above + per named graph in
        // `open_named`), redo any committed parent-level transaction frame, idempotently. A
        // multi-op UPDATE body's resolved per-slot quad-delta is one atomic `txn.log` frame; if a
        // crash interrupted materialisation after that single fsync, the per-graph WALs hold only a
        // PREFIX of the body — replaying the frame heals the desync. Re-applying records the WALs
        // already materialised is a no-op: `apply_delta` replays with set semantics (re-inserting a
        // present triple / deleting an absent one changes nothing) and `ensure_named` is
        // find-or-create.
        //
        // [OPUS-4.8] (Copilot #135) DURABILITY ORDER (load-bearing): the records MUST be
        // MATERIALISED INTO THE DURABLE PER-GRAPH WAL (`redo_txn_record` -> `apply_delta`, which
        // appends + fsyncs) BEFORE the `txn.log` is truncated. The earlier code applied them with
        // `apply_delta_mem` (IN-MEMORY only) and then truncated the journal, so a record that lived
        // ONLY in `txn.log` — the precise crash window the journal exists for — was applied to this
        // process's memory but written nowhere durable, then ERASED by the truncation, so it did NOT
        // survive the NEXT restart. Re-logging into the WAL first (then truncate) makes the recovered
        // state durable across any number of restarts; a re-crash between WAL-redo and truncation
        // simply redoes the (idempotent) frame again on the next open.
        let txn_records = TxnJournal::replay(dir)?;
        for (insert, slot, t) in txn_records {
            g.redo_txn_record(insert, slot, t).map_err(std::io::Error::other)?;
        }
        // Every committed record is now durably in a per-graph WAL (fsync'd by `apply_delta`); only
        // NOW is it safe to truncate the journal that was the sole durable home of those records.
        let mut journal = TxnJournal::open(dir)?;
        journal.truncate()?;
        g.txn = Some(journal);
        Ok(g)
    }


// Exact crates/sparq-core/src/lib.rs:2885-2903
    pub fn fork(&self) -> Graph {
        Graph {
            dict: self.dict.fork(),
            store: self.store.fork(),
            numerics: self.numerics.fork(),
            temporals: self.temporals.fork(),
            // sq-lr2ii: the fork shares the same values; recompute the guard lazily.
            high_precision_decimal: std::sync::atomic::AtomicU8::new(0),
            named: self.named.iter().map(|(name, g)| (name.clone(), g.fork())).collect(),
            // A fork is a fresh logical copy; rebuild the prefix index lazily on first use.
            graph_prefix_index: std::sync::Mutex::new(None),
            #[cfg(feature = "mmap")]
            wal: None,
            // [OPUS-4.8] (sq-ycle) A fork/snapshot is a logically-independent in-memory copy with
            // NO directory association — no WAL and no redo journal (like `wal: None`).
            #[cfg(feature = "mmap")]
            txn: None,
        }
    }


// Exact crates/sparq-core/src/lib.rs:2924-2926
    pub fn snapshot(&self) -> GraphSnapshot {
        GraphSnapshot { graph: self.fork() }
    }


// Exact crates/sparq-core/src/lib.rs:2947-2957
    pub fn apply_delta(&mut self, inserts: &[[Term; 3]], deletes: &[[Term; 3]]) -> Result<(), String> {
        if inserts.is_empty() && deletes.is_empty() {
            return Ok(());
        }
        #[cfg(feature = "mmap")]
        if let Some(w) = &mut self.wal {
            w.append_batch(inserts, deletes).map_err(|e| format!("WAL append failed: {e}"))?;
        }
        self.apply_delta_mem(inserts, deletes);
        Ok(())
    }


// Exact crates/sparq-core/src/lib.rs:2973-2981
    pub fn insert_triple(
        &mut self,
        subject: impl Into<Term>,
        predicate: impl Into<Term>,
        object: impl Into<Term>,
    ) -> Result<(), String> {
        let triple = [subject.into(), predicate.into(), object.into()];
        self.apply_delta(&[triple], &[])
    }


// Exact crates/sparq-core/src/lib.rs:2990-2998
    pub fn remove_triple(
        &mut self,
        subject: impl Into<Term>,
        predicate: impl Into<Term>,
        object: impl Into<Term>,
    ) -> Result<(), String> {
        let triple = [subject.into(), predicate.into(), object.into()];
        self.apply_delta(&[], &[triple])
    }


// Exact crates/sparq-core/src/lib.rs:3038-3061
    pub fn clear_default_durable(&mut self) -> Result<(), String> {
        #[cfg(feature = "mmap")]
        if self.wal.is_some() {
            // Retract every current default-graph triple as a WAL-logged delete batch.
            let triples: Vec<[Term; 3]> = {
                let scan = self.store.scan(&[None, None, None]);
                scan.rows
                    .iter()
                    .map(|r| {
                        let spo = scan.to_spo(r);
                        [self.dict.term(spo[0]), self.dict.term(spo[1]), self.dict.term(spo[2])]
                    })
                    .collect()
            };
            if !triples.is_empty() {
                self.apply_delta(&[], &triples)?;
            }
            return Ok(());
        }
        // In-memory graph: no WAL/dir to preserve — clear the store content in place. The
        // dictionary is intentionally kept (ids stay stable; the empty store references none).
        self.store = TripleStore::from_triples(Vec::new());
        Ok(())
    }


// Exact crates/sparq-core/src/lib.rs:3076-3084
    pub fn clear_named_durable(&mut self, name: &Term) -> Result<bool, String> {
        match self.named.iter().position(|(n, _)| n == name) {
            Some(i) => {
                self.named[i].1.clear_default_durable()?;
                Ok(true)
            }
            None => Ok(false),
        }
    }


// Exact crates/sparq-core/src/lib.rs:3372-3397
    fn apply_delta_mem(&mut self, inserts: &[[Term; 3]], deletes: &[[Term; 3]]) {
        // A delete only matters if every term resolves — otherwise the triple cannot be
        // present, and deleting must NOT intern the (absent) terms.
        let del_ids: Vec<[Id; 3]> = deletes
            .iter()
            .filter_map(|[s, p, o]| Some([self.id_of(s)?, self.id_of(p)?, self.id_of(o)?]))
            .collect();
        let old_len = self.dict.len();
        let ins_ids: Vec<[Id; 3]> = inserts
            .iter()
            .map(|[s, p, o]| [self.dict.intern(s), self.dict.intern(p), self.dict.intern(o)])
            .collect();
        // Keep the numeric- and temporal-filter caches covering the grown dictionary.
        self.numerics.extend_for(&self.dict, old_len);
        self.temporals.extend_for(&self.dict, old_len);
        // sq-lr2ii: an inserted term may be an f64-inexact decimal. If the sargable-safety
        // guard was memoised as "no such decimal" (1), reset it to recompute over the grown
        // dictionary; a "found" (2) verdict is monotonic (terms are never removed) and stays.
        let _ = self.high_precision_decimal.compare_exchange(
            1,
            0,
            std::sync::atomic::Ordering::Relaxed,
            std::sync::atomic::Ordering::Relaxed,
        );
        self.store.apply_delta(&ins_ids, &del_ids);
    }


// Exact crates/sparq-core/src/lib.rs:2006-2033
    fn redo_txn_record(&mut self, insert: bool, slot: Option<Term>, t: [Term; 3]) -> Result<(), String> {
        match slot {
            None => {
                if insert {
                    self.apply_delta(&[t], &[])?;
                } else {
                    self.apply_delta(&[], &[t])?;
                }
            }
            Some(name) => {
                // An insert into an absent named graph must materialise the graph (find-or-create);
                // a delete from an absent one is a no-op (nothing to retract).
                let idx = if insert {
                    Some(self.ensure_named(&name)?)
                } else {
                    self.named.iter().position(|(n, _)| *n == name)
                };
                if let Some(i) = idx {
                    if insert {
                        self.named[i].1.apply_delta(&[t], &[])?;
                    } else {
                        self.named[i].1.apply_delta(&[], &[t])?;
                    }
                }
            }
        }
        Ok(())
    }


// Exact crates/sparq-core/src/lib.rs:3406-3443
    pub fn compact(&mut self) -> Result<(), String> {
        #[cfg(feature = "mmap")]
        let dir = self.wal.as_ref().map(|w| w.dir.clone());
        // Fold the structural-fork layers first (ids unchanged throughout): named
        // graphs recursively, then this graph's dictionary extension into a fresh
        // frozen base (so the NEXT fork is O(1) again) and the forked caches flat.
        // No-ops on a never-forked graph.
        for (_, g) in &mut self.named {
            g.compact()?;
        }
        if self.dict.is_forked() {
            self.dict = self.dict.compacted();
            let n = self.dict.len();
            let numerics = std::mem::replace(&mut self.numerics, NumData::Sparse(rustc_hash::FxHashMap::default()));
            self.numerics = numerics.fold(n);
            let temporals = std::mem::replace(&mut self.temporals, TempData::Sparse(rustc_hash::FxHashMap::default()));
            self.temporals = temporals.fold(n);
        }
        if self.store.has_overlay() {
            let triples: Vec<[Id; 3]> = {
                let scan = self.store.scan(&[None, None, None]);
                scan.rows.iter().map(|r| scan.to_spo(r)).collect()
            };
            self.store = TripleStore::from_triples(triples);
        } else {
            // Nothing pending: for a directory-backed graph just discard the (no-op) log.
            #[cfg(feature = "mmap")]
            if let Some(w) = &mut self.wal {
                w.truncate().map_err(|e| e.to_string())?;
            }
            return Ok(());
        }
        #[cfg(feature = "mmap")]
        if let Some(dir) = dir {
            self.persist_swap(&dir)?;
        }
        Ok(())
    }


// Exact crates/sparq-core/src/lib.rs:3462-3471
    fn persist_swap(&mut self, dir: &std::path::Path) -> Result<(), String> {
        // Close THIS graph's WAL before the directory swap (its `wal.log` is about to be renamed
        // away with `dir`), then run the shared crash-safe swap, writing `self`'s CURRENT image as
        // the new base. Adopt the re-opened directory-backed graph in place. [OPUS-4.8] (sq-ft7u)
        self.wal = None;
        let reopened = swap_dir_to_new_base(dir, |new_dir| self.save(new_dir))
            .map_err(|e| e.to_string())?;
        *self = reopened;
        Ok(())
    }


// Exact crates/sparq-core/src/lib.rs:3548-3566
    pub fn vacuum(&mut self) -> Result<(), String> {
        #[cfg(feature = "mmap")]
        let dir = self.wal.as_ref().map(|w| w.dir.clone());
        // Re-intern the whole live dataset into a fresh graph with an EMPTY dictionary, so any
        // term orphaned by a prior delete/drop is dropped (it is never re-interned).
        let mut fresh = self.reintern_live()?;
        #[cfg(feature = "mmap")]
        if let Some(dir) = dir {
            // Close THIS graph's WAL before the directory swap (its `wal.log` is about to be
            // renamed away). `persist_swap` writes `fresh`'s image to `dir` and re-opens
            // `fresh` memory-mapped from the new base with a fresh WAL; adopt that re-opened,
            // directory-backed graph back into `self` so subsequent updates WAL-append to it.
            self.wal = None;
            fresh.persist_swap(&dir)?;
        }
        // Adopt the freshly re-interned (and, when persisted, re-opened) image in place.
        std::mem::swap(self, &mut fresh);
        Ok(())
    }


// Exact crates/sparq-core/src/lib.rs:3572-3595
    fn reintern_live(&self) -> Result<Graph, String> {
        // Dump the live default-graph triples as Terms (overlay already merged by `scan`).
        let live: Vec<[Term; 3]> = {
            let scan = self.store.scan(&[None, None, None]);
            scan.rows
                .iter()
                .map(|r| {
                    let spo = scan.to_spo(r);
                    [self.dict.term(spo[0]), self.dict.term(spo[1]), self.dict.term(spo[2])]
                })
                .collect()
        };
        let mut fresh = Graph::from_parts(Dict::new(), Vec::new());
        if !live.is_empty() {
            fresh.apply_delta(&live, &[])?;
        }
        // Named graphs: re-intern each recursively and attach it as an IN-MEMORY sub-graph (no
        // directory/WAL — the parent's `save` writes the whole `named/` sub-tree afresh).
        for (name, sub) in &self.named {
            let sub_fresh = sub.reintern_live()?;
            fresh.named.push((name.clone(), sub_fresh));
        }
        Ok(fresh)
    }


// Exact crates/sparq-core/src/lib.rs:3498-3503
    pub fn restore_into_durable(
        dir: &std::path::Path,
        fresh: Graph,
    ) -> std::io::Result<Graph> {
        swap_dir_to_new_base(dir, |new_dir| fresh.save(new_dir))
    }


// Exact crates/sparq-core/src/lib.rs:3645-3666
fn swap_dir_to_new_base<F>(dir: &std::path::Path, write_new_base: F) -> std::io::Result<Graph>
where
    F: FnOnce(&std::path::Path) -> std::io::Result<()>,
{
    let new_dir = dir.with_extension("compact-new");
    let old_dir = dir.with_extension("compact-old");
    std::fs::remove_dir_all(&new_dir).ok();
    std::fs::remove_dir_all(&old_dir).ok();
    // Build + sync the new base FIRST. A failure here leaves `dir` untouched (fail-closed) —
    // the two renames below have not started, so the old durable store is intact.
    write_new_base(&new_dir)?;
    fsync_dir(&new_dir)?;
    let parent = dir.parent().unwrap_or_else(|| std::path::Path::new("."));
    std::fs::rename(dir, &old_dir)?;
    fsync_dir(parent)?;
    std::fs::rename(&new_dir, dir)?;
    fsync_dir(parent)?;
    // Re-open memory-mapped from the new base (fresh, empty WAL); only then drop the old files.
    let reopened = Graph::open(dir)?;
    std::fs::remove_dir_all(&old_dir).ok();
    Ok(reopened)
}

// Exact crates/sparq-core/src/lib.rs:1679-1701
    fn build(dict: Dict, triples: Vec<[Id; 3]>) -> Graph {
        let store = TripleStore::from_triples(triples);
        let numerics = NumData::Owned(numerics_of(&dict));
        // Unlike the numerics cache (dense f64 = 8 B/term), the dense temporal cells
        // are 16 B/term — go sparse straight away when temporals are rare (usually:
        // none at all -> an empty map, zero memory), keeping the load-time memory
        // metric flat for non-temporal datasets. Temporal-heavy data stays dense.
        let temporals = TempData::Owned(temporals_of(&dict)).into_sparse_if_worthwhile();
        Graph {
            dict,
            store,
            numerics,
            temporals,
            high_precision_decimal: std::sync::atomic::AtomicU8::new(0),
            named: Vec::new(),
            graph_prefix_index: std::sync::Mutex::new(None),
            #[cfg(feature = "mmap")]
            wal: None,
            // [OPUS-4.8] (sq-ycle) In-memory graph: no parent-level redo journal.
            #[cfg(feature = "mmap")]
            txn: None,
        }
    }

// Exact crates/sparq-core/src/lib.rs:1716-1739
    pub fn into_compressed(self) -> Graph {
        let triples: Vec<[Id; 3]> = {
            let scan = self.store.scan(&[None, None, None]);
            scan.rows.iter().map(|r| scan.to_spo(r)).collect()
        };
        Graph {
            store: TripleStore::from_triples_compressed(triples),
            dict: self.dict.into_blob(),
            // The numeric cache is mostly (often entirely) NaN — keep only the real
            // numeric literals when sparse, freeing the dense f64-per-term Vec.
            numerics: self.numerics.into_sparse_if_worthwhile(),
            temporals: self.temporals.into_sparse_if_worthwhile(),
            // sq-lr2ii: re-encoding keeps the same values; recompute the guard lazily.
            high_precision_decimal: std::sync::atomic::AtomicU8::new(0),
            named: self.named,
            graph_prefix_index: std::sync::Mutex::new(None),
            #[cfg(feature = "mmap")]
            wal: self.wal,
            // [OPUS-4.8] (sq-ycle) Re-encoding keeps the directory association — carry the redo
            // journal handle with the WAL, so a compressed graph stays atomically durable.
            #[cfg(feature = "mmap")]
            txn: self.txn,
        }
    }

// Exact crates/sparq-core/src/lib.rs:1752-1764
    pub fn save(&self, dir: &std::path::Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        self.store.save(dir)?; // folds any delta-overlay into the persisted permutations
        self.dict.save_mmap(dir)?; // includes appended (delta-overlay) terms
        // [OPUS-4.8] (sq-7ph8) STREAM the numeric/temporal caches block-by-block straight from
        // the in-RAM cache instead of materialising a whole-dictionary dense intermediate
        // (`dense_numerics`/`dense_temporals`) first — bounding the finalize RSS peak for a
        // SPARSE/FORKED cache (the common non-numeric/non-temporal case).
        let n = self.dict.len();
        stream_write_numerics(&dir.join("numerics.bin"), n, &self.numerics)?;
        stream_write_temporals(&dir.join("temporals.bin"), n, &self.temporals)?;
        self.save_named(dir, false)
    }

// Exact crates/sparq-core/src/lib.rs:1772-1782
    pub fn save_compressed(&self, dir: &std::path::Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        self.store.save_compressed(dir)?; // folds any delta-overlay, like `save`
        self.dict.save_mmap(dir)?;
        // [OPUS-4.8] (sq-7ph8) Stream the caches block-by-block — see `save` for the rationale.
        let n = self.dict.len();
        stream_write_numerics(&dir.join("numerics.bin"), n, &self.numerics)?;
        stream_write_temporals(&dir.join("temporals.bin"), n, &self.temporals)?;
        // [OPUS-4.8] (sq-3ui0) Named graphs are persisted block-compressed too.
        self.save_named(dir, true)
    }

// Exact crates/sparq-core/src/lib.rs:1796-1821
    fn save_named(&self, dir: &std::path::Path, compressed: bool) -> std::io::Result<()> {
        let named_dir = dir.join(NAMED_SUBDIR);
        let manifest = dir.join(NAMED_MANIFEST);
        if self.named.is_empty() {
            // Default-graph-only: ensure no stale named state lingers, then write nothing.
            std::fs::remove_dir_all(&named_dir).ok();
            std::fs::remove_file(&manifest).ok();
            return Ok(());
        }
        // Rewrite the subtree from scratch so removed/renamed graphs cannot survive a re-save.
        std::fs::remove_dir_all(&named_dir).ok();
        std::fs::create_dir_all(&named_dir)?;
        for (i, (_, sub)) in self.named.iter().enumerate() {
            let sub_dir = named_dir.join(i.to_string());
            if compressed {
                sub.save_compressed(&sub_dir)?;
            } else {
                sub.save(&sub_dir)?;
            }
        }
        // Write the manifest LAST (after every sub-graph is durable) so a crash mid-save
        // never leaves a manifest pointing at a missing/partial sub-graph. The presence of
        // a complete `named.bin` is the commit point for the named-graph set.
        let names: Vec<Term> = self.named.iter().map(|(n, _)| n.clone()).collect();
        write_named_manifest(dir, &names)
    }

