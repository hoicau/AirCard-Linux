//! Explicit, pinned public compatibility-token setup. Never logs downloaded material.
use std::{
    io::Read,
    os::unix::fs::{DirBuilderExt, MetadataExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

pub const SOURCE_URL: &str = "https://raw.githubusercontent.com/shinkuan/AirCard-Linux/7686fd21e3e598d2217c5e6da88229cd1bca4e07/crates/aircard-core/src/services/grappa.rs";
const SOURCE_SHA256: &str = "9240bc9ccea542993f9349d4ea6db30b7b57a2cc703d382802e122fda50a0787";
const LIMIT: usize = 1024 * 1024;

pub fn default_path() -> Result<PathBuf, String> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .map(|p| p.join(".local/share"))
        })
        .ok_or("Cannot locate your data directory. Choose an existing token with Browse.")?;
    Ok(base.join("aircard/token.bin"))
}

pub fn cached() -> Option<PathBuf> {
    let path = default_path().ok()?;
    crate::local::token(Some(&path)).ok()?;
    Some(path)
}

fn parse(source: &[u8], expected_sha256: &str) -> Result<Vec<u8>, String> {
    if source.len() > LIMIT || aircard_core::sha256(source) != expected_sha256 {
        return Err("Token source integrity check failed. No token was installed.".into());
    }
    let source = std::str::from_utf8(source).map_err(|_| "Invalid token source encoding.")?;
    let entries: Vec<_> = source
        .split('"')
        .enumerate()
        .filter(|(i, s)| i % 2 == 1 && s.len() == 168 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        .map(|(_, s)| s)
        .collect();
    if entries.len() != 10 {
        return Err("Unexpected token table. No token was installed.".into());
    }
    (0..168)
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&entries[0][i..i + 2], 16).map_err(|_| "Invalid token data.".into())
        })
        .collect()
}

fn download() -> Result<Vec<u8>, String> {
    let mut child = Command::new("curl")
        .args([
            "--disable",
            "--proto",
            "=https",
            "--tlsv1.2",
            "--fail",
            "--silent",
            "--connect-timeout",
            "10",
            "--max-time",
            "25",
            "--max-filesize",
            "1048576",
            SOURCE_URL,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "Cannot start curl. Install curl and ca-certificates, then retry.")?;
    let mut source = Vec::new();
    let read = child
        .stdout
        .take()
        .expect("piped stdout")
        .take((LIMIT + 1) as u64)
        .read_to_end(&mut source);
    if read.is_err() || source.len() > LIMIT {
        let _ = child.kill();
        let _ = child.wait();
        return Err("Token download exceeded its limit or could not be read. Retry setup.".into());
    }
    let status = child
        .wait()
        .map_err(|_| "Could not wait for token download.")?;
    if !status.success() {
        return Err("Could not download the sync token from GitHub. Check your connection and ca-certificates, then retry, or use Browse for an existing token.".into());
    }
    parse(&source, SOURCE_SHA256)
}

fn install_with(
    path: &Path,
    fetch: impl FnOnce() -> Result<Vec<u8>, String>,
) -> Result<(), String> {
    // A malformed existing file is reported, never silently replaced.
    if path.symlink_metadata().is_ok() {
        return crate::local::token(Some(path)).map(|_| ()).map_err(|_| {
            "The saved token is invalid or not private. Choose a valid 84-byte file with permissions 0600, or move the invalid file before retrying setup.".into()
        });
    }
    let token = fetch()?;
    if token.len() != 84 {
        return Err("Invalid token length. No token was installed.".into());
    }
    crate::local::write_new(path, &token).map_err(|_| {
        "Cannot save the token. Choose a writable new path; existing files are preserved.".into()
    })
}

pub fn setup(output: Option<&Path>) -> Result<PathBuf, String> {
    let path = match output {
        Some(path) => path.to_path_buf(),
        None => {
            let path = default_path()?;
            let directory = path.parent().expect("data directory");
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(directory)
                .map_err(|_| "Cannot create the private AirCard data directory.")?;
            let metadata = directory
                .symlink_metadata()
                .map_err(|_| "Cannot inspect the AirCard data directory.")?;
            if !metadata.is_dir()
                || metadata.mode() & 0o077 != 0
                || metadata.uid() != nix::unistd::getuid().as_raw()
            {
                return Err("AirCard's data directory must be owned by you, with permissions 0700, and must not be a symlink.".into());
            }
            path
        }
    };
    install_with(&path, download)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pinned_parser_rejects_changed_or_malformed_sources() {
        let source = format!(
            "static TOKENS = [{}];",
            vec![format!("\"{}\"", "ab".repeat(84)); 10].join(",")
        );
        let digest = aircard_core::sha256(source.as_bytes());
        assert_eq!(parse(source.as_bytes(), &digest).unwrap(), vec![0xab; 84]);
        assert!(parse(source.as_bytes(), SOURCE_SHA256).is_err());
        let malformed = source.replacen("ab", "zz", 1);
        assert!(
            parse(
                malformed.as_bytes(),
                &aircard_core::sha256(malformed.as_bytes())
            )
            .is_err()
        );
    }
    #[test]
    fn installation_is_private_reusable_and_never_overwrites() {
        let dir = std::env::temp_dir().join(format!("aircard-token-test-{}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        struct Clean(PathBuf);
        impl Drop for Clean {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _clean = Clean(dir.clone());
        let path = dir.join("token.bin");
        install_with(&path, || Ok(vec![42; 84])).unwrap();
        assert_eq!(path.metadata().unwrap().mode() & 0o777, 0o600);
        install_with(&path, || panic!("cached token must not download")).unwrap();
        std::fs::write(&path, b"invalid").unwrap();
        assert!(install_with(&path, || panic!("existing file must survive")).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"invalid");
        let link = dir.join("link");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(install_with(&link, || panic!("symlink must not be followed")).is_err());
        let failed = dir.join("failed");
        assert!(install_with(&failed, || Err("offline".into())).is_err());
        assert!(!failed.exists());
    }
}
