//! The stdio transport — the standard MCP transport.
//!
//! [OPUS-4.8] (sq-0z43i, gh #909) MCP's stdio transport is line-delimited JSON-RPC:
//! the client writes one JSON object per line to the server's stdin and reads one
//! JSON object per line from its stdout. This module is a thin blocking loop over
//! [`McpServer::handle_message`] — it owns no protocol logic, so the dispatch core is
//! tested directly without spawning a process. Gated behind the `stdio` feature (it
//! is the only part of the crate that touches I/O); it uses only `std::io`, so it
//! pulls no dependency.

use std::io::{BufRead, Write};

use crate::server::McpServer;

/// Serve the MCP protocol over an arbitrary reader/writer pair using the stdio
/// framing (one JSON-RPC value per line — an object, or a batch array).
/// Reads requests from `reader` line by line,
/// dispatches each through `server`, and writes each response as a single line to
/// `writer` (flushing after every response so the client sees it promptly).
///
/// Returns when `reader` reaches EOF (the client closed the connection) or on the
/// first write/read I/O error. Blank lines are skipped; a line that is not valid
/// JSON-RPC produces an error response (per JSON-RPC), it does not stop the loop.
pub fn serve<R: BufRead, W: Write>(
    server: &mut McpServer,
    reader: R,
    mut writer: W,
) -> std::io::Result<()> {
    for line in reader.lines() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(response) = server.handle_message(line) {
            writer.write_all(response.as_bytes())?;
            writer.write_all(b"\n")?;
            writer.flush()?;
        }
    }
    Ok(())
}

/// Serve the MCP protocol over this process's standard input and output — the
/// conventional way an MCP client launches and talks to a server subprocess. Blocks
/// until stdin reaches EOF. A convenience wrapper over [`serve`].
pub fn serve_stdio(server: &mut McpServer) -> std::io::Result<()> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let reader = stdin.lock();
    let writer = stdout.lock();
    serve(server, reader, writer)
}
