#![forbid(unsafe_code)]
//! Native ATC observation and bounded handshake. No metadata or asset completion is sent.
use aircard_core::{MAX_PLIST_BYTES, decode_binary, encode_binary, safe_leaf};
use plist::{Dictionary, Value};
use serde::Serialize;
use std::{
    collections::BTreeSet,
    io::{self, Read, Write},
    time::{Duration, Instant},
};
use thiserror::Error;
pub mod handshake;
pub mod observation;

#[derive(Debug, Error)]
pub enum Error {
    #[error("transport: {0}")]
    Io(#[from] io::Error),
    #[error("plist: {0}")]
    Plist(#[from] aircard_core::Error),
    #[error("invalid or oversized frame length: {0}")]
    FrameLength(usize),
    #[error("unexpected event {event:?} in state {state:?}")]
    Order { state: State, event: Event },
    #[error("manifest rejected: {0}")]
    Manifest(&'static str),
}
/// BE32 candidate codec, ONLY for synthetic fixtures/research. ATC uses LE32 (see handshake).
pub mod research_framing {
    use super::*;
    pub fn read_be32_binary(reader: &mut impl Read) -> Result<Value, Error> {
        let mut header = [0; 4];
        reader.read_exact(&mut header)?;
        let length = u32::from_be_bytes(header) as usize;
        if !(8..=MAX_PLIST_BYTES).contains(&length) {
            return Err(Error::FrameLength(length));
        }
        let mut data = vec![0; length];
        reader.read_exact(&mut data)?;
        Ok(decode_binary(&data)?)
    }
    pub fn write_be32_binary(writer: &mut impl Write, value: &Value) -> Result<(), Error> {
        let data = encode_binary(value)?;
        writer.write_all(&(data.len() as u32).to_be_bytes())?;
        writer.write_all(&data)?;
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum State {
    Connecting,
    SyncAllowed,
    HostInfo,
    SyncRequest,
    ReadyForSync,
    MetadataSyncFinished,
    AssetManifest,
    AssetCompleted,
    Finished,
    Failed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    ReceiveSyncAllowed,
    SendHostInfo,
    SendSyncRequest,
    ReceiveReadyForSync,
    SendMetadataSyncFinished,
    ReceiveAssetManifest,
    SendAssetCompleted,
    ReceiveSyncFinished,
    ReceiveSyncFailed,
    Timeout,
    Cancel,
    Disconnect,
}
#[derive(Debug)]
pub struct StateMachine {
    pub state: State,
}
impl Default for StateMachine {
    fn default() -> Self {
        Self {
            state: State::Connecting,
        }
    }
}
impl StateMachine {
    pub fn advance(&mut self, event: Event) -> Result<State, Error> {
        use Event::*;
        use State::*;
        if matches!(self.state, Finished | Failed) {
            return Err(Error::Order {
                state: self.state,
                event,
            });
        }
        let next = match (self.state, event) {
            (_, ReceiveSyncFailed | Timeout | Cancel | Disconnect) => Failed,
            (Connecting, ReceiveSyncAllowed) => SyncAllowed,
            (SyncAllowed, SendHostInfo) => HostInfo,
            (HostInfo, SendSyncRequest) => SyncRequest,
            (SyncRequest, ReceiveReadyForSync) => ReadyForSync,
            (ReadyForSync, SendMetadataSyncFinished) => MetadataSyncFinished,
            (MetadataSyncFinished, ReceiveAssetManifest) => AssetManifest,
            (AssetManifest, SendAssetCompleted) => AssetCompleted,
            (AssetCompleted, ReceiveSyncFinished) => Finished,
            (state, event) => {
                self.state = Failed;
                return Err(Error::Order { state, event });
            }
        };
        self.state = next;
        Ok(next)
    }
}
/// High-level parameters observed in Windows v1.2.2. This is NOT a wire envelope.
pub fn host_info(library_id: &str) -> Value {
    let mut d = Dictionary::new();
    for (key, value) in [
        ("Type", "iTunes"),
        ("Version", "13.7.0.161"),
        ("MacOSVersion", "Linux"),
        ("SyncHostName", "AirCard-Linux"),
        ("LibraryID", library_id),
    ] {
        d.insert(key.into(), Value::String(value.into()));
    }
    for key in ["SyncedDataclasses", "SyncedAssetTypes"] {
        d.insert(key.into(), Value::Array(vec![Value::String("Book".into())]));
    }
    d.insert("Wakeable".into(), Value::Boolean(false));
    Value::Dictionary(d)
}
/// Pure validation of a decoded AssetManifest parameter, without filesystem policy or writes.
pub fn validate_manifest(value: &Value, expected: &BTreeSet<String>) -> Result<Vec<String>, Error> {
    if expected.is_empty()
        || expected.len() > 128
        || expected.iter().any(|id| safe_leaf(id).is_err())
    {
        return Err(Error::Manifest("invalid expected asset set"));
    }
    let d = value
        .as_dictionary()
        .ok_or(Error::Manifest("expected dictionary"))?;
    if d.len() != 1 {
        return Err(Error::Manifest("only Book dataclass is accepted"));
    }
    let entries = d
        .get("Book")
        .and_then(Value::as_array)
        .ok_or(Error::Manifest("missing Book list"))?;
    if entries.len() > 128 {
        return Err(Error::Manifest("too many entries"));
    }
    let mut ids = BTreeSet::new();
    for entry in entries {
        let d = entry
            .as_dictionary()
            .ok_or(Error::Manifest("entry is not dictionary"))?;
        let id = d
            .get("AssetID")
            .and_then(Value::as_string)
            .ok_or(Error::Manifest("missing AssetID"))?;
        if safe_leaf(id).is_err() || !expected.contains(id) {
            return Err(Error::Manifest("unexpected or unsafe AssetID"));
        }
        if d.get("IsDownload").and_then(Value::as_boolean) != Some(true) {
            return Err(Error::Manifest("asset is not an explicit download"));
        }
        if !ids.insert(id.to_owned()) {
            return Err(Error::Manifest("duplicate AssetID"));
        }
    }
    if &ids != expected {
        return Err(Error::Manifest("expected asset is absent"));
    }
    Ok(ids.into_iter().collect())
}

pub trait ReceiveTransport {
    fn receive(&mut self, buffer: &mut [u8], timeout_ms: u32) -> io::Result<usize>;
}
pub trait AirTrafficClient {
    fn smoke(&mut self, timeout: Duration, cancelled: &dyn Fn() -> bool) -> SmokeReport;
}
pub struct PassiveClient<T> {
    pub transport: T,
}
#[derive(Debug, Serialize)]
pub struct SmokeReport {
    pub state: State,
    pub service_started: bool,
    pub bytes_received: usize,
    pub read_chunks: usize,
    pub first_direction: &'static str,
    pub framing: &'static str,
    pub binary_plist_candidate: bool,
    pub candidate_be32_length: Option<u32>,
    pub candidate_le32_length: Option<u32>,
    pub elapsed_ms: u128,
    pub outcome: &'static str,
    pub protocol_validated: bool,
    pub first_message_validated: bool,
    pub ready_for_sync: bool,
    pub stopped_because: &'static str,
    pub observations: Vec<observation::Candidate>,
}
impl<T: ReceiveTransport> AirTrafficClient for PassiveClient<T> {
    fn smoke(&mut self, timeout: Duration, cancelled: &dyn Fn() -> bool) -> SmokeReport {
        let start = Instant::now();
        let mut bytes = Vec::new();
        let mut chunks = 0;
        let mut outcome = "receive_timeout";
        while start.elapsed() < timeout {
            if cancelled() {
                outcome = "cancelled";
                break;
            }
            let mut buf = [0; 4096];
            let remaining = timeout
                .saturating_sub(start.elapsed())
                .as_millis()
                .clamp(1, 250) as u32;
            match self.transport.receive(&mut buf, remaining) {
                Ok(0) => {
                    outcome = "disconnected";
                    break;
                }
                Ok(n) if n <= buf.len() => {
                    chunks += 1;
                    bytes.extend_from_slice(&buf[..n]);
                    if bytes.len() >= 65536 {
                        outcome = "observation_limit";
                        break;
                    }
                }
                Ok(_) => {
                    outcome = "invalid_transport_length";
                    break;
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                    ) =>
                {
                    continue;
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::UnexpectedEof
                            | io::ErrorKind::ConnectionReset
                            | io::ErrorKind::BrokenPipe
                    ) =>
                {
                    outcome = "disconnected";
                    break;
                }
                Err(_) => {
                    outcome = "transport_error";
                    break;
                }
            }
        }
        let header = bytes.get(..4).map(|h| <[u8; 4]>::try_from(h).unwrap());
        let candidate = decode_binary(&bytes).is_ok()
            || (header.is_some_and(|h| u32::from_be_bytes(h) as usize == bytes.len() - 4)
                && decode_binary(&bytes[4..]).is_ok());
        let observations = observation::inspect(&bytes);
        let first_message_validated = observations.iter().any(|c| {
            c.byte_order == "little_endian"
                && !c.length_includes_header
                && c.trailing_bytes == 0
                && c.sync_allowed_envelopes > 0
        });
        SmokeReport {
            state: if first_message_validated {
                State::SyncAllowed
            } else {
                State::Failed
            },
            service_started: true,
            bytes_received: bytes.len(),
            read_chunks: chunks,
            first_direction: if bytes.is_empty() {
                "unknown"
            } else {
                "device_to_host_observed"
            },
            framing: if first_message_validated {
                "observed_le32_payload_length_binary_plist"
            } else {
                "unconfirmed_no_bytes_sent"
            },
            binary_plist_candidate: candidate || !observations.is_empty(),
            candidate_be32_length: header.map(u32::from_be_bytes),
            candidate_le32_length: header.map(u32::from_le_bytes),
            elapsed_ms: start.elapsed().as_millis(),
            outcome,
            protocol_validated: false,
            observations,
            first_message_validated,
            ready_for_sync: false,
            stopped_because: if first_message_validated {
                "outbound_envelope_unconfirmed_no_messages_sent"
            } else {
                "no_validated_sync_allowed"
            },
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    #[test]
    fn candidate_codec_handles_short_reads_and_writes() {
        struct Fragmented(Cursor<Vec<u8>>);
        impl Read for Fragmented {
            fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
                let n = b.len().min(1);
                self.0.read(&mut b[..n])
            }
        }
        let value = host_info("synthetic-library");
        let mut bytes = Vec::new();
        research_framing::write_be32_binary(&mut bytes, &value).unwrap();
        assert_eq!(
            research_framing::read_be32_binary(&mut Fragmented(Cursor::new(bytes))).unwrap(),
            value
        );
        assert!(research_framing::read_be32_binary(&mut Cursor::new([255; 4])).is_err());
        assert!(research_framing::read_be32_binary(&mut Cursor::new([0, 0, 0, 40, 1])).is_err());
    }
    #[test]
    fn complete_model_and_terminal_idempotency() {
        use Event::*;
        let mut sm = StateMachine::default();
        for event in [
            ReceiveSyncAllowed,
            SendHostInfo,
            SendSyncRequest,
            ReceiveReadyForSync,
            SendMetadataSyncFinished,
            ReceiveAssetManifest,
            SendAssetCompleted,
            ReceiveSyncFinished,
        ] {
            sm.advance(event).unwrap();
        }
        assert_eq!(sm.state, State::Finished);
        assert!(sm.advance(SendAssetCompleted).is_err());
        assert_eq!(sm.state, State::Finished);
    }
    #[test]
    fn wrong_order_timeout_disconnect_and_cancel_fail_closed() {
        let mut sm = StateMachine::default();
        assert!(sm.advance(Event::ReceiveReadyForSync).is_err());
        assert_eq!(sm.state, State::Failed);
        for event in [
            Event::Timeout,
            Event::Disconnect,
            Event::Cancel,
            Event::ReceiveSyncFailed,
        ] {
            assert_eq!(
                StateMachine::default().advance(event).unwrap(),
                State::Failed
            );
        }
    }
    fn manifest(id: &str) -> Value {
        Value::Dictionary(Dictionary::from_iter([(
            "Book".to_owned(),
            Value::Array(vec![Value::Dictionary(Dictionary::from_iter([
                ("AssetID".to_owned(), Value::String(id.into())),
                ("IsDownload".to_owned(), Value::Boolean(true)),
            ]))]),
        )]))
    }
    #[test]
    fn strict_manifest() {
        let expected = BTreeSet::from(["test-book".into()]);
        assert_eq!(
            validate_manifest(&manifest("test-book"), &expected).unwrap(),
            vec!["test-book"]
        );
        for id in ["../escape", "missing", "/absolute", "dir/file"] {
            assert!(validate_manifest(&manifest(id), &expected).is_err());
        }
        assert!(validate_manifest(&manifest("test-book"), &BTreeSet::new()).is_err());
        let mut value = manifest("test-book");
        let entries = value
            .as_dictionary_mut()
            .unwrap()
            .get_mut("Book")
            .unwrap()
            .as_array_mut()
            .unwrap();
        entries.push(entries[0].clone());
        assert!(validate_manifest(&value, &expected).is_err());
    }
    #[test]
    fn passive_smoke_never_promotes_candidate_to_protocol_success() {
        struct Mock {
            data: Cursor<Vec<u8>>,
        }
        impl ReceiveTransport for Mock {
            fn receive(&mut self, b: &mut [u8], _: u32) -> io::Result<usize> {
                let n = b.len().min(3);
                self.data.read(&mut b[..n])
            }
        }
        let mut bytes = Vec::new();
        research_framing::write_be32_binary(&mut bytes, &host_info("fixture")).unwrap();
        let mut client = PassiveClient {
            transport: Mock {
                data: Cursor::new(bytes.clone()),
            },
        };
        let report = client.smoke(Duration::from_secs(1), &|| false);
        assert_eq!(report.bytes_received, bytes.len());
        assert!(report.binary_plist_candidate);
        assert!(!report.protocol_validated);
        assert_eq!(report.outcome, "disconnected");
    }
    #[test]
    fn observed_sync_allowed_requires_root_envelope_and_complete_stream() {
        struct Mock(Cursor<Vec<u8>>);
        impl ReceiveTransport for Mock {
            fn receive(&mut self, b: &mut [u8], _: u32) -> io::Result<usize> {
                self.0.read(b)
            }
        }
        for valid in [true, false] {
            let params = Value::Dictionary(Dictionary::new());
            let value = Value::Dictionary(Dictionary::from_iter([
                (
                    "Command",
                    Value::String(if valid { "SyncAllowed" } else { "Other" }.into()),
                ),
                ("Params", params),
                ("Type", Value::Integer(1.into())),
                ("Nested", Value::String("SyncAllowed".into())),
            ]));
            let payload = encode_binary(&value).unwrap();
            let mut bytes = (payload.len() as u32).to_le_bytes().to_vec();
            bytes.extend(payload);
            let report = PassiveClient {
                transport: Mock(Cursor::new(bytes.clone())),
            }
            .smoke(Duration::from_secs(1), &|| false);
            assert_eq!(report.first_message_validated, valid);
            assert!(!report.ready_for_sync);
            assert!(!report.protocol_validated);
            bytes.push(0);
            let report = PassiveClient {
                transport: Mock(Cursor::new(bytes)),
            }
            .smoke(Duration::from_secs(1), &|| false);
            assert!(!report.first_message_validated);
        }
    }
    #[test]
    fn passive_smoke_timeout_and_cancellation() {
        struct Silent;
        impl ReceiveTransport for Silent {
            fn receive(&mut self, _: &mut [u8], _: u32) -> io::Result<usize> {
                Err(io::ErrorKind::TimedOut.into())
            }
        }
        let mut client = PassiveClient { transport: Silent };
        assert_eq!(
            client.smoke(Duration::from_millis(1), &|| false).outcome,
            "receive_timeout"
        );
        assert_eq!(
            client.smoke(Duration::from_secs(1), &|| true).outcome,
            "cancelled"
        );
    }
}
