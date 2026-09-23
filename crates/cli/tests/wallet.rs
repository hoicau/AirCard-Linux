use std::process::Command;
#[test]
fn restore_validation_reports_backup_failures_without_reading_the_token() {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let directory = std::env::temp_dir().join(format!(
        "aircard-backup-input-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&directory).unwrap();
    struct Clean(std::path::PathBuf);
    impl Drop for Clean {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _clean = Clean(directory.clone());
    let backup = directory.join("private-backup.json");
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&backup)
        .unwrap();
    let link = directory.join("backup-link");
    std::os::unix::fs::symlink(&backup, &link).unwrap();
    let check = |path: &std::path::Path, expected: &str| {
        let output = Command::new(env!("CARGO_BIN_EXE_aircard"))
            .arg("card-restore")
            .arg(path)
            .args([
                "--journal",
                "/nonexistent/journal",
                "--grappa-token",
                "/nonexistent/token",
            ])
            .env(
                "USBMUXD_SOCKET_ADDRESS",
                "unix:/nonexistent/aircard-test.socket",
            )
            .output()
            .unwrap();
        assert!(!output.status.success());
        let event: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(event["error"]["operation"], expected);
        assert!(!String::from_utf8_lossy(&output.stdout).contains("private-backup.json"));
    };
    check(&directory.join("missing"), "card_backup_input_not_found");
    check(&directory, "card_backup_input_not_file");
    check(&link, "card_backup_input_symlink");
    std::fs::set_permissions(&backup, std::fs::Permissions::from_mode(0o644)).unwrap();
    check(&backup, "card_backup_input_permissions");
    std::fs::set_permissions(&backup, std::fs::Permissions::from_mode(0o600)).unwrap();
    file.set_len(520 * 1024 * 1024 + 1).unwrap();
    check(&backup, "card_backup_input_size");
    file.set_len(0).unwrap();
    check(&backup, "card_backup_decode");
    let error = cli::local::token(Some(&directory.join("missing"))).unwrap_err();
    assert_eq!(error.operation, "grappa_token_input_not_found");
}
#[test]
fn card_dry_run_never_reads_token_or_connects() {
    let output = Command::new(env!("CARGO_BIN_EXE_aircard"))
        .args([
            "card-test",
            "--card-hash",
            "AAoUHigyPEZQWmRueIKMlqCqtL4=",
            "--journal",
            "/nonexistent/journal",
            "--grappa-token",
            "/nonexistent/token",
            "--udid",
            "nonexistent-device",
            "--transport",
            "wifi",
        ])
        .env(
            "USBMUXD_SOCKET_ADDRESS",
            "unix:/nonexistent/aircard-test.socket",
        )
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let event: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(event["event"], "dry_run");
    assert_eq!(event["applied"], false);
}
#[test]
fn unrelated_features_are_not_product_commands() {
    for command in [
        "prepare-theme",
        "theme-canary",
        "theme-key-test",
        "books-test",
        "books-restore",
        "atc-ready",
        "afc-self-test",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_aircard"))
            .arg(command)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
    }
}
#[test]
fn unsafe_card_target_fails_without_device_access() {
    let output = Command::new(env!("CARGO_BIN_EXE_aircard"))
        .args([
            "card-test",
            "--card-hash",
            "../other",
            "--journal",
            "/nonexistent/journal",
            "--grappa-token",
            "/nonexistent/token",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let event: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(event["error"]["operation"], "card_hash");
}
