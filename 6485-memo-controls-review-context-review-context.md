# Issue6485: source context supplement

This supplement accompanies the immutable corrected-controls report and packet. It adds no executions or production patch.

```json
{
  "source_head": "f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464",
  "prior_bundle_manifest_sha256": "c2ce45e38b13296594d8fbea1e7aa1216af68219817b754a89ea3729319b9d6e",
  "scope_proof": "Coordinated public Graph methods load/from_parts/apply_delta/insert/remove/fork/compact/vacuum maintain or reconstruct numeric caches. apply_delta_mem accepts complete oxrdf Terms and synchronously interns every component; it has no pending/deferred-ID finalization. Builder-only sharded/spill paths finish their IDs before Graph construction/open and memo0.",
  "public_contract_precision": [
    "Graph.dict and Graph.store are public mutable fields. The viewed public docs do not prohibit arbitrary replacement or promise automatic cache resynchronization after it. Do not label all direct edits invalid by fiat.",
    "Direct dictionary replacement/interning can make existing private numeric caches and a computed memo stale even before this proposal. Graph::from_parts is the inspected public path for constructing a fresh coherent Graph from an edited dictionary/triple set.",
    "No newly introduced regression through the inspected coordinated public mutation operations was found. This is source reasoning plus the executed baseline controls; the production length condition is not yet implemented.",
    "A potential observable distinction exists after direct replacement: old exact decimal and new high-precision decimal can share an f64 cache value and dictionary cardinality; stale memo1 currently gets reset incidentally by a later nonempty apply_delta, whereas the proposed length condition would retain it. This is an unexecuted source-derived scenario, not a demonstrated runtime regression; current API docs do not by themselves resolve its support status.",
    "Independent review should decide whether the established coherent-Graph invariant suffices for this narrow change or requires a separate public-field consistency remedy. No automatic API redesign or production edit is authorized here."
  ],
  "smallest_proposal": "Within the coordinated apply_delta_mem path, wrap only the existing state1\u21920 compare_exchange with self.dict.len()!=old_len, leaving all interning/cache extension/store application/state2 logic unchanged. Held for review of the public-field qualification.",
  "included_context": "Public fields/docs, numeric lookup and extension, precision predicate, graph build/open/WAL, apply_delta/apply_delta_mem, compaction/rebuild/fork, complete interning dispatch and push/len, deferred-builder boundary excerpts. Full unchanged core sources remain in the 50-file controls bundle. Builder excerpt prologues/epilogues can be partial; apply_delta_mem, cache extension, memo and synchronous intern dispatch are complete."
}
```

### crates/sparq-core/src/lib.rs:65–87

```rust
    pub dict: Dict,
    pub store: TripleStore,
    /// Parallel to the dictionary: the f64 value of each numeric literal (NaN for
    /// non-numeric terms). Lets the engine evaluate numeric filters / comparisons
    /// / ORDER BY without materialising the term and parsing its string each time
    /// — a lightweight, u32-id-preserving stand-in for QLever's inline ValueIds.
    numerics: NumData,
    /// Parallel to the dictionary: the precomputed comparison key of each
    /// `xsd:dateTime`/`xsd:dateTimeStamp`/`xsd:date` literal (see
    /// [`temporal::Temporal`]). The temporal twin of `numerics`: dateTime
    /// FILTER / ORDER BY / MIN/MAX read the timeline value O(1) from the cache
    /// instead of materialising the term and re-parsing its lexical per row.
    temporals: TempData,
    /// [OPUS-4.8] (sq-lr2ii) Memoised guard against the engine's f64 sargable-FILTER fast
    /// path deciding a comparison wrongly for an f64-INEXACT decimal. `0` = not yet computed,
    /// `1` = known to hold NO such decimal (fast path safe), `2` = holds at least one (the
    /// engine must decline the fast path and use the exact evaluator). Lazily filled by
    /// [`has_high_precision_decimal`](Self::has_high_precision_decimal); reset to `0` by a
    /// delta that appends terms (`apply_delta_mem`) so it is recomputed over the grown
    /// dictionary. Interior-mutable so a shared `&Graph` can populate it; never observable in
    /// results (pure correctness gate).
    high_precision_decimal: std::sync::atomic::AtomicU8,
    /// Named graphs (each a self-contained `Graph`), keyed by their name term. Empty for the
```

