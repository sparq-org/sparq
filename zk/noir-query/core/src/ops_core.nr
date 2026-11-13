//! Exact core table relations. Each expected row is constructed inside the gadget.

use crate::table::{TermCell, Table, unbound, same_term, same_row, empty, append, canonicalize, check_table, assert_table, open_credential};

pub struct Dataset<let D: u32> {
    pub rows: [[TermCell; 3]; D],
    pub active: [bool; D],
}

/// Stand-in dataset opening. Suite authentication is supplied by lane A.
pub fn open_dataset<let C: u32, let N: u32, let D: u32>(
    credentials: [[[TermCell; 4]; N]; C], counts: [u32; C], credential_count: u32,
    bounds: [u32; 3],
) -> Dataset<D> {
    std::static_assert(D == C * N, "dataset capacity shape");
    assert((bounds[0] > 0) & (bounds[0] <= C), "credential request bound");
    assert((bounds[1] > 0) & (bounds[1] <= N), "quad request bound");
    assert(credential_count <= bounds[0], "credential count");
    let mut dataset = Dataset {rows: [[unbound(); 3]; D], active: [false; D]};
    for c in 0..C {
        assert(counts[c] <= bounds[1], "quad count");
        assert((c < credential_count) | (counts[c] == 0), "credential padding count");
        open_credential(credentials[c], counts[c], c);
        for n in 0..N {
            dataset.rows[c * N + n] = [credentials[c][n][0], credentials[c][n][1], credentials[c][n][2]];
            dataset.active[c * N + n] = n < counts[c];
        }
    }
    dataset
}

pub fn check_bound<let R: u32, let V: u32>(table: Table<R, V>, bound: u32) {
    assert((bound > 0) & (bound <= R), "row request bound");
    assert(table.len <= bound, "row request capacity");
}

pub fn check_parameters<let P: u32>(parameters: [TermCell; P]) {
    for p in 0..P {
        crate::table::check_cell(parameters[p]);
        assert(parameters[p].bound & (parameters[p].scope == 0), "public constant");
    }
}

/// Bind the request's digest through a public output of the generated program.
pub fn request_binding(request_digest: [u8; 32]) -> Field {
    let mut high: Field = 0;
    let mut low: Field = 0;
    for i in 0..16 {
        high = high * 256 + request_digest[i] as Field;
        low = low * 256 + request_digest[i + 16] as Field;
    }
    compose_core::hashes::h3(0x4e51524551, high, low)
}

pub fn unit<let R: u32>(out: Table<R, 0>) {
    assert_table(append(empty(), [], true), out);
}

pub fn values<let P: u32, let R: u32, let V: u32>(cells: [[TermCell; V]; P], out: Table<R, V>) {
    let mut expected = empty();
    for i in 0..P { expected = append(expected, cells[i], true); }
    assert_table(canonicalize(expected), out);
}

/// Pattern slots are column indices, with V denoting a public constant.
pub fn scan<let D: u32, let R: u32, let V: u32>(
    dataset: [[TermCell; 3]; D], active: [bool; D],
    slots: [u32; 3], constants: [TermCell; 3], out: Table<R, V>,
) {
    let mut expected = empty();
    for i in 0..D {
        let mut duplicate = false;
        for j in 0..D { duplicate = duplicate | ((j < i) & active[j] & same_row(dataset[i], dataset[j])); }
        let mut matches = active[i] & !duplicate;
        let mut row = [unbound(); V];
        for p in 0..3 {
            assert(slots[p] <= V, "scan column");
            if slots[p] == V { matches = matches & same_term(dataset[i][p], constants[p]); }
            for v in 0..V {
                if slots[p] == v {
                    matches = matches & (!row[v].bound | same_term(row[v], dataset[i][p]));
                    row[v] = dataset[i][p];
                }
            }
        }
        expected = append(expected, row, matches);
    }
    assert_table(canonicalize(expected), out);
}

