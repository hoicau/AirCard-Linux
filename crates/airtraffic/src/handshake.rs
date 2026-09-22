//! Native bounded ATC handshake and single-asset client. Public protocol provenance: docs/READY-FOR-SYNC.md.
//! Explicit caller-supplied Grappa data and manifest contents are never logged.
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
    FinishedSyncingMetadata,
    AssetManifest,
    FileComplete,
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
            "FinishedSyncingMetadata" => Self::FinishedSyncingMetadata,
            "AssetManifest" => Self::AssetManifest,
            "FileComplete" => Self::FileComplete,
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
            Self::FinishedSyncingMetadata => "FinishedSyncingMetadata",
            Self::FileComplete => "FileComplete",
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
    UnsupportedGrappa,
    ManifestRejected,
    PreconditionChanged,
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
    pub grappa_support: Option<GrappaSupport>,
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
    pub manifest_validated: bool,
    pub asset_completion_sent: bool,
    pub last_envelope: Option<EnvelopeShape>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct GrappaSupport {
    pub version: u64,
    pub device_type: u64,
    pub protocol_version: u64,
}
impl GrappaSupport {
    fn from_params(params: &Dictionary) -> Option<Self> {
        let d = params.get("GrappaSupportInfo")?.as_dictionary()?;
        Some(Self {
            version: d.get("version")?.as_unsigned_integer()?,
            device_type: d.get("deviceType")?.as_unsigned_integer()?,
            protocol_version: d.get("protocolVersion")?.as_unsigned_integer()?,
        })
    }
}
#[derive(Debug, Serialize)]
pub struct EnvelopeShape {
    pub command: Command,
    pub session: Option<u64>,
    pub message_type: Option<u64>,
    pub params_present: bool,
    pub params_dictionary: bool,
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
        None if matches!(command, Command::Ping | Command::SyncFinished) => Dictionary::new(),
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
        Command::HostInfo
            | Command::RequestingSync
            | Command::Pong
            | Command::FinishedSyncingMetadata
            | Command::FileComplete
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
pub struct SyncOptions<'a> {
    pub library_id: &'a str,
    pub grappa: Option<&'a [u8]>,
    pub timeout: Duration,
    pub cancelled: &'a dyn Fn() -> bool,
}
pub struct SyncAsset {
    pub asset_id: String,
    pub asset_path: String,
    pub retained_ids: std::collections::BTreeSet<String>,
}
impl SyncAsset {
    pub fn validate(&self) -> Result<(), FailureKind> {
        aircard_core::safe_leaf(&self.asset_id).map_err(|_| FailureKind::InvalidInput)?;
        aircard_core::safe_relative_path(&self.asset_path)
            .map_err(|_| FailureKind::InvalidInput)?;
        if self.retained_ids.len() > 127
            || self.retained_ids.contains(&self.asset_id)
            || self
                .retained_ids
                .iter()
                .any(|id| aircard_core::safe_leaf(id).is_err())
        {
            return Err(FailureKind::InvalidInput);
        }
        Ok(())
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
        log: impl FnMut(&LogEvent),
    ) -> HandshakeReport {
        self.ready_authenticated(library_id, None, timeout, cancelled, log)
    }
    /// Explicit authorization data; the caller controls its provenance and the apply gate.
    /// Only the publicly documented 84-byte / (1, 0, 1) variant is supported.
    pub fn ready_authenticated(
        self,
        library_id: &str,
        grappa: Option<&[u8]>,
        timeout: Duration,
        cancelled: &dyn Fn() -> bool,
        log: impl FnMut(&LogEvent),
    ) -> HandshakeReport {
        self.execute(
            SyncOptions {
                library_id,
                grappa,
                timeout,
                cancelled,
            },
            None,
            &mut || true,
            log,
        )
    }
    /// Caller must snapshot and validate the staged source before choosing this applied path.
    pub fn synchronize(
        self,
        options: SyncOptions<'_>,
        asset: &SyncAsset,
        log: impl FnMut(&LogEvent),
    ) -> HandshakeReport {
        self.synchronize_checked(options, asset, &mut || true, log)
    }
    pub fn synchronize_checked(
        self,
        options: SyncOptions<'_>,
        asset: &SyncAsset,
        guard: &mut dyn FnMut() -> bool,
        log: impl FnMut(&LogEvent),
    ) -> HandshakeReport {
        self.execute(options, Some(asset), guard, log)
    }
    fn execute(
        self,
        options: SyncOptions<'_>,
        asset: Option<&SyncAsset>,
        guard: &mut dyn FnMut() -> bool,
        mut log: impl FnMut(&LogEvent),
    ) -> HandshakeReport {
        let SyncOptions {
            library_id,
            grappa,
            timeout,
            cancelled,
        } = options;
        let start = Instant::now();
        let mut run = Run {
            transport: self.transport,
            start,
            timeout,
            cancelled,
            log: &mut log,
            state: StateMachine::default(),
            grappa,
            grappa_support: None,
            report: HandshakeReport {
                event: if asset.is_some() {
                    "atc_sync_complete"
                } else {
                    "atc_ready_complete"
                },
                state: State::Connecting,
                ready_for_sync: false,
                bytes_received: 0,
                bytes_sent: 0,
                frames_received: 0,
                frames_sent: 0,
                elapsed_ms: 0,
                failure: None,
                metadata_or_assets_sent: false,
                manifest_validated: false,
                asset_completion_sent: false,
                last_envelope: None,
            },
        };
        let result = (|| {
            if let Some(asset) = asset {
                asset.validate().map_err(|kind| run.failure(kind))?;
            }
            run.handshake(library_id)?;
            if let Some(asset) = asset {
                run.sync_asset(asset, guard)?;
            }
            Ok(())
        })();
        if let Err(failure) = result {
            run.report.failure = Some(failure);
            run.state.state = State::Failed;
        }
        run.report.state = run.state.state;

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
    grappa: Option<&'a [u8]>,
    grappa_support: Option<GrappaSupport>,
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
        self.report.last_envelope = value.as_dictionary().map(|d| EnvelopeShape {
            command: d
                .get("Command")
                .and_then(Value::as_string)
                .map_or(Command::Unknown, Command::parse),
            session: d.get("Session").and_then(Value::as_unsigned_integer),
            message_type: d.get("Type").and_then(Value::as_unsigned_integer),
            params_present: d.contains_key("Params"),
            params_dictionary: d.get("Params").and_then(Value::as_dictionary).is_some(),
        });
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
            grappa_support: GrappaSupport::from_params(&message.params),
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
            if message.command == Command::Capabilities && message.session <= 1 {
                self.grappa_support = GrappaSupport::from_params(&message.params);
            }
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
                // iOS retains the previous session identifier across connections.
                // Startup notifications have been observed on both sessions 0 and 1.
                let startup = self.state.state == State::Connecting
                    && wanted == Command::SyncAllowed
                    && message.session <= 1;
                if message.session != session && !startup {
                    return Err(self.failure(FailureKind::UnexpectedSession));
                }
                return Ok(message);
            }
            // Observed after authenticated RequestingSync on iOS 27.0: an updated
            // permission notification on the new sync session precedes readiness.
            // It never substitutes for the explicit ReadyForSync response.
            if self.state.state != State::Connecting
                && message.command == Command::SyncAllowed
                && message.session == 1
            {
                match message
                    .params
                    .get("DataProtected")
                    .and_then(Value::as_boolean)
                {
                    Some(false) => continue,
                    Some(true) => return Err(self.failure(FailureKind::DeviceProtected)),
                    None => return Err(self.failure(FailureKind::InvalidEnvelope)),
                }
            }
            if message.session <= 1
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
        if self.grappa.is_some_and(|token| token.len() != 84) {
            return Err(self.failure(FailureKind::InvalidInput));
        }
        let allowed = self.wait_for(Command::SyncAllowed, 0)?;
        if self.grappa.is_some()
            && self.grappa_support
                != Some(GrappaSupport {
                    version: 1,
                    device_type: 0,
                    protocol_version: 1,
                })
        {
            return Err(self.failure(FailureKind::UnsupportedGrappa));
        }
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
        let mut host = host_info(library_id);
        if let Some(token) = self.grappa {
            host.as_dictionary_mut()
                .expect("host model")
                .insert("Grappa".into(), Value::Data(token.to_vec()));
        }
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
        let mut params = Dictionary::from_iter([
            (
                "Dataclasses",
                Value::Array(vec![Value::String("Book".into())]),
            ),
            ("DataclassAnchors", Value::Dictionary(Dictionary::new())),
            ("HostInfo", host),
        ]);
        if let Some(token) = self.grappa {
            params.insert("Grappa".into(), Value::Data(token.to_vec()));
        }
        self.send(Command::RequestingSync, 1, Some(params))?;
        self.advance(Event::SendSyncRequest)?;
        self.wait_for(Command::ReadyForSync, 1)?;
        self.advance(Event::ReceiveReadyForSync)?;
        self.report.ready_for_sync = true;
        Ok(())
    }
    fn sync_asset(
        &mut self,
        asset: &SyncAsset,
        guard: &mut dyn FnMut() -> bool,
    ) -> Result<(), Failure> {
        self.report.metadata_or_assets_sent = true;
        self.send(
            Command::FinishedSyncingMetadata,
            1,
            Some(Dictionary::from_iter([
                (
                    "SyncTypes",
                    Value::Dictionary(Dictionary::from_iter([("Book", Value::Integer(1.into()))])),
                ),
                ("DataclassAnchors", Value::Dictionary(Dictionary::new())),
            ])),
        )?;
        self.advance(Event::SendMetadataSyncFinished)?;
        let message = self.wait_for(Command::AssetManifest, 1)?;
        let manifest = message
            .params
            .get("AssetManifest")
            .ok_or_else(|| self.failure(FailureKind::ManifestRejected))?;
        validate_selected_manifest(manifest, asset)
            .map_err(|_| self.failure(FailureKind::ManifestRejected))?;
        self.report.manifest_validated = true;
        self.advance(Event::ReceiveAssetManifest)?;
        if !guard() {
            return Err(self.failure(FailureKind::PreconditionChanged));
        }
        self.send(
            Command::FileComplete,
            1,
            Some(Dictionary::from_iter([
                ("AssetID", Value::String(asset.asset_id.clone())),
                ("Dataclass", Value::String("Book".into())),
                ("AssetPath", Value::String(asset.asset_path.clone())),
            ])),
        )?;
        self.report.asset_completion_sent = true;
        self.advance(Event::SendAssetCompleted)?;
        self.wait_for(Command::SyncFinished, 1)?;
        self.advance(Event::ReceiveSyncFinished)?;
        Ok(())
    }
}

fn validate_selected_manifest(manifest: &Value, asset: &SyncAsset) -> Result<(), ()> {
    let root = manifest.as_dictionary().ok_or(())?;
    if root.len() != 1 {
        return Err(());
    }
    let books = root.get("Book").and_then(Value::as_array).ok_or(())?;
    if books.len() > 128 {
        return Err(());
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut selected = false;
    for item in books {
        let d = item.as_dictionary().ok_or(())?;
        let id = d.get("AssetID").and_then(Value::as_string).ok_or(())?;
        let download = d.get("IsDownload").and_then(Value::as_boolean).ok_or(())?;
        if !seen.insert(id) {
            return Err(());
        }
        if id == asset.asset_id {
            if !download {
                return Err(());
            }
            selected = true;
        } else if !asset.retained_ids.contains(id) || download {
            return Err(());
        }
    }
    if selected { Ok(()) } else { Err(()) }
}

#[cfg(test)]
mod tests;
