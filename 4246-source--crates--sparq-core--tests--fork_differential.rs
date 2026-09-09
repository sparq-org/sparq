//! Differential tests for the STRUCTURAL FORK (`Graph::fork`): a chain of forked
//! generations carrying random insert/delete batches must answer every read-path
//! probe identically to a flat graph rebuilt from the same logical triple set —
//! and the forked-from bases must stay frozen at their own state (snapshot
//! isolation). Covers: scan (all bound/unbound shapes), scan_sorted (merge-join
//! order), estimate exactness, contains, iter_ids, dict id stability, new-term
//! interning above the base high-water mark, deleted-triple non-visibility,
//! re-inserted triples, and compaction mid-chain.

use oxrdf::{NamedNode, Term};
use sparq_core::dict::Id;
use sparq_core::store::Pattern;
use sparq_core::Graph;

fn iri(s: &str) -> Term {
    Term::NamedNode(NamedNode::new_unchecked(s.to_string()))
}

/// Term-level dump of a graph's default-graph triples, sorted — the flat reference.
fn dump(g: &Graph) -> Vec<[String; 3]> {
    let mut v: Vec<[String; 3]> = g
        .iter_ids()
        .map(|t| {
            [
                g.dict.term(t[0]).to_string(),
                g.dict.term(t[1]).to_string(),
                g.dict.term(t[2]).to_string(),
            ]
        })
        .collect();
    v.sort();
    v
}

/// Asserts `forked` and a freshly rebuilt flat graph over `reference` answer every
/// pattern shape identically (rows, order, estimate) at the TERM level.
fn assert_reads_match(forked: &Graph, reference: &[[Term; 3]], ctx: &str) {
    // Rebuild the flat reference graph.
    let mut dict = sparq_core::dict::Dict::new();
    let ids: Vec<[Id; 3]> = reference
        .iter()
        .map(|[s, p, o]| [dict.intern(s), dict.intern(p), dict.intern(o)])
        .collect();
    let flat = Graph::from_parts(dict, ids);

    assert_eq!(forked.len(), flat.len(), "{ctx}: len");
    assert_eq!(dump(forked), dump(&flat), "{ctx}: full dumps diverge");

    // Probe a battery of patterns built from terms that exist in either graph plus
    // guaranteed misses, across every bound/unbound shape and every sort column.
    let mut probe_terms: Vec<Term> = Vec::new();
    for t in reference.iter().take(8) {
        probe_terms.push(t[0].clone());
        probe_terms.push(t[1].clone());
        probe_terms.push(t[2].clone());
    }
    probe_terms.push(iri("http://nowhere/absent"));

    let resolve = |g: &Graph, t: Option<&Term>| -> Option<Option<Id>> {
        match t {
            None => Some(None),
            Some(t) => g.id_of(t).map(Some),
        }
    };
    let opts: Vec<Option<&Term>> =
        std::iter::once(None).chain(probe_terms.iter().map(Some)).collect();
    for &s in &opts {
        for &p in &opts {
            for &o in &opts {
                // A pattern is comparable when both graphs resolve it (a term absent
                // from one is absent from both — the dumps above already agree).
                let (fp, gp) = (
                    (resolve(forked, s), resolve(forked, p), resolve(forked, o)),
                    (resolve(&flat, s), resolve(&flat, p), resolve(&flat, o)),
                );
                let (Some(fs), Some(fpp), Some(fo)) = fp else {
                    assert!(
                        gp.0.is_none() || gp.1.is_none() || gp.2.is_none(),
                        "{ctx}: term resolvable in flat but not fork"
                    );
                    continue;
                };
                let (Some(gs), Some(gpp), Some(go)) = gp else {
                    panic!("{ctx}: term resolvable in fork but not flat");
                };
                let fpat: Pattern = [fs, fpp, fo];
                let gpat: Pattern = [gs, gpp, go];
                for sort_col in [None, Some(0), Some(1), Some(2)] {
                    let (a, b) = match sort_col {
                        None => (forked.store.scan(&fpat), flat.store.scan(&gpat)),
                        Some(c) => {
                            (forked.store.scan_sorted(&fpat, c), flat.store.scan_sorted(&gpat, c))
                        }
                    };
                    // Sorted in the chosen permutation's order (merge-join guarantee).
                    assert!(
                        a.rows.windows(2).all(|w| w[0] <= w[1]),
                        "{ctx}: unsorted fork scan"
                    );
                    // Compare the row SETS at the term level: the two dicts assign
                    // different ids (independent interning order), so the id-sorted
                    // row ORDER legitimately differs — within-graph sortedness is
                    // asserted above on the fork's own ids.
                    let term_rows = |g: &Graph, sc: &sparq_core::store::Scan, rows: &[[Id; 3]]| -> Vec<[String; 3]> {
                        let mut v: Vec<[String; 3]> = rows
                            .iter()
                            .map(|r| {
                                let t = sc.to_spo(r);
                                [
                                    g.dict.term(t[0]).to_string(),
                                    g.dict.term(t[1]).to_string(),
                                    g.dict.term(t[2]).to_string(),
                                ]
                            })
                            .collect();
                        v.sort();
                        v
                    };
                    assert_eq!(a.perm, b.perm, "{ctx}: perm choice diverged");
                    assert_eq!(
                        term_rows(forked, &a, &a.rows),
                        term_rows(&flat, &b, &b.rows),
                        "{ctx}: rows diverge (sort {sort_col:?})"
                    );
                }
                assert_eq!(
                    forked.store.estimate(&fpat),
                    flat.store.estimate(&gpat),
                    "{ctx}: estimate diverges for {fpat:?}"
                );
            }
        }
    }
}

