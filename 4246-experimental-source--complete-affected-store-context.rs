// Exact store.rs:169-339
#[derive(Default, Clone)]
struct Overlay {
    added: Vec<[Id; 3]>,
    deleted: FxHashSet<[Id; 3]>,
    /// CACHED perm-sorted projections of `added`, indexed by `perm as usize`
    /// (sq-7d3dj.16). See [`Overlay::added_sorted`].
    added_by_perm: [std::sync::OnceLock<Vec<[Id; 3]>>; 6],
    /// [GPT-6 Astra] Lazy deletion projections for range counts (#4246). Keep the
    /// hash set above for merge membership; even SPO needs its own sorted projection.
    /// Full use retains up to `BUILT.len()` vectors, each with `deleted.len()`
    /// twelve-byte rows plus capacity slack, alongside that hash set. The sole
    /// production in-place mutator is `TripleStore::apply_delta`, which invalidates
    /// these projections when a tombstone is inserted or removed.
    #[cfg(feature = "overlay-deleted-projections")]
    deleted_by_perm: [std::sync::OnceLock<Vec<[Id; 3]>>; 6],
}

impl Overlay {
    /// All of `added` projected into `perm` column order and SORTED in it — computed
    /// ONCE per permutation and reused until `added` changes (sq-7d3dj.16).
    ///
    /// Built LAZILY on the first scan that needs this permutation rather than eagerly
    /// for all six in [`TripleStore::apply_delta`]: a write batch then stays O(batch)
    /// (it only drops the caches, see [`Overlay::invalidate_added`]) instead of paying
    /// O(6·k log k) per call, and a store only ever materialises the projections its
    /// query mix actually scans — so the memory cost is bounded by the permutations in
    /// use, not a flat 6×. SPO needs no projection or sort at all: `added` is already
    /// canonical-SPO sorted, so that permutation ALIASES it and costs nothing.
    ///
    /// `OnceLock` (not `RefCell`) because scans take `&self` and `TripleStore` must stay
    /// `Sync`; concurrent readers share the synchronized initialization.
    fn added_sorted(&self, perm: Perm) -> &[[Id; 3]] {
        let order = perm.order();
        if order == [0, 1, 2] {
            return &self.added; // SPO: `added` is already the projection, already sorted
        }
        self.added_by_perm[perm as usize].get_or_init(|| {
            let mut rows: Vec<[Id; 3]> =
                self.added.iter().map(|t| [t[order[0]], t[order[1]], t[order[2]]]).collect();
            rows.sort_unstable();
            rows
        })
    }

    /// Drops added projections before any nonempty delta, preserving prior behavior.
    fn invalidate_added(&mut self) {
        for slot in &mut self.added_by_perm {
            slot.take();
        }
    }

    /// [GPT-6 Astra] Drops deleted projections only when the tombstone set changed.
    #[cfg(feature = "overlay-deleted-projections")]
    fn invalidate_deleted(&mut self) {
        for slot in &mut self.deleted_by_perm {
            slot.take();
        }
    }

    /// [GPT-6 Astra] Counts deletions using a lazily sorted projection (#4246).
    /// The first request costs O(d log d) and one additional vector; later requests
    /// use two binary searches. Clone copies initialized vectors by value, and
    /// apply_delta invalidates only the mutated overlay under exclusive access.
    /// Concurrent first readers of the same permutation wait for its one sorting
    /// initializer; the cold sort is serialized for that permutation.
    #[cfg(feature = "overlay-deleted-projections")]
    fn deleted_count(&self, perm: Perm, lo: [Id; 3], hi: [Id; 3]) -> usize {
        if self.deleted.is_empty() {
            return 0;
        }
        let rows = self.deleted_by_perm[perm as usize].get_or_init(|| {
            let order = perm.order();
            let mut rows: Vec<[Id; 3]> = self
                .deleted
                .iter()
                .map(|t| [t[order[0]], t[order[1]], t[order[2]]])
                .collect();
            rows.sort_unstable();
            rows
        });
        rows.partition_point(|r| *r <= hi) - rows.partition_point(|r| *r < lo)
    }

