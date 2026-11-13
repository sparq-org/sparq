//! Exact property-path relations for the research, externally unaudited query method.
use crate::table::{TermCell, Table, unbound, same_term, same_row, check_table, empty, append, canonicalize, assert_table};

// All membership tests range over the entire bounded relation.
fn contains<let R: u32>(table: Table<R,2>, row: [TermCell;2]) -> bool {
    let mut found = false;
    for i in 0..R { found = found | ((i < table.len) & same_row(table.rows[i],row)); }
    found
}
fn insert<let R: u32>(table: Table<R,2>, row: [TermCell;2], active: bool, distinct: bool) -> Table<R,2> {
    append(table,row,active & (!distinct | !contains(table,row)))
}
pub fn endpoint<let R: u32, let V: u32>(context: Table<R,V>, col: u32, constant: TermCell) -> TermCell {
    check_table(context); assert(context.len <= 1, "path substitution context");
    let mut value = constant;
    for v in 0..V { if (col == v) & (context.len > 0) { value = context.rows[0][v]; } }
    value
}
pub fn graph_active<let D: u32, let C: u32>(active: [bool;D], capacities: [u32;C], uniform: u32, named: bool, credential: u32) -> [bool;D] {
    let mut start: u32 = 0; let mut end: u32 = 0; let mut offset: u32 = 0;
    for c in 0..C {
        let width = if D == C * uniform { uniform } else { capacities[c] };
        if c == credential { start = offset; end = offset + width; }
        offset = offset + width;
    }
    assert(offset == D, "path dataset layout");
    let mut out = active;
    for i in 0..D { out[i] = active[i] & (!named | ((i >= start) & (i < end) & (credential < C))); }
    out
}
pub fn edges<let D: u32, let P: u32, let R: u32>(dataset: [[TermCell;3];D], active: [bool;D], predicates: [TermCell;P], used: u32, negated: bool) -> Table<R,2> {
    assert(used <= P, "path predicate capacity");
    let mut out = empty();
    for i in 0..D {
        let mut duplicate = false;
        for j in 0..D { duplicate = duplicate | ((j < i) & active[j] & same_row(dataset[i],dataset[j])); }
        let mut listed = false;
        for p in 0..P { listed = listed | ((p < used) & same_term(dataset[i][1],predicates[p])); }
        let matches = if negated { !listed } else { listed };
        out = insert(out,[dataset[i][0],dataset[i][2]],active[i] & !duplicate & matches,negated);
    }
    canonicalize(out)
}
pub fn identity<let D: u32, let R: u32>(dataset: [[TermCell;3];D], active: [bool;D], anchors: [TermCell;2]) -> Table<R,2> {
    let mut out = empty();
    for i in 0..D { for p in 0..2 {
        let cell = dataset[i][p*2]; out = insert(out,[cell,cell],active[i],true);
    } }
    for p in 0..2 { out = insert(out,[anchors[p],anchors[p]],anchors[p].bound,true); }
    canonicalize(out)
}
pub fn inverse<let R: u32>(input: Table<R,2>) -> Table<R,2> {
    check_table(input); let mut out = empty();
    for i in 0..R { out = append(out,[input.rows[i][1],input.rows[i][0]],i < input.len); }
    canonicalize(out)
}
pub fn sequence<let R: u32>(left: Table<R,2>, right: Table<R,2>) -> Table<R,2> {
    check_table(left); check_table(right); let mut out = empty();
    for i in 0..R { for j in 0..R {
        out = append(out,[left.rows[i][0],right.rows[j][1]],(i < left.len) & (j < right.len) & same_term(left.rows[i][1],right.rows[j][0]));
    } }
    canonicalize(out)
}
pub fn alternative<let R: u32>(left: Table<R,2>, right: Table<R,2>, distinct: bool) -> Table<R,2> {
    check_table(left); check_table(right); let mut out = empty();
    for i in 0..R { out = insert(out,left.rows[i],i < left.len,distinct); }
    for i in 0..R { out = insert(out,right.rows[i],i < right.len,distinct); }
    canonicalize(out)
}
pub fn optional<let R: u32>(input: Table<R,2>, zero: Table<R,2>) -> Table<R,2> {
    alternative(input,zero,true)
}
pub fn closure<let DEPTH: u32, let R: u32>(input: Table<R,2>, zero: Table<R,2>, include_zero: bool, public_depth: u32) -> Table<R,2> {
    std::static_assert(DEPTH > 0, "compiled path depth");
    assert(public_depth == DEPTH, "public path depth");
    check_table(input); check_table(zero);
    let mut out = alternative(input,empty(),true);
    for _ in 1..DEPTH {
        let prior = out;
        for i in 0..R { for j in 0..R {
            let row = [prior.rows[i][0],input.rows[j][1]];
            out = insert(out,row,(i < prior.len) & (j < input.len) & same_term(prior.rows[i][1],input.rows[j][0]),true);
        } }
    }
    // Check depth d+1 against the complete base relation before adding identities.
    for i in 0..R { for j in 0..R {
        let extends = (i < out.len) & (j < input.len) & same_term(out.rows[i][1],input.rows[j][0]);
        assert(!extends | contains(out,[out.rows[i][0],input.rows[j][1]]), "path depth capacity");
    } }
    if include_zero { out = alternative(canonicalize(out),zero,true); }
    canonicalize(out)
}
pub fn bind<let R: u32, let V: u32>(input: Table<R,2>, anchors: [TermCell;2], slots: [u32;2], captured: [bool;2], enabled: bool, out: Table<R,V>) {
    check_table(input); let mut expected = empty();
    for i in 0..R {
        let mut matches = (i < input.len) & enabled; let mut row = [unbound();V];
        for p in 0..2 {
            assert(slots[p] <= V, "path endpoint column");
            matches = matches & (!anchors[p].bound | same_term(anchors[p],input.rows[i][p]));
            for v in 0..V { if (slots[p] == v) & !captured[p] {
                matches = matches & (!row[v].bound | same_term(row[v],input.rows[i][p])); row[v] = input.rows[i][p];
            } }
        }
        expected = append(expected,row,matches);
    }
    assert_table(canonicalize(expected),out);
}

