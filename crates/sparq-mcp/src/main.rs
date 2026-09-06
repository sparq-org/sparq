//! [SONNET-4.6] (sq-5xgxe, gh #3218) The `sparq-mcp` binary: load an RDF dataset and
//! serve it as MCP agent tools over the standard stdio transport (line-delimited
//! JSON-RPC 2.0 on this process's stdin/stdout), which is how an MCP client launches
//! and talks to a server subprocess.
//!
//! Usage:
//!   sparq-mcp [--allow-update] [--format FMT] [--query-timeout SECS] [--max-rows N] [DATA_FILE]
//!
//! It is a shim: `cli::parse_args` builds the [`ServerConfig`], `cli::load_graph` reads
//! the dataset, and `serve_stdio` runs the loop — all of which are library code, so this
//! file holds no protocol or policy logic of its own.
//!
//! SECURITY: writes are OFF unless `--allow-update` is passed. Without it the server is
//! strictly read-only — the `update` tool is neither advertised in `tools/list` nor
//! callable. There is no authentication: the stdio pipe is the trust boundary, so run
//! the binary only from a client you trust (see the crate README's trust model).

use sparq_mcp::cli;
use sparq_mcp::McpServer;

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("sparq-mcp: {}", message);
            std::process::ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let options = cli::parse_args(std::env::args().skip(1))?;
    if options.help {
        print!("{}", cli::USAGE);
        return Ok(());
    }

    let graph = cli::load_graph(options.data_file.as_deref(), options.format.as_deref())?;
    // Startup lines go to stderr — stdout is the JSON-RPC channel and must carry
    // nothing but responses.
    match &options.data_file {
        Some(path) => eprintln!("loaded {} triples from {}", graph.len(), path.display()),
        None => eprintln!("no data file given — serving an empty graph"),
    }
    let allow_update = options.config.allow_update;
    eprintln!(
        "sparq-mcp serving on stdio ({})",
        if allow_update {
            "WRITES ENABLED: the `update` tool can mutate this dataset"
        } else {
            "read-only: the `update` tool is disabled; pass --allow-update to enable writes"
        }
    );

    let mut server = McpServer::with_config(graph, options.config);
    sparq_mcp::serve_stdio(&mut server).map_err(|e| format!("stdio transport: {}", e))
}
