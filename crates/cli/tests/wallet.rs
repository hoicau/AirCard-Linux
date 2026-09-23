use std::process::Command;
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
