use std::process::Command;

#[test]
fn dry_run_needs_no_mux_socket_pairing_or_valid_device() {
    let output = Command::new(env!("CARGO_BIN_EXE_aircard"))
        .args([
            "atc-ready",
            "--transport",
            "wifi",
            "--udid",
            "nonexistent-test-device",
        ])
        .env(
            "USBMUXD_SOCKET_ADDRESS",
            "unix:/nonexistent/aircard-test.socket",
        )
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let event: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(event["event"], "dry_run");
    assert_eq!(event["applied"], false);
    assert_eq!(event["metadata_or_assets_sent"], false);
}
