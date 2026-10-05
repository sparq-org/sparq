use sparq_core::Graph;
use std::cell::Cell;
use std::fmt::Write;

thread_local! { static PREPARED: Cell<(usize, usize)> = const { Cell::new((0, 0)) }; }
pub(super) fn observe(rows: usize) {
    PREPARED.with(|c| { let (scans, ids) = c.get(); c.set((scans + 1, ids + rows)); });
}
const TEXT: &str = "SELECT ?s WHERE { ?s <urn:peer> <urn:X> ; <urn:status> \"pending\" ; <urn:priority> ?p ; <urn:seq> ?seq ; <urn:a> ?a ; <urn:b> ?b ; <urn:c> ?c } ORDER BY DESC(?p)";

fn graph(prefix: usize, overlay: bool, selective_last: bool) -> Graph {
    let mut ttl = String::new();
    for i in 0..4096 {
        let status = if i < 4096 - prefix { "pending" } else { "done" };
        writeln!(ttl, "<urn:s{i}> <urn:peer> <urn:X> ; <urn:status> \"{status}\" ; <urn:priority> {i} ; <urn:seq> {i} ; <urn:a> {i} ; <urn:b> {i} .").unwrap();
        if !selective_last || i == 4095 { writeln!(ttl, "<urn:s{i}> <urn:c> {i} .").unwrap(); }
    }
    let base = Graph::load_str(&ttl, "turtle").unwrap();
    if !overlay { return base; }
    let mut changed = base.fork();
    let mut deletes = String::from("DELETE DATA {");
    for i in (0..4096).step_by(5) { writeln!(deletes, "<urn:s{i}> <urn:priority> {i} .").unwrap(); }
    deletes.push('}');
    crate::update_in_place(&mut changed, &deletes).unwrap();
    assert_eq!(changed.pending_delta_len(), 820);
    changed
}

#[test]
fn later_variable_preparation_is_avoided_before_block_decline() {
    for overlay in [false, true] {
        let g = graph(2047, overlay, false);
        let full = crate::query(&g, TEXT).unwrap();
        PREPARED.with(|c| c.set((0, 0)));
        let got = crate::query(&g, &format!("{TEXT} LIMIT 1")).unwrap();
        assert_eq!(got.rows, full.rows[..1]);
        assert_eq!(PREPARED.with(Cell::get), (2, 4096 + 2049), "later variable scans/vectors must remain unprepared");
    }
}

#[test]
fn successful_rows_prepare_each_probe_once() {
    let g = graph(0, false, false);
    for k in [1, 32] {
        PREPARED.with(|c| c.set((0, 0)));
        assert_eq!(crate::query(&g, &format!("{TEXT} LIMIT {k}")).unwrap().rows.len(), k);
        let expected = if sparq_core::store::BUILT.contains(&sparq_core::store::Perm::Pso) { (6, 6 * 4096) } else { (2, 2 * 4096) };
        assert_eq!(PREPARED.with(Cell::get), expected);
    }
}

#[test]
fn late_selective_probe_still_declines_before_emitting() {
    let g = graph(0, false, true);
    let full = crate::query(&g, TEXT).unwrap();
    assert_eq!(full.rows.len(), 1);
    let limited = format!("{TEXT} LIMIT 1");
    assert_eq!(crate::query(&g, &limited).unwrap().rows, full.rows);
    let trace = crate::explain_analyze(&g, &limited).unwrap();
    assert!(trace.contains("BGP [binary GOO]"), "late cardinality guard must decline: {trace}");
}