    /// The `added` triples matching the inclusive `[lo, hi]` key range, as rows in
    /// `perm` column order, SORTED in that order. A BORROWED sub-slice of the cached
    /// perm-sorted projection located by two binary searches — O(log k + m) on k
    /// insertions and m matches, and allocation-free.
    fn added_rows(&self, perm: Perm, lo: [Id; 3], hi: [Id; 3]) -> &[[Id; 3]] {
        let rows = self.added_sorted(perm);
        let start = rows.partition_point(|r| *r < lo);
        let end = rows.partition_point(|r| *r <= hi);
        &rows[start..end]
    }

    /// Merges the (perm-sorted) base rows with the overlay for one scan: base rows whose
    /// canonical triple is deleted are dropped, and the matching `added` rows are merge-
    /// interleaved — so the output keeps the permutation's sort order, preserving the
    /// guarantees downstream merge joins rely on. `added` is disjoint from the base, so
    /// no duplicate handling is needed.
    fn merge(&self, base: &[[Id; 3]], perm: Perm, lo: [Id; 3], hi: [Id; 3]) -> Vec<[Id; 3]> {
        let add = self.added_rows(perm, lo, hi);
        let order = perm.order();
        let mut out = Vec::with_capacity(base.len() + add.len());
        let mut ai = 0;
        let check_deleted = !self.deleted.is_empty();
        for &row in base {
            if check_deleted {
                let mut spo = [0; 3];
                spo[order[0]] = row[0];
                spo[order[1]] = row[1];
                spo[order[2]] = row[2];
                if self.deleted.contains(&spo) {
                    continue;
                }
            }
            while ai < add.len() && add[ai] < row {
                out.push(add[ai]);
                ai += 1;
            }
            out.push(row);
        }
        out.extend_from_slice(&add[ai..]);
        out
    }

    /// How many overlay triples fall in the `[lo, hi]` range of `perm` — the exact
    /// correction to a base range count. The `added` side rides the cached perm-sorted
    /// projection. [GPT-6 Astra] Deleted triples use the original linear filter by
    /// default; the experimental feature opts into lazy sorted projections.
    fn count_correction(&self, perm: Perm, lo: [Id; 3], hi: [Id; 3]) -> (usize, usize) {
        let add = self.added_rows(perm, lo, hi).len();
        #[cfg(feature = "overlay-deleted-projections")]
        let del = self.deleted_count(perm, lo, hi);
        #[cfg(not(feature = "overlay-deleted-projections"))]
        let del = {
            let order = perm.order();
            self.deleted
                .iter()
                .filter(|t| {
                    let r = [t[order[0]], t[order[1]], t[order[2]]];
                    r >= lo && r <= hi
                })
                .count()
        };
        (add, del)
    }

    fn is_empty(&self) -> bool {
        self.added.is_empty() && self.deleted.is_empty()
    }

    fn heap_bytes(&self) -> usize {
        // The cached perm-sorted projections are part of the overlay's footprint; SPO
        // aliases `added`; [GPT-6 Astra] deleted projections own all requested perms.
        let cached: usize = self
            .added_by_perm
            .iter()
            .filter_map(|slot| slot.get())
            .map(|rows| rows.capacity() * std::mem::size_of::<[Id; 3]>())
            .sum();
        #[cfg(feature = "overlay-deleted-projections")]
        let cached = cached + self.deleted_by_perm.iter()
            .filter_map(|slot| slot.get())
            .map(|rows| rows.capacity() * std::mem::size_of::<[Id; 3]>())
            .sum::<usize>();
        self.added.capacity() * std::mem::size_of::<[Id; 3]>()
            + self.deleted.capacity() * 13
            + cached
    }
}



