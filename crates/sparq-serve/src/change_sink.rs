//! [OPUS-5] (sq-l6zks, gh-3216) The **external-broker sink seam** over the durable
//! [`change_stream`](crate::change_stream) — a [`ChangeSink`] trait, a stable broker-message
//! encoding, a resumable [`BrokerRelay`] pump, and one in-tree, dependency-free
//! [`NatsSink`] (core-NATS publish). This is the SEPARATE, HEAVIER opt-in the change-stream
//! module deferred: the `change-sink` feature (default OFF) implies `change-stream` and is
//! the only thing in this crate that can open a network socket.
//!
//! ## The shape
//!
//! ```text
//!   writer thread ──record_commit──▶ durable segmented log   (change-stream: source of truth)
//!                                          │
//!            host pump thread ──poll──▶ BrokerRelay ──encode──▶ ChangeSink ──▶ Kafka / NATS / …
//!                                          └─ persists a delivered-through watermark
//! ```
//!
//! **The relay is deliberately NOT on the writer thread.** A commit hook that published to a
//! broker inline would put broker latency — and a broker outage — directly in the write-ack
//! path of every commit. Instead the durable log stays the source of truth and the relay is a
//! separate consumer of it, driven by a host thread calling [`BrokerRelay::pump`] on whatever
//! cadence it likes. A broker that is down stalls the relay, never the writer; when it comes
//! back the relay resumes from its persisted watermark.
//!
//! ## What this crate does and does NOT ship
//!
//! - **Ships:** the [`ChangeSink`] trait, the broker-message encoding
//!   ([`encode_message`], JSON, same per-change entry shape as sparq-server's `GET /streams`),
//!   the resumable [`BrokerRelay`] with its durable offset file, an in-memory
//!   [`RecordingSink`], and [`NatsSink`] — a **core-NATS publisher written against `std`
//!   only** (`INFO`/`CONNECT`/`PUB`/`PING`-`PONG`).
//! - **Does NOT ship: a Kafka client.** The Kafka wire protocol is a binary, versioned,
//!   CRC-framed protocol; a correct in-tree implementation would be a large, untestable-
//!   without-a-broker body of code, and pulling `rdkafka` would add a C library (and
//!   `async-nats` would add an async runtime) to a crate whose charter is sync, runtime-
//!   agnostic and library-first. So Kafka — and any other broker, and any deployment that
//!   needs TLS, SASL, retries or batching — is reached by **implementing [`ChangeSink`]
//!   over your own client**. That is the same posture `sparq-server`'s `AuditSink` takes:
//!   heavy/external sinks stay out of core, the seam is in core.
//!
//! ## Delivery semantics (stated honestly)
//!
//! - **At-least-once, never exactly-once.** The watermark is persisted AFTER the sink's
//!   [`flush`](ChangeSink::flush) returns, so a crash in that window re-delivers the last
//!   batch. Consumers **must** dedupe on `sequenceNumber` (it is gapless and monotonic —
//!   dedupe is a `<=` comparison against a stored high-water mark).
//! - **Ordered per relay.** One relay delivers records strictly in `seq` order and never
//!   advances past a failed record. The encoded [`BrokerMessage::key`] is CONSTANT for the
//!   whole stream (not per-record) precisely so a partitioned broker keeps every record in
//!   one partition — a per-record key would shard the stream and destroy commit order.
//! - **Fail-closed on a trimmed offset.** If retention dropped the records a resuming relay
//!   still needs, [`pump`](BrokerRelay::pump) returns an error rather than silently skipping
//!   them. Feed [`delivered_through_seq`](BrokerRelay::delivered_through_seq) into
//!   [`RetentionPolicy::acked_through_seq`](crate::change_stream::RetentionPolicy::acked_through_seq)
//!   and that cannot happen: retention will not drop a segment the relay has not delivered.
//! - **A re-base gap record is delivered, not swallowed.** A
//!   [`ChangeRecord::rebase`](crate::change_stream::ChangeRecord::rebase) marker is encoded as
//!   an explicit `"op": "REBASE"` entry with `"rebase": true`, so a downstream consumer sees
//!   that the span before it was NOT captured instead of reading an empty commit as "nothing
//!   changed".
//!
//! ## Boundaries (same as the rest of the change-stream family)
//!
//! Payloads are **plaintext JSON** and carry no signature: transport security, authenticity
//! and at-rest encryption are the deployment's concern, exactly as for
//! [`backup`](crate::backup) and [`change_stream`](crate::change_stream). In particular
//! [`NatsSink`] speaks **plain TCP with no TLS** — it is appropriate for a trusted network or
//! a sidecar on localhost. A deployment that needs TLS, authentication beyond a NATS token,
//! or broker-side acknowledgement stronger than a `PING`/`PONG` round-trip should implement
//! [`ChangeSink`] over a client that provides those. Change records contain the actual quads
//! that were written, so pointing a relay at a broker is a data-egress decision.

use std::fmt;
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::backup::BackupError;
use crate::change_stream::{earliest_retained_seq, read_from, ChangeOp, ChangeRecord};

/// Everything that can go wrong delivering a change record to a broker.
#[derive(Debug)]
pub enum SinkError {
    /// Local I/O failed — reading the change log's segments, or persisting the relay's
    /// delivered-through watermark.
    Io(io::Error),
    /// The durable change log could not be read (corruption, or a poll offset that retention
    /// already trimmed away — see [`BrokerRelay::pump`]).
    Log(BackupError),
    /// The broker rejected the message or reported an error (e.g. a NATS `-ERR` line).
    Broker(String),
    /// The broker spoke something this client does not understand, or the connection closed
    /// mid-exchange.
    Protocol(String),
    /// The relay/sink was configured with something unusable (a bad consumer name, a subject
    /// that is not publishable). Detected before anything is sent.
    Config(String),
}

impl fmt::Display for SinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // [OPUS-5] positional format args (CodeQL rust/unused-variable false-positive),
            // matching the rest of the crate.
            SinkError::Io(e) => write!(f, "change-sink I/O error: {}", e),
            SinkError::Log(e) => write!(f, "change-sink log error: {}", e),
            SinkError::Broker(m) => write!(f, "change-sink broker error: {}", m),
            SinkError::Protocol(m) => write!(f, "change-sink protocol error: {}", m),
            SinkError::Config(m) => write!(f, "change-sink configuration error: {}", m),
        }
    }
}

impl std::error::Error for SinkError {}

impl From<io::Error> for SinkError {
    fn from(e: io::Error) -> Self {
        SinkError::Io(e)
    }
}

impl From<BackupError> for SinkError {
    fn from(e: BackupError) -> Self {
        SinkError::Log(e)
    }
}

/// Message header naming the payload's media type.
pub const HEADER_CONTENT_TYPE: &str = "content-type";
/// Header naming the record's gapless sequence number — the dedupe key for the at-least-once
/// contract, duplicated out of the payload so a consumer can dedupe without parsing the body.
pub const HEADER_SEQ: &str = "sparq-change-seq";
/// Header naming the generation the record captures the commit TO.
pub const HEADER_GENERATION: &str = "sparq-change-generation";
/// Header flagging an operator re-base GAP marker (`true`/`false`).
pub const HEADER_REBASE: &str = "sparq-change-rebase";
/// The payload media type produced by [`encode_message`].
pub const CONTENT_TYPE_JSON: &str = "application/json";

