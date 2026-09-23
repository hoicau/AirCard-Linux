//! Private, per-device operation folders. Only confirmed operations create directories.
use std::{
    os::unix::fs::{DirBuilderExt, MetadataExt},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

pub struct Storage {
    root: PathBuf,
}
#[derive(Clone)]
pub struct Paths {
    pub backup: PathBuf,
    pub recovery: PathBuf,
}
impl Storage {
    #[cfg(test)]
    pub fn at(root: PathBuf) -> Self {
        Self { root }
    }
    pub fn default() -> Result<Self, String> {
        let token = cli::sync_token::default_path()?;
        Ok(Self {
            root: token.parent().unwrap().to_path_buf(),
        })
    }
    fn device_dir(&self, udid: &str) -> PathBuf {
        self.root
            .join("operations")
            .join(aircard_core::sha256(udid.as_bytes()))
    }
    pub fn fresh(&self, udid: &str) -> Paths {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let directory = self.device_dir(udid).join(format!(
            "{time:032}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        Paths {
            backup: directory.join("backup.json"),
            recovery: directory.join("recovery"),
        }
    }
    pub fn recent(&self, udid: &str) -> (Option<PathBuf>, Option<PathBuf>) {
        let device = self.device_dir(udid);
        if [&self.root, &self.root.join("operations"), &device]
            .iter()
            .any(|p| !private_dir(p))
        {
            return (None, None);
        }
        let Ok(entries) = std::fs::read_dir(device) else {
            return (None, None);
        };
        let mut directories: Vec<_> = entries
            .take(10_000)
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| private_dir(p))
            .collect();
        directories.sort_unstable();
        let mut backup = None;
        let mut recovery = None;
        for directory in directories.into_iter().rev() {
            let candidate = directory.join("backup.json");
            if backup.is_none() && candidate.symlink_metadata().is_ok_and(|m| m.is_file()) {
                backup = Some(candidate);
            }
            let candidate = directory.join("recovery");
            if recovery.is_none() && private_dir(&candidate) {
                recovery = Some(candidate);
            }
            if backup.is_some() && recovery.is_some() {
                break;
            }
        }
        (backup, recovery)
    }
    pub fn prepare(&self, udid: &str, paths: &Paths) -> Result<(), String> {
        let device = self.device_dir(udid);
        let directory = paths
            .backup
            .parent()
            .ok_or("Invalid automatic backup path.")?;
        if directory.parent() != Some(device.as_path())
            || paths.recovery != directory.join("recovery")
        {
            return Err("Automatic save locations changed. Review the operation again.".into());
        }
        // The user data base may already be public; AirCard and its descendants must be private.
        if let Some(base) = self.root.parent() {
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(base)
                .map_err(
                    |_| "Cannot create the local data directory. Choose custom save locations.",
                )?;
        }
        for path in [&self.root, &self.root.join("operations"), &device] {
            if let Err(e) = std::fs::DirBuilder::new().mode(0o700).create(path)
                && e.kind() != std::io::ErrorKind::AlreadyExists
            {
                return Err("Cannot create automatic save locations. Choose custom locations under Advanced save locations.".into());
            }
            if !private_dir(path) {
                return Err("AirCard save directories must be owned by you, have permissions 0700 and not be symbolic links.".into());
            }
        }
        // A fresh container reserves this operation without creating the CLI's journal directory.
        cli::local::create_private_directory(directory)
            .map_err(|_| "Cannot reserve a new save location. Review the operation again.".into())
    }
}
fn private_dir(path: &Path) -> bool {
    path.symlink_metadata().is_ok_and(|m| {
        m.is_dir() && m.mode() & 0o077 == 0 && m.uid() == nix::unistd::getuid().as_raw()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_are_unique_private_and_rediscovered_without_crossing_devices() {
        let root = std::env::temp_dir().join(format!("aircard-storage-{}", std::process::id()));
        struct Clean(PathBuf);
        impl Drop for Clean {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _clean = Clean(root.clone());
        let storage = Storage { root };
        let first = storage.fresh("phone-a");
        let next = storage.fresh("phone-a");
        assert_ne!(first.backup, next.backup);
        assert!(!storage.root.exists(), "Planning must not create files");
        storage.prepare("phone-a", &first).unwrap();
        assert!(!first.backup.exists() && !first.recovery.exists());
        cli::local::write_new(&first.backup, b"synthetic backup").unwrap();
        cli::local::create_private_directory(&first.recovery).unwrap();
        assert!(
            storage.prepare("phone-a", &first).is_err(),
            "Never reuse an operation"
        );
        storage.prepare("phone-a", &next).unwrap();
        let reopened = Storage {
            root: storage.root.clone(),
        };
        assert_eq!(
            reopened.recent("phone-a"),
            (Some(first.backup.clone()), Some(first.recovery.clone()))
        );
        assert_eq!(reopened.recent("phone-b"), (None, None));
        std::fs::remove_dir(&first.recovery).unwrap();
        assert_eq!(reopened.recent("phone-a"), (Some(first.backup), None));
        std::os::unix::fs::symlink(&first.recovery, &next.recovery).unwrap();
        assert_eq!(reopened.recent("phone-a").1, None);
    }
}
