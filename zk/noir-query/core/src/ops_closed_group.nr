//! Standalone research grouping over the complete input; not externally audited.
//! Numeric encoding is a constrained, compile-time callback supplied by the suite.
//! Expression errors are a separate lane, to be wired from expression gadgets.

use crate::table::{TermCell, Table, unbound, same_term, same_row,
    row_less, check_table, check_cell, equal_row};
use crate::ops_closed_order::{term_relation, check_order_cell};

// The shared comparison-key class uses signed, scaled offset form.
global OFFSET: Field = 0x800000000000000000000000000000000000000000000000000000000000;
global SCALE: Field = 1000000000000000000;

/// Tags: COUNT(*)=0, COUNT=1, SUM=2, MIN=3, MAX=4, AVG=5,
/// SAMPLE=6, GROUP_CONCAT=7. COUNT(*) uses column V as its sentinel.
pub struct Aggregate { pub kind: u8, pub column: u32, pub distinct: bool }
pub struct NumericTypes { pub integer_dt: Field, pub decimal_dt: Field }
/// Local numeric-comparison HAVING seam. General expressions wire from lane D.
pub struct Having { pub enabled: bool, pub column: u32, pub comparison: u8, pub rhs: TermCell }

fn same_group<let V: u32, let G: u32>(a: [TermCell; V], b: [TermCell; V], columns: [u32; G]) -> bool {
    let mut same = true;
    for c in columns { same = same & same_term(a[c], b[c]); }
    same
}

fn integer_key(c: TermCell, types: NumericTypes) -> Field {
    assert((c.kind == 3) & (c.key_class == 1) & (c.dt == types.integer_dt), "integer aggregate arithmetic required");
    c.key.assert_max_bit_size::<240>();
    let magnitude = if c.key.lt(OFFSET) { OFFSET - c.key } else { c.key - OFFSET };
    // A range-bounded field quotient can exist only for an exact integer.
    let quotient = magnitude / SCALE;
    quotient.assert_max_bit_size::<180>();
    c.key - OFFSET
}

fn numeric<Env>(key: Field, decimal: bool, types: NumericTypes, encode: fn[Env](Field, bool) -> TermCell) -> TermCell {
    key.assert_max_bit_size::<240>();
    let c = encode(key, decimal);
    check_cell(c);
    assert(c.bound & (c.kind == 3) & (c.key_class == 1) & (c.key == key)
        & (c.dt == if decimal { types.decimal_dt } else { types.integer_dt })
        & (c.lang == 0) & (c.scope == 0), "numeric encoding contract");
    c
}

struct Prepared<let A: u32> {
    values: [TermCell; A], selected: [bool; A], numbers: [Field; A],
    errors: [bool; A], extremum: [bool; A],
}

fn prepare<let A: u32, let V: u32, let G: u32>(
    input: Table<A, V>, columns: [u32; G], errors: [bool; A], spec: Aggregate, types: NumericTypes,
) -> Prepared<A> {
    let mut data = Prepared { values: [unbound(); A], selected: [false; A], numbers: [0; A],
        errors: [false; A], extremum: [false; A] };
    for i in 0..A { for v in 0..V { if v == spec.column { data.values[i] = input.rows[i][v]; } } }
    for i in 0..A {
        let mut duplicate = false;
        for j in 0..A {
            let same = if spec.kind == 0 { same_row(input.rows[i], input.rows[j]) }
                else { same_term(data.values[i], data.values[j]) & (errors[i] == errors[j]) };
            duplicate = duplicate | ((j < i) & (j < input.len) & same_group(input.rows[i], input.rows[j], columns) & same);
        }
        data.selected[i] = (i < input.len) & (!spec.distinct | !duplicate);
        let present = data.selected[i] & data.values[i].bound & !errors[i];
        // Evaluating an unbound aggregate operand is an error, not SQL NULL.
        data.errors[i] = data.selected[i] & (errors[i] | !data.values[i].bound)
            & (spec.kind >= 2) & (spec.kind <= 5);
        if ((spec.kind == 2) | (spec.kind == 5)) & present {
            let numeric_type = (data.values[i].kind == 3) & ((data.values[i].key_class == 1)
                | (data.values[i].key_class == 2) | (data.values[i].key_class == 127));
            if numeric_type { data.numbers[i] = integer_key(data.values[i], types); }
            else { data.errors[i] = true; }
        }
        if ((spec.kind == 3) | (spec.kind == 4)) & present { check_order_cell(data.values[i]); }
    }
    // Compute extrema against complete groups once, before emitting any group.
    for i in 0..A {
        let present = data.selected[i] & data.values[i].bound & !errors[i];
        let mut blocked = false;
        if ((spec.kind == 3) | (spec.kind == 4)) & present {
            for j in 0..A {
                if data.selected[j] & data.values[j].bound & !errors[j] & same_group(input.rows[i], input.rows[j], columns) {
                    let (lt, _, gt) = term_relation(data.values[j], data.values[i]);
                    blocked = blocked | if spec.kind == 3 { lt } else { gt };
                }
            }
        }
        data.extremum[i] = present & !blocked;
    }
    data
}

