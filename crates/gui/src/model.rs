use serde_json::Value;
#[derive(Default)]
pub struct Discovery {
    pub started: bool,
    pub listening: bool,
    pub remaining: u64,
    pub lines: u64,
    pub cancelled: bool,
    pub finished: bool,
    pub failure: Option<String>,
}
impl Discovery {
    pub fn begin(&mut self) {
        *self = Self {
            started: true,
            ..Self::default()
        };
    }
    pub fn event(&mut self, event: &Value) {
        match event["event"].as_str() {
            Some("service_started") if event["service"] == "com.apple.syslog_relay" => {
                self.listening = true;
                self.remaining = 60;
            }
            Some("syslog_progress") => {
                self.remaining = event["remaining_seconds"].as_u64().unwrap_or(0);
                self.lines = event["lines"].as_u64().unwrap_or(0);
            }
            Some("syslog_complete") => {
                self.finished = true;
                self.listening = false;
                self.lines = event["lines"].as_u64().unwrap_or(0);
                self.cancelled = event["cancelled"].as_bool().unwrap_or(false);
            }
            _ => {}
        }
    }
    pub fn result(&self, count: usize) -> String {
        if self.cancelled {
            "Card detection stopped. Choose a detected identifier or retry when ready.".into()
        } else if let Some(message) = &self.failure {
            message.clone()
        } else if count == 1 {
            "Card identifier filled in. Check that you opened the intended card, then close Wallet before applying.".into()
        } else if count > 1 {
            format!(
                "Found {count} card identifiers. Choose the intended card below, or retry while opening only that card."
            )
        } else if self.lines == 0 {
            "No iPhone logs received. Unlock the phone, reconnect USB and refresh devices, then retry.".into()
        } else {
            "No card identifier appeared in Wallet logs. Return to the card list, retry detection, then reopen the intended card. Some iOS versions or cards may hide identifiers.".into()
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub udid: String,
    pub route: String,
    pub ios: String,
    pub paired: bool,
    pub status: String,
}
impl Device {
    pub fn from_event(v: &Value) -> Option<Self> {
        if v["event"] != "device" {
            return None;
        }
        let udid = v["udid"].as_str()?.to_string();
        let route = v["transport"].as_str()?.to_string();
        if udid == "[redacted]" || !matches!(route.as_str(), "usb" | "wifi") {
            return None;
        }
        let status = v["info"]["pairing"]
            .as_str()
            .or(v["error"]["kind"].as_str())
            .unwrap_or("unknown")
            .to_string();
        Some(Self {
            udid,
            route,
            ios: v["info"]["ios_version"]
                .as_str()
                .unwrap_or("unknown")
                .into(),
            paired: status == "paired_session_verified",
            status,
        })
    }
    pub fn label(&self) -> String {
        let tail: String = self
            .udid
            .chars()
            .rev()
            .take(6)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        format!("iPhone …{tail} / {}", self.route.to_uppercase())
    }
    pub fn selector(&self) -> Vec<String> {
        vec![
            "--udid".into(),
            self.udid.clone(),
            "--transport".into(),
            self.route.clone(),
        ]
    }
}

/// Auto selection is limited to one physical device; USB wins only among its routes.
pub fn preferred_device(devices: &[Device], route: Option<&str>) -> Option<usize> {
    let candidates: Vec<_> = devices
        .iter()
        .enumerate()
        .filter(|(_, d)| d.paired && route.is_none_or(|r| d.route == r))
        .collect();
    let id = &candidates.first()?.1.udid;
    if candidates.iter().any(|(_, d)| &d.udid != id) {
        return None;
    }
    candidates
        .iter()
        .find(|(_, d)| d.route == "usb")
        .or_else(|| candidates.first())
        .map(|(i, _)| *i)
}

pub fn progress_text(event: &Value) -> Option<&'static str> {
    match event["stage"]
        .as_str()
        .filter(|_| event["event"] == "stage")
    {
        Some("enumerate") => Some("Finding your iPhone"),
        Some("existing_pair_session") => Some("Checking iPhone trust"),
        Some("start_syslog") => Some("Connecting to Wallet activity"),
        Some("retry_read_connection") => Some("Reconnecting once"),
        Some("start_afc") => Some("Checking file access"),
        Some("prepare_transfer") => Some("Preparing artwork transfer"),
        Some("read_original_artwork") => Some("Backing up original artwork"),
        Some("write_artwork") => Some("Writing card artwork"),
        Some("verify_artwork") => Some("Verifying card artwork"),
        Some("refresh_caches") => Some("Refreshing Wallet caches"),
        Some("restore_originals") => Some("Restoring original files"),
        Some("preserve_books") => Some("Preserving Books data"),
        Some("restore_books") => Some("Restoring Books data"),
        Some("save_backup") => Some("Saving private backup"),
        Some("cleanup") => Some("Finishing cleanup"),
        _ => match event["event"].as_str() {
            Some("probe_complete") => Some("Device checks passed"),
            Some("card_recovery_complete") => Some("Recovery complete"),
            _ => None,
        },
    }
}

/// Use bounded protocol fields for user guidance; raw identifiers stay out of status copy.
pub fn error_message(event: &Value) -> Option<String> {
    if let Some(kind) = event["failure"]["kind"].as_str() {
        return Some(match kind {
            "unsupported_grappa" => "This iPhone requested an unsupported sync authentication method. Keep the recovery directory; the downloaded token does not establish device compatibility.",
            "device_rejected" => "The iPhone rejected the sync request. Check the sync token and iOS compatibility; keep any recovery directory.",
            "device_protected" => "The iPhone is locked or its data is protected. Unlock it and keep any recovery directory.",
            "precondition_changed" => "Books data changed during the operation. Close Books and keep the recovery directory.",
            "timeout" => "The iPhone did not complete the sync stage in time. Keep the recovery directory and reconnect the original iPhone.",
            "disconnected" => "The iPhone disconnected during sync. Reconnect the original iPhone and keep the recovery directory.",
            "cancelled" => "Sync cancelled. Wait for restoration and cleanup to finish.",
            _ => "The iPhone could not complete the sync stage. Keep the recovery directory and inspect Details for the protocol failure.",
        }.into());
    }
    let error = &event["error"];
    let kind = error["kind"].as_str().or(event["kind"].as_str());
    let message = match kind {
        Some("locked") => "Unlock your iPhone and retry the device check.",
        Some("not_paired" | "trust_pending" | "trust_denied") => {
            "Connect by USB, unlock your iPhone and confirm Trust through your system's pairing tool. Then refresh devices."
        }
        Some("usbmux_unavailable") => {
            "Cannot reach usbmuxd. Install your distribution's usbmuxd/libimobiledevice packages and check the service. See Help for commands."
        }
        Some("no_device") => {
            "No iPhone is available on the selected connection. Reconnect and unlock it, then refresh devices."
        }
        Some("tls") => {
            "The trusted connection failed. Check this computer's existing pairing and reconnect the iPhone."
        }
        Some("service_denied") => {
            "The iPhone refused this service. Unlock it, close other sync apps and check iOS compatibility."
        }
        Some("disconnected") => {
            "The iPhone connection was lost. Reconnect the same iPhone and retry the device check."
        }
        Some("timeout") => {
            "The connection timed out. Unlock the iPhone and check the cable or Wi-Fi connection."
        }
        Some("invalid_input")
            if error["operation"]
                .as_str()
                .is_some_and(|s| s.starts_with("grappa_token") || s.starts_with("local_input")) =>
        {
            "A local input file could not be read. Check its path, size and permissions. Sync tokens must be 84 bytes with permissions 0600."
        }
        _ => {
            return event["hint"]
                .as_str()
                .or(error["hint"].as_str())
                .map(str::to_string);
        }
    };
    Some(message.into())
}
#[derive(Clone)]
pub struct Confirmation {
    pub title: String,
    pub summary: String,
    pub device: Device,
    pub args: Vec<String>,
    pub accepted: bool,
}
impl Confirmation {
    pub fn can_apply(&self) -> bool {
        self.accepted && self.device.paired && self.args.iter().all(|a| a != "--apply")
    }
}
pub fn redacted(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        if matches!(k.as_str(), "udid" | "device_fingerprint" | "hash") {
                            Value::String("[redacted]".into())
                        } else {
                            redacted(v)
                        },
                    )
                })
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(redacted).collect()),
        _ => value.clone(),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn device(id: &str, route: &str, paired: bool) -> Device {
        Device {
            udid: id.into(),
            route: route.into(),
            paired,
            ios: "fixture".into(),
            status: "fixture".into(),
        }
    }
    #[test]
    fn automatic_route_never_guesses_between_phones_or_changes_explicit_wifi() {
        let routes = vec![device("a", "wifi", true), device("a", "usb", true)];
        assert_eq!(preferred_device(&routes, None), Some(1));
        assert_eq!(preferred_device(&routes, Some("wifi")), Some(0));
        assert_eq!(preferred_device(&routes[..1], None), Some(0));
        assert_eq!(preferred_device(&routes[..1], Some("usb")), None);
        let mut ambiguous = routes;
        ambiguous.push(device("b", "usb", true));
        assert_eq!(preferred_device(&ambiguous, None), None);
        assert_eq!(preferred_device(&[device("a", "usb", false)], None), None);
    }
    #[test]
    fn status_messages_keep_protocol_rejection_specific_and_private() {
        let event = serde_json::json!({"event":"atc_sync","failure":{"kind":"unsupported_grappa","stage":"HostInfo","device_session":12345}});
        let message = error_message(&event).unwrap();
        assert!(message.contains("unsupported sync authentication"));
        assert!(!message.contains("12345"));
        let event = serde_json::json!({"event":"error","error":{"kind":"locked","hint":"private raw error","operation":"existing_pair_session"}});
        assert!(error_message(&event).unwrap().contains("Unlock"));
        assert_eq!(
            progress_text(&serde_json::json!({"event":"stage","stage":"write_artwork"})),
            Some("Writing card artwork")
        );
        assert_eq!(
            progress_text(&serde_json::json!({"event":"atc_message","stage":"ReadyForSync"})),
            None
        );
    }
    #[test]
    fn discovery_distinguishes_ready_empty_no_logs_and_cancelled() {
        let mut scan = Discovery::default();
        scan.begin();
        assert!(!scan.listening);
        scan.event(
            &serde_json::json!({"event":"service_started","service":"com.apple.syslog_relay"}),
        );
        assert!(scan.listening);
        scan.event(&serde_json::json!({"event":"syslog_complete","lines":20,"cancelled":false}));
        assert!(scan.finished && !scan.listening);
        assert!(scan.result(0).contains("No card identifier"));
        assert!(scan.result(1).contains("filled in"));
        assert!(scan.result(2).contains("Choose"));
        scan.begin();
        assert!(scan.result(0).contains("No iPhone logs"));
        scan.event(&serde_json::json!({"event":"syslog_complete","lines":0,"cancelled":true}));
        assert!(scan.result(0).contains("stopped"));
    }
    #[test]
    fn confirmations_bind_device_route_and_explicit_consent() {
        let d = Device {
            udid: "synthetic-device".into(),
            route: "wifi".into(),
            ios: "27.0".into(),
            paired: true,
            status: "paired_session_verified".into(),
        };
        let mut c = Confirmation {
            title: "Test".into(),
            summary: "test".into(),
            device: d,
            args: vec!["card-apply".into()],
            accepted: false,
        };
        assert!(!c.can_apply());
        c.accepted = true;
        assert!(c.can_apply());
        assert!(c.device.selector().contains(&"wifi".to_string()));
        c.device.paired = false;
        assert!(!c.can_apply());
    }
    #[test]
    fn device_parse_and_logs_never_retain_identifiers_in_copy_details() {
        let event = serde_json::json!({"event":"device","udid":"private-identifier","transport":"usb","info":{"pairing":"paired_session_verified","ios_version":"27.0"}});
        assert!(Device::from_event(&event).unwrap().paired);
        assert!(!redacted(&event).to_string().contains("private-identifier"));
    }
}