### crates/sparq-core/src/lib.rs:185–267

```rust
/// `id`, NaN for non-numeric): owned dense in RAM, mmap'd from disk (out-of-core), or
/// SPARSE — only the numeric terms in a hash map. Most RDF terms are IRIs/strings (NaN),
/// and small integers inline (carrying their own value, never cached), so the dense
/// cache is mostly — often entirely — NaN; the sparse form stores only the few real
/// numeric literals, the right shape for the memory-bound browser store.
enum NumData {
    Owned(Vec<f64>),
    /// The mmap'd dense cache, plus a small side map for terms APPENDED after open
    /// (delta-overlay updates) — the mmap'd file cannot grow, the dictionary can.
    #[cfg(feature = "mmap")]
    Mapped(memmap2::Mmap, rustc_hash::FxHashMap<Id, f64>),
    Sparse(rustc_hash::FxHashMap<Id, f64>),
    /// A FORKED graph's cache: the base graph's cache SHARED immutably (Arc) plus a
    /// small side map for terms interned after the fork — the in-RAM twin of
    /// `Mapped`'s grow-over-immutable-base shape. Lookup: base first, side map as the
    /// fallback (the side map only ever holds ids the base does not cover).
    /// INVARIANT: the inner cache is never itself `Forked` (fork flattens one level).
    Forked { base: std::sync::Arc<NumData>, extra: rustc_hash::FxHashMap<Id, f64> },
}

impl NumData {
    /// The cached numeric value of a 1-based dictionary id, or `None` if it is not a
    /// (cached) numeric literal. The engine's O(1) numeric fast path.
    // [FABLE-5] Keep the dominant dense-cache probe small enough that LTO reliably places it
    // inside numeric gather loops; the larger storage-variant dispatch stays outlined.
    #[inline(always)]
    fn lookup(&self, id: Id) -> Option<f64> {
        if let NumData::Owned(values) = self {
            let value = *values.get((id - 1) as usize)?;
            return (!value.is_nan()).then_some(value);
        }
        self.lookup_non_owned(id)
    }

    #[inline(never)]
    fn lookup_non_owned(&self, id: Id) -> Option<f64> {
        match self {
            NumData::Sparse(m) => m.get(&id).copied(),
            #[cfg(feature = "mmap")]
            NumData::Mapped(_, extra) => match self.as_slice().get((id - 1) as usize) {
                Some(v) if !v.is_nan() => Some(*v),
                Some(_) => None,
                // Beyond the mmap'd dense cache: a term appended after open.
                None => extra.get(&id).copied(),
            },
            NumData::Owned(_) => unreachable!("Owned handled by lookup"),
            NumData::Forked { base, extra } => {
                base.lookup(id).or_else(|| extra.get(&id).copied())
            }
        }
    }

    /// Appends cache entries for freshly interned dictionary ids `old_len+1 ..= dict.len()`
    /// (delta-overlay growth): the dense backing extends in place; the sparse / mmap'd
    /// backings record only the real numeric values in their side map.
    fn extend_for(&mut self, dict: &Dict, old_len: usize) {
        for i in old_len..dict.len() {
            let id = i as Id + 1;
            let v = numeric_of(&dict.term(id));
            match self {
                NumData::Owned(vec) => vec.push(v),
                NumData::Sparse(m) => {
                    if !v.is_nan() {
                        m.insert(id, v);
                    }
                }
                #[cfg(feature = "mmap")]
                NumData::Mapped(_, extra) => {
                    if !v.is_nan() {
                        extra.insert(id, v);
                    }
                }
                NumData::Forked { extra, .. } => {
                    if !v.is_nan() {
                        extra.insert(id, v);
                    }
                }
            }
        }
    }

    /// The cache for a structurally FORKED graph: the base cache shared immutably,
    /// new terms recorded in a per-fork side map. Forking a fork shares the same
```

### crates/sparq-core/src/lib.rs:790–824

