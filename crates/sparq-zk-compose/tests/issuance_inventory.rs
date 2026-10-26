//! Inventory of raw graph-commitment signer calls outside test code.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const RAW: [&str; 4] = [
    "sign_commitment", "sign_commitment_with_salt",
    "sign_commitment_with_status", "sign_commitment_with_holder",
];

// Path, signer, argument tokens, expected occurrences, reason.
const ALLOWED: [(&str, &str, &str, usize, &str); 2] = [
    ("sparq-trust/examples/trust_graph_closure.rs", "sign_commitment",
        "& certification_message ( & cert )", 1, "certification; not a graph commitment"),
    ("sparq-trust/examples/admission_cost.rs", "sign_commitment",
        "& certification_message ( & cert )", 2, "certification; not a graph commitment"),
];

#[derive(Debug)]
struct Token<'a> { text: &'a str, line: usize }

// Keep identifiers and punctuation, dropping comments and literal contents.
// Delimiters inside literals must not change the test-item boundary.
fn raw_quote(bytes: &[u8], mut i: usize) -> Option<(usize, usize)> {
    if b"bc".contains(&bytes[i]) { i += 1; }
    if bytes.get(i) != Some(&b'r') { return None; }
    i += 1;
    let start = i;
    while bytes.get(i) == Some(&b'#') { i += 1; }
    (bytes.get(i) == Some(&b'"')).then_some((i, i - start))
}

fn tokens(source: &str) -> Vec<Token<'_>> {
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut line = 1;
    while i < bytes.len() {
        let start = i;
        let start_line = line;
        if bytes[i].is_ascii_whitespace() { i += 1; }
        else if bytes[i..].starts_with(b"//") {
            while i < bytes.len() && bytes[i] != b'\n' { i += 1; }
        } else if bytes[i..].starts_with(b"/*") {
            i += 2;
            let mut depth = 1;
            while i < bytes.len() && depth > 0 {
                if bytes[i..].starts_with(b"/*") { depth += 1; i += 2; }
                else if bytes[i..].starts_with(b"*/") { depth -= 1; i += 2; }
                else { i += 1; }
            }
        } else if let Some((quote, hashes)) = raw_quote(bytes, i) {
            i = quote + 1;
            while i < bytes.len() {
                if bytes[i] == b'"' && bytes.get(i + 1..i + 1 + hashes)
                    .is_some_and(|suffix| suffix.iter().all(|&c| c == b'#')) {
                    i += 1 + hashes; break;
                }
                i += 1;
            }
        } else if bytes[i] == b'"' || (bytes[i] == b'\'' &&
            (bytes.get(i + 2) == Some(&b'\'') || bytes.get(i + 1) == Some(&b'\\'))) {
            let quote = bytes[i]; i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' { i = (i + 2).min(bytes.len()); }
                else if bytes[i] == quote { i += 1; break; }
                else { i += 1; }
            }
        } else {
            i += 1;
            if bytes[start].is_ascii_alphanumeric() || bytes[start] == b'_' {
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
            }
            // Non-ASCII source text is irrelevant to the signer identifiers.
            if source.is_char_boundary(start) && source.is_char_boundary(i) {
                out.push(Token { text: &source[start..i], line: start_line });
            }
        }
        line += bytes[start..i].iter().filter(|&&c| c == b'\n').count();
    }
    out
}

