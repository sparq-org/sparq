//! [SONNET-4.6] (sq-5xgxe, gh #3218) The `sparq-mcp` binary's argument parsing and
//! dataset loading — the library half of the shipped server binary, so the startup
//! contract (which flag turns writes on, which file is loaded as what format) is
//! unit-testable without spawning a process.
//!
//! `src/main.rs` is a thin shim over [`parse_args`] + [`load_graph`]: it builds an
//! [`McpServer`](crate::McpServer) from the returned [`Options`] and hands it to
//! [`serve_stdio`](crate::serve_stdio). Gated
//! behind the `stdio` feature — the binary needs that transport, and an embedder who
//! wires its own transport needs neither.

use std::path::{Path, PathBuf};

use sparq_core::Graph;

use crate::server::ServerConfig;

/// The binary's usage text (printed for `--help` and on a bad argument).
pub const USAGE: &str = "\
sparq-mcp — serve an RDF dataset as MCP agent tools over stdio (line-delimited JSON-RPC 2.0)

Usage:
  sparq-mcp [--allow-update] [--format FMT] [--query-timeout SECS] [--max-rows N] [DATA_FILE]

Arguments:
  DATA_FILE               RDF file to load into the served graph. Omit for an empty graph.

Options:
  --allow-update          Expose the mutating `update` tool. OFF by default: without this
                          flag the server is strictly read-only and `update` is neither
                          advertised nor callable.
  --format FMT            Format of DATA_FILE: turtle | ntriples | nquads | trig.
                          Default: inferred from the file extension (.nt/.nq/.trig/.ttl),
                          else turtle.
  --query-timeout SECS    Per-tool-call wall-clock deadline in seconds; 0 disables. [30]
  --max-rows N            Row cap on any materialised result; 0 disables. [1000000]
  -h, --help              Print this help and exit.
";

/// The parsed command line: what to load, and the [`ServerConfig`] to serve it with.
#[derive(Debug, Clone)]
pub struct Options {
    /// The RDF file to load, or `None` for an empty graph.
    pub data_file: Option<PathBuf>,
    /// The explicit `--format`, or `None` to infer it from the file extension.
    pub format: Option<String>,
    /// The server configuration the flags built — notably `allow_update`, which is
    /// `false` unless `--allow-update` was passed.
    pub config: ServerConfig,
    /// Whether `-h` / `--help` was requested (main prints [`USAGE`] and exits 0).
    pub help: bool,
}

/// Parse the binary's arguments (the caller passes `std::env::args().skip(1)`).
///
/// Read-only is the default: `allow_update` is `true` only when `--allow-update` is
/// present. An unknown flag, a missing flag value, a non-numeric number or a second
/// positional argument is an `Err` — the binary refuses to start rather than serving a
/// dataset the operator did not ask for.
pub fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Result<Options, String> {
    let mut options = Options {
        data_file: None,
        format: None,
        config: ServerConfig::default(),
        help: false,
    };

    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => options.help = true,
            // The single write switch. Everything else the binary accepts is a
            // read-side bound; this is the one flag that widens what an agent can do.
            "--allow-update" => options.config.allow_update = true,
            "--format" => {
                options.format = Some(args.next().ok_or("--format requires a value")?);
            }
            "--query-timeout" => {
                let secs = parse_number(args.next(), "--query-timeout")?;
                options.config.query_timeout_secs = (secs > 0).then_some(secs);
            }
            "--max-rows" => {
                let rows = parse_number(args.next(), "--max-rows")?;
                options.config.max_rows = (rows > 0).then_some(rows as usize);
            }
            other if other.starts_with('-') => {
                return Err(format!("unknown option: {}", other));
            }
            positional => {
                if options.data_file.is_some() {
                    return Err(format!(
                        "unexpected second data file: {} (one DATA_FILE only)",
                        positional
                    ));
                }
                options.data_file = Some(PathBuf::from(positional));
            }
        }
    }

    Ok(options)
}

fn parse_number(value: Option<String>, flag: &str) -> Result<u64, String> {
    let raw = value.ok_or_else(|| format!("{} requires a value", flag))?;
    raw.parse::<u64>()
        .map_err(|e| format!("{}: invalid number {:?}: {}", flag, raw, e))
}

/// The load format for `path`: the explicit `--format` when given, else inferred from
/// the file extension, else `turtle`.
///
/// The extension map matches the rest of the tree (`sparq-server`'s shapes loader and
/// the engine's `LOAD` path): `nt` → ntriples, `nq` → nquads, `trig` → trig, anything
/// else → turtle.
pub fn format_for(path: &Path, explicit: Option<&str>) -> String {
    if let Some(format) = explicit {
        return format.to_string();
    }
    match path.extension().and_then(|e| e.to_str()) {
        Some("nt") => "ntriples",
        Some("nq") => "nquads",
        Some("trig") => "trig",
        _ => "turtle",
    }
    .to_string()
}

