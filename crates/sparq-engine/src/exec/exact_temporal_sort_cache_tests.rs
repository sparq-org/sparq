// The comparator consumes prevalidated keys, not dictionary IDs.
use super::*;

#[test]
fn temporal_sort_compares_borrowed_keys_without_dictionary_reparsing() {
    let graph = Graph::load_str("", "nt").unwrap();
    let local = LocalVocab::default();
    let a = SortCell::Temp {
        id: 1,
        key: ExactTemporal::of_lit("2024-01-01T00:00:00.000000001Z", "http://www.w3.org/2001/XMLSchema#dateTime").unwrap(),
    };
    let b = SortCell::Temp {
        id: 2,
        key: ExactTemporal::of_lit("2024-01-01T00:00:00.000000002Z", "http://www.w3.org/2001/XMLSchema#dateTime").unwrap(),
    };
    assert_eq!(cmp_sort_cells(&graph, &local, &a, &b), Ordering::Less);
    assert_eq!(cmp_sort_cells(&graph, &local, &b, &a), Ordering::Greater);
    assert_eq!(cmp_sort_cells(&graph, &local, &a, &a), Ordering::Equal);
}
