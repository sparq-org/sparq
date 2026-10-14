//! THE load-bearing Phase-0 deliverable (bead sq-s1uy, epic sq-dnko): a hermetic proof
//! that the lean core — `sparq-core` and `sparq-engine` — has NO dependency edge to
//! `sparq-fedclient`, in both feature states. The dependency arrow points one-way INTO
//! the engine (the client reuses the engine's SERVICE transport + SSRF guard + local
//! eval); the engine must NEVER depend on the client, or the opt-in-feature-architecture
//! constraint (core stays lean, the WASM artifact unchanged) is broken.
//!
//! This test reads the *workspace* dependency graph from `cargo metadata`, so it gates
//! regardless of which features the test binary itself is compiled with: a dependency
//! edge can only exist if it is declared in a Cargo.toml (cargo features can ADD an
//! optional edge but can never invent one that is not declared somewhere), and
//! `--all-features` resolves every conditional edge, so an edge introduced under ANY
//! feature is visible here. It MUST fail if a future edit makes `sparq-core` or
//! `sparq-engine` depend on `sparq-fedclient`.
//!
//! It is the in-suite mirror of `scripts/fedclient-boundary-guard.sh` (the CI step):
//! the script uses `cargo tree -i`, this test uses `cargo metadata`'s resolve graph;
//! either alone would catch a violation, both together is belt-and-braces.
//!
//! sq-s1uy — flagged for Fable re-review.

use std::collections::{HashMap, HashSet};
use std::process::Command;

/// The lean members that must never depend on the federation client.
const LEAN_CORE: &[&str] = &["sparq-core", "sparq-engine"];
const CLIENT: &str = "sparq-fedclient";

/// Minimal hand-rolled JSON value walker over `cargo metadata` output. We avoid pulling
/// a JSON dependency into a `publish = false` skeleton crate just for a boundary test;
/// the two shapes we need (the `packages[].{name,id}` map and the `resolve.nodes[]`
/// adjacency) are extracted with a tiny tolerant scanner. If the scanner ever fails to
/// find the resolve graph the test fails LOUD (never silently passes).
mod tinyjson {
    /// A JSON value, only the variants `cargo metadata` emits. `Bool`/`Num`/`Null` are
    /// parsed faithfully (so the scanner stays a correct tolerant JSON reader) but this
    /// boundary test only ever reads the `Str`/`Arr`/`Obj` shapes — hence the carried
    /// scalars are intentionally unread.
    #[derive(Debug)]
    #[allow(dead_code)]
    pub enum Json {
        Null,
        Bool(bool),
        Num(f64),
        Str(String),
        Arr(Vec<Json>),
        Obj(Vec<(String, Json)>),
    }

    impl Json {
        pub fn get(&self, key: &str) -> Option<&Json> {
            match self {
                Json::Obj(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
                _ => None,
            }
        }
        pub fn as_arr(&self) -> Option<&[Json]> {
            match self {
                Json::Arr(v) => Some(v),
                _ => None,
            }
        }
        pub fn as_str(&self) -> Option<&str> {
            match self {
                Json::Str(s) => Some(s),
                _ => None,
            }
        }
    }

    pub fn parse(input: &str) -> Result<Json, String> {
        let bytes = input.as_bytes();
        let mut pos = 0usize;
        let v = parse_value(bytes, &mut pos)?;
        Ok(v)
    }

    fn skip_ws(b: &[u8], pos: &mut usize) {
        while *pos < b.len() {
            match b[*pos] {
                b' ' | b'\t' | b'\n' | b'\r' => *pos += 1,
                _ => break,
            }
        }
    }

    fn parse_value(b: &[u8], pos: &mut usize) -> Result<Json, String> {
        skip_ws(b, pos);
        if *pos >= b.len() {
            return Err("unexpected end of input".into());
        }
        match b[*pos] {
            b'{' => parse_obj(b, pos),
            b'[' => parse_arr(b, pos),
            b'"' => Ok(Json::Str(parse_str(b, pos)?)),
            b't' => parse_lit(b, pos, "true", Json::Bool(true)),
            b'f' => parse_lit(b, pos, "false", Json::Bool(false)),
            b'n' => parse_lit(b, pos, "null", Json::Null),
            _ => parse_num(b, pos),
        }
    }