/// Read `data_file` and parse it into the graph the server will serve; `None` yields an
/// empty graph (the server still answers queries — they just return no rows).
///
/// Loading goes through [`Graph::load_dataset`], so an N-Quads / TriG file keeps its
/// named graphs as named graphs instead of folding them into the default graph; other
/// formats defer to the triple loader.
pub fn load_graph(data_file: Option<&Path>, format: Option<&str>) -> Result<Graph, String> {
    let Some(path) = data_file else {
        return Graph::load_str("", "turtle").map_err(|e| format!("init: {}", e));
    };
    let format = format_for(path, format);
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {}", path.display(), e))?;
    Graph::load_dataset(&text, &format)
        .map_err(|e| format!("failed to load {} as {}: {}", path.display(), format, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Options, String> {
        parse_args(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn defaults_are_read_only_and_bounded() {
        let options = parse(&[]).unwrap();
        assert!(!options.config.allow_update, "read-only unless asked");
        assert!(!options.help);
        assert_eq!(options.data_file, None);
        assert_eq!(options.format, None);
        assert_eq!(options.config.query_timeout_secs, Some(30));
        assert_eq!(options.config.max_rows, Some(1_000_000));
    }

    // The headline guard: `--allow-update` is the ONLY way the parsed config turns the
    // mutation surface on. Delete the match arm and this goes red.
    #[test]
    fn allow_update_flag_is_the_only_write_switch() {
        assert!(parse(&["--allow-update"]).unwrap().config.allow_update);
        // Neither a data file nor any other flag enables writes.
        for args in [
            vec!["data.ttl"],
            vec!["--format", "turtle"],
            vec!["--query-timeout", "5"],
            vec!["--max-rows", "10"],
        ] {
            assert!(
                !parse(&args).unwrap().config.allow_update,
                "{:?} must not enable writes",
                args
            );
        }
    }

    #[test]
    fn positional_data_file_and_explicit_format() {
        let options = parse(&["--format", "nquads", "/tmp/data.txt"]).unwrap();
        assert_eq!(options.data_file, Some(PathBuf::from("/tmp/data.txt")));
        assert_eq!(options.format.as_deref(), Some("nquads"));
    }

    #[test]
    fn zero_disables_the_bounds() {
        let options = parse(&["--query-timeout", "0", "--max-rows", "0"]).unwrap();
        assert_eq!(options.config.query_timeout_secs, None);
        assert_eq!(options.config.max_rows, None);
    }

    #[test]
    fn numeric_flags_are_wired_through() {
        let options = parse(&["--query-timeout", "7", "--max-rows", "42"]).unwrap();
        assert_eq!(options.config.query_timeout_secs, Some(7));
        assert_eq!(options.config.max_rows, Some(42));
    }

    #[test]
    fn help_is_requested_by_either_spelling() {
        assert!(parse(&["-h"]).unwrap().help);
        assert!(parse(&["--help"]).unwrap().help);
        assert!(USAGE.contains("--allow-update"));
    }

    #[test]
    fn bad_arguments_are_refused() {
        assert!(parse(&["--nope"]).unwrap_err().contains("unknown option"));
        assert!(parse(&["--format"]).unwrap_err().contains("requires a value"));
        assert!(parse(&["--max-rows"]).unwrap_err().contains("requires a value"));
        assert!(parse(&["--query-timeout", "soon"])
            .unwrap_err()
            .contains("invalid number"));
        assert!(parse(&["a.ttl", "b.ttl"])
            .unwrap_err()
            .contains("second data file"));
    }

    #[test]
    fn format_is_inferred_from_the_extension_unless_given() {
        assert_eq!(format_for(Path::new("d.nt"), None), "ntriples");
        assert_eq!(format_for(Path::new("d.nq"), None), "nquads");
        assert_eq!(format_for(Path::new("d.trig"), None), "trig");
        assert_eq!(format_for(Path::new("d.ttl"), None), "turtle");
        assert_eq!(format_for(Path::new("d"), None), "turtle");
        // An explicit --format always wins over the extension.
        assert_eq!(format_for(Path::new("d.nt"), Some("trig")), "trig");
    }

    #[test]
    fn no_data_file_loads_an_empty_graph() {
        let graph = load_graph(None, None).unwrap();
        assert_eq!(graph.len(), 0);
    }

    #[test]
    fn data_file_is_read_and_parsed_with_the_inferred_format() {
        let path = std::env::temp_dir().join("sparq-mcp-cli-load-test.nt");
        std::fs::write(&path, "<http://ex/a> <http://ex/p> <http://ex/b> .\n").unwrap();
        let graph = load_graph(Some(&path), None).unwrap();
        assert_eq!(graph.len(), 1);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn an_unreadable_data_file_is_an_error_not_an_empty_graph() {
        let missing = std::env::temp_dir().join("sparq-mcp-cli-does-not-exist.ttl");
        std::fs::remove_file(&missing).ok();
        let err = load_graph(Some(&missing), None).err().expect("a missing file must error");
        assert!(err.contains("cannot read"), "{}", err);
    }

    // The dataset loader, not the triple loader: an N-Quads file's named graph survives
    // as a named graph instead of being folded into the default graph.
    #[test]
    fn nquads_named_graphs_are_preserved() {
        let path = std::env::temp_dir().join("sparq-mcp-cli-named-test.nq");
        std::fs::write(
            &path,
            "<http://ex/a> <http://ex/p> <http://ex/b> <http://ex/g> .\n",
        )
        .unwrap();
        let graph = load_graph(Some(&path), None).unwrap();
        let mut named_triples = 0;
        graph.for_named_graphs_with_prefix("http://ex/", |_name, g| named_triples += g.len());
        assert_eq!(named_triples, 1, "the quad is not in a named graph");
        // The triple loader would have folded it into the default graph instead.
        assert_eq!(graph.len(), 0, "the quad landed in the default graph");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_malformed_data_file_is_an_error() {
        let path = std::env::temp_dir().join("sparq-mcp-cli-malformed-test.ttl");
        std::fs::write(&path, "this is not turtle").unwrap();
        let err = load_graph(Some(&path), None)
            .err()
            .expect("malformed turtle must error");
        assert!(err.contains("failed to load"), "{}", err);
        std::fs::remove_file(&path).ok();
    }
}
