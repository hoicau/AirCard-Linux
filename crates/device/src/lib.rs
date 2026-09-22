#![forbid(unsafe_code)]
pub mod books;
use aircard_linux_adapter as native;
pub mod self_test;
use serde::Serialize;
use std::io;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    Usb,
    Wifi,
}
impl Transport {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Usb => "usb",
            Self::Wifi => "wifi",
        }
    }
    fn native(self) -> i32 {
        match self {
            Self::Usb => 1,
            Self::Wifi => 2,
        }
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct Device {
    pub udid: String,
    pub transport: Transport,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    Cancelled,
    NoDevice,
    AmbiguousDevice,
    NotPaired,
    TrustDenied,
    TrustPending,
    Locked,
    ServiceDenied,
    Disconnected,
    Timeout,
    Tls,
    UsbmuxUnavailable,
    InvalidInput,
    NotFound,
    Native,
    Conflict,
}
#[derive(Debug, Error, Serialize)]
#[error("{kind:?} at {operation} (native domain={domain}, code={code}): {hint}")]
pub struct Error {
    pub kind: ErrorKind,
    pub operation: String,
    pub domain: i32,
    pub code: i32,
    pub hint: String,
}
impl Error {
    pub fn new(kind: ErrorKind, operation: &str) -> Self {
        let hint = match kind {
            ErrorKind::Cancelled => "Operation cancelled; inspect cleanup status before retrying.",
            ErrorKind::NoDevice => {
                "Connect the paired device; Wi-Fi requires network discovery and Wi-Fi sync."
            }
            ErrorKind::AmbiguousDevice => {
                "Select exactly one device and route with --udid and --transport."
            }
            ErrorKind::NotPaired => {
                "An existing valid host pairing is required. Pair manually with official trust confirmation."
            }
            ErrorKind::TrustDenied => {
                "The device denied this host. Confirm trust manually on your own device."
            }
            ErrorKind::TrustPending => "Complete the trust prompt manually on the device.",
            ErrorKind::Locked => "Unlock the device and retry.",
            ErrorKind::ServiceDenied => {
                "The device refused this service; check iOS support, lock state and active sync clients."
            }
            ErrorKind::Disconnected => {
                "The selected transport disconnected; reconnect and start a new read-only session."
            }
            ErrorKind::Timeout => "The operation exceeded its deadline.",
            ErrorKind::Tls => "The service TLS handshake failed; check the existing pairing.",
            ErrorKind::UsbmuxUnavailable => {
                "Check usbmuxd service/socket access and network discovery."
            }
            ErrorKind::InvalidInput => "Input violates the configured path or size limits.",
            ErrorKind::NotFound => "The requested AFC path does not exist.",
            ErrorKind::Conflict => {
                "Device state changed or target already exists; no automatic overwrite is allowed."
            }
            ErrorKind::Native => {
                "Native operation failed; retain only the domain/code and operation for diagnosis."
            }
        };
        Self {
            kind,
            operation: operation.into(),
            domain: 0,
            code: 0,
            hint: hint.into(),
        }
    }
    fn native(e: native::Error, operation: &str) -> Self {
        let kind = match (e.domain, e.code) {
            (1, -3) => ErrorKind::NoDevice,
            (1, -6) | (2, -5) | (3, -4) => ErrorKind::Tls,
            (1, -7) | (2, -7) | (3, -7) | (4, 12) => ErrorKind::Timeout,
            (2, -17 | -35) => ErrorKind::Locked,
            (2, -18) => ErrorKind::TrustDenied,
            (2, -19) => ErrorKind::TrustPending,
            (2, -2 | -4 | -20 | -21 | -29 | -31) => ErrorKind::NotPaired,
            (2, -26 | -27 | -28 | -34 | -36) | (3, -5) | (4, 10) => ErrorKind::ServiceDenied,
            (1, -2 | -5) | (2, -8) | (3, -3) | (4, 11 | 30) => ErrorKind::Disconnected,
            (4, 8) => ErrorKind::NotFound,
            (5, _) => ErrorKind::UsbmuxUnavailable,
            (6, _) => ErrorKind::InvalidInput,
            _ => ErrorKind::Native,
        };
        Self {
            domain: e.domain,
            code: e.code,
            ..Self::new(kind, operation)
        }
    }
}
pub type Result<T> = std::result::Result<T, Error>;
pub fn select(
    devices: &[Device],
    udid: Option<&str>,
    transport: Option<Transport>,
) -> Result<Device> {
    let selected: Vec<_> = devices
        .iter()
        .filter(|d| {
            udid.is_none_or(|id| d.udid == id) && transport.is_none_or(|t| d.transport == t)
        })
        .collect();
    match selected.as_slice() {
        [] => Err(Error::new(ErrorKind::NoDevice, "select")),
        [device] => Ok((*device).clone()),
        _ => Err(Error::new(ErrorKind::AmbiguousDevice, "select")),
    }
}
#[derive(Debug, Serialize)]
pub struct DeviceInfo {
    pub ios_version: String,
    pub pairing: String,
    pub transport: Transport,
}
pub trait ServiceTransport {
    fn receive(&mut self, buf: &mut [u8], timeout_ms: u32) -> io::Result<usize>;
    fn tls(&self) -> bool;
}
/// A writable service connection; receive-only mocks/providers need not implement it.
pub trait DuplexServiceTransport: ServiceTransport {
    fn send(&mut self, buf: &[u8], timeout_ms: u32) -> io::Result<usize>;
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum FileKind {
    File,
    Directory,
    Symlink,
    Other,
}
#[derive(Debug, Clone, Copy, Serialize)]
pub struct FileInfo {
    pub kind: FileKind,
    pub size: u64,
}
pub trait AfcAccess {
    fn stat(&mut self, _path: &str) -> Result<FileInfo> {
        Err(Error::new(ErrorKind::InvalidInput, "stat_unsupported"))
    }
    fn rename(&mut self, _source: &str, _target: &str) -> Result<()> {
        Err(Error::new(ErrorKind::InvalidInput, "rename_unsupported"))
    }

    fn list(&mut self, path: &str) -> Result<Vec<String>>;
    fn read(&mut self, path: &str, limit: usize) -> Result<Vec<u8>>;
    fn mkdir(&mut self, path: &str) -> Result<()>;
    fn write(&mut self, path: &str, data: &[u8]) -> Result<()>;
    fn remove(&mut self, path: &str) -> Result<()>;
    fn tls(&self) -> bool;
}
pub trait DeviceProvider {
    type Service: ServiceTransport;
    type Afc: AfcAccess;
    fn list_devices(&self) -> Result<Vec<Device>>;
    fn pairing(&self, device: &Device) -> Result<DeviceInfo>;
    fn start_service(&self, device: &Device, service: &str) -> Result<Self::Service>;
    fn syslog(&self, device: &Device) -> Result<Self::Service> {
        self.start_service(device, "com.apple.syslog_relay")
    }
    fn afc(&self, device: &Device) -> Result<Self::Afc>;
}
/// Verify existing trust before requesting a service; usable with mock backends.
pub fn open_verified_service<P: DeviceProvider>(
    provider: &P,
    selected: &Device,
    name: &str,
) -> Result<(DeviceInfo, P::Service)> {
    let info = provider.pairing(selected)?;
    let service = provider.start_service(selected, name)?;
    Ok((info, service))
}

pub struct LinuxDeviceProvider;
impl LinuxDeviceProvider {
    fn session(&self, device: &Device) -> Result<native::Session> {
        native::Session::open(&device.udid, device.transport.native())
            .map_err(|e| Error::native(e, "existing_pair_session"))
    }
}
impl DeviceProvider for LinuxDeviceProvider {
    type Service = LinuxService;
    type Afc = LinuxAfc;
    fn list_devices(&self) -> Result<Vec<Device>> {
        native::list()
            .map_err(|e| Error::native(e, "enumerate"))?
            .into_iter()
            .map(|(udid, transport)| {
                let transport = match transport {
                    1 => Transport::Usb,
                    2 => Transport::Wifi,
                    _ => return Err(Error::new(ErrorKind::Native, "unknown_transport")),
                };
                Ok(Device { udid, transport })
            })
            .collect()
    }
    fn pairing(&self, device: &Device) -> Result<DeviceInfo> {
        let ios_version = self
            .session(device)?
            .version()
            .map_err(|e| Error::native(e, "product_version"))?;
        Ok(DeviceInfo {
            ios_version,
            pairing: "paired_session_verified".into(),
            transport: device.transport,
        })
    }
    fn start_service(&self, device: &Device, service: &str) -> Result<LinuxService> {
        if !["com.apple.atc", "com.apple.syslog_relay"].contains(&service) {
            return Err(Error::new(ErrorKind::InvalidInput, "service_allowlist"));
        }
        self.session(device)?
            .service(service)
            .map(LinuxService)
            .map_err(|e| Error::native(e, "start_service"))
    }
    fn afc(&self, device: &Device) -> Result<LinuxAfc> {
        self.session(device)?
            .afc()
            .map(|inner| LinuxAfc(inner, false))
            .map_err(|e| Error::native(e, "start_afc"))
    }
}
pub struct LinuxService(native::Service);
impl ServiceTransport for LinuxService {
    fn receive(&mut self, buf: &mut [u8], timeout_ms: u32) -> io::Result<usize> {
        self.0.receive(buf, timeout_ms).map_err(|e| {
            let error = Error::native(e, "receive");
            let kind = match error.kind {
                ErrorKind::Timeout => io::ErrorKind::TimedOut,
                ErrorKind::Disconnected => io::ErrorKind::ConnectionReset,
                _ => io::ErrorKind::Other,
            };
            io::Error::new(kind, error)
        })
    }
    fn tls(&self) -> bool {
        self.0.tls
    }
}
impl DuplexServiceTransport for LinuxService {
    fn send(&mut self, buf: &[u8], timeout_ms: u32) -> io::Result<usize> {
        self.0.send(buf, timeout_ms).map_err(|e| {
            let error = Error::native(e, "send");
            let kind = match error.kind {
                ErrorKind::Timeout => io::ErrorKind::TimedOut,
                ErrorKind::Disconnected => io::ErrorKind::ConnectionReset,
                _ => io::ErrorKind::Other,
            };
            io::Error::new(kind, error)
        })
    }
}
pub struct LinuxAfc(native::Afc, bool);
impl LinuxAfc {
    /// Capability used only by an explicitly applied, snapshotted Books transaction.
    pub fn with_books_write_scope(mut self) -> Self {
        self.1 = true;
        self
    }
    fn writable(&self, path: &str) -> Result<()> {
        self::path(path)?;
        if self.1
            && (path == "Books"
                || path.starts_with("Books/")
                || path == "Airlock"
                || path == "Airlock/Book"
                || path.strip_prefix("Airlock/Book/").is_some_and(|leaf| {
                    leaf.starts_with("AirCard-Linux-Test-") && aircard_core::safe_leaf(leaf).is_ok()
                }))
        {
            return Ok(());
        }
        writable(path)
    }

    fn reject_symlinks(&mut self, value: &str, include_leaf: bool) -> Result<()> {
        path(value)?;
        if value == "." {
            return Ok(());
        }
        let parts: Vec<_> = value.split('/').collect();
        let count = if include_leaf {
            parts.len()
        } else {
            parts.len() - 1
        };
        for index in 0..count {
            let prefix = parts[..=index].join("/");
            let info = self
                .0
                .info(&prefix)
                .map_err(|e| Error::native(e, "afc_path_stat"))?;
            let kind = info
                .as_chunks::<2>()
                .0
                .iter()
                .find(|p| p[0] == "st_ifmt")
                .map(|p| p[1].as_str());
            if kind == Some("S_IFLNK") || (index + 1 < parts.len() && kind != Some("S_IFDIR")) {
                return Err(Error::new(
                    ErrorKind::InvalidInput,
                    "afc_symlink_or_non_directory",
                ));
            }
        }
        Ok(())
    }
}

fn path(path: &str) -> Result<()> {
    if path == "." {
        return Ok(());
    }
    aircard_core::safe_relative_path(path)
        .map_err(|_| Error::new(ErrorKind::InvalidInput, "afc_path"))
}
fn writable(path: &str) -> Result<()> {
    self::path(path)?;
    if !path.starts_with("AirCard-Linux-PoC-") {
        return Err(Error::new(ErrorKind::InvalidInput, "afc_scratch_only"));
    }
    Ok(())
}
impl AfcAccess for LinuxAfc {
    fn stat(&mut self, value: &str) -> Result<FileInfo> {
        self.reject_symlinks(value, false)?;
        let info = self
            .0
            .info(value)
            .map_err(|e| Error::native(e, "afc_stat"))?;
        let get = |key: &str| {
            info.as_chunks::<2>()
                .0
                .iter()
                .find(|p| p[0] == key)
                .map(|p| p[1].as_str())
        };
        let kind = match get("st_ifmt") {
            Some("S_IFREG") => FileKind::File,
            Some("S_IFDIR") => FileKind::Directory,
            Some("S_IFLNK") => FileKind::Symlink,
            _ => FileKind::Other,
        };
        let size = get("st_size")
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| Error::new(ErrorKind::Native, "afc_invalid_stat"))?;
        Ok(FileInfo { kind, size })
    }
    fn rename(&mut self, source: &str, target: &str) -> Result<()> {
        self.writable(source)?;
        self.writable(target)?;
        self.reject_symlinks(source, true)?;
        self.reject_symlinks(target, false)?;
        match self.stat(target) {
            Ok(info) if info.kind != FileKind::File => {
                return Err(Error::new(
                    ErrorKind::InvalidInput,
                    "rename_target_not_regular",
                ));
            }
            Err(e) if e.kind != ErrorKind::NotFound => return Err(e),
            _ => {}
        }
        self.0
            .rename(source, target)
            .map_err(|e| Error::native(e, "afc_rename"))
    }

    fn list(&mut self, value: &str) -> Result<Vec<String>> {
        self.reject_symlinks(value, true)?;
        self.0.list(value).map_err(|e| Error::native(e, "afc_list"))
    }
    fn read(&mut self, value: &str, limit: usize) -> Result<Vec<u8>> {
        self.reject_symlinks(value, true)?;
        if limit > 16 * 1024 * 1024 {
            return Err(Error::new(ErrorKind::InvalidInput, "afc_read_limit"));
        }
        let info = self
            .0
            .info(value)
            .map_err(|e| Error::native(e, "afc_stat"))?;
        if !info
            .as_chunks::<2>()
            .0
            .iter()
            .any(|pair| pair[0] == "st_ifmt" && pair[1] == "S_IFREG")
        {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "afc_regular_file_required",
            ));
        }
        let mut file = self
            .0
            .open(value, false)
            .map_err(|e| Error::native(e, "afc_open_read"))?;
        let mut data = vec![0; limit + 1];
        let mut used = 0;
        while used < data.len() {
            let n = file
                .read(&mut data[used..])
                .map_err(|e| Error::native(e, "afc_read"))?;
            if n == 0 {
                break;
            }
            used += n;
        }
        file.close().map_err(|e| Error::native(e, "afc_close"))?;
        if used > limit {
            return Err(Error::new(ErrorKind::InvalidInput, "afc_read_limit"));
        }
        data.truncate(used);
        Ok(data)
    }
    fn mkdir(&mut self, value: &str) -> Result<()> {
        self.writable(value)?;
        self.reject_symlinks(value, false)?;
        self.0
            .mkdir(value)
            .map_err(|e| Error::native(e, "afc_mkdir"))
    }
    fn write(&mut self, value: &str, data: &[u8]) -> Result<()> {
        self.writable(value)?;
        self.reject_symlinks(value, false)?;
        match self.0.info(value) {
            Err(e) if e.domain == 4 && e.code == 8 => {}
            Err(e) => return Err(Error::native(e, "afc_write_stat")),
            Ok(_) => return Err(Error::new(ErrorKind::InvalidInput, "afc_refuse_overwrite")),
        }
        if data.len() > 16 * 1024 * 1024 {
            return Err(Error::new(ErrorKind::InvalidInput, "afc_write_limit"));
        }
        let mut file = self
            .0
            .open(value, true)
            .map_err(|e| Error::native(e, "afc_open_write"))?;
        let mut offset = 0;
        while offset < data.len() {
            let n = file
                .write(&data[offset..])
                .map_err(|e| Error::native(e, "afc_write"))?;
            if n == 0 {
                return Err(Error::new(ErrorKind::Disconnected, "afc_write_zero"));
            }
            offset += n;
        }
        file.close().map_err(|e| Error::native(e, "afc_close"))
    }
    fn remove(&mut self, value: &str) -> Result<()> {
        self.writable(value)?;
        self.reject_symlinks(value, true)?;
        self.0
            .remove(value)
            .map_err(|e| Error::native(e, "afc_remove"))
    }
    fn tls(&self) -> bool {
        self.0.tls
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_selection_never_falls_back() {
        let devices = vec![
            Device {
                udid: "synthetic".into(),
                transport: Transport::Usb,
            },
            Device {
                udid: "synthetic".into(),
                transport: Transport::Wifi,
            },
        ];
        assert_eq!(
            select(&devices, None, None).unwrap_err().kind,
            ErrorKind::AmbiguousDevice
        );
        assert_eq!(
            select(&devices, Some("synthetic"), Some(Transport::Wifi))
                .unwrap()
                .transport,
            Transport::Wifi
        );
        assert_eq!(
            select(&devices[..1], None, Some(Transport::Wifi))
                .unwrap_err()
                .kind,
            ErrorKind::NoDevice
        );
        assert_eq!(
            select(&[], None, None).unwrap_err().kind,
            ErrorKind::NoDevice
        );
    }
    #[test]
    fn native_failure_classification() {
        for (domain, code, kind) in [
            (2, -17, ErrorKind::Locked),
            (2, -29, ErrorKind::NotPaired),
            (2, -18, ErrorKind::TrustDenied),
            (2, -19, ErrorKind::TrustPending),
            (2, -34, ErrorKind::ServiceDenied),
            (3, -7, ErrorKind::Timeout),
            (3, -3, ErrorKind::Disconnected),
            (5, -2, ErrorKind::UsbmuxUnavailable),
        ] {
            assert_eq!(
                Error::native(native::Error { domain, code }, "test").kind,
                kind
            );
        }
    }
}