/// One broker-ready message: exactly one [`ChangeRecord`] (i.e. one whole commit), encoded.
///
/// A commit is kept WHOLE — the quad-level changes ride inside one message rather than being
/// flattened to one message per quad — so a consumer can apply a commit atomically and so the
/// stream's `seq` remains the single dedupe/resume key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrokerMessage {
    /// Where to publish: a Kafka topic, a NATS subject, whatever the sink's broker calls it.
    pub subject: String,
    /// The partition/ordering key. **Constant for the whole stream** (see the module docs):
    /// a partitioned broker must keep every record of one relay in one partition or commit
    /// order is lost. Sinks whose broker has no such concept ignore it.
    pub key: String,
    /// Broker headers (`content-type` plus the seq/generation/rebase metadata). Brokers
    /// without header support (core NATS `PUB`) ignore these — the same values are in the
    /// payload.
    pub headers: Vec<(String, String)>,
    /// The encoded record body — JSON, UTF-8.
    pub payload: Vec<u8>,
}

/// Default publish subject / topic when none is configured.
pub const DEFAULT_SUBJECT: &str = "sparq.changes";
/// Default cap on how many records one [`BrokerRelay::pump`] delivers.
pub const DEFAULT_PUMP_MAX_BATCH: usize = 1024;

/// How a [`BrokerRelay`] encodes and addresses the records it delivers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SinkConfig {
    /// The broker subject/topic every record is published to.
    pub subject: String,
    /// The constant partition/ordering key stamped on every message (defaults to the
    /// subject). See [`BrokerMessage::key`] for why it is deliberately not per-record.
    pub partition_key: String,
    /// Maximum records delivered by one [`pump`](BrokerRelay::pump) call; the report's
    /// [`has_more`](PumpReport::has_more) says whether more were available. A cap of `0` is
    /// treated as `1` (a pump always makes progress when there is anything to deliver).
    pub max_batch: usize,
}

impl Default for SinkConfig {
    fn default() -> Self {
        SinkConfig::new(DEFAULT_SUBJECT)
    }
}

impl SinkConfig {
    /// A config publishing to `subject`, with the partition key defaulted to the subject and
    /// the default batch cap.
    pub fn new(subject: impl Into<String>) -> Self {
        let subject = subject.into();
        SinkConfig {
            partition_key: subject.clone(),
            subject,
            max_batch: DEFAULT_PUMP_MAX_BATCH,
        }
    }

    /// Overrides the constant partition/ordering key.
    pub fn with_partition_key(mut self, key: impl Into<String>) -> Self {
        self.partition_key = key.into();
        self
    }

    /// Overrides the per-pump record cap.
    pub fn with_max_batch(mut self, max_batch: usize) -> Self {
        self.max_batch = max_batch;
        self
    }

    /// The effective batch cap (never zero — a pump must make progress).
    fn effective_max_batch(&self) -> usize {
        self.max_batch.max(1)
    }
}

/// Encodes ONE change record as a broker message.
///
/// The payload is a JSON object:
///
/// ```json
/// { "sequenceNumber": 3, "generation": 3, "commitTimestampNanos": "1700000000000000000",
///   "rebase": false,
///   "records": [ { "eventId": { "commitNum": 3, "opNum": 1 }, "op": "ADD",
///                  "data": { "stmt": "<s> <p> <o> ." } } ] }
/// ```
///
/// The `records[]` entries are byte-for-byte the shape sparq-server's `GET /streams` emits
/// (`eventId`/`op`/`data.stmt`, Amazon-Neptune-Streams flavoured), so one consumer-side parser
/// serves both surfaces; the commit-level envelope around them is what the HTTP endpoint's
/// pagination fields carry instead. `commitTimestampNanos` is a STRING (a `u128` nanosecond
/// value does not survive a JSON number in most consumers). A re-base gap record encodes as a
/// single `"op": "REBASE"` entry with no `data`, never as an empty commit.
pub fn encode_message(record: &ChangeRecord, config: &SinkConfig) -> BrokerMessage {
    BrokerMessage {
        subject: config.subject.clone(),
        key: config.partition_key.clone(),
        headers: vec![
            (HEADER_CONTENT_TYPE.to_string(), CONTENT_TYPE_JSON.to_string()),
            (HEADER_SEQ.to_string(), record.seq.to_string()),
            (HEADER_GENERATION.to_string(), record.generation.to_string()),
            (HEADER_REBASE.to_string(), record.rebase.to_string()),
        ],
        payload: encode_payload(record).into_bytes(),
    }
}

/// The JSON body of [`encode_message`].
fn encode_payload(record: &ChangeRecord) -> String {
    let mut s = String::new();
    s.push_str("{\"sequenceNumber\":");
    s.push_str(&record.seq.to_string());
    s.push_str(",\"generation\":");
    s.push_str(&record.generation.to_string());
    s.push_str(",\"commitTimestampNanos\":\"");
    s.push_str(&record.timestamp_unix_nanos.to_string());
    s.push_str("\",\"rebase\":");
    s.push_str(if record.rebase { "true" } else { "false" });
    s.push_str(",\"records\":[");
    if record.rebase {
        // The honest gap marker — one entry, no `data` (there is no quad). Flattening it away
        // would present an uncaptured span as "no changes".
        s.push_str("{\"eventId\":{\"commitNum\":");
        s.push_str(&record.seq.to_string());
        s.push_str(",\"opNum\":1},\"op\":\"REBASE\"}");
    } else {
        for (i, change) in record.changes.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            s.push_str("{\"eventId\":{\"commitNum\":");
            s.push_str(&record.seq.to_string());
            s.push_str(",\"opNum\":");
            s.push_str(&(i + 1).to_string());
            s.push_str("},\"op\":\"");
            s.push_str(match change.op {
                ChangeOp::Insert => "ADD",
                ChangeOp::Delete => "REMOVE",
            });
            s.push_str("\",\"data\":{\"stmt\":");
            push_json_string(&mut s, &change.quad);
            s.push_str("}}");
        }
    }
    s.push_str("]}");
    s
}

/// Appends `s` to `out` as a JSON string literal. This crate has no JSON dependency (the
/// change-stream family is deliberately dependency-free), and an N-Quads line routinely
/// contains `"` and `\`, so the escaping is done here rather than assumed away.
fn push_json_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// The pluggable external-broker seam: where a delivered [`BrokerMessage`] actually goes.
///
/// Implement this over your own broker client (an `rdkafka` producer, an HTTP webhook, a
/// message bus) — heavy/external clients stay OUT of this crate (module docs). `deliver`
/// takes `&mut self` because a sink typically owns a connection; a [`BrokerRelay`] drives one
/// sink from one thread, so no interior synchronisation is required.
///
/// **The contract [`BrokerRelay`] relies on:** `deliver` returning `Ok` means the message was
/// handed to the broker client; `flush` returning `Ok` means everything handed over since the
/// last flush is as durable as that broker will make it. The relay only advances (and
/// persists) its watermark after a successful `flush`, so a sink that buffers MUST NOT report
/// success from `flush` until its buffer has actually gone out — otherwise records can be
/// lost, not merely re-delivered.
pub trait ChangeSink {
    /// Publishes one encoded change record. Errors abort the current pump; the relay does not
    /// advance past the failed record, so the next pump retries it.
    fn deliver(&mut self, message: &BrokerMessage) -> Result<(), SinkError>;

    /// Barrier: everything delivered since the last flush is now as durable as this broker
    /// makes it. The default is a no-op, correct only for a sink that does not buffer.
    fn flush(&mut self) -> Result<(), SinkError> {
        Ok(())
    }
}

/// An in-memory [`ChangeSink`] that just records what it was given — the seam's test double,
/// and a useful "dry run" sink when wiring a relay up for the first time.
#[derive(Clone, Debug, Default)]
pub struct RecordingSink {
    messages: Vec<BrokerMessage>,
    flushes: usize,
}

impl RecordingSink {
    /// An empty recording sink.
    pub fn new() -> Self {
        Self::default()
    }