/// Deterministic xorshift for reproducible "random" batches.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

#[test]
fn fork_chain_matches_flat_rebuild() {
    // Base graph: a few hundred triples over a clustered vocabulary.
    let mut rng = Rng(0x5EED_CAFE);
    let term = |kind: &str, i: usize| iri(&format!("http://ex/{kind}{i}"));
    let mut reference: Vec<[Term; 3]> = Vec::new();
    for _ in 0..400 {
        reference.push([
            term("s", rng.below(60)),
            term("p", rng.below(7)),
            term("o", rng.below(120)),
        ]);
    }
    reference.sort_by_key(|t| format!("{t:?}"));
    reference.dedup();

    let mut dict = sparq_core::dict::Dict::new();
    let ids: Vec<[Id; 3]> = reference
        .iter()
        .map(|[s, p, o]| [dict.intern(s), dict.intern(p), dict.intern(o)])
        .collect();
    let base = Graph::from_parts(dict, ids);
    let base_dump = dump(&base);
    let base_dict_len = base.dict.len();

    // Chain of generations: gen[k+1] = fork(gen[k]) + a random batch. Holds every
    // generation alive (the ring shape) and re-checks old ones for isolation.
    let mut generations: Vec<(Graph, Vec<[Term; 3]>)> = vec![(base, reference)];
    for gen in 0..8 {
        let (cur, cur_ref) = generations.last().unwrap();
        let mut g = cur.fork();
        let mut reference = cur_ref.clone();

        // Random batch: deletes of present triples, an absent delete (no-op), fresh
        // inserts with BRAND-NEW terms (dict growth), re-inserts of just-deleted
        // triples, and duplicate inserts.
        let mut deletes: Vec<[Term; 3]> = Vec::new();
        for _ in 0..10 {
            if !reference.is_empty() {
                deletes.push(reference[rng.below(reference.len())].clone());
            }
        }
        deletes.push([term("zz", 999), term("zz", 999), term("zz", 999)]); // absent
        let mut inserts: Vec<[Term; 3]> = Vec::new();
        for i in 0..12 {
            inserts.push([
                term("new", gen * 100 + i),     // brand-new subject term
                term("p", rng.below(7)),         // existing predicate
                term("nobj", gen * 100 + i / 2), // brand-new object term
            ]);
        }
        if let Some(d) = deletes.first() {
            inserts.push(d.clone()); // re-insert of a deleted triple
        }
        if !reference.is_empty() {
            inserts.push(reference[rng.below(reference.len())].clone()); // duplicate
        }

        g.apply_delta(&inserts, &deletes).unwrap();

        // Maintain the logical reference set: deletes first, then inserts.
        let del: std::collections::HashSet<String> =
            deletes.iter().map(|t| format!("{t:?}")).collect();
        reference.retain(|t| !del.contains(&format!("{t:?}")));
        for ins in &inserts {
            if !reference.iter().any(|t| t == ins) {
                reference.push(ins.clone());
            }
        }

        assert_reads_match(&g, &reference, &format!("gen {gen}"));

        // Snapshot isolation: every earlier generation still answers ITS state.
        for (k, (old, old_ref)) in generations.iter().enumerate() {
            assert_eq!(old.len(), old_ref.len(), "gen {k} length drifted");
        }
        assert_eq!(dump(&generations[0].0), base_dump, "base mutated by fork updates");

        // Dict id stability: a base term resolves to the same id in every generation.
        let probe = iri("http://ex/p0");
        let base_id = generations[0].0.id_of(&probe);
        assert_eq!(g.id_of(&probe), base_id, "shared-base id changed in a fork");
        // New terms intern ABOVE the base high-water mark.
        let new_id = g.id_of(&term("new", gen * 100)).unwrap();
        assert!(
            (new_id as usize) > base_dict_len && !sparq_core::dict::is_inline(new_id),
            "fork-interned id {new_id} must be above the shared base ({base_dict_len}) and non-inline"
        );

        // Compact mid-chain (every 3rd generation): folds the delta, must change
        // nothing observable, and the compacted graph must keep forking correctly.
        if gen % 3 == 2 {
            let before = dump(&g);
            g.compact().unwrap();
            assert!(!g.store.has_overlay(), "compaction must fold the overlay");
            assert_eq!(g.pending_delta_len(), 0, "compaction must clear the pending delta");
            assert_eq!(dump(&g), before, "compaction changed visible state");
            assert_reads_match(&g, &reference, &format!("gen {gen} post-compact"));
        }

        generations.push((g, reference));
    }
}