```rust

/// [OPUS-4.8] (sq-lr2ii) `true` iff dict id `id` is an `xsd:decimal` literal whose lexical
/// form carries MORE than 15 significant digits — i.e. its f64 image may be inexact, so the
/// engine's f64 sargable FILTER fast path is unsafe for it. Only `xsd:decimal` is checked:
/// integer exactness is handled by the engine's constant-side guard and float/double values
/// are their own f64. See [`Graph::has_high_precision_decimal`].
fn is_high_precision_decimal(dict: &Dict, id: Id) -> bool {
    matches!(dict.term_parts(id),
        dict::TermParts::Lit { value, datatype, lang: None }
            if datatype == xsd::DECIMAL.as_str() && decimal_significant_digits(value) > 15)
}

/// Significant decimal digits of a plain decimal lexical `[+-]?digits(.digits)?`: leading
/// integer zeros and leading fractional zeros (for a value `< 1`) are not significant, e.g.
/// `007.50` -> 3, `0.00123` -> 3, `1.000000000000000001` -> 19. `usize::MAX` for a lexical that
/// is not a plain decimal (treated as unsafe by the `> 15` guard). Kept LOCAL to sparq-core:
/// this crate is the leanest tier and cannot depend on `sparq-substrate`; the count mirrors the
/// engine's `sig_digits` (constant-side guard) so the two round-trip decisions stay consistent.
fn decimal_significant_digits(s: &str) -> usize {
    let s = s.trim();
    let s = s.strip_prefix(['+', '-']).unwrap_or(s);
    let (int, frac) = s.split_once('.').unwrap_or((s, ""));
    if (int.is_empty() && frac.is_empty()) || !int.bytes().chain(frac.bytes()).all(|c| c.is_ascii_digit()) {
        return usize::MAX;
    }
    let int = int.trim_start_matches('0');
    let frac = frac.trim_end_matches('0');
    if int.is_empty() {
        frac.trim_start_matches('0').len()
    } else {
        int.len() + frac.len()
    }
}

/// True for `xsd:integer` and its derived integer subtypes (NOT decimal/double/float) —
```

### crates/sparq-core/src/lib.rs:1380–1387

```rust

    /// Builds a graph from an already-interned dictionary + triple set (e.g. after opt-in
    /// reasoning materialized additional triples). Public counterpart of the internal
    /// `build`.
    pub fn from_parts(dict: Dict, triples: Vec<[Id; 3]>) -> Graph {
        Self::build(dict, triples)
    }

```

### crates/sparq-core/src/lib.rs:1666–1703

```rust
            // Join parse first (it feeds stage 3 — surface a parse error), then the producer.
            parser.join().map_err(|_| "parse thread panicked".to_string())??;
            producer.join().map_err(|_| "read thread panicked".to_string())?
        })?;
        if sharded {
            let (dict, ids) = finish_sharded(sd, all);
            return Ok(Self::build(dict, ids));
        }
        Ok(Self::build(global, all))
    }

    /// Builds the store + numeric cache from interned triples (shared by the
    /// string and streaming loaders).
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

    /// Like [`load_str`](Self::load_str) but stores the permutation indexes
```

### crates/sparq-core/src/lib.rs:1910–1963

```rust
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
```

### crates/sparq-core/src/lib.rs:2600–2645