    /// Everything delivered so far, in delivery order.
    pub fn messages(&self) -> &[BrokerMessage] {
        &self.messages
    }

    /// How many times [`flush`](ChangeSink::flush) has been called.
    pub fn flush_count(&self) -> usize {
        self.flushes
    }

    /// Drains the recorded messages.
    pub fn take(&mut self) -> Vec<BrokerMessage> {
        std::mem::take(&mut self.messages)
    }
}

impl ChangeSink for RecordingSink {
    fn deliver(&mut self, message: &BrokerMessage) -> Result<(), SinkError> {
        self.messages.push(message.clone());
        Ok(())
    }

    fn flush(&mut self) -> Result<(), SinkError> {
        self.flushes += 1;
        Ok(())
    }
}

/// Connection options for [`NatsSink`].
///
/// `Debug` is implemented by hand so [`auth_token`](Self::auth_token) is REDACTED — an
/// operator logging their sink config must not thereby log the credential.
#[derive(Clone, Default)]
pub struct NatsOptions {
    /// Client name advertised in the `CONNECT` handshake (shows up in NATS server monitoring).
    pub client_name: String,
    /// Optional NATS `auth_token`. Sent in the plaintext `CONNECT` line — this client does not
    /// speak TLS (module docs).
    pub auth_token: Option<String>,
    /// Read/write timeout applied to a socket opened by [`NatsSink::connect`]. `None` leaves
    /// the socket blocking indefinitely, which will hang a pump against a wedged broker.
    pub timeout: Option<Duration>,
}

impl fmt::Debug for NatsOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NatsOptions")
            .field("client_name", &self.client_name)
            .field(
                "auth_token",
                &self.auth_token.as_ref().map(|_| "<redacted>"),
            )
            .field("timeout", &self.timeout)
            .finish()
    }
}

/// A **core-NATS publisher** [`ChangeSink`], written against `std` only — no client library,
/// no async runtime, no TLS (module docs).
///
/// It speaks the four lines of the NATS text protocol a publisher needs: it reads the server's
/// `INFO` greeting, sends `CONNECT` (with `verbose:false`), publishes with `PUB`, and uses a
/// `PING`/`PONG` round-trip as its [`flush`](ChangeSink::flush) barrier — the point at which
/// the server has processed everything written before it, which is what makes the relay's
/// watermark meaningful. Server-initiated `PING`s are answered while waiting; a `-ERR` line
/// becomes [`SinkError::Broker`].
///
/// **Core NATS, not JetStream:** a published message is not persisted by the broker, so
/// delivery is only as durable as the subscriber that is listening. The durable, replayable
/// copy is the [`change_stream`](crate::change_stream) log this relay reads from — that is the
/// source of truth, and a relay can always be re-pointed and replayed from it.
/// [`BrokerMessage::key`] and [`BrokerMessage::headers`] are ignored (core `PUB` carries
/// neither; the same values are in the payload).
pub struct NatsSink<S: Read + Write> {
    io: BufReader<S>,
    /// Messages written but not yet confirmed by a `PING`/`PONG` round-trip.
    pending: usize,
}

// Hand-written so a sink over a non-`Debug` transport is still `Debug` (and so no part of the
// connection's buffered bytes can be printed).
impl<S: Read + Write> fmt::Debug for NatsSink<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NatsSink")
            .field("pending", &self.pending)
            .finish_non_exhaustive()
    }
}

impl NatsSink<TcpStream> {
    /// Connects to a NATS server over **plain TCP** and completes the handshake, returning a
    /// sink ready to publish. A bad address, a closed connection, or an authentication
    /// rejection surfaces here rather than on the first delivery (the handshake ends with a
    /// `PING`/`PONG`, which the server refuses if the `CONNECT` was not accepted).
    pub fn connect<A: ToSocketAddrs>(addr: A, options: &NatsOptions) -> Result<Self, SinkError> {
        let stream = TcpStream::connect(addr)?;
        if let Some(timeout) = options.timeout {
            stream.set_read_timeout(Some(timeout))?;
            stream.set_write_timeout(Some(timeout))?;
        }
        NatsSink::handshake(stream, options)
    }
}

impl<S: Read + Write> NatsSink<S> {
    /// Completes the NATS handshake over an already-established byte stream. Use this to run
    /// the publisher over a transport this crate cannot open itself (a TLS stream, a Unix
    /// socket, a test double); [`connect`](NatsSink::connect) is the plain-TCP shortcut.
    pub fn handshake(stream: S, options: &NatsOptions) -> Result<Self, SinkError> {
        let mut sink = NatsSink {
            io: BufReader::new(stream),
            pending: 0,
        };
        let greeting = sink.read_line()?;
        if !greeting.starts_with("INFO") {
            return Err(SinkError::Protocol(format!(
                "expected an INFO greeting from the NATS server, got {:?}",
                greeting
            )));
        }

        // `verbose:false` — the server does not ack each PUB, so PING/PONG is the barrier.
        let mut connect = String::from(
            "CONNECT {\"verbose\":false,\"pedantic\":false,\"tls_required\":false,\
             \"protocol\":1,\"lang\":\"rust\",\"name\":",
        );
        push_json_string(&mut connect, &options.client_name);
        if let Some(token) = &options.auth_token {
            connect.push_str(",\"auth_token\":");
            push_json_string(&mut connect, token);
        }
        connect.push_str("}\r\n");
        sink.write_frame(connect.as_bytes())?;
        // The handshake's own PING/PONG: it fails fast on a rejected CONNECT.
        sink.write_frame(b"PING\r\n")?;
        sink.io.get_mut().flush()?;
        sink.await_pong()?;
        Ok(sink)
    }

    /// The underlying byte stream (diagnostics; the sink keeps owning it).
    pub fn stream(&self) -> &S {
        self.io.get_ref()
    }

    fn write_frame(&mut self, bytes: &[u8]) -> Result<(), SinkError> {
        self.io.get_mut().write_all(bytes)?;
        Ok(())
    }

    /// Reads one protocol line with its `\r\n` stripped. EOF is a protocol error (the server
    /// hung up mid-exchange), never a silent success.
    fn read_line(&mut self) -> Result<String, SinkError> {
        let mut line = String::new();
        let n = self.io.read_line(&mut line)?;
        if n == 0 {
            return Err(SinkError::Protocol(
                "the NATS connection closed while awaiting a reply".to_string(),
            ));
        }
        while line.ends_with('\n') || line.ends_with('\r') {
            line.pop();
        }
        Ok(line)
    }

    /// Waits for the `PONG` answering our `PING`, servicing whatever the server says first.
    fn await_pong(&mut self) -> Result<(), SinkError> {
        loop {
            let line = self.read_line()?;
            let verb = line.split(' ').next().unwrap_or("");
            match verb {
                "PONG" => return Ok(()),
                // The server pings us on its own schedule; answer it and keep waiting.
                "PING" => {
                    self.write_frame(b"PONG\r\n")?;
                    self.io.get_mut().flush()?;
                }
                // `+OK` (only with verbose) and a re-sent `INFO` (cluster topology update) are
                // both benign here.
                "+OK" | "INFO" => {}
                "-ERR" => return Err(SinkError::Broker(line)),
                _ => {
                    return Err(SinkError::Protocol(format!(
                        "unexpected line from the NATS server: {:?}",
                        line
                    )))
                }
            }
        }
    }
}

impl<S: Read + Write> ChangeSink for NatsSink<S> {
    fn deliver(&mut self, message: &BrokerMessage) -> Result<(), SinkError> {
        validate_nats_subject(&message.subject)?;
        let header = format!("PUB {} {}\r\n", message.subject, message.payload.len());
        self.write_frame(header.as_bytes())?;
        self.write_frame(&message.payload)?;
        self.write_frame(b"\r\n")?;
        self.pending += 1;
        Ok(())
    }