    fn parse_lit(b: &[u8], pos: &mut usize, lit: &str, val: Json) -> Result<Json, String> {
        if b[*pos..].starts_with(lit.as_bytes()) {
            *pos += lit.len();
            Ok(val)
        } else {
            Err(format!("expected literal {}", lit))
        }
    }

    fn parse_num(b: &[u8], pos: &mut usize) -> Result<Json, String> {
        let start = *pos;
        while *pos < b.len() {
            match b[*pos] {
                b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E' => *pos += 1,
                _ => break,
            }
        }
        let s = std::str::from_utf8(&b[start..*pos]).map_err(|e| e.to_string())?;
        s.parse::<f64>().map(Json::Num).map_err(|e| e.to_string())
    }

    fn parse_str(b: &[u8], pos: &mut usize) -> Result<String, String> {
        // assumes b[*pos] == '"'
        *pos += 1;
        let mut out = String::new();
        while *pos < b.len() {
            let c = b[*pos];
            *pos += 1;
            match c {
                b'"' => return Ok(out),
                b'\\' => {
                    if *pos >= b.len() {
                        return Err("bad escape".into());
                    }
                    let e = b[*pos];
                    *pos += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'n' => out.push('\n'),
                        b't' => out.push('\t'),
                        b'r' => out.push('\r'),
                        b'b' => out.push('\u{0008}'),
                        b'f' => out.push('\u{000C}'),
                        b'u' => {
                            if *pos + 4 > b.len() {
                                return Err("bad \\u escape".into());
                            }
                            let hex = std::str::from_utf8(&b[*pos..*pos + 4])
                                .map_err(|e| e.to_string())?;
                            let code = u32::from_str_radix(hex, 16).map_err(|e| e.to_string())?;
                            *pos += 4;
                            out.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
                        }
                        _ => return Err("unknown escape".into()),
                    }
                }
                _ => {
                    // copy a UTF-8 byte run verbatim; re-decode at the end is unnecessary
                    // because cargo metadata package names/ids are ASCII, but be tolerant.
                    out.push(c as char);
                }
            }
        }
        Err("unterminated string".into())
    }

    fn parse_arr(b: &[u8], pos: &mut usize) -> Result<Json, String> {
        *pos += 1; // consume '['
        let mut items = Vec::new();
        loop {
            skip_ws(b, pos);
            if *pos >= b.len() {
                return Err("unterminated array".into());
            }
            if b[*pos] == b']' {
                *pos += 1;
                return Ok(Json::Arr(items));
            }
            items.push(parse_value(b, pos)?);
            skip_ws(b, pos);
            if *pos < b.len() && b[*pos] == b',' {
                *pos += 1;
            }
        }
    }

    fn parse_obj(b: &[u8], pos: &mut usize) -> Result<Json, String> {
        *pos += 1; // consume '{'
        let mut entries = Vec::new();
        loop {
            skip_ws(b, pos);
            if *pos >= b.len() {
                return Err("unterminated object".into());
            }
            if b[*pos] == b'}' {
                *pos += 1;
                return Ok(Json::Obj(entries));
            }
            skip_ws(b, pos);
            let key = parse_str(b, pos)?;
            skip_ws(b, pos);
            if *pos >= b.len() || b[*pos] != b':' {
                return Err("expected ':' in object".into());
            }
            *pos += 1;
            let val = parse_value(b, pos)?;
            entries.push((key, val));
            skip_ws(b, pos);
            if *pos < b.len() && b[*pos] == b',' {
                *pos += 1;
            }
        }
    }
}

use tinyjson::{parse, Json};