```rust
            // (`consolidate`) is a LATER phase, reported separately below, so `None` here.
            build_timing::report(
                "parse+route+stage done",
                build_timing::PathKind::Pipelined,
                None,
                _t_build.elapsed().as_secs_f64(),
            );
        }

        // Phases 2-4: external dedup/rank. The dictionary files and the numeric/temporal
        // caches are STREAM-written in final-id order here, so no dict-finalize thread
        // (and no resident dictionary) exists later.
        let t_cons = std::time::Instant::now();
        let plan = dictspill::consolidate(interner, dir, &tmp, cfg)?;
        if build_timing::enabled() {
            eprintln!("[build-timing] dict consolidate (spilled): {:.2}s", t_cons.elapsed().as_secs_f64());
        }

        // Phase 5: remap the staged triples to final dense ids, spilling SPO-sorted runs.
        let mut buf: Vec<[Id; 3]> = Vec::with_capacity(chunk.min(1 << 24));
        let mut runs: Vec<std::path::PathBuf> = Vec::new();
        dictspill::remap_staged(plan, &mut buf, &mut runs, &tmp, chunk)?;
        extsort::spill_run(&mut buf, &mut runs, &tmp).map_err(|e| e.to_string())?;
        drop(buf); // free the chunk buffer before the k-way merge + sibling sorts

        // Merge the SPO runs (ids are FINAL already — no `remap_perm_file` pass needed).
        let spo_path = dir.join(format!("perm{}.bin", Perm::Spo as usize));
        extsort::kway_merge(&runs, &spo_path).map_err(|e| e.to_string())?;
        if build_timing::enabled() {
            eprintln!("[build-timing] kway_merge SPO done | {:.2}s wall to here", _t_build.elapsed().as_secs_f64());
        }
        for r in &runs {
            std::fs::remove_file(r).ok();
        }

        // Sibling permutations: the same concurrent external sorts as
        // `build_external_opts` (minus its dict-finalize thread).
        let (map, n) = extsort::map_perm(&spo_path).map_err(|e| e.to_string())?;
        // SAFETY: perm0 is a whole number of [u32;3] rows written above; map outlives the loop.
        let spo: &[[Id; 3]] =
            unsafe { std::slice::from_raw_parts(map.as_ptr().cast::<[Id; 3]>(), n) };
        let siblings: Vec<Perm> = BUILT.iter().copied().filter(|&p| p != Perm::Spo).collect();
        // [OPUS-4.8] sq-vkz7: opt-in compressed build, same as `build_external_opts` —
        // siblings write `SPQCPRM1` directly; SPO is re-encoded after they finish.
        let compressed = build_compressed_perms();
        let sib_sort = |perm: Perm, sub: &std::path::Path, per: usize| -> Result<(), String> {
```

### crates/sparq-core/src/lib.rs:2730–2757

```rust
    /// constant `1`, yet is not equal to it). The engine consults this at the sargable-decision
    /// point to DECLINE that fast path and fall back to the exact general evaluator; a graph
    /// without any such decimal keeps the fast path (a decimal of `<= 15` significant digits
    /// round-trips through f64 unambiguously, and a large-integer collision needs a `> 15`-digit
    /// CONSTANT, which the engine already declines). Integers/float/double are exempt: an
    /// integer's exactness is the constant-side guard's job and a float/double's value IS its f64.
    ///
    /// Memoised (see `high_precision_decimal`): the first call scans the graph's numeric terms
    /// once (short-circuiting on the first offender); later calls are an atomic load. A delta
    /// that appends terms resets the memo so it is recomputed over the grown dictionary. This is
    /// a CONSERVATIVE, graph-wide gate — one offending decimal declines the numeric fast path for
    /// every comparison on the graph — chosen for safety (correctness is never at risk; only the
    /// pushdown optimisation is skipped for affected graphs).
    pub fn has_high_precision_decimal(&self) -> bool {
        use std::sync::atomic::Ordering::Relaxed;
        match self.high_precision_decimal.load(Relaxed) {
            2 => true,
            1 => false,
            _ => {
                let found = (1..=self.dict.len() as Id)
                    .any(|id| self.numerics.lookup(id).is_some() && is_high_precision_decimal(&self.dict, id));
                self.high_precision_decimal.store(if found { 2 } else { 1 }, Relaxed);
                found
            }
        }
    }

    pub fn len(&self) -> usize {
```

### crates/sparq-core/src/lib.rs:2883–2902

```rust
    /// generation pattern is an in-memory serving construct — durability stays with
    /// whatever owns the base.
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
```

### crates/sparq-core/src/lib.rs:2936–2959

