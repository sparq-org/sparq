// The sparq-py cdylib disables its lib test harness because linking a PyO3
// extension-module harness fails on macOS. Compile the pure renderer directly so
// these tests remain ordinary Rust integration tests with no Python dependency.
#[path = "../src/term_render.rs"]
mod term_render;

use term_render::render_n3;

// sq-bif.40: pin literal-arm precedence and catch-all edge behavior.
#[test]
fn language_takes_precedence_over_datatype() {
    assert_eq!(
        render_n3(
            "literal",
            "x",
            Some("en"),
            Some("http://www.w3.org/2001/XMLSchema#integer"),
            None,
        ),
        "\"x\"@en"
    );
}

#[test]
fn direction_without_language_is_dropped() {
    assert_eq!(render_n3("literal", "x", None, None, Some("ltr")), "\"x\"");
}

#[test]
fn renders_empty_literal() {
    assert_eq!(render_n3("literal", "", None, None, None), "\"\"");
}

#[test]
fn unknown_kind_falls_through_to_literal_rendering() {
    assert_eq!(render_n3("quad", "v", None, None, None), "\"v\"");
}
