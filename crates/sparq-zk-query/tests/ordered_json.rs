#![cfg(feature = "ordered-json")]

#[test]
fn the_alternate_map_backend_is_actually_selected() {
    let mut map = serde_json::Map::new();
    map.insert("z".into(), serde_json::Value::Null);
    map.insert("a".into(), serde_json::Value::Null);
    assert_eq!(
        map.keys().map(String::as_str).collect::<Vec<_>>(),
        vec!["z", "a"]
    );
}