```rust

    // ---- Incremental updates (T17): delta-overlay + WAL durability ------------------

    /// Applies an incremental update batch — `deletes` first, then `inserts` (SPARQL's
    /// DELETE/INSERT application order) — through the store's DELTA-OVERLAY: O(batch)
    /// work instead of the O(n) full rebuild. New terms are interned APPEND-ONLY (the
    /// dictionary grows; existing ids never change), so readers of existing ids are
    /// unaffected. For a directory-backed graph (opened via [`open`](Self::open)) the
    /// batch is appended to the write-ahead log and fsync'd BEFORE it is applied, so a
    /// crash replays it on the next open. Fold the overlay back into the immutable base
    /// periodically with [`compact`](Self::compact).
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

    /// [OPUS-4.8] (gh-1122) Insert a SINGLE triple from `oxrdf` terms — the ergonomic
```

### crates/sparq-core/src/lib.rs:3369–3445

```rust

    /// The in-memory half of [`apply_delta`](Self::apply_delta) (no WAL append) — also
    /// the target the WAL replays into on [`open`](Self::open).
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

    /// Folds the delta-overlay into a REBUILT immutable base (the periodic compaction
    /// that keeps scans overlay-free). The dictionary is kept as-is — ids are stable;
    /// terms only referenced by deleted triples linger until a full reload (cheap, and
    /// it keeps compaction O(triples) with no re-interning). For a directory-backed
    /// graph the new base is persisted ATOMICALLY (written to a fresh sibling directory,
    /// then swapped in via rename) and the write-ahead log truncated; the graph re-opens
    /// memory-mapped from the new base.
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

    /// [OPUS-4.8] (sq-x32t) ROLLBACK-SAFE on-disk swap shared by [`compact`](Self::compact) and
```

### crates/sparq-core/src/lib.rs:3460–3473

```rust
    /// re-opened memory-mapped from the new base with a fresh, empty WAL.
    #[cfg(feature = "mmap")]
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

    /// [OPUS-4.8] (sq-ft7u) PUBLIC crash-safe RESTORE-INTO-DURABLE seam. Atomically REPLACE the
```

### crates/sparq-core/src/lib.rs:3538–3595

```rust
    /// The live dataset (default graph + every named graph, recursively) is dumped to `[Term; 3]`
    /// and re-interned into a brand-new [`Graph`] with an empty [`Dict`]; that fresh image is then
    /// swapped in via the SAME rollback-safe `persist_swap` the compaction
    /// uses (atomic, crash-safe, WAL-truncated). The live triple set is preserved EXACTLY
    /// (round-trip). For an in-memory graph it just replaces the dictionary/store in place.
    ///
    /// PHYSICAL-ERASURE SCOPE (honest): this scrubs the engine's own on-disk segments + dictionary;
    /// it cannot reach bytes already copied off-box (filesystem snapshots, block-level COW history,
    /// external backups), which the storage/backup tier must handle per the retention-erasure
    /// runbook.
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

    /// [OPUS-4.8] (sq-x32t) Builds a fresh in-memory [`Graph`] holding exactly this graph's LIVE
    /// triples (default + named, recursively), re-interned into a brand-new [`Dict`] — so terms
    /// orphaned by a delete/drop are absent from the result. Carries NO directory/WAL association
    /// (the caller's [`persist_swap`](Self::persist_swap) gives the swapped-in graph its own).
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
```

### crates/sparq-core/src/lib.rs:4794–4811

```rust

/// Consolidates the sharded dict into one `Dict` (parallel arena move + parallel-hash
/// lookup-table rebuild, so the result serves `lookup`/`intern` like a serially-built
/// dict) and remaps `all` from temporary sharded ids to the final dense ids in parallel.
#[cfg(feature = "parallel")]
fn finish_sharded(sd: dict::ShardedDict, mut all: Vec<[Id; 3]>) -> (Dict, Vec<[Id; 3]>) {
    use rayon::prelude::*;
    let (mut dict, base, stride) = sd.into_merged();
    dict.build_table();
    all.par_iter_mut().for_each(|t| {
        for c in t.iter_mut() {
            *c = dict::remap_sharded(*c, &base, stride);
        }
    });
    (dict, all)
}

/// [OPUS-4.8] (T1) Intern an oxttl-parsed subject (`NamedNode`/`BlankNode`) from its BORROWED
```

### crates/sparq-core/src/dict.rs:27–87

