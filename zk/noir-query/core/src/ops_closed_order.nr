//! Research standalone ORDER BY relation over supplied cells; not externally audited.
//! Lexical ordering keys must be supplied by the term-opening gadget.

use crate::table::{TermCell, Table, unbound, empty, append, check_table, check_cell,
    same_term, equal_cell, equal_row, row_less};

pub struct OrderKey { pub column: u32, pub descending: bool }

fn kind_rank(c: TermCell) -> u8 {
    if !c.bound { 0 } else if c.kind == 2 { 1 } else if c.kind == 1 { 2 } else { 3 }
}

pub fn check_order_cell(c: TermCell) {
    check_cell(c);
    if c.bound & (c.kind == 1) {
        assert(c.key_class == 6, "ORDER BY IRI lexical key opening required");
        c.key.assert_max_bit_size::<240>();
    }
    if c.bound & (c.kind == 3) {
        assert(c.key_class != 2, "ORDER BY double arithmetic pending");
        assert(c.key_class != 7, "ORDER BY lexical opening required");
        assert(c.key_class != 127, "ORDER BY comparison capacity");
        assert((c.key_class <= 6), "ORDER BY key class");
        if c.key_class > 0 { c.key.assert_max_bit_size::<240>(); }
    }
}

/// (less, equal, greater); all false means SPARQL leaves the pair unordered.
/// An unordered pair is not replaced with a pairwise encoding comparison: that
/// would create cycles when mixed with value comparisons. sort_rows extends the
/// partial order by choosing the canonical smallest currently eligible row.
pub fn term_relation(a: TermCell, b: TermCell) -> (bool, bool, bool) {
    let mixed_timezone = a.bound & b.bound & (a.kind == 3) & (b.kind == 3) & (a.dt == b.dt)
        & (((a.key_class == 3) & (b.key_class == 4)) | ((a.key_class == 4) & (b.key_class == 3)));
    assert(!mixed_timezone, "ORDER BY mixed timezone comparison pending");
    let ar = kind_rank(a); let br = kind_rank(b);
    let mut less = ar < br;
    let mut greater = ar > br;
    let mut equal = same_term(a, b);
    let comparable = (ar == br) & a.bound & b.bound & (
        (a.kind == 1)
        | ((a.kind == 3) & (a.key_class == b.key_class) & (
            (a.key_class == 1) | (a.key_class == 5)
            | (((a.key_class == 3) | (a.key_class == 4)) & (a.dt == b.dt))
            | ((a.key_class == 6) & (a.lang == 0) & (b.lang == 0) & (a.dt == b.dt))
        ))
    );
    if comparable {
        if (a.kind == 1) | (a.key_class == 6) {
            assert((a.key != b.key) | same_term(a, b), "ORDER BY lexical key collision opening required");
        }
        less = a.key.lt(b.key); greater = b.key.lt(a.key); equal = a.key == b.key;
    }
    (less, equal, greater)
}

fn precedes<let V: u32, let K: u32>(a: [TermCell; V], b: [TermCell; V], keys: [OrderKey; K]) -> bool {
    let mut tied = true; let mut less = false;
    for key in keys {
        let (lt, eq, gt) = term_relation(a[key.column], b[key.column]);
        less = less | (tied & if key.descending { gt } else { lt });
        tied = tied & eq;
    }
    less
}

pub fn check_sequence<let R: u32, let V: u32>(table: Table<R, V>) {
    assert(table.len <= R, "table capacity");
    for i in 0..R { for v in 0..V {
        check_cell(table.rows[i][v]);
        assert((i < table.len) | equal_cell(table.rows[i][v], unbound()), "table padding");
    } }
}

pub fn sort_rows<let A: u32, let R: u32, let V: u32, let K: u32>(
    input: Table<A, V>, keys: [OrderKey; K],
) -> Table<R, V> {
    check_table(input);
    for key in keys {
        assert(key.column < V, "ORDER BY column");
        for i in 0..A { if i < input.len { check_order_cell(input.rows[i][key.column]); } }
    }
    let mut before = [[false; A]; A];
    for i in 0..A { for j in 0..A {
        before[i][j] = (i < input.len) & (j < input.len) & precedes(input.rows[i], input.rows[j], keys);
    } }
    let mut used = [false; A];
    let mut expected = empty();
    for position in 0..A {
        let mut found = false; let mut selected: u32 = 0; let mut row = [unbound(); V];
        for i in 0..A {
            let mut blocked = false;
            for j in 0..A { blocked = blocked | (!used[j] & before[j][i]); }
            let eligible = (i < input.len) & !used[i] & !blocked;
            if eligible & (!found | row_less(input.rows[i], row)) {
                selected = i; row = input.rows[i]; found = true;
            }
        }
        assert((position >= input.len) | found, "ORDER BY comparison cycle");
        for i in 0..A { used[i] = used[i] | (found & (selected == i)); }
        expected = append(expected, row, found);
    }
    expected
}

/// Recomputes the entire deterministic permutation; retains bag multiplicities.
pub fn order_by<let A: u32, let R: u32, let V: u32, let K: u32>(
    input: Table<A, V>, out: Table<R, V>, keys: [OrderKey; K],
) {
    let expected: Table<R, V> = sort_rows(input, keys);
    check_sequence(out);
    assert(expected.len == out.len, "operator length");
    for i in 0..R { assert(equal_row(expected.rows[i], out.rows[i]), "ORDER BY rows"); }
}