pub fn op_edges<let D: u32, let P: u32, let R: u32>(dataset: [[TermCell;3];D], active: [bool;D], predicates: [TermCell;P], used: u32, negated: bool, out: Table<R,2>) { assert_table(edges(dataset,active,predicates,used,negated),out); }
pub fn op_inverse<let A: u32, let R: u32>(input: Table<A,2>, out: Table<R,2>) {
    check_table(input); let mut expected = empty();
    for i in 0..A { expected = append(expected,[input.rows[i][1],input.rows[i][0]],i < input.len); }
    assert_table(canonicalize(expected),out);
}
pub fn op_sequence<let R: u32>(left: Table<R,2>, right: Table<R,2>, out: Table<R,2>) { assert_table(sequence(left,right),out); }
pub fn op_alternative<let R: u32>(left: Table<R,2>, right: Table<R,2>, out: Table<R,2>) { assert_table(alternative(left,right,false),out); }
pub fn op_optional<let R: u32>(input: Table<R,2>, zero: Table<R,2>, out: Table<R,2>) { assert_table(optional(input,zero),out); }
pub fn op_closure<let DEPTH: u32, let R: u32>(input: Table<R,2>, zero: Table<R,2>, include_zero: bool, public_depth: u32, out: Table<R,2>) { assert_table(closure::<DEPTH,R>(input,zero,include_zero,public_depth),out); }

pub fn check_depth<let DEPTH: u32>(public_depth: u32) {
    std::static_assert(DEPTH > 0, "compiled path depth");
    assert(public_depth == DEPTH, "public path depth");
}

pub fn restrict<let R: u32>(input: Table<R,2>, anchors: [TermCell;2]) -> Table<R,2> {
    check_table(input); let mut out = empty();
    for i in 0..R {
        let matches = (!anchors[0].bound | same_term(input.rows[i][0],anchors[0]))
            & (!anchors[1].bound | same_term(input.rows[i][1],anchors[1]));
        out = append(out,input.rows[i],(i < input.len) & matches);
    }
    out
}

pub fn enabled<let R: u32, let V: u32>(context: Table<R,V>) -> bool {
    check_table(context); assert(context.len <= 1, "path substitution context");
    context.len > 0
}