```rust
/// from the id (QLever's value-id idea, kept in `u32` so the index stays compact).
/// Inline integers also sort by value in the permutations (enabling range pruning).
///
/// The `u32` id space is partitioned: dictionary ids `[1, INLINE_BASE)` (≈2.1 billion
/// distinct terms — enough for e.g. full-Wikidata's term count without widening to `u64`,
/// which would double the index), inline integers `[INLINE_BASE, INLINE_BASE + 2^30)`, and
/// the engine's local-vocab ids `[INLINE_BASE + 2^30, 2^32)`. `0` is `NO_ID`.
pub const INLINE_BASE: Id = 1 << 31;
/// The largest value encodable inline (the inline range stays 2^30 wide; bigger integers
/// fall back to the dictionary).
const INLINE_MAX: u32 = (1 << 30) - 1;

/// [OPUS-4.8] (review 1409) On-disk format marker for the mmap dictionary (`dict-meta.bin`).
/// `INLINE_BASE` partitions the `u32` id space, so persisted RAW ids only mean what they say
/// under the SAME partition the file was written with. A file written before `INLINE_BASE`
/// moved (from `1 << 30` to `1 << 31`) encodes inline integers in `[1<<30, 1<<31)`, which the
/// current code would misread as dictionary ids — silent numeric corruption / panics. The
/// header records the partition so `open_mmap` can REJECT a mismatched/legacy store with a
/// clear rebuild-required error instead of silently misinterpreting its ids.
///
/// `dict-meta.bin` previously began with `prefixes.len() as u32` (a small count). This magic
/// is chosen to be distinguishable from any plausible legacy prefix count, so the reader can
/// detect header-less legacy files. ("DMV1" — Dict Meta, V1; little-endian.)
// clippy/dead_code: the on-disk meta header is read/written only by the `mmap`/`dict-spill`
// persistence paths, which are cfg'd out of the default feature set.
#[allow(dead_code)]
pub(crate) const DICT_META_MAGIC: u32 = 0x31_56_4D_44; // b"DMV1" little-endian
/// Bump when the on-disk meta layout changes incompatibly.
#[allow(dead_code)]
pub(crate) const DICT_META_VERSION: u32 = 1;

/// If a literal `value`/`datatype` is a canonical non-negative `xsd:integer` in
/// range, its inline id. Only the canonical lexical form (no leading zeros / sign)
/// inlines, so `"030"^^integer` stays a distinct dictionary term.
#[inline]
fn try_inline_lit(value: &str, datatype: &str) -> Option<Id> {
    if datatype == xsd::INTEGER.as_str() {
        if let Ok(v) = value.parse::<u32>() {
            if v <= INLINE_MAX && v.to_string() == value {
                return Some(INLINE_BASE + v);
            }
        }
    }
    None
}

/// If `term` is a canonical non-negative `xsd:integer` in range, its inline id.
fn try_inline(term: &Term) -> Option<Id> {
    match term {
        Term::Literal(l) => try_inline_lit(l.value(), l.datatype().as_str()),
        _ => None,
    }
}

/// Whether an id encodes an inline integer value. (`INLINE_BASE << 1` would overflow `u32`,
/// so the upper bound is expressed via the inline width.)
#[inline]
pub fn is_inline(id: Id) -> bool {
    id >= INLINE_BASE && id - INLINE_BASE <= INLINE_MAX
}

```

### crates/sparq-core/src/dict.rs:1218–1265

