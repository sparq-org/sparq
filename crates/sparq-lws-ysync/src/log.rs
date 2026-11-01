use crate::Fault;
use yrs::{merge_updates_v1, updates::decoder::Decode, Update};

/// Resource kind in a room's append-only log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogKind {
    Update,
    Snapshot,
}

/// A room-log envelope. Integer fields use network byte order.
#[derive(Clone, Debug)]
pub struct LogFile {
    pub kind: LogKind,
    pub millis: u64,
    pub nonce: u32,
    pub update: Vec<u8>,
}

impl LogFile {
    /// Parse a log resource independently of its filename.
    pub fn decode(bytes: &[u8]) -> Result<Self, Fault> {
        if bytes.len() < 16 {
            return Err(Fault::Malformed);
        }
        let kind = match &bytes[..4] {
            b"JYU1" => LogKind::Update,
            b"JYS1" => LogKind::Snapshot,
            _ => return Err(Fault::Malformed),
        };
        let update = bytes[16..].to_vec();
        Update::decode_v1(&update).map_err(|_| Fault::Malformed)?;
        Ok(Self {
            kind,
            millis: u64::from_be_bytes(bytes[4..12].try_into().unwrap()),
            nonce: u32::from_be_bytes(bytes[12..16].try_into().unwrap()),
            update,
        })
    }

    /// Encode the fixed envelope followed by a Yjs v1 update.
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(16 + self.update.len());
        bytes.extend_from_slice(match self.kind {
            LogKind::Update => b"JYU1",
            LogKind::Snapshot => b"JYS1",
        });
        bytes.extend_from_slice(&self.millis.to_be_bytes());
        bytes.extend_from_slice(&self.nonce.to_be_bytes());
        bytes.extend_from_slice(&self.update);
        bytes
    }
}

/// Select the newest snapshot, then merge later updates in envelope order.
/// The returned state retains unresolved Yjs dependencies as well as integrated structs.
pub fn load(mut files: Vec<LogFile>, limit: usize) -> Result<Vec<u8>, Fault> {
    files.sort_by_key(|f| (f.millis, f.nonce));
    let snapshot = files.iter().rposition(|f| f.kind == LogKind::Snapshot);
    let since = snapshot.map(|i| files[i].millis).unwrap_or(0);
    let mut state = vec![0, 0];
    if let Some(i) = snapshot {
        state = files[i].update.clone();
    }
    for file in files
        .iter()
        .filter(|f| f.kind == LogKind::Update && f.millis >= since)
    {
        state = merge_updates_v1([state.as_slice(), &file.update]).map_err(|_| Fault::Malformed)?;
        if state.len() > limit {
            return Err(Fault::TooLarge);
        }
    }
    if state.len() > limit {
        return Err(Fault::TooLarge);
    }
    Ok(state)
}
