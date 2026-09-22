#![forbid(unsafe_code)]
//! Platform-independent data validation. No device or GUI dependencies.
pub mod assets;
pub mod books;
pub mod passthm;
pub mod scanner;
use plist::Value;
use serde::{Deserialize, Serialize};
use std::io::Cursor;
use thiserror::Error;

pub const MAX_PLIST_BYTES: usize = 1024 * 1024;
#[derive(Debug, Error)]
pub enum Error {
    #[error("unsafe relative path: empty, traversal, separator, drive prefix or control character")]
    UnsafePath,
    #[error("plist limit exceeded (1 MiB, 32 nesting levels, 16384 nodes)")]
    Limit,
    #[error("expected binary plist with bplist00 header")]
    NotBinary,
    #[error("invalid plist: {0}")]
    Plist(#[from] plist::Error),
}
pub fn safe_relative_path(path: &str) -> Result<(), Error> {
    if path.is_empty()
        || path.len() > 1024
        || path.contains(['\\', ':', '\0'])
        || path.chars().any(char::is_control)
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(Error::UnsafePath);
    }
    Ok(())
}
pub fn safe_leaf(path: &str) -> Result<(), Error> {
    safe_relative_path(path)?;
    if path.contains('/') || path.len() > 255 {
        return Err(Error::UnsafePath);
    }
    Ok(())
}
fn validate_value(value: &Value, depth: usize, nodes: &mut usize) -> Result<(), Error> {
    *nodes += 1;
    if depth > 32 || *nodes > 16384 {
        return Err(Error::Limit);
    }
    match value {
        Value::Array(values) => {
            for value in values {
                validate_value(value, depth + 1, nodes)?;
            }
        }
        Value::Dictionary(values) => {
            for value in values.values() {
                validate_value(value, depth + 1, nodes)?;
            }
        }
        _ => {}
    }
    Ok(())
}
pub fn decode_binary(bytes: &[u8]) -> Result<Value, Error> {
    if !bytes.starts_with(b"bplist00") {
        return Err(Error::NotBinary);
    }
    decode_plist(bytes)
}
/// Bounded XML/binary/ASCII plist decoder for local resource and Books metadata.
pub fn decode_plist(bytes: &[u8]) -> Result<Value, Error> {
    if bytes.len() > MAX_PLIST_BYTES {
        return Err(Error::Limit);
    }
    // Bound the event stream BEFORE building a recursive Value. Binary plists can
    // reference the same object repeatedly, so wire size alone is not a memory bound.
    use plist::stream::{Event, Reader};
    let mut events = Vec::new();
    let mut depth = 0usize;
    let mut expanded_bytes = 0usize;
    for event in Reader::new(Cursor::new(bytes)) {
        let event = event?;
        match &event {
            Event::StartArray(length) | Event::StartDictionary(length) => {
                depth += 1;
                if depth > 32 || length.is_some_and(|n| n > 16384) {
                    return Err(Error::Limit);
                }
            }
            Event::EndCollection => {
                depth = depth.saturating_sub(1);
            }
            Event::Data(data) => expanded_bytes += data.len(),
            Event::String(s) => expanded_bytes += s.len(),
            _ => {}
        }
        if events.len() >= 16384 || expanded_bytes > MAX_PLIST_BYTES {
            return Err(Error::Limit);
        }
        events.push(Ok(event));
    }
    let value = Value::from_events(events)?;
    validate_value(&value, 0, &mut 0)?;
    Ok(value)
}
pub fn encode_binary(value: &Value) -> Result<Vec<u8>, Error> {
    validate_value(value, 0, &mut 0)?;
    let mut bytes = Vec::new();
    value.to_writer_binary(&mut bytes)?;
    if bytes.len() > MAX_PLIST_BYTES {
        return Err(Error::Limit);
    }
    Ok(bytes)
}
/// Diagnostic metadata only. This is deliberately not a Books backup or restore artifact.
#[derive(Debug, Serialize, Deserialize)]
pub struct DiagnosticSnapshot {
    pub schema_version: u32,
    pub kind: String,
    pub ios_version: String,
    pub transport: String,
    pub pairing: String,
    pub afc_root_entries: usize,
    pub afc_tls: bool,
}