```rust
    }

    #[inline]
    fn intern_prefix(&mut self, prefix: &str) -> u32 {
        if let Some(&id) = self.prefix_ids.get(prefix) {
            return id;
        }
        let id = self.prefixes.len() as u32;
        let boxed: Box<str> = prefix.into();
        self.prefixes.push(boxed.clone());
        self.prefix_ids.insert(boxed, id);
        id
    }

    #[inline]
    fn intern_datatype(&mut self, dt: &str) -> u32 {
        if let Some(&id) = self.datatype_ids.get(dt) {
            return id;
        }
        let id = self.datatypes.len() as u32;
        self.datatypes.push(NamedNode::new_unchecked(dt));
        self.datatype_ids.insert(dt.into(), id);
        id
    }

    /// Assigns the next id to a freshly built `Stored` and indexes it. With a compacted
    /// base (blob/mmap), the new term is APPENDED to the arena above `base` — the dict's
    /// append-only growth path for delta-overlay updates.
    #[inline]
    fn push(&mut self, hash: u64, stored: Stored) -> Id {
        let id = (self.base + self.terms.len()) as Id + 1; // 1-based
        // Enforced in release too: once the (non-inline) dictionary reaches INLINE_BASE
        // distinct terms, new ids would collide with the inline-integer range and decode
        // as integers — silent corruption. 2^31 ≈ 2.1B distinct non-integer terms is the
        // hard capacity limit of the u32 scheme; fail loudly (widen Id to u64).
        assert!(id < INLINE_BASE, "dictionary exceeded the id capacity (2^31 distinct non-integer terms); widen Id to u64");
        self.terms.push(stored);
        let (base, terms, blob, prefixes, datatypes) =
            (self.base, &self.terms, &self.blob, &self.prefixes, &self.datatypes);
        self.table.insert_unique(hash, id, |&i| hash_tabled(i, base, terms, blob, prefixes, datatypes));
        id
    }

    /// Finds a term in the MAPPED base by content hash (binary search of the mmap'd
    /// sorted-hash index, verifying candidates with `eq`). `None` when not mapped or
    /// absent — the caller then consults the table (blob base + appended arena terms).
    #[cfg(feature = "mmap")]
    #[inline]
```

### crates/sparq-core/src/dict.rs:1460–1555

```rust
        }
        self.table.find(hash, |&id| self.tabled_eq_term(id, term)).copied()
    }

    /// Interns an IRI term from its string, returning its id.
    #[inline]
    pub fn intern_iri(&mut self, iri: &str) -> Id {
        let hash = hash_iri(iri);
        if let Some(id) = self.find_iri(hash, iri) {
            return id;
        }
        let (p, suffix) = split_iri(iri);
        let prefix = self.intern_prefix(p);
        self.push(hash, Stored::Iri { prefix, suffix: suffix.into() })
    }

    /// Interns an IRI already split into (prefix, suffix) — used by the parallel-load
    /// merge, where the canonical split is already known (no re-split, no concat).
    #[inline]
    fn intern_iri_parts(&mut self, prefix: &str, suffix: &str) -> Id {
        let hash = hash_iri_parts(prefix, suffix);
        if let Some(id) = self.find_iri_parts(hash, prefix, suffix) {
            return id;
        }
        let prefix = self.intern_prefix(prefix);
        self.push(hash, Stored::Iri { prefix, suffix: suffix.into() })
    }

    /// Interns a literal from its components, returning its id. Canonical small
    /// `xsd:integer`s are encoded inline and never stored.
    #[inline]
    pub fn intern_lit(&mut self, value: &str, datatype: &str, lang: Option<&str>) -> Id {
        if let Some(id) = try_inline_lit(value, datatype) {
            return id;
        }
        let hash = hash_lit(value, datatype, lang);
        if let Some(id) = self.find_lit(hash, value, datatype, lang) {
            return id;
        }
        let datatype = self.intern_datatype(datatype);
        self.push(hash, Stored::Lit { value: value.into(), datatype, lang: lang.map(Into::into) })
    }

    /// Interns a blank node from its label, returning its id.
    #[inline]
    pub fn intern_blank(&mut self, label: &str) -> Id {
        let hash = hash_blank(label);
        if let Some(id) = self.find_blank(hash, label) {
            return id;
        }
        self.push(hash, Stored::Blank(label.into()))
    }

    /// Interns an RDF 1.2 triple term whose components are ALREADY interned in this dict
    /// (ids may include inline-integer ids). Content-addressed by the component ids, so
    /// separately-interned identical triples share one id.
    ///
    /// [OPUS-4.8] `pub(crate)` so the byte-level N-Triples parser (`nt`) can intern a
    /// `<<( s p o )>>` triple-term object STRUCTURALLY (interning s/p/o first, then the
    /// triple by their ids) — the identical path `intern(&Term::Triple(_))` takes — so the
    /// N-Triples and Turtle loaders agree on triple-term content-addressing.
    pub(crate) fn intern_triple_ids(&mut self, ids: [Id; 3]) -> Id {
        let hash = hash_triple_ids(ids);
        if let Some(id) = self.find_triple_ids(hash, ids) {
            return id;
        }
        self.push(hash, Stored::Triple(ids))
    }

    /// Interns a term, returning its id (creating it if new). Dispatches to the
    /// component interners so the `Term` and byte-slice paths share one code path.
    /// An RDF 1.2 triple term is stored STRUCTURALLY: its s/p/o are interned first
    /// (recursing through a nested triple-term object) and the triple holds their ids.
    #[inline]
    pub fn intern(&mut self, term: &Term) -> Id {
        match term {
            Term::NamedNode(n) => self.intern_iri(n.as_str()),
            Term::Literal(l) => self.intern_lit(l.value(), l.datatype().as_str(), lang_with_dir(l).as_deref()),
            Term::BlankNode(b) => self.intern_blank(b.as_str()),
            Term::Triple(t) => {
                let s = match t.subject {
                    oxrdf::NamedOrBlankNode::NamedNode(ref n) => self.intern_iri(n.as_str()),
                    oxrdf::NamedOrBlankNode::BlankNode(ref b) => self.intern_blank(b.as_str()),
                };
                let p = self.intern_iri(t.predicate.as_str());
                let o = self.intern(&t.object);
                self.intern_triple_ids([s, p, o])
            }
        }
    }

    /// Interns a borrowed term (already-split components) without building a `Term` or
    /// concatenating the IRI — the fast path used by the sharded parallel merge for LEAF
    /// terms.
    ///
    /// Triple terms are NOT handled here: `TermParts::Triple` carries ids of the SOURCE
```