    fn flush(&mut self) -> Result<(), SinkError> {
        if self.pending == 0 {
            return Ok(());
        }
        self.write_frame(b"PING\r\n")?;
        self.io.get_mut().flush()?;
        self.await_pong()?;
        self.pending = 0;
        Ok(())
    }
}

/// Fail-closed subject check: a publish subject must be a non-empty, whitespace-free,
/// control-character-free literal. Wildcards (`*`, `>`) are subscribe-side only — publishing
/// to one is operator error, and a subject carrying `\r`/`\n` or a space would inject a
/// forged protocol line, so both are rejected before anything is written.
fn validate_nats_subject(subject: &str) -> Result<(), SinkError> {
    if subject.is_empty() {
        return Err(SinkError::Config(
            "NATS publish subject must not be empty".to_string(),
        ));
    }
    for c in subject.chars() {
        if c.is_whitespace() || c.is_control() {
            return Err(SinkError::Config(format!(
                "NATS publish subject must not contain whitespace or control characters: {:?}",
                subject
            )));
        }
        if c == '*' || c == '>' {
            return Err(SinkError::Config(format!(
                "NATS publish subject must be a literal, not a wildcard: {:?}",
                subject
            )));
        }
    }
    Ok(())
}

/// Magic line opening a relay offset file — distinct from every change-stream/backup magic, so
/// the artifacts can never be confused.
const OFFSET_MAGIC: &str = "SPARQ-CHANGESINK-OFFSET";
/// Offset-file format version; an unknown version is rejected rather than mis-read.
const OFFSET_FORMAT_VERSION: u32 = 1;
/// Filename shape of a relay's offset file inside the change-log directory. Deliberately does
/// NOT match the change-stream segment pattern (`changestream-<seq>.cdc`), so segment
/// discovery never sees it.
const OFFSET_PREFIX: &str = "changesink-";
const OFFSET_EXT: &str = "offset";

/// What one [`BrokerRelay::pump`] delivered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PumpReport {
    /// Records handed to the sink and flushed by this pump.
    pub delivered: usize,
    /// The relay's watermark after this pump (`None` if it has still delivered nothing).
    pub delivered_through_seq: Option<u64>,
    /// `true` if the batch cap stopped the pump short — call again immediately.
    pub has_more: bool,
}

/// A resumable pump from one durable [`change_stream`](crate::change_stream) log directory to
/// one [`ChangeSink`].
///
/// The relay owns a **durable delivered-through watermark**, persisted next to the log's
/// segments as `changesink-<consumer>.offset`, so a restarted process resumes where it left
/// off instead of replaying the whole stream. Several relays with DIFFERENT consumer names can
/// read the same log independently (each keeps its own watermark); two relays sharing a
/// consumer name would fight over one watermark and is operator error.
///
/// Durability boundary: each watermark is fsync'd and atomically renamed into place, and on
/// Unix the containing directory is fsync'd too, so a crash after `pump` returns observes the
/// watermark it reported. On non-Unix targets a directory cannot be fsync'd, so a crash in
/// that window can still expose the PREVIOUS watermark and replay its records — which is why
/// delivery is at-least-once everywhere and consumers must dedupe on `sequenceNumber`.
///
/// Drive it from a host thread — never from the writer's commit hook (module docs):
///
/// ```no_run
/// # use sparq_serve::change_sink::{BrokerRelay, RecordingSink, SinkConfig};
/// let mut relay = BrokerRelay::open(
///     "/var/lib/sparq/changes",
///     "search-index",
///     RecordingSink::new(),
///     SinkConfig::new("sparq.changes"),
/// )?;
/// loop {
///     let report = relay.pump()?;
///     if !report.has_more {
///         std::thread::sleep(std::time::Duration::from_millis(200));
///     }
/// }
/// # Ok::<(), sparq_serve::change_sink::SinkError>(())
/// ```
pub struct BrokerRelay<S: ChangeSink> {
    dir: PathBuf,
    consumer: String,
    offset_path: PathBuf,
    config: SinkConfig,
    sink: S,
    delivered_through: Option<u64>,
}

// Hand-written so a relay over a non-`Debug` sink is still `Debug`.
impl<S: ChangeSink> fmt::Debug for BrokerRelay<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BrokerRelay")
            .field("dir", &self.dir)
            .field("consumer", &self.consumer)
            .field("config", &self.config)
            .field("delivered_through", &self.delivered_through)
            .finish_non_exhaustive()
    }
}

impl<S: ChangeSink> BrokerRelay<S> {
    /// Opens a relay over the change-log directory `dir`, under the durable consumer identity
    /// `consumer`, delivering to `sink`.
    ///
    /// Fail-closed: the directory must already exist (it is the writer's change log, which
    /// [`ChangeLog::open`](crate::change_stream::ChangeLog::open) creates — a relay never
    /// invents one, so a typo'd path is an error rather than a silently empty stream), the
    /// consumer name must be a short filename-safe token, and an unreadable or malformed
    /// offset file is an error rather than a silent restart-from-scratch.
    ///
    /// A relay with NO persisted offset starts at the earliest record still RETAINED in the
    /// log — there is no prior position for retention to have violated. A relay that DOES have
    /// an offset which retention has since trimmed away fails closed on
    /// [`pump`](Self::pump) instead of skipping the dropped records.
    pub fn open(
        dir: impl AsRef<Path>,
        consumer: &str,
        sink: S,
        config: SinkConfig,
    ) -> Result<Self, SinkError> {
        let dir = dir.as_ref().to_path_buf();
        if !dir.is_dir() {
            return Err(SinkError::Config(format!(
                "change-log directory {:?} does not exist — open the ChangeLog first",
                dir
            )));
        }
        validate_consumer_name(consumer)?;
        let offset_path = dir.join(format!("{}{}.{}", OFFSET_PREFIX, consumer, OFFSET_EXT));
        let delivered_through = read_offset(&offset_path)?;
        Ok(BrokerRelay {
            dir,
            consumer: consumer.to_string(),
            offset_path,
            config,
            sink,
            delivered_through,
        })
    }

    /// This relay's durable consumer identity.
    pub fn consumer(&self) -> &str {
        &self.consumer
    }

    /// The highest `seq` this relay has delivered AND flushed (`None` before the first
    /// delivery). Feed it to
    /// [`RetentionPolicy::acked_through_seq`](crate::change_stream::RetentionPolicy::acked_through_seq)
    /// so retention never drops a segment this consumer still needs.
    pub fn delivered_through_seq(&self) -> Option<u64> {
        self.delivered_through
    }

    /// The sink being driven.
    pub fn sink(&self) -> &S {
        &self.sink
    }

    /// Mutable access to the sink (e.g. to drain a [`RecordingSink`]).
    pub fn sink_mut(&mut self) -> &mut S {
        &mut self.sink
    }

    /// Gives the sink back.
    pub fn into_sink(self) -> S {
        self.sink
    }

