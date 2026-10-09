//! The stdio transport — the standard MCP transport.
//!
//! [OPUS-4.8] (sq-0z43i, gh #909) MCP's stdio transport is line-delimited JSON-RPC:
//! the client writes one JSON object per line to the server's stdin and reads one
//! JSON object per line from its stdout. This module is a thin blocking loop over
//! [`McpServer::handle_message`] — it owns no protocol logic, so the dispatch core is
//! tested directly without spawning a process. Gated behind the `stdio` feature (it
//! is the only part of the crate that touches I/O); it uses only `std::io`, so it
//! pulls no dependency.

use std::io::{self, BufRead, Write};

use crate::jsonrpc::{Response, RpcError, INVALID_REQUEST};
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
///
/// A line longer than the server's
/// [`max_request_bytes`](crate::ServerConfig::max_request_bytes) is answered with an
/// `INVALID_REQUEST` error (id `null`) and skipped; its bytes are consumed as they
/// arrive and never accumulated, so memory stays bounded by the cap. (gh #6051)
pub fn serve<R: BufRead, W: Write>(
    server: &mut McpServer,
    mut reader: R,
    mut writer: W,
) -> io::Result<()> {
    let max = server.config().max_request_bytes;
    let mut buf = Vec::new();
    loop {
        let response = match read_line_bounded(&mut reader, max, &mut buf)? {
            Line::Eof => return Ok(()),
            Line::TooLong => Some(oversized(max.unwrap_or(0))),
            Line::Complete => {
                let line = std::str::from_utf8(&buf)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?
                    .trim();
                if line.is_empty() {
                    continue;
                }
                server.handle_message(line)
            }
        };
        if let Some(response) = response {
            writer.write_all(response.as_bytes())?;
            writer.write_all(b"\n")?;
            writer.flush()?;
        }
    }
}

/// The outcome of one bounded line read.
enum Line {
    /// The reader was already at EOF: no more lines.
    Eof,
    /// A line (without its `\n`) is in the buffer.
    Complete,
    /// The line exceeded the cap; it has been consumed and discarded.
    TooLong,
}

/// Read one `\n`-terminated line into `buf` (cleared first), holding at most `max`
/// bytes of it. Past the cap the rest of the line is consumed straight out of the
/// reader's own buffer and dropped, so an endless line costs no memory beyond the cap.
fn read_line_bounded<R: BufRead>(
    reader: &mut R,
    max: Option<usize>,
    buf: &mut Vec<u8>,
) -> io::Result<Line> {
    buf.clear();
    let mut read_any = false;
    let mut too_long = false;
    loop {
        let (used, done) = {
            let available = match reader.fill_buf() {
                Ok(bytes) => bytes,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            };
            if available.is_empty() {
                break;
            }
            read_any = true;
            let (chunk, used, done) = match available.iter().position(|&b| b == b'\n') {
                Some(i) => (&available[..i], i + 1, true),
                None => (available, available.len(), false),
            };
            if !too_long {
                if max.is_some_and(|m| buf.len() + chunk.len() > m) {
                    too_long = true;
                    buf.clear();
                } else {
                    buf.extend_from_slice(chunk);
                }
            }
            (used, done)
        };
        reader.consume(used);
        if done {
            break;
        }
    }
    Ok(match (read_any, too_long) {
        (false, _) => Line::Eof,
        (true, true) => Line::TooLong,
        (true, false) => Line::Complete,
    })
}

/// The error response for an over-cap line: nothing was parsed, so the id is null.
fn oversized(max: usize) -> String {
    let response = Response::err(
        serde_json::Value::Null,
        RpcError::new(
            INVALID_REQUEST,
            format!("invalid JSON-RPC request: exceeds the {max}-byte request size limit"),
        ),
    );
    serde_json::to_string(&response).expect("a JSON-RPC response always serializes")
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