fn aggregate<Env, let A: u32>(
    members: [bool; A], data: Prepared<A>, spec: Aggregate, types: NumericTypes,
    encode: fn[Env](Field, bool) -> TermCell,
) -> TermCell {
    let mut count: u32 = 0; let mut sum: Field = 0; let mut error = false;
    let mut chosen = unbound(); let mut found = false;
    for i in 0..A {
        let selected = members[i] & data.selected[i];
        // COUNT counts bound operands even when SUM has a numeric type error.
        let present = selected & data.values[i].bound;
        count = count + (if spec.kind == 0 { selected } else { present }) as u32;
        if members[i] { sum = sum + data.numbers[i]; error = error | data.errors[i]; }
        if members[i] & data.extremum[i] & (!found | row_less([data.values[i]], [chosen])) {
            chosen = data.values[i]; found = true;
        }
    }
    let mut result = if (((spec.kind == 3) | (spec.kind == 4)) & !error) | (spec.kind == 6) { chosen } else { unbound() };
    if (spec.kind == 0) | (spec.kind == 1) { result = numeric(OFFSET + (count as Field) * SCALE, false, types, encode); }
    if ((spec.kind == 2) & !error) | ((spec.kind == 5) & (!error | (count == 0))) {
        let decimal = (spec.kind == 5) & (count > 0);
        let divisor = if decimal { count as Field } else { 1 };
        // No rounding or modular quotient can pass the key range check.
        result = numeric(OFFSET + sum / divisor, decimal, types, encode);
    }
    result
}

fn having_keeps<let W: u32>(row: [TermCell; W], having: Having) -> bool {
    let mut keep = true;
    if having.enabled {
        assert(having.column < W, "HAVING column");
        assert(having.comparison <= 4, "HAVING comparison");
        let a = row[having.column]; let b = having.rhs;
        check_cell(b);
        let compatible = a.bound & b.bound & (a.kind == 3) & (b.kind == 3)
            & (a.key_class == b.key_class) & ((a.key_class == 1) | (a.key_class == 5));
        assert(!a.bound | !b.bound | compatible, "HAVING numeric comparison required");
        if compatible { a.key.assert_max_bit_size::<240>(); b.key.assert_max_bit_size::<240>(); }
        let eq = a.key == b.key; let lt = a.key.lt(b.key); let gt = b.key.lt(a.key);
        keep = compatible & if having.comparison == 0 { eq }
            else if having.comparison == 1 { lt } else if having.comparison == 2 { lt | eq }
            else if having.comparison == 3 { gt } else { gt | eq };
    }
    keep
}

