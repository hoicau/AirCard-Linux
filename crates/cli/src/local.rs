//! Local file policy lives at the application boundary, never in core/airtraffic.
use device::{Error, ErrorKind, Result};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::Path,
};

pub fn read(path: &Path, limit: usize, private: bool) -> Result<Vec<u8>> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
        .open(path)
        .map_err(|error| {
            let (operation, hint) = if error.raw_os_error() == Some(nix::libc::ELOOP) {
                ("local_input_symlink", "Select the actual file; symbolic links are not accepted.")
            } else {
                match error.kind() {
                    std::io::ErrorKind::NotFound => ("local_input_not_found", "The input file does not exist. Select an existing file."),
                    std::io::ErrorKind::PermissionDenied => ("local_input_permission_denied", "The input file cannot be opened. Check read permission and access to its parent directories."),
                    _ => ("local_input_open", "The input file cannot be opened. Check its path and access permissions."),
                }
            };
            input_error(operation, hint)
        })?;
    let info = file
        .metadata()
        .map_err(|_| Error::new(ErrorKind::InvalidInput, "local_input_stat"))?;
    if !info.is_file() {
        return Err(input_error(
            "local_input_not_file",
            "Select a regular file, not a directory or special file.",
        ));
    }
    if info.len() > limit as u64 {
        return Err(input_error(
            "local_input_size",
            "The input file exceeds this operation's size limit.",
        ));
    }
    if private && info.permissions().mode() & 0o077 != 0 {
        return Err(input_error(
            "local_input_permissions",
            "The private input file allows group or other-user access. Set its permissions to 0600 (chmod 600).",
        ));
    }
    let mut data = Vec::new();
    Read::by_ref(&mut file)
        .take((limit + 1) as u64)
        .read_to_end(&mut data)
        .map_err(|_| Error::new(ErrorKind::Native, "local_input_read"))?;
    if data.len() > limit {
        return Err(input_error(
            "local_input_size",
            "The input file exceeds this operation's size limit.",
        ));
    }
    Ok(data)
}
fn input_error(operation: &str, hint: &str) -> Error {
    let mut error = Error::new(ErrorKind::InvalidInput, operation);
    error.hint = hint.into();
    error
}
/// Preserve the failure reason while identifying the input without exposing its path.
pub fn read_context(path: &Path, limit: usize, private: bool, context: &str) -> Result<Vec<u8>> {
    read(path, limit, private).map_err(|mut error| {
        error.operation = error.operation.replacen("local_input", context, 1);
        error
    })
}
/// Persist the parent directory entry before any device mutation can rely on this journal.
pub fn create_private_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(path)
        .map_err(|_| Error::new(ErrorKind::Conflict, "local_directory_create_new"))?;
    if File::open(parent(path)).and_then(|p| p.sync_all()).is_err() {
        let _ = std::fs::remove_dir(path);
        return Err(Error::new(ErrorKind::Native, "local_directory_sync"));
    }
    Ok(())
}
pub fn remove_empty_directory(path: &Path) -> Result<()> {
    std::fs::remove_dir(path)
        .and_then(|()| File::open(parent(path))?.sync_all())
        .map_err(|_| Error::new(ErrorKind::Native, "local_directory_cleanup"))
}
pub fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(path)
        .map_err(|_| Error::new(ErrorKind::Conflict, "local_output_create_new"))?;
    let result = (|| -> std::io::Result<()> {
        file.write_all(bytes)?;
        file.sync_all()?;
        File::open(parent(path))?.sync_all()
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(path);
        return Err(Error::new(ErrorKind::Native, "local_output_sync"));
    }
    Ok(())
}
fn parent(path: &Path) -> &Path {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}
pub fn remove(path: &Path) -> Result<()> {
    std::fs::remove_file(path)
        .and_then(|()| File::open(parent(path))?.sync_all())
        .map_err(|_| Error::new(ErrorKind::Native, "local_cleanup"))
}
pub fn token(path: Option<&Path>) -> Result<Option<Vec<u8>>> {
    path.map(|p| {
        let bytes = read_context(p, 84, true, "grappa_token_input")?;
        if bytes.len() != 84 {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "grappa_token_must_be_84_bytes",
            ));
        }
        Ok(bytes)
    })
    .transpose()
}

