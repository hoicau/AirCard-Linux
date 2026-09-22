use airtraffic::{AirTrafficClient, PassiveClient, ReceiveTransport};
use device::{
    AfcAccess, Device, DeviceInfo, DeviceProvider, Error, ErrorKind, ServiceTransport, Transport,
};
use std::{cell::Cell, io, time::Duration};
struct MockLockdown {
    pairing_error: Option<ErrorKind>,
    service_error: Option<ErrorKind>,
    starts: Cell<usize>,
    disconnected: bool,
}
struct MockService(bool);
impl ServiceTransport for MockService {
    fn receive(&mut self, _: &mut [u8], _: u32) -> io::Result<usize> {
        Err(if self.0 {
            io::ErrorKind::ConnectionReset
        } else {
            io::ErrorKind::TimedOut
        }
        .into())
    }
    fn tls(&self) -> bool {
        true
    }
}
struct MockAfc;
impl AfcAccess for MockAfc {
    fn list(&mut self, _: &str) -> device::Result<Vec<String>> {
        Ok(Vec::new())
    }
    fn read(&mut self, _: &str, _: usize) -> device::Result<Vec<u8>> {
        unreachable!()
    }
    fn write(&mut self, _: &str, _: &[u8]) -> device::Result<()> {
        unreachable!()
    }
    fn mkdir(&mut self, _: &str) -> device::Result<()> {
        unreachable!()
    }
    fn remove(&mut self, _: &str) -> device::Result<()> {
        unreachable!()
    }
    fn tls(&self) -> bool {
        false
    }
}
fn selected() -> Device {
    Device {
        udid: "synthetic-not-a-device".into(),
        transport: Transport::Wifi,
    }
}
impl DeviceProvider for MockLockdown {
    type Service = MockService;
    type Afc = MockAfc;
    fn list_devices(&self) -> device::Result<Vec<Device>> {
        Ok(vec![selected()])
    }
    fn pairing(&self, d: &Device) -> device::Result<DeviceInfo> {
        if let Some(kind) = self.pairing_error {
            return Err(Error::new(kind, "mock_start_session"));
        }
        Ok(DeviceInfo {
            ios_version: "synthetic".into(),
            pairing: "paired_session_verified".into(),
            transport: d.transport,
        })
    }
    fn start_service(&self, d: &Device, _: &str) -> device::Result<MockService> {
        assert_eq!(d.transport, Transport::Wifi);
        self.starts.set(self.starts.get() + 1);
        if let Some(kind) = self.service_error {
            return Err(Error::new(kind, "mock_start_service"));
        }
        Ok(MockService(self.disconnected))
    }
    fn afc(&self, _: &Device) -> device::Result<MockAfc> {
        Ok(MockAfc)
    }
}
struct Bridge(MockService);
impl ReceiveTransport for Bridge {
    fn receive(&mut self, b: &mut [u8], t: u32) -> io::Result<usize> {
        self.0.receive(b, t)
    }
}
#[test]
fn trust_failure_prevents_service_start() {
    for kind in [
        ErrorKind::NotPaired,
        ErrorKind::TrustPending,
        ErrorKind::TrustDenied,
        ErrorKind::Locked,
    ] {
        let provider = MockLockdown {
            pairing_error: Some(kind),
            service_error: None,
            starts: Cell::new(0),
            disconnected: false,
        };
        assert!(device::open_verified_service(&provider, &selected(), "com.apple.atc").is_err());
        assert_eq!(provider.starts.get(), 0);
    }
}
#[test]
fn service_refusal_retains_stage_and_code() {
    let provider = MockLockdown {
        pairing_error: None,
        service_error: Some(ErrorKind::ServiceDenied),
        starts: Cell::new(0),
        disconnected: false,
    };
    let result = device::open_verified_service(&provider, &selected(), "com.apple.atc");
    let Err(error) = result else {
        panic!("expected refusal")
    };
    assert_eq!(error.kind, ErrorKind::ServiceDenied);
    assert_eq!(error.operation, "mock_start_service");
}
#[test]
fn mock_lockdown_and_tls_service_timeout_or_disconnect() {
    for disconnected in [false, true] {
        let provider = MockLockdown {
            pairing_error: None,
            service_error: None,
            starts: Cell::new(0),
            disconnected,
        };
        let (_, service) =
            device::open_verified_service(&provider, &selected(), "com.apple.atc").unwrap();
        assert!(service.tls());
        let report = PassiveClient {
            transport: Bridge(service),
        }
        .smoke(Duration::from_millis(1), &|| false);
        assert_eq!(
            report.outcome,
            if disconnected {
                "disconnected"
            } else {
                "receive_timeout"
            }
        );
        assert!(!report.first_message_validated);
    }
}