### crates/sparq-core/src/dict.rs:2307–2370

```rust
            0
        }
    }

    /// Folds a forked dictionary's extension into a FRESH frozen base (ids
    /// unchanged), so subsequent [`fork`](Self::fork)s are O(1) again — the dict
    /// half of `Graph::compact`. Non-forked dicts are returned unchanged (well,
    /// cloned); O(n) — call it from a compaction path only.
    pub fn compacted(&self) -> Dict {
        if self.frozen.is_none() {
            return self.clone();
        }
        let n = self.len();
        let mut terms: Vec<Stored> = Vec::with_capacity(n);
        for id in 1..=n as Id {
            terms.push(match self.record(id) {
                StoredRef::Iri { prefix, suffix } => Stored::Iri { prefix, suffix: suffix.into() },
                StoredRef::Lit { value, datatype, lang } => {
                    Stored::Lit { value: value.into(), datatype, lang: lang.map(Into::into) }
                }
                StoredRef::Blank(b) => Stored::Blank(b.into()),
                StoredRef::Triple(ids) => Stored::Triple(ids),
            });
        }
        // The fork's prefix/datatype tables are supersets of the frozen base's with
        // identical indices (fork-time clone + append-only growth), so every record
        // copied above stays valid against them.
        let mut flat = Dict {
            prefixes: self.prefixes.clone(),
            prefix_ids: self.prefix_ids.clone(),
            datatypes: self.datatypes.clone(),
            datatype_ids: self.datatype_ids.clone(),
            terms,
            table: HashTable::new(),
            blob: None,
            #[cfg(feature = "mmap")]
            mapped: None,
            base: 0,
            frozen: None,
        };
        flat.build_table();
        Dict {
            prefixes: flat.prefixes.clone(),
            prefix_ids: flat.prefix_ids.clone(),
            datatypes: flat.datatypes.clone(),
            datatype_ids: flat.datatype_ids.clone(),
            terms: Vec::new(),
            table: HashTable::new(),
            blob: None,
            #[cfg(feature = "mmap")]
            mapped: None,
            base: n,
            frozen: Some(std::sync::Arc::new(flat)),
        }
    }

    pub fn len(&self) -> usize {
        // `base` is the blob/mmap'd record count (0 for a plain arena dict); appended
        // (delta-overlay) terms live in the arena above it.
        self.base + self.terms.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
```