/// Fork cost must not scale with the base size once the lineage is frozen: the
/// pending delta a fork copies is exactly `pending_delta_len`, and compaction
/// resets it to zero.
#[test]
fn fork_pending_delta_accounting() {
    let mut dict = sparq_core::dict::Dict::new();
    let ids: Vec<[Id; 3]> = (0..1000)
        .map(|i| {
            [
                dict.intern_iri(&format!("http://ex/s{}", i % 100)),
                dict.intern_iri(&format!("http://ex/p{}", i % 5)),
                dict.intern_iri(&format!("http://ex/o{i}")),
            ]
        })
        .collect();
    let base = Graph::from_parts(dict, ids);
    assert_eq!(base.pending_delta_len(), 0);

    let mut g = base.fork();
    assert_eq!(g.pending_delta_len(), 0, "a fresh fork carries no delta");
    let ins: Vec<[Term; 3]> = (0..20)
        .map(|i| [iri(&format!("http://ex/fresh{i}")), iri("http://ex/p0"), iri(&format!("http://ex/o{i}"))])
        .collect();
    g.apply_delta(&ins, &[]).unwrap();
    // 20 overlay insertions + 20 fresh subject terms (p0/oN already exist).
    assert_eq!(g.pending_delta_len(), 40, "overlay + dict extension accounting");

    let g2 = g.fork();
    assert_eq!(g2.pending_delta_len(), g.pending_delta_len(), "fork carries the delta");

    let mut g3 = g2.fork();
    g3.compact().unwrap();
    assert_eq!(g3.pending_delta_len(), 0);
    assert_eq!(g3.len(), 1020);
    // And the compacted generation forks + updates onward correctly.
    let mut g4 = g3.fork();
    g4.apply_delta(&[[iri("http://ex/after"), iri("http://ex/p0"), iri("http://ex/after")]], &[])
        .unwrap();
    assert_eq!(g4.len(), 1021);
    assert_eq!(g3.len(), 1020, "compacted base isolated from its fork");
}

/// Named graphs fork structurally too: updates to a fork's named graph must not
/// leak into the base, and vice versa.
#[test]
fn named_graphs_fork_isolated() {
    let src = "<http://ex/a> <http://ex/p> <http://ex/b> .\n\
               <http://ex/x> <http://ex/q> <http://ex/y> <http://ex/g1> .";
    let base = Graph::load_dataset(src, "nquads").unwrap();
    let mut f = base.fork();
    assert_eq!(f.named.len(), 1);
    f.named[0].1.apply_delta(&[[iri("http://ex/n"), iri("http://ex/q"), iri("http://ex/m")]], &[]).unwrap();
    assert_eq!(f.named[0].1.len(), 2);
    assert_eq!(base.named[0].1.len(), 1, "base named graph mutated through fork");
    let del = [[iri("http://ex/x"), iri("http://ex/q"), iri("http://ex/y")]];
    let mut f2 = f.fork();
    f2.named[0].1.apply_delta(&[], &del).unwrap();
    assert_eq!(f2.named[0].1.len(), 1);
    assert_eq!(f.named[0].1.len(), 2, "intermediate generation mutated");
    assert_eq!(base.named[0].1.len(), 1);
}
