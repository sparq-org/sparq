use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sparq_zk::field::{field_from_hex_str_strict, Fr};

pub(crate) type Result<T> = std::result::Result<T, String>;
pub(crate) fn parse<T: DeserializeOwned>(s: &str) -> Result<T> {
    serde_json::from_str(s).map_err(|e| format!("JSON input rejected: {e}"))
}
pub(crate) fn json(value: &impl Serialize) -> Result<String> {
    fn sort(value: &mut Value) {
        match value {
            Value::Object(map) => { for value in map.values_mut() { sort(value); } map.sort_keys(); },
            Value::Array(values) => { for value in values { sort(value); } },
            _ => {},
        }
    }
    let mut value=serde_json::to_value(value).map_err(err)?;sort(&mut value);
    serde_json::to_string(&value).map_err(err)
}
pub(crate) fn err(e: impl std::fmt::Display) -> String { e.to_string() }
pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
pub(crate) fn bytes(s: &str) -> Result<Vec<u8>> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    if !s.len().is_multiple_of(2) || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("hex encoding rejected".into());
    }
    s.as_bytes().chunks_exact(2).map(|pair| {
        u8::from_str_radix(std::str::from_utf8(pair).map_err(err)?, 16).map_err(err)
    }).collect()
}
pub(crate) fn word(s: &str) -> Result<[u8; 32]> {
    bytes(s)?.try_into().map_err(|_| "expected a full challenge or seed".into())
}
pub(crate) fn field(s: &str) -> Result<Fr> {
    field_from_hex_str_strict(s).ok_or_else(|| "noncanonical field encoding".into())
}
pub(crate) fn digest(bytes: &[u8]) -> String { hex(&Sha256::digest(bytes)) }
pub(crate) fn noir_numbers(value: Value) -> Value {
    match value {
        Value::Number(n) => Value::String(n.to_string()),
        Value::Array(v) => Value::Array(v.into_iter().map(noir_numbers).collect()),
        Value::Object(v) => Value::Object(v.into_iter().map(|(k,v)| (k,noir_numbers(v))).collect()),
        v => v,
    }
}

// This converts only the query crate renderer's inline TOML vocabulary into JSON.
// Quoted strings are already JSON escaped; no query or witness logic lives here.
pub(crate) fn witness_json(input: &str) -> Result<Value> {
    let mut result = serde_json::Map::new();
    for line in input.lines() {
        let (name, value) = line.split_once(" = ").ok_or("witness serialization rejected")?;
        let bytes = value.as_bytes(); let mut out = String::new(); let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'"' {
                let start = i; i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' { i += 2; }
                    else if bytes[i] == b'"' { i += 1; break; }
                    else { i += 1; }
                }
                out.push_str(value.get(start..i).ok_or("witness string rejected")?);
            } else if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
                let start = i;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') { i += 1; }
                let word = &value[start..i];
                if word == "true" || word == "false" { out.push_str(word); }
                else { out.push('"'); out.push_str(word); out.push('"'); }
            } else { out.push(if bytes[i] == b'=' { ':' } else { bytes[i] as char }); i += 1; }
        }
        result.insert(name.into(), serde_json::from_str(&out).map_err(err)?);
    }
    Ok(noir_numbers(Value::Object(result)))
}