// Exact store.rs:946-985
    pub fn apply_delta(&mut self, inserts: &[[Id; 3]], deletes: &[[Id; 3]]) {
        if inserts.is_empty() && deletes.is_empty() {
            return;
        }
        let mut ov = self.overlay.take().unwrap_or_default();
        // [GPT-6 Astra] Added projections retain the existing conservative reset.
        // Preserve deletion projections across inserts/no-ops: only actual tombstone
        // changes require another sort. No overlay read occurs before publication.
        ov.invalidate_added();
        #[cfg(feature = "overlay-deleted-projections")]
        let mut deleted_changed = false;
        for t in deletes {
            if let Ok(i) = ov.added.binary_search(t) {
                ov.added.remove(i); // retract a pending insertion
            } else if self.base_contains(*t) {
                #[cfg(feature = "overlay-deleted-projections")]
                { deleted_changed |= ov.deleted.insert(*t); }
                #[cfg(not(feature = "overlay-deleted-projections"))]
                { ov.deleted.insert(*t); }
            }
        }
        for t in inserts {
            if ov.deleted.remove(t) {
                #[cfg(feature = "overlay-deleted-projections")]
                { deleted_changed = true; }
                continue; // re-insert of a deleted base triple: just undelete
            }
            if self.base_contains(*t) {
                continue; // already present in the base
            }
            if let Err(i) = ov.added.binary_search(t) {
                ov.added.insert(i, *t);
            }
        }
        #[cfg(feature = "overlay-deleted-projections")]
        if deleted_changed {
            ov.invalidate_deleted();
        }
        self.overlay = if ov.is_empty() { None } else { Some(ov) };
    }


// Exact store.rs:1011-1017
    pub fn fork(&self) -> TripleStore {
        TripleStore {
            perms: std::sync::Arc::clone(&self.perms),
            pred_stats: std::sync::Arc::clone(&self.pred_stats),
            overlay: self.overlay.clone(),
        }
    }


// Exact store.rs:1027-1030
    pub fn heap_bytes(&self) -> usize {
        self.perms.iter().map(PermData::heap_bytes).sum::<usize>()
            + self.overlay.as_ref().map_or(0, |ov| ov.heap_bytes())
    }


// Exact store.rs:1130-1155
    fn scan_with(&self, pattern: &Pattern, perm: Perm, lead: usize) -> Scan<'_> {
        let (lo, hi) = Self::bounds(pattern, perm, lead);
        let base = self.perms[perm as usize].rows_in(lo, hi);
        // The single overlay branch on the scan hot path: with no pending updates the
        // base range is returned untouched (borrowed, zero copies); with an overlay the
        // deleted triples are filtered out and the inserted ones merge-interleaved, so
        // the rows keep the permutation's sort order (merge joins stay valid).
        //
        // ZERO-COPY FAST PATH (sq-7d3dj.3) [OPUS-4.8]: even WITH an overlay, most ranges a small
        // overlay does not touch. `count_correction` tells us exactly how many
        // `added`/`deleted` triples fall in this range; when it is `(0, 0)` the
        // overlay contributes nothing here — no `added` row projects into `[lo, hi]` (so
        // nothing is interleaved) and no in-range base row is deleted (so nothing is
        // dropped) — hence `merge` would reproduce `base` verbatim, rows AND sort order.
        // We therefore return the BORROWED base slice directly, restoring allocation-free
        // scans for every untouched range (the read-mostly mutated-server common case)
        // instead of paying the owned merge path — which copies the whole base range into a
        // fresh `Vec` and merge-interleaves the (separately, already perm-sorted) in-range
        // `added` rows. It never re-sorts the range; the cost is the copy plus the interleave.
        let rows = match &self.overlay {
            None => base,
            Some(ov) if ov.count_correction(perm, lo, hi) == (0, 0) => base,
            Some(ov) => std::borrow::Cow::Owned(ov.merge(&base, perm, lo, hi)),
        };
        Scan { rows, perm }
    }


// Exact store.rs:1161-1172
    pub fn estimate(&self, pattern: &Pattern) -> usize {
        let (perm, lead) = Self::choose(pattern);
        let (lo, hi) = Self::bounds(pattern, perm, lead);
        let base = self.perms[perm as usize].count_in(lo, hi);
        match &self.overlay {
            None => base,
            Some(ov) => {
                let (add, del) = ov.count_correction(perm, lo, hi);
                base + add - del
            }
        }
    }