fn group_row<Env, let A: u32, let V: u32, let G: u32, let M: u32, let W: u32>(
    input: Table<A, V>, keys: [TermCell; G], columns: [u32; G], specs: [Aggregate; M],
    prepared: [Prepared<A>; M], types: NumericTypes, encode: fn[Env](Field, bool) -> TermCell,
) -> ([TermCell; W], bool) {
    let mut row = [unbound(); W];
    let mut members = [false; A]; let mut found = false;
    for j in 0..A {
        let mut matches = j < input.len;
        for g in 0..G { matches = matches & same_term(keys[g], input.rows[j][columns[g]]); }
        members[j] = matches;
        if matches & !found { for g in 0..G { row[g] = input.rows[j][columns[g]]; } }
        found = found | matches;
    }
    for m in 0..M { row[G + m] = aggregate(members, prepared[m], specs[m], types, encode); }
    (row, found | ((G == 0) & (input.len == 0)))
}

/// Output columns are the grouping columns followed by aggregate results.
/// Empty key lists create one global group, including over an empty input.
/// Each output group is evaluated over all input rows. Unique group keys plus
/// the independently computed number of surviving groups prove completeness.
pub fn group<Env, let A: u32, let R: u32, let V: u32, let G: u32, let M: u32, let W: u32>(
    input: Table<A, V>, out: Table<R, W>, columns: [u32; G], specs: [Aggregate; M],
    errors: [[bool; M]; A], types: NumericTypes, having: Having,
    encode: fn[Env](Field, bool) -> TermCell,
) {
    std::static_assert(W == G + M, "group output shape");
    std::static_assert(A <= 32, "integer accumulator capacity");
    check_table(input);
    for c in columns { assert(c < V, "GROUP BY column"); }
    for m in 0..M {
        assert(specs[m].kind <= 7, "aggregate kind");
        assert(specs[m].kind != 7, "GROUP_CONCAT lexical opening required");
        assert(if specs[m].kind == 0 { specs[m].column == V } else { specs[m].column < V }, "aggregate column");
        for i in 0..A {
            assert((i < input.len) | !errors[i][m], "aggregate error padding");
            if specs[m].kind == 0 { assert(!errors[i][m], "COUNT star error lane"); }
            for v in 0..V { if v == specs[m].column {
                assert(!errors[i][m] | !input.rows[i][v].bound, "aggregate error cell");
            } }
        }
    }
    if having.enabled {
        assert(having.column < W, "HAVING column");
        assert(having.comparison <= 4, "HAVING comparison");
        check_cell(having.rhs);
    }
    let mut prepared = [Prepared { values: [unbound(); A], selected: [false; A], numbers: [0; A],
        errors: [false; A], extremum: [false; A] }; M];
    for m in 0..M {
        let mut errs = [false; A];
        for j in 0..A { errs[j] = errors[j][m]; }
        prepared[m] = prepare(input, columns, errs, specs[m], types);
    }
    let mut count: u32 = 0;
    for i in 0..A + 1 {
        let mut source = [unbound(); V];
        for j in 0..A { if j == i { source = input.rows[j]; } }
        let mut duplicate = false;
        for j in 0..A { duplicate = duplicate | ((j < i) & (j < input.len) & same_group(source, input.rows[j], columns)); }
        let active = ((i < input.len) & !duplicate) | ((G == 0) & (input.len == 0) & (i == A));
        let mut keep = true;
        if having.enabled & active {
            let mut keys = [unbound(); G];
            for g in 0..G { keys[g] = source[columns[g]]; }
            let (row, _): ([TermCell; W], bool) = group_row(input, keys, columns, specs, prepared, types, encode);
            keep = having_keeps(row, having);
        }
        count = count + (active & keep) as u32;
    }
    assert(count <= R, "operator capacity");
    check_table(out);
    assert(count == out.len, "operator length");
    for r in 0..R { if r < out.len {
        let mut keys = [unbound(); G];
        for g in 0..G { keys[g] = out.rows[r][g]; }
        let (expected, exists) = group_row(input, keys, columns, specs, prepared, types, encode);
        assert(exists, "operator rows");
        assert(equal_row(expected, out.rows[r]), "operator rows");
        assert(having_keeps(expected, having), "HAVING rows");
        for prior in 0..R {
            let mut same = true;
            for g in 0..G { same = same & same_term(out.rows[prior][g], keys[g]); }
            assert((prior >= r) | !same, "duplicate output group");
        }
    } }
}
