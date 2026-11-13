//! W3C JSON-LD 1.1 HTML content algorithms at json-ld-api 8654ac2.
/// Measured native HTML/XHTML pass floor. May only rise.
/// `#tr006` expects an empty dataset without a script; Process HTML step 4 requires
/// `loading document failed` when extractAllScripts is false. It remains a failure.
pub const FLOOR: usize = 49;
/// Maximum failures at the pinned suite revision. May only fall.
pub const FAIL_CEILING: usize = 1;