    /// Delivers everything the log holds after the watermark, up to the configured batch cap,
    /// then flushes the sink and persists the new watermark.
    ///
    /// **Ordering of effects (this is the at-least-once boundary):** deliver → flush →
    /// persist. A crash between the flush and the persist re-delivers the batch on restart;
    /// consumers dedupe on `sequenceNumber`. The watermark is NEVER advanced past a record
    /// whose `deliver` failed, and never advanced at all if `flush` failed — a failed pump
    /// leaves the relay ready to retry exactly the records the broker did not confirm.
    ///
    /// Errors: a sink failure is returned after the successfully-delivered prefix has been
    /// flushed and committed (so progress is not thrown away); a log-read failure —
    /// including a watermark that retention has trimmed past — is returned without delivering
    /// anything.
    ///
    /// Memory note: the underlying poll materialises the records it returns, so a first pump
    /// over a long-retained log is bounded by [`SinkConfig::max_batch`] in DELIVERIES but not
    /// yet in read-side allocation; keep the cap modest and pump often on a large backlog.
    pub fn pump(&mut self) -> Result<PumpReport, SinkError> {
        let from = match self.delivered_through {
            Some(seq) => seq.saturating_add(1),
            None => earliest_retained_seq(&self.dir)?,
        };
        let mut records = read_from(&self.dir, from)?;
        let cap = self.config.effective_max_batch();
        let has_more = records.len() > cap;
        records.truncate(cap);

        let mut delivered = 0usize;
        let mut last_ok = self.delivered_through;
        let mut deliver_err = None;
        for record in &records {
            let message = encode_message(record, &self.config);
            match self.sink.deliver(&message) {
                Ok(()) => {
                    delivered += 1;
                    last_ok = Some(record.seq);
                }
                Err(e) => {
                    deliver_err = Some(e);
                    break;
                }
            }
        }

        // Flush the prefix that WAS accepted, then commit the watermark. A flush failure means
        // nothing is confirmed, so the watermark does not move at all.
        if delivered > 0 {
            self.sink.flush()?;
            if last_ok != self.delivered_through {
                if let Some(seq) = last_ok {
                    write_offset(&self.offset_path, seq)?;
                }
                self.delivered_through = last_ok;
            }
        }

        if let Some(e) = deliver_err {
            return Err(e);
        }
        Ok(PumpReport {
            delivered,
            delivered_through_seq: self.delivered_through,
            has_more,
        })
    }
}

/// A consumer name becomes part of a filename, so it is restricted to a short ASCII token.
/// Fail-closed rather than sanitising: two different names must never collapse to one
/// watermark file.
fn validate_consumer_name(consumer: &str) -> Result<(), SinkError> {
    if consumer.is_empty() || consumer.len() > 64 {
        return Err(SinkError::Config(format!(
            "relay consumer name must be 1..=64 characters, got {}",
            consumer.len()
        )));
    }
    if !consumer
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err(SinkError::Config(format!(
            "relay consumer name must contain only ASCII alphanumerics, '-', '_' or '.': {:?}",
            consumer
        )));
    }
    Ok(())
}

/// Reads a relay's persisted watermark; `None` when it has never delivered anything (no file).
/// A malformed file is an error, never a silent restart from the beginning.
fn read_offset(path: &Path) -> Result<Option<u64>, SinkError> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(SinkError::Io(e)),
    };
    let mut lines = text.lines();
    let header = lines.next().unwrap_or("");
    let expected = format!("{} {}", OFFSET_MAGIC, OFFSET_FORMAT_VERSION);
    if header != expected {
        return Err(SinkError::Config(format!(
            "not a sparq change-sink offset file (expected header {:?}, found {:?}) at {:?}",
            expected, header, path
        )));
    }
    let body = lines.next().unwrap_or("");
    let seq = body
        .strip_prefix("delivered-through ")
        .and_then(|s| s.trim().parse::<u64>().ok())
        .ok_or_else(|| {
            SinkError::Config(format!(
                "malformed change-sink offset body {:?} at {:?}",
                body, path
            ))
        })?;
    Ok(Some(seq))
}

/// fsyncs the directory CONTAINING `path`, committing the directory entry a preceding
/// `create`/`rename` only staged. Syncing a file's CONTENTS does not persist the name that
/// points at them: on POSIX, without this a crash right after the rename can resurrect the
/// old directory entry (the previous watermark) or lose a freshly-created one entirely.
#[cfg(unix)]
fn sync_parent_dir(path: &Path) -> Result<(), SinkError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::File::open(parent)?.sync_all()?;
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn sync_parent_dir(_path: &Path) -> Result<(), SinkError> {
    // Directory fsync is a POSIX notion — a directory cannot be opened as a file here. On
    // these platforms the watermark's durability is only what the filesystem gives a
    // replacing rename, which is weaker than the Unix guarantee; a crash in that window can
    // still expose the previous watermark, so consumers must dedupe on `sequenceNumber`
    // (which the delivery contract requires regardless).
    Ok(())
}