/// Each output column maps to a source column, or its source width for absent.
pub fn join<let A: u32, let B: u32, let R: u32, let VA: u32, let VB: u32, let V: u32>(
    left: Table<A, VA>, right: Table<B, VB>,
    left_cols: [u32; V], right_cols: [u32; V], out: Table<R, V>,
) {
    check_table(left); check_table(right);
    let mut expected = empty();
    for i in 0..A { for j in 0..B {
        let mut row = [unbound(); V];
        let mut compatible = (i < left.len) & (j < right.len);
        for v in 0..V {
            assert((left_cols[v] <= VA) & (right_cols[v] <= VB), "join column");
            let mut a = unbound(); let mut b = unbound();
            for c in 0..VA { if c == left_cols[v] { a = left.rows[i][c]; } }
            for c in 0..VB { if c == right_cols[v] { b = right.rows[j][c]; } }
            compatible = compatible & (!a.bound | !b.bound | same_term(a, b));
            row[v] = if a.bound { a } else { b };
        }
        expected = append(expected, row, compatible);
    } }
    assert_table(canonicalize(expected), out);
}

pub struct Boolean { pub value: bool, pub is_error: bool }

pub fn filter<let A: u32, let R: u32, let V: u32>(
    input: Table<A, V>, predicates: [Boolean; A], out: Table<R, V>,
) {
    check_table(input);
    let mut expected = empty();
    for i in 0..A {
        expected = append(expected, input.rows[i], (i < input.len) & predicates[i].value & !predicates[i].is_error);
    }
    assert_table(expected, out);
}

pub fn project<let A: u32, let R: u32, let VA: u32, let V: u32>(
    input: Table<A, VA>, columns: [u32; V], out: Table<R, V>,
) {
    check_table(input);
    let mut expected = empty();
    for i in 0..A {
        let mut row = [unbound(); V];
        for v in 0..V {
            assert(columns[v] <= VA, "project column");
            for c in 0..VA { if c == columns[v] { row[v] = input.rows[i][c]; } }
        }
        expected = append(expected, row, i < input.len);
    }
    assert_table(canonicalize(expected), out);
}

pub fn distinct<let A: u32, let R: u32, let V: u32>(input: Table<A, V>, out: Table<R, V>) {
    check_table(input);
    let mut expected = empty();
    for i in 0..A {
        let mut duplicate = false;
        for j in 0..A { duplicate = duplicate | ((j < i) & same_row(input.rows[i], input.rows[j])); }
        expected = append(expected, input.rows[i], (i < input.len) & !duplicate);
    }
    assert_table(expected, out);
}

pub fn slice<let A: u32, let R: u32, let V: u32>(input: Table<A, V>, offset: u32, has_limit: bool, limit: u32, out: Table<R, V>) {
    check_table(input);
    let mut expected = empty();
    for i in 0..A {
        // Saturating subtraction avoids overflow for an offset past the input.
        let after_offset = if i >= offset { i - offset } else { 0 };
        expected = append(expected, input.rows[i], (i < input.len) & (i >= offset) & (!has_limit | (after_offset < limit)));
    }
    assert_table(expected, out);
}

pub fn ask<let R: u32, let V: u32>(input: Table<R, V>, result: bool) {
    check_table(input);
    assert(result == (input.len > 0), "ASK result");
}

pub fn select<let R: u32, let V: u32>(input: Table<R, V>, result: Table<R, V>) {
    check_table(input);
    let mut released = input;
    let mut labels: u32 = 0;
    for r in 0..R { for v in 0..V {
        let original = input.rows[r][v];
        if (r < input.len) & original.bound & (original.kind == 2) {
            let mut seen = false;
            let mut label: Field = 0;
            for prior_row in 0..R { for prior_col in 0..V {
                let before = (prior_row < r) | ((prior_row == r) & (prior_col < v));
                if before & same_term(original, input.rows[prior_row][prior_col]) {
                    seen = true;
                    label = released.rows[prior_row][prior_col].enc;
                }
            } }
            if !seen { labels = labels + 1; label = labels as Field; }
            released.rows[r][v] = TermCell {
                bound: true, kind: 2, enc: label, key_class: 0, key: 0, dt: 0, lang: 0, scope: 0,
            };
        }
    } }
    assert_table(canonicalize(released), result);
}
