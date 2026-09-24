//! Offline setup from the built-in public compatibility-token table. Never logs token bytes.
use std::{
    os::unix::fs::{DirBuilderExt, MetadataExt},
    path::{Path, PathBuf},
};

// Public protocol material supplied for offline setup; see docs/SYNC-TOKEN.md.
// Keep entry 0 as the default, matching the previous remote-source setup.
const TOKENS_HEX: [&str; 10] = [
    "01012ba6a01f2ccf66a02613d5b72e0bc916004058a001a6874d18b5bd7b3395e25d79fa3ffcc67e718106d485c51540b828d1620e9f94f582d3bcc6f97e9088c923095ad8d36ab568fb45df61e286d25354b04c",
    "0101efa33b1586f410087474b2ccaf8cdb4d0040e5aee6017fdcf774a51c1980b4238076e86218af5a5f169470df90b73f4fc893a22da94fb9745c10f23df0620cfe3f19be3f2ab37d2f7590d8597ab51ebcced0",
    "0101aab479a6d8226f3e1d7a57a2501337e50240e54444b5f1d04101205a7a2f3d148d18440e2edef03d37fcdc7423c0bb441b4c4a355169d511d67ae3466fdf8865e69a8aed45867801ea8e1bbb48889ba0b834",
    "010194ab2ece86e05d7313e4075a947ab3be0240c9237c46b3c519d2ec297304413dab741827016e5eb9af8792bc2b3d6f12be25397931f41bffb887d042b97057c03ed8d72a3acb72378d30f7a073e3d7590f62",
    "01018cec0ea2c25446c90133d435eaafb0150240c6c28dd42f62f8907133c462fc8b6def05e543ab2d59952f6eb3b38e382d492cd2881beceaeaea67fc1331f77fca50fde6bed35622009670e6d6e4a36b09c088",
    "0101243b2587f14dd812751c6710730f46d50440fe2e5e9ccfe70200487e14c131412381fd7c214241b182ca04ebe0c1f3cdd54ac17eef31705c06289e02f672fa8c0d9dff167f1c925df876d5814d3265f55b06",
    "01016f8908f8f972bdc8fe99002f7648e86c024011a469c4320bb7e44137756dde3ecfbff08a55081e532c12a06101c6ae5283a014512d977eafad06c34b1f116422f6bf72ef56f8d734d37db287b170be7a3a82",
    "01015c5fcc103d0460f5bbfd48c387806e8e0340e3323ed780fbeccc2908c059d9a81e75976bf058b411f62a9a6e1df7ba307f69226942373f484690799b230bc60e99036f40eae25229aff6bb31ab74820e68a9",
    "0101f3f542aaa17252a8f81b3dc5b007adc304408d2496cee3af54113ffa9fba392b4143d4c38e4d79680e8e9feb554e450c1f89220c02375a9063b3a62bf61f62bd073991ebf215c6c2e28938d1aa53ed580c02",
    "0101fc26f7e89d1635d86b5c886df69f526a0040038b6c33049f6bd5cc526b4ee7ccce614135d2c73f8326d9af6d28399a231510917493d4fdaa395b4d1c1e1b09bdf3fe6fb7e16ad6d5f5cc18b93730874e6f4e",
];

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

fn decode_token(hex: &str) -> Result<Vec<u8>, String> {
    if hex.len() != 168 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Invalid built-in token data. No token was installed.".into());
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&hex[i..i + 2], 16)
                .map_err(|_| "Invalid built-in token data.".into())
        })
        .collect()
}

fn install_with(path: &Path, load: impl FnOnce() -> Result<Vec<u8>, String>) -> Result<(), String> {
    // A malformed existing file is reported, never silently replaced.
    if path.symlink_metadata().is_ok() {
        return crate::local::token(Some(path)).map(|_| ()).map_err(|_| {
            "The saved token is invalid or not private. Choose a valid 84-byte file with permissions 0600, or move the invalid file before retrying setup.".into()
        });
    }
    let token = load()?;
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
    install_with(&path, || decode_token(TOKENS_HEX[0]))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn built_in_table_contains_ten_valid_tokens() {
        for hex in TOKENS_HEX {
            let token = decode_token(hex).unwrap();
            assert_eq!(token.len(), 84);
            assert_eq!(&token[..2], &[1, 1]);
        }
        for invalid in [
            "".to_string(),
            "ab".repeat(83),
            "ab".repeat(85),
            "zz".repeat(84),
            "é".repeat(84),
        ] {
            assert!(decode_token(&invalid).is_err());
        }
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
        install_with(&path, || panic!("cached token must not be replaced")).unwrap();
        std::fs::write(&path, b"invalid").unwrap();
        assert!(install_with(&path, || panic!("existing file must survive")).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"invalid");
        let link = dir.join("link");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(install_with(&link, || panic!("symlink must not be followed")).is_err());
        let failed = dir.join("failed");
        assert!(install_with(&failed, || Err("invalid built-in data".into())).is_err());
        assert!(!failed.exists());
    }
}
