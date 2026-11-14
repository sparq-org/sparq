//! Bounded, canonically ordered tables for the research query method.
//! Cells enter the proof path through the authenticated suite adapter.

pub struct TermCell {
    pub bound: bool,
    pub kind: u8,
    pub enc: Field,
    pub key_class: u8,
    pub key: Field,
    pub dt: Field,
    pub lang: Field,
    pub scope: u32,
}

pub fn unbound() -> TermCell {
    TermCell { bound: false, kind: 0, enc: 0, key_class: 0, key: 0, dt: 0, lang: 0, scope: 0 }
}

pub fn same_term(a: TermCell, b: TermCell) -> bool {
    (a.bound == b.bound) & (!a.bound | ((a.kind == b.kind) & (a.enc == b.enc) & (a.scope == b.scope)))
}

pub fn equal_cell(a: TermCell, b: TermCell) -> bool {
    (a.bound == b.bound) & (a.kind == b.kind) & (a.enc == b.enc)
        & (a.key_class == b.key_class) & (a.key == b.key)
        & (a.dt == b.dt) & (a.lang == b.lang) & (a.scope == b.scope)
}

pub fn check_cell(cell: TermCell) {
    if cell.bound {
        assert((cell.kind >= 1) & (cell.kind <= 3), "term kind");
        assert((cell.kind == 2) | (cell.scope == 0), "term scope");
    } else {
        assert(equal_cell(cell, unbound()), "unbound padding");
    }
}

pub struct Table<let R: u32, let V: u32> {
    pub rows: [[TermCell; V]; R],
    pub len: u32,
}

pub fn empty<let R: u32, let V: u32>() -> Table<R, V> {
    Table { rows: [[unbound(); V]; R], len: 0 }
}

pub fn equal_row<let V: u32>(a: [TermCell; V], b: [TermCell; V]) -> bool {
    let mut same = true;
    for v in 0..V { same = same & equal_cell(a[v], b[v]); }
    same
}

pub fn same_row<let V: u32>(a: [TermCell; V], b: [TermCell; V]) -> bool {
    let mut same = true;
    for v in 0..V { same = same & same_term(a[v], b[v]); }
    same
}

// Field comparisons and bounded sorting dominate these table operations.
pub fn row_less<let V: u32>(a: [TermCell; V], b: [TermCell; V]) -> bool {
    let mut less = false;
    let mut equal = true;
    for v in 0..V {
        let cell_less = (!a[v].bound & b[v].bound)
            | ((a[v].bound == b[v].bound) & a[v].bound
                & (a[v].enc.lt(b[v].enc) | ((a[v].enc == b[v].enc) & ((a[v].kind < b[v].kind) | ((a[v].kind == b[v].kind) & (a[v].scope < b[v].scope))))));
        less = less | (equal & cell_less);
        equal = equal & same_term(a[v], b[v]);
    }
    less
}

pub fn check_table<let R: u32, let V: u32>(table: Table<R, V>) {
    assert(table.len <= R, "table capacity");
    for i in 0..R {
        for v in 0..V {
            check_cell(table.rows[i][v]);
            assert((i < table.len) | equal_cell(table.rows[i][v], unbound()), "table padding");
        }
        if i > 0 {
            assert((i >= table.len) | !row_less(table.rows[i], table.rows[i - 1]), "table order");
        }
    }
}

pub fn append<let R: u32, let V: u32>(mut table: Table<R, V>, row: [TermCell; V], active: bool) -> Table<R, V> {
    assert(!active | (table.len < R), "operator capacity");
    for i in 0..R { if active & (table.len == i) { table.rows[i] = row; } }
    table.len = table.len + active as u32;
    table
}

struct SortRow<let V: u32> { row: [TermCell; V], active: bool, ordinal: u32 }

fn exchange<let V: u32>(a: SortRow<V>, b: SortRow<V>) -> (SortRow<V>, SortRow<V>) {
    let less = (b.active & !a.active) | (a.active & b.active &
        (row_less(b.row, a.row) | (same_row(b.row, a.row) & (b.ordinal < a.ordinal))));
    (if less { b } else { a }, if less { a } else { b })
}

pub fn canonicalize<let R: u32, let V: u32>(mut table: Table<R, V>) -> Table<R, V> {
    if R == 4 {
        let mut rows = [[unbound(); V]; 4];
        // Index the generic input only within its own bound.
        for i in 0..R { rows[i] = table.rows[i]; }
        let a = SortRow { row: rows[0], active: 0 < table.len, ordinal: 0 };
        let b = SortRow { row: rows[1], active: 1 < table.len, ordinal: 1 };
        let c = SortRow { row: rows[2], active: 2 < table.len, ordinal: 2 };
        let d = SortRow { row: rows[3], active: 3 < table.len, ordinal: 3 };
        let (a, b) = exchange(a, b); let (c, d) = exchange(c, d);
        let (a, c) = exchange(a, c); let (b, d) = exchange(b, d);
        let (b, c) = exchange(b, c);
        rows = [a.row, b.row, c.row, d.row];
        for i in 0..R { table.rows[i] = rows[i]; }
    } else if (R == 8) | (R == 16) | (R == 32) {
        let mut wires = [SortRow {row:[unbound();V],active:false,ordinal:0};R];
        for i in 0..R {
            wires[i] = SortRow {row:table.rows[i],active:i < table.len,ordinal:i};
        }
        // Bitonic stages compare a total key including the original ordinal.
        // Reversing intermediate runs therefore preserves stable final order.
        for stage in 0..5 {
            let width = 1u32 << (stage + 1);
            if width <= R {
                for pass in 0..5 {
                    if pass <= stage {
                        let stride = width >> (pass + 1);
                        for i in 0..R {
                            let partner = i ^ stride;
                            if i < partner {
                                let (low, high) = exchange(wires[i],wires[partner]);
                                let ascending = (i & width) == 0;
                                wires[i] = if ascending {low} else {high};
                                wires[partner] = if ascending {high} else {low};
                            }
                        }
                    }
                }
            }
        }
        for i in 0..R {table.rows[i] = wires[i].row;}
    } else {
        for _ in 0..R {
            for i in 0..R - 1 {
                if (i + 1 < table.len) & row_less(table.rows[i + 1], table.rows[i]) {
                    let row = table.rows[i];
                    table.rows[i] = table.rows[i + 1];
                    table.rows[i + 1] = row;
                }
            }
        }
    }
    table
}

pub fn assert_table<let R: u32, let V: u32>(expected: Table<R, V>, out: Table<R, V>) {
    check_table(out);
    assert(expected.len == out.len, "operator length");
    for i in 0..R { assert(equal_row(expected.rows[i], out.rows[i]), "operator rows"); }
}

/// Validate supplied operator-fixture cells. Authentication is a separate entry point.
pub fn check_credential_cells<let N: u32>(quads: [[TermCell; 4]; N], len: u32, credential_index: u32) {
    assert(len <= N, "credential capacity");
    for i in 0..N {
        for p in 0..4 {
            check_cell(quads[i][p]);
            assert((i < len) | equal_cell(quads[i][p], unbound()), "credential padding");
        }
        if i < len {
            assert(quads[i][0].bound & quads[i][1].bound & quads[i][2].bound, "triple bound");
            assert((quads[i][0].kind != 3) & (quads[i][1].kind == 1), "triple positions");
            assert(!quads[i][3].bound, "credential named graph not yet supported");
            for p in 0..3 {
                assert((quads[i][p].kind != 2) | (quads[i][p].scope == credential_index + 1), "credential blank-node scope");
            }
        }
    }
}
