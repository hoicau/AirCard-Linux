//! Passive framing hypotheses only. Never transmit a candidate encoding.
use aircard_core::{MAX_PLIST_BYTES, decode_binary};
use plist::Value;
use serde::Serialize;
#[derive(Debug, Serialize)]
pub struct Candidate {
    pub byte_order: &'static str,
    pub length_includes_header: bool,
    pub frame_lengths: Vec<usize>,
    pub complete_bytes: usize,
    pub trailing_bytes: usize,
    pub valid_binary_plists: usize,
    pub known_message_names: Vec<String>,
    pub recognized_envelope_keys: Vec<String>,
    pub unrecognized_key_count: usize,
    pub sync_allowed_envelopes: usize,
    pub envelope_shapes: Vec<EnvelopeShape>,
}
#[derive(Debug, Serialize)]
pub struct EnvelopeShape {
    pub command: &'static str,
    pub params_kind: &'static str,
    pub type_kind: &'static str,
}
fn kind(v: Option<&Value>) -> &'static str {
    match v {
        Some(Value::Dictionary(_)) => "dictionary",
        Some(Value::Array(_)) => "array",
        Some(Value::String(_)) => "string",
        Some(Value::Integer(_)) => "integer",
        Some(Value::Boolean(_)) => "boolean",
        None => "absent",
        _ => "other",
    }
}
const MESSAGE_NAMES: &[&str] = &[
    "SyncAllowed",
    "HostInfo",
    "SyncRequest",
    "ReadyForSync",
    "MetadataSyncFinished",
    "AssetManifest",
    "AssetCompleted",
    "SyncFinished",
    "SyncFailed",
];
const KEYS: &[&str] = &[
    "Name",
    "MessageName",
    "MessageType",
    "Message",
    "Command",
    "Type",
    "Params",
    "Parameters",
    "Payload",
    "Contents",
    "Content",
    "Version",
    "__Name",
    "__Params",
    "name",
    "params",
    "message",
    "command",
    "AllowSync",
    "SyncAllowed",
    "AssetManifest",
];
fn names(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(s) if MESSAGE_NAMES.contains(&s.as_str()) => out.push(s.clone()),
        Value::Dictionary(d) => {
            for value in d.values() {
                names(value, out);
            }
        }
        Value::Array(a) => {
            for value in a {
                names(value, out);
            }
        }
        _ => {}
    }
}
pub fn inspect(bytes: &[u8]) -> Vec<Candidate> {
    let mut candidates = Vec::new();
    for little in [true, false] {
        for includes_header in [false, true] {
            let mut candidate = Candidate {
                byte_order: if little {
                    "little_endian"
                } else {
                    "big_endian"
                },
                length_includes_header: includes_header,
                frame_lengths: Vec::new(),
                complete_bytes: 0,
                trailing_bytes: bytes.len(),
                valid_binary_plists: 0,
                known_message_names: Vec::new(),
                recognized_envelope_keys: Vec::new(),
                unrecognized_key_count: 0,
                sync_allowed_envelopes: 0,
                envelope_shapes: Vec::new(),
            };
            let mut offset = 0;
            while bytes.len().saturating_sub(offset) >= 4 && candidate.frame_lengths.len() < 128 {
                let header: [u8; 4] = bytes[offset..offset + 4].try_into().unwrap();
                let length = if little {
                    u32::from_le_bytes(header)
                } else {
                    u32::from_be_bytes(header)
                } as usize;
                let Some(payload_len) = length.checked_sub(if includes_header { 4 } else { 0 })
                else {
                    break;
                };
                if !(8..=MAX_PLIST_BYTES).contains(&payload_len)
                    || payload_len > bytes.len() - offset - 4
                {
                    break;
                }
                let payload = &bytes[offset + 4..offset + 4 + payload_len];
                let Ok(value) = decode_binary(payload) else {
                    break;
                };
                candidate.frame_lengths.push(payload_len);
                candidate.valid_binary_plists += 1;
                names(&value, &mut candidate.known_message_names);
                if let Some(d) = value.as_dictionary() {
                    let command = d.get("Command").and_then(Value::as_string);
                    let known = MESSAGE_NAMES
                        .iter()
                        .copied()
                        .find(|name| Some(*name) == command)
                        .unwrap_or("unrecognized");
                    let params_kind = kind(d.get("Params"));
                    let type_kind = kind(d.get("Type"));
                    if known == "SyncAllowed"
                        && params_kind == "dictionary"
                        && type_kind == "integer"
                    {
                        candidate.sync_allowed_envelopes += 1;
                    }
                    candidate.envelope_shapes.push(EnvelopeShape {
                        command: known,
                        params_kind,
                        type_kind,
                    });
                    for key in d.keys() {
                        if KEYS.contains(&key.as_str()) {
                            candidate.recognized_envelope_keys.push(key.clone());
                        } else {
                            candidate.unrecognized_key_count += 1;
                        }
                    }
                }
                offset += 4 + payload_len;
            }
            if candidate.valid_binary_plists > 0 {
                candidate.complete_bytes = offset;
                candidate.trailing_bytes = bytes.len() - offset;
                candidates.push(candidate);
            }
        }
    }
    candidates
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn concatenated_little_endian_candidates_are_observations() {
        let mut stream = Vec::new();
        for name in ["SyncAllowed", "ReadyForSync"] {
            let payload = aircard_core::encode_binary(&Value::String(name.into())).unwrap();
            stream.extend((payload.len() as u32).to_le_bytes());
            stream.extend(payload);
        }
        let candidates = inspect(&stream);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].valid_binary_plists, 2);
        assert_eq!(candidates[0].trailing_bytes, 0);
        assert_eq!(
            candidates[0].known_message_names,
            vec!["SyncAllowed", "ReadyForSync"]
        );
        stream.extend([0, 0]);
        assert_eq!(inspect(&stream)[0].trailing_bytes, 2);
    }
    #[test]
    fn malformed_lengths_never_allocate_payloads() {
        assert!(inspect(&[255; 32]).is_empty());
        assert!(inspect(&[0; 32]).is_empty());
    }
}
