use std::collections::HashMap;
use std::sync::Arc;

use yrs::encoding::read::Cursor;
use yrs::sync::awareness::AwarenessUpdateEntry;
use yrs::sync::{AwarenessUpdate, Message, SyncMessage};
use yrs::updates::{
    decoder::{Decode, Decoder, DecoderV1},
    encoder::Encode,
};
use yrs::{merge_updates_v1, ClientID, Doc, ReadTxn, Transact, Update};

/// Relay termination reason, mapped directly to the socket close code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Fault {
    #[error("invalid ticket")]
    Ticket,
    #[error("forbidden")]
    Forbidden,
    #[error("message or document limit")]
    TooLarge,
    #[error("capacity")]
    Capacity,
    #[error("invalid protocol frame")]
    Malformed,
    #[error("room storage unavailable")]
    Store,
}

impl Fault {
    pub fn code(self) -> u16 {
        match self {
            Self::Ticket => 4401,
            Self::Forbidden => 4403,
            Self::TooLarge => 4413,
            Self::Capacity => 4429,
            Self::Malformed => 1002,
            Self::Store => 1011,
        }
    }
}

pub(crate) struct Presence {
    owner: u64,
    entry: AwarenessUpdateEntry,
}

pub(crate) struct Document {
    pub state: Vec<u8>,
    doc: Doc,
    presence: HashMap<ClientID, Presence>,
}

impl Document {
    pub fn new(state: Vec<u8>) -> Result<Self, Fault> {
        let doc = Doc::new();
        doc.transact_mut()
            .apply_update(Update::decode_v1(&state).map_err(|_| Fault::Malformed)?)
            .map_err(|_| Fault::Malformed)?;
        Ok(Self {
            state,
            doc,
            presence: HashMap::new(),
        })
    }

    pub fn start(&self) -> Vec<Vec<u8>> {
        vec![
            Message::Sync(SyncMessage::SyncStep1(self.doc.transact().state_vector())).encode_v1(),
            self.awareness().encode_v1(),
        ]
    }

    fn awareness(&self) -> Message {
        Message::Awareness(AwarenessUpdate {
            clients: self
                .presence
                .iter()
                .filter(|(_, p)| p.entry.json.as_ref() != "null")
                .map(|(id, p)| (*id, p.entry.clone()))
                .collect(),
        })
    }

    pub fn candidate(
        &self,
        update: &[u8],
        writable: bool,
        limit: usize,
    ) -> Result<Option<(Vec<u8>, Doc)>, Fault> {
        let merged =
            merge_updates_v1([self.state.as_slice(), update]).map_err(|_| Fault::Malformed)?;
        if merged == self.state {
            return Ok(None);
        }
        // This check precedes every state change and persistence admission.
        if !writable {
            return Err(Fault::Forbidden);
        }
        if merged.len() > limit {
            return Err(Fault::TooLarge);
        }
        let doc = Doc::new();
        doc.transact_mut()
            .apply_update(Update::decode_v1(&merged).map_err(|_| Fault::Malformed)?)
            .map_err(|_| Fault::Malformed)?;
        Ok(Some((merged, doc)))
    }

    pub fn replace(&mut self, state: Vec<u8>, doc: Doc) {
        self.doc = doc;
        self.state = state;
    }

    pub fn merge(&mut self, update: &[u8], writable: bool, limit: usize) -> Result<bool, Fault> {
        if let Some((state, doc)) = self.candidate(update, writable, limit)? {
            self.replace(state, doc);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn reply(&self, message: Message) -> Result<Vec<u8>, Fault> {
        Ok(match message {
            Message::Sync(SyncMessage::SyncStep1(sv)) => Message::Sync(SyncMessage::SyncStep2(
                self.doc.transact().encode_state_as_update_v1(&sv),
            ))
            .encode_v1(),
            Message::AwarenessQuery => self.awareness().encode_v1(),
            Message::Auth(None) => return Ok(Vec::new()),
            _ => return Err(Fault::Malformed),
        })
    }

    pub fn presence(
        &mut self,
        peer: u64,
        update: AwarenessUpdate,
        max_ids: usize,
        max_bytes: usize,
    ) -> Result<Vec<u8>, Fault> {
        let new_ids = update
            .clients
            .keys()
            .filter(|id| !self.presence.contains_key(id))
            .count();
        if self.presence.len() + new_ids > max_ids {
            return Err(Fault::Capacity);
        }
        let mut size: usize = self.presence.values().map(|p| p.entry.json.len()).sum();
        for (id, entry) in &update.clients {
            if serde_json::from_str::<serde_json::Value>(&entry.json).is_err() {
                return Err(Fault::Malformed);
            }
            if let Some(p) = self.presence.get(id) {
                if p.owner != peer {
                    return Err(Fault::Forbidden);
                }
                size = size.saturating_sub(p.entry.json.len());
            }
            size = size.saturating_add(entry.json.len());
        }
        if size > max_bytes {
            return Err(Fault::TooLarge);
        }
        let mut clients = HashMap::new();
        for (id, entry) in update.clients {
            let accept = self.presence.get(&id).is_none_or(|p| {
                entry.clock > p.entry.clock
                    || (entry.clock == p.entry.clock && entry.json.as_ref() == "null")
            });
            if accept {
                clients.insert(id, entry.clone());
                self.presence.insert(id, Presence { owner: peer, entry });
            }
        }
        Ok(Message::Awareness(AwarenessUpdate { clients }).encode_v1())
    }

    pub fn leave(&mut self, peer: u64) -> Vec<u8> {
        let mut clients = HashMap::new();
        self.presence.retain(|id, p| {
            if p.owner == peer {
                let mut entry = p.entry.clone();
                entry.json = Arc::from("null");
                clients.insert(*id, entry);
                false
            } else {
                true
            }
        });
        Message::Awareness(AwarenessUpdate { clients }).encode_v1()
    }
}

pub(crate) fn messages(bytes: &[u8]) -> Result<Vec<Message>, Fault> {
    let mut remaining = bytes;
    let mut result = Vec::new();
    while !remaining.is_empty() {
        let mut decoder = DecoderV1::new(Cursor::new(remaining));
        result.push(Message::decode(&mut decoder).map_err(|_| Fault::Malformed)?);
        let left = decoder.read_to_end().map_err(|_| Fault::Malformed)?.len();
        remaining = &remaining[remaining.len() - left..];
    }
    Ok(result)
}