/// Incremental syslog framing with a hard per-line bound. Overlong lines are discarded fully.
#[derive(Default)]
pub struct LogLines {
    pending: Vec<u8>,
    dropping: bool,
}
impl LogLines {
    pub fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        let mut lines = Vec::new();
        for &byte in bytes {
            if byte == b'\n' || byte == 0 {
                if !self.dropping && !self.pending.is_empty() {
                    lines.push(String::from_utf8_lossy(&self.pending).into_owned());
                }
                self.pending.clear();
                self.dropping = false;
            } else if !self.dropping {
                if self.pending.len() >= 16384 {
                    self.pending.clear();
                    self.dropping = true;
                } else {
                    self.pending.push(byte);
                }
            }
        }
        lines
    }
}
/// Stable checksum for local integrity/device-binding; callers must keep device digests private.
pub fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paths_reject_traversal_and_windows_escapes() {
        for bad in [
            "", "/a", "../a", "a/../b", "a//b", "a/./b", "C:abc", "a\\b", "a\0b", "a\nb",
        ] {
            assert!(safe_relative_path(bad).is_err(), "{bad:?}");
        }
        assert!(safe_relative_path("themes/en-2-A B C.png").is_ok());
        assert!(safe_leaf("a/b").is_err());
    }
    #[test]
    fn binary_roundtrip_and_bounds() {
        let value = Value::Array(vec![Value::String("Book".into()), Value::Boolean(true)]);
        assert_eq!(
            decode_binary(&encode_binary(&value).unwrap()).unwrap(),
            value
        );
        assert!(decode_binary(b"<?xml version='1.0'?>").is_err());
        assert!(decode_binary(b"bplist00broken").is_err());
        assert!(decode_binary(&vec![0; MAX_PLIST_BYTES + 1]).is_err());
        let mut nested = Value::Boolean(true);
        for _ in 0..34 {
            nested = Value::Array(vec![nested]);
        }
        assert!(encode_binary(&nested).is_err());
    }
    #[test]
    fn binary_expansion_and_depth_limited_before_tree_construction() {
        let value = Value::Array(vec![Value::String("x".repeat(400_000)); 3]);
        let mut wire = Vec::new();
        value.to_writer_binary(&mut wire).unwrap();
        assert!(wire.len() < MAX_PLIST_BYTES); // binary writer deduplicates repeated strings
        assert!(matches!(decode_binary(&wire), Err(Error::Limit)));
        let mut deep = Value::Boolean(true);
        for _ in 0..40 {
            deep = Value::Array(vec![deep]);
        }
        wire.clear();
        deep.to_writer_binary(&mut wire).unwrap();
        assert!(matches!(decode_binary(&wire), Err(Error::Limit)));
    }
    #[test]
    fn scanner_filters_wallet_and_noise() {
        use base64::Engine;
        let bytes: Vec<u8> = (0..20).map(|x| x * 11).collect();
        let hash = base64::engine::general_purpose::STANDARD.encode(bytes);
        assert_eq!(
            scanner::extract_card_hash_from_line(&format!("passd card_hash={hash}")),
            Some(hash.clone())
        );
        assert!(scanner::extract_card_hash_from_line(&format!("unrelated {hash}")).is_none());
        for bad in [
            "AAAAAAAAAAAAAAAAAAAAAAAAAAA=",
            "M6nDwZrkYbFlsodLgCbvyFZQ1cc=",
            "not-a-hash",
        ] {
            assert!(!scanner::is_valid_card_hash(bad));
        }
    }
    #[test]
    fn syslog_fragmentation_and_oversize_recovery() {
        let mut decoder = LogLines::default();
        assert!(decoder.push(b"pass").is_empty());
        assert_eq!(decoder.push(b"d\0next\n"), vec!["passd", "next"]);
        assert!(decoder.push(&vec![b'x'; 17000]).is_empty());
        assert_eq!(decoder.push(b"discard\nvalid\n"), vec!["valid"]);
    }
}
