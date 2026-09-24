#![forbid(unsafe_code)]
mod customization;
mod wallet;
use cli::local;
mod resources;
use airtraffic::ReceiveTransport;
use clap::{Parser, Subcommand, ValueEnum};
use device::{
    AfcAccess, DeviceProvider, DuplexServiceTransport, LinuxDeviceProvider, ServiceTransport,
    Transport,
};
use serde_json::json;
use std::{
    io::{self, Write},
    process::{Command as Process, ExitCode},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Route {
    Usb,
    Wifi,
}
impl From<Route> for Transport {
    fn from(v: Route) -> Self {
        match v {
            Route::Usb => Self::Usb,
            Route::Wifi => Self::Wifi,
        }
    }
}
#[derive(Parser, Debug)]
#[command(
    name = "aircard",
    version,
    about = "Wallet card artwork, private backups and recovery. Device writes require --apply."
)]
struct Args {
    #[arg(long, global = true)]
    udid: Option<String>,
    #[arg(long, value_enum, global = true)]
    transport: Option<Route>,
    /// Per-service deadline (1..300 seconds); the transaction watchdog allows bounded recovery.
    #[arg(long, default_value_t = 15, value_parser = clap::value_parser!(u64).range(1..=300), global = true)]
    timeout: u64,
    #[command(subcommand)]
    command: Command,
}
#[derive(Debug, Subcommand)]
enum Command {
    /// Privately cache the built-in sync token. Works offline; no device required.
    SetupToken {
        /// New token file; defaults to the per-user AirCard data directory.
        #[arg(long)]
        output: Option<std::path::PathBuf>,
    },
    /// Offline center crop, PNG preview and Wallet resource ZIP. No output means dry-run.
    PrepareCard {
        input: std::path::PathBuf,
        #[arg(long)]
        output: Option<std::path::PathBuf>,
        #[arg(long)]
        preview: Option<std::path::PathBuf>,
    },
    /// Controlled Wallet artwork test with private originals, cache invalidation and automatic restore.
    CardTest {
        #[arg(long)]
        card_hash: String,
        #[arg(long)]
        journal: std::path::PathBuf,
        #[arg(long)]
        grappa_token: std::path::PathBuf,
        #[arg(long,default_value_t=180,value_parser=clap::value_parser!(u64).range(0..=300))]
        hold_seconds: u64,
        #[arg(long)]
        apply: bool,
    },
    /// Apply an image to the selected Wallet card and save a private restore backup.
    CardApply {
        input: std::path::PathBuf,
        /// Fail if the prepared PNG differs from the preview approved in the GUI.
        #[arg(long)]
        expected_artwork_sha256: Option<String>,
        #[arg(long)]
        card_hash: String,
        #[arg(long)]
        backup: std::path::PathBuf,
        #[arg(long)]
        journal: std::path::PathBuf,
        #[arg(long)]
        grappa_token: std::path::PathBuf,
        #[arg(long)]
        apply: bool,
    },
    /// Restore a card backup on its original paired device.
    CardRestore {
        input: std::path::PathBuf,
        #[arg(long)]
        journal: std::path::PathBuf,
        #[arg(long)]
        grappa_token: std::path::PathBuf,
        #[arg(long)]
        apply: bool,
    },
    /// Recover an interrupted Wallet operation using its private journal directory.
    CardRecover {
        journal: std::path::PathBuf,
        #[arg(long)]
        grappa_token: std::path::PathBuf,
        #[arg(long)]
        apply: bool,
    },
    /// Enumerate routes and verify existing host pairing. Identifiers redacted by default.
    Devices {
        #[arg(long)]
        show_identifiers: bool,
    },
    /// Check existing pairing, iOS version and AFC access without writing.
    Probe,
    /// Detect Wallet identifiers in live logs. Hashes are suppressed unless explicitly requested.
    Scan {
        #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u64).range(1..=3600))]
        duration: u64,
        #[arg(long)]
        show_hashes: bool,
        /// Stop after receiving candidate card identifiers (used by automatic GUI discovery).
        #[arg(long)]
        until_match: bool,
    },
}
fn emit(value: &impl serde::Serialize) {
    println!(
        "{}",
        serde_json::to_string(value).expect("serializable event")
    );
    let _ = io::stdout().flush();
}
fn error(e: &device::Error) {
    emit(&json!({"event":"error", "error":e}));
}
struct Bridge<T>(T);
impl<T: ServiceTransport> ReceiveTransport for Bridge<T> {
    fn receive(&mut self, b: &mut [u8], t: u32) -> io::Result<usize> {
        self.0.receive(b, t)
    }
}
impl<T: DuplexServiceTransport> airtraffic::handshake::DuplexTransport for Bridge<T> {
    fn send(&mut self, bytes: &[u8], timeout_ms: u32) -> io::Result<usize> {
        self.0.send(bytes, timeout_ms)
    }
}
fn watchdog(args: &Args, cancel: &AtomicBool) -> u8 {
    let mut child = match Process::new(std::env::current_exe().expect("executable path"))
        .args(std::env::args_os().skip(1))
        .env("AIRCARD_INTERNAL_WORKER", "1")
        .spawn()
    {
        Ok(child) => child,
        Err(_) => {
            emit(&json!({"event":"error","kind":"worker_start_failed"}));
            return 1;
        }
    };
    let seconds = match &args.command {
        Command::SetupToken { .. } => 35,
        Command::Scan { duration, .. } => *duration + args.timeout * 4 + 10,
        Command::Probe => args.timeout * 4 + 10,
        Command::CardTest { hold_seconds, .. } => args.timeout * 30 + hold_seconds + 90,
        Command::CardApply { .. } | Command::CardRestore { .. } | Command::CardRecover { .. } => {
            args.timeout * 30 + 90
        }
        _ => args.timeout + 5,
    };
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.code().unwrap_or(1) as u8,
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return 1;
            }
            _ => {}
        }
        if cancel.load(Ordering::Relaxed) || start.elapsed() > Duration::from_secs(seconds) {
            // Let the child observe cancellation and finish any scratch cleanup first.
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(child.id() as i32),
                nix::sys::signal::Signal::SIGINT,
            );
            let grace = Instant::now();
            let cleanup_grace = if matches!(
                args.command,
                Command::CardTest { .. }
                    | Command::CardApply { .. }
                    | Command::CardRestore { .. }
                    | Command::CardRecover { .. }
            ) {
                90
            } else {
                2
            };
            while grace.elapsed() < Duration::from_secs(cleanup_grace) {
                if matches!(child.try_wait(), Ok(Some(_))) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(40));
            }
            if !matches!(child.try_wait(), Ok(Some(_))) {
                let _ = child.kill();
            }
            let _ = child.wait();
            emit(
                &json!({"event":"error","kind":if cancel.load(Ordering::Relaxed){"cancelled"}else{"native_call_deadline"},"stage":"worker","hint":"Keep the private transaction directory. Use card-recover with the original device before retrying."}),
            );
            return if cancel.load(Ordering::Relaxed) {
                130
            } else {
                124
            };
        }
        std::thread::sleep(Duration::from_millis(40));
    }
}
fn run(args: &Args, cancel: &AtomicBool) -> device::Result<u8> {
    if let Command::SetupToken { output } = &args.command {
        return Ok(match cli::sync_token::setup(output.as_deref()) {
            Ok(path) => {
                emit(&json!({"event":"token_ready","path":path}));
                0
            }
            Err(message) => {
                emit(&json!({"event":"error","hint":message}));
                1
            }
        });
    }
    if let Some(code) = resources::run(&args.command)? {
        return Ok(code);
    }
    if let Some(code) = wallet::offline(&args.command)? {
        return Ok(code);
    }
    let provider = LinuxDeviceProvider;
    emit(&json!({"event":"stage","stage":"enumerate"}));
    let devices = provider.list_devices()?;
    if let Command::Devices { show_identifiers } = &args.command {
        let mut count = 0;
        for (index, d) in devices.iter().enumerate() {
            if args.udid.as_deref().is_some_and(|id| id != d.udid)
                || args
                    .transport
                    .is_some_and(|t| Transport::from(t) != d.transport)
            {
                continue;
            }
            count += 1;
            let info = provider.pairing(d);
            emit(
                &json!({"event":"device", "index":index,"udid":if *show_identifiers { &d.udid } else { "[redacted]" },"transport":d.transport,"info":info.as_ref().ok(),"error":info.as_ref().err()}),
            );
        }
        emit(&json!({"event":"devices_complete","count":count}));
        return Ok(0);
    }
    let selected = device::select(
        &devices,
        args.udid.as_deref(),
        args.transport.map(Into::into),
    )?;
    emit(&json!({"event":"stage","stage":"existing_pair_session","transport":selected.transport}));
    let read_only = matches!(args.command, Command::Scan { .. } | Command::Probe);
    let info = if read_only {
        read_connection_retry(|| provider.pairing(&selected), cancel)?
    } else {
        provider.pairing(&selected)?
    };
    emit(&json!({"event":"paired","info":info}));
    match &args.command {
        Command::Devices { .. } | Command::PrepareCard { .. } | Command::SetupToken { .. } => {
            unreachable!()
        }
        Command::CardTest { .. }
        | Command::CardApply { .. }
        | Command::CardRestore { .. }
        | Command::CardRecover { .. } => {
            return wallet::run(&args.command, &provider, &selected, args.timeout, cancel);
        }
        Command::Probe => {
            emit(&json!({"event":"stage","stage":"start_afc"}));
            let mut afc = read_connection_retry(|| provider.afc(&selected), cancel)?;
            let snapshot = aircard_core::DiagnosticSnapshot {
                schema_version: 1,
                kind: "diagnostic_only_not_a_backup".into(),
                ios_version: info.ios_version,
                transport: info.transport.as_str().into(),
                pairing: info.pairing,
                afc_root_entries: afc.list(".")?.len(),
                afc_tls: afc.tls(),
            };
            emit(&json!({"event":"probe_complete","diagnostics":snapshot}));
        }
        Command::Scan {
            duration,
            show_hashes,
            until_match,
        } => capture(
            &provider,
            &selected,
            *duration,
            *show_hashes,
            *until_match,
            cancel,
        )?,
    }
    Ok(0)
}
/// Only used to open read-only sessions. Transaction setup and writes never use this retry.
fn read_connection_retry<T>(
    mut connect: impl FnMut() -> device::Result<T>,
    cancel: &AtomicBool,
) -> device::Result<T> {
    if cancel.load(Ordering::Relaxed) {
        return Err(device::Error::new(
            device::ErrorKind::Cancelled,
            "read_connection",
        ));
    }
    match connect() {
        Err(e)
            if matches!(
                e.kind,
                device::ErrorKind::Timeout | device::ErrorKind::Disconnected
            ) && !cancel.load(Ordering::Relaxed) =>
        {
            emit(
                &json!({"event":"stage","stage":"retry_read_connection","attempt":2,"max_attempts":2,"error":e}),
            );
            connect()
        }
        result => result,
    }
}
fn capture(
    provider: &impl DeviceProvider,
    selected: &device::Device,
    duration: u64,
    show_hashes: bool,
    until_match: bool,
    cancel: &AtomicBool,
) -> device::Result<()> {
    emit(&json!({"event":"stage","stage":"start_syslog"}));
    let mut service = read_connection_retry(|| provider.syslog(selected), cancel)?;
    emit(
        &json!({"event":"service_started","service":"com.apple.syslog_relay","tls":service.tls()}),
    );
    let start = Instant::now();
    let mut lines = aircard_core::LogLines::default();
    let mut bytes = 0;
    let mut count = 0;
    let mut matches = 0;
    let mut last = Instant::now();
    let mut seen = std::collections::BTreeSet::new();
    let mut first_match = None;
    while start.elapsed() < Duration::from_secs(duration) && !cancel.load(Ordering::Relaxed) {
        let mut buf = [0; 8192];
        match service.receive(&mut buf, 250) {
            Ok(0) => {
                return Err(device::Error::new(
                    device::ErrorKind::Disconnected,
                    "syslog_eof",
                ));
            }
            Ok(n) => {
                bytes += n;
                for line in lines.push(&buf[..n]) {
                    count += 1;
                    for hash in aircard_core::scanner::extract_card_hashes_from_line(&line) {
                        matches += 1;
                        if seen.len() < 1024 && seen.insert(hash.clone()) {
                            emit(
                                &json!({"event":"card_match","hash":if show_hashes{hash.as_str()}else{"[redacted]"}}),
                            );
                        }
                    }
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::TimedOut
                        | io::ErrorKind::WouldBlock
                        | io::ErrorKind::Interrupted
                ) => {}
            Err(_) => {
                return Err(device::Error::new(
                    device::ErrorKind::Disconnected,
                    "syslog_receive",
                ));
            }
        }
        if until_match && !seen.is_empty() {
            // Collect the rest of the activity burst, including fragmented lines, so
            // nearby identifiers are offered as choices rather than silently ignored.
            let first = first_match.get_or_insert_with(Instant::now);
            if first.elapsed() >= Duration::from_secs(2) {
                break;
            }
        }
        if last.elapsed() >= Duration::from_secs(1) {
            emit(
                &json!({"event":"syslog_progress","bytes":bytes,"lines":count,"card_matches":matches,"remaining_seconds":duration.saturating_sub(start.elapsed().as_secs())}),
            );
            last = Instant::now();
        }
    }
    emit(
        &json!({"event":"syslog_complete","bytes":bytes,"lines":count,"card_matches":matches,"cancelled":cancel.load(Ordering::Relaxed)}),
    );
    if bytes == 0 {
        return Err(device::Error::new(
            device::ErrorKind::Timeout,
            "syslog_no_data",
        ));
    }
    Ok(())
}
fn main() -> ExitCode {
    let args = Args::parse();
    let cancelled = Arc::new(AtomicBool::new(false));
    let handler = cancelled.clone();
    ctrlc::set_handler(move || {
        handler.store(true, Ordering::Relaxed);
    })
    .expect("install cancellation handler");
    let code = if std::env::var_os("AIRCARD_INTERNAL_WORKER").is_some() {
        match run(&args, &cancelled) {
            Ok(code) => code,
            Err(e) => {
                error(&e);
                1
            }
        }
    } else {
        watchdog(&args, &cancelled)
    };
    let _ = io::stdout().flush();
    ExitCode::from(code)
}

#[cfg(test)]
mod connection_tests {
    use super::*;
    #[test]
    fn read_connection_retries_only_transient_failures_once() {
        for kind in [
            device::ErrorKind::Timeout,
            device::ErrorKind::Disconnected,
            device::ErrorKind::Locked,
            device::ErrorKind::NotPaired,
            device::ErrorKind::Tls,
        ] {
            let mut calls = 0;
            let result: device::Result<()> = read_connection_retry(
                || {
                    calls += 1;
                    Err(device::Error::new(kind, "fixture"))
                },
                &AtomicBool::new(false),
            );
            assert!(result.is_err());
            assert_eq!(
                calls,
                if matches!(
                    kind,
                    device::ErrorKind::Timeout | device::ErrorKind::Disconnected
                ) {
                    2
                } else {
                    1
                }
            );
        }
        let mut calls = 0;
        let value = read_connection_retry(
            || {
                calls += 1;
                if calls == 1 {
                    Err(device::Error::new(device::ErrorKind::Timeout, "fixture"))
                } else {
                    Ok(42)
                }
            },
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(value, 42);
        assert_eq!(calls, 2);
        let result: device::Result<()> =
            read_connection_retry(|| panic!("cancelled"), &AtomicBool::new(true));
        assert_eq!(result.unwrap_err().kind, device::ErrorKind::Cancelled);
    }
}
