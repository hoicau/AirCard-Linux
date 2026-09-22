#![forbid(unsafe_code)]
use airtraffic::{AirTrafficClient, PassiveClient, ReceiveTransport};
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
    about = "Experimental paired-device Linux PoC. ATC handshake requires --apply; no asset sync."
)]
struct Args {
    #[arg(long, global = true)]
    udid: Option<String>,
    #[arg(long, value_enum, global = true)]
    transport: Option<Route>,
    /// Per-command deadline, including a native-call watchdog (1..300 seconds).
    #[arg(long, default_value_t = 15, value_parser = clap::value_parser!(u64).range(1..=300), global = true)]
    timeout: u64,
    #[command(subcommand)]
    command: Command,
}
#[derive(Debug, Subcommand)]
enum Command {
    /// Enumerate routes and verify existing host pairing. Identifiers redacted by default.
    Devices {
        #[arg(long)]
        show_identifiers: bool,
    },
    /// Check existing pairing, iOS version and AFC access without writing.
    Probe,
    /// Bounded syslog capture. Output is counts by default; --raw prints private device logs locally.
    Syslog {
        #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u64).range(1..=3600))]
        duration: u64,
        #[arg(long)]
        raw: bool,
    },
    /// Apply upstream Wallet hash filters. Hashes are suppressed unless explicitly requested.
    Scan {
        #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u64).range(1..=3600))]
        duration: u64,
        #[arg(long)]
        show_hashes: bool,
    },
    /// Emit diagnostic metadata, not a restorable Books backup.
    Snapshot,
    /// Start com.apple.atc, receive only, report lengths and unknown framing. Exit 3 if unvalidated.
    AtcSmoke,
    /// Attempt HostInfo/RequestingSync and stop at ReadyForSync. No metadata or asset transfer.
    AtcReady {
        /// Required to open a device sync session; otherwise only show the plan.
        #[arg(long)]
        apply: bool,
    },
    /// List AFC entries. Only count is printed by default.
    AfcList {
        #[arg(default_value = ".")]
        path: String,
        #[arg(long)]
        show_names: bool,
    },
    /// Read a bounded regular AFC file; report only byte count.
    AfcRead {
        path: String,
        #[arg(long, default_value_t = 1048576, value_parser = clap::value_parser!(u64).range(1..=1048576))]
        limit: u64,
    },
    /// Dry-run or create/read/remove a controlled scratch asset. Requires --apply for device writes.
    AfcSelfTest {
        #[arg(long)]
        apply: bool,
    },
}
fn emit(value: &impl serde::Serialize) {
    println!(
        "{}",
        serde_json::to_string(value).expect("serializable event")
    );
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
        Command::Syslog { duration, .. } | Command::Scan { duration, .. } => {
            *duration + args.timeout + 5
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
            while grace.elapsed() < Duration::from_secs(2) {
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
                &json!({"event":"error","kind":if cancel.load(Ordering::Relaxed){"cancelled"}else{"native_call_deadline"},"stage":"worker","hint":"Inspect the last stage event. An interrupted AFC self-test may require scratch cleanup."}),
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
    if matches!(args.command, Command::AtcReady { apply: false }) {
        emit(
            &json!({"event":"dry_run","applied":false,"plan":["verify existing host pairing and selected transport","start com.apple.atc with required TLS","receive SyncAllowed","send HostInfo on session 0","send RequestingSync(Book) on session 1","answer Ping with Pong","stop on ReadyForSync or any failure","close service"],"metadata_or_assets_sent":false}),
        );
        return Ok(0);
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
    let info = provider.pairing(&selected)?;
    emit(&json!({"event":"paired","info":info}));
    match &args.command {
        Command::Devices { .. } => unreachable!(),
        Command::Probe | Command::Snapshot => {
            emit(&json!({"event":"stage","stage":"start_afc"}));
            let mut afc = provider.afc(&selected)?;
            let snapshot = aircard_core::DiagnosticSnapshot {
                schema_version: 1,
                kind: "diagnostic_only_not_a_backup".into(),
                ios_version: info.ios_version,
                transport: info.transport.as_str().into(),
                pairing: info.pairing,
                afc_root_entries: afc.list(".")?.len(),
                afc_tls: afc.tls(),
            };
            emit(&snapshot);
        }
        Command::AtcSmoke => {
            emit(
                &json!({"event":"stage","stage":"start_service","service":"com.apple.atc","state":"Connecting"}),
            );
            let service = match device::open_verified_service(&provider, &selected, "com.apple.atc")
            {
                Ok((_, service)) => service,
                Err(e) => {
                    emit(
                        &json!({"event":"atc_smoke","state":"Failed","failed_from":"Connecting","service_started":false,"bytes_received":0,"first_direction":"unknown","framing":"unconfirmed","protocol_validated":false,"error":e}),
                    );
                    return Ok(3);
                }
            };
            emit(&json!({"event":"service_started","service":"com.apple.atc","tls":service.tls()}));
            let report = PassiveClient {
                transport: Bridge(service),
            }
            .smoke(Duration::from_secs(args.timeout), &|| {
                cancel.load(Ordering::Relaxed)
            });
            let code = if report.first_message_validated { 0 } else { 3 };
            emit(&report);
            return Ok(code);
        }
        Command::AtcReady { apply: true } => {
            emit(
                &json!({"event":"stage","stage":"start_service","service":"com.apple.atc","state":"Connecting"}),
            );
            let service = match device::open_verified_service(&provider, &selected, "com.apple.atc")
            {
                Ok((_, service)) => service,
                Err(e) => {
                    emit(
                        &json!({"event":"atc_ready_complete","state":"Failed","ready_for_sync":false,"stage":"start_service","error":e}),
                    );
                    return Ok(3);
                }
            };
            emit(&json!({"event":"service_started","service":"com.apple.atc","tls":service.tls()}));
            let library_id =
                std::fs::read_to_string("/proc/sys/kernel/random/uuid").map_err(|_| {
                    device::Error::new(device::ErrorKind::Native, "generate_library_id")
                })?;
            let report = airtraffic::handshake::HandshakeClient {
                transport: Bridge(service),
            }
            .ready(
                library_id.trim(),
                Duration::from_secs(args.timeout),
                &|| cancel.load(Ordering::Relaxed),
                emit,
            );
            let code = if report.ready_for_sync {
                0
            } else if report
                .failure
                .as_ref()
                .is_some_and(|f| f.kind == airtraffic::handshake::FailureKind::Cancelled)
            {
                130
            } else {
                3
            };
            emit(&report);
            return Ok(code);
        }
        Command::AtcReady { apply: false } => unreachable!("dry run handled before device access"),
        Command::Syslog { duration, raw } => {
            capture(&provider, &selected, *duration, *raw, false, false, cancel)?
        }
        Command::Scan {
            duration,
            show_hashes,
        } => capture(
            &provider,
            &selected,
            *duration,
            false,
            true,
            *show_hashes,
            cancel,
        )?,
        Command::AfcList { path, show_names } => {
            let names = provider.afc(&selected)?.list(path)?;
            emit(
                &json!({"event":"afc_list","count":names.len(),"names":if *show_names{Some(names)}else{None}}),
            );
        }
        Command::AfcRead { path, limit } => {
            let data = provider.afc(&selected)?.read(path, *limit as usize)?;
            emit(&json!({"event":"afc_read","bytes":data.len()}));
        }
        Command::AfcSelfTest { apply } => {
            if !apply {
                emit(
                    &json!({"event":"dry_run","changes":["create unique AFC scratch directory","write 26-byte synthetic file","read and compare","remove file and directory"],"applied":false}),
                );
                return Ok(0);
            }
            let mut afc = provider.afc(&selected)?;
            let root = format!(
                "AirCard-Linux-PoC-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            );
            emit(
                &json!({"event":"scratch_created_plan","scratch_path":root,"cleanup":"remove roundtrip.txt then directory if interrupted"}),
            );
            let report =
                device::self_test::roundtrip(&mut afc, &root, &|| cancel.load(Ordering::Relaxed))?;
            let success = report.roundtrip_ok && report.cleanup_ok;
            emit(&report);
            if !success {
                return Ok(1);
            }
        }
    }
    Ok(0)
}
fn capture(
    provider: &impl DeviceProvider,
    selected: &device::Device,
    duration: u64,
    raw: bool,
    scan: bool,
    show_hashes: bool,
    cancel: &AtomicBool,
) -> device::Result<()> {
    emit(&json!({"event":"stage","stage":"start_syslog"}));
    let mut service = provider.syslog(selected)?;
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
                    if raw {
                        println!("{line}");
                    }
                    if scan
                        && let Some(hash) =
                            aircard_core::scanner::extract_card_hash_from_line(&line)
                    {
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
        if last.elapsed() >= Duration::from_secs(1) {
            emit(
                &json!({"event":"syslog_progress","bytes":bytes,"lines":count,"card_matches":matches}),
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
