use super::*;
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    rc::Rc,
};

const LIBRARY: &str = "11111111-2222-3333-4444-555555555555";
enum ReadStep {
    Bytes(Vec<u8>),
    Timeout,
    Delay(Duration),
    Error(io::ErrorKind),
}
#[derive(Default)]
struct Trace {
    sent: Vec<u8>,
    calls: usize,
    closed: bool,
}
struct Mock {
    reads: VecDeque<ReadStep>,
    trace: Rc<RefCell<Trace>>,
    write_limit: usize,
    fail_send: Option<usize>,
    cancel: Option<Rc<Cell<bool>>>,
    timeouts_forever: bool,
}
impl Mock {
    fn new(data: Vec<u8>) -> (Self, Rc<RefCell<Trace>>) {
        let trace = Rc::new(RefCell::new(Trace::default()));
        (
            Self {
                reads: VecDeque::from([ReadStep::Bytes(data)]),
                trace: trace.clone(),
                write_limit: usize::MAX,
                fail_send: None,
                cancel: None,
                timeouts_forever: false,
            },
            trace,
        )
    }
}
impl Drop for Mock {
    fn drop(&mut self) {
        self.trace.borrow_mut().closed = true;
    }
}
impl ReceiveTransport for Mock {
    fn receive(&mut self, buffer: &mut [u8], _: u32) -> io::Result<usize> {
        match self.reads.pop_front() {
            Some(ReadStep::Bytes(data)) => {
                let n = data.len().min(buffer.len());
                buffer[..n].copy_from_slice(&data[..n]);
                if n < data.len() {
                    self.reads.push_front(ReadStep::Bytes(data[n..].to_vec()));
                }
                Ok(n)
            }
            Some(ReadStep::Timeout) => Err(io::ErrorKind::TimedOut.into()),
            Some(ReadStep::Error(kind)) => Err(kind.into()),
            Some(ReadStep::Delay(duration)) => {
                thread::sleep(duration);
                self.receive(buffer, 1)
            }
            None if self.timeouts_forever => {
                thread::sleep(Duration::from_millis(1));
                Err(io::ErrorKind::TimedOut.into())
            }
            None => Ok(0),
        }
    }
}
impl DuplexTransport for Mock {
    fn send(&mut self, buffer: &[u8], _: u32) -> io::Result<usize> {
        let mut trace = self.trace.borrow_mut();
        trace.calls += 1;
        if self.fail_send == Some(trace.calls) {
            return Err(io::ErrorKind::TimedOut.into());
        }
        let n = self.write_limit.min(buffer.len());
        trace.sent.extend(&buffer[..n]);
        if let Some(cancel) = &self.cancel {
            cancel.set(true);
        }
        Ok(n)
    }
}
fn incoming(command: &str, session: u64, params: Option<Dictionary>) -> Vec<u8> {
    let mut d = Dictionary::from_iter([
        ("Command", Value::String(command.into())),
        ("Session", Value::Integer(session.into())),
        ("Type", Value::Integer(0.into())),
        ("Id", Value::Integer(42.into())),
    ]);
    if let Some(params) = params {
        d.insert("Params".into(), Value::Dictionary(params));
    }
    let payload = encode_binary(&Value::Dictionary(d)).unwrap();
    let mut bytes = (payload.len() as u32).to_le_bytes().to_vec();
    bytes.extend(payload);
    bytes
}
fn message(command: &str, session: u64) -> Vec<u8> {
    incoming(command, session, Some(Dictionary::new()))
}
fn run(mock: Mock) -> HandshakeReport {
    HandshakeClient { transport: mock }.ready(LIBRARY, Duration::from_secs(2), &|| false, |_| {})
}
fn sent_messages(bytes: &[u8]) -> Vec<Value> {
    let mut input = bytes;
    let mut messages = Vec::new();
    while !input.is_empty() {
        let len = u32::from_le_bytes(input[..4].try_into().unwrap()) as usize;
        messages.push(decode_binary(&input[4..4 + len]).unwrap());
        input = &input[4 + len..];
    }
    messages
}
#[test]
fn fragmented_io_and_timeouts_preserve_frames_and_exact_handshake() {
    let mut data = Vec::new();
    for command in [
        "Capabilities",
        "InstalledAssets",
        "AssetMetrics",
        "SyncAllowed",
    ] {
        data.extend(message(command, 0));
    }
    data.extend(incoming("Ping", 1, None));
    data.extend(message("ReadyForSync", 1));
    let (mut mock, trace) = Mock::new(Vec::new());
    for byte in data {
        mock.reads.push_back(ReadStep::Bytes(vec![byte]));
        mock.reads.push_back(ReadStep::Timeout);
    }
    mock.reads.pop_front();
    mock.write_limit = 7;
    let report = run(mock);
    assert!(report.ready_for_sync);
    assert_eq!(report.state, State::ReadyForSync);
    assert_eq!(report.frames_received, 6);
    assert_eq!(report.frames_sent, 3);
    assert!(!report.metadata_or_assets_sent);
    let trace = trace.borrow();
    assert!(trace.closed);
    let sent = sent_messages(&trace.sent);
    let host = sent[0].as_dictionary().unwrap();
    assert_eq!(host.len(), 3);
    assert_eq!(host["Command"].as_string(), Some("HostInfo"));
    assert_eq!(host["Session"].as_unsigned_integer(), Some(0));
    let params = host["Params"].as_dictionary().unwrap();
    assert_eq!(params["LocalCloudSupport"].as_boolean(), Some(false));
    let request = sent[1].as_dictionary().unwrap();
    assert_eq!(request.len(), 3);
    assert_eq!(request["Command"].as_string(), Some("RequestingSync"));
    assert_eq!(request["Session"].as_unsigned_integer(), Some(1));
    let params = request["Params"].as_dictionary().unwrap();
    assert_eq!(
        params["Dataclasses"],
        Value::Array(vec![Value::String("Book".into())])
    );
    assert!(
        params["DataclassAnchors"]
            .as_dictionary()
            .unwrap()
            .is_empty()
    );
    assert!(!params.contains_key("Grappa"));
    assert_eq!(
        params["HostInfo"].as_dictionary().unwrap()["LibraryID"].as_string(),
        Some(LIBRARY)
    );
    let pong = sent[2].as_dictionary().unwrap();
    assert_eq!(pong.len(), 2);
    assert_eq!(pong["Command"].as_string(), Some("Pong"));
    assert_eq!(pong["Session"].as_unsigned_integer(), Some(1));
}
#[test]
fn rejection_is_terminal_even_if_ready_follows() {
    for session in [0, 1] {
        let mut data = message("SyncAllowed", 0);
        data.extend(incoming(
            "SyncFailed",
            session,
            Some(Dictionary::from_iter([(
                "ErrorCode",
                Value::Integer(4.into()),
            )])),
        ));
        data.extend(message("ReadyForSync", 1));
        let (mock, trace) = Mock::new(data);
        let report = run(mock);
        assert!(!report.ready_for_sync);
        let failure = report.failure.unwrap();
        assert_eq!(failure.kind, FailureKind::DeviceRejected);
        assert_eq!(failure.stage, State::SyncRequest);
        assert_eq!(failure.device_error_code, Some(4));
        assert_eq!(failure.device_session, Some(session));
        assert_eq!(report.frames_received, 2);
        assert!(trace.borrow().closed);
    }
}
#[test]
fn wrong_session_and_manifest_are_never_ready() {
    for (command, session, kind) in [
        ("ReadyForSync", 0, FailureKind::UnexpectedSession),
        ("AssetManifest", 1, FailureKind::UnexpectedMessage),
        ("SyncFinished", 1, FailureKind::UnexpectedMessage),
    ] {
        let mut data = message("SyncAllowed", 0);
        data.extend(message(command, session));
        let (mock, _) = Mock::new(data);
        let report = run(mock);
        assert!(!report.ready_for_sync);
        assert_eq!(report.failure.unwrap().kind, kind);
    }
}
#[test]
fn unexpected_order_and_protected_device_do_not_send() {
    for (data, kind) in [
        (message("ReadyForSync", 1), FailureKind::UnexpectedMessage),
        (
            incoming(
                "SyncAllowed",
                0,
                Some(Dictionary::from_iter([(
                    "DataProtected",
                    Value::Boolean(true),
                )])),
            ),
            FailureKind::DeviceProtected,
        ),
        (
            incoming("SyncAllowed", 0, None),
            FailureKind::InvalidEnvelope,
        ),
        (
            incoming(
                "SyncAllowed",
                0,
                Some(Dictionary::from_iter([(
                    "DataProtected",
                    Value::String("unknown".into()),
                )])),
            ),
            FailureKind::InvalidEnvelope,
        ),
    ] {
        let (mock, trace) = Mock::new(data);
        assert_eq!(run(mock).failure.unwrap().kind, kind);
        assert!(trace.borrow().sent.is_empty());
        assert!(trace.borrow().closed);
    }
}
#[test]
fn malformed_and_oversized_frames_fail_before_writes() {
    for bytes in [
        0u32.to_le_bytes().to_vec(),
        ((MAX_PLIST_BYTES + 1) as u32).to_le_bytes().to_vec(),
        [8u32.to_le_bytes().as_slice(), b"nonsense"].concat(),
    ] {
        let (mock, trace) = Mock::new(bytes);
        assert_eq!(run(mock).failure.unwrap().kind, FailureKind::InvalidFrame);
        assert!(trace.borrow().sent.is_empty());
    }
}
#[test]
fn envelope_types_are_validated() {
    for field in ["Session", "Type", "Command", "Params"] {
        let frame = message("SyncAllowed", 0);
        let mut v = decode_binary(&frame[4..]).unwrap();
        v.as_dictionary_mut()
            .unwrap()
            .insert(field.into(), Value::Boolean(false));
        assert!(matches!(
            parse(v, frame.len() - 4),
            Err(FailureKind::InvalidEnvelope)
        ));
    }
}
#[test]
fn disconnect_during_header_or_payload_is_terminal() {
    let frame = message("SyncAllowed", 0);
    for n in [0, 2, frame.len() - 1] {
        let (mock, trace) = Mock::new(frame[..n].to_vec());
        assert_eq!(run(mock).failure.unwrap().kind, FailureKind::Disconnected);
        assert!(trace.borrow().closed);
        assert!(trace.borrow().sent.is_empty());
    }
    let (mut mock, _) = Mock::new(Vec::new());
    mock.reads = VecDeque::from([ReadStep::Error(io::ErrorKind::ConnectionReset)]);
    assert_eq!(run(mock).failure.unwrap().kind, FailureKind::Disconnected);
}
#[test]
fn failed_partial_send_is_not_retried() {
    let (mut mock, trace) = Mock::new(message("SyncAllowed", 0));
    mock.write_limit = 3;
    mock.fail_send = Some(2);
    let report = run(mock);
    assert_eq!(report.failure.unwrap().kind, FailureKind::Timeout);
    assert_eq!(report.frames_sent, 0);
    assert_eq!(report.bytes_sent, 3);
    let trace = trace.borrow();
    assert_eq!(trace.calls, 2);
    assert!(trace.closed);
}
#[test]
fn cancellation_before_and_after_host_info_closes_transport() {
    for initially_cancelled in [true, false] {
        let cancel = Rc::new(Cell::new(initially_cancelled));
        let (mut mock, trace) = Mock::new(message("SyncAllowed", 0));
        mock.cancel = Some(cancel.clone());
        let report = HandshakeClient { transport: mock }.ready(
            LIBRARY,
            Duration::from_secs(2),
            &|| cancel.get(),
            |_| {},
        );
        assert_eq!(report.failure.unwrap().kind, FailureKind::Cancelled);
        let trace = trace.borrow();
        assert_eq!(
            sent_messages(&trace.sent).len(),
            usize::from(!initially_cancelled)
        );
        assert!(trace.closed);
    }
}
#[test]
fn total_deadline_limits_repeated_timeouts_and_late_ready() {
    let (mut mock, trace) = Mock::new(Vec::new());
    mock.reads.clear();
    mock.timeouts_forever = true;
    let report = HandshakeClient { transport: mock }.ready(
        LIBRARY,
        Duration::from_millis(5),
        &|| false,
        |_| {},
    );
    assert_eq!(report.failure.unwrap().kind, FailureKind::Timeout);
    assert!(trace.borrow().closed);

    let (mut mock, _) = Mock::new(message("SyncAllowed", 0));
    // The deadline can expire inside a transport call: no late success is accepted.
    mock.reads.push_back(ReadStep::Bytes(
        (message("ReadyForSync", 1).len() as u32 - 4)
            .to_le_bytes()
            .to_vec(),
    ));
    mock.reads
        .push_back(ReadStep::Delay(Duration::from_millis(200)));
    mock.reads
        .push_back(ReadStep::Bytes(message("ReadyForSync", 1)[4..].to_vec()));
    let report = HandshakeClient { transport: mock }.ready(
        LIBRARY,
        Duration::from_millis(350),
        &|| false,
        |_| {},
    );
    assert_eq!(report.failure.unwrap().kind, FailureKind::Timeout);
    assert!(!report.ready_for_sync);
}
#[test]
fn excessive_startup_messages_are_bounded() {
    let data = message("Capabilities", 0).repeat(129);
    let (mock, trace) = Mock::new(data);
    let report = run(mock);
    assert_eq!(report.failure.unwrap().kind, FailureKind::ResourceLimit);
    assert_eq!(report.frames_received, 128);
    assert!(trace.borrow().sent.is_empty());
}
#[test]
fn invalid_host_identity_fails_without_io() {
    let (mock, trace) = Mock::new(message("SyncAllowed", 0));
    let report = HandshakeClient { transport: mock }.ready(
        "arbitrary input",
        Duration::from_secs(2),
        &|| false,
        |_| {},
    );
    assert_eq!(report.failure.unwrap().kind, FailureKind::InvalidInput);
    assert_eq!(report.bytes_received, 0);
    assert!(trace.borrow().sent.is_empty());
}
