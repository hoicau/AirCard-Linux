//! Private operation folders grouped by device and card. Only confirmed operations create directories.
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
    card_key: Option<String>,
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
    fn scope_dir(&self, udid: &str, card_key: Option<&str>) -> PathBuf {
        match card_key {
            Some(key) => self.device_dir(udid).join("cards").join(key),
            None => self.device_dir(udid).join("restores"),
        }
    }
    pub fn fresh(&self, udid: &str, card: Option<&str>) -> Paths {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let card_key = card.map(|hash| aircard_core::sha256(hash.as_bytes()));
        let directory = self.scope_dir(udid, card_key.as_deref()).join(format!(
            "{time:032}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        Paths {
            backup: directory.join("backup.json"),
            recovery: directory.join("recovery"),
            card_key,
        }
    }
    pub fn recent(&self, udid: &str, card: Option<&str>) -> (Option<PathBuf>, Option<PathBuf>) {
        let device = self.device_dir(udid);
        if [&self.root, &self.root.join("operations"), &device]
            .iter()
            .any(|p| !private_dir(p))
        {
            return (None, None);
        }
        let backup = card.and_then(|hash| {
            let scope = self.scope_dir(udid, Some(&aircard_core::sha256(hash.as_bytes())));
            if !private_dir(&device.join("cards")) {
                return None;
            }
            let mut directories = child_dirs(&scope);
            directories.sort_unstable();
            directories
                .into_iter()
                .rev()
                .map(|d| d.join("backup.json"))
                .find(|p| p.symlink_metadata().is_ok_and(|m| m.is_file()))
        });
        // Recovery is device-wide: another card's unfinished sync must still be visible.
        // Direct children retain recovery discovery for the previous directory layout.
        let mut directories: Vec<_> = child_dirs(&device)
            .into_iter()
            .chain(child_dirs(&device.join("restores")))
            .chain(
                child_dirs(&device.join("cards"))
                    .into_iter()
                    .flat_map(|d| child_dirs(&d)),
            )
            .take(10_000)
            .collect();
        directories.sort_unstable_by(|a, b| a.file_name().cmp(&b.file_name()));
        let mut recovery = None;
        for directory in directories.into_iter().rev() {
            let candidate = directory.join("recovery");
            if private_dir(&candidate) {
                recovery = Some(candidate);
                break;
            }
        }
        (backup, recovery)
    }
    pub fn prepare(&self, udid: &str, paths: &Paths) -> Result<(), String> {
        let device = self.device_dir(udid);
        let scope = self.scope_dir(udid, paths.card_key.as_deref());
        let directory = paths
            .backup
            .parent()
            .ok_or("Invalid automatic backup path.")?;
        if directory.parent() != Some(scope.as_path())
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
        let mut parents = vec![
            self.root.clone(),
            self.root.join("operations"),
            device.clone(),
        ];
        if paths.card_key.is_some() {
            parents.push(device.join("cards"));
        }
        parents.push(scope);
        for path in &parents {
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
fn child_dirs(path: &Path) -> Vec<PathBuf> {
    if !private_dir(path) {
        return vec![];
    }
    std::fs::read_dir(path)
        .into_iter()
        .flatten()
        .take(10_000)
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| private_dir(p))
        .collect()
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
    fn backups_are_isolated_by_card_and_recovery_remains_device_wide() {
        let root = std::env::temp_dir().join(format!("aircard-storage-{}", std::process::id()));
        struct Clean(PathBuf);
        impl Drop for Clean {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _clean = Clean(root.clone());
        let storage = Storage { root };
        let first = storage.fresh("phone-a", Some("card-a"));
        let next = storage.fresh("phone-a", Some("card-a"));
        let other = storage.fresh("phone-a", Some("card-b"));
        assert_ne!(first.backup, next.backup);
        assert_eq!(
            first.backup.parent().unwrap().parent(),
            next.backup.parent().unwrap().parent()
        );
        assert_ne!(
            first.backup.parent().unwrap().parent(),
            other.backup.parent().unwrap().parent()
        );
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
        storage.prepare("phone-a", &other).unwrap();
        cli::local::write_new(&other.backup, b"another card backup").unwrap();
        let reopened = Storage {
            root: storage.root.clone(),
        };
        assert_eq!(
            reopened.recent("phone-a", Some("card-a")),
            (Some(first.backup.clone()), Some(first.recovery.clone()))
        );
        assert_eq!(reopened.recent("phone-b", Some("card-a")), (None, None));
        assert_eq!(
            reopened.recent("phone-a", Some("card-b")),
            (Some(other.backup), Some(first.recovery.clone()))
        );
        assert_eq!(
            reopened.recent("phone-a", None),
            (None, Some(first.recovery.clone()))
        );
        assert_eq!(
            reopened.recent("phone-a", Some("new-card")),
            (None, Some(first.recovery.clone()))
        );
        std::fs::remove_dir(&first.recovery).unwrap();
        assert_eq!(
            reopened.recent("phone-a", Some("card-a")),
            (Some(first.backup.clone()), None)
        );
        std::os::unix::fs::symlink(&first.recovery, &next.recovery).unwrap();
        assert_eq!(reopened.recent("phone-a", Some("card-a")).1, None);
        let restore = storage.fresh("phone-a", None);
        storage.prepare("phone-a", &restore).unwrap();
        cli::local::create_private_directory(&restore.recovery).unwrap();
        assert_eq!(
            reopened.recent("phone-a", Some("card-a")).1,
            Some(restore.recovery.clone())
        );
        std::fs::remove_dir(&restore.recovery).unwrap();
        let legacy = storage.device_dir("phone-a").join("000-legacy-operation");
        cli::local::create_private_directory(&legacy).unwrap();
        cli::local::write_new(&legacy.join("backup.json"), b"legacy backup kept in place").unwrap();
        cli::local::create_private_directory(&legacy.join("recovery")).unwrap();
        assert_eq!(
            reopened.recent("phone-a", None),
            (None, Some(legacy.join("recovery")))
        );
        assert!(legacy.join("backup.json").is_file());
        cli::local::write_new(&next.backup, b"newer backup of card a").unwrap();
        assert_eq!(
            reopened.recent("phone-a", Some("card-a")).0,
            Some(next.backup)
        );
        assert!(
            first.backup.is_file(),
            "Repeated Apply must keep the earlier backup"
        );
    }
}
