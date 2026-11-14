//! Exact core table relations. Each expected row is constructed inside the gadget.

use crate::table::{TermCell, Table, unbound, same_term, same_row, empty, append, canonicalize, check_table, assert_table, check_credential_cells};

pub struct Dataset<let D: u32> {
    pub rows: [[TermCell; 3]; D],
    pub active: [bool; D],
    pub graphs: [TermCell; D],
}

/// The named view retains only facts carrying an authenticated graph name.
/// Default-only method assertions remain available in the complete dataset.
pub fn named_view<let D: u32>(dataset: Dataset<D>) -> Dataset<D> {
    let mut result = dataset;
    for i in 0..D {
        result.active[i] = dataset.active[i] & dataset.graphs[i].bound;
    }
    result
}

/// Supplied-cell assembly for isolated operator fixtures.
pub fn open_dataset<let C: u32, let N: u32, let D: u32>(
    credentials: [[[TermCell; 4]; N]; C], counts: [u32; C], credential_count: u32,
    bounds: [u32; 3],
) -> Dataset<D> {
    std::static_assert(D == C * N, "dataset capacity shape");
    assert((bounds[0] > 0) & (bounds[0] <= C), "credential request bound");
    assert((bounds[1] > 0) & (bounds[1] <= N), "quad request bound");
    assert(credential_count <= bounds[0], "credential count");
    let mut dataset = Dataset {rows: [[unbound(); 3]; D], active: [false; D], graphs: [unbound();D]};
    for c in 0..C {
        assert(counts[c] <= bounds[1], "quad count");
        assert((c < credential_count) | (counts[c] == 0), "credential padding count");
        check_credential_cells(credentials[c], counts[c], c);
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

pub fn check_parameters<let P: u32>(parameters: [TermCell; P], used: u32) {
    assert(used <= P, "parameter shape");
    for p in 0..P {
        crate::table::check_cell(parameters[p]);
        if p < used {
            assert(parameters[p].bound & (parameters[p].scope == 0), "public constant");
        } else {
            assert(crate::table::equal_cell(parameters[p], unbound()), "parameter padding");
        }
    }
}

pub fn check_indices<let I: u32>(indices: [u32; I], used: u32) {
    assert(used <= I, "index shape");
    for i in 0..I { assert((i < used) | (indices[i] == 0), "index padding"); }
}

pub fn unit<let R: u32, let V: u32>(out: Table<R, V>) {
    assert_table(append(empty(), [unbound(); V], true), out);
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
    check_table(out);
    for r in 1..R {
        assert((r>=out.len)|!same_row(out.rows[r],out.rows[r-1]),"scan distinct bindings");
    }
    let mut seen=[false;R];
    let mut metadata=[[[0 as Field;4];V];R];
    for i in 0..D {
        let mut matches=active[i];
        let mut row=[unbound();V];
        for p in 0..3 {
            assert(slots[p]<=V,"scan column");
            if slots[p]==V {matches=matches&same_term(dataset[i][p],constants[p]);}
            for v in 0..V {if slots[p]==v {
                matches=matches&(!row[v].bound|same_term(row[v],dataset[i][p]));
                row[v]=dataset[i][p];
            }}
        }
        let mut found:Field=0;
        for r in 0..R {
            let hit=matches&(r<out.len)&same_row(row,out.rows[r]);
            // The first matching source position supplies each metadata field.
            let first=hit&!seen[r];
            for v in 0..V {let fields=crate::table::metadata_fields(row[v]);
                for f in 0..4 {metadata[r][v][f]=metadata[r][v][f]+first as Field*fields[f];}
            }
            seen[r]=seen[r]|hit;
            found=found+hit as Field;
        }
        assert(found==matches as Field,"scan completeness or capacity");
    }
    for r in 0..R {
        assert((r>=out.len)|seen[r],"scan extra binding");
        for v in 0..V {assert((r>=out.len)|(metadata[r][v]==crate::table::metadata_fields(out.rows[r][v])),"scan first payload");}
    }
}

/// Each output column maps to a source column, or its source width for absent.
pub fn join<let A: u32, let B: u32, let R: u32, let VA: u32, let VB: u32, let V: u32>(
    left: Table<A, VA>, right: Table<B, VB>,
    left_cols: [u32; V], right_cols: [u32; V], out: Table<R, V>,
) {
    check_table(left); check_table(right);
    check_table(out);
    let mut seen=[false;R];
    for i in 0..A {for j in 0..B {
        let mut row=[unbound();V];
        let mut compatible=(i<left.len)&(j<right.len);
        for v in 0..V {
            assert((left_cols[v]<=VA)&(right_cols[v]<=VB),"join column");
            let mut a=unbound();let mut b=unbound();
            for c in 0..VA {if c==left_cols[v] {a=left.rows[i][c];}}
            for c in 0..VB {if c==right_cols[v] {b=right.rows[j][c];}}
            compatible=compatible&(!a.bound|!b.bound|same_term(a,b));
            row[v]=if a.bound {a} else {b};
        }
        let mut found=false;
        let mut metadata=[[0 as Field;4];V];
        // Consume the first unused occurrence in this output identity run.
        // Equal bindings retain the source-pair order and their full payloads.
        for r in 0..R {
            let hit=compatible&(r<out.len)&!seen[r]&!found&same_row(row,out.rows[r]);
            found=found|hit;seen[r]=seen[r]|hit;
            for v in 0..V {let fields=crate::table::metadata_fields(out.rows[r][v]);
                for f in 0..4 {metadata[v][f]=metadata[v][f]+hit as Field*fields[f];}
            }
        }
        assert(found==compatible,"join completeness or capacity");
        for v in 0..V {assert(!compatible|(metadata[v]==crate::table::metadata_fields(row[v])),"join source payload");}
    }}
    for r in 0..R {assert((r>=out.len)|seen[r],"join extra binding");}
}

pub struct Boolean { pub value: bool, pub is_error: bool }

pub fn filter<let A: u32, let R: u32, let V: u32>(
    input: Table<A, V>, predicates: [Boolean; A], out: Table<R, V>,
) {
    let mut selected=[false;A];
    for i in 0..A {selected[i]=predicates[i].value&!predicates[i].is_error;}
    crate::table::assert_selection(input,selected,out);
}

pub fn project<let A: u32, let R: u32, let VA: u32, let V: u32>(
    input: Table<A, VA>, columns: [u32; V], out: Table<R, V>,
) {
    check_table(input);
    let mut expected=empty::<A,V>();expected.len=input.len;
    for i in 0..A {
        let mut row=[unbound();V];
        for v in 0..V {
            assert(columns[v]<=VA,"project column");
            for c in 0..VA {if c==columns[v] {row[v]=input.rows[i][c];}}
        }
        expected.rows[i]=row;
    }
    let mut prefix=true;
    for v in 0..V {prefix=prefix&(columns[v]==v);}
    // A leading-column projection preserves the input's canonical row order.
    if prefix {crate::table::assert_ordered(expected,out);}
    else {crate::table::assert_canonicalized(expected,out);}
}

pub fn distinct<let A: u32, let R: u32, let V: u32>(input: Table<A, V>, out: Table<R, V>) {
    check_table(input);
    let mut expected = empty();
    for i in 0..A {
        let mut duplicate = false;
        // Canonical input order places every equal binding in one contiguous run.
        if i > 0 { duplicate = same_row(input.rows[i], input.rows[i-1]); }
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
        } else {
            released.rows[r][v].key_class = 0;
            released.rows[r][v].key = 0;
        }
    } }
    crate::table::assert_canonicalized(released,result);
}
