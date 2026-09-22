//! Native, handshake-only ATC client. Public protocol provenance: docs/READY-FOR-SYNC.md.
//! Stops at ReadyForSync. It has no metadata/asset completion or authentication-token path.
use crate::{Event, ReceiveTransport, State, StateMachine, host_info};
use aircard_core::{MAX_PLIST_BYTES, decode_binary, encode_binary};
use plist::{Dictionary, Value};
use serde::Serialize;
use std::{
    io, thread,
    time::{Duration, Instant},
};

pub trait DuplexTransport: ReceiveTransport {
    /// A successful partial write consumes exactly the returned byte count.
    /// An error is terminal: delivery may already have partially occurred, so never retry it.
    fn send(&mut self, buffer: &[u8], timeout_ms: u32) -> io::Result<usize>;
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Command {
    Capabilities,
    InstalledAssets,
    AssetMetrics,
    SyncAllowed,
    HostInfo,
    RequestingSync,
    ReadyForSync,
    SyncFailed,
    SyncFinished,
    Ping,
    Pong,
    Unknown,
}
impl Command {
    fn parse(name: &str) -> Self {
        match name {
            "Capabilities" => Self::Capabilities,
            "InstalledAssets" => Self::InstalledAssets,
            "AssetMetrics" => Self::AssetMetrics,
            "SyncAllowed" => Self::SyncAllowed,
            "HostInfo" => Self::HostInfo,
            "RequestingSync" => Self::RequestingSync,
            "ReadyForSync" => Self::ReadyForSync,
            "SyncFailed" => Self::SyncFailed,
            "SyncFinished" => Self::SyncFinished,
            "Ping" => Self::Ping,
            "Pong" => Self::Pong,
            _ => Self::Unknown,
        }
    }
    fn wire(self) -> &'static str {
        match self {
            Self::HostInfo => "HostInfo",
            Self::RequestingSync => "RequestingSync",
            Self::Pong => "Pong",
            _ => unreachable!("send allowlist"),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    InvalidInput,
    Timeout,
    Cancelled,
    Disconnected,
    Transport,
    InvalidFrame,
    InvalidEnvelope,
    UnexpectedMessage,
    UnexpectedSession,
    DeviceRejected,
    DeviceProtected,
    ResourceLimit,
}
#[derive(Debug, Serialize)]
pub struct Failure {
    pub kind: FailureKind,
    pub stage: State,
    pub device_error_code: Option<i64>,
    pub device_session: Option<u64>,
}
#[derive(Debug, Serialize)]
pub struct LogEvent {
    pub event: &'static str,
    pub direction: &'static str,
    pub stage: State,
    pub command: Command,
    pub session: u64,
    pub payload_bytes: usize,
    pub elapsed_ms: u128,
    pub device_error_code: Option<i64>,
    pub data_protected: Option<bool>,
}
#[derive(Debug, Serialize)]
pub struct HandshakeReport {
    pub event: &'static str,
    pub state: State,
    pub ready_for_sync: bool,
    pub bytes_received: usize,
    pub bytes_sent: usize,
    pub frames_received: usize,
    pub frames_sent: usize,
    pub elapsed_ms: u128,
    pub failure: Option<Failure>,
    pub metadata_or_assets_sent: bool,
}
struct Message {
    command: Command,
    session: u64,
    params: Dictionary,
    length: usize,
}
fn parse(value: Value, length: usize) -> Result<Message, FailureKind> {
    let Value::Dictionary(mut d) = value else {
        return Err(FailureKind::InvalidEnvelope);
    };
    let command = Command::parse(
        d.get("Command")
            .and_then(Value::as_string)
            .ok_or(FailureKind::InvalidEnvelope)?,
    );
    let session = d
        .get("Session")
        .and_then(Value::as_unsigned_integer)
        .ok_or(FailureKind::InvalidEnvelope)?;
    // The inspected device sends request-type (0) envelopes. Refuse unobserved types.
    if d.get("Type").and_then(Value::as_unsigned_integer) != Some(0) {
        return Err(FailureKind::InvalidEnvelope);
    }
    let params = match d.remove("Params") {
        Some(Value::Dictionary(p)) => p,
        None if command == Command::Ping => Dictionary::new(),
        _ => return Err(FailureKind::InvalidEnvelope),
    };
    Ok(Message {
        command,
        session,
        params,
        length,
    })
}
/// Checked outbound format established by a public native implementation.
fn outbound(
    command: Command,
    session: u64,
    params: Option<Dictionary>,
) -> Result<Vec<u8>, FailureKind> {
    if !matches!(
        command,
        Command::HostInfo | Command::RequestingSync | Command::Pong
    ) {
        return Err(FailureKind::InvalidInput);
    }
    let mut d = Dictionary::from_iter([
        ("Command", Value::String(command.wire().into())),
        ("Session", Value::Integer(session.into())),
    ]);
    if let Some(params) = params {
        d.insert("Params".into(), Value::Dictionary(params));
    }
    let payload = encode_binary(&Value::Dictionary(d)).map_err(|_| FailureKind::InvalidFrame)?;
    let mut frame = (payload.len() as u32).to_le_bytes().to_vec();
    frame.extend(payload);
    Ok(frame)
}
fn io_failure(error: io::Error) -> FailureKind {
    match error.kind() {
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => FailureKind::Timeout,
        io::ErrorKind::UnexpectedEof
        | io::ErrorKind::ConnectionReset
        | io::ErrorKind::ConnectionAborted
        | io::ErrorKind::BrokenPipe => FailureKind::Disconnected,
        _ => FailureKind::Transport,
    }
}
/// Owns the connection and always drops it on return, including timeout/rejection/cancellation.
pub struct HandshakeClient<T> {
    pub transport: T,
}
impl<T: DuplexTransport> HandshakeClient<T> {
    pub fn ready(
        self,
        library_id: &str,
        timeout: Duration,
        cancelled: &dyn Fn() -> bool,
        mut log: impl FnMut(&LogEvent),
    ) -> HandshakeReport {
        let start = Instant::now();
        let mut run = Run {
            transport: self.transport,
            start,
            timeout,
            cancelled,
            log: &mut log,
            state: StateMachine::default(),
            report: HandshakeReport {
                event: "atc_ready_complete",
                state: State::Connecting,
                ready_for_sync: false,
                bytes_received: 0,
                bytes_sent: 0,
                frames_received: 0,
                frames_sent: 0,
                elapsed_ms: 0,
                failure: None,
                metadata_or_assets_sent: false,
            },
        };
        let result = run.handshake(library_id);
        if let Err(failure) = result {
            run.report.failure = Some(failure);
            run.state.state = State::Failed;
        }
        run.report.state = run.state.state;
        run.report.ready_for_sync = run.state.state == State::ReadyForSync;
        run.report.elapsed_ms = start.elapsed().as_millis();
        run.report
    }
}
struct Run<'a, T, F> {
    transport: T,
    start: Instant,
    timeout: Duration,
    cancelled: &'a dyn Fn() -> bool,
    log: &'a mut F,
    state: StateMachine,
    report: HandshakeReport,
}
impl<T: DuplexTransport, F: FnMut(&LogEvent)> Run<'_, T, F> {
    fn failure(&self, kind: FailureKind) -> Failure {
        Failure {
            kind,
            stage: self.state.state,
            device_error_code: None,
            device_session: None,
        }
    }
    fn budget(&self) -> Result<u32, Failure> {
        if (self.cancelled)() {
            return Err(self.failure(FailureKind::Cancelled));
        }
        let left = self.timeout.saturating_sub(self.start.elapsed());
        if left.is_zero() {
            return Err(self.failure(FailureKind::Timeout));
        }
        Ok(left.as_millis().clamp(1, 250) as u32)
    }
    fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), Failure> {
        let mut offset = 0;
        while offset < buf.len() {
            let timeout = self.budget()?;
            match self.transport.receive(&mut buf[offset..], timeout) {
                Ok(0) => return Err(self.failure(FailureKind::Disconnected)),
                Ok(n) if n <= buf.len() - offset => {
                    offset += n;
                    self.report.bytes_received += n;
                }
                Ok(_) => return Err(self.failure(FailureKind::Transport)),
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::TimedOut
                            | io::ErrorKind::WouldBlock
                            | io::ErrorKind::Interrupted
                    ) =>
                {
                    continue;
                }
                Err(e) => return Err(self.failure(io_failure(e))),
            }
        }
        Ok(())
    }
    fn receive(&mut self) -> Result<Message, Failure> {
        if self.report.frames_received >= 128 {
            return Err(self.failure(FailureKind::ResourceLimit));
        }
        let mut header = [0; 4];
        self.read_exact(&mut header)?;
        let length = u32::from_le_bytes(header) as usize;
        if !(8..=MAX_PLIST_BYTES).contains(&length) {
            return Err(self.failure(FailureKind::InvalidFrame));
        }
        if self.report.bytes_received + length > 4 * MAX_PLIST_BYTES {
            return Err(self.failure(FailureKind::ResourceLimit));
        }
        let mut payload = vec![0; length];
        self.read_exact(&mut payload)?;
        let value = decode_binary(&payload).map_err(|_| self.failure(FailureKind::InvalidFrame))?;
        let message = parse(value, length).map_err(|kind| self.failure(kind))?;
        self.report.frames_received += 1;
        self.emit("receive", &message);
        Ok(message)
    }
    fn emit(&mut self, direction: &'static str, message: &Message) {
        (self.log)(&LogEvent {
            event: "atc_message",
            direction,
            stage: self.state.state,
            command: message.command,
            session: message.session,
            payload_bytes: message.length,
            elapsed_ms: self.start.elapsed().as_millis(),
            device_error_code: message
                .params
                .get("ErrorCode")
                .and_then(Value::as_signed_integer),
            data_protected: message
                .params
                .get("DataProtected")
                .and_then(Value::as_boolean),
        });
    }
    fn send(
        &mut self,
        command: Command,
        session: u64,
        params: Option<Dictionary>,
    ) -> Result<(), Failure> {
        let frame = outbound(command, session, params).map_err(|kind| self.failure(kind))?;
        let mut offset = 0;
        while offset < frame.len() {
            let timeout = self.budget()?;
            // Never retransmit after an error: the native layer may already have sent data.
            let n = self
                .transport
                .send(&frame[offset..], timeout)
                .map_err(|e| self.failure(io_failure(e)))?;
            if n == 0 {
                return Err(self.failure(FailureKind::Disconnected));
            }
            if n > frame.len() - offset {
                return Err(self.failure(FailureKind::Transport));
            }
            offset += n;
            self.report.bytes_sent += n;
        }
        self.report.frames_sent += 1;
        self.emit(
            "send",
            &Message {
                command,
                session,
                params: Dictionary::new(),
                length: frame.len() - 4,
            },
        );
        Ok(())
    }
    fn advance(&mut self, event: Event) -> Result<(), Failure> {
        let stage = self.state.state;
        self.state.advance(event).map_err(|_| Failure {
            kind: FailureKind::UnexpectedMessage,
            stage,
            device_error_code: None,
            device_session: None,
        })?;
        Ok(())
    }
    fn wait_for(&mut self, wanted: Command, session: u64) -> Result<Message, Failure> {
        loop {
            let message = self.receive()?;
            self.budget()?;
            if message.command == Command::Ping {
                if message.session > 1 {
                    return Err(self.failure(FailureKind::UnexpectedSession));
                }
                self.send(Command::Pong, 1, None)?;
                continue;
            }
            if message.command == Command::SyncFailed {
                // Fail closed for every session; never suppress a rejection as supposedly stale.
                return Err(Failure {
                    kind: FailureKind::DeviceRejected,
                    stage: self.state.state,
                    device_error_code: message
                        .params
                        .get("ErrorCode")
                        .and_then(Value::as_signed_integer),
                    device_session: Some(message.session),
                });
            }
            if message.command == wanted {
                if message.session != session {
                    return Err(self.failure(FailureKind::UnexpectedSession));
                }
                return Ok(message);
            }
            if message.session == 0
                && matches!(
                    message.command,
                    Command::Capabilities | Command::InstalledAssets | Command::AssetMetrics
                )
            {
                continue;
            }
            return Err(self.failure(FailureKind::UnexpectedMessage));
        }
    }
    fn handshake(&mut self, library_id: &str) -> Result<(), Failure> {
        // Avoid accepting arbitrary data as host identity. The CLI supplies a fresh UUID.
        if library_id.len() != 36
            || !library_id.bytes().enumerate().all(|(i, b)| {
                if [8, 13, 18, 23].contains(&i) {
                    b == b'-'
                } else {
                    b.is_ascii_hexdigit()
                }
            })
        {
            return Err(self.failure(FailureKind::InvalidInput));
        }
        let allowed = self.wait_for(Command::SyncAllowed, 0)?;
        if allowed
            .params
            .get("DataProtected")
            .is_some_and(|v| v.as_boolean().is_none())
        {
            return Err(self.failure(FailureKind::InvalidEnvelope));
        }
        if allowed
            .params
            .get("DataProtected")
            .and_then(Value::as_boolean)
            == Some(true)
        {
            return Err(self.failure(FailureKind::DeviceProtected));
        }
        self.advance(Event::ReceiveSyncAllowed)?;
        let host = host_info(library_id);
        self.send(
            Command::HostInfo,
            0,
            Some(Dictionary::from_iter([
                ("HostInfo", host.clone()),
                ("LocalCloudSupport", Value::Boolean(false)),
            ])),
        )?;
        self.advance(Event::SendHostInfo)?;
        // Public reference pacing, bounded by the same deadline and cancellation.
        let pause = Instant::now();
        while pause.elapsed() < Duration::from_millis(200) {
            self.budget()?;
            thread::sleep(Duration::from_millis(5));
        }
        self.send(
            Command::RequestingSync,
            1,
            Some(Dictionary::from_iter([
                (
                    "Dataclasses",
                    Value::Array(vec![Value::String("Book".into())]),
                ),
                ("DataclassAnchors", Value::Dictionary(Dictionary::new())),
                ("HostInfo", host),
            ])),
        )?;
        self.advance(Event::SendSyncRequest)?;
        self.wait_for(Command::ReadyForSync, 1)?;
        self.advance(Event::ReceiveReadyForSync)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