/// All instances serialize operations touching Books. The empty lock file remains so
/// unlink/recreate cannot split contenders across different inodes; the kernel releases locks.
pub struct DeviceLease {
    _lock: nix::fcntl::Flock<File>,
}
impl DeviceLease {
    pub fn acquire() -> Result<Self> {
        let uid = nix::unistd::getuid().as_raw();
        let base = std::env::var_os("XDG_RUNTIME_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        Self::at(&base.join(format!("aircard-{uid}")))
    }
    fn at(directory: &Path) -> Result<Self> {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt};
        if let Err(e) = std::fs::DirBuilder::new().mode(0o700).create(directory)
            && e.kind() != std::io::ErrorKind::AlreadyExists
        {
            return Err(Error::new(ErrorKind::Native, "device_lock_directory"));
        }
        let meta = std::fs::symlink_metadata(directory)
            .map_err(|_| Error::new(ErrorKind::Native, "device_lock_stat"))?;
        let uid = nix::unistd::getuid().as_raw();
        if !meta.is_dir() || meta.uid() != uid || meta.mode() & 0o077 != 0 {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "device_lock_directory_permissions",
            ));
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
            .open(directory.join("books.lock"))
            .map_err(|_| Error::new(ErrorKind::InvalidInput, "device_lock_open"))?;
        let meta = file
            .metadata()
            .map_err(|_| Error::new(ErrorKind::Native, "device_lock_stat"))?;
        if !meta.is_file() || meta.uid() != uid || meta.mode() & 0o077 != 0 || meta.nlink() != 1 {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "device_lock_file_permissions",
            ));
        }
        let lock = nix::fcntl::Flock::lock(file, nix::fcntl::FlockArg::LockExclusiveNonblock)
            .map_err(|_| Error::new(ErrorKind::Conflict, "another_books_operation_is_running"))?;
        Ok(Self { _lock: lock })
    }
}
/// Durable replacement for an existing private journal. The temporary is on the same filesystem.
pub fn replace_journal(path: &Path, expected: &[u8], bytes: &[u8]) -> Result<()> {
    if read(path, expected.len(), true)? != expected {
        return Err(Error::new(ErrorKind::Conflict, "journal_changed"));
    }
    let id = std::fs::read_to_string("/proc/sys/kernel/random/uuid")
        .map_err(|_| Error::new(ErrorKind::Native, "journal_random_id"))?;
    let temporary = parent(path).join(format!(".aircard-journal-{}.tmp", id.trim()));
    write_new(&temporary, bytes)?;
    let result =
        std::fs::rename(&temporary, path).and_then(|()| File::open(parent(path))?.sync_all());
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
        return Err(Error::new(ErrorKind::Native, "journal_replace_sync"));
    }
    Ok(())
}
#[cfg(test)]
mod lease_tests {
    use super::*;
    #[test]
    fn concurrent_operation_fails_then_releases_and_rejects_symlink() {
        let directory = std::env::temp_dir().join(format!(
            "aircard-lock-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        struct Clean(std::path::PathBuf);
        impl Drop for Clean {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _clean = Clean(directory.clone());
        let lease = DeviceLease::at(&directory).unwrap();
        assert!(DeviceLease::at(&directory).is_err());
        drop(lease);
        drop(DeviceLease::at(&directory).unwrap());
        std::fs::remove_file(directory.join("books.lock")).unwrap();
        std::os::unix::fs::symlink("missing", directory.join("books.lock")).unwrap();
        assert!(DeviceLease::at(&directory).is_err());
    }
}