/// Run `cargo metadata --all-features --format-version 1` and return its stdout.
/// `--all-features` makes EVERY conditional dependency edge visible, so an edge added
/// under any feature is caught.
fn cargo_metadata_json() -> String {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let out = Command::new(cargo)
        .args([
            "metadata",
            "--all-features",
            "--format-version",
            "1",
            "--quiet",
        ])
        .output()
        .expect("failed to spawn `cargo metadata`");
    assert!(
        out.status.success(),
        "`cargo metadata` failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("cargo metadata stdout was not UTF-8")
}

/// (id -> name), (name -> [ids]), (id -> [dependency ids]) — the three maps the closure
/// walk needs from `cargo metadata`.
type Graph = (
    HashMap<String, String>,
    HashMap<String, Vec<String>>,
    HashMap<String, Vec<String>>,
);

/// Build (package-id -> package-name) and the resolve adjacency (package-id ->
/// [dependency package-ids]) from parsed metadata, plus (name -> [ids]).
fn graph(meta: &Json) -> Graph {
    let mut id_to_name = HashMap::new();
    let mut name_to_ids: HashMap<String, Vec<String>> = HashMap::new();
    let packages = meta
        .get("packages")
        .and_then(Json::as_arr)
        .expect("metadata.packages missing");
    for p in packages {
        let id = p.get("id").and_then(Json::as_str).expect("package.id");
        let name = p.get("name").and_then(Json::as_str).expect("package.name");
        id_to_name.insert(id.to_string(), name.to_string());
        name_to_ids
            .entry(name.to_string())
            .or_default()
            .push(id.to_string());
    }

    let mut adj: HashMap<String, Vec<String>> = HashMap::new();
    let resolve = meta
        .get("resolve")
        .expect("metadata.resolve missing (cargo resolved no graph?)");
    let nodes = resolve
        .get("nodes")
        .and_then(Json::as_arr)
        .expect("metadata.resolve.nodes missing");
    assert!(!nodes.is_empty(), "resolve graph had zero nodes");
    for n in nodes {
        let id = n.get("id").and_then(Json::as_str).expect("node.id");
        let deps: Vec<String> = n
            .get("dependencies")
            .and_then(Json::as_arr)
            .map(|a| {
                a.iter()
                    .filter_map(Json::as_str)
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default();
        adj.insert(id.to_string(), deps);
    }
    (id_to_name, name_to_ids, adj)
}

/// All package NAMES transitively reachable from `start_name` in the resolve graph.
fn reachable_names(
    start_name: &str,
    id_to_name: &HashMap<String, String>,
    name_to_ids: &HashMap<String, Vec<String>>,
    adj: &HashMap<String, Vec<String>>,
) -> HashSet<String> {
    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut stack: Vec<String> = name_to_ids.get(start_name).cloned().unwrap_or_default();
    assert!(
        !stack.is_empty(),
        "package `{}` not found in workspace metadata",
        start_name
    );
    while let Some(id) = stack.pop() {
        if !seen_ids.insert(id.clone()) {
            continue;
        }
        if let Some(deps) = adj.get(&id) {
            for d in deps {
                stack.push(d.clone());
            }
        }
    }
    seen_ids
        .iter()
        .filter_map(|i| id_to_name.get(i).cloned())
        .collect()
}

#[test]
fn lean_core_has_no_edge_to_fedclient() {
    let json = cargo_metadata_json();
    let meta = parse(&json).expect("failed to parse cargo metadata JSON");
    let (id_to_name, name_to_ids, adj) = graph(&meta);

    // Sanity: the crate under proof IS in the workspace (else the test is vacuous).
    assert!(
        name_to_ids.contains_key(CLIENT),
        "`{}` is not a workspace member — the boundary test would be vacuous",
        CLIENT
    );

    for lean in LEAN_CORE {
        let reach = reachable_names(lean, &id_to_name, &name_to_ids, &adj);
        assert!(
            !reach.contains(CLIENT),
            "BOUNDARY VIOLATION: `{}` transitively depends on `{}` — the lean core must \
             NEVER depend on the federation client (the dependency arrow points ONE WAY \
             into the engine). Remove the edge.",
            lean,
            CLIENT
        );
    }
}

#[test]
fn fedclient_reaches_its_reuse_seams_under_all_features() {
    // The positive side: with `--all-features`, the client SHOULD reach the seams §4
    // names it reuses (sparq-engine + sparq-fedplan). This guards against the skeleton
    // accidentally dropping the intended one-way edges (which would make the negative
    // test above pass for the wrong reason).
    let json = cargo_metadata_json();
    let meta = parse(&json).expect("failed to parse cargo metadata JSON");
    let (id_to_name, name_to_ids, adj) = graph(&meta);
    let reach = reachable_names(CLIENT, &id_to_name, &name_to_ids, &adj);
    for seam in ["sparq-engine", "sparq-fedplan"] {
        assert!(
            reach.contains(seam),
            "expected `{}` to reach its reuse seam `{}` under --all-features",
            CLIENT,
            seam
        );
    }
}
