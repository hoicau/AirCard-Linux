//! Controlled AFC roundtrip and cleanup, injectable for hardware-free failure tests.
use crate::{AfcAccess, Error, ErrorKind, Result};
use serde::Serialize;
#[derive(Debug, Serialize)]
pub struct SelfTestReport {
    pub roundtrip_ok: bool,
    pub cleanup_ok: bool,
    pub scratch_path: String,
    pub operation_error: Option<Error>,
    pub cleanup_errors: Vec<Error>,
}
pub fn roundtrip(
    afc: &mut impl AfcAccess,
    root: &str,
    cancelled: &dyn Fn() -> bool,
) -> Result<SelfTestReport> {
    aircard_core::safe_leaf(root)
        .map_err(|_| Error::new(ErrorKind::InvalidInput, "scratch_path"))?;
    if !root.starts_with("AirCard-Linux-PoC-") || afc.list(".")?.iter().any(|s| s == root) {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "scratch_collision_or_prefix",
        ));
    }
    if cancelled() {
        return Err(Error::new(ErrorKind::Cancelled, "afc_self_test"));
    }
    afc.mkdir(root)?;
    let file = format!("{root}/roundtrip.txt");
    let result = (|| {
        if cancelled() {
            return Err(Error::new(ErrorKind::Cancelled, "afc_self_test"));
        }
        afc.write(&file, b"AirCard synthetic AFC test\n")?;
        let bytes = afc.read(&file, 1024)?;
        if bytes != b"AirCard synthetic AFC test\n" {
            return Err(Error::new(ErrorKind::Native, "afc_roundtrip_mismatch"));
        }
        Ok(())
    })();
    // Always attempt both cleanups. A partial write can exist even when write returned an error.
    let mut cleanup_errors = Vec::new();
    if let Err(e) = afc.remove(&file)
        && e.kind != ErrorKind::NotFound
    {
        cleanup_errors.push(e);
    }
    if let Err(e) = afc.remove(root)
        && e.kind != ErrorKind::NotFound
    {
        cleanup_errors.push(e);
    }
    Ok(SelfTestReport {
        roundtrip_ok: result.is_ok(),
        cleanup_ok: cleanup_errors.is_empty(),
        scratch_path: root.into(),
        operation_error: result.err(),
        cleanup_errors,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Mock {
        data: Vec<u8>,
        removed: Vec<String>,
        fail_write: bool,
        fail_remove: bool,
    }
    impl AfcAccess for Mock {
        fn list(&mut self, _: &str) -> Result<Vec<String>> {
            Ok(Vec::new())
        }
        fn mkdir(&mut self, _: &str) -> Result<()> {
            Ok(())
        }
        fn write(&mut self, _: &str, data: &[u8]) -> Result<()> {
            self.data = data.to_vec();
            if self.fail_write {
                Err(Error::new(ErrorKind::Disconnected, "write"))
            } else {
                Ok(())
            }
        }
        fn read(&mut self, _: &str, _: usize) -> Result<Vec<u8>> {
            Ok(self.data.clone())
        }
        fn remove(&mut self, path: &str) -> Result<()> {
            self.removed.push(path.into());
            if self.fail_remove {
                Err(Error::new(ErrorKind::Disconnected, "remove"))
            } else {
                Ok(())
            }
        }
        fn tls(&self) -> bool {
            false
        }
    }
    #[test]
    fn cleanup_runs_after_partial_write_failure() {
        let mut mock = Mock {
            fail_write: true,
            ..Mock::default()
        };
        let report = roundtrip(&mut mock, "AirCard-Linux-PoC-synthetic", &|| false).unwrap();
        assert!(!report.roundtrip_ok);
        assert!(report.cleanup_ok);
        assert_eq!(mock.removed.len(), 2);
    }
    #[test]
    fn cleanup_failure_is_preserved() {
        let mut mock = Mock {
            fail_remove: true,
            ..Mock::default()
        };
        let report = roundtrip(&mut mock, "AirCard-Linux-PoC-synthetic", &|| false).unwrap();
        assert!(report.roundtrip_ok);
        assert!(!report.cleanup_ok);
        assert_eq!(report.cleanup_errors.len(), 2);
    }
    #[test]
    fn cancelled_before_start_does_not_write() {
        let mut mock = Mock::default();
        assert!(roundtrip(&mut mock, "AirCard-Linux-PoC-synthetic", &|| true).is_err());
        assert!(mock.data.is_empty());
    }
}
