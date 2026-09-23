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
