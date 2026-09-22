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
        .map_err(|_| Error::new(ErrorKind::InvalidInput, "local_input_open"))?;
    let info = file
        .metadata()
        .map_err(|_| Error::new(ErrorKind::InvalidInput, "local_input_stat"))?;
    if !info.is_file()
        || info.len() > limit as u64
        || (private && info.permissions().mode() & 0o077 != 0)
    {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "local_input_type_size_or_permissions",
        ));
    }
    let mut data = Vec::new();
    Read::by_ref(&mut file)
        .take((limit + 1) as u64)
        .read_to_end(&mut data)
        .map_err(|_| Error::new(ErrorKind::Native, "local_input_read"))?;
    if data.len() > limit {
        return Err(Error::new(ErrorKind::InvalidInput, "local_input_size"));
    }
    Ok(data)
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
        let bytes = read(p, 84, true)?;
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