/// Persists a relay's watermark durably: write a temp file, fsync it, rename over the old one,
/// then fsync the containing directory — so a crash mid-write leaves the PREVIOUS watermark
/// intact rather than a truncated file (which would fail closed on the next open), and a crash
/// AFTER this returns observes the new watermark rather than the old one.
///
/// The trailing directory fsync is load-bearing, not belt-and-braces: `sync_data` on the temp
/// file persists only its CONTENTS, while the rename that publishes it under `path` is a
/// directory-entry change that can still be lost. Without it the durable-resume contract is
/// unmet and a restart can replay records the broker already confirmed.
///
/// Every watermark after the first renames over an EXISTING file. `fs::rename` replaces an
/// existing destination file on all supported platforms (that is its documented contract, not
/// a Unix-only accident), so the update path needs no platform-specific replace; the
/// platform-dependent case is renaming *directories*, which this never does.
///
/// Every reported failure fails SAFE — towards re-delivery, never towards a skipped record. A
/// failure before the rename cannot have touched `path`, so the previous valid watermark stands.
/// A failure AT the directory fsync is the one case where `path` already holds the new value
/// (just not durably); the error still propagates, which stops the caller from advancing its
/// in-memory watermark, so the worst outcome is re-delivering records a consumer must dedupe
/// anyway.
fn write_offset(path: &Path, seq: u64) -> Result<(), SinkError> {
    let tmp = path.with_extension(format!("{}.tmp", OFFSET_EXT));
    let body = format!(
        "{} {}\ndelivered-through {}\n",
        OFFSET_MAGIC, OFFSET_FORMAT_VERSION, seq
    );
    {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(body.as_bytes())?;
        file.flush()?;
        file.sync_data()?;
    }
    fs::rename(&tmp, path)?;
    sync_parent_dir(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::change_stream::{ChangeLog, ChangeLogConfig, RetentionPolicy};
    use crate::epoch::PodId;
    use crate::ring::GenerationRing;
    use sparq_core::Graph;

    fn graph_with(nq: &str) -> Graph {
        Graph::load_dataset(nq, "nquads").expect("seed parses")
    }

    fn scratch(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "sparq-cdc-sink-{}-{}-{:?}",
            tag,
            std::process::id(),
            std::thread::current().id()
        ))
    }

    /// A ring plus a change log holding three recorded commits (seqs 0,1,2 / generations
    /// 1,2,3). Returns the log directory.
    fn seeded_log(tmp: &Path, config: ChangeLogConfig) -> PathBuf {
        let pod = PodId::new("http://ex/g");
        let ring: GenerationRing<Graph> = GenerationRing::new(graph_with(
            "<http://ex/s0> <http://ex/p> <http://ex/o0> .\n",
        ));
        let g0 = ring.current();
        let g1 = ring.publish(
            graph_with(
                "<http://ex/s0> <http://ex/p> <http://ex/o0> .\n\
                 <http://ex/s1> <http://ex/p> \"quote\\\" and \\\\ backslash\" .\n",
            ),
            [pod.clone()],
        );
        let g2 = ring.publish(
            graph_with("<http://ex/s0> <http://ex/p> <http://ex/o0> .\n"),
            [pod.clone()],
        );
        let g3 = ring.publish(
            graph_with(
                "<http://ex/s0> <http://ex/p> <http://ex/o0> .\n\
                 <http://ex/s2> <http://ex/p> \"in-g\" <http://ex/g> .\n",
            ),
            [pod],
        );
        let mut log = ChangeLog::open_with_config(tmp, config).expect("open log");
        log.record_commit(&g0, &g1).expect("record 0->1");
        log.record_commit(&g1, &g2).expect("record 1->2");
        log.record_commit(&g2, &g3).expect("record 2->3");
        tmp.to_path_buf()
    }

    fn payload_of(message: &BrokerMessage) -> String {
        String::from_utf8(message.payload.clone()).expect("payload is UTF-8")
    }

    fn header_of<'a>(message: &'a BrokerMessage, name: &str) -> &'a str {
        message
            .headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
            .unwrap_or_else(|| panic!("header {} present", name))
    }

    /// The encoding contract: one message per COMMIT, the `GET /streams` per-change entry
    /// shape inside it, headers carrying the dedupe key, and a CONSTANT partition key.
    #[test]
    fn encodes_a_commit_as_one_broker_message() {
        let tmp = scratch("encode");
        let _ = fs::remove_dir_all(&tmp);
        let dir = seeded_log(&tmp, ChangeLogConfig::default());

        let mut relay = BrokerRelay::open(
            &dir,
            "encode-test",
            RecordingSink::new(),
            SinkConfig::new("sparq.changes").with_partition_key("shard-a"),
        )
        .expect("open relay");
        let report = relay.pump().expect("pump");
        assert_eq!(report.delivered, 3);
        assert!(!report.has_more);

        let messages = relay.sink().messages().to_vec();
        assert_eq!(messages.len(), 3, "one message per COMMIT, not per quad");
        for m in &messages {
            assert_eq!(m.subject, "sparq.changes");
            assert_eq!(
                m.key, "shard-a",
                "the partition key is constant so commit order survives partitioning"
            );
            assert_eq!(header_of(m, HEADER_CONTENT_TYPE), CONTENT_TYPE_JSON);
            assert_eq!(header_of(m, HEADER_REBASE), "false");
        }
        assert_eq!(header_of(&messages[0], HEADER_SEQ), "0");
        assert_eq!(header_of(&messages[0], HEADER_GENERATION), "1");
        assert_eq!(header_of(&messages[2], HEADER_SEQ), "2");

        // Commit 0 inserted one triple whose literal contains a `"` and a `\` — the JSON must
        // escape both (this crate hand-rolls its JSON; the escaping is load-bearing).
        let first = payload_of(&messages[0]);
        assert!(first.starts_with("{\"sequenceNumber\":0,\"generation\":1,"), "{}", first);
        assert!(first.contains("\"rebase\":false"), "{}", first);
        assert!(
            first.contains("{\"eventId\":{\"commitNum\":0,\"opNum\":1},\"op\":\"ADD\""),
            "{}",
            first
        );
        assert!(
            first.contains("\\\"quote\\\\\\\" and \\\\\\\\ backslash\\\""),
            "escaped quad literal, got {}",
            first
        );

        // Commit 1 deleted that triple.
        assert!(payload_of(&messages[1]).contains("\"op\":\"REMOVE\""));
        let _ = fs::remove_dir_all(&tmp);
    }

    /// An operator re-base GAP record must reach the broker as an explicit REBASE marker — a
    /// consumer that read it as an empty commit would believe nothing changed across a span
    /// that was never captured.
    #[test]
    fn rebase_gap_record_is_delivered_as_an_explicit_marker() {
        let tmp = scratch("rebase");
        let _ = fs::remove_dir_all(&tmp);
        let dir = seeded_log(&tmp, ChangeLogConfig::default());
        {
            let mut log = ChangeLog::open(&dir).expect("reopen");
            log.rebase_to(9).expect("rebase");
        }

        let mut relay = BrokerRelay::open(
            &dir,
            "rebase-test",
            RecordingSink::new(),
            SinkConfig::default(),
        )
        .expect("open relay");
        relay.pump().expect("pump");
        let messages = relay.sink().messages().to_vec();
        assert_eq!(messages.len(), 4);

        let gap = &messages[3];
        assert_eq!(header_of(gap, HEADER_REBASE), "true");
        assert_eq!(header_of(gap, HEADER_GENERATION), "9");
        let body = payload_of(gap);
        assert!(body.contains("\"rebase\":true"), "{}", body);
        assert!(body.contains("\"op\":\"REBASE\""), "{}", body);
        assert!(
            !body.contains("\"records\":[]"),
            "a gap must not encode as an empty commit: {}",
            body
        );
        let _ = fs::remove_dir_all(&tmp);
    }

    /// THE load-bearing relay invariant: the watermark is durable, so a restarted process
    /// resumes instead of replaying — and newly recorded commits are picked up from there.
    #[test]
    fn relay_resumes_from_its_durable_watermark_after_restart() {
        let tmp = scratch("resume");
        let _ = fs::remove_dir_all(&tmp);
        let dir = seeded_log(&tmp, ChangeLogConfig::default());

        {
            let mut relay =
                BrokerRelay::open(&dir, "svc", RecordingSink::new(), SinkConfig::default())
                    .expect("open relay");
            let report = relay.pump().expect("pump");
            assert_eq!(report.delivered, 3);
            assert_eq!(report.delivered_through_seq, Some(2));
            assert_eq!(relay.sink().flush_count(), 1, "one flush per pump");
        } // drop the relay == process restart

        // A NEW relay under the same consumer name replays NOTHING.
        let mut relay = BrokerRelay::open(&dir, "svc", RecordingSink::new(), SinkConfig::default())
            .expect("reopen relay");
        assert_eq!(relay.delivered_through_seq(), Some(2));
        let report = relay.pump().expect("pump after restart");
        assert_eq!(report.delivered, 0, "already-delivered records are not replayed");
        assert!(relay.sink().messages().is_empty());

        // A DIFFERENT consumer name keeps its own watermark and sees the whole stream.
        let mut other =
            BrokerRelay::open(&dir, "other", RecordingSink::new(), SinkConfig::default())
                .expect("open second relay");
        assert_eq!(other.delivered_through_seq(), None);
        assert_eq!(other.pump().expect("pump").delivered, 3);
        let _ = fs::remove_dir_all(&tmp);
    }

    /// The watermark file is REPLACED, not just created: a second pump renames over the
    /// already-existing offset file, and it is the SECOND value that a reopen observes. (The
    /// first write goes to a fresh path, so a create-only rename would still pass the
    /// restart test above while leaving every later watermark stuck at the first one.)
    #[test]
    fn a_second_watermark_replaces_the_first_and_survives_a_reopen() {
        let tmp = scratch("replace");
        let _ = fs::remove_dir_all(&tmp);
        let dir = seeded_log(&tmp, ChangeLogConfig::default());
        let offset_path = dir.join("changesink-repl.offset");

        {
            // A batch cap of 1 forces three separate watermark writes: create, then replace,
            // then replace again.
            let mut relay = BrokerRelay::open(
                &dir,
                "repl",
                RecordingSink::new(),
                SinkConfig::default().with_max_batch(1),
            )
            .expect("open relay");
            assert_eq!(relay.pump().expect("pump 1").delivered_through_seq, Some(0));
            assert!(offset_path.is_file(), "the first pump creates the watermark");
            assert_eq!(relay.pump().expect("pump 2").delivered_through_seq, Some(1));
            assert_eq!(relay.pump().expect("pump 3").delivered_through_seq, Some(2));
        } // drop == process restart

        // The on-disk file holds the LAST watermark, and no temp file is left behind.
        assert!(
            fs::read_to_string(&offset_path)
                .expect("read watermark")
                .contains("delivered-through 2"),
            "the replaced watermark must hold the newest seq"
        );
        assert!(
            !dir.join("changesink-repl.offset.tmp").exists(),
            "the temp file is consumed by the rename"
        );

        // And a reopen resumes from it rather than from the first watermark.
        let mut reopened =
            BrokerRelay::open(&dir, "repl", RecordingSink::new(), SinkConfig::default())
                .expect("reopen relay");
        assert_eq!(reopened.delivered_through_seq(), Some(2));
        assert_eq!(
            reopened.pump().expect("pump after restart").delivered,
            0,
            "a replaced watermark must not replay records it already covered"
        );
        let _ = fs::remove_dir_all(&tmp);
    }

    /// Direct coverage of the durable-replace helper `write_offset` — create, replace, and the
    /// trailing parent-directory fsync — over a bare directory, with no relay in the way.
    ///
    /// HONESTY BOUND: an in-process test cannot establish CRASH durability. Nothing here
    /// proves the directory entry survives a power cut; only a fault-injection or
    /// crash-restart harness could, and that is out of scope for a unit test. What this pins
    /// is the sequence and its error reporting.
    #[test]
    fn write_offset_replaces_durably_and_reports_a_failed_sync() {
        let tmp = scratch("write-offset");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create scratch dir");
        let path = tmp.join("direct.offset");

        write_offset(&path, 7).expect("first watermark: create + fsync + rename + dir fsync");
        assert_eq!(read_offset(&path).expect("read back"), Some(7));

        // The replace path renames over an existing file and re-syncs the directory entry.
        write_offset(&path, 41).expect("second watermark: replace");
        assert_eq!(
            read_offset(&path).expect("read back replaced"),
            Some(41),
            "the replaced watermark must hold the newest seq"
        );
        assert!(
            !tmp.join("direct.offset.tmp").exists(),
            "the temp file is consumed by the rename"
        );

        // The parent-directory fsync is REACHED, not just defined. A directory that is
        // writable+searchable but not READABLE still accepts the create and the rename, and
        // fails only at the `File::open` the sync needs — so this assertion goes red if the
        // post-rename sync is dropped, which the reopen tests above cannot detect.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let closed = scratch("write-offset-nosync");
            let _ = fs::remove_dir_all(&closed);
            fs::create_dir_all(&closed).expect("create scratch dir");
            let closed_path = closed.join("direct.offset");
            fs::set_permissions(&closed, fs::Permissions::from_mode(0o300))
                .expect("drop read permission");
            // Root bypasses the permission bits, so only assert where the denial is real.
            if fs::File::open(&closed).is_err() {
                assert!(
                    write_offset(&closed_path, 5).is_err(),
                    "a watermark whose directory entry cannot be fsync'd must be reported"
                );
            }
            let _ = fs::set_permissions(&closed, fs::Permissions::from_mode(0o700));
            let _ = fs::remove_dir_all(&closed);
        }

        // A watermark written into a directory that does not exist fails rather than
        // reporting a durable write it never made.
        let missing = tmp.join("no-such-dir").join("direct.offset");
        assert!(
            write_offset(&missing, 1).is_err(),
            "a write that cannot be made durable must be reported, not swallowed"
        );

        let _ = fs::remove_dir_all(&tmp);
    }

    /// The batch cap bounds one pump and reports that more is waiting.
    #[test]
    fn max_batch_bounds_a_pump_and_reports_more() {
        let tmp = scratch("batch");
        let _ = fs::remove_dir_all(&tmp);
        let dir = seeded_log(&tmp, ChangeLogConfig::default());

        let mut relay = BrokerRelay::open(
            &dir,
            "batched",
            RecordingSink::new(),
            SinkConfig::default().with_max_batch(2),
        )
        .expect("open relay");
        let first = relay.pump().expect("pump 1");
        assert_eq!((first.delivered, first.has_more), (2, true));
        assert_eq!(first.delivered_through_seq, Some(1));
        let second = relay.pump().expect("pump 2");
        assert_eq!((second.delivered, second.has_more), (1, false));
        assert_eq!(second.delivered_through_seq, Some(2));
        let _ = fs::remove_dir_all(&tmp);
    }

    /// A sink that fails on its Nth delivery.
    struct FlakySink {
        inner: RecordingSink,
        fail_after: usize,
        delivered: usize,
    }

    impl ChangeSink for FlakySink {
        fn deliver(&mut self, message: &BrokerMessage) -> Result<(), SinkError> {
            if self.delivered >= self.fail_after {
                return Err(SinkError::Broker("broker unavailable".to_string()));
            }
            self.delivered += 1;
            self.inner.deliver(message)
        }

        fn flush(&mut self) -> Result<(), SinkError> {
            self.inner.flush()
        }
    }

    /// A broker outage must not advance the watermark past what it accepted — the failed
    /// record is retried, never skipped.
    #[test]
    fn a_sink_failure_does_not_advance_the_watermark_past_it() {
        let tmp = scratch("flaky");
        let _ = fs::remove_dir_all(&tmp);
        let dir = seeded_log(&tmp, ChangeLogConfig::default());

        let sink = FlakySink {
            inner: RecordingSink::new(),
            fail_after: 2,
            delivered: 0,
        };
        let mut relay =
            BrokerRelay::open(&dir, "flaky", sink, SinkConfig::default()).expect("open relay");
        let err = relay.pump().expect_err("the third delivery fails");
        assert!(matches!(err, SinkError::Broker(_)), "{:?}", err);
        assert_eq!(
            relay.delivered_through_seq(),
            Some(1),
            "the accepted prefix is committed, the failed record is not"
        );

        // A fresh relay under the same consumer resumes AT the record that failed.
        let mut retry =
            BrokerRelay::open(&dir, "flaky", RecordingSink::new(), SinkConfig::default())
                .expect("reopen relay");
        let report = retry.pump().expect("retry pump");
        assert_eq!(report.delivered, 1);
        assert_eq!(header_of(&retry.sink().messages()[0], HEADER_SEQ), "2");
        let _ = fs::remove_dir_all(&tmp);
    }

    /// Retention that trimmed past a relay's watermark FAILS the pump closed — silently
    /// resuming at the new earliest record would drop changes the consumer never saw. (A
    /// deployment avoids this by feeding `delivered_through_seq` into the retention policy.)
    #[test]
    fn a_watermark_trimmed_away_by_retention_fails_the_pump_closed() {
        let tmp = scratch("trimmed");
        let _ = fs::remove_dir_all(&tmp);
        // One record per segment, so retention can drop whole segments record-by-record.
        let dir = seeded_log(
            &tmp,
            ChangeLogConfig {
                segment_target_bytes: 1,
                fsync: false,
            },
        );

        let mut relay = BrokerRelay::open(
            &dir,
            "slow",
            RecordingSink::new(),
            SinkConfig::default().with_max_batch(1),
        )
        .expect("open relay");
        assert_eq!(relay.pump().expect("pump 1").delivered_through_seq, Some(0));

        // Now drop everything but the active segment while the relay still needs seq 1.
        {
            let mut log = ChangeLog::open(&dir).expect("reopen log");
            let report = log
                .apply_retention(&RetentionPolicy {
                    max_total_bytes: Some(0),
                    ..RetentionPolicy::default()
                })
                .expect("retention");
            assert!(report.segments_dropped > 0, "the test needs a real trim");
            assert!(report.first_retained_seq > 1, "seq 1 must be gone");
        }

        let err = relay.pump().expect_err("the trimmed offset fails closed");
        assert!(matches!(err, SinkError::Log(_)), "{:?}", err);
        let _ = fs::remove_dir_all(&tmp);
    }

    /// A relay with no persisted offset starts at the earliest RETAINED record — there is no
    /// prior position for retention to have violated.
    #[test]
    fn a_fresh_relay_starts_at_the_earliest_retained_record() {
        let tmp = scratch("fresh");
        let _ = fs::remove_dir_all(&tmp);
        let dir = seeded_log(
            &tmp,
            ChangeLogConfig {
                segment_target_bytes: 1,
                fsync: false,
            },
        );
        let first_retained = {
            let mut log = ChangeLog::open(&dir).expect("reopen log");
            log.apply_retention(&RetentionPolicy {
                max_total_bytes: Some(0),
                ..RetentionPolicy::default()
            })
            .expect("retention")
            .first_retained_seq
        };
        assert!(first_retained > 0, "the test needs a real trim");

        let mut relay = BrokerRelay::open(&dir, "new", RecordingSink::new(), SinkConfig::default())
            .expect("open relay");
        let report = relay.pump().expect("pump");
        assert_eq!(report.delivered, 1);
        assert_eq!(
            header_of(&relay.sink().messages()[0], HEADER_SEQ),
            first_retained.to_string()
        );
        let _ = fs::remove_dir_all(&tmp);
    }

    /// Configuration is fail-closed: a consumer name that is not filename-safe, and a missing
    /// log directory, are errors rather than a silently wrong watermark file / empty stream.
    #[test]
    fn relay_configuration_is_fail_closed() {
        let tmp = scratch("config");
        let _ = fs::remove_dir_all(&tmp);
        let dir = seeded_log(&tmp, ChangeLogConfig::default());

        for bad in ["", "has space", "../escape", "a/b"] {
            let err = BrokerRelay::open(&dir, bad, RecordingSink::new(), SinkConfig::default())
                .expect_err("bad consumer name is rejected");
            assert!(matches!(err, SinkError::Config(_)), "{:?}", err);
        }
        let err = BrokerRelay::open(
            dir.join("nope"),
            "svc",
            RecordingSink::new(),
            SinkConfig::default(),
        )
        .expect_err("missing directory is rejected");
        assert!(matches!(err, SinkError::Config(_)), "{:?}", err);
        let _ = fs::remove_dir_all(&tmp);
    }

    /// A corrupt offset file fails closed instead of silently replaying the stream.
    #[test]
    fn a_corrupt_offset_file_fails_closed() {
        let tmp = scratch("badoffset");
        let _ = fs::remove_dir_all(&tmp);
        let dir = seeded_log(&tmp, ChangeLogConfig::default());
        fs::write(dir.join("changesink-svc.offset"), b"garbage\n").expect("write bad offset");

        let err = BrokerRelay::open(&dir, "svc", RecordingSink::new(), SinkConfig::default())
            .expect_err("a malformed offset file is rejected");
        assert!(matches!(err, SinkError::Config(_)), "{:?}", err);
        let _ = fs::remove_dir_all(&tmp);
    }

    /// A scripted in-memory byte stream: canned server lines in, captured client bytes out.
    struct MockIo {
        reads: io::Cursor<Vec<u8>>,
        writes: Vec<u8>,
    }

    impl MockIo {
        fn new(script: &str) -> Self {
            MockIo {
                reads: io::Cursor::new(script.as_bytes().to_vec()),
                writes: Vec::new(),
            }
        }
    }

    impl Read for MockIo {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.reads.read(buf)
        }
    }

    impl Write for MockIo {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.writes.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn message(payload: &str) -> BrokerMessage {
        BrokerMessage {
            subject: "sparq.changes".to_string(),
            key: "sparq.changes".to_string(),
            headers: Vec::new(),
            payload: payload.as_bytes().to_vec(),
        }
    }

    /// The NATS publisher speaks the protocol a publisher needs: INFO in, CONNECT out, PUB
    /// with a correct BYTE length, and PING/PONG as the flush barrier.
    #[test]
    fn nats_sink_handshakes_publishes_and_flushes() {
        // INFO greeting, PONG for the handshake ping, PONG for the flush ping.
        let io = MockIo::new("INFO {\"server_id\":\"x\"}\r\nPONG\r\nPONG\r\n");
        let options = NatsOptions {
            client_name: "sparq-relay".to_string(),
            auth_token: Some("s3cr3t".to_string()),
            timeout: None,
        };
        let mut sink = NatsSink::handshake(io, &options).expect("handshake");
        // A multi-byte payload: the PUB length is in BYTES, not chars.
        sink.deliver(&message("{\"x\":\"é\"}")).expect("publish");
        sink.flush().expect("flush");

        let written = String::from_utf8(sink.stream().writes.clone()).expect("utf8");
        assert!(written.starts_with("CONNECT {"), "{}", written);
        assert!(written.contains("\"verbose\":false"), "{}", written);
        assert!(written.contains("\"name\":\"sparq-relay\""), "{}", written);
        assert!(written.contains("\"auth_token\":\"s3cr3t\""), "{}", written);
        assert!(
            // 9 CHARS, 10 BYTES — the length NATS wants is the byte count.
            written.contains("PUB sparq.changes 10\r\n{\"x\":\"é\"}\r\n"),
            "{}",
            written
        );
        assert_eq!(written.matches("PING\r\n").count(), 2, "handshake + flush");

        // The redacting Debug must not leak the token.
        let shown = format!("{:?}", options);
        assert!(!shown.contains("s3cr3t"), "{}", shown);
        assert!(shown.contains("redacted"), "{}", shown);
    }

    /// A `-ERR` from the broker (e.g. an authorization rejection) surfaces as a broker error,
    /// and a hung-up connection as a protocol error — neither is mistaken for success.
    #[test]
    fn nats_sink_surfaces_server_errors() {
        let io = MockIo::new("INFO {}\r\n-ERR 'Authorization Violation'\r\n");
        let err = NatsSink::handshake(io, &NatsOptions::default()).expect_err("rejected");
        assert!(matches!(err, SinkError::Broker(_)), "{:?}", err);

        let io = MockIo::new("INFO {}\r\n");
        let err = NatsSink::handshake(io, &NatsOptions::default()).expect_err("hung up");
        assert!(matches!(err, SinkError::Protocol(_)), "{:?}", err);

        let io = MockIo::new("+OK\r\n");
        let err = NatsSink::handshake(io, &NatsOptions::default()).expect_err("no greeting");
        assert!(matches!(err, SinkError::Protocol(_)), "{:?}", err);
    }

    /// A server-initiated PING while we wait for our PONG is answered, not treated as a
    /// protocol violation.
    #[test]
    fn nats_sink_answers_a_server_ping_while_awaiting_pong() {
        let io = MockIo::new("INFO {}\r\nPING\r\nPONG\r\n");
        let sink = NatsSink::handshake(io, &NatsOptions::default()).expect("handshake");
        let written = String::from_utf8(sink.stream().writes.clone()).expect("utf8");
        assert!(written.contains("PONG\r\n"), "answered the server ping: {}", written);
    }

    /// A subject that is a wildcard, empty, or carries whitespace/control characters is
    /// rejected BEFORE anything is written — a `\r\n` in a subject would forge a protocol line.
    #[test]
    fn nats_sink_rejects_unpublishable_subjects() {
        for bad in ["", "sparq.*", "sparq.>", "sparq changes", "sparq\r\nPUB x 0"] {
            let io = MockIo::new("INFO {}\r\nPONG\r\n");
            let mut sink = NatsSink::handshake(io, &NatsOptions::default()).expect("handshake");
            let mut msg = message("{}");
            msg.subject = bad.to_string();
            let err = sink.deliver(&msg).expect_err("unpublishable subject");
            assert!(matches!(err, SinkError::Config(_)), "{:?} for {:?}", err, bad);
            let written = String::from_utf8_lossy(&sink.stream().writes).to_string();
            assert!(!written.contains("PUB"), "nothing was written: {}", written);
        }
    }
}
