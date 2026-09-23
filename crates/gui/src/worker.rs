//! GUI jobs never own native device handles. The CLI supplies the bounded device worker.
use serde_json::Value;
use std::{
    io::{BufRead, BufReader, Read},
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};

pub enum Message {
    Event(Value),
    Finished(Result<(), String>),
}
pub struct Job {
    pub receiver: Receiver<Message>,
    pub cancel: Arc<AtomicBool>,
}
fn event(tx: &SyncSender<Message>, value: Value) {
    let _ = tx.send(Message::Event(value));
}
pub fn start(binary: PathBuf, args: Vec<String>) -> Job {
    let (tx, receiver) = mpsc::sync_channel(256);
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    thread::spawn(move || {
        let result = verify_cli(&binary, &flag).and_then(|()| {
            event(&tx, serde_json::json!({"event":"cli_ready"}));
            run(&binary, &args, &flag, &tx)
        });
        let _ = tx.send(Message::Finished(result));
    });
    Job { receiver, cancel }
}
fn verify_cli(binary: &Path, cancel: &AtomicBool) -> Result<(), String> {
    let mut child = Command::new(binary)
        .arg("--version")
        .env_remove("AIRCARD_INTERNAL_WORKER")
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .process_group(0).spawn()
        .map_err(|_| "Cannot start the matching aircard CLI. Keep aircard and aircard-gui from the same release together and install the native libraries listed in docs/INSTALL.md.".to_string())?;
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let error_reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr.take(4097).read_to_end(&mut bytes).map(|_| bytes)
    });
    let reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.take(257).read_to_end(&mut bytes).map(|_| bytes)
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Err(_) => break None,
            _ => {}
        }
        if cancel.load(Ordering::Relaxed) || start.elapsed() >= Duration::from_secs(3) {
            break None;
        }
        thread::sleep(Duration::from_millis(20));
    };
    let _ = nix::sys::signal::killpg(
        nix::unistd::Pid::from_raw(child.id() as i32),
        nix::sys::signal::Signal::SIGKILL,
    );
    let _ = child.wait();
    let bytes = reader.join().ok().and_then(Result::ok).unwrap_or_default();
    let errors = error_reader
        .join()
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default();
    if cancel.load(Ordering::Relaxed) {
        return Err("CLI check cancelled.".into());
    }
    if status.is_none() {
        return Err("CLI version check did not finish within 3 seconds. Reinstall both binaries from the same release.".into());
    }
    if !status.is_some_and(|s| s.success()) {
        if let Some(message) = loader_error(&errors) {
            return Err(message);
        }
        return Err("CLI cannot run. Reinstall both binaries from the same release and check native library dependencies in docs/INSTALL.md.".into());
    }
    if bytes.len() > 256
        || String::from_utf8_lossy(&bytes).trim() != concat!("aircard ", env!("CARGO_PKG_VERSION"))
    {
        return Err(format!(
            "CLI version mismatch. This GUI requires aircard {}. Replace both binaries with files from the same release.",
            env!("CARGO_PKG_VERSION")
        ));
    }
    Ok(())
}
fn loader_error(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?;
    if let Some(rest) = text.split("error while loading shared libraries: ").nth(1) {
        let name = rest.split(':').next()?;
        // Show only a bounded library name, never the executable path or arbitrary stderr.
        if name.len() <= 128
            && name.contains(".so")
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
        {
            return Some(format!(
                "CLI cannot load {name}. Extract the complete release, keeping its lib directory beside aircard. If the library is a system dependency, install a compatible package or build locally; see docs/INSTALL.md."
            ));
        }
    }
    if text.contains("version `GLIBC_") && text.contains("not found") {
        return Some("This release requires a newer glibc than your system provides. Use a compatible build or compile locally; see BUILD-INFO.txt and docs/INSTALL.md.".into());
    }
    None
}
fn run(
    binary: &Path,
    args: &[String],
    cancel: &AtomicBool,
    tx: &SyncSender<Message>,
) -> Result<(), String> {
    let mut child=Command::new(binary).args(args).env_remove("AIRCARD_INTERNAL_WORKER")
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).process_group(0)
        .spawn().map_err(|_|"Cannot start the matching aircard CLI. Keep aircard and aircard-gui in the same directory.".to_string())?;
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let expected = if !args.iter().any(|a| a == "--apply")
        && args.first().is_some_and(|a| a.starts_with("card-"))
    {
        "dry_run"
    } else {
        match args.first().map(String::as_str) {
            Some("devices") => "devices_complete",
            Some("setup-token") => "token_ready",
            Some("scan") => "syslog_complete",
            Some("probe") => "probe_complete",
            Some("card-apply" | "card-restore") => "card_operation_complete",
            Some("card-recover") => "card_recovery_complete",
            Some("prepare-card") => "card_prepared",
            _ => "unsupported_worker_command",
        }
    };
    let completed = Arc::new(AtomicBool::new(false));
    let completion = completed.clone();
    let output = tx.clone();
    let reader = thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut total = 0usize;
        loop {
            let mut line = Vec::new();
            let Ok(n) = reader.by_ref().take(65537).read_until(b'\n', &mut line) else {
                break;
            };
            if n == 0 {
                break;
            }
            total += n;
            if n > 65536 || total > 4 * 1024 * 1024 {
                break;
            }
            if let Ok(value) = serde_json::from_slice::<Value>(&line) {
                if value["event"] == expected {
                    completion.store(true, Ordering::Relaxed);
                }
                event(&output, value);
            }
        }
    });
    let errors = thread::spawn(move || {
        let mut count = 0;
        let mut buf = [0u8; 4096];
        let mut stderr = stderr;
        while let Ok(n) = stderr.read(&mut buf) {
            if n == 0 {
                break;
            }
            count += n;
        }
        count
    });
    let start = Instant::now();
    let mut stopping = None;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Err(_) => break Err("Could not wait for the CLI worker.".to_string()),
            Ok(None) => {}
        }
        if (cancel.load(Ordering::Relaxed) || start.elapsed() > Duration::from_secs(1200))
            && stopping.is_none()
        {
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(child.id() as i32),
                nix::sys::signal::Signal::SIGINT,
            );
            stopping = Some(Instant::now());
            event(
                tx,
                serde_json::json!({"event":"stage","stage":"Cancelling; waiting for cleanup"}),
            );
        }
        if stopping.is_some_and(|s| s.elapsed() > Duration::from_secs(100)) {
            let _ = nix::sys::signal::killpg(
                nix::unistd::Pid::from_raw(child.id() as i32),
                nix::sys::signal::Signal::SIGKILL,
            );
            let _ = child.wait();
            break Err("Worker stopped after the cleanup deadline. Keep the recovery directory and use Recover before retrying.".into());
        }
        thread::sleep(Duration::from_millis(30));
    };
    // Reap the entire private process group, including a worker left by a crashed watchdog.
    let _ = nix::sys::signal::killpg(
        nix::unistd::Pid::from_raw(child.id() as i32),
        nix::sys::signal::Signal::SIGKILL,
    );
    let _ = child.wait();
    let _ = reader.join();
    let stderr_bytes = errors.join().unwrap_or(0);
    let status = status?;
    if status.success() && completed.load(Ordering::Relaxed) {
        Ok(())
    } else if status.success() {
        Err("CLI ended without the required completion event. Keep its recovery directory and inspect the last stage.".into())
    } else {
        Err(format!(
            "CLI exited with code {}. See the stage and error details below. Stderr: {stderr_bytes} bytes.",
            status
                .code()
                .map_or_else(|| "signal".into(), |c| c.to_string())
        ))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn loader_diagnostics_show_only_the_missing_library_or_glibc_requirement() {
        let message = loader_error(b"/private/downloads/aircard: error while loading shared libraries: libusbmuxd-2.0.so.6: cannot open shared object file: No such file or directory\n").unwrap();
        assert!(message.contains("libusbmuxd-2.0.so.6"));
        assert!(message.contains("lib directory"));
        assert!(!message.contains("private/downloads"));
        assert!(
            loader_error(b"aircard: /lib/libc.so.6: version `GLIBC_2.39' not found")
                .unwrap()
                .contains("newer glibc")
        );
        assert!(loader_error(b"private arbitrary stderr").is_none());
        assert!(
            loader_error(b"error while loading shared libraries: /private/secret.so: missing")
                .is_none()
        );
    }
    #[test]
    fn mismatched_cli_is_rejected_before_running_commands() {
        // /bin/echo accepts --version, but is not the matching AirCard CLI.
        let error = verify_cli(Path::new("/bin/echo"), &AtomicBool::new(false)).unwrap_err();
        assert!(error.contains("version mismatch"));
    }
    #[test]
    fn missing_worker_is_actionable_and_does_not_panic() {
        let job = start(PathBuf::from("/nonexistent/aircard-test"), vec![]);
        assert!(
            matches!(job.receiver.recv_timeout(Duration::from_secs(2)),Ok(Message::Finished(Err(e))) if e.contains("matching aircard CLI"))
        );
    }
    #[test]
    fn cancellation_reaps_a_running_process_group() {
        let (tx, rx) = mpsc::sync_channel(256);
        let flag = Arc::new(AtomicBool::new(false));
        let other = flag.clone();
        let handle = thread::spawn(move || {
            run(
                Path::new("/bin/sh"),
                &[
                    "-c".into(),
                    "printf '%s\\n' '{\"event\":\"started\"}'; exec sleep 20".into(),
                ],
                &other,
                &tx,
            )
        });
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(2)),
            Ok(Message::Event(_))
        ));
        flag.store(true, Ordering::Relaxed);
        assert!(handle.join().unwrap().is_err());
    }
}