fn close(tokens: &[Token<'_>], start: usize, open: &str, end: &str) -> usize {
    let mut depth = 0;
    for (i, token) in tokens.iter().enumerate().skip(start) {
        if token.text == open { depth += 1; }
        if token.text == end {
            depth -= 1;
            if depth == 0 { return i; }
        }
    }
    panic!("unclosed {open} in source inventory");
}

#[derive(Default)]
struct Scan {
    calls: Vec<(usize, String, String)>,
    test_modules: Vec<String>,
}

fn scan(source: &str) -> Scan {
    let t = tokens(source);
    let mut out = Scan::default();
    let mut i = 0;
    while i < t.len() {
        if t[i].text == "#" && t.get(i + 1).is_some_and(|t| t.text == "[") {
            let end = close(&t, i + 1, "[", "]");
            let attr: Vec<_> = t[i + 2..end].iter().map(|t| t.text).collect();
            let mut depth = 0i32;
            let direct_test = attr.starts_with(&["cfg", "(", "all", "("]) &&
                attr[4..].iter().any(|&token| {
                    let required = depth == 0 && token == "test";
                    if token == "(" { depth += 1; }
                    if token == ")" { depth -= 1; }
                    required
                });
            let test_only = attr == ["test"] || attr == ["cfg", "(", "test", ")"] || direct_test;
            if test_only {
                i = end + 1;
                if t.get(i).is_some_and(|t| t.text == "mod") &&
                    t.get(i + 2).is_some_and(|t| t.text == ";") {
                    out.test_modules.push(t[i + 1].text.to_string());
                }
                while i < t.len() && t[i].text != "{" && t[i].text != ";" { i += 1; }
                if i < t.len() && t[i].text == "{" { i = close(&t, i, "{", "}"); }
                i += 1;
                continue;
            }
        }
        if RAW.contains(&t[i].text) && i > 0 && [".", ":"].contains(&t[i - 1].text) &&
            t.get(i + 1).is_some_and(|t| t.text == "(") {
            let end = close(&t, i + 1, "(", ")");
            out.calls.push((t[i].line, t[i].text.to_string(),
                t[i + 2..end].iter().map(|t| t.text).collect::<Vec<_>>().join(" ")));
        }
        i += 1;
    }
    out
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read inventory directory") {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_str().unwrap();
        if path.is_dir() {
            if !["tests", "target"].contains(&name) { rust_files(&path, out); }
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn non_test_raw_signers_have_an_explicit_issuance_allow_list() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    rust_files(&crates, &mut files);
    files.sort();
    let mut scans = BTreeMap::new();
    let mut test_files = BTreeSet::new();
    for file in files {
        let source = std::fs::read_to_string(&file).expect("read Rust inventory source");
        let scanned = scan(&source);
        for module in &scanned.test_modules {
            let dir = file.parent().unwrap();
            test_files.insert(dir.join(format!("{module}.rs")));
            test_files.insert(dir.join(module).join("mod.rs"));
        }
        scans.insert(file, scanned);
    }
    let mut counts = BTreeMap::new();
    for (file, scanned) in scans {
        if test_files.contains(&file) { continue; }
        let path = file.strip_prefix(&crates).unwrap().to_str().unwrap();
        for (line, signer, args) in scanned.calls {
            let allowed = ALLOWED.iter().position(|&(p, s, a, _, reason)|
                path == p && signer == s && args == a && !reason.is_empty());
            let index = allowed.unwrap_or_else(|| panic!(
                "{path}:{line}: {signer} requires an explicit allow-list entry; use SecretKey::attest for graph issuance"
            ));
            *counts.entry(index).or_insert(0) += 1;
        }
    }
    for (i, &(path, signer, _, expected, reason)) in ALLOWED.iter().enumerate() {
        assert_eq!(counts.get(&i).copied().unwrap_or(0), expected,
            "{path}: {signer} ({reason}) changed; use SecretKey::attest for graph issuance");
    }
}

#[test]
fn inventory_scans_calls_and_excludes_only_test_items_and_literals() {
    let source = r##"
        // key.sign_commitment(&comment);
        const DOC: &str = r#"key.sign_commitment(&literal); }"#;
        const BYTES: &[u8] = br#"" } key.sign_commitment(&literal)"#;
        #[cfg(test)] mod tests { fn f() { key.sign_commitment(&fixture); } }
        #[cfg(all(test, feature = "x"))] mod gated { fn f() { key.sign_commitment(&fixture); } }
        #[cfg(test)] mod external_fixture;
        #[test] fn test() { key.sign_commitment(&fixture); }
        fn production() { key.sign_commitment_with_status(&c, &salt, &status); }
        #[cfg(any(test, feature = "x"))] fn shared() { SecretKey::sign_commitment(&key, &c); }
        #[cfg(all(any(test, feature = "x"), feature = "y"))]
        fn also_shared() { key.sign_commitment(&c); }
        fn holder() { key.sign_commitment_with_holder(&c, &salt, &status, &h); }
        fn salted() { key.sign_commitment_with_salt(&c, &salt); }
    "##;
    let scanned = scan(source);
    assert_eq!(scanned.test_modules, vec!["external_fixture"]);
    let calls = scanned.calls;
    assert_eq!(calls.iter().map(|(_, s, _)| s.as_str()).collect::<Vec<_>>(), vec![
        "sign_commitment_with_status", "sign_commitment", "sign_commitment", "sign_commitment_with_holder",
        "sign_commitment_with_salt",
    ]);
}
