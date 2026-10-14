//! Pure RDF term rendering used by the Python `Term` wrapper.

pub(crate) const XSD_STRING: &str = "http://www.w3.org/2001/XMLSchema#string";

/// Renders the fields of a Python-facing term in the notation used by `Term.__repr__`.
// sq-ct7gh: keep this function free of PyO3 so the disabled cdylib test
// harness does not prevent direct Rust coverage of rendering and literal escaping.
pub(crate) fn render_n3(
    kind: &str,
    value: &str,
    language: Option<&str>,
    datatype: Option<&str>,
    direction: Option<&str>,
) -> String {
    match kind {
        "uri" => format!("<{value}>"),
        "bnode" => format!("_:{value}"),
        "triple" => value.to_owned(),
        _ => {
            let escaped = value
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\n', "\\n");
            if let Some(language) = language {
                match direction {
                    Some(direction) => format!("\"{escaped}\"@{language}--{direction}"),
                    None => format!("\"{escaped}\"@{language}"),
                }
            } else {
                match datatype {
                    None | Some(XSD_STRING) => format!("\"{escaped}\""),
                    Some(datatype) => format!("\"{escaped}\"^^<{datatype}>"),
                }
            }
        }
    }
}
