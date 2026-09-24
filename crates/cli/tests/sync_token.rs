use std::{os::unix::fs::MetadataExt, path::PathBuf, process::Command};

#[test]
fn setup_works_without_external_programs_and_preserves_existing_files() {
    let root = std::env::temp_dir().join(format!("aircard-offline-token-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    struct Clean(PathBuf);
    impl Drop for Clean {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _clean = Clean(root.clone());
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_aircard"))
            .arg("setup-token")
            .args(args)
            .env("PATH", &root) // No curl or other executable is available here.
            .env("XDG_DATA_HOME", root.join("data"))
            .env(
                "USBMUXD_SOCKET_ADDRESS",
                "unix:/nonexistent/aircard-test.socket",
            )
            .output()
            .unwrap()
    };
    let output = run(&[]);
    assert!(output.status.success(), "{:?}", output);
    let event: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(event["event"], "token_ready");
    assert_eq!(
        event.as_object().unwrap().len(),
        2,
        "only event and path are emitted"
    );
    let path = root.join("data/aircard/token.bin");
    assert_eq!(event["path"], path.to_str().unwrap());
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(bytes.len(), 84);
    assert_eq!(
        aircard_core::sha256(&bytes),
        "76ff7eae7d51aad6785a835b160c37f018a79c70579fdfe5231a1d73976a7ea5"
    );
    assert_eq!(path.metadata().unwrap().mode() & 0o777, 0o600);
    assert_eq!(
        path.parent().unwrap().metadata().unwrap().mode() & 0o777,
        0o700
    );

    let cached = path.metadata().unwrap();
    assert!(run(&[]).status.success());
    assert_eq!(path.metadata().unwrap().ino(), cached.ino());
    assert_eq!(
        path.metadata().unwrap().modified().unwrap(),
        cached.modified().unwrap()
    );

    let custom = root.join("custom.bin");
    let args = ["--output", custom.to_str().unwrap()];
    assert!(run(&args).status.success());
    assert_eq!(std::fs::read(&custom).unwrap(), bytes);
    std::fs::write(&custom, [42; 84]).unwrap();
    assert!(run(&args).status.success());
    assert_eq!(std::fs::read(&custom).unwrap(), vec![42; 84]);
    std::fs::write(&custom, b"invalid").unwrap();
    let output = run(&args);
    assert!(!output.status.success());
    assert_eq!(std::fs::read(&custom).unwrap(), b"invalid");
    let event: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(event["event"], "error");
    assert!(event["hint"].as_str().unwrap().contains("84-byte"));
}
